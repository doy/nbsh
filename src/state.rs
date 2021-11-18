use textmode::Textmode as _;

pub struct State {
    readline: crate::readline::Readline,
    history: crate::history::History,
    focus: Focus,
    escape: bool,
    hide_readline: bool,
}

impl State {
    pub fn new() -> Self {
        let readline = crate::readline::Readline::new();
        let history = crate::history::History::new();
        let focus = Focus::Readline;
        Self {
            readline,
            history,
            focus,
            escape: false,
            hide_readline: false,
        }
    }

    pub async fn handle_key(
        &mut self,
        key: textmode::Key,
    ) -> Option<crate::action::Action> {
        if self.escape {
            self.escape = false;
            let mut fallthrough = false;
            match key {
                textmode::Key::Ctrl(b'e') => {
                    fallthrough = true;
                }
                textmode::Key::Ctrl(b'l') => {
                    return Some(crate::action::Action::ForceRedraw);
                }
                textmode::Key::Char('f') => {
                    if let Focus::History(idx) = self.focus {
                        return Some(
                            crate::action::Action::ToggleFullscreen(idx),
                        );
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
                    return Some(crate::action::Action::UpdateFocus(
                        new_focus,
                    ));
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
                    return Some(crate::action::Action::UpdateFocus(
                        new_focus,
                    ));
                }
                textmode::Key::Char('r') => {
                    return Some(crate::action::Action::UpdateFocus(
                        Focus::Readline,
                    ));
                }
                _ => {}
            }
            if !fallthrough {
                return None;
            }
        } else if key == textmode::Key::Ctrl(b'e') {
            self.escape = true;
            return None;
        }

        match self.focus {
            Focus::Readline => self.readline.handle_key(key).await,
            Focus::History(idx) => {
                self.history.handle_key(key, idx).await;
                None
            }
        }
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        hard: bool,
    ) -> anyhow::Result<()> {
        out.clear();
        match self.focus {
            Focus::Readline => {
                self.history
                    .render(out, self.readline.lines(), None)
                    .await?;
                self.readline.render(out, true).await?;
            }
            Focus::History(idx) => {
                if self.hide_readline || self.history.is_fullscreen(idx).await
                {
                    self.history.render(out, 0, Some(idx)).await?;
                } else {
                    self.history
                        .render(out, self.readline.lines(), Some(idx))
                        .await?;
                    let pos = out.screen().cursor_position();
                    self.readline.render(out, false).await?;
                    out.move_to(pos.0, pos.1);
                }
            }
        }
        if hard {
            out.hard_refresh().await?;
        } else {
            out.refresh().await?;
        }
        Ok(())
    }

    pub async fn handle_action(
        &mut self,
        action: crate::action::Action,
        out: &mut textmode::Output,
        action_w: &async_std::channel::Sender<crate::action::Action>,
    ) {
        match action {
            crate::action::Action::Render => {
                self.render(out, false).await.unwrap();
            }
            crate::action::Action::ForceRedraw => {
                self.render(out, true).await.unwrap();
            }
            crate::action::Action::Run(ref cmd) => {
                let idx =
                    self.history.run(cmd, action_w.clone()).await.unwrap();
                self.focus = Focus::History(idx);
                self.hide_readline = true;
                self.render(out, false).await.unwrap();
            }
            crate::action::Action::UpdateFocus(new_focus) => {
                self.focus = new_focus;
                self.hide_readline = false;
                self.render(out, false).await.unwrap();
            }
            crate::action::Action::ToggleFullscreen(idx) => {
                self.history.toggle_fullscreen(idx).await;
                self.render(out, false).await.unwrap();
            }
            crate::action::Action::Resize(new_size) => {
                self.readline.resize(new_size).await;
                self.history.resize(new_size).await;
                out.set_size(new_size.0, new_size.1);
                out.hard_refresh().await.unwrap();
                self.render(out, false).await.unwrap();
            }
            crate::action::Action::Quit => {
                // the debouncer should return None in this case
                unreachable!();
            }
        }
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Focus {
    Readline,
    History(usize),
}
