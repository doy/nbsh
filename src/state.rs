use textmode::Textmode as _;

pub struct State {
    readline: crate::readline::Readline,
    history: crate::history::History,
    focus: Focus,
    scene: Scene,
    escape: bool,
    hide_readline: bool,
    offset: time::UtcOffset,
}

impl State {
    pub fn new(offset: time::UtcOffset) -> Self {
        Self {
            readline: crate::readline::Readline::new(),
            history: crate::history::History::new(),
            focus: Focus::Readline,
            scene: Scene::Readline,
            escape: false,
            hide_readline: false,
            offset,
        }
    }

    pub async fn handle_key(
        &mut self,
        key: textmode::Key,
    ) -> Option<crate::action::Action> {
        if self.escape {
            self.escape = false;
            self.handle_key_escape(key).await
        } else if key == textmode::Key::Ctrl(b'e') {
            self.escape = true;
            None
        } else {
            match self.focus {
                Focus::Readline => self.readline.handle_key(key).await,
                Focus::History(idx) => {
                    self.history.handle_key(key, idx).await;
                    None
                }
                Focus::Scrolling(idx) => {
                    self.handle_key_scrolling(key, idx).await
                }
            }
        }
    }

    async fn handle_key_escape(
        &mut self,
        key: textmode::Key,
    ) -> Option<crate::action::Action> {
        match key {
            textmode::Key::Ctrl(b'e') => {
                if let Focus::History(idx) = self.focus {
                    self.history.handle_key(key, idx).await;
                }
                None
            }
            textmode::Key::Ctrl(b'l') => {
                Some(crate::action::Action::ForceRedraw)
            }
            textmode::Key::Char('f') => {
                if let Focus::History(idx) = self.focus {
                    self.history.toggle_fullscreen(idx).await;
                    Some(crate::action::Action::CheckUpdateScene)
                } else {
                    None
                }
            }
            textmode::Key::Char('j') | textmode::Key::Down => {
                Some(crate::action::Action::UpdateFocus(Focus::Scrolling(
                    self.scroll_down(self.focus_idx()),
                )))
            }
            textmode::Key::Char('k') | textmode::Key::Up => {
                Some(crate::action::Action::UpdateFocus(Focus::Scrolling(
                    self.scroll_up(self.focus_idx()),
                )))
            }
            textmode::Key::Char('r') => {
                Some(crate::action::Action::UpdateFocus(Focus::Readline))
            }
            _ => None,
        }
    }

    async fn handle_key_scrolling(
        &mut self,
        key: textmode::Key,
        idx: Option<usize>,
    ) -> Option<crate::action::Action> {
        match key {
            textmode::Key::Ctrl(b'm') => {
                let focus = if let Some(idx) = idx {
                    self.history.running(idx).await
                } else {
                    true
                };
                if focus {
                    Some(crate::action::Action::UpdateFocus(
                        idx.map_or(Focus::Readline, |idx| {
                            Focus::History(idx)
                        }),
                    ))
                } else {
                    None
                }
            }
            textmode::Key::Char('j') | textmode::Key::Down => {
                Some(crate::action::Action::UpdateFocus(Focus::Scrolling(
                    self.scroll_down(self.focus_idx()),
                )))
            }
            textmode::Key::Char('k') | textmode::Key::Up => {
                Some(crate::action::Action::UpdateFocus(Focus::Scrolling(
                    self.scroll_up(self.focus_idx()),
                )))
            }
            _ => None,
        }
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        hard: bool,
    ) -> anyhow::Result<()> {
        out.clear();
        match self.scene {
            Scene::Readline => match self.focus {
                Focus::Readline => {
                    self.history
                        .render(
                            out,
                            self.readline.lines(),
                            None,
                            false,
                            self.offset,
                        )
                        .await?;
                    self.readline.render(out, true, self.offset).await?;
                }
                Focus::History(idx) => {
                    if self.hide_readline {
                        self.history
                            .render(out, 0, Some(idx), false, self.offset)
                            .await?;
                    } else {
                        self.history
                            .render(
                                out,
                                self.readline.lines(),
                                Some(idx),
                                false,
                                self.offset,
                            )
                            .await?;
                        let pos = out.screen().cursor_position();
                        self.readline.render(out, false, self.offset).await?;
                        out.move_to(pos.0, pos.1);
                    }
                }
                Focus::Scrolling(idx) => {
                    self.history
                        .render(
                            out,
                            self.readline.lines(),
                            idx,
                            true,
                            self.offset,
                        )
                        .await?;
                    self.readline
                        .render(out, idx.is_none(), self.offset)
                        .await?;
                    out.hide_cursor(true);
                }
            },
            Scene::Fullscreen => {
                if let Focus::History(idx) = self.focus {
                    self.history.render_fullscreen(out, idx).await;
                } else {
                    unreachable!();
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
        let mut hard_refresh = false;
        match action {
            crate::action::Action::Render => {}
            crate::action::Action::ForceRedraw => {
                hard_refresh = true;
            }
            crate::action::Action::Run(ref cmd) => {
                let idx =
                    self.history.run(cmd, action_w.clone()).await.unwrap();
                self.focus = Focus::History(idx);
                self.hide_readline = true;
            }
            crate::action::Action::UpdateFocus(mut new_focus) => {
                match new_focus {
                    Focus::Readline | Focus::Scrolling(None) => {}
                    Focus::History(ref mut idx)
                    | Focus::Scrolling(Some(ref mut idx)) => {
                        if *idx >= self.history.entry_count() {
                            *idx = self.history.entry_count() - 1;
                        }
                    }
                }
                self.focus = new_focus;
                self.hide_readline = false;
                self.scene = self.default_scene(new_focus).await;
            }
            crate::action::Action::UpdateScene(new_scene) => {
                self.scene = new_scene;
            }
            crate::action::Action::CheckUpdateScene => {
                self.scene = self.default_scene(self.focus).await;
            }
            crate::action::Action::Resize(new_size) => {
                self.readline.resize(new_size).await;
                self.history.resize(new_size).await;
                out.set_size(new_size.0, new_size.1);
                out.hard_refresh().await.unwrap();
            }
            crate::action::Action::Quit => {
                // the debouncer should return None in this case
                unreachable!();
            }
        }
        self.render(out, hard_refresh).await.unwrap();
    }

    async fn default_scene(&self, focus: Focus) -> Scene {
        match focus {
            Focus::Readline | Focus::Scrolling(_) => Scene::Readline,
            Focus::History(idx) => {
                if self.history.should_fullscreen(idx).await {
                    Scene::Fullscreen
                } else {
                    Scene::Readline
                }
            }
        }
    }

    fn focus_idx(&self) -> Option<usize> {
        match self.focus {
            Focus::History(idx) => Some(idx),
            Focus::Readline => None,
            Focus::Scrolling(idx) => idx,
        }
    }

    fn scroll_up(&self, idx: Option<usize>) -> Option<usize> {
        idx.map_or_else(
            || Some(self.history.entry_count() - 1),
            |idx| Some(idx.saturating_sub(1)),
        )
    }

    fn scroll_down(&self, idx: Option<usize>) -> Option<usize> {
        idx.and_then(|idx| {
            if idx >= self.history.entry_count() - 1 {
                None
            } else {
                Some(idx + 1)
            }
        })
    }
}

#[derive(Copy, Clone, Debug)]
pub enum Focus {
    Readline,
    History(usize),
    Scrolling(Option<usize>),
}

#[derive(Copy, Clone, Debug)]
pub enum Scene {
    Readline,
    Fullscreen,
}
