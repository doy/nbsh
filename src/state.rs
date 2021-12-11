use textmode::Textmode as _;

pub struct State {
    readline: crate::readline::Readline,
    history: crate::history::History,
    focus: crate::action::Focus,
    scene: crate::action::Scene,
    escape: bool,
    hide_readline: bool,
    offset: time::UtcOffset,
}

impl State {
    pub fn new(offset: time::UtcOffset) -> Self {
        Self {
            readline: crate::readline::Readline::new(),
            history: crate::history::History::new(),
            focus: crate::action::Focus::Readline,
            scene: crate::action::Scene::Readline,
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
                crate::action::Focus::Readline => {
                    self.readline
                        .handle_key(key, self.history.entry_count())
                        .await
                }
                crate::action::Focus::History(idx) => {
                    self.history.handle_key(key, idx).await;
                    None
                }
                crate::action::Focus::Scrolling(_) => {
                    self.handle_key_escape(key).await
                }
            }
        }
    }

    async fn handle_key_escape(
        &mut self,
        key: textmode::Key,
    ) -> Option<crate::action::Action> {
        match key {
            textmode::Key::Ctrl(b'd') => {
                return Some(crate::action::Action::Quit);
            }
            textmode::Key::Ctrl(b'e') => {
                self.set_focus(
                    crate::action::Focus::Scrolling(self.focus_idx()),
                    None,
                )
                .await;
            }
            textmode::Key::Ctrl(b'l') => {
                return Some(crate::action::Action::ForceRedraw);
            }
            textmode::Key::Ctrl(b'm') => {
                let idx = self.focus_idx();
                let (focus, entry) = if let Some(idx) = idx {
                    let entry = self.history.entry(idx).await;
                    (entry.running(), Some(entry))
                } else {
                    (true, None)
                };
                if focus {
                    self.set_focus(
                        idx.map_or(crate::action::Focus::Readline, |idx| {
                            crate::action::Focus::History(idx)
                        }),
                        entry,
                    )
                    .await;
                }
            }
            textmode::Key::Char(' ') => {
                if let Some(idx) = self.focus_idx() {
                    let entry = self.history.entry(idx).await;
                    self.readline.set_input(&entry.cmd());
                    self.set_focus(
                        crate::action::Focus::Readline,
                        Some(entry),
                    )
                    .await;
                }
            }
            textmode::Key::Char('e') => {
                if let crate::action::Focus::History(idx) = self.focus {
                    self.history
                        .handle_key(textmode::Key::Ctrl(b'e'), idx)
                        .await;
                }
            }
            textmode::Key::Char('f') => {
                if let Some(idx) = self.focus_idx() {
                    let mut entry = self.history.entry(idx).await;
                    let mut focus = crate::action::Focus::History(idx);
                    if let crate::action::Focus::Scrolling(_) = self.focus {
                        entry.set_fullscreen(true);
                    } else {
                        entry.toggle_fullscreen();
                        if !entry.should_fullscreen() && !entry.running() {
                            focus =
                                crate::action::Focus::Scrolling(Some(idx));
                        }
                    }
                    self.set_focus(focus, Some(entry)).await;
                }
            }
            textmode::Key::Char('j') | textmode::Key::Down => {
                self.set_focus(
                    crate::action::Focus::Scrolling(
                        self.scroll_down(self.focus_idx()),
                    ),
                    None,
                )
                .await;
            }
            textmode::Key::Char('k') | textmode::Key::Up => {
                self.set_focus(
                    crate::action::Focus::Scrolling(
                        self.scroll_up(self.focus_idx()),
                    ),
                    None,
                )
                .await;
            }
            textmode::Key::Char('r') => {
                self.set_focus(crate::action::Focus::Readline, None).await;
            }
            _ => {}
        }
        Some(crate::action::Action::Render)
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        hard: bool,
    ) -> anyhow::Result<()> {
        out.clear();
        match self.scene {
            crate::action::Scene::Readline => match self.focus {
                crate::action::Focus::Readline => {
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
                crate::action::Focus::History(idx) => {
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
                crate::action::Focus::Scrolling(idx) => {
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
            crate::action::Scene::Fullscreen => {
                if let crate::action::Focus::History(idx) = self.focus {
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
            crate::action::Action::Render => {
                // for instance, if we are rerendering because of command
                // output, that output could increase the number of lines of
                // output of a command, pushing the currently focused entry
                // off the top of the screen
                self.history
                    .make_focus_visible(
                        self.readline.lines(),
                        self.focus_idx(),
                        matches!(
                            self.focus,
                            crate::action::Focus::Scrolling(_)
                        ),
                    )
                    .await;
            }
            crate::action::Action::ForceRedraw => {
                hard_refresh = true;
            }
            crate::action::Action::Run(ref cmd) => {
                let idx =
                    self.history.run(cmd, action_w.clone()).await.unwrap();
                self.set_focus(crate::action::Focus::History(idx), None)
                    .await;
                self.hide_readline = true;
            }
            crate::action::Action::UpdateFocus(new_focus) => {
                self.set_focus(new_focus, None).await;
            }
            crate::action::Action::UpdateScene(new_scene) => {
                self.scene = new_scene;
            }
            crate::action::Action::CheckUpdateScene => {
                self.scene = self.default_scene(self.focus, None).await;
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

    async fn default_scene(
        &self,
        focus: crate::action::Focus,
        entry: Option<async_std::sync::MutexGuardArc<crate::history::Entry>>,
    ) -> crate::action::Scene {
        match focus {
            crate::action::Focus::Readline
            | crate::action::Focus::Scrolling(_) => {
                crate::action::Scene::Readline
            }
            crate::action::Focus::History(idx) => {
                let fullscreen = if let Some(entry) = entry {
                    entry.should_fullscreen()
                } else {
                    self.history.entry(idx).await.should_fullscreen()
                };
                if fullscreen {
                    crate::action::Scene::Fullscreen
                } else {
                    crate::action::Scene::Readline
                }
            }
        }
    }

    async fn set_focus(
        &mut self,
        new_focus: crate::action::Focus,
        entry: Option<async_std::sync::MutexGuardArc<crate::history::Entry>>,
    ) {
        self.focus = new_focus;
        self.hide_readline = false;
        self.scene = self.default_scene(new_focus, entry).await;
        // passing entry into default_scene above consumes it, which means
        // that the mutex lock will be dropped before we call into
        // make_focus_visible, which is important because otherwise we might
        // get a deadlock depending on what is visible
        self.history
            .make_focus_visible(
                self.readline.lines(),
                self.focus_idx(),
                matches!(self.focus, crate::action::Focus::Scrolling(_)),
            )
            .await;
    }

    fn focus_idx(&self) -> Option<usize> {
        match self.focus {
            crate::action::Focus::History(idx) => Some(idx),
            crate::action::Focus::Readline => None,
            crate::action::Focus::Scrolling(idx) => idx,
        }
    }

    fn scroll_up(&self, idx: Option<usize>) -> Option<usize> {
        idx.map_or_else(
            || {
                let count = self.history.entry_count();
                if count == 0 {
                    None
                } else {
                    Some(count - 1)
                }
            },
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
