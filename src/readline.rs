use textmode::Textmode as _;
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

pub struct Readline {
    size: (u16, u16),
    input_line: String,
    pos: usize,
}

impl Readline {
    pub fn new() -> Self {
        Self {
            size: (24, 80),
            input_line: "".into(),
            pos: 0,
        }
    }

    pub async fn handle_key(
        &mut self,
        key: textmode::Key,
    ) -> Option<crate::action::Action> {
        match key {
            textmode::Key::String(s) => self.add_input(&s),
            textmode::Key::Char(c) => {
                self.add_input(&c.to_string());
            }
            textmode::Key::Ctrl(b'c') => self.clear_input(),
            textmode::Key::Ctrl(b'd') => {
                return Some(crate::action::Action::Quit);
            }
            textmode::Key::Ctrl(b'l') => {
                return Some(crate::action::Action::ForceRedraw);
            }
            textmode::Key::Ctrl(b'm') => {
                let cmd = self.input();
                self.clear_input();
                return Some(crate::action::Action::Run(cmd));
            }
            textmode::Key::Ctrl(b'u') => self.clear_backwards(),
            textmode::Key::Backspace => self.backspace(),
            textmode::Key::Left => self.cursor_left(),
            textmode::Key::Right => self.cursor_right(),
            textmode::Key::Up => {
                return Some(crate::action::Action::UpdateFocus(
                    crate::action::Focus::Scrolling(Some(usize::MAX)),
                ))
            }
            _ => {}
        }
        Some(crate::action::Action::Render)
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        focus: bool,
        offset: time::UtcOffset,
    ) -> anyhow::Result<()> {
        let pwd = crate::env::pwd()?;
        let user = crate::env::user()?;
        let hostname = crate::env::hostname()?;
        let time = crate::env::time(offset)?;
        let prompt_char = crate::env::prompt_char()?;

        let id = format!("{}@{}", user, hostname);
        let idlen: u16 = id.len().try_into().unwrap();
        let timelen: u16 = time.len().try_into().unwrap();

        out.move_to(self.size.0 - 2, 0);
        if focus {
            out.set_bgcolor(textmode::Color::Rgb(32, 32, 64));
        } else {
            out.set_bgcolor(textmode::Color::Rgb(32, 32, 32));
        }
        out.write(b"\x1b[K");
        out.write_str(" (");
        out.write_str(&pwd);
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
        out.write_str(&self.input_line);
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

    pub fn lines(&self) -> usize {
        2 // XXX handle wrapping
    }

    fn input(&self) -> String {
        self.input_line.clone()
    }

    fn add_input(&mut self, s: &str) {
        self.input_line.insert_str(self.byte_pos(), s);
        self.pos += s.chars().count();
    }

    fn backspace(&mut self) {
        while self.pos > 0 {
            self.pos -= 1;
            let width =
                self.input_line.remove(self.byte_pos()).width().unwrap_or(0);
            if width > 0 {
                break;
            }
        }
    }

    fn clear_input(&mut self) {
        self.input_line.clear();
        self.pos = 0;
    }

    fn clear_backwards(&mut self) {
        self.input_line = self.input_line.chars().skip(self.pos).collect();
        self.pos = 0;
    }

    fn cursor_left(&mut self) {
        if self.pos == 0 {
            return;
        }
        self.pos -= 1;
        while let Some(c) = self.input_line.chars().nth(self.pos) {
            if c.width().unwrap_or(0) == 0 {
                self.pos -= 1;
            } else {
                break;
            }
        }
    }

    fn cursor_right(&mut self) {
        if self.pos == self.input_line.chars().count() {
            return;
        }
        self.pos += 1;
        while let Some(c) = self.input_line.chars().nth(self.pos) {
            if c.width().unwrap_or(0) == 0 {
                self.pos += 1;
            } else {
                break;
            }
        }
    }

    fn pos_width(&self) -> u16 {
        self.input_line
            .chars()
            .take(self.pos)
            .collect::<String>()
            .width()
            .try_into()
            .unwrap()
    }

    fn byte_pos(&self) -> usize {
        self.input_line
            .char_indices()
            .nth(self.pos)
            .map_or(self.input_line.len(), |(i, _)| i)
    }
}
