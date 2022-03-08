use crate::shell::prelude::*;

pub struct Entry {
    cmdline: String,
    env: Env,
    pty: super::pty::Pty,
    fullscreen: Option<bool>,
    start_instant: std::time::Instant,
    start_time: time::OffsetDateTime,
    state: State,
}

impl Entry {
    pub fn new(
        cmdline: String,
        env: Env,
        size: (u16, u16),
        event_w: crate::shell::event::Writer,
    ) -> Result<Self> {
        let start_instant = std::time::Instant::now();
        let start_time = time::OffsetDateTime::now_utc();

        let (pty, pts) = super::pty::Pty::new(size, event_w.clone()).unwrap();
        let (child, fh) = Self::spawn_command(&cmdline, &env, &pts)?;
        tokio::spawn(Self::task(child, fh, env.idx(), event_w));
        Ok(Self {
            cmdline,
            env,
            pty,
            fullscreen: None,
            start_instant,
            start_time,
            state: State::Running((0, 0)),
        })
    }

    pub fn render(
        &self,
        out: &mut impl textmode::Textmode,
        entry_count: usize,
        vt: &mut super::pty::Vt,
        focused: bool,
        scrolling: bool,
        offset: time::UtcOffset,
    ) {
        let idx = self.env.idx();
        let size = out.screen().size();
        let time = self.state.exit_info().map_or_else(
            || {
                format!(
                    "[{}]",
                    crate::format::time(self.start_time.to_offset(offset))
                )
            },
            |info| {
                format!(
                    "({}) [{}]",
                    crate::format::duration(
                        info.instant - self.start_instant
                    ),
                    crate::format::time(self.start_time.to_offset(offset)),
                )
            },
        );

        if vt.bell(focused) {
            out.write(b"\x07");
        }

        Self::set_bgcolor(out, idx, focused);
        out.set_fgcolor(textmode::color::YELLOW);
        let entry_count_width = format!("{}", entry_count + 1).len();
        let idx_str = format!("{}", idx + 1);
        out.write_str(&" ".repeat(entry_count_width - idx_str.len()));
        out.write_str(&idx_str);
        out.write_str(" ");
        out.reset_attributes();

        Self::set_bgcolor(out, idx, focused);
        if let Some(info) = self.state.exit_info() {
            if info.status.signal().is_some() {
                out.set_fgcolor(textmode::color::MAGENTA);
            } else if info.status.success() {
                out.set_fgcolor(textmode::color::DARKGREY);
            } else {
                out.set_fgcolor(textmode::color::RED);
            }
            out.write_str(&crate::format::exit_status(info.status));
        } else {
            out.write_str("     ");
        }
        out.reset_attributes();

        if vt.is_bell() {
            out.set_bgcolor(textmode::Color::Rgb(64, 16, 16));
        } else {
            Self::set_bgcolor(out, idx, focused);
        }
        out.write_str("$ ");
        Self::set_bgcolor(out, idx, focused);
        let start = usize::from(out.screen().cursor_position().1);
        let end = usize::from(size.1) - time.len() - 2;
        let max_len = end - start;
        let cmd = if self.cmd().len() > max_len {
            &self.cmd()[..(max_len - 4)]
        } else {
            self.cmd()
        };
        if let State::Running(span) = self.state {
            let span = (span.0.min(cmd.len()), span.1.min(cmd.len()));
            if !cmd[..span.0].is_empty() {
                out.write_str(&cmd[..span.0]);
            }
            if !cmd[span.0..span.1].is_empty() {
                out.set_bgcolor(textmode::Color::Rgb(16, 64, 16));
                out.write_str(&cmd[span.0..span.1]);
                Self::set_bgcolor(out, idx, focused);
            }
            if !cmd[span.1..].is_empty() {
                out.write_str(&cmd[span.1..]);
            }
        } else {
            out.write_str(cmd);
        }
        if self.cmd().len() > max_len {
            if let State::Running(span) = self.state {
                if span.0 < cmd.len() && span.1 > cmd.len() {
                    out.set_bgcolor(textmode::Color::Rgb(16, 64, 16));
                }
            }
            out.write_str(" ");
            if let State::Running(span) = self.state {
                if span.1 > cmd.len() {
                    out.set_bgcolor(textmode::Color::Rgb(16, 64, 16));
                }
            }
            out.set_fgcolor(textmode::color::BLUE);
            out.write_str("...");
        }
        out.reset_attributes();

        Self::set_bgcolor(out, idx, focused);
        let cur_pos = out.screen().cursor_position();
        out.write_str(&" ".repeat(
            usize::from(size.1) - time.len() - 1 - usize::from(cur_pos.1),
        ));
        out.write_str(&time);
        out.write_str(" ");
        out.reset_attributes();

        if vt.binary() {
            let msg = "This appears to be binary data. Fullscreen this entry to view anyway.";
            let len: u16 = msg.len().try_into().unwrap();
            out.move_to(
                out.screen().cursor_position().0 + 1,
                (size.1 - len) / 2,
            );
            out.set_fgcolor(textmode::color::RED);
            out.write_str(msg);
            out.hide_cursor(true);
        } else {
            let last_row =
                vt.output_lines(focused && !scrolling, self.state.running());
            let mut max_lines = self.max_lines(entry_count);
            if last_row > max_lines {
                out.write(b"\r\n");
                out.set_fgcolor(textmode::color::BLUE);
                out.write_str("...");
                out.reset_attributes();
                max_lines -= 1;
            }
            let mut out_row = out.screen().cursor_position().0 + 1;
            let screen = vt.screen();
            let pos = screen.cursor_position();
            let mut wrapped = false;
            let mut cursor_found = None;
            for (idx, row) in screen
                .rows_formatted(0, size.1)
                .enumerate()
                .take(last_row)
                .skip(last_row.saturating_sub(max_lines))
            {
                let idx: u16 = idx.try_into().unwrap();
                out.reset_attributes();
                if !wrapped {
                    out.move_to(out_row, 0);
                }
                out.write(&row);
                wrapped = screen.row_wrapped(idx);
                if pos.0 == idx {
                    cursor_found = Some(out_row);
                }
                out_row += 1;
            }
            if focused && !scrolling {
                if let Some(row) = cursor_found {
                    out.hide_cursor(screen.hide_cursor());
                    out.move_to(row, pos.1);
                } else {
                    out.hide_cursor(true);
                }
            }
        }

        out.reset_attributes();
    }

