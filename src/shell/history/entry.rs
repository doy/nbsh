use crate::shell::prelude::*;

pub struct Entry {
    cmdline: String,
    env: Env,
    pty: super::pty::Pty,
    job: super::job::Job,
    fullscreen: Option<bool>,
}

impl Entry {
    pub fn new(
        cmdline: String,
        env: Env,
        size: (u16, u16),
        event_w: crate::shell::event::Writer,
    ) -> Self {
        let (pty, pts) = super::pty::Pty::new(size, event_w.clone()).unwrap();
        let job = super::job::Job::new(&cmdline, env.clone(), &pts, event_w)
            .unwrap();
        Self {
            cmdline,
            env,
            pty,
            job,
            fullscreen: None,
        }
    }

    pub fn render(
        &self,
        out: &mut impl textmode::Textmode,
        idx: usize,
        entry_count: usize,
        state: &super::job::State,
        vt: &mut super::pty::Vt,
        size: (u16, u16),
        focused: bool,
        scrolling: bool,
        offset: time::UtcOffset,
    ) {
        let time = state.exit_info().map_or_else(
            || {
                format!(
                    "[{}]",
                    crate::format::time(
                        self.job.start_time().to_offset(offset)
                    )
                )
            },
            |info| {
                format!(
                    "({}) [{}]",
                    crate::format::duration(
                        *info.instant() - *self.job.start_instant()
                    ),
                    crate::format::time(
                        self.job.start_time().to_offset(offset)
                    ),
                )
            },
        );

        vt.bell(out, focused);

        set_bgcolor(out, idx, focused);
        out.set_fgcolor(textmode::color::YELLOW);
        let entry_count_width = format!("{}", entry_count + 1).len();
        let idx_str = format!("{}", idx + 1);
        out.write_str(&" ".repeat(entry_count_width - idx_str.len()));
        out.write_str(&idx_str);
        out.write_str(" ");
        out.reset_attributes();

        set_bgcolor(out, idx, focused);
        if let Some(info) = state.exit_info() {
            let status = info.status();
            if status.signal().is_some() {
                out.set_fgcolor(textmode::color::MAGENTA);
            } else if status.success() {
                out.set_fgcolor(textmode::color::DARKGREY);
            } else {
                out.set_fgcolor(textmode::color::RED);
            }
            out.write_str(&crate::format::exit_status(status));
        } else {
            out.write_str("     ");
        }
        out.reset_attributes();

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
        if let super::job::State::Running(span) = state {
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
            if let super::job::State::Running(span) = state {
                if span.0 < cmd.len() && span.1 > cmd.len() {
                    out.set_bgcolor(textmode::Color::Rgb(16, 64, 16));
                }
            }
            out.write_str(" ");
            if let super::job::State::Running(span) = state {
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
                vt.output_lines(focused && !scrolling, state.running());
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
            vt.bell(out, true);
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
        self.job.running()
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

    pub fn lock_state(&self) -> std::sync::MutexGuard<super::job::State> {
        self.job.lock_state()
    }

    pub fn set_span(&self, span: (usize, usize)) {
        self.job.set_span(span);
    }

    fn max_lines(&self, entry_count: usize) -> usize {
        if self.env.idx() == entry_count - 1 {
            15
        } else {
            5
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
