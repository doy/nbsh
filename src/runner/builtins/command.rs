use crate::runner::prelude::*;

pub struct Command {
    exe: crate::parse::Exe,
    f: super::Builtin,
    cfg: Cfg,
}

impl Command {
    pub fn new(
        exe: crate::parse::Exe,
        io: Io,
    ) -> Result<Self, crate::parse::Exe> {
        if let Some(s) = exe.exe().to_str() {
            if let Some(f) = super::BUILTINS.get(s) {
                Ok(Self {
                    exe,
                    f,
                    cfg: Cfg::new(io),
                })
            } else {
                Err(exe)
            }
        } else {
            Err(exe)
        }
    }

    pub fn stdin(&mut self, fh: std::fs::File) {
        self.cfg.io.set_stdin(fh);
    }

    pub fn stdout(&mut self, fh: std::fs::File) {
        self.cfg.io.set_stdout(fh);
    }

    pub fn stderr(&mut self, fh: std::fs::File) {
        self.cfg.io.set_stderr(fh);
    }

    // Safety: see pre_exec in async_std::os::unix::process::CommandExt (this
    // is just a wrapper)
    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        self.cfg.pre_exec(f);
    }

    pub fn apply_redirects(&mut self, redirects: &[crate::parse::Redirect]) {
        self.cfg.io.apply_redirects(redirects);
    }

    pub fn spawn(self, env: &Env) -> anyhow::Result<Child> {
        let Self { f, exe, cfg } = self;
        (f)(exe, env, cfg)
    }
}

pub struct Cfg {
    io: Io,
    pre_exec: Option<
        Box<dyn 'static + FnMut() -> std::io::Result<()> + Send + Sync>,
    >,
}

impl Cfg {
    fn new(io: Io) -> Self {
        Self { io, pre_exec: None }
    }

    pub fn io(&self) -> &Io {
        &self.io
    }

    // Safety: see pre_exec in async_std::os::unix::process::CommandExt (this
    // is just a wrapper)
    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        self.pre_exec = Some(Box::new(f));
    }

    pub fn setup_command(mut self, cmd: &mut crate::runner::Command) {
        self.io.setup_command(cmd);
        if let Some(pre_exec) = self.pre_exec.take() {
            // Safety: pre_exec can only have been set by calling the pre_exec
            // method, which is itself unsafe, so the safety comments at the
            // point where that is called are the relevant ones
            unsafe { cmd.pre_exec(pre_exec) };
        }
    }
}

#[derive(Debug, Clone)]
pub struct Io {
    fds: std::collections::HashMap<
        std::os::unix::io::RawFd,
        std::sync::Arc<File>,
    >,
}

impl Io {
    pub fn new() -> Self {
        Self {
            fds: std::collections::HashMap::new(),
        }
    }

    fn stdin(&self) -> Option<std::sync::Arc<File>> {
        self.fds.get(&0).map(std::sync::Arc::clone)
    }

    pub fn set_stdin<T: std::os::unix::io::IntoRawFd>(&mut self, stdin: T) {
        if let Some(file) = self.fds.remove(&0) {
            File::maybe_drop(file);
        }
        self.fds.insert(
            0,
            // Safety: we just acquired stdin via into_raw_fd, which acquires
            // ownership of the fd, so we are now the sole owner
            std::sync::Arc::new(unsafe { File::input(stdin.into_raw_fd()) }),
        );
    }

    fn stdout(&self) -> Option<std::sync::Arc<File>> {
        self.fds.get(&1).map(std::sync::Arc::clone)
    }

    pub fn set_stdout<T: std::os::unix::io::IntoRawFd>(&mut self, stdout: T) {
        if let Some(file) = self.fds.remove(&1) {
            File::maybe_drop(file);
        }
        self.fds.insert(
            1,
            // Safety: we just acquired stdout via into_raw_fd, which acquires
            // ownership of the fd, so we are now the sole owner
            std::sync::Arc::new(unsafe {
                File::output(stdout.into_raw_fd())
            }),
        );
    }

    fn stderr(&self) -> Option<std::sync::Arc<File>> {
        self.fds.get(&2).map(std::sync::Arc::clone)
    }

    pub fn set_stderr<T: std::os::unix::io::IntoRawFd>(&mut self, stderr: T) {
        if let Some(file) = self.fds.remove(&2) {
            File::maybe_drop(file);
        }
        self.fds.insert(
            2,
            // Safety: we just acquired stderr via into_raw_fd, which acquires
            // ownership of the fd, so we are now the sole owner
            std::sync::Arc::new(unsafe {
                File::output(stderr.into_raw_fd())
            }),
        );
    }

    pub fn apply_redirects(&mut self, redirects: &[crate::parse::Redirect]) {
        for redirect in redirects {
            let to = match &redirect.to {
                crate::parse::RedirectTarget::Fd(fd) => {
                    std::sync::Arc::clone(&self.fds[fd])
                }
                crate::parse::RedirectTarget::File(path) => {
                    let fd = redirect.dir.open(path).unwrap();
                    match redirect.dir {
                        crate::parse::Direction::In => {
                            // Safety: we just opened fd, and nothing else has
                            // or can use it
                            std::sync::Arc::new(unsafe { File::input(fd) })
                        }
                        crate::parse::Direction::Out
                        | crate::parse::Direction::Append => {
                            // Safety: we just opened fd, and nothing else has
                            // or can use it
                            std::sync::Arc::new(unsafe { File::output(fd) })
                        }
                    }
                }
            };
            self.fds.insert(redirect.from, to);
        }
    }

