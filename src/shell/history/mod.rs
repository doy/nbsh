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
        for (idx, mut entry) in
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

    pub async fn render_fullscreen(
        &self,
        out: &mut impl textmode::Textmode,
        idx: usize,
    ) {
        let mut entry = self.entries[idx].clone().lock_owned().await;
        entry.render_fullscreen(out);
    }

    pub async fn send_input(&mut self, idx: usize, input: Vec<u8>) {
        self.entry(idx).await.send_input(input).await;
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        for entry in &self.entries {
            entry.clone().lock_owned().await.resize(size).await;
        }
    }

    pub async fn run(
        &mut self,
        cmdline: &str,
        env: &Env,
        event_w: tokio::sync::mpsc::UnboundedSender<Event>,
    ) -> anyhow::Result<usize> {
        let (input_w, input_r) = tokio::sync::mpsc::unbounded_channel();
        let (resize_w, resize_r) = tokio::sync::mpsc::unbounded_channel();

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
        self.entries[idx].clone().lock_owned().await
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
            let entry = entry.clone().lock_owned().await;
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
    input_r: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    resize_r: tokio::sync::mpsc::UnboundedReceiver<(u16, u16)>,
    event_w: tokio::sync::mpsc::UnboundedSender<Event>,
) {
    tokio::task::spawn(async move {
        let pty = match pty::Pty::new(
            entry.clone().lock_owned().await.size(),
            &entry,
            input_r,
            resize_r,
            event_w.clone(),
        ) {
            Ok(pty) => pty,
            Err(e) => {
                let mut entry = entry.clone().lock_owned().await;
                entry.process(
                    format!("nbsh: failed to allocate pty: {}\r\n", e)
                        .as_bytes(),
                );
                env.set_status(std::process::ExitStatus::from_raw(1 << 8));
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
                    let mut entry = entry.clone().lock_owned().await;
                    entry.process(
                        format!(
                            "nbsh: failed to spawn {}: {}\r\n",
                            cmdline, e
                        )
                        .as_bytes(),
                    );
                    env.set_status(std::process::ExitStatus::from_raw(
                        1 << 8,
                    ));
                    entry.finish(env, event_w).await;
                    return;
                }
            };
        env.set_status(status);

        entry.clone().lock_owned().await.finish(env, event_w).await;
        pty.close().await;
    });
}

async fn spawn_commands(
    cmdline: &str,
    pty: &pty::Pty,
    env: &mut Env,
    event_w: tokio::sync::mpsc::UnboundedSender<Event>,
) -> anyhow::Result<std::process::ExitStatus> {
    enum Res {
        Read(crate::runner::Event),
        Exit(std::io::Result<std::process::ExitStatus>),
    }

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
    let mut child = pty.spawn(cmd)?;
    nix::unistd::close(from_w)?;

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
                            io_e.kind() == std::io::ErrorKind::UnexpectedEof
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
    while let Some(res) = stream.next().await {
        match res {
            Res::Read(event) => match event {
                crate::runner::Event::RunPipeline(idx, span) => {
                    event_w.send(Event::ChildRunPipeline(idx, span)).unwrap();
                }
                crate::runner::Event::Suspend(idx) => {
                    event_w.send(Event::ChildSuspend(idx)).unwrap();
                }
                crate::runner::Event::Exit(new_env) => {
                    *env = new_env;
                }
            },
            Res::Exit(Ok(status)) => {
                exit_status = Some(status);
            }
            Res::Exit(Err(e)) => {
                anyhow::bail!(e);
            }
        }
    }
    Ok(exit_status.unwrap())
}
