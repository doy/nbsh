use crate::shell::prelude::*;

enum State {
    Running((usize, usize)),
    Exited(ExitInfo),
}

pub struct Entry {
    cmdline: String,
    env: Env,
    state: State,
    vt: vt100::Parser,
    audible_bell_state: usize,
    visual_bell_state: usize,
    audible_bell: bool,
    visual_bell: bool,
    real_bell_pending: bool,
    fullscreen: Option<bool>,
    input: async_std::channel::Sender<Vec<u8>>,
    resize: async_std::channel::Sender<(u16, u16)>,
    start_time: time::OffsetDateTime,
    start_instant: std::time::Instant,
}

impl Entry {
    pub fn new(
        cmdline: String,
        env: Env,
        size: (u16, u16),
        input: async_std::channel::Sender<Vec<u8>>,
        resize: async_std::channel::Sender<(u16, u16)>,
    ) -> Self {
        let span = (0, cmdline.len());
        Self {
            cmdline,
            env,
            state: State::Running(span),
            vt: vt100::Parser::new(size.0, size.1, 0),
            audible_bell_state: 0,
            visual_bell_state: 0,
            audible_bell: false,
            visual_bell: false,
            real_bell_pending: false,
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

        self.bell(out);
        if focused {
            self.audible_bell = false;
            self.visual_bell = false;
        }

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

        if self.audible_bell || self.visual_bell {
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

        if self.binary() {
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
            let last_row = self.output_lines(focused && !scrolling);
            let mut max_lines = self.max_lines(entry_count);
            if last_row > max_lines {
                out.write(b"\r\n");
                out.set_fgcolor(textmode::color::BLUE);
                out.write_str("...");
                out.reset_attributes();
                max_lines -= 1;
            }
            let mut out_row = out.screen().cursor_position().0 + 1;
            let screen = self.vt.screen();
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
        out.write(&self.vt.screen().state_formatted());
        self.bell(out);
        self.audible_bell = false;
        self.visual_bell = false;
        out.reset_attributes();
    }

    pub async fn send_input(&self, bytes: Vec<u8>) {
        if self.running() {
            self.input.send(bytes).await.unwrap();
        }
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        if self.running() {
            self.resize.send(size).await.unwrap();
            self.vt.set_size(size.0, size.1);
        }
    }

    pub fn size(&self) -> (u16, u16) {
        self.vt.screen().size()
    }

    pub fn process(&mut self, input: &[u8]) {
        self.vt.process(input);
        let screen = self.vt.screen();

        let new_audible_bell_state = screen.audible_bell_count();
        if new_audible_bell_state != self.audible_bell_state {
            self.audible_bell = true;
            self.real_bell_pending = true;
            self.audible_bell_state = new_audible_bell_state;
        }

        let new_visual_bell_state = screen.visual_bell_count();
        if new_visual_bell_state != self.visual_bell_state {
            self.visual_bell = true;
            self.real_bell_pending = true;
            self.visual_bell_state = new_visual_bell_state;
        }
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

    pub fn binary(&self) -> bool {
        self.vt.screen().errors() > 5
    }

    pub fn lines(&self, entry_count: usize, focused: bool) -> usize {
        1 + std::cmp::min(
            self.output_lines(focused),
            self.max_lines(entry_count),
        )
    }

    fn max_lines(&self, entry_count: usize) -> usize {
        if self.env.idx() == entry_count - 1 {
            usize::from(self.size().0) * 2 / 3
        } else {
            5
        }
    }

    pub fn output_lines(&self, focused: bool) -> usize {
        if self.binary() {
            return 1;
        }

        let screen = self.vt.screen();
        let mut last_row = 0;
        for (idx, row) in screen.rows(0, self.size().1).enumerate() {
            if !row.is_empty() {
                last_row = idx + 1;
            }
        }
        if focused && self.running() {
            last_row = std::cmp::max(
                last_row,
                usize::from(screen.cursor_position().0) + 1,
            );
        }
        last_row
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

    pub async fn finish(
        &mut self,
        env: Env,
        event_w: async_std::channel::Sender<Event>,
    ) {
        self.state = State::Exited(ExitInfo::new(*env.latest_status()));
        self.env = env;
        event_w.send(Event::PtyClose).await.unwrap();
    }

    fn exit_info(&self) -> Option<&ExitInfo> {
        match &self.state {
            State::Running(..) => None,
            State::Exited(exit_info) => Some(exit_info),
        }
    }

    fn bell(&mut self, out: &mut impl textmode::Textmode) {
        if self.real_bell_pending {
            if self.audible_bell {
                out.write(b"\x07");
            }
            if self.visual_bell {
                out.write(b"\x1bg");
            }
            self.real_bell_pending = false;
        }
    }
}

struct ExitInfo {
    status: async_std::process::ExitStatus,
    instant: std::time::Instant,
}

impl ExitInfo {
    fn new(status: async_std::process::ExitStatus) -> Self {
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
