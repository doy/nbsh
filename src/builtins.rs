use async_std::io::WriteExt as _;
use std::os::unix::process::ExitStatusExt as _;

type Builtin = &'static (dyn Fn(
    &crate::parse::Exe,
    &crate::command::Env,
) -> anyhow::Result<Child>
              + Sync
              + Send);

#[allow(clippy::as_conversions)]
static BUILTINS: once_cell::sync::Lazy<
    std::collections::HashMap<&'static str, Builtin>,
> = once_cell::sync::Lazy::new(|| {
    let mut builtins = std::collections::HashMap::new();
    builtins.insert("cd", &cd as Builtin);
    builtins.insert("and", &and);
    builtins.insert("or", &or);
    builtins.insert("command", &command);
    builtins.insert("builtin", &builtin);
    builtins
});

pub struct Command {
    exe: crate::parse::Exe,
    f: Builtin,
    stdin: Box<dyn async_std::io::Read>,
    stdout: Box<dyn async_std::io::Write>,
    stderr: Box<dyn async_std::io::Write>,
}

impl Command {
    pub fn new(exe: &crate::parse::Exe) -> Option<Self> {
        BUILTINS.get(exe.exe()).map(|f| Self {
            exe: exe.clone(),
            f,
            stdin: Box::new(async_std::io::stdin()),
            stdout: Box::new(async_std::io::stdout()),
            stderr: Box::new(async_std::io::stderr()),
        })
    }

    pub fn stdin(&mut self, fh: std::fs::File) {
        self.stdin = Box::new(async_std::fs::File::from(fh));
    }

    pub fn stdout(&mut self, fh: std::fs::File) {
        self.stdout = Box::new(async_std::fs::File::from(fh));
    }

    pub fn stderr(&mut self, fh: std::fs::File) {
        self.stderr = Box::new(async_std::fs::File::from(fh));
    }

    pub fn spawn(self, env: &crate::command::Env) -> anyhow::Result<Child> {
        (self.f)(&self.exe, env)
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

fn cd(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
) -> anyhow::Result<Child> {
    async fn async_cd(
        exe: &crate::parse::Exe,
        _env: &crate::command::Env,
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
                    async_std::io::stderr()
                        .write(b"unimplemented\n")
                        .await
                        .unwrap();
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
                async_std::io::stderr()
                    .write(
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
        fut: Box::pin(async move { async_cd(&exe, &env).await }),
        wrapped_child: None,
    })
}

fn and(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    if env.latest_status().success() {
        let cmd = crate::command::Command::new(&exe);
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
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    if env.latest_status().success() {
        let env = env.clone();
        Ok(Child {
            fut: Box::pin(async move { *env.latest_status() }),
            wrapped_child: None,
        })
    } else {
        let cmd = crate::command::Command::new(&exe);
        Ok(Child {
            fut: Box::pin(async move { unreachable!() }),
            wrapped_child: Some(Box::new(cmd.spawn(env)?)),
        })
    }
}

fn command(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    let cmd = crate::command::Command::new_binary(&exe);
    Ok(Child {
        fut: Box::pin(async move { unreachable!() }),
        wrapped_child: Some(Box::new(cmd.spawn(env)?)),
    })
}

fn builtin(
    exe: &crate::parse::Exe,
    env: &crate::command::Env,
) -> anyhow::Result<Child> {
    let exe = exe.shift();
    let cmd = crate::command::Command::new_builtin(&exe);
    Ok(Child {
        fut: Box::pin(async move { unreachable!() }),
        wrapped_child: Some(Box::new(cmd.spawn(env)?)),
    })
}

fn home() -> std::path::PathBuf {
    std::env::var_os("HOME").unwrap().into()
}
