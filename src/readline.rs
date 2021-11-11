use textmode::Textmode as _;

pub struct Readline {
    size: (u16, u16),
    prompt: String,
    input_line: String,
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
            textmode::Key::Ctrl(b'm') => {
                self.action
                    .send(crate::action::Action::Run(self.input()))
                    .await
                    .unwrap();
                self.clear_input();
            }
            textmode::Key::Ctrl(b'u') => self.clear_backwards(),
            textmode::Key::Backspace => self.backspace(),
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
    ) -> anyhow::Result<()> {
        out.move_to(self.size.0 - 1, 0);
        out.write_str(&self.prompt);
        out.write_str(&self.input_line);
        Ok(())
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
    }

    fn input(&self) -> String {
        self.input_line.clone()
    }

    fn add_input(&mut self, s: &str) {
        self.input_line.push_str(s);
    }

    fn backspace(&mut self) {
        self.input_line.pop();
    }

    fn clear_input(&mut self) {
        self.input_line.clear();
    }

    fn clear_backwards(&mut self) {
        self.input_line.clear();
    }
}
