use crate::shell::prelude::*;

mod entry;
pub use entry::Entry;
mod pty;
mod vt;

pub struct History {
    size: (u16, u16),
    entries: Vec<std::sync::Arc<std::sync::Mutex<Entry>>>,
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

    pub fn render(
        &self,
        out: &mut impl textmode::Textmode,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
        offset: time::UtcOffset,
    ) {
        let mut used_lines = repl_lines;
        let mut cursor = None;
        for (idx, mut entry) in
            self.visible(repl_lines, focus, scrolling).rev()
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
    }

    pub fn render_fullscreen(
        &self,
        out: &mut impl textmode::Textmode,
        idx: usize,
    ) {
        self.with_entry_mut(idx, |entry| entry.render_fullscreen(out));
    }

    pub fn send_input(&mut self, idx: usize, input: Vec<u8>) {
        self.with_entry(idx, |entry| entry.send_input(input));
    }

    pub fn should_fullscreen(&self, idx: usize) -> bool {
        self.with_entry(idx, Entry::should_fullscreen)
    }

    pub fn running(&self, idx: usize) -> bool {
        self.with_entry(idx, Entry::running)
    }

    pub fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        for entry in &self.entries {
            entry.lock().unwrap().resize(size);
        }
    }

    pub fn run(
        &mut self,
        cmdline: String,
        env: Env,
        event_w: crate::shell::event::Writer,
    ) -> usize {
        let (input_w, input_r) = tokio::sync::mpsc::unbounded_channel();
        let (resize_w, resize_r) = tokio::sync::mpsc::unbounded_channel();

        let entry = std::sync::Arc::new(std::sync::Mutex::new(Entry::new(
            cmdline.clone(),
            env.clone(),
            self.size,
            input_w,
            resize_w,
        )));
        run_commands(
            cmdline,
            std::sync::Arc::clone(&entry),
            env,
            input_r,
            resize_r,
            event_w,
        );

        self.entries.push(entry);
        self.entries.len() - 1
    }

    pub fn with_entry<T>(
        &self,
        idx: usize,
        f: impl FnOnce(&Entry) -> T,
    ) -> T {
        let entry = self.entries[idx].lock().unwrap();
        f(&*entry)
    }

    pub fn with_entry_mut<T>(
        &self,
        idx: usize,
        f: impl FnOnce(&mut Entry) -> T,
    ) -> T {
        let mut entry = self.entries[idx].lock().unwrap();
        f(&mut *entry)
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn make_focus_visible(
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
                .map(|(idx, _)| idx)
                .last()
                .unwrap()
        {
            self.scroll_pos -= 1;
        }
    }

    fn visible(
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
            let entry = entry.lock().unwrap();
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

struct VisibleEntries<'a> {
    entries:
        std::collections::VecDeque<(usize, std::sync::MutexGuard<'a, Entry>)>,
}

impl<'a> VisibleEntries<'a> {
    fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
        }
    }

    fn add(&mut self, idx: usize, entry: std::sync::MutexGuard<'a, Entry>) {
        // push_front because we are adding them in reverse order
        self.entries.push_front((idx, entry));
    }
}

impl<'a> std::iter::Iterator for VisibleEntries<'a> {
    type Item = (usize, std::sync::MutexGuard<'a, Entry>);

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.pop_front()
    }
}

impl<'a> std::iter::DoubleEndedIterator for VisibleEntries<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.pop_back()
    }
}

fn run_commands(
    cmdline: String,
    entry: std::sync::Arc<std::sync::Mutex<Entry>>,
    mut env: Env,
    input_r: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    resize_r: tokio::sync::mpsc::UnboundedReceiver<(u16, u16)>,
    event_w: crate::shell::event::Writer,
) {
    tokio::task::spawn(async move {
        let size = entry.lock().unwrap().size();
        let pty = match pty::Pty::new(
            size,
            &entry,
            input_r,
            resize_r,
            event_w.clone(),
        ) {
            Ok(pty) => pty,
            Err(e) => {
                let mut entry = entry.lock().unwrap();
                entry.process(
                    format!("nbsh: failed to allocate pty: {}\r\n", e)
                        .as_bytes(),
                );
                env.set_status(std::process::ExitStatus::from_raw(1 << 8));
                entry.finish(env, &event_w);
                return;
            }
        };

        let status =
            match spawn_commands(&cmdline, &pty, &mut env, event_w.clone())
                .await
            {
                Ok(status) => status,
                Err(e) => {
                    let mut entry = entry.lock().unwrap();
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
                    entry.finish(env, &event_w);
                    return;
                }
            };
        env.set_status(status);

        entry.lock().unwrap().finish(env, &event_w);
        pty.close();
    });
}

async fn spawn_commands(
    cmdline: &str,
    pty: &pty::Pty,
    env: &mut Env,
    event_w: crate::shell::event::Writer,
) -> Result<std::process::ExitStatus> {
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
                    event_w.send(Event::ChildRunPipeline(idx, span));
                }
                crate::runner::Event::Suspend(idx) => {
                    event_w.send(Event::ChildSuspend(idx));
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
