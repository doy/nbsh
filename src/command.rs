use async_std::io::ReadExt as _;
use async_std::os::unix::process::CommandExt as _;
use async_std::stream::StreamExt as _;
use std::os::unix::io::FromRawFd as _;
use std::os::unix::process::ExitStatusExt as _;

const PID0: nix::unistd::Pid = nix::unistd::Pid::from_raw(0);

#[derive(Clone)]
pub struct Env {
    latest_status: async_std::process::ExitStatus,
}

impl Env {
    pub fn new(code: i32) -> Self {
        Self {
            latest_status: async_std::process::ExitStatus::from_raw(
                code << 8,
            ),
        }
    }

    pub fn set_status(&mut self, status: async_std::process::ExitStatus) {
        self.latest_status = status;
    }

    pub fn latest_status(&self) -> &async_std::process::ExitStatus {
        &self.latest_status
    }
}

pub enum Command {
    Binary(async_std::process::Command),
    Builtin(crate::builtins::Command),
}

impl Command {
    pub fn new(exe: &crate::parse::Exe) -> Self {
        crate::builtins::Command::new(exe)
            .map_or_else(|| Self::new_binary(exe), Self::Builtin)
    }

    pub fn new_binary(exe: &crate::parse::Exe) -> Self {
        let mut cmd = async_std::process::Command::new(exe.exe());
        cmd.args(exe.args());
        Self::Binary(cmd)
    }

    pub fn new_builtin(exe: &crate::parse::Exe) -> Self {
        crate::builtins::Command::new(exe)
            .map_or_else(|| todo!(), Self::Builtin)
    }

    fn stdin(&mut self, fh: std::fs::File) {
        match self {
            Self::Binary(cmd) => {
                cmd.stdin(fh);
            }
            Self::Builtin(cmd) => {
                cmd.stdin(fh);
            }
        }
    }

    fn stdout(&mut self, fh: std::fs::File) {
        match self {
            Self::Binary(cmd) => {
                cmd.stdout(fh);
            }
            Self::Builtin(cmd) => {
                cmd.stdout(fh);
            }
        }
    }

    fn stderr(&mut self, fh: std::fs::File) {
        match self {
            Self::Binary(cmd) => {
                cmd.stderr(fh);
            }
            Self::Builtin(cmd) => {
                cmd.stderr(fh);
            }
        }
    }

    unsafe fn pre_exec<F>(&mut self, f: F)
    where
        F: 'static + FnMut() -> std::io::Result<()> + Send + Sync,
    {
        match self {
            Self::Binary(cmd) => {
                cmd.pre_exec(f);
            }
            Self::Builtin(_) => {}
        }
    }

    pub fn spawn(self, env: &Env) -> anyhow::Result<Child> {
        match self {
            Self::Binary(mut cmd) => {
                let child = cmd.spawn()?;
                Ok(Child::Binary(child))
            }
            Self::Builtin(cmd) => Ok(Child::Builtin(cmd.spawn(env)?)),
        }
    }
}

pub enum Child {
    Binary(async_std::process::Child),
    Builtin(crate::builtins::Child),
}

impl Child {
    pub fn id(&self) -> Option<u32> {
        match self {
            Self::Binary(child) => Some(child.id()),
            Self::Builtin(child) => child.id(),
        }
    }

    #[async_recursion::async_recursion]
    pub async fn status(self) -> anyhow::Result<std::process::ExitStatus> {
        match self {
            Self::Binary(child) => Ok(child.status_no_drop().await?),
            Self::Builtin(child) => Ok(child.status().await?),
        }
    }
}

pub async fn run() -> anyhow::Result<i32> {
    let (code, pipeline) = read_data().await?;
    let env = Env::new(code);
    let children = spawn_children(&pipeline, &env)?;
    let count = children.len();

    let mut children: futures_util::stream::FuturesUnordered<_> =
        children
            .into_iter()
            .enumerate()
            .map(|(i, child)| async move {
                (child.status().await, i == count - 1)
            })
            .collect();
    let mut final_status = None;
    while let Some((status, last)) = children.next().await {
        let status = status.unwrap_or_else(|_| {
            async_std::process::ExitStatus::from_raw(1 << 8)
        });
        // this conversion is safe because the Signal enum is repr(i32)
        #[allow(clippy::as_conversions)]
        if status.signal() == Some(nix::sys::signal::Signal::SIGINT as i32) {
            nix::sys::signal::raise(nix::sys::signal::Signal::SIGINT)?;
        }
        if last {
            final_status = Some(status);
        }
    }

    let final_status = final_status.unwrap();
    if let Some(signal) = final_status.signal() {
        nix::sys::signal::raise(signal.try_into().unwrap())?;
    }
    Ok(final_status.code().unwrap())
}

