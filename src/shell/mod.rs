use crate::shell::prelude::*;

use notify::Watcher as _;
use textmode::Textmode as _;

mod event;
mod git;
mod history;
mod prelude;
mod readline;

pub async fn main() -> Result<i32> {
    let mut input = textmode::blocking::Input::new()?;
    let mut output = textmode::Output::new().await?;

    // avoid the guards getting stuck in a task that doesn't run to
    // completion
    let _input_guard = input.take_raw_guard();
    let _output_guard = output.take_screen_guard();

    let (event_w, event_r) = event::channel();

    {
        let mut signals = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::window_change(),
        )?;
        let event_w = event_w.clone();
        tokio::task::spawn(async move {
            event_w.send(Event::Resize(
                terminal_size::terminal_size().map_or(
                    (24, 80),
                    |(terminal_size::Width(w), terminal_size::Height(h))| {
                        (h, w)
                    },
                ),
            ));
            while signals.recv().await.is_some() {
                event_w.send(Event::Resize(
                    terminal_size::terminal_size().map_or(
                        (24, 80),
                        |(
                            terminal_size::Width(w),
                            terminal_size::Height(h),
                        )| { (h, w) },
                    ),
                ));
            }
        });
    }

    {
        let event_w = event_w.clone();
        std::thread::spawn(move || {
            while let Some(key) = input.read_key().unwrap() {
                event_w.send(Event::Key(key));
            }
        });
    }

    // redraw the clock every second
    {
        let event_w = event_w.clone();
        tokio::task::spawn(async move {
            let now_clock = time::OffsetDateTime::now_utc();
            let now_instant = tokio::time::Instant::now();
            let mut interval = tokio::time::interval_at(
                now_instant
                    + std::time::Duration::from_nanos(
                        1_000_000_000_u64
                            .saturating_sub(now_clock.nanosecond().into()),
                    ),
                std::time::Duration::from_secs(1),
            );
            loop {
                interval.tick().await;
                event_w.send(Event::ClockTimer);
            }
        });
    }

    let (git_w, mut git_r): (
        tokio::sync::mpsc::UnboundedSender<std::path::PathBuf>,
        _,
    ) = tokio::sync::mpsc::unbounded_channel();
    {
        let event_w = event_w.clone();
        // clippy can't tell that we assign to this later
        #[allow(clippy::no_effect_underscore_binding)]
        let mut _active_watcher = None;
        tokio::task::spawn(async move {
            while let Some(mut dir) = git_r.recv().await {
                while let Ok(newer_dir) = git_r.try_recv() {
                    dir = newer_dir;
                }
                let repo = git2::Repository::discover(&dir).ok();
                if repo.is_some() {
                    let (sync_watch_w, sync_watch_r) =
                        std::sync::mpsc::channel();
                    let (watch_w, mut watch_r) =
                        tokio::sync::mpsc::unbounded_channel();
                    let mut watcher = notify::RecommendedWatcher::new(
                        sync_watch_w,
                        std::time::Duration::from_millis(100),
                    )
                    .unwrap();
                    watcher
                        .watch(&dir, notify::RecursiveMode::Recursive)
                        .unwrap();
                    tokio::task::spawn_blocking(move || {
                        while let Ok(event) = sync_watch_r.recv() {
                            let watch_w = watch_w.clone();
                            let send_failed = watch_w.send(event).is_err();
                            if send_failed {
                                break;
                            }
                        }
                    });
                    let event_w = event_w.clone();
                    tokio::task::spawn(async move {
                        while watch_r.recv().await.is_some() {
                            let repo = git2::Repository::discover(&dir).ok();
                            let info = tokio::task::spawn_blocking(|| {
                                repo.map(|repo| git::Info::new(&repo))
                            })
                            .await
                            .unwrap();
                            event_w.send(Event::GitInfo(info));
                        }
                    });
                    _active_watcher = Some(watcher);
                } else {
                    _active_watcher = None;
                }
                let info = tokio::task::spawn_blocking(|| {
                    repo.map(|repo| git::Info::new(&repo))
                })
                .await
                .unwrap();
                event_w.send(Event::GitInfo(info));
            }
        });
    }

    let mut shell = Shell::new(crate::info::get_offset())?;
    let mut prev_dir = shell.env.pwd().to_path_buf();
    git_w.send(prev_dir.clone()).unwrap();
    while let Some(event) = event_r.recv().await {
        let dir = shell.env().pwd();
        if dir != prev_dir {
            prev_dir = dir.to_path_buf();
            git_w.send(dir.to_path_buf()).unwrap();
        }
        match shell.handle_event(event, &event_w) {
            Some(Action::Refresh) => {
                shell.render(&mut output)?;
                output.refresh().await?;
            }
            Some(Action::HardRefresh) => {
                shell.render(&mut output)?;
                output.hard_refresh().await?;
            }
            Some(Action::Resize(rows, cols)) => {
                output.set_size(rows, cols);
                shell.render(&mut output)?;
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
    pub fn new(offset: time::UtcOffset) -> Result<Self> {
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

    pub fn render(&self, out: &mut impl textmode::Textmode) -> Result<()> {
        out.clear();
        out.write(&vt100::Parser::default().screen().input_mode_formatted());
        match self.scene {
            Scene::Readline => match self.focus {
                Focus::Readline => {
                    self.history.render(
                        out,
                        self.readline.lines(),
                        None,
                        false,
                        self.offset,
                    );
                    self.readline.render(
                        out,
                        &self.env,
                        self.git.as_ref(),
                        true,
                        self.offset,
                    )?;
                }
                Focus::History(idx) => {
                    if self.hide_readline {
                        self.history.render(
                            out,
                            0,
                            Some(idx),
                            false,
                            self.offset,
                        );
                    } else {
                        self.history.render(
                            out,
                            self.readline.lines(),
                            Some(idx),
                            false,
                            self.offset,
                        );
                        let pos = out.screen().cursor_position();
                        self.readline.render(
                            out,
                            &self.env,
                            self.git.as_ref(),
                            false,
                            self.offset,
                        )?;
                        out.move_to(pos.0, pos.1);
                    }
                }
                Focus::Scrolling(idx) => {
                    self.history.render(
                        out,
                        self.readline.lines(),
                        idx,
                        true,
                        self.offset,
                    );
                    self.readline.render(
                        out,
                        &self.env,
                        self.git.as_ref(),
                        idx.is_none(),
                        self.offset,
                    )?;
                    out.hide_cursor(true);
                }
            },
            Scene::Fullscreen => {
                if let Focus::History(idx) = self.focus {
                    self.history.entry(idx).render_fullscreen(out);
                } else {
                    unreachable!();
                }
            }
        }
        Ok(())
    }

    pub fn handle_event(
        &mut self,
        event: Event,
        event_w: &crate::shell::event::Writer,
    ) -> Option<Action> {
        match event {
            Event::Key(key) => {
                return if self.escape {
                    self.escape = false;
                    self.handle_key_escape(&key, event_w.clone())
                } else if key == textmode::Key::Ctrl(b'e') {
                    self.escape = true;
                    None
                } else {
                    match self.focus {
                        Focus::Readline => {
                            self.handle_key_readline(&key, event_w.clone())
                        }
                        Focus::History(idx) => {
                            self.handle_key_history(key, idx);
                            None
                        }
                        Focus::Scrolling(_) => {
                            self.handle_key_escape(&key, event_w.clone())
                        }
                    }
                };
            }
            Event::Resize(new_size) => {
                self.readline.resize(new_size);
                self.history.resize(new_size);
                return Some(Action::Resize(new_size.0, new_size.1));
            }
            Event::PtyOutput => {
                let idx = self.focus_idx();
                // the number of visible lines may have changed, so make sure
                // the focus is still visible
                self.history.make_focus_visible(
                    self.readline.lines(),
                    idx,
                    matches!(self.focus, Focus::Scrolling(_)),
                );
                self.scene = self.default_scene(self.focus);
            }
            Event::ChildExit(idx, env) => {
                if self.focus_idx() == Some(idx) {
                    if let Some(env) = env {
                        if self.hide_readline {
                            let idx = self.env.idx();
                            self.env = env;
                            self.env.set_idx(idx);
                        }
                    }
                    self.set_focus(if self.hide_readline {
                        Focus::Readline
                    } else {
                        Focus::Scrolling(Some(idx))
                    });
                }
            }
            Event::ChildRunPipeline(idx, span) => {
                self.history.entry_mut(idx).set_span(span);
            }
            Event::ChildSuspend(idx) => {
                if self.focus_idx() == Some(idx) {
                    self.set_focus(Focus::Readline);
                }
            }
            Event::GitInfo(info) => {
                self.git = info;
            }
            Event::ClockTimer => {}
        };
        Some(Action::Refresh)
    }

    fn handle_key_escape(
        &mut self,
        key: &textmode::Key,
        event_w: crate::shell::event::Writer,
    ) -> Option<Action> {
        match key {
            textmode::Key::Ctrl(b'd') => {
                return Some(Action::Quit);
            }
            textmode::Key::Ctrl(b'e') => {
                self.set_focus(Focus::Scrolling(self.focus_idx()));
            }
            textmode::Key::Ctrl(b'l') => {
                return Some(Action::HardRefresh);
            }
            textmode::Key::Ctrl(b'm') => {
                if let Some(idx) = self.focus_idx() {
                    self.readline.clear_input();
                    self.history.run(
                        self.history.entry(idx).cmd().to_string(),
                        self.env.clone(),
                        event_w,
                    );
                    let idx = self.history.entry_count() - 1;
                    self.set_focus(Focus::History(idx));
                    self.hide_readline = true;
                    self.env.set_idx(idx + 1);
                } else {
                    self.set_focus(Focus::Readline);
                }
            }
            textmode::Key::Char(' ') => {
                let idx = self.focus_idx();
                if let Some(idx) = idx {
                    if self.history.entry(idx).running() {
                        self.set_focus(Focus::History(idx));
                    }
                } else {
                    self.set_focus(Focus::Readline);
                }
            }
            textmode::Key::Char('e') => {
                if let Focus::History(idx) = self.focus {
                    self.handle_key_history(textmode::Key::Ctrl(b'e'), idx);
                }
            }
            textmode::Key::Char('f') => {
                if let Some(idx) = self.focus_idx() {
                    let mut focus = Focus::History(idx);
                    let entry = self.history.entry_mut(idx);
                    if let Focus::Scrolling(_) = self.focus {
                        entry.set_fullscreen(true);
                    } else {
                        entry.toggle_fullscreen();
                        if !entry.should_fullscreen() && !entry.running() {
                            focus = Focus::Scrolling(Some(idx));
                        }
                    }
                    self.set_focus(focus);
                }
            }
            textmode::Key::Char('i') => {
                if let Some(idx) = self.focus_idx() {
                    self.readline
                        .set_input(self.history.entry(idx).cmd().to_string());
                    self.set_focus(Focus::Readline);
                }
            }
            textmode::Key::Char('j') | textmode::Key::Down => {
                self.set_focus(Focus::Scrolling(
                    self.scroll_down(self.focus_idx()),
                ));
            }
            textmode::Key::Char('k') | textmode::Key::Up => {
                self.set_focus(Focus::Scrolling(
                    self.scroll_up(self.focus_idx()),
                ));
            }
            textmode::Key::Char('n') => {
                self.set_focus(self.next_running());
            }
            textmode::Key::Char('p') => {
                self.set_focus(self.prev_running());
            }
            textmode::Key::Char('r') => {
                self.set_focus(Focus::Readline);
            }
            _ => {
                return None;
            }
        }
        Some(Action::Refresh)
    }

    fn handle_key_readline(
        &mut self,
        key: &textmode::Key,
        event_w: crate::shell::event::Writer,
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
                    self.history.run(
                        input.to_string(),
                        self.env.clone(),
                        event_w,
                    );
                    let idx = self.history.entry_count() - 1;
                    self.set_focus(Focus::History(idx));
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
                    self.set_focus(Focus::Scrolling(Some(entry_count - 1)));
                }
            }
            _ => return None,
        }
        Some(Action::Refresh)
    }

    fn handle_key_history(&mut self, key: textmode::Key, idx: usize) {
        self.history.entry(idx).input(key.into_bytes());
    }

    fn default_scene(&self, focus: Focus) -> Scene {
        match focus {
            Focus::Readline | Focus::Scrolling(_) => Scene::Readline,
            Focus::History(idx) => {
                if self.history.entry(idx).should_fullscreen() {
                    Scene::Fullscreen
                } else {
                    Scene::Readline
                }
            }
        }
    }

    fn set_focus(&mut self, new_focus: Focus) {
        self.focus = new_focus;
        self.hide_readline = false;
        self.scene = self.default_scene(new_focus);
        // passing entry into default_scene above consumes it, which means
        // that the mutex lock will be dropped before we call into
        // make_focus_visible, which is important because otherwise we might
        // get a deadlock depending on what is visible
        self.history.make_focus_visible(
            self.readline.lines(),
            self.focus_idx(),
            matches!(self.focus, Focus::Scrolling(_)),
        );
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

    fn next_running(&self) -> Focus {
        let count = self.history.entry_count();
        let cur = self.focus_idx().unwrap_or(count);
        for idx in ((cur + 1)..count).chain(0..cur) {
            if self.history.entry(idx).running() {
                return Focus::History(idx);
            }
        }
        self.focus_idx().map_or(Focus::Readline, Focus::History)
    }

    fn prev_running(&self) -> Focus {
        let count = self.history.entry_count();
        let cur = self.focus_idx().unwrap_or(count);
        for idx in ((cur + 1)..count).chain(0..cur).rev() {
            if self.history.entry(idx).running() {
                return Focus::History(idx);
            }
        }
        self.focus_idx().map_or(Focus::Readline, Focus::History)
    }
}
