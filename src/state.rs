use textmode::Textmode as _;

pub struct State {
    readline: crate::readline::Readline,
    history: crate::history::History,
    focus: Focus,
}

impl State {
    pub fn new(actions: async_std::channel::Sender<Action>) -> Self {
        let readline = crate::readline::Readline::new(actions.clone());
        let history = crate::history::History::new(actions);
        let focus = Focus::Readline;
        Self {
            readline,
            history,
            focus,
        }
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
    ) -> anyhow::Result<()> {
        out.clear();
        if let Focus::Readline = self.focus {
            self.history.render(out, self.readline.lines()).await?;
            self.readline.render(out).await?;
        } else {
            self.history.render(out, 0).await?;
        }
        out.refresh().await?;
        Ok(())
    }

    pub async fn handle_action(
        &mut self,
        action: Action,
        output: &mut textmode::Output,
    ) {
        match action {
            Action::Render => {
                self.render(output).await.unwrap();
            }
            Action::Run(ref cmd) => {
                self.history.run(cmd).await.unwrap();
            }
            Action::UpdateFocus(new_focus) => {
                self.focus = new_focus;
                self.render(output).await.unwrap();
            }
        }
    }

    pub async fn handle_input(&mut self, key: Option<textmode::Key>) -> bool {
        if let Some(key) = key {
            let quit = match self.focus {
                Focus::Readline => self.readline.handle_key(key).await,
                Focus::History(idx) => {
                    self.history.handle_key(key, idx).await
                }
            };
            if quit {
                return true;
            }
        } else {
            return true;
        }
        false
    }
}

#[derive(Copy, Clone)]
pub enum Focus {
    Readline,
    History(usize),
}

pub enum Action {
    Render,
    Run(String),
    UpdateFocus(Focus),
}
