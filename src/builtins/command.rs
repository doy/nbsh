use async_std::io::{ReadExt as _, WriteExt as _};
use std::os::unix::io::{AsRawFd as _, FromRawFd as _, IntoRawFd as _};

pub struct Command {
    exe: crate::parse::Exe,
    f: super::Builtin,
    io: Io,
}

impl Command {
    pub fn new(exe: crate::parse::Exe) -> Result<Self, crate::parse::Exe> {
        if let Some(f) = super::BUILTINS.get(exe.exe()) {
            Ok(Self {
                exe,
                f,
                io: Io::new(),
            })
        } else {
            Err(exe)
        }
    }

    pub fn stdin(&mut self, fh: std::fs::File) {
        self.io.set_stdin(fh);
    }

    pub fn stdout(&mut self, fh: std::fs::File) {
        self.io.set_stdout(fh);
    }

    pub fn stderr(&mut self, fh: std::fs::File) {
        self.io.set_stderr(fh);
    }

    // Safety: see pre_exec in async_std::os::unix::process::CommandExt (this
    // is just a wrapper)
    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        self.io.pre_exec(f);
    }

    pub fn spawn(self, env: &crate::env::Env) -> anyhow::Result<Child> {
        let Self { f, exe, io } = self;
        (f)(exe, env, io)
    }
}

pub struct Io {
    fds: std::collections::HashMap<
        std::os::unix::io::RawFd,
        std::os::unix::io::RawFd,
    >,
    pre_exec: Option<
        Box<dyn 'static + FnMut() -> std::io::Result<()> + Send + Sync>,
    >,
}

impl Io {
    fn new() -> Self {
        let mut fds = std::collections::HashMap::new();
        fds.insert(0.as_raw_fd(), 0.as_raw_fd());
        fds.insert(1.as_raw_fd(), 1.as_raw_fd());
        fds.insert(2.as_raw_fd(), 2.as_raw_fd());
        Self {
            fds,
            pre_exec: None,
        }
    }

    fn stdin(&self) -> Option<async_std::fs::File> {
        self.fds
            .get(&0.as_raw_fd())
            .copied()
            // Safety: TODO this is likely unsafe
            .map(|fd| unsafe { async_std::fs::File::from_raw_fd(fd) })
    }

    fn set_stdin<T: std::os::unix::io::IntoRawFd>(&mut self, stdin: T) {
        if let Some(fd) = self.fds.get(&0.as_raw_fd()) {
            if *fd > 2 {
                // Safety: TODO this is likely unsafe
                drop(unsafe { async_std::fs::File::from_raw_fd(*fd) });
            }
        }
        self.fds.insert(0.as_raw_fd(), stdin.into_raw_fd());
    }

    fn stdout(&self) -> Option<async_std::fs::File> {
        self.fds
            .get(&1.as_raw_fd())
            .copied()
            // Safety: TODO this is likely unsafe
            .map(|fd| unsafe { async_std::fs::File::from_raw_fd(fd) })
    }

    fn set_stdout<T: std::os::unix::io::IntoRawFd>(&mut self, stdout: T) {
        if let Some(fd) = self.fds.get(&1.as_raw_fd()) {
            if *fd > 2 {
                // Safety: TODO this is likely unsafe
                drop(unsafe { async_std::fs::File::from_raw_fd(*fd) });
            }
        }
        self.fds.insert(1.as_raw_fd(), stdout.into_raw_fd());
    }

    fn stderr(&self) -> Option<async_std::fs::File> {
        self.fds
            .get(&2.as_raw_fd())
            .copied()
            // Safety: TODO this is likely unsafe
            .map(|fd| unsafe { async_std::fs::File::from_raw_fd(fd) })
    }

    fn set_stderr<T: std::os::unix::io::IntoRawFd>(&mut self, stderr: T) {
        if let Some(fd) = self.fds.get(&2.as_raw_fd()) {
            if *fd > 2 {
                // Safety: TODO this is likely unsafe
                drop(unsafe { async_std::fs::File::from_raw_fd(*fd) });
            }
        }
        self.fds.insert(2.as_raw_fd(), stderr.into_raw_fd());
    }

    // Safety: see pre_exec in async_std::os::unix::process::CommandExt (this
    // is just a wrapper)
    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        self.pre_exec = Some(Box::new(f));
    }

    pub async fn read_stdin(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        if let Some(mut fh) = self.stdin() {
            let res = fh.read(buf).await;
            let _ = fh.into_raw_fd();
            Ok(res?)
        } else {
            Ok(0)
        }
    }

    pub async fn write_stdout(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(mut fh) = self.stdout() {
            let res = fh.write_all(buf).await;
            let _ = fh.into_raw_fd();
            Ok(res.map(|_| ())?)
        } else {
            Ok(())
        }
    }

    pub async fn write_stderr(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(mut fh) = self.stderr() {
            let res = fh.write_all(buf).await;
            let _ = fh.into_raw_fd();
            Ok(res.map(|_| ())?)
        } else {
            Ok(())
        }
    }

    pub fn setup_command(mut self, cmd: &mut crate::pipeline::Command) {
        if let Some(stdin) = self.stdin() {
            let stdin = stdin.into_raw_fd();
            if stdin != 0 {
                // Safety: TODO this is likely unsafe
                cmd.stdin(unsafe { std::fs::File::from_raw_fd(stdin) });
                self.fds.remove(&0.as_raw_fd());
            }
        }
        if let Some(stdout) = self.stdout() {
            let stdout = stdout.into_raw_fd();
            if stdout != 1 {
                // Safety: TODO this is likely unsafe
                cmd.stdout(unsafe { std::fs::File::from_raw_fd(stdout) });
                self.fds.remove(&1.as_raw_fd());
            }
        }
        if let Some(stderr) = self.stderr() {
            let stderr = stderr.into_raw_fd();
            if stderr != 2 {
                // Safety: TODO this is likely unsafe
                cmd.stderr(unsafe { std::fs::File::from_raw_fd(stderr) });
                self.fds.remove(&2.as_raw_fd());
            }
        }
        if let Some(pre_exec) = self.pre_exec.take() {
            // Safety: pre_exec can only have been set by calling the pre_exec
            // method, which is itself unsafe, so the safety comments at the
            // point where that is called are the relevant ones
            unsafe { cmd.pre_exec(pre_exec) };
        }
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        for fd in self.fds.values() {
            if *fd > 2 {
                // Safety: TODO this is likely unsafe
                drop(unsafe { std::fs::File::from_raw_fd(*fd) });
            }
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

    // can't use async_recursion because it enforces a 'static lifetime
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
