use crate::shell::prelude::*;

use notify::Watcher as _;
use textmode::Textmode as _;

mod event;
mod git;
mod history;
mod prelude;
mod readline;

pub async fn main() -> anyhow::Result<i32> {
    let mut input = textmode::Input::new().await?;
    let mut output = textmode::Output::new().await?;

    // avoid the guards getting stuck in a task that doesn't run to
    // completion
    let _input_guard = input.take_raw_guard();
    let _output_guard = output.take_screen_guard();

    let (event_w, event_r) = async_std::channel::unbounded();

    {
        // nix::sys::signal::Signal is repr(i32)
        #[allow(clippy::as_conversions)]
        let signals = signal_hook_async_std::Signals::new(&[
            nix::sys::signal::Signal::SIGWINCH as i32,
        ])?;
        let event_w = event_w.clone();
        async_std::task::spawn(async move {
            // nix::sys::signal::Signal is repr(i32)
            #[allow(clippy::as_conversions)]
            let mut signals = async_std::stream::once(
                nix::sys::signal::Signal::SIGWINCH as i32,
            )
            .chain(signals);
            while signals.next().await.is_some() {
                event_w
                    .send(Event::Resize(
                        terminal_size::terminal_size().map_or(
                            (24, 80),
                            |(
                                terminal_size::Width(w),
                                terminal_size::Height(h),
                            )| { (h, w) },
                        ),
                    ))
                    .await
                    .unwrap();
            }
        });
    }

    {
        let event_w = event_w.clone();
        async_std::task::spawn(async move {
            while let Some(key) = input.read_key().await.unwrap() {
                event_w.send(Event::Key(key)).await.unwrap();
            }
        });
    }

    // redraw the clock every second
    {
        let event_w = event_w.clone();
        async_std::task::spawn(async move {
            let first_sleep = 1_000_000_000_u64.saturating_sub(
                time::OffsetDateTime::now_utc().nanosecond().into(),
            );
            async_std::task::sleep(std::time::Duration::from_nanos(
                first_sleep,
            ))
            .await;
            let mut interval = async_std::stream::interval(
                std::time::Duration::from_secs(1),
            );
            event_w.send(Event::ClockTimer).await.unwrap();
            while interval.next().await.is_some() {
                event_w.send(Event::ClockTimer).await.unwrap();
            }
        });
    }

    let (git_w, git_r): (async_std::channel::Sender<std::path::PathBuf>, _) =
        async_std::channel::unbounded();
    {
        let event_w = event_w.clone();
        let mut _active_watcher = None;
        async_std::task::spawn(async move {
            while let Ok(mut dir) = git_r.recv().await {
                while let Ok(newer_dir) = git_r.try_recv() {
                    dir = newer_dir;
                }
                let repo = git2::Repository::discover(&dir).ok();
                if repo.is_some() {
                    let (sync_watch_w, sync_watch_r) =
                        std::sync::mpsc::channel();
                    let (watch_w, watch_r) = async_std::channel::unbounded();
                    let mut watcher = notify::RecommendedWatcher::new(
                        sync_watch_w,
                        std::time::Duration::from_millis(100),
                    )
                    .unwrap();
                    watcher
                        .watch(&dir, notify::RecursiveMode::Recursive)
                        .unwrap();
                    async_std::task::spawn(blocking::unblock(move || {
                        while let Ok(event) = sync_watch_r.recv() {
                            let watch_w = watch_w.clone();
                            let send_failed =
                                async_std::task::block_on(async move {
                                    watch_w.send(event).await.is_err()
                                });
                            if send_failed {
                                break;
                            }
                        }
                    }));
                    let event_w = event_w.clone();
                    async_std::task::spawn(async move {
                        while watch_r.recv().await.is_ok() {
                            let repo = git2::Repository::discover(&dir).ok();
                            let info = blocking::unblock(|| {
                                repo.map(|repo| git::Info::new(&repo))
                            })
                            .await;
                            if event_w
                                .send(Event::GitInfo(info))
                                .await
                                .is_err()
                            {
                                break;
                            }
                        }
                    });
                    _active_watcher = Some(watcher);
                } else {
                    _active_watcher = None;
                }
                let info = blocking::unblock(|| {
                    repo.map(|repo| git::Info::new(&repo))
                })
                .await;
                event_w.send(Event::GitInfo(info)).await.unwrap();
            }
        });
    }

    let mut shell = Shell::new(crate::info::get_offset())?;
    let mut prev_dir = shell.env.current_dir().to_path_buf();
    git_w.send(prev_dir.clone()).await.unwrap();
    let event_reader = event::Reader::new(event_r);
    while let Some(event) = event_reader.recv().await {
        let dir = shell.env().current_dir();
        if dir != prev_dir {
            prev_dir = dir.to_path_buf();
            git_w.send(dir.to_path_buf()).await.unwrap();
        }
        match shell.handle_event(event, &event_w).await {
            Some(Action::Refresh) => {
                shell.render(&mut output).await?;
                output.refresh().await?;
            }
            Some(Action::HardRefresh) => {
                shell.render(&mut output).await?;
                output.hard_refresh().await?;
            }
            Some(Action::Resize(rows, cols)) => {
                output.set_size(rows, cols);
                shell.render(&mut output).await?;
                output.hard_refresh().await?;
            }
            Some(Action::Quit) => break,
            None => {}
        }
    }

    Ok(0)
}

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