    pub fn render_fullscreen(&self, out: &mut impl textmode::Textmode) {
        self.pty.with_vt_mut(|vt| {
            out.write(&vt.screen().state_formatted());
            if vt.bell(true) {
                out.write(b"\x07");
            }
            out.reset_attributes();
        });
    }

    pub fn input(&self, bytes: Vec<u8>) {
        self.pty.input(bytes);
    }

    pub fn resize(&self, size: (u16, u16)) {
        self.pty.resize(size);
    }

    pub fn cmd(&self) -> &str {
        &self.cmdline
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(fullscreen) = self.fullscreen {
            self.fullscreen = Some(!fullscreen);
        } else {
            self.fullscreen = Some(!self.pty.fullscreen());
        }
    }

    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = Some(fullscreen);
    }

    pub fn running(&self) -> bool {
        self.state.running()
    }

    pub fn exited(&mut self, exit_info: ExitInfo) {
        self.state = State::Exited(exit_info);
    }

    pub fn lines(&self, entry_count: usize, focused: bool) -> usize {
        let running = self.running();
        1 + std::cmp::min(
            self.pty.with_vt(|vt| vt.output_lines(focused, running)),
            self.max_lines(entry_count),
        )
    }

    pub fn should_fullscreen(&self) -> bool {
        self.fullscreen.unwrap_or_else(|| self.pty.fullscreen())
    }

    pub fn lock_vt(&self) -> std::sync::MutexGuard<super::pty::Vt> {
        self.pty.lock_vt()
    }

    pub fn set_span(&mut self, new_span: (usize, usize)) {
        if let State::Running(ref mut span) = self.state {
            *span = new_span;
        }
    }

    fn max_lines(&self, entry_count: usize) -> usize {
        if self.env.idx() == entry_count - 1 {
            15
        } else {
            5
        }
    }

    fn set_bgcolor(
        out: &mut impl textmode::Textmode,
        idx: usize,
        focus: bool,
    ) {
        if focus {
            out.set_bgcolor(textmode::Color::Rgb(0x56, 0x1b, 0x8b));
        } else if idx % 2 == 0 {
            out.set_bgcolor(textmode::Color::Rgb(0x24, 0x21, 0x00));
        } else {
            out.set_bgcolor(textmode::Color::Rgb(0x20, 0x20, 0x20));
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
        let (from_r, from_w) =
            nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC)?;
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

    async fn task(
        mut child: tokio::process::Child,
        fh: std::fs::File,
        idx: usize,
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
                        event_w.send(Event::ChildRunPipeline(idx, new_span));
                    }
                    crate::runner::Event::Suspend => {
                        event_w.send(Event::ChildSuspend(idx));
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
        event_w.send(Event::ChildExit(
            idx,
            ExitInfo::new(exit_status.unwrap()),
            new_env,
        ));
    }
}

enum State {
    Running((usize, usize)),
    Exited(ExitInfo),
}

impl State {
    fn exit_info(&self) -> Option<&ExitInfo> {
        match self {
            Self::Running(_) => None,
            Self::Exited(exit_info) => Some(exit_info),
        }
    }

    fn running(&self) -> bool {
        self.exit_info().is_none()
    }
}

#[derive(Debug)]
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
}
