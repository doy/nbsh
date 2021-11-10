use textmode::Textmode as _;

pub struct Repl {
    prompt: String,
    input_line: String,
}

impl Repl {
    pub fn new() -> Self {
        Self {
            prompt: "$ ".into(),
            input_line: "".into(),
        }
    }

    pub fn input(&self) -> String {
        self.input_line.clone()
    }

    pub fn add_input(&mut self, s: &str) {
        self.input_line.push_str(s);
    }

    pub fn backspace(&mut self) {
        self.input_line.pop();
    }

    pub fn clear_input(&mut self) {
        self.input_line.clear();
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
}
