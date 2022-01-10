use crate::runner::prelude::*;

pub struct Command {
    inner: Inner,
    exe: std::path::PathBuf,
    redirects: Vec<crate::parse::Redirect>,
    pre_exec: Option<
        Box<dyn FnMut() -> std::io::Result<()> + Send + Sync + 'static>,
    >,
}
impl Command {
    pub fn new(exe: crate::parse::Exe, io: super::builtins::Io) -> Self {
        let exe_path = exe.exe().to_path_buf();
        let redirects = exe.redirects().to_vec();
        Self {
            inner: super::builtins::Command::new(exe, io).map_or_else(
                |exe| Self::new_binary(exe).inner,
                Inner::Builtin,
            ),
            exe: exe_path,
            redirects,
            pre_exec: None,
        }
    }

    #[allow(clippy::needless_pass_by_value)]
    pub fn new_binary(exe: crate::parse::Exe) -> Self {
        let exe_path = exe.exe().to_path_buf();
        let redirects = exe.redirects().to_vec();
        let mut cmd = async_std::process::Command::new(exe.exe());
        cmd.args(exe.args());
        Self {
            inner: Inner::Binary(cmd),
            exe: exe_path,
            redirects,
            pre_exec: None,
        }
    }

    pub fn new_builtin(
        exe: crate::parse::Exe,
        io: super::builtins::Io,
    ) -> Self {
        let exe_path = exe.exe().to_path_buf();
        let redirects = exe.redirects().to_vec();
        Self {
            inner: super::builtins::Command::new(exe, io)
                .map_or_else(|_| todo!(), Inner::Builtin),
            exe: exe_path,
            redirects,
            pre_exec: None,
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
        self.pre_exec = Some(Box::new(f));
    }

    pub fn spawn(self, env: &Env) -> anyhow::Result<Child> {
        let Self {
            inner,
            exe,
            redirects,
            pre_exec,
        } = self;

        #[allow(clippy::as_conversions)]
        let pre_exec = pre_exec.map_or_else(
            || {
                let redirects = redirects.clone();
                Box::new(move || {
                    apply_redirects(&redirects)?;
                    Ok(())
                })
                    as Box<dyn FnMut() -> std::io::Result<()> + Send + Sync>
            },
            |mut pre_exec| {
                let redirects = redirects.clone();
                Box::new(move || {
                    apply_redirects(&redirects)?;
                    pre_exec()?;
                    Ok(())
                })
            },
        );
        match inner {
            Inner::Binary(mut cmd) => {
                // Safety: open, dup2, and close are async-signal-safe
                // functions
                unsafe { cmd.pre_exec(pre_exec) };
                Ok(Child::Binary(cmd.spawn().map_err(|e| {
                    anyhow::anyhow!(
                        "{}: {}",
                        crate::format::io_error(&e),
                        exe.display()
                    )
                })?))
            }
            Inner::Builtin(mut cmd) => {
                // Safety: open, dup2, and close are async-signal-safe
                // functions
                unsafe { cmd.pre_exec(pre_exec) };
                cmd.apply_redirects(&redirects);
                Ok(Child::Builtin(cmd.spawn(env)?))
            }
        }
    }
}

pub enum Inner {
    Binary(async_std::process::Command),
    Builtin(super::builtins::Command),
}

pub enum Child<'a> {
    Binary(async_std::process::Child),
    Builtin(super::builtins::Child<'a>),
}

impl<'a> Child<'a> {
    pub fn id(&self) -> Option<u32> {
        match self {
            Self::Binary(child) => Some(child.id()),
            Self::Builtin(child) => child.id(),
        }
    }

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

fn apply_redirects(
    redirects: &[crate::parse::Redirect],
) -> std::io::Result<()> {
    for redirect in redirects {
        match &redirect.to {
            crate::parse::RedirectTarget::Fd(fd) => {
                nix::unistd::dup2(*fd, redirect.from)?;
            }
            crate::parse::RedirectTarget::File(path) => {
                let fd = redirect.dir.open(path)?;
                if fd != redirect.from {
                    nix::unistd::dup2(fd, redirect.from)?;
                    nix::unistd::close(fd)?;
                }
            }
        }
    }
    Ok(())
}
