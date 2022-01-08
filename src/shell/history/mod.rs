use crate::shell::prelude::*;

mod entry;
pub use entry::Entry;
mod pty;

pub struct History {
    size: (u16, u16),
    entries: Vec<async_std::sync::Arc<async_std::sync::Mutex<Entry>>>,
    scroll_pos: usize,
}

impl History {
    pub fn new() -> Self {
        Self {
            size: (24, 80),
            entries: vec![],
            scroll_pos: 0,
        }
    }

    // render always happens on the main task
    #[allow(clippy::future_not_send)]
    pub async fn render(
        &self,
        out: &mut impl textmode::Textmode,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
        offset: time::UtcOffset,
    ) -> anyhow::Result<()> {
        let mut used_lines = repl_lines;
        let mut cursor = None;
        for (idx, entry) in
            self.visible(repl_lines, focus, scrolling).await.rev()
        {
            let focused = focus.map_or(false, |focus| idx == focus);
            used_lines +=
                entry.lines(self.entry_count(), focused && !scrolling);
            out.move_to(
                (usize::from(self.size.0) - used_lines).try_into().unwrap(),
                0,
            );
            entry.render(
                out,
                idx,
                self.entry_count(),
                self.size,
                focused,
                scrolling,
                offset,
            );
            if focused && !scrolling {
                cursor = Some((
                    out.screen().cursor_position(),
                    out.screen().hide_cursor(),
                ));
            }
        }
        if let Some((pos, hide)) = cursor {
            out.move_to(pos.0, pos.1);
            out.hide_cursor(hide);
        }
        Ok(())
    }

    // render always happens on the main task
    #[allow(clippy::future_not_send)]
    pub async fn render_fullscreen(
        &self,
        out: &mut impl textmode::Textmode,
        idx: usize,
    ) {
        let mut entry = self.entries[idx].lock_arc().await;
        entry.render_fullscreen(out);
    }

    pub async fn send_input(&mut self, idx: usize, input: Vec<u8>) {
        self.entry(idx).await.send_input(input).await;
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        for entry in &self.entries {
            entry.lock_arc().await.resize(size).await;
        }
    }

    pub async fn run(
        &mut self,
        ast: crate::parse::ast::Commands,
        env: &Env,
        event_w: async_std::channel::Sender<Event>,
    ) -> anyhow::Result<usize> {
        let (input_w, input_r) = async_std::channel::unbounded();
        let (resize_w, resize_r) = async_std::channel::unbounded();

        let entry = async_std::sync::Arc::new(async_std::sync::Mutex::new(
            Entry::new(
                ast.input_string().to_string(),
                env.clone(),
                self.size,
                input_w,
                resize_w,
            ),
        ));
        run_commands(
            ast,
            async_std::sync::Arc::clone(&entry),
            env.clone(),
            input_r,
            resize_r,
            event_w,
        );

        self.entries.push(entry);
        Ok(self.entries.len() - 1)
    }

    pub async fn parse_error(
        &mut self,
        e: crate::parse::Error,
        env: &Env,
        event_w: async_std::channel::Sender<Event>,
    ) -> anyhow::Result<usize> {
        // XXX would be great to not have to do this
        let (input_w, input_r) = async_std::channel::unbounded();
        let (resize_w, resize_r) = async_std::channel::unbounded();
        input_w.close();
        input_r.close();
        resize_w.close();
        resize_r.close();

        let err_str = format!("{}", e);
        let entry = async_std::sync::Arc::new(async_std::sync::Mutex::new(
            Entry::new(
                e.into_input(),
                env.clone(),
                self.size,
                input_w,
                resize_w,
            ),
        ));
        self.entries.push(async_std::sync::Arc::clone(&entry));

        let mut entry = entry.lock_arc().await;
        entry.process(err_str.replace('\n', "\r\n").as_bytes());
        let mut env = env.clone();
        env.set_status(async_std::process::ExitStatus::from_raw(1 << 8));
        entry.finish(env, event_w).await;

        Ok(self.entries.len() - 1)
    }

    pub async fn entry(
        &self,
        idx: usize,
    ) -> async_std::sync::MutexGuardArc<Entry> {
        self.entries[idx].lock_arc().await
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub async fn make_focus_visible(
        &mut self,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
    ) {
        if self.entries.is_empty() || focus.is_none() {
            return;
        }
        let focus = focus.unwrap();

        let mut done = false;
        while focus
            < self
                .visible(repl_lines, Some(focus), scrolling)
                .await
                .map(|(idx, _)| idx)
                .next()
                .unwrap()
        {
            self.scroll_pos += 1;
            done = true;
        }
        if done {
            return;
        }

        while focus
            > self
                .visible(repl_lines, Some(focus), scrolling)
                .await
                .map(|(idx, _)| idx)
                .last()
                .unwrap()
        {
            self.scroll_pos -= 1;
        }
    }

    async fn visible(
        &self,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
    ) -> VisibleEntries {
        let mut iter = VisibleEntries::new();
        if self.entries.is_empty() {
            return iter;
        }

        let mut used_lines = repl_lines;
        for (idx, entry) in
            self.entries.iter().enumerate().rev().skip(self.scroll_pos)
        {
            let entry = entry.lock_arc().await;
            let focused = focus.map_or(false, |focus| idx == focus);
            used_lines +=
                entry.lines(self.entry_count(), focused && !scrolling);
            if used_lines > usize::from(self.size.0) {
                break;
            }
            iter.add(idx, entry);
        }
        iter
    }
}

struct VisibleEntries {
    entries: std::collections::VecDeque<(
        usize,
        async_std::sync::MutexGuardArc<Entry>,
    )>,
}

impl VisibleEntries {
    fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
        }
    }

    fn add(
        &mut self,
        idx: usize,
        entry: async_std::sync::MutexGuardArc<Entry>,
    ) {
        // push_front because we are adding them in reverse order
        self.entries.push_front((idx, entry));
    }
}

