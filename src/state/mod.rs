mod history;
mod readline;

#[derive(Copy, Clone, Debug)]
enum Focus {
    Readline,
    History(usize),
    Scrolling(Option<usize>),
}

#[derive(Copy, Clone, Debug)]
enum Scene {
    Readline,
    Fullscreen,
}

pub enum Action {
    Refresh,
    HardRefresh,
    Resize(u16, u16),
    Quit,
}

pub struct State {
    readline: readline::Readline,
    history: history::History,
    focus: Focus,
    scene: Scene,
    escape: bool,
    hide_readline: bool,
    offset: time::UtcOffset,
}

impl State {
    pub fn new(offset: time::UtcOffset) -> Self {
        Self {
            readline: readline::Readline::new(),
            history: history::History::new(),
            focus: Focus::Readline,
            scene: Scene::Readline,
            escape: false,
            hide_readline: false,
            offset,
        }
    }

    // render always happens on the main task
    #[allow(clippy::future_not_send)]
    pub async fn render(
        &self,
        out: &mut impl textmode::Textmode,
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
                    self.readline
                        .render(
                            out,
                            self.history.entry_count(),
                            true,
                            self.offset,
                        )
                        .await?;
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
                        self.readline
                            .render(
                                out,
                                self.history.entry_count(),
                                false,
                                self.offset,
                            )
                            .await?;
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
                        .render(
                            out,
                            self.history.entry_count(),
                            idx.is_none(),
                            self.offset,
                        )
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
        Ok(())
    }

    pub async fn handle_event(
        &mut self,
        event: crate::event::Event,
        event_w: &async_std::channel::Sender<crate::event::Event>,
    ) -> Option<Action> {
        match event {
            crate::event::Event::Key(key) => {
                return if self.escape {
                    self.escape = false;
                    self.handle_key_escape(key).await
                } else if key == textmode::Key::Ctrl(b'e') {
                    self.escape = true;
                    None
                } else {
                    match self.focus {
                        Focus::Readline => {
                            self.handle_key_readline(key, event_w.clone())
                                .await
                        }
                        Focus::History(idx) => {
                            self.handle_key_history(key, idx).await;
                            None
                        }
                        Focus::Scrolling(_) => {
                            self.handle_key_escape(key).await
                        }
                    }
                };
            }
            crate::event::Event::Resize(new_size) => {
                self.readline.resize(new_size).await;
                self.history.resize(new_size).await;
                return Some(Action::Resize(new_size.0, new_size.1));
            }
            crate::event::Event::ProcessOutput => {
                // the number of visible lines may have changed, so make sure
                // the focus is still visible
                self.history
                    .make_focus_visible(
                        self.readline.lines(),
                        self.focus_idx(),
                        matches!(self.focus, Focus::Scrolling(_)),
                    )
                    .await;
            }
            crate::event::Event::ProcessAlternateScreen => {
                self.scene = self.default_scene(self.focus, None).await;
            }
            crate::event::Event::ProcessExit => {
                if let Some(idx) = self.focus_idx() {
                    let entry = self.history.entry(idx).await;
                    if !entry.running() {
                        self.set_focus(
                            if self.hide_readline {
                                Focus::Readline
                            } else {
                                Focus::Scrolling(Some(idx))
                            },
                            Some(entry),
                        )
                        .await;
                    }
                }
            }
            crate::event::Event::ClockTimer => {}
        };
        Some(Action::Refresh)
    }