async fn read_data() -> anyhow::Result<(i32, crate::parse::Pipeline)> {
    // Safety: this code is only called by crate::history::run_pipeline, which
    // passes data through on fd 3, and which will not spawn this process
    // unless the pipe was successfully opened on that fd
    let mut fd3 = unsafe { async_std::fs::File::from_raw_fd(3) };
    let mut be_bytes = [0; 4];
    fd3.read_exact(&mut be_bytes).await?;
    let code = i32::from_be_bytes(be_bytes);
    let mut pipeline = String::new();
    fd3.read_to_string(&mut pipeline).await?;
    let ast = crate::parse::Pipeline::parse(&pipeline)?;
    Ok((code, ast))
}

fn spawn_children(
    pipeline: &crate::parse::Pipeline,
    env: &Env,
) -> anyhow::Result<Vec<Child>> {
    let mut cmds: Vec<_> = pipeline.exes().iter().map(Command::new).collect();
    for i in 0..(cmds.len() - 1) {
        let (r, w) = pipe()?;
        cmds[i].stdout(w);
        cmds[i + 1].stdin(r);
    }

    let mut children = vec![];
    let mut pg_pid = None;
    for mut cmd in cmds.drain(..) {
        // Safety: setpgid is an async-signal-safe function
        unsafe {
            cmd.pre_exec(move || {
                setpgid_child(pg_pid)?;
                Ok(())
            });
        }
        let child = cmd.spawn(env)?;
        if let Some(id) = child.id() {
            let child_pid = id_to_pid(id);
            setpgid_parent(child_pid, pg_pid)?;
            if pg_pid.is_none() {
                pg_pid = Some(child_pid);
                set_foreground_pg(child_pid)?;
            }
        }
        children.push(child);
    }
    Ok(children)
}

fn pipe() -> anyhow::Result<(std::fs::File, std::fs::File)> {
    let (r, w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    // Safety: these file descriptors were just returned by pipe2 above, which
    // means they must be valid otherwise that call would have returned an
    // error
    Ok((unsafe { std::fs::File::from_raw_fd(r) }, unsafe {
        std::fs::File::from_raw_fd(w)
    }))
}

fn set_foreground_pg(pg: nix::unistd::Pid) -> anyhow::Result<()> {
    let pty = nix::fcntl::open(
        "/dev/tty",
        nix::fcntl::OFlag::empty(),
        nix::sys::stat::Mode::empty(),
    )?;
    nix::unistd::tcsetpgrp(pty, pg)?;
    nix::unistd::close(pty)?;
    nix::sys::signal::kill(neg_pid(pg), nix::sys::signal::Signal::SIGCONT)
        .or_else(|e| {
            // the process group has already exited
            if e == nix::errno::Errno::ESRCH {
                Ok(())
            } else {
                Err(e)
            }
        })?;
    Ok(())
}

fn setpgid_child(pg: Option<nix::unistd::Pid>) -> std::io::Result<()> {
    nix::unistd::setpgid(PID0, pg.unwrap_or(PID0))?;
    Ok(())
}

fn setpgid_parent(
    pid: nix::unistd::Pid,
    pg: Option<nix::unistd::Pid>,
) -> anyhow::Result<()> {
    nix::unistd::setpgid(pid, pg.unwrap_or(PID0)).or_else(|e| {
        // EACCES means that the child already called exec, but if it did,
        // then it also must have already called setpgid itself, so we don't
        // care. ESRCH means that the process already exited, which is similar
        if e == nix::errno::Errno::EACCES || e == nix::errno::Errno::ESRCH {
            Ok(())
        } else {
            Err(e)
        }
    })?;
    Ok(())
}

fn id_to_pid(id: u32) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(id.try_into().unwrap())
}

fn neg_pid(pid: nix::unistd::Pid) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(-pid.as_raw())
}
