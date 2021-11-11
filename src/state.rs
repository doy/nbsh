use textmode::Textmode as _;

pub struct State {
    readline: crate::readline::Readline,
    history: crate::history::History,
    focus: Focus,
    output: textmode::Output,
    action: async_std::channel::Sender<crate::action::Action>,
}

impl State {
    pub fn new(
        action: async_std::channel::Sender<crate::action::Action>,
        output: textmode::Output,
    ) -> Self {
        let readline = crate::readline::Readline::new(action.clone());
        let history = crate::history::History::new(action.clone());
        let focus = Focus::Readline;
        Self {
            readline,
            history,
            focus,
            output,
            action,
        }
    }

    pub async fn render(&mut self) -> anyhow::Result<()> {
        self.output.clear();
        match self.focus {
            Focus::Readline => {
                self.history
                    .render(&mut self.output, self.readline.lines(), None)
                    .await?;
                self.readline.render(&mut self.output).await?;
            }
            Focus::History(idx) => {
                self.history.render(&mut self.output, 0, Some(idx)).await?;
            }
        }
        self.output.refresh().await?;
        Ok(())
    }

    pub async fn handle_action(&mut self, action: crate::action::Action) {
        match action {
            crate::action::Action::Render => {
                self.render().await.unwrap();
            }
            crate::action::Action::Run(ref cmd) => {
                self.history.run(cmd).await.unwrap();
            }
            crate::action::Action::UpdateFocus(new_focus) => {
                self.focus = new_focus;
                self.render().await.unwrap();
            }
            crate::action::Action::Resize(new_size) => {
                self.readline.resize(new_size).await;
                self.history.resize(new_size).await;
                self.output.set_size(new_size.0, new_size.1);
                self.output.hard_refresh().await.unwrap();
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

    pub async fn resize(&mut self) {
        let size = terminal_size::terminal_size().map_or(
            (24, 80),
            |(terminal_size::Width(w), terminal_size::Height(h))| (h, w),
        );
        self.action
            .send(crate::action::Action::Resize(size))
            .await
            .unwrap();
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Focus {
    Readline,
    History(usize),
}
