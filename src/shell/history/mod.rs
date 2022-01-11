use crate::shell::prelude::*;

mod entry;
pub use entry::Entry;
mod pty;

pub struct History {
    size: (u16, u16),
    entries: Vec<crate::mutex::Mutex<Entry>>,
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
        cmdline: &str,
        env: &Env,
        event_w: async_std::channel::Sender<Event>,
    ) -> anyhow::Result<usize> {
        let (input_w, input_r) = async_std::channel::unbounded();
        let (resize_w, resize_r) = async_std::channel::unbounded();

        let entry = crate::mutex::new(Entry::new(
            cmdline.to_string(),
            env.clone(),
            self.size,
            input_w,
            resize_w,
        ));
        run_commands(
            cmdline.to_string(),
            crate::mutex::clone(&entry),
            env.clone(),
            input_r,
            resize_r,
            event_w,
        );

        self.entries.push(entry);
        Ok(self.entries.len() - 1)
    }

    pub async fn entry(&self, idx: usize) -> crate::mutex::Guard<Entry> {
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
    entries: std::collections::VecDeque<(usize, crate::mutex::Guard<Entry>)>,
}

impl VisibleEntries {
    fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
        }
    }

    fn add(&mut self, idx: usize, entry: crate::mutex::Guard<Entry>) {
        // push_front because we are adding them in reverse order
        self.entries.push_front((idx, entry));
    }
}

impl std::iter::Iterator for VisibleEntries {
    type Item = (usize, crate::mutex::Guard<Entry>);

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
    cmdline: String,
    entry: crate::mutex::Mutex<Entry>,
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

        let status =
            match spawn_commands(&cmdline, &pty, &mut env, event_w.clone())
                .await
            {
                Ok(status) => status,
                Err(e) => {
                    let mut entry = entry.lock_arc().await;
                    entry.process(
                        format!(
                            "nbsh: failed to spawn {}: {}\r\n",
                            cmdline, e
                        )
                        .as_bytes(),
                    );
                    env.set_status(async_std::process::ExitStatus::from_raw(
                        1 << 8,
                    ));
                    entry.finish(env, event_w).await;
                    return;
                }
            };
        env.set_status(status);

        entry.lock_arc().await.finish(env, event_w).await;
        pty.close().await;
    });
}

async fn spawn_commands(
    cmdline: &str,
    pty: &pty::Pty,
    env: &mut Env,
    event_w: async_std::channel::Sender<Event>,
) -> anyhow::Result<async_std::process::ExitStatus> {
    let mut cmd = pty_process::Command::new(std::env::current_exe()?);
    cmd.arg("--internal-cmd-runner");
    env.apply(&mut cmd);
    let (to_r, to_w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    let (from_r, from_w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
    // Safety: dup2 is an async-signal-safe function
    unsafe {
        cmd.pre_exec(move || {
            nix::unistd::dup2(to_r, 3)?;
            nix::unistd::dup2(from_w, 4)?;
            Ok(())
        });
    }
    let child = pty.spawn(cmd)?;
    nix::unistd::close(to_r)?;
    nix::unistd::close(from_w)?;

    // Safety: to_w was just opened above, was not used until now, and can't
    // be used after this because from_raw_fd takes it by move
    write_env(
        unsafe { async_std::fs::File::from_raw_fd(to_w) },
        cmdline,
        env,
    )
    .await?;

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
            Read(bincode::Result<crate::runner::Event>),
            Exit(std::io::Result<std::process::ExitStatus>),
        }

        let read_r = read_r.clone();
        let read = async move { Res::Read(read_r.recv().await.unwrap()) };
        let exit = async {
            Res::Exit(if exit_done.is_none() {
                child.status_no_drop().await
            } else {
                std::future::pending().await
            })
        };
        match read.or(exit).await {
            Res::Read(Ok(event)) => match event {
                crate::runner::Event::RunPipeline(idx, span) => {
                    event_w
                        .send(Event::ChildRunPipeline(idx, span))
                        .await
                        .unwrap();
                    new_read();
                }
                crate::runner::Event::Suspend(idx) => {
                    event_w.send(Event::ChildSuspend(idx)).await.unwrap();
                    new_read();
                }
                crate::runner::Event::Exit(new_env) => {
                    *env = new_env;
                    read_done = true;
                }
            },
            Res::Read(Err(e)) => {
                if let bincode::ErrorKind::Io(io_e) = &*e {
                    if io_e.kind() == std::io::ErrorKind::UnexpectedEof {
                        read_done = true;
                    } else {
                        anyhow::bail!(e);
                    }
                } else {
                    anyhow::bail!(e);
                }
            }
            Res::Exit(Ok(status)) => {
                exit_done = Some(status);
            }
            Res::Exit(Err(e)) => {
                anyhow::bail!(e);
            }
        }
        if let (true, Some(status)) = (read_done, exit_done) {
            nix::unistd::close(from_r)?;
            // nix::sys::signal::Signal is repr(i32)
            #[allow(clippy::as_conversions)]
            return Ok(status);
        }
    }
}

async fn write_env(
    mut to_w: async_std::fs::File,
    pipeline: &str,
    env: &Env,
) -> anyhow::Result<()> {
    to_w.write_all(&bincode::serialize(pipeline)?).await?;
    to_w.write_all(&env.as_bytes()).await?;
    Ok(())
}
