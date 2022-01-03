use async_std::os::unix::process::CommandExt as _;

pub enum Command {
    Binary(async_std::process::Command),
    Builtin(crate::builtins::Command),
}

impl Command {
    pub fn new(exe: crate::parse::Exe) -> Self {
        crate::builtins::Command::new(exe)
            .map_or_else(Self::new_binary, Self::Builtin)
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn new_binary(exe: crate::parse::Exe) -> Self {
        let mut cmd = async_std::process::Command::new(exe.exe());
        cmd.args(exe.args());
        Self::Binary(cmd)
    }

    pub fn new_builtin(exe: crate::parse::Exe) -> Self {
        crate::builtins::Command::new(exe)
            .map_or_else(|_| todo!(), Self::Builtin)
    }

    pub fn stdin(&mut self, fh: std::fs::File) {
        match self {
            Self::Binary(cmd) => {
                cmd.stdin(fh);
            }
            Self::Builtin(cmd) => {
                cmd.stdin(fh);
            }
        }
    }

    pub fn stdout(&mut self, fh: std::fs::File) {
        match self {
            Self::Binary(cmd) => {
                cmd.stdout(fh);
            }
            Self::Builtin(cmd) => {
                cmd.stdout(fh);
            }
        }
    }

    pub fn stderr(&mut self, fh: std::fs::File) {
        match self {
            Self::Binary(cmd) => {
                cmd.stderr(fh);
            }
            Self::Builtin(cmd) => {
                cmd.stderr(fh);
            }
        }
    }

    pub unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        match self {
            Self::Binary(cmd) => {
                cmd.pre_exec(f);
            }
            Self::Builtin(cmd) => {
                cmd.pre_exec(f);
            }
        }
    }

    pub fn spawn(self, env: &crate::env::Env) -> anyhow::Result<Child> {
        match self {
            Self::Binary(mut cmd) => Ok(Child::Binary(cmd.spawn()?)),
            Self::Builtin(cmd) => Ok(Child::Builtin(cmd.spawn(env)?)),
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
