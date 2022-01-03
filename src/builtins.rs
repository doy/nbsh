use async_std::io::{ReadExt as _, WriteExt as _};
use std::os::unix::io::{AsRawFd as _, FromRawFd as _, IntoRawFd as _};
use std::os::unix::process::ExitStatusExt as _;

struct Io {
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
            .map(|fd| unsafe { async_std::fs::File::from_raw_fd(fd) })
    }

    fn set_stdin<T: std::os::unix::io::IntoRawFd>(&mut self, stdin: T) {
        if let Some(fd) = self.fds.get(&0.as_raw_fd()) {
            if *fd > 2 {
                drop(unsafe { async_std::fs::File::from_raw_fd(*fd) });
            }
        }
        self.fds.insert(0.as_raw_fd(), stdin.into_raw_fd());
    }

    fn stdout(&self) -> Option<async_std::fs::File> {
        self.fds
            .get(&1.as_raw_fd())
            .copied()
            .map(|fd| unsafe { async_std::fs::File::from_raw_fd(fd) })
    }

    fn set_stdout<T: std::os::unix::io::IntoRawFd>(&mut self, stdout: T) {
        if let Some(fd) = self.fds.get(&1.as_raw_fd()) {
            if *fd > 2 {
                drop(unsafe { async_std::fs::File::from_raw_fd(*fd) });
            }
        }
        self.fds.insert(1.as_raw_fd(), stdout.into_raw_fd());
    }

    fn stderr(&self) -> Option<async_std::fs::File> {
        self.fds
            .get(&2.as_raw_fd())
            .copied()
            .map(|fd| unsafe { async_std::fs::File::from_raw_fd(fd) })
    }

    fn set_stderr<T: std::os::unix::io::IntoRawFd>(&mut self, stderr: T) {
        if let Some(fd) = self.fds.get(&2.as_raw_fd()) {
            if *fd > 2 {
                drop(unsafe { async_std::fs::File::from_raw_fd(*fd) });
            }
        }
        self.fds.insert(2.as_raw_fd(), stderr.into_raw_fd());
    }

    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        self.pre_exec = Some(Box::new(f));
    }

    async fn read_stdin(&self, buf: &mut [u8]) -> anyhow::Result<usize> {
        if let Some(mut fh) = self.stdin() {
            let res = fh.read(buf).await;
            let _ = fh.into_raw_fd();
            Ok(res?)
        } else {
            Ok(0)
        }
    }

    async fn write_stdout(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(mut fh) = self.stdout() {
            let res = fh.write_all(buf).await;
            let _ = fh.into_raw_fd();
            Ok(res.map(|_| ())?)
        } else {
            Ok(())
        }
    }

    async fn write_stderr(&self, buf: &[u8]) -> anyhow::Result<()> {
        if let Some(mut fh) = self.stderr() {
            let res = fh.write_all(buf).await;
            let _ = fh.into_raw_fd();
            Ok(res.map(|_| ())?)
        } else {
            Ok(())
        }
    }

    fn setup_command(mut self, cmd: &mut crate::command::Command) {
        if let Some(stdin) = self.stdin() {
            let stdin = stdin.into_raw_fd();
            if stdin != 0 {
                cmd.stdin(unsafe { std::fs::File::from_raw_fd(stdin) });
                self.fds.remove(&0.as_raw_fd());
            }
        }
        if let Some(stdout) = self.stdout() {
            let stdout = stdout.into_raw_fd();
            if stdout != 1 {
                cmd.stdout(unsafe { std::fs::File::from_raw_fd(stdout) });
                self.fds.remove(&1.as_raw_fd());
            }
        }
        if let Some(stderr) = self.stderr() {
            let stderr = stderr.into_raw_fd();
            if stderr != 2 {
                cmd.stderr(unsafe { std::fs::File::from_raw_fd(stderr) });
                self.fds.remove(&2.as_raw_fd());
            }
        }
        if let Some(pre_exec) = self.pre_exec.take() {
            unsafe { cmd.pre_exec(pre_exec) };
        }
    }
}

impl Drop for Io {
    fn drop(&mut self) {
        for fd in self.fds.values() {
            if *fd > 2 {
                drop(unsafe { std::fs::File::from_raw_fd(*fd) });
            }
        }
    }
}

type Builtin = &'static (dyn Fn(
    &crate::parse::Exe,
    &crate::command::Env,
    Io,
) -> anyhow::Result<Child>
              + Sync
              + Send);

#[allow(clippy::as_conversions)]
static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    let mut builtins = std::collections::HashMap::new();
    builtins.insert("cd", &cd as Builtin);
    builtins.insert("echo", &echo);
    builtins.insert("and", &and);
    builtins.insert("or", &or);
    builtins.insert("command", &command);
    builtins.insert("builtin", &builtin);
    builtins
});

pub struct Command {
    exe: crate::parse::Exe,
    f: Builtin,
    io: Io,
}

impl Command {
    pub fn new(exe: &crate::parse::Exe) -> Option<Self> {
        BUILTINS.get(exe.exe()).map(|f| Self {
            exe: exe.clone(),
            f,
            io: Io::new(),
        })
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

    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        self.io.pre_exec(f);
    }

    pub fn spawn(self, env: &crate::command::Env) -> anyhow::Result<Child> {
        let Self { f, exe, io } = self;
        (f)(&exe, env, io)
    }
}

