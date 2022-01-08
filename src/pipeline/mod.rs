use crate::pipeline::prelude::*;

const PID0: nix::unistd::Pid = nix::unistd::Pid::from_raw(0);

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Event {
    Suspend(usize),
    Exit(Env),
}

mod builtins;
mod command;
pub use command::{Child, Command};
mod prelude;

pub async fn run() -> anyhow::Result<i32> {
    // Safety: we don't create File instances for or read/write data on fds
    // 0-4 anywhere else
    let stdin = unsafe { async_std::fs::File::from_raw_fd(0) };
    let stdout = unsafe { async_std::fs::File::from_raw_fd(1) };
    let stderr = unsafe { async_std::fs::File::from_raw_fd(2) };
    let shell_read = unsafe { async_std::fs::File::from_raw_fd(3) };
    let shell_write = unsafe { async_std::fs::File::from_raw_fd(4) };
    cloexec(3)?;
    cloexec(4)?;

    let mut io = builtins::Io::new();
    io.set_stdin(stdin);
    io.set_stdout(stdout);
    io.set_stderr(stderr);

    let (pipeline, mut env) = read_data(shell_read).await?;
    run_with_env(&pipeline, &mut env, &io, &shell_write).await?;
    let status = *env.latest_status();

    env.update()?;
    write_event(&shell_write, Event::Exit(env)).await?;

    if let Some(signal) = status.signal() {
        nix::sys::signal::raise(signal.try_into().unwrap())?;
    }
    Ok(status.code().unwrap())
}

async fn run_with_env(
    pipeline: &str,
    env: &mut Env,
    io: &builtins::Io,
    shell_write: &async_std::fs::File,
) -> anyhow::Result<()> {
    let pipeline = crate::parse::ast::Pipeline::parse(pipeline)?;
    let (children, pg) = spawn_children(pipeline, env, io)?;
    let status = wait_children(children, pg, env, io, shell_write).await;
    env.set_status(status);
    Ok(())
}

async fn read_data(
    mut fh: async_std::fs::File,
) -> anyhow::Result<(String, Env)> {
    let mut data = vec![];
    fh.read_to_end(&mut data).await?;
    let pipeline = bincode::deserialize(&data).unwrap();
    let len: usize = bincode::serialized_size(&pipeline)
        .unwrap()
        .try_into()
        .unwrap();
    let env = Env::from_bytes(&data[len..]);
    Ok((pipeline, env))
}

async fn write_event(
    mut fh: &async_std::fs::File,
    event: Event,
) -> anyhow::Result<()> {
    fh.write_all(&bincode::serialize(&event)?).await?;
    fh.flush().await?;
    Ok(())
}

fn spawn_children<'a>(
    pipeline: crate::parse::ast::Pipeline,
    env: &'a Env,
    io: &builtins::Io,
) -> anyhow::Result<(Vec<Child<'a>>, Option<nix::unistd::Pid>)> {
    let pipeline = pipeline.eval(env);
    let mut cmds: Vec<_> = pipeline
        .into_exes()
        .map(|exe| Command::new_with_io(exe, io.clone()))
        .collect();
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
    env: &Env,
    io: &builtins::Io,
    shell_write: &async_std::fs::File,
) -> std::process::ExitStatus {
    enum Res {
        Child(nix::Result<nix::sys::wait::WaitStatus>),
        Builtin(Option<(anyhow::Result<std::process::ExitStatus>, bool)>),
    }

    macro_rules! bail {
        ($e:expr) => {
            // if writing to stderr is not possible, we still want to exit
            // normally with a failure exit code
            #[allow(clippy::let_underscore_drop)]
            let _ =
                io.write_stderr(format!("nbsh: {}\n", $e).as_bytes()).await;
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

    let (wait_w, wait_r) = async_std::channel::unbounded();
    let new_wait = move || {
        if let Some(pg) = pg {
            let wait_w = wait_w.clone();
            async_std::task::spawn(async move {
                let res = blocking::unblock(move || {
                    nix::sys::wait::waitpid(
                        neg_pid(pg),
                        Some(nix::sys::wait::WaitPidFlag::WUNTRACED),
                    )
                })
                .await;
                if wait_w.is_closed() {
                    // we shouldn't be able to drop real process terminations
                    assert!(res.is_err());
                } else {
                    wait_w.send(res).await.unwrap();
                }
            });
        }
    };

    new_wait();
    loop {
        if children.is_empty() && builtins.is_empty() {
            break;
        }

        let child = async { Res::Child(wait_r.recv().await.unwrap()) };
        let builtin = async {
            Res::Builtin(if builtins.is_empty() {
                std::future::pending().await
            } else {
                builtins.next().await
            })
        };
        match child.race(builtin).await {
            Res::Child(Ok(status)) => {
                match status {
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
                            final_status =
                                Some(std::process::ExitStatus::from_raw(
                                    signal as i32,
                                ));
                        }
                    }
                    nix::sys::wait::WaitStatus::Stopped(pid, signal) => {
                        if signal == nix::sys::signal::Signal::SIGTSTP {
                            if let Err(e) = write_event(
                                shell_write,
                                Event::Suspend(env.idx()),
                            )
                            .await
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
                }
                new_wait();
            }
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
    // Safety: these file descriptors were just returned by pipe2 above, and
    // are only available in this function, so nothing else can be accessing
    // them
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

fn cloexec(fd: std::os::unix::io::RawFd) -> anyhow::Result<()> {
    nix::fcntl::fcntl(
        fd,
        nix::fcntl::FcntlArg::F_SETFD(nix::fcntl::FdFlag::FD_CLOEXEC),
    )?;
    Ok(())
}

fn id_to_pid(id: u32) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(id.try_into().unwrap())
}

fn neg_pid(pid: nix::unistd::Pid) -> nix::unistd::Pid {
    nix::unistd::Pid::from_raw(-pid.as_raw())
}
