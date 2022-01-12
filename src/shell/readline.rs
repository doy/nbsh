use crate::shell::prelude::*;

use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

pub struct Readline {
    size: (u16, u16),
    input_line: String,
    scroll: usize,
    pos: usize,
}

impl Readline {
    pub fn new() -> Self {
        Self {
            size: (24, 80),
            input_line: "".into(),
            scroll: 0,
            pos: 0,
        }
    }

    pub async fn render(
        &self,
        out: &mut impl textmode::Textmode,
        env: &Env,
        git: Option<&super::git::Info>,
        focus: bool,
        offset: time::UtcOffset,
    ) -> anyhow::Result<()> {
        let pwd = env.current_dir();
        let user = crate::info::user()?;
        let hostname = crate::info::hostname()?;
        let time = crate::info::time(offset)?;
        let prompt_char = crate::info::prompt_char()?;

        let id = format!("{}@{}", user, hostname);
        let idlen: u16 = id.len().try_into().unwrap();
        let timelen: u16 = time.len().try_into().unwrap();

        out.move_to(self.size.0 - 2, 0);
        if focus {
            out.set_bgcolor(textmode::Color::Rgb(0x56, 0x1b, 0x8b));
        } else if env.idx() % 2 == 0 {
            out.set_bgcolor(textmode::Color::Rgb(0x24, 0x21, 0x00));
        } else {
            out.set_bgcolor(textmode::Color::Rgb(0x20, 0x20, 0x20));
        }
        out.write(b"\x1b[K");
        out.set_fgcolor(textmode::color::YELLOW);
        out.write_str(&format!("{}", env.idx() + 1));
        out.reset_attributes();
        if focus {
            out.set_bgcolor(textmode::Color::Rgb(0x56, 0x1b, 0x8b));
        } else if env.idx() % 2 == 0 {
            out.set_bgcolor(textmode::Color::Rgb(0x24, 0x21, 0x00));
        } else {
            out.set_bgcolor(textmode::Color::Rgb(0x20, 0x20, 0x20));
        }
        out.write_str(" (");
        out.write_str(&crate::format::path(pwd));
        if let Some(info) = git {
            out.write_str(&format!("|{}", info));
        }
        out.write_str(")");
        out.move_to(self.size.0 - 2, self.size.1 - 4 - idlen - timelen);
        out.write_str(&id);
        out.write_str(" [");
        out.write_str(&time);
        out.write_str("]");

        out.move_to(self.size.0 - 1, 0);
        out.reset_attributes();
        out.write_str(&prompt_char);
        out.write_str(" ");
        out.reset_attributes();
        out.write(b"\x1b[K");
        out.write_str(self.visible_input());
        out.reset_attributes();
        out.move_to(self.size.0 - 1, 2 + self.pos_width());
        if focus {
            out.hide_cursor(false);
        }
        Ok(())
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
    }

    // self will be used eventually
    #[allow(clippy::unused_self)]
    pub fn lines(&self) -> usize {
        2 // XXX handle wrapping
    }

    pub fn input(&self) -> &str {
        &self.input_line
    }

    pub fn add_input(&mut self, s: &str) {
        self.input_line.insert_str(self.byte_pos(), s);
        self.inc_pos(s.chars().count());
    }

    pub fn set_input(&mut self, s: &str) {
        self.input_line = s.to_string();
        self.set_pos(s.chars().count());
    }

    pub fn backspace(&mut self) {
        while self.pos > 0 {
            self.dec_pos(1);
            let width =
                self.input_line.remove(self.byte_pos()).width().unwrap_or(0);
            if width > 0 {
                break;
            }
        }
    }

    pub fn clear_input(&mut self) {
        self.input_line.clear();
        self.set_pos(0);
    }

    pub fn clear_backwards(&mut self) {
        self.input_line = self.input_line.chars().skip(self.pos).collect();
        self.set_pos(0);
    }

    pub fn cursor_left(&mut self) {
        if self.pos == 0 {
            return;
        }
        self.dec_pos(1);
        while let Some(c) = self.input_line.chars().nth(self.pos) {
            if c.width().unwrap_or(0) == 0 {
                self.dec_pos(1);
            } else {
                break;
            }
        }
    }

    pub fn cursor_right(&mut self) {
        if self.pos == self.input_line.chars().count() {
            return;
        }
        self.inc_pos(1);
        while let Some(c) = self.input_line.chars().nth(self.pos) {
            if c.width().unwrap_or(0) == 0 {
                self.inc_pos(1);
            } else {
                break;
            }
        }
    }

    fn set_pos(&mut self, pos: usize) {
        self.pos = pos;
        if self.pos < self.scroll || self.pos_width() > self.size.1 - 2 {
            self.scroll = self.pos;
            let mut extra_scroll = usize::from(self.size.1) / 2;
            while extra_scroll > 0 && self.scroll > 0 {
                self.scroll -= 1;
                extra_scroll -= self
                    .input_line
                    .chars()
                    .nth(self.scroll)
                    .unwrap()
                    .width()
                    .unwrap_or(1);
            }
        }
    }

    fn inc_pos(&mut self, inc: usize) {
        self.set_pos(self.pos + inc);
    }

    fn dec_pos(&mut self, dec: usize) {
        self.set_pos(self.pos - dec);
    }

    fn pos_width(&self) -> u16 {
        let start = self
            .input_line
            .char_indices()
            .nth(self.scroll)
            .map_or(self.input_line.len(), |(i, _)| i);
        let end = self
            .input_line
            .char_indices()
            .nth(self.pos)
            .map_or(self.input_line.len(), |(i, _)| i);
        self.input_line[start..end].width().try_into().unwrap()
    }

    fn byte_pos(&self) -> usize {
        self.input_line
            .char_indices()
            .nth(self.pos)
            .map_or(self.input_line.len(), |(i, _)| i)
    }

    fn visible_input(&self) -> &str {
        let start = self
            .input_line
            .char_indices()
            .nth(self.scroll)
            .map_or(self.input_line.len(), |(i, _)| i);
        let mut end = self.input_line.len();
        let mut width = 0;
        for (i, c) in self.input_line.char_indices().skip(self.scroll) {
            if width >= usize::from(self.size.1) - 2 {
                end = i;
                break;
            }
            width += c.width().unwrap_or(1);
        }
        &self.input_line[start..end]
    }
}
