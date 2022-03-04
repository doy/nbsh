use crate::shell::prelude::*;

pub struct Job {
    state: std::sync::Arc<std::sync::Mutex<State>>,
    start_time: time::OffsetDateTime,
    start_instant: std::time::Instant,
}

impl Job {
    pub fn new(
        cmdline: &str,
        env: Env,
        pts: &pty_process::Pts,
        event_w: crate::shell::event::Writer,
    ) -> Result<Self> {
        let start_time = time::OffsetDateTime::now_utc();
        let start_instant = std::time::Instant::now();
        let (child, fh) = spawn_command(cmdline, &env, pts)?;
        let state = std::sync::Arc::new(std::sync::Mutex::new(
            State::Running((0, 0)),
        ));
        tokio::spawn(Self::task(
            child,
            fh,
            std::sync::Arc::clone(&state),
            env,
            event_w,
        ));
        Ok(Self {
            state,
            start_time,
            start_instant,
        })
    }

    pub fn start_time(&self) -> &time::OffsetDateTime {
        &self.start_time
    }

    pub fn start_instant(&self) -> &std::time::Instant {
        &self.start_instant
    }

    pub fn with_state<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        let state = self.state.lock().unwrap();
        f(&state)
    }

    pub fn with_state_mut<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self.state.lock().unwrap();
        f(&mut state)
    }

    pub fn lock_state(&self) -> std::sync::MutexGuard<State> {
        self.state.lock().unwrap()
    }

    pub fn running(&self) -> bool {
        self.with_state(|state| matches!(state, State::Running(..)))
    }

    pub fn set_span(&self, new_span: (usize, usize)) {
        self.with_state_mut(|state| {
            if let State::Running(span) = state {
                *span = new_span;
            }
        });
    }

    async fn task(
        mut child: tokio::process::Child,
        fh: std::fs::File,
        state: std::sync::Arc<std::sync::Mutex<State>>,
        env: Env,
        event_w: crate::shell::event::Writer,
    ) {
        enum Res {
            Read(crate::runner::Event),
            Exit(std::io::Result<std::process::ExitStatus>),
        }

        let (read_w, read_r) = tokio::sync::mpsc::unbounded_channel();
        tokio::task::spawn_blocking(move || loop {
            let event = bincode::deserialize_from(&fh);
            match event {
                Ok(event) => {
                    read_w.send(event).unwrap();
                }
                Err(e) => {
                    match &*e {
                        bincode::ErrorKind::Io(io_e) => {
                            assert!(
                                io_e.kind()
                                    == std::io::ErrorKind::UnexpectedEof
                            );
                        }
                        e => {
                            panic!("{}", e);
                        }
                    }
                    break;
                }
            }
        });

        let mut stream: futures_util::stream::SelectAll<_> = [
            tokio_stream::wrappers::UnboundedReceiverStream::new(read_r)
                .map(Res::Read)
                .boxed(),
            futures_util::stream::once(child.wait())
                .map(Res::Exit)
                .boxed(),
        ]
        .into_iter()
        .collect();
        let mut exit_status = None;
        let mut new_env = None;
        while let Some(res) = stream.next().await {
            match res {
                Res::Read(event) => match event {
                    crate::runner::Event::RunPipeline(new_span) => {
                        // we could just update the span in place here, but we
                        // do this as an event so that we can also trigger a
                        // refresh
                        event_w.send(Event::ChildRunPipeline(
                            env.idx(),
                            new_span,
                        ));
                    }
                    crate::runner::Event::Suspend => {
                        event_w.send(Event::ChildSuspend(env.idx()));
                    }
                    crate::runner::Event::Exit(env) => {
                        new_env = Some(env);
                    }
                },
                Res::Exit(status) => {
                    exit_status = Some(status.unwrap());
                }
            }
        }
        *state.lock().unwrap() =
            State::Exited(ExitInfo::new(exit_status.unwrap()));
        event_w.send(Event::ChildExit(env.idx(), new_env));
    }
}

pub enum State {
    Running((usize, usize)),
    Exited(ExitInfo),
}

impl State {
    pub fn exit_info(&self) -> Option<&ExitInfo> {
        match self {
            Self::Running(_) => None,
            Self::Exited(exit_info) => Some(exit_info),
        }
    }

    pub fn running(&self) -> bool {
        self.exit_info().is_none()
    }
}

pub struct ExitInfo {
    status: std::process::ExitStatus,
    instant: std::time::Instant,
}

impl ExitInfo {
    fn new(status: std::process::ExitStatus) -> Self {
        Self {
            status,
            instant: std::time::Instant::now(),
        }
    }

    pub fn status(&self) -> std::process::ExitStatus {
        self.status
    }

    pub fn instant(&self) -> &std::time::Instant {
        &self.instant
    }
}

fn spawn_command(
    cmdline: &str,
    env: &Env,
    pts: &pty_process::Pts,
) -> Result<(tokio::process::Child, std::fs::File)> {
    let mut cmd = pty_process::Command::new(std::env::current_exe()?);
    cmd.args(&["-c", cmdline, "--status-fd", "3"]);
    env.apply(&mut cmd);
    let (from_r, from_w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    // Safety: from_r was just opened above and is not used anywhere else
    let fh = unsafe { std::fs::File::from_raw_fd(from_r) };
    // Safety: dup2 is an async-signal-safe function
    unsafe {
        cmd.pre_exec(move || {
            nix::unistd::dup2(from_w, 3)?;
            Ok(())
        });
    }
    let child = cmd.spawn(pts)?;
    nix::unistd::close(from_w)?;
    Ok((child, fh))
}