impl std::iter::Iterator for VisibleEntries {
    type Item = (usize, async_std::sync::MutexGuardArc<Entry>);

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.pop_front()
    }
}

impl std::iter::DoubleEndedIterator for VisibleEntries {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.pop_back()
    }
}

fn run_commands(
    ast: crate::parse::ast::Commands,
    entry: async_std::sync::Arc<async_std::sync::Mutex<Entry>>,
    mut env: Env,
    input_r: async_std::channel::Receiver<Vec<u8>>,
    resize_r: async_std::channel::Receiver<(u16, u16)>,
    event_w: async_std::channel::Sender<Event>,
) {
    async_std::task::spawn(async move {
        let pty = match pty::Pty::new(
            entry.lock_arc().await.size(),
            &entry,
            input_r,
            resize_r,
            event_w.clone(),
        ) {
            Ok(pty) => pty,
            Err(e) => {
                let mut entry = entry.lock_arc().await;
                entry.process(
                    format!("nbsh: failed to allocate pty: {}\r\n", e)
                        .as_bytes(),
                );
                env.set_status(async_std::process::ExitStatus::from_raw(
                    1 << 8,
                ));
                entry.finish(env, event_w).await;
                return;
            }
        };

        for pipeline in ast.pipelines() {
            env.set_pipeline(pipeline.input_string().to_string());
            match run_pipeline(&pty, &mut env, event_w.clone()).await {
                Ok((pipeline_status, done)) => {
                    env.set_status(pipeline_status);
                    if done {
                        break;
                    }
                }
                Err(e) => {
                    entry
                        .lock_arc()
                        .await
                        .process(format!("nbsh: {}\r\n", e).as_bytes());
                    env.set_status(async_std::process::ExitStatus::from_raw(
                        1 << 8,
                    ));
                }
            }
        }
        entry.lock_arc().await.finish(env, event_w).await;

        pty.close().await;
    });
}

async fn run_pipeline(
    pty: &pty::Pty,
    env: &mut Env,
    event_w: async_std::channel::Sender<Event>,
) -> anyhow::Result<(async_std::process::ExitStatus, bool)> {
    let mut cmd = pty_process::Command::new(std::env::current_exe().unwrap());
    cmd.arg("--internal-cmd-runner");
    env.apply(&mut cmd);
    let (to_r, to_w) =
        nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).unwrap();
    let (from_r, from_w) =
        nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).unwrap();
    // Safety: dup2 is an async-signal-safe function
    unsafe {
        cmd.pre_exec(move || {
            nix::unistd::dup2(to_r, 3)?;
            nix::unistd::dup2(from_w, 4)?;
            Ok(())
        });
    }
    let child = pty.spawn(cmd).unwrap();
    nix::unistd::close(to_r).unwrap();
    nix::unistd::close(from_w).unwrap();

    // Safety: to_w was just opened above, was not used until now, and can't
    // be used after this because we rebound the variable
    let mut to_w = unsafe { async_std::fs::File::from_raw_fd(to_w) };
    to_w.write_all(&env.as_bytes()).await.unwrap();
    drop(to_w);

    let (read_w, read_r) = async_std::channel::unbounded();
    let new_read = move || {
        let read_w = read_w.clone();
        async_std::task::spawn(async move {
            let event = blocking::unblock(move || {
                // Safety: from_r was just opened above and is only
                // referenced in this closure, which takes ownership of it
                // at the start and returns ownership of it at the end
                let fh = unsafe { std::fs::File::from_raw_fd(from_r) };
                let event = bincode::deserialize_from(&fh);
                let _ = fh.into_raw_fd();
                event
            })
            .await;
            if read_w.is_closed() {
                // we should never drop read_r while there are still valid
                // things to read
                assert!(event.is_err());
            } else {
                read_w.send(event).await.unwrap();
            }
        });
    };

    new_read();
    let mut read_done = false;
    let mut exit_done = None;
    loop {
        enum Res {
            Read(bincode::Result<crate::pipeline::Event>),
            Exit(std::io::Result<std::process::ExitStatus>),
        }

        let read_r = read_r.clone();
        let read = async move { Res::Read(read_r.recv().await.unwrap()) };
        let exit = async { Res::Exit(child.status_no_drop().await) };
        match read.or(exit).await {
            Res::Read(Ok(event)) => {
                match event {
                    crate::pipeline::Event::Suspend(idx) => {
                        event_w.send(Event::ChildSuspend(idx)).await.unwrap();
                    }
                    crate::pipeline::Event::Exit(new_env) => {
                        *env = new_env;
                    }
                }
                new_read();
            }
            Res::Read(Err(e)) => {
                if let bincode::ErrorKind::Io(e) = &*e {
                    if e.kind() == std::io::ErrorKind::UnexpectedEof {
                        read_done = true;
                        continue;
                    }
                }
                anyhow::bail!(e);
            }
            Res::Exit(Ok(status)) => {
                exit_done = Some(status);
            }
            Res::Exit(Err(e)) => {
                anyhow::bail!(e);
            }
        }
        if let (true, Some(status)) = (read_done, exit_done) {
            // nix::sys::signal::Signal is repr(i32)
            #[allow(clippy::as_conversions)]
            return Ok((
                status,
                // i'm not sure what exactly the expected behavior
                // here is - in zsh, SIGINT kills the whole command
                // line while SIGTERM doesn't, but i don't know what
                // the precise logic is or how other signals are
                // handled
                status.signal()
                    == Some(nix::sys::signal::Signal::SIGINT as i32),
            ));
        }
    }
}
