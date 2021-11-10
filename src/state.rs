use textmode::Textmode as _;

pub struct State {
    readline: crate::readline::Readline,
    history: crate::history::History,
    focus: Focus,
    output: textmode::Output,
}

impl State {
    pub fn new(
        actions: async_std::channel::Sender<Action>,
        output: textmode::Output,
    ) -> Self {
        let readline = crate::readline::Readline::new(actions.clone());
        let history = crate::history::History::new(actions);
        let focus = Focus::Readline;
        Self {
            readline,
            history,
            focus,
            output,
        }
    }

    pub async fn render(&mut self) -> anyhow::Result<()> {
        self.output.clear();
        if let Focus::Readline = self.focus {
            self.history
                .render(&mut self.output, self.readline.lines())
                .await?;
            self.readline.render(&mut self.output).await?;
        } else {
            self.history.render(&mut self.output, 0).await?;
        }
        self.output.refresh().await?;
        Ok(())
    }

    pub async fn handle_action(&mut self, action: Action) {
        match action {
            Action::Render => {
                self.render().await.unwrap();
            }
            Action::Run(ref cmd) => {
                self.history.run(cmd).await.unwrap();
            }
            Action::UpdateFocus(new_focus) => {
                self.focus = new_focus;
                self.render().await.unwrap();
            }
        }
    }

    pub async fn handle_input(&mut self, key: textmode::Key) -> bool {
        match self.focus {
            Focus::Readline => self.readline.handle_key(key).await,
            Focus::History(idx) => self.history.handle_key(key, idx).await,
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Focus {
    Readline,
    History(usize),
}

#[derive(Debug)]
pub enum Action {
    Render,
    Run(String),
    UpdateFocus(Focus),
}
