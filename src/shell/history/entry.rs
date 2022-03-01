use crate::shell::prelude::*;

enum State {
    Running((usize, usize)),
    Exited(ExitInfo),
}

pub struct Entry {
    cmdline: String,
    env: Env,
    state: State,
    vt: super::vt::Vt,
    fullscreen: Option<bool>,
    input: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
    resize: tokio::sync::mpsc::UnboundedSender<(u16, u16)>,
    start_time: time::OffsetDateTime,
    start_instant: std::time::Instant,
}

impl Entry {
    pub fn new(
        cmdline: String,
        env: Env,
        size: (u16, u16),
        input: tokio::sync::mpsc::UnboundedSender<Vec<u8>>,
        resize: tokio::sync::mpsc::UnboundedSender<(u16, u16)>,
    ) -> Self {
        let span = (0, cmdline.len());
        Self {
            cmdline,
            env,
            state: State::Running(span),
            vt: super::vt::Vt::new(size),
            input,
            resize,
            fullscreen: None,
            start_time: time::OffsetDateTime::now_utc(),
            start_instant: std::time::Instant::now(),
        }
    }

    pub fn render(
        &mut self,
        out: &mut impl textmode::Textmode,
        idx: usize,
        entry_count: usize,
        size: (u16, u16),
        focused: bool,
        scrolling: bool,
        offset: time::UtcOffset,
    ) {
        let time = self.exit_info().map_or_else(
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

        set_bgcolor(out, idx, focused);
        out.set_fgcolor(textmode::color::YELLOW);
        let entry_count_width = format!("{}", entry_count + 1).len();
        let idx_str = format!("{}", idx + 1);
        out.write_str(&" ".repeat(entry_count_width - idx_str.len()));
        out.write_str(&idx_str);
        out.write_str(" ");
        out.reset_attributes();

        set_bgcolor(out, idx, focused);
        if let Some(info) = self.exit_info() {
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

        self.vt.bell(out, focused);

        let vt = &self.vt;

        if vt.is_bell() {
            out.set_bgcolor(textmode::Color::Rgb(64, 16, 16));
        } else {
            set_bgcolor(out, idx, focused);
        }
        out.write_str("$ ");
        set_bgcolor(out, idx, focused);
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
                set_bgcolor(out, idx, focused);
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

        set_bgcolor(out, idx, focused);
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
                vt.output_lines(focused && !scrolling, self.running());
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

    pub fn render_fullscreen(&mut self, out: &mut impl textmode::Textmode) {
        let vt = &mut self.vt;
        out.write(&vt.screen().state_formatted());
        vt.bell(out, true);
        out.reset_attributes();
    }

    pub fn send_input(&self, bytes: Vec<u8>) {
        if self.running() {
            self.input.send(bytes).unwrap();
        }
    }

    pub fn resize(&mut self, size: (u16, u16)) {
        if self.running() {
            self.resize.send(size).unwrap();
            self.vt.set_size(size);
        }
    }

    pub fn size(&self) -> (u16, u16) {
        self.vt.size()
    }

    pub fn process(&mut self, input: &[u8]) {
        self.vt.process(input);
    }

    pub fn cmd(&self) -> &str {
        &self.cmdline
    }

    pub fn env(&self) -> &Env {
        &self.env
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(fullscreen) = self.fullscreen {
            self.fullscreen = Some(!fullscreen);
        } else {
            self.fullscreen = Some(!self.vt.screen().alternate_screen());
        }
    }

    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = Some(fullscreen);
    }

    pub fn running(&self) -> bool {
        matches!(self.state, State::Running(_))
    }

    pub fn lines(&self, entry_count: usize, focused: bool) -> usize {
        1 + std::cmp::min(
            self.vt.output_lines(focused, self.running()),
            self.max_lines(entry_count),
        )
    }

    fn max_lines(&self, entry_count: usize) -> usize {
        if self.env.idx() == entry_count - 1 {
            15
        } else {
            5
        }
    }

    pub fn should_fullscreen(&self) -> bool {
        self.fullscreen
            .unwrap_or_else(|| self.vt.screen().alternate_screen())
    }

    pub fn set_span(&mut self, span: (usize, usize)) {
        if matches!(self.state, State::Running(_)) {
            self.state = State::Running(span);
        }
    }

    pub fn finish(
        &mut self,
        env: Env,
        event_w: &crate::shell::event::Writer,
    ) {
        self.state = State::Exited(ExitInfo::new(env.latest_status()));
        self.env = env;
        event_w.send(Event::PtyClose);
    }

    fn exit_info(&self) -> Option<&ExitInfo> {
        match &self.state {
            State::Running(..) => None,
            State::Exited(exit_info) => Some(exit_info),
        }
    }
}

struct ExitInfo {
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

fn set_bgcolor(out: &mut impl textmode::Textmode, idx: usize, focus: bool) {
    if focus {
        out.set_bgcolor(textmode::Color::Rgb(0x56, 0x1b, 0x8b));
    } else if idx % 2 == 0 {
        out.set_bgcolor(textmode::Color::Rgb(0x24, 0x21, 0x00));
    } else {
        out.set_bgcolor(textmode::Color::Rgb(0x20, 0x20, 0x20));
    }
}
