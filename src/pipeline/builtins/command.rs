use crate::pipeline::prelude::*;

pub struct Command {
    exe: crate::parse::Exe,
    f: super::Builtin,
    cfg: Cfg,
}

impl Command {
    pub fn new(exe: crate::parse::Exe) -> Result<Self, crate::parse::Exe> {
        if let Some(s) = exe.exe().to_str() {
            if let Some(f) = super::BUILTINS.get(s) {
                Ok(Self {
                    exe,
                    f,
                    cfg: Cfg::new(),
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
    fn new() -> Self {
        Self {
            io: Io::new(),
            pre_exec: None,
        }
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

    pub fn setup_command(mut self, cmd: &mut crate::pipeline::Command) {
        self.io.setup_command(cmd);
        if let Some(pre_exec) = self.pre_exec.take() {
            // Safety: pre_exec can only have been set by calling the pre_exec
            // method, which is itself unsafe, so the safety comments at the
            // point where that is called are the relevant ones
            unsafe { cmd.pre_exec(pre_exec) };
        }
    }
}

pub struct Io {
    fds: std::collections::HashMap<
        std::os::unix::io::RawFd,
        crate::mutex::Mutex<File>,
    >,
}

impl Io {
    fn new() -> Self {
        let mut fds = std::collections::HashMap::new();
        fds.insert(0, crate::mutex::new(unsafe { File::input(0) }));
        fds.insert(1, crate::mutex::new(unsafe { File::output(1) }));
        fds.insert(2, crate::mutex::new(unsafe { File::output(2) }));
        Self { fds }
    }

    fn stdin(&self) -> Option<crate::mutex::Mutex<File>> {
        self.fds.get(&0).map(async_std::sync::Arc::clone)
    }

    fn set_stdin<T: std::os::unix::io::IntoRawFd>(&mut self, stdin: T) {
        if let Some(file) = self.fds.remove(&0) {
            let file = crate::mutex::unwrap(file);
            if file.as_raw_fd() <= 2 {
                let _ = file.into_raw_fd();
            }
        }
        self.fds.insert(
            0,
            crate::mutex::new(unsafe { File::input(stdin.into_raw_fd()) }),
        );
    }

    fn stdout(&self) -> Option<crate::mutex::Mutex<File>> {
        self.fds.get(&1).map(async_std::sync::Arc::clone)
    }

    fn set_stdout<T: std::os::unix::io::IntoRawFd>(&mut self, stdout: T) {
        if let Some(file) = self.fds.remove(&1) {
            let file = crate::mutex::unwrap(file);
            if file.as_raw_fd() <= 2 {
                let _ = file.into_raw_fd();
            }
        }
        self.fds.insert(
            1,
            crate::mutex::new(unsafe { File::output(stdout.into_raw_fd()) }),
        );
    }

    fn stderr(&self) -> Option<crate::mutex::Mutex<File>> {
        self.fds.get(&2).map(async_std::sync::Arc::clone)
    }

    fn set_stderr<T: std::os::unix::io::IntoRawFd>(&mut self, stderr: T) {
        if let Some(file) = self.fds.remove(&2) {
            let file = crate::mutex::unwrap(file);
            if file.as_raw_fd() <= 2 {
                let _ = file.into_raw_fd();
            }
        }
        self.fds.insert(
            2,
            crate::mutex::new(unsafe { File::output(stderr.into_raw_fd()) }),
        );
    }

    pub fn apply_redirects(&mut self, redirects: &[crate::parse::Redirect]) {
        for redirect in redirects {
            let to = match &redirect.to {
                crate::parse::RedirectTarget::Fd(fd) => {
                    async_std::sync::Arc::clone(&self.fds[fd])
                }
                crate::parse::RedirectTarget::File(path) => {
                    let fd = redirect.dir.open(path).unwrap();
                    match redirect.dir {
                        crate::parse::Direction::In => {
                            crate::mutex::new(unsafe { File::input(fd) })
                        }
                        crate::parse::Direction::Out
                        | crate::parse::Direction::Append => {
                            crate::mutex::new(unsafe { File::output(fd) })
                        }
                    }
                }
            };
            self.fds.insert(redirect.from, to);
        }
    }

    pub async fn read_stdin(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        if let Some(fh) = self.stdin() {
            if let File::In(fh) = &mut *fh.lock_arc().await {
                Ok(fh.read(buf).await?)
            } else {
                Ok(0)
            }
        } else {
            Ok(0)
        }
    }

    pub async fn write_stdout(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(fh) = self.stdout() {
            if let File::Out(fh) = &mut *fh.lock_arc().await {
                Ok(fh.write_all(buf).await.map(|_| ())?)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    pub async fn write_stderr(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(fh) = self.stderr() {
            if let File::Out(fh) = &mut *fh.lock_arc().await {
                Ok(fh.write_all(buf).await.map(|_| ())?)
            } else {
                Ok(())
            }
        } else {
            Ok(())
        }
    }

    pub fn setup_command(mut self, cmd: &mut crate::pipeline::Command) {
        if let Some(stdin) = self.fds.remove(&0) {
            let stdin = crate::mutex::unwrap(stdin).into_raw_fd();
            if stdin != 0 {
                // Safety: TODO this is likely unsafe
                cmd.stdin(unsafe { std::fs::File::from_raw_fd(stdin) });
                self.fds.remove(&0);
            }
        }
        if let Some(stdout) = self.fds.remove(&1) {
            let stdout = crate::mutex::unwrap(stdout).into_raw_fd();
            if stdout != 1 {
                // Safety: TODO this is likely unsafe
                cmd.stdout(unsafe { std::fs::File::from_raw_fd(stdout) });
                self.fds.remove(&1);
            }
        }
        if let Some(stderr) = self.fds.remove(&2) {
            let stderr = crate::mutex::unwrap(stderr).into_raw_fd();
            if stderr != 2 {
                // Safety: TODO this is likely unsafe
                cmd.stderr(unsafe { std::fs::File::from_raw_fd(stderr) });
                self.fds.remove(&2);
            }
        }
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        for (_, file) in self.fds.drain() {
            let file = crate::mutex::unwrap(file);
            if file.as_raw_fd() <= 2 {
                let _ = file.into_raw_fd();
            }
        }
    }
}

#[derive(Debug)]
pub enum File {
    In(async_std::io::BufReader<async_std::fs::File>),
    Out(async_std::fs::File),
}

impl File {
    unsafe fn input(fd: std::os::unix::io::RawFd) -> Self {
        Self::In(async_std::io::BufReader::new(
            async_std::fs::File::from_raw_fd(fd),
        ))
    }

    unsafe fn output(fd: std::os::unix::io::RawFd) -> Self {
        Self::Out(async_std::fs::File::from_raw_fd(fd))
    }
}

impl std::os::unix::io::AsRawFd for File {
    fn as_raw_fd(&self) -> std::os::unix::io::RawFd {
        match self {
            Self::In(fh) => fh.get_ref().as_raw_fd(),
            Self::Out(fh) => fh.as_raw_fd(),
        }
    }
}

impl std::os::unix::io::IntoRawFd for File {
    fn into_raw_fd(self) -> std::os::unix::io::RawFd {
        match self {
            Self::In(fh) => fh.into_inner().into_raw_fd(),
            Self::Out(fh) => fh.into_raw_fd(),
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
    wrapped_child: Option<Box<crate::pipeline::Child<'a>>>,
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

    pub fn new_wrapped(child: crate::pipeline::Child<'a>) -> Self {
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
