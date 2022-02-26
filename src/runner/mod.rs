use crate::runner::prelude::*;

mod builtins;
mod command;
pub use command::{Child, Command};
mod prelude;
mod sys;

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
    shell_write: &mut Option<tokio::fs::File>,
) -> Result<i32> {
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
    shell_write: &mut Option<tokio::fs::File>,
) -> Result<()> {
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
                                .map(|w| async {
                                    w.eval(env)
                                        .await
                                        .map(IntoIterator::into_iter)
                                })
                                .collect::<futures_util::stream::FuturesOrdered<_>>()
                                .try_collect::<Vec<_>>().await?
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
    shell_write: &mut Option<tokio::fs::File>,
) -> Result<()> {
    write_event(shell_write, Event::RunPipeline(env.idx(), pipeline.span()))
        .await?;
    // Safety: pipelines are run serially, so only one copy of these will ever
    // exist at once. note that reusing a single copy of these at the top
    // level would not be safe, because in the case of a command line like
    // "echo foo; ls", we would pass the stdout fd to the ls process while it
    // is still open here, and may still have data buffered.
    let stdin = unsafe { std::fs::File::from_raw_fd(0) };
    let stdout = unsafe { std::fs::File::from_raw_fd(1) };
    let stderr = unsafe { std::fs::File::from_raw_fd(2) };
    let mut io = builtins::Io::new();
    io.set_stdin(stdin);
    io.set_stdout(stdout);
    io.set_stderr(stderr);

    let pwd = env.pwd().to_path_buf();
    let pipeline = pipeline.eval(env).await?;
    let interactive = shell_write.is_some();
    let (children, pg) = spawn_children(pipeline, env, &io, interactive)?;
    let status = wait_children(children, pg, env, shell_write).await;
    if interactive {
        sys::set_foreground_pg(nix::unistd::getpid())?;
    }
    env.update()?;
    env.set_status(status);
    if env.pwd() != pwd {
        env.set_prev_pwd(pwd);
    }
    Ok(())
}

async fn write_event(
    fh: &mut Option<tokio::fs::File>,
    event: Event,
) -> Result<()> {
    if let Some(fh) = fh {
        fh.write_all(&bincode::serialize(&event)?).await?;
        fh.flush().await?;
    }
    Ok(())
}

fn spawn_children(
    pipeline: crate::parse::Pipeline,
    env: &Env,
    io: &builtins::Io,
    interactive: bool,
) -> Result<(Vec<Child>, Option<nix::unistd::Pid>)> {
    let mut cmds: Vec<_> = pipeline
        .into_exes()
        .map(|exe| Command::new(exe, io.clone()))
        .collect();
    for i in 0..(cmds.len() - 1) {
        let (r, w) = sys::pipe()?;
        cmds[i].stdout(w);
        cmds[i + 1].stdin(r);
    }

    let mut children = vec![];
    let mut pg_pid = None;
    for mut cmd in cmds {
        // Safety: setpgid is an async-signal-safe function
        unsafe {
            cmd.pre_exec(move || {
                sys::setpgid_child(pg_pid)?;
                Ok(())
            });
        }
        let child = cmd.spawn(env)?;
        if let Some(id) = child.id() {
            let child_pid = sys::id_to_pid(id);
            sys::setpgid_parent(child_pid, pg_pid)?;
            if pg_pid.is_none() {
                pg_pid = Some(child_pid);
                if interactive {
                    sys::set_foreground_pg(child_pid)?;
                }
            }
        }
        children.push(child);
    }
    Ok((children, pg_pid))
}

async fn wait_children(
    children: Vec<Child>,
    pg: Option<nix::unistd::Pid>,
    env: &Env,
    shell_write: &mut Option<tokio::fs::File>,
) -> std::process::ExitStatus {
    enum Res {
        Child(nix::Result<nix::sys::wait::WaitStatus>),
        Builtin((Result<std::process::ExitStatus>, bool)),
    }

    macro_rules! bail {
        ($e:expr) => {
            eprintln!("nbsh: {}\n", $e);
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
            (sys::id_to_pid(child.id().unwrap()), (child, i == count - 1))
        })
        .collect();
    let mut builtin_count = builtins.len();
    let builtins: futures_util::stream::FuturesUnordered<_> =
        builtins
            .into_iter()
            .map(|(i, child)| async move {
                (child.status().await, i == count - 1)
            })
            .collect();

    let (wait_w, wait_r) = tokio::sync::mpsc::unbounded_channel();
    if let Some(pg) = pg {
        tokio::task::spawn_blocking(move || loop {
            let res = nix::sys::wait::waitpid(
                sys::neg_pid(pg),
                Some(nix::sys::wait::WaitPidFlag::WUNTRACED),
            );
            match wait_w.send(res) {
                Ok(_) => {}
                Err(tokio::sync::mpsc::error::SendError(res)) => {
                    // we should never drop wait_r while there are still valid
                    // things to read
                    assert!(res.is_err());
                    break;
                }
            }
        });
    }

    let mut stream: futures_util::stream::SelectAll<_> = [
        tokio_stream::wrappers::UnboundedReceiverStream::new(wait_r)
            .map(Res::Child)
            .boxed(),
        builtins.map(Res::Builtin).boxed(),
    ]
    .into_iter()
    .collect();
    while let Some(res) = stream.next().await {
        match res {
            Res::Child(Ok(status)) => {
                match status {
                    // we can't call child.status() here to unify these
                    // branches because our waitpid call already collected the
                    // status
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
            }
            Res::Child(Err(e)) => {
                bail!(e);
            }
            Res::Builtin((Ok(status), last)) => {
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
                builtin_count -= 1;
            }
            Res::Builtin((Err(e), _)) => {
                bail!(e);
            }
        }

        if children.is_empty() && builtin_count == 0 {
            break;
        }
    }

    final_status.unwrap()
}
