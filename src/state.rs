use textmode::Textmode as _;

pub struct State {
    readline: crate::readline::Readline,
    history: crate::history::History,
    focus: Focus,
    output: textmode::Output,
    escape: bool,
    hide_readline: bool,
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
            escape: false,
            hide_readline: false,
            action,
        }
    }

    pub async fn render(&mut self, hard: bool) -> anyhow::Result<()> {
        self.output.clear();
        match self.focus {
            Focus::Readline => {
                self.history
                    .render(&mut self.output, self.readline.lines(), None)
                    .await?;
                self.readline.render(&mut self.output, true).await?;
            }
            Focus::History(idx) => {
                if self.hide_readline {
                    self.history
                        .render(&mut self.output, 0, Some(idx))
                        .await?;
                } else {
                    self.history
                        .render(
                            &mut self.output,
                            self.readline.lines(),
                            Some(idx),
                        )
                        .await?;
                    let pos = self.output.screen().cursor_position();
                    self.readline.render(&mut self.output, false).await?;
                    self.output.move_to(pos.0, pos.1);
                }
            }
        }
        if hard {
            self.output.hard_refresh().await?;
        } else {
            self.output.refresh().await?;
        }
        Ok(())
    }

    pub async fn handle_action(&mut self, action: crate::action::Action) {
        match action {
            crate::action::Action::Render => {
                self.render(false).await.unwrap();
            }
            crate::action::Action::ForceRedraw => {
                self.render(true).await.unwrap();
            }
            crate::action::Action::Run(ref cmd) => {
                let idx = self.history.run(cmd).await.unwrap();
                self.focus = Focus::History(idx);
                self.hide_readline = true;
            }
            crate::action::Action::UpdateFocus(new_focus) => {
                self.focus = new_focus;
                self.hide_readline = false;
                self.render(false).await.unwrap();
            }
            crate::action::Action::Resize(new_size) => {
                self.readline.resize(new_size).await;
                self.history.resize(new_size).await;
                self.output.set_size(new_size.0, new_size.1);
                self.output.hard_refresh().await.unwrap();
                self.render(false).await.unwrap();
            }
            crate::action::Action::Quit => {
                // the debouncer should return None in this case
                unreachable!();
            }
        }
    }

    pub async fn handle_input(&mut self, key: textmode::Key) {
        if self.escape {
            let mut fallthrough = false;
            match key {
                textmode::Key::Ctrl(b'e') => {
                    fallthrough = true;
                }
                textmode::Key::Ctrl(b'l') => {
                    self.render(true).await.unwrap();
                }
                textmode::Key::Char('f') => {
                    if let Focus::History(idx) = self.focus {
                        self.history.toggle_fullscreen(idx).await;
                        self.render(false).await.unwrap();
                    }
                }
                textmode::Key::Char('j') => {
                    let new_focus = match self.focus {
                        Focus::History(idx) => {
                            if idx >= self.history.entry_count() - 1 {
                                Focus::Readline
                            } else {
                                Focus::History(idx + 1)
                            }
                        }
                        Focus::Readline => Focus::Readline,
                    };
                    self.focus = new_focus;
                    self.hide_readline = false;
                    self.render(false).await.unwrap();
                }
                textmode::Key::Char('k') => {
                    let new_focus = match self.focus {
                        Focus::History(idx) => {
                            if idx == 0 {
                                Focus::History(0)
                            } else {
                                Focus::History(idx - 1)
                            }
                        }
                        Focus::Readline => {
                            Focus::History(self.history.entry_count() - 1)
                        }
                    };
                    self.focus = new_focus;
                    self.hide_readline = false;
                    self.render(false).await.unwrap();
                }
                textmode::Key::Char('r') => {
                    self.focus = Focus::Readline;
                    self.hide_readline = false;
                    self.render(false).await.unwrap();
                }
                _ => {}
            }
            self.escape = false;
            if !fallthrough {
                return;
            }
        } else if key == textmode::Key::Ctrl(b'e') {
            self.escape = true;
            return;
        }

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
