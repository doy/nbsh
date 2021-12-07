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
        let readline = crate::readline::Readline::new();
        let history = crate::history::History::new();
        let focus = Focus::Readline;
        let scene = Scene::Readline;
        Self {
            readline,
            history,
            focus,
            scene,
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
                        self.history.toggle_fullscreen(idx).await;
                        return Some(crate::action::Action::CheckUpdateScene);
                    }
                }
                textmode::Key::Char('j') => {
                    let new_focus = match self.focus {
                        Focus::History(idx) => {
                            if idx >= self.history.entry_count() - 1 {
                                Focus::Scrolling(None)
                            } else {
                                Focus::Scrolling(Some(idx + 1))
                            }
                        }
                        Focus::Readline => Focus::Scrolling(None),
                        Focus::Scrolling(idx) => {
                            idx.map_or(Focus::Scrolling(None), |idx| {
                                if idx >= self.history.entry_count() - 1 {
                                    Focus::Scrolling(None)
                                } else {
                                    Focus::Scrolling(Some(idx + 1))
                                }
                            })
                        }
                    };
                    return Some(crate::action::Action::UpdateFocus(
                        new_focus,
                    ));
                }
                textmode::Key::Char('k') => {
                    let new_focus = match self.focus {
                        Focus::History(idx) => {
                            if idx == 0 {
                                Focus::Scrolling(Some(0))
                            } else {
                                Focus::Scrolling(Some(idx - 1))
                            }
                        }
                        Focus::Readline => Focus::Scrolling(Some(
                            self.history.entry_count() - 1,
                        )),
                        Focus::Scrolling(idx) => idx.map_or(
                            Focus::Scrolling(Some(
                                self.history.entry_count() - 1,
                            )),
                            |idx| {
                                if idx == 0 {
                                    Focus::Scrolling(Some(0))
                                } else {
                                    Focus::Scrolling(Some(idx - 1))
                                }
                            },
                        ),
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
            Focus::Scrolling(idx) => match key {
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
                textmode::Key::Char('j') => {
                    let new_focus = match self.focus {
                        Focus::History(idx) => {
                            if idx >= self.history.entry_count() - 1 {
                                Focus::Scrolling(None)
                            } else {
                                Focus::Scrolling(Some(idx + 1))
                            }
                        }
                        Focus::Readline => Focus::Scrolling(None),
                        Focus::Scrolling(idx) => {
                            idx.map_or(Focus::Scrolling(None), |idx| {
                                if idx >= self.history.entry_count() - 1 {
                                    Focus::Scrolling(None)
                                } else {
                                    Focus::Scrolling(Some(idx + 1))
                                }
                            })
                        }
                    };
                    Some(crate::action::Action::UpdateFocus(new_focus))
                }
                textmode::Key::Char('k') => {
                    let new_focus = match self.focus {
                        Focus::History(idx) => {
                            if idx == 0 {
                                Focus::Scrolling(Some(0))
                            } else {
                                Focus::Scrolling(Some(idx - 1))
                            }
                        }
                        Focus::Readline => Focus::Scrolling(Some(
                            self.history.entry_count() - 1,
                        )),
                        Focus::Scrolling(idx) => idx.map_or(
                            Focus::Scrolling(Some(
                                self.history.entry_count() - 1,
                            )),
                            |idx| {
                                if idx == 0 {
                                    Focus::Scrolling(Some(0))
                                } else {
                                    Focus::Scrolling(Some(idx - 1))
                                }
                            },
                        ),
                    };
                    Some(crate::action::Action::UpdateFocus(new_focus))
                }
                _ => None,
            },
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
                        .render(out, self.readline.lines(), None, self.offset)
                        .await?;
                    self.readline.render(out, true, self.offset).await?;
                }
                Focus::History(idx) => {
                    if self.hide_readline {
                        self.history
                            .render(out, 0, Some(idx), self.offset)
                            .await?;
                    } else {
                        self.history
                            .render(
                                out,
                                self.readline.lines(),
                                Some(idx),
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
                        .render(out, self.readline.lines(), idx, self.offset)
                        .await?;
                    self.readline
                        .render(out, idx.is_none(), self.offset)
                        .await?;
                    out.write(b"\x1b[?25l");
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
            crate::action::Action::UpdateFocus(new_focus) => {
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
