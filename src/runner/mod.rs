use crate::runner::prelude::*;

mod builtins;
mod command;
pub use command::{Child, Command};
mod prelude;

const PID0: nix::unistd::Pid = nix::unistd::Pid::from_raw(0);

#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Event {
    RunPipeline(usize, (usize, usize)),
    Suspend(usize),
    Exit(Env),
}

struct Stack {
    frames: Vec<Frame>,
}

impl Stack {
    fn new() -> Self {
        Self { frames: vec![] }
    }

    fn push(&mut self, frame: Frame) {
        self.frames.push(frame);
    }

    fn pop(&mut self) -> Frame {
        self.frames.pop().unwrap()
    }

    fn top(&self) -> Option<&Frame> {
        self.frames.last()
    }

    fn top_mut(&mut self) -> Option<&mut Frame> {
        self.frames.last_mut()
    }

    fn current_pc(&self, pc: usize) -> bool {
        match self.top() {
            Some(Frame::If(..)) | None => false,
            Some(Frame::While(_, start) | Frame::For(_, start, _)) => {
                *start == pc
            }
        }
    }

    fn should_execute(&self) -> bool {
        for frame in &self.frames {
            if matches!(
                frame,
                Frame::If(false, ..)
                    | Frame::While(false, ..)
                    | Frame::For(false, ..)
            ) {
                return false;
            }
        }
        true
    }
}

enum Frame {
    If(bool, bool),
    While(bool, usize),
    For(bool, usize, Vec<String>),
}

pub async fn run(
    commands: &str,
    shell_write: Option<&async_std::fs::File>,
) -> anyhow::Result<i32> {
    let mut env = Env::new_from_env()?;
    run_commands(commands, &mut env, shell_write).await?;
    let status = env.latest_status();
    write_event(shell_write, Event::Exit(env)).await?;

    if let Some(signal) = status.signal() {
        nix::sys::signal::raise(signal.try_into().unwrap())?;
    }
    Ok(status.code().unwrap())
}

async fn run_commands(
    commands: &str,
    env: &mut Env,
    shell_write: Option<&async_std::fs::File>,
) -> anyhow::Result<()> {
    let commands = crate::parse::ast::Commands::parse(commands)?;
    let commands = commands.commands();
    let mut pc = 0;
    let mut stack = Stack::new();
    while pc < commands.len() {
        match &commands[pc] {
            crate::parse::ast::Command::Pipeline(pipeline) => {
                if stack.should_execute() {
                    run_pipeline(pipeline.clone(), env, shell_write).await?;
                }
                pc += 1;
            }
            crate::parse::ast::Command::If(pipeline) => {
                let should = stack.should_execute();
                if !stack.current_pc(pc) {
                    stack.push(Frame::If(false, false));
                }
                if should {
                    let status = env.latest_status();
                    run_pipeline(pipeline.clone(), env, shell_write).await?;
                    if let Some(Frame::If(should, found)) = stack.top_mut() {
                        *should = env.latest_status().success();
                        if *should {
                            *found = true;
                        }
                    } else {
                        unreachable!();
                    }
                    env.set_status(status);
                }
                pc += 1;
            }
            crate::parse::ast::Command::While(pipeline) => {
                let should = stack.should_execute();
                if !stack.current_pc(pc) {
                    stack.push(Frame::While(false, pc));
                }
                if should {
                    let status = env.latest_status();
                    run_pipeline(pipeline.clone(), env, shell_write).await?;
                    if let Some(Frame::While(should, _)) = stack.top_mut() {
                        *should = env.latest_status().success();
                    } else {
                        unreachable!();
                    }
                    env.set_status(status);
                }
                pc += 1;
            }
            crate::parse::ast::Command::For(var, list) => {
                let should = stack.should_execute();
                if !stack.current_pc(pc) {
                    stack.push(Frame::For(
                        false,
                        pc,
                        if stack.should_execute() {
                            list.clone()
                                .into_iter()
                                .map(|w| {
                                    w.eval(env).map(IntoIterator::into_iter)
                                })
                                .collect::<Result<Vec<_>, _>>()?
                                .into_iter()
                                .flatten()
                                .collect()
                        } else {
                            vec![]
                        },
                    ));
                }
                if should {
                    if let Some(Frame::For(should, _, list)) = stack.top_mut()
                    {
                        *should = !list.is_empty();
                        if *should {
                            let val = list.remove(0);
                            // XXX i really need to just pick one location and
                            // stick with it instead of trying to keep these
                            // in sync
                            env.set_var(var, &val);
                            std::env::set_var(var, &val);
                        }
                    } else {
                        unreachable!();
                    }
                }
                pc += 1;
            }
            crate::parse::ast::Command::Else(pipeline) => {
                let mut top = stack.pop();
                if stack.should_execute() {
                    if let Frame::If(ref mut should, ref mut found) = top {
                        if *found {
                            *should = false;
                        } else if let Some(pipeline) = pipeline {
                            let status = env.latest_status();
                            run_pipeline(pipeline.clone(), env, shell_write)
                                .await?;
                            *should = env.latest_status().success();
                            if *should {
                                *found = true;
                            }
                            env.set_status(status);
                        } else {
                            *should = true;
                            *found = true;
                        }
                    } else {
                        todo!();
                    }
                }
                stack.push(top);
                pc += 1;
            }
            crate::parse::ast::Command::End => match stack.top() {
                Some(Frame::If(..)) => {
                    stack.pop();
                    pc += 1;
                }
                Some(
                    Frame::While(should, start)
                    | Frame::For(should, start, _),
                ) => {
                    if *should {
                        pc = *start;
                    } else {
                        stack.pop();
                        pc += 1;
                    }
                }
                None => todo!(),
            },
        }
    }
    Ok(())
}