    async fn handle_key_escape(
        &mut self,
        key: textmode::Key,
    ) -> Option<Action> {
        match key {
            textmode::Key::Ctrl(b'd') => {
                return Some(Action::Quit);
            }
            textmode::Key::Ctrl(b'e') => {
                self.set_focus(Focus::Scrolling(self.focus_idx()), None)
                    .await;
            }
            textmode::Key::Ctrl(b'l') => {
                return Some(Action::HardRefresh);
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
                        idx.map_or(Focus::Readline, |idx| {
                            Focus::History(idx)
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
                    self.set_focus(Focus::Readline, Some(entry)).await;
                }
            }
            textmode::Key::Char('e') => {
                if let Focus::History(idx) = self.focus {
                    self.handle_key_history(textmode::Key::Ctrl(b'e'), idx)
                        .await;
                }
            }
            textmode::Key::Char('f') => {
                if let Some(idx) = self.focus_idx() {
                    let mut entry = self.history.entry(idx).await;
                    let mut focus = Focus::History(idx);
                    if let Focus::Scrolling(_) = self.focus {
                        entry.set_fullscreen(true);
                    } else {
                        entry.toggle_fullscreen();
                        if !entry.should_fullscreen() && !entry.running() {
                            focus = Focus::Scrolling(Some(idx));
                        }
                    }
                    self.set_focus(focus, Some(entry)).await;
                }
            }
            textmode::Key::Char('j') | textmode::Key::Down => {
                self.set_focus(
                    Focus::Scrolling(self.scroll_down(self.focus_idx())),
                    None,
                )
                .await;
            }
            textmode::Key::Char('k') | textmode::Key::Up => {
                self.set_focus(
                    Focus::Scrolling(self.scroll_up(self.focus_idx())),
                    None,
                )
                .await;
            }
            textmode::Key::Char('n') => {
                self.set_focus(self.next_running().await, None).await;
            }
            textmode::Key::Char('p') => {
                self.set_focus(self.prev_running().await, None).await;
            }
            textmode::Key::Char('r') => {
                self.set_focus(Focus::Readline, None).await;
            }
            _ => {
                return None;
            }
        }
        Some(Action::Refresh)
    }

    async fn handle_key_readline(
        &mut self,
        key: textmode::Key,
        event_w: async_std::channel::Sender<crate::event::Event>,
    ) -> Option<Action> {
        match key {
            textmode::Key::Char(c) => {
                self.readline.add_input(&c.to_string());
            }
            textmode::Key::Ctrl(b'c') => self.readline.clear_input(),
            textmode::Key::Ctrl(b'd') => {
                return Some(Action::Quit);
            }
            textmode::Key::Ctrl(b'l') => {
                return Some(Action::HardRefresh);
            }
            textmode::Key::Ctrl(b'm') => {
                let cmd = self.readline.input();
                self.readline.clear_input();
                let idx =
                    self.history.run(&cmd, event_w.clone()).await.unwrap();
                self.set_focus(Focus::History(idx), None).await;
                self.hide_readline = true;
            }
            textmode::Key::Ctrl(b'u') => self.readline.clear_backwards(),
            textmode::Key::Backspace => self.readline.backspace(),
            textmode::Key::Left => self.readline.cursor_left(),
            textmode::Key::Right => self.readline.cursor_right(),
            textmode::Key::Up => {
                let entry_count = self.history.entry_count();
                if entry_count > 0 {
                    self.set_focus(
                        Focus::Scrolling(Some(entry_count - 1)),
                        None,
                    )
                    .await;
                }
            }
            _ => return None,
        }
        Some(Action::Refresh)
    }

    async fn handle_key_history(&mut self, key: textmode::Key, idx: usize) {
        self.history
            .entry(idx)
            .await
            .send_input(key.into_bytes())
            .await;
    }

    async fn default_scene(
        &self,
        focus: Focus,
        entry: Option<async_std::sync::MutexGuardArc<history::Entry>>,
    ) -> Scene {
        match focus {
            Focus::Readline | Focus::Scrolling(_) => Scene::Readline,
            Focus::History(idx) => {
                let fullscreen = if let Some(entry) = entry {
                    entry.should_fullscreen()
                } else {
                    self.history.entry(idx).await.should_fullscreen()
                };
                if fullscreen {
                    Scene::Fullscreen
                } else {
                    Scene::Readline
                }
            }
        }
    }

    async fn set_focus(
        &mut self,
        new_focus: Focus,
        entry: Option<async_std::sync::MutexGuardArc<history::Entry>>,
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
                matches!(self.focus, Focus::Scrolling(_)),
            )
            .await;
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

    async fn next_running(&self) -> Focus {
        let count = self.history.entry_count();
        let cur = self.focus_idx().unwrap_or(count);
        for idx in ((cur + 1)..count).chain(0..cur) {
            if self.history.entry(idx).await.running() {
                return Focus::History(idx);
            }
        }
        self.focus_idx().map_or(Focus::Readline, Focus::History)
    }

    async fn prev_running(&self) -> Focus {
        let count = self.history.entry_count();
        let cur = self.focus_idx().unwrap_or(count);
        for idx in ((cur + 1)..count).chain(0..cur).rev() {
            if self.history.entry(idx).await.running() {
                return Focus::History(idx);
            }
        }
        self.focus_idx().map_or(Focus::Readline, Focus::History)
    }
}
