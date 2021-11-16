use textmode::Textmode as _;
use unicode_width::{UnicodeWidthChar as _, UnicodeWidthStr as _};

pub struct Readline {
    size: (u16, u16),
    prompt: String,
    input_line: String,
    pos: usize,
    action: async_std::channel::Sender<crate::action::Action>,
}

impl Readline {
    pub fn new(
        action: async_std::channel::Sender<crate::action::Action>,
    ) -> Self {
        Self {
            size: (24, 80),
            prompt: "$ ".into(),
            input_line: "".into(),
            pos: 0,
            action,
        }
    }

    pub async fn handle_key(&mut self, key: textmode::Key) -> bool {
        match key {
            textmode::Key::String(s) => self.add_input(&s),
            textmode::Key::Char(c) => {
                self.add_input(&c.to_string());
            }
            textmode::Key::Ctrl(b'c') => self.clear_input(),
            textmode::Key::Ctrl(b'd') => {
                return true;
            }
            textmode::Key::Ctrl(b'l') => {
                self.action
                    .send(crate::action::Action::ForceRedraw)
                    .await
                    .unwrap();
            }
            textmode::Key::Ctrl(b'm') => {
                self.action
                    .send(crate::action::Action::Run(self.input()))
                    .await
                    .unwrap();
                self.clear_input();
            }
            textmode::Key::Ctrl(b'u') => self.clear_backwards(),
            textmode::Key::Backspace => self.backspace(),
            textmode::Key::Left => self.cursor_left(),
            textmode::Key::Right => self.cursor_right(),
            _ => {}
        }
        self.action
            .send(crate::action::Action::Render)
            .await
            .unwrap();
        false
    }

    pub fn lines(&self) -> usize {
        1 // XXX handle wrapping, multiline prompts
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        focus: bool,
    ) -> anyhow::Result<()> {
        out.move_to(self.size.0 - 1, 0);
        if focus {
            out.set_fgcolor(textmode::color::BLACK);
            out.set_bgcolor(textmode::color::CYAN);
        } else {
            out.set_bgcolor(textmode::Color::Rgb(32, 32, 32));
        }
        out.write_str(&self.prompt);
        out.reset_attributes();
        out.set_bgcolor(textmode::Color::Rgb(32, 32, 32));
        out.write_str(&self.input_line);
        out.write_str(
            &" ".repeat(
                (self.size.1 - self.prompt_width() - self.input_line_width())
                    .try_into()
                    .unwrap(),
            ),
        );
        out.reset_attributes();
        out.move_to(self.size.0 - 1, self.prompt_width() + self.pos_width());
        Ok(())
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
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

    fn prompt_width(&self) -> u16 {
        self.prompt.width().try_into().unwrap()
    }

    fn input_line_width(&self) -> u16 {
        self.input_line.width().try_into().unwrap()
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
