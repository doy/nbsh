use textmode::Textmode as _;

pub struct Readline {
    prompt: String,
    input_line: String,
    action: async_std::channel::Sender<crate::state::Action>,
}

impl Readline {
    pub fn new(
        action: async_std::channel::Sender<crate::state::Action>,
    ) -> Self {
        Self {
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
                    .send(crate::state::Action::Run(self.input()))
                    .await
                    .unwrap();
                self.clear_input();
            }
            textmode::Key::Backspace => self.backspace(),
            _ => {}
        }
        self.action
            .send(crate::state::Action::Render)
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
        out.move_to(23, 0);
        out.write_str(&self.prompt);
        out.write_str(&self.input_line);
        Ok(())
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
}