pub struct Shell {
    readline: readline::Readline,
    history: history::History,
    env: Env,
    git: Option<git::Info>,
    focus: Focus,
    scene: Scene,
    escape: bool,
    hide_readline: bool,
    offset: time::UtcOffset,
}

impl Shell {
    pub fn new(offset: time::UtcOffset) -> anyhow::Result<Self> {
        let mut env = Env::new()?;
        env.set_var("SHELL", std::env::current_exe()?);
        env.set_var("TERM", "screen");
        Ok(Self {
            readline: readline::Readline::new(),
            history: history::History::new(),
            env,
            git: None,
            focus: Focus::Readline,
            scene: Scene::Readline,
            escape: false,
            hide_readline: false,
            offset,
        })
    }

    pub async fn render(
        &self,
        out: &mut impl textmode::Textmode,
    ) -> anyhow::Result<()> {
        out.clear();
        out.write(&vt100::Parser::default().screen().input_mode_formatted());
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
                            &self.env,
                            self.git.as_ref(),
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
                                &self.env,
                                self.git.as_ref(),
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
                            &self.env,
                            self.git.as_ref(),
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
        event: Event,
        event_w: &async_std::channel::Sender<Event>,
    ) -> Option<Action> {
        match event {
            Event::Key(key) => {
                return if self.escape {
                    self.escape = false;
                    self.handle_key_escape(key, event_w.clone()).await
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
                            self.handle_key_escape(key, event_w.clone()).await
                        }
                    }
                };
            }
            Event::Resize(new_size) => {
                self.readline.resize(new_size).await;
                self.history.resize(new_size).await;
                return Some(Action::Resize(new_size.0, new_size.1));
            }
            Event::PtyOutput => {
                // the number of visible lines may have changed, so make sure
                // the focus is still visible
                self.history
                    .make_focus_visible(
                        self.readline.lines(),
                        self.focus_idx(),
                        matches!(self.focus, Focus::Scrolling(_)),
                    )
                    .await;
                self.scene = self.default_scene(self.focus, None).await;
            }
            Event::PtyClose => {
                if let Some(idx) = self.focus_idx() {
                    let entry = self.history.entry(idx).await;
                    if !entry.running() {
                        if self.hide_readline {
                            let idx = self.env.idx();
                            self.env = entry.env().clone();
                            self.env.set_idx(idx);
                        }
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
            Event::ChildRunPipeline(idx, span) => {
                self.history.entry(idx).await.set_span(span);
            }
            Event::ChildSuspend(idx) => {
                if self.focus_idx() == Some(idx) {
                    self.set_focus(Focus::Readline, None).await;
                }
            }
            Event::GitInfo(info) => {
                self.git = info;
            }
            Event::ClockTimer => {}
        };
        Some(Action::Refresh)
    }

    async fn handle_key_escape(
        &mut self,
        key: textmode::Key,
        event_w: async_std::channel::Sender<Event>,
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
                if let Some(idx) = self.focus_idx() {
                    self.readline.clear_input();
                    let entry = self.history.entry(idx).await;
                    let input = entry.cmd();
                    let idx = self
                        .history
                        .run(input, &self.env, event_w.clone())
                        .await
                        .unwrap();
                    self.set_focus(Focus::History(idx), Some(entry)).await;
                    self.hide_readline = true;
                    self.env.set_idx(idx + 1);
                } else {
                    self.set_focus(Focus::Readline, None).await;
                }
            }
            textmode::Key::Char(' ') => {
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
            textmode::Key::Char('i') => {
                if let Some(idx) = self.focus_idx() {
                    let entry = self.history.entry(idx).await;
                    self.readline.set_input(entry.cmd());
                    self.set_focus(Focus::Readline, Some(entry)).await;
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
        event_w: async_std::channel::Sender<Event>,
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
                let input = self.readline.input();
                if !input.is_empty() {
                    let idx = self
                        .history
                        .run(input, &self.env, event_w.clone())
                        .await
                        .unwrap();
                    self.set_focus(Focus::History(idx), None).await;
                    self.hide_readline = true;
                    self.env.set_idx(idx + 1);
                    self.readline.clear_input();
                }
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
        self.history.send_input(idx, key.into_bytes()).await;
    }

    async fn default_scene(
        &self,
        focus: Focus,
        entry: Option<crate::mutex::Guard<history::Entry>>,
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
        entry: Option<crate::mutex::Guard<history::Entry>>,
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

    fn env(&self) -> &Env {
        &self.env
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