async fn run_pipeline(
    pipeline: crate::parse::ast::Pipeline,
    env: &mut Env,
    shell_write: Option<&async_std::fs::File>,
) -> anyhow::Result<()> {
    write_event(shell_write, Event::RunPipeline(env.idx(), pipeline.span()))
        .await?;
    // Safety: pipelines are run serially, so only one copy of these will ever
    // exist at once. note that reusing a single copy of these at the top
    // level would not be safe, because in the case of a command line like
    // "echo foo; ls", we would pass the stdout fd to the ls process while it
    // is still open here, and may still have data buffered.
    let stdin = unsafe { async_std::fs::File::from_raw_fd(0) };
    let stdout = unsafe { async_std::fs::File::from_raw_fd(1) };
    let stderr = unsafe { async_std::fs::File::from_raw_fd(2) };
    let mut io = builtins::Io::new();
    io.set_stdin(stdin);
    io.set_stdout(stdout);
    io.set_stderr(stderr);

    let pwd = env.pwd().to_path_buf();
    let (children, pg) = spawn_children(pipeline, env, &io)?;
    let status = wait_children(children, pg, env, &io, shell_write).await;
    set_foreground_pg(nix::unistd::getpid())?;
    env.update()?;
    env.set_status(status);
    if env.pwd() != pwd {
        env.set_prev_pwd(pwd);
    }
    Ok(())
}

async fn write_event(
    fh: Option<&async_std::fs::File>,
    event: Event,
) -> anyhow::Result<()> {
    if let Some(mut fh) = fh {
        fh.write_all(&bincode::serialize(&event)?).await?;
        fh.flush().await?;
    }
    Ok(())
}

fn spawn_children<'a>(
    pipeline: crate::parse::ast::Pipeline,
    env: &'a Env,
    io: &builtins::Io,
) -> anyhow::Result<(Vec<Child<'a>>, Option<nix::unistd::Pid>)> {
    let pipeline = pipeline.eval(env)?;
    let mut cmds: Vec<_> = pipeline
        .into_exes()
        .map(|exe| Command::new(exe, io.clone()))
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
    shell_write: Option<&async_std::fs::File>,
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
                        if signal == nix::sys::signal::Signal::SIGINT {
                            if let Err(e) = nix::sys::signal::raise(
                                nix::sys::signal::Signal::SIGINT,
                            ) {
                                bail!(e);
                            }
                        }
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

    // if a background process calls tcsetpgrp, the kernel will send it
    // SIGTTOU which suspends it. if that background process is the session
    // leader and doesn't have SIGTTOU blocked, the kernel will instead just
    // return ENOTTY from the tcsetpgrp call rather than sending a signal to
    // avoid deadlocking the process. therefore, we need to ensure that
    // SIGTTOU is blocked here.

    // Safety: setting a signal handler to SigIgn is always safe
    unsafe {
        nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGTTOU,
            nix::sys::signal::SigHandler::SigIgn,
        )?;
    }
    let res = nix::unistd::tcsetpgrp(pty, pg);
    // Safety: setting a signal handler to SigDfl is always safe
    unsafe {
        nix::sys::signal::signal(
            nix::sys::signal::Signal::SIGTTOU,
            nix::sys::signal::SigHandler::SigDfl,
        )?;
    }
    res?;

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