pub struct Child {
    fut: std::pin::Pin<
        Box<
            dyn std::future::Future<Output = std::process::ExitStatus>
                + Sync
                + Send,
        >,
    >,
    wrapped_child: Option<Box<crate::command::Child>>,
}

impl Child {
    pub fn id(&self) -> Option<u32> {
        self.wrapped_child.as_ref().and_then(|cmd| cmd.id())
    }

    #[async_recursion::async_recursion]
    pub async fn status(
        self,
    ) -> anyhow::Result<async_std::process::ExitStatus> {
        if let Some(child) = self.wrapped_child {
            child.status().await
        } else {
            Ok(self.fut.await)
        }
    }
}

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
fn cd(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
    io: Io,
) -> anyhow::Result<Child> {
    async fn async_cd(
        exe: &crate::parse::Exe,
        _env: &crate::command::Env,
        io: Io,
    ) -> std::process::ExitStatus {
        let dir = exe
            .args()
            .into_iter()
            .map(std::convert::AsRef::as_ref)
            .next()
            .unwrap_or("");

        let dir = if dir.is_empty() {
            home()
        } else if dir.starts_with('~') {
            let path: std::path::PathBuf = dir.into();
            if let std::path::Component::Normal(prefix) =
                path.components().next().unwrap()
            {
                if prefix.to_str() == Some("~") {
                    home().join(path.strip_prefix(prefix).unwrap())
                } else {
                    // TODO
                    io.write_stderr(b"unimplemented\n").await.unwrap();
                    return async_std::process::ExitStatus::from_raw(1 << 8);
                }
            } else {
                unreachable!()
            }
        } else {
            dir.into()
        };
        let code = match std::env::set_current_dir(&dir) {
            Ok(()) => 0,
            Err(e) => {
                io.write_stderr(
                    format!(
                        "{}: {}: {}\n",
                        exe.exe(),
                        crate::format::io_error(&e),
                        dir.display()
                    )
                    .as_bytes(),
                )
                .await
                .unwrap();
                1
            }
        };
        async_std::process::ExitStatus::from_raw(code << 8)
    }

    let exe = exe.clone();
    let env = env.clone();
    Ok(Child {
        fut: Box::pin(async move { async_cd(&exe, &env, io).await }),
        wrapped_child: None,
    })
}

// clippy can't tell that the type is necessary
#[allow(clippy::unnecessary_wraps)]
// mostly just for testing and ensuring that builtins work, i'll likely remove
// this later, since the binary seems totally fine
fn echo(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
    io: Io,
) -> anyhow::Result<Child> {
    async fn async_echo(
        exe: &crate::parse::Exe,
        _env: &crate::command::Env,
        io: Io,
    ) -> std::process::ExitStatus {
        macro_rules! write_stdout {
            ($bytes:expr) => {
                if let Err(e) = io.write_stdout($bytes).await {
                    io.write_stderr(format!("echo: {}", e).as_bytes())
                        .await
                        .unwrap();
                    return async_std::process::ExitStatus::from_raw(1 << 8);
                }
            };
        }
        let count = exe.args().count();
        for (i, arg) in exe.args().enumerate() {
            write_stdout!(arg.as_bytes());
            if i == count - 1 {
                write_stdout!(b"\n");
            } else {
                write_stdout!(b" ");
            }
        }

        async_std::process::ExitStatus::from_raw(0)
    }

    let exe = exe.clone();
    let env = env.clone();
    Ok(Child {
        fut: Box::pin(async move { async_echo(&exe, &env, io).await }),
        wrapped_child: None,
    })
}

fn and(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
    io: Io,
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    if env.latest_status().success() {
        let mut cmd = crate::command::Command::new(&exe);
        io.setup_command(&mut cmd);
        Ok(Child {
            fut: Box::pin(async move { unreachable!() }),
            wrapped_child: Some(Box::new(cmd.spawn(env)?)),
        })
    } else {
        let env = env.clone();
        Ok(Child {
            fut: Box::pin(async move { *env.latest_status() }),
            wrapped_child: None,
        })
    }
}

fn or(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
    io: Io,
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    if env.latest_status().success() {
        let env = env.clone();
        Ok(Child {
            fut: Box::pin(async move { *env.latest_status() }),
            wrapped_child: None,
        })
    } else {
        let mut cmd = crate::command::Command::new(&exe);
        io.setup_command(&mut cmd);
        Ok(Child {
            fut: Box::pin(async move { unreachable!() }),
            wrapped_child: Some(Box::new(cmd.spawn(env)?)),
        })
    }
}

fn command(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
    io: Io,
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    let mut cmd = crate::command::Command::new_binary(&exe);
    io.setup_command(&mut cmd);
    Ok(Child {
        fut: Box::pin(async move { unreachable!() }),
        wrapped_child: Some(Box::new(cmd.spawn(env)?)),
    })
}

fn builtin(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
    io: Io,
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    let mut cmd = crate::command::Command::new_builtin(&exe);
    io.setup_command(&mut cmd);
    Ok(Child {
        fut: Box::pin(async move { unreachable!() }),
        wrapped_child: Some(Box::new(cmd.spawn(env)?)),
    })
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").unwrap().into()
}