    pub async fn read_line_stdin(&self) -> anyhow::Result<(String, bool)> {
        let mut buf = vec![];
        if let Some(fh) = self.stdin() {
            if let File::In(fh) = &*fh {
                // we have to read only a single character at a time here
                // because stdin needs to be shared across all commands in the
                // command list, some of which may be builtins and others of
                // which may be external commands - if we read past the end of
                // a line, then the characters past the end of that line will
                // no longer be available to the next command, since we have
                // them buffered in memory rather than them being on the stdin
                // pipe.
                let mut bytes = fh.bytes();
                while let Some(byte) = bytes.next().await {
                    let byte = byte?;
                    buf.push(byte);
                    if byte == b'\n' {
                        break;
                    }
                }
            }
        }
        let done = buf.is_empty();
        let mut buf = String::from_utf8(buf).unwrap();
        if buf.ends_with('\n') {
            buf.truncate(buf.len() - 1);
        }
        Ok((buf, done))
    }

    pub async fn write_stdout(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(fh) = self.stdout() {
            if let File::Out(fh) = &*fh {
                Ok((&*fh).write_all(buf).await.map(|_| ())?)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    pub async fn write_stderr(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(fh) = self.stderr() {
            if let File::Out(fh) = &*fh {
                Ok((&*fh).write_all(buf).await.map(|_| ())?)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    pub fn setup_command(mut self, cmd: &mut crate::runner::Command) {
        if let Some(stdin) = self.fds.remove(&0) {
            if let Ok(stdin) = std::sync::Arc::try_unwrap(stdin) {
                let stdin = stdin.into_raw_fd();
                if stdin != 0 {
                    // Safety: we just acquired stdin via into_raw_fd, which
                    // acquires ownership of the fd, so we are now the sole
                    // owner
                    cmd.stdin(unsafe { std::fs::File::from_raw_fd(stdin) });
                    self.fds.remove(&0);
                }
            }
        }
        if let Some(stdout) = self.fds.remove(&1) {
            if let Ok(stdout) = std::sync::Arc::try_unwrap(stdout) {
                let stdout = stdout.into_raw_fd();
                if stdout != 1 {
                    // Safety: we just acquired stdout via into_raw_fd, which
                    // acquires ownership of the fd, so we are now the sole
                    // owner
                    cmd.stdout(unsafe { std::fs::File::from_raw_fd(stdout) });
                    self.fds.remove(&1);
                }
            }
        }
        if let Some(stderr) = self.fds.remove(&2) {
            if let Ok(stderr) = std::sync::Arc::try_unwrap(stderr) {
                let stderr = stderr.into_raw_fd();
                if stderr != 2 {
                    // Safety: we just acquired stderr via into_raw_fd, which
                    // acquires ownership of the fd, so we are now the sole
                    // owner
                    cmd.stderr(unsafe { std::fs::File::from_raw_fd(stderr) });
                    self.fds.remove(&2);
                }
            }
        }
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        for (_, file) in self.fds.drain() {
            File::maybe_drop(file);
        }
    }
}

#[derive(Debug)]
pub enum File {
    In(async_std::fs::File),
    Out(async_std::fs::File),
}

impl File {
    // Safety: fd must not be owned by any other File object
    pub unsafe fn input(fd: std::os::unix::io::RawFd) -> Self {
        Self::In(async_std::fs::File::from_raw_fd(fd))
    }

    // Safety: fd must not be owned by any other File object
    pub unsafe fn output(fd: std::os::unix::io::RawFd) -> Self {
        Self::Out(async_std::fs::File::from_raw_fd(fd))
    }

    fn maybe_drop(file: std::sync::Arc<Self>) {
        if let Ok(file) = std::sync::Arc::try_unwrap(file) {
            if file.as_raw_fd() <= 2 {
                let _ = file.into_raw_fd();
            }
        }
    }
}

impl std::os::unix::io::AsRawFd for File {
    fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        match self {
            Self::In(fh) | Self::Out(fh) => fh.as_raw_fd(),
        }
    }
}

impl std::os::unix::io::IntoRawFd for File {
    fn into_raw_fd(self) -> std::os::unix::io::RawFd {
        match self {
            Self::In(fh) | Self::Out(fh) => fh.into_raw_fd(),
        }
    }
}

pub struct Child<'a> {
    fut: std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::process::ExitStatus>
                + Sync
                + Send
                + 'a,
        >,
    >,
    wrapped_child: Option<Box<crate::runner::Child<'a>>>,
}

impl<'a> Child<'a> {
    pub fn new_fut<F>(fut: F) -> Self
    where
        F: std::future::Future<Output = std::process::ExitStatus>
            + Sync
            + Send
            + 'a,
    {
        Self {
            fut: Box::pin(fut),
            wrapped_child: None,
        }
    }

    pub fn new_wrapped(child: crate::runner::Child<'a>) -> Self {
        Self {
            fut: Box::pin(async move { unreachable!() }),
            wrapped_child: Some(Box::new(child)),
        }
    }

    pub fn id(&self) -> Option<u32> {
        self.wrapped_child.as_ref().and_then(|cmd| cmd.id())
    }

    pub fn status(
        self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<async_std::process::ExitStatus>,
                > + Send
                + Sync
                + 'a,
        >,
    > {
        Box::pin(async move {
            if let Some(child) = self.wrapped_child {
                child.status().await
            } else {
                Ok(self.fut.await)
            }
        })
    }
}
