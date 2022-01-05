use async_std::io::{ReadExt as _, WriteExt as _};
use async_std::stream::StreamExt as _;
use futures_lite::future::FutureExt as _;
use std::os::unix::io::{FromRawFd as _, IntoRawFd as _};
use std::os::unix::process::ExitStatusExt as _;

const PID0: nix::unistd::Pid = nix::unistd::Pid::from_raw(0);

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Event {
    Suspend(usize),
    Exit(crate::env::Env),
}

mod command;
pub use command::{Child, Command};

pub async fn run() -> anyhow::Result<i32> {
    let mut env = read_data().await?;
    run_with_env(&mut env).await?;
    let status = *env.latest_status();
    let pwd = std::env::current_dir()?;
    env.set_current_dir(pwd);
    write_event(Event::Exit(env)).await?;
    if let Some(signal) = status.signal() {
        nix::sys::signal::raise(signal.try_into().unwrap())?;
    }
    Ok(status.code().unwrap())
}

async fn run_with_env(env: &mut crate::env::Env) -> anyhow::Result<()> {
    let pipeline = crate::parse::Pipeline::parse(env.pipeline().unwrap())?;
    let (children, pg) = spawn_children(pipeline, env)?;
    let status = wait_children(children, pg, env).await;
    env.set_status(status);
    Ok(())
}

async fn read_data() -> anyhow::Result<crate::env::Env> {
    // Safety: this code is only called by crate::history::run_pipeline, which
    // passes data through on fd 3, and which will not spawn this process
    // unless the pipe was successfully opened on that fd
    let mut fd3 = unsafe { async_std::fs::File::from_raw_fd(3) };
    let mut data = vec![];
    fd3.read_to_end(&mut data).await?;
    let env = crate::env::Env::from_bytes(&data);
    Ok(env)
}

async fn write_event(event: Event) -> anyhow::Result<()> {
    let mut fd4 = unsafe { async_std::fs::File::from_raw_fd(4) };
    fd4.write_all(&bincode::serialize(&event)?).await?;
    fd4.flush().await?;
    let _ = fd4.into_raw_fd();
    Ok(())
}

fn spawn_children(
    pipeline: crate::parse::Pipeline,
    env: &crate::env::Env,
) -> anyhow::Result<(Vec<Child>, Option<nix::unistd::Pid>)> {
    let mut cmds: Vec<_> = pipeline.into_exes().map(Command::new).collect();
    for i in 0..(cmds.len() - 1) {
        let (r, w) = pipe()?;
        cmds[i].stdout(w);
        cmds[i + 1].stdin(r);
    }

    let mut children = vec![];
    let mut pg_pid = None;
    for mut cmd in cmds {
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
    Ok((children, pg_pid))
}

async fn wait_children(
    children: Vec<Child<'_>>,
    pg: Option<nix::unistd::Pid>,
    env: &crate::env::Env,
) -> std::process::ExitStatus {
    enum Res {
        Child(nix::Result<nix::sys::wait::WaitStatus>),
        Builtin(Option<(anyhow::Result<std::process::ExitStatus>, bool)>),
    }

    macro_rules! bail {
        ($msg:expr) => {
            eprintln!("{}", $msg);
            return std::process::ExitStatus::from_raw(1 << 8);
        };
    }

    let mut final_status = None;

    let count = children.len();
    let (children, builtins): (Vec<_>, Vec<_>) = children
        .into_iter()
        .enumerate()
        .partition(|(_, child)| child.id().is_some());
    let mut children: std::collections::HashMap<_, _> = children
        .into_iter()
        .map(|(i, child)| {
            (id_to_pid(child.id().unwrap()), (child, i == count - 1))
        })
        .collect();
    let mut builtins: futures_util::stream::FuturesUnordered<_> =
        builtins
            .into_iter()
            .map(|(i, child)| async move {
                (child.status().await, i == count - 1)
            })
            .collect();
    loop {
        if children.is_empty() && builtins.is_empty() {
            break;
        }

        let child = async {
            Res::Child(if let Some(pg) = pg {
                blocking::unblock(move || {
                    nix::sys::wait::waitpid(
                        neg_pid(pg),
                        Some(nix::sys::wait::WaitPidFlag::WUNTRACED),
                    )
                })
                .await
            } else {
                std::future::pending().await
            })
        };
        let builtin = async {
            Res::Builtin(if builtins.is_empty() {
                std::future::pending().await
            } else {
                builtins.next().await
            })
        };
        match child.race(builtin).await {
            Res::Child(Ok(status)) => match status {
                // we can't call child.status() here to unify these branches
                // because our waitpid call already collected the status
                nix::sys::wait::WaitStatus::Exited(pid, code) => {
                    let (_, last) = children.remove(&pid).unwrap();
                    if last {
                        final_status = Some(
                            std::process::ExitStatus::from_raw(code << 8),
                        );
                    }
                }
                nix::sys::wait::WaitStatus::Signaled(pid, signal, _) => {
                    let (_, last) = children.remove(&pid).unwrap();
                    // this conversion is safe because the Signal enum is
                    // repr(i32)
                    #[allow(clippy::as_conversions)]
                    if last {
                        final_status = Some(
                            std::process::ExitStatus::from_raw(signal as i32),
                        );
                    }
                }
                nix::sys::wait::WaitStatus::Stopped(pid, signal) => {
                    if signal == nix::sys::signal::Signal::SIGTSTP {
                        if let Err(e) =
                            write_event(Event::Suspend(env.idx())).await
                        {
                            bail!(e);
                        }
                        if let Err(e) = nix::sys::signal::kill(
                            pid,
                            nix::sys::signal::Signal::SIGCONT,
                        ) {
                            bail!(e);
                        }
                    }
                }
                _ => {}
            },
            Res::Child(Err(e)) => {
                bail!(e);
            }
            Res::Builtin(Some((Ok(status), last))) => {
                // this conversion is safe because the Signal enum is
                // repr(i32)
                #[allow(clippy::as_conversions)]
                if status.signal()
                    == Some(nix::sys::signal::Signal::SIGINT as i32)
                {
                    if let Err(e) = nix::sys::signal::raise(
                        nix::sys::signal::Signal::SIGINT,
                    ) {
                        bail!(e);
                    }
                }
                if last {
                    final_status = Some(status);
                }
            }
            Res::Builtin(Some((Err(e), _))) => {
                bail!(e);
            }
            Res::Builtin(None) => {}
        }
    }

    final_status.unwrap()
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
