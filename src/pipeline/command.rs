use async_std::os::unix::process::CommandExt as _;

pub struct Command {
    inner: Inner,
    exe: String,
}
pub enum Inner {
    Binary(async_std::process::Command),
    Builtin(crate::builtins::Command),
}

impl Command {
    pub fn new(exe: crate::parse::Exe) -> Self {
        let exe_str = exe.exe().to_string();
        Self {
            inner: crate::builtins::Command::new(exe).map_or_else(
                |exe| Self::new_binary(exe).inner,
                Inner::Builtin,
            ),
            exe: exe_str,
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn new_binary(exe: crate::parse::Exe) -> Self {
        let exe_str = exe.exe().to_string();
        let mut cmd = async_std::process::Command::new(exe.exe());
        cmd.args(exe.args());
        Self {
            inner: Inner::Binary(cmd),
            exe: exe_str,
        }
    }

    pub fn new_builtin(exe: crate::parse::Exe) -> Self {
        let exe_str = exe.exe().to_string();
        Self {
            inner: crate::builtins::Command::new(exe)
                .map_or_else(|_| todo!(), Inner::Builtin),
            exe: exe_str,
        }
    }

    pub fn stdin(&mut self, fh: std::fs::File) {
        match &mut self.inner {
            Inner::Binary(cmd) => {
                cmd.stdin(fh);
            }
            Inner::Builtin(cmd) => {
                cmd.stdin(fh);
            }
        }
    }

    pub fn stdout(&mut self, fh: std::fs::File) {
        match &mut self.inner {
            Inner::Binary(cmd) => {
                cmd.stdout(fh);
            }
            Inner::Builtin(cmd) => {
                cmd.stdout(fh);
            }
        }
    }

    pub fn stderr(&mut self, fh: std::fs::File) {
        match &mut self.inner {
            Inner::Binary(cmd) => {
                cmd.stderr(fh);
            }
            Inner::Builtin(cmd) => {
                cmd.stderr(fh);
            }
        }
    }

    // Safety: see pre_exec in async_std::os::unix::process::CommandExt (this
    // is just a wrapper)
    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        match &mut self.inner {
            Inner::Binary(cmd) => {
                cmd.pre_exec(f);
            }
            Inner::Builtin(cmd) => {
                cmd.pre_exec(f);
            }
        }
    }

    pub fn spawn(self, env: &crate::env::Env) -> anyhow::Result<Child> {
        match self.inner {
            Inner::Binary(mut cmd) => {
                Ok(Child::Binary(cmd.spawn().map_err(|e| {
                    anyhow::anyhow!(
                        "{}: {}",
                        crate::format::io_error(&e),
                        self.exe
                    )
                })?))
            }
            Inner::Builtin(cmd) => Ok(Child::Builtin(cmd.spawn(env)?)),
        }
    }
}

pub enum Child<'a> {
    Binary(async_std::process::Child),
    Builtin(crate::builtins::Child<'a>),
}

impl<'a> Child<'a> {
    pub fn id(&self) -> Option<u32> {
        match self {
            Self::Binary(child) => Some(child.id()),
            Self::Builtin(child) => child.id(),
        }
    }

    // can't use async_recursion because it enforces a 'static lifetime
    pub fn status(
        self,
    ) -> std::pin::Pin<
        Box<
            dyn std::future::Future<
                    Output = anyhow::Result<std::process::ExitStatus>,
                > + Send
                + Sync
                + 'a,
        >,
    > {
        Box::pin(async move {
            match self {
                Self::Binary(child) => Ok(child.status_no_drop().await?),
                Self::Builtin(child) => Ok(child.status().await?),
            }
        })
    }
}
