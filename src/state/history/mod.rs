use async_std::io::WriteExt as _;
use std::os::unix::io::FromRawFd as _;
use std::os::unix::process::ExitStatusExt as _;

mod builtins;
mod pty;

pub struct History {
    size: (u16, u16),
    entries: Vec<async_std::sync::Arc<async_std::sync::Mutex<Entry>>>,
    scroll_pos: usize,
}

impl History {
    pub fn new() -> Self {
        Self {
            size: (24, 80),
            entries: vec![],
            scroll_pos: 0,
        }
    }

    // render always happens on the main task
    #[allow(clippy::future_not_send)]
    pub async fn render(
        &self,
        out: &mut impl textmode::Textmode,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
        offset: time::UtcOffset,
    ) -> anyhow::Result<()> {
        let mut used_lines = repl_lines;
        let mut cursor = None;
        for (idx, entry) in
            self.visible(repl_lines, focus, scrolling).await.rev()
        {
            let focused = focus.map_or(false, |focus| idx == focus);
            used_lines += entry.lines(self.size.1, focused && !scrolling);
            out.move_to(
                (usize::from(self.size.0) - used_lines).try_into().unwrap(),
                0,
            );
            entry.render(
                out,
                idx,
                self.entry_count(),
                self.size.1,
                focused,
                scrolling,
                offset,
            );
            if focused && !scrolling {
                cursor = Some((
                    out.screen().cursor_position(),
                    out.screen().hide_cursor(),
                ));
            }
        }
        if let Some((pos, hide)) = cursor {
            out.move_to(pos.0, pos.1);
            out.hide_cursor(hide);
        }
        Ok(())
    }

    // render always happens on the main task
    #[allow(clippy::future_not_send)]
    pub async fn render_fullscreen(
        &self,
        out: &mut impl textmode::Textmode,
        idx: usize,
    ) {
        let mut entry = self.entries[idx].lock_arc().await;
        entry.render_fullscreen(out);
    }

    pub async fn send_input(&mut self, idx: usize, input: Vec<u8>) {
        self.entry(idx).await.send_input(input).await;
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        for entry in &self.entries {
            entry.lock_arc().await.resize(size).await;
        }
    }

    pub async fn run(
        &mut self,
        ast: &crate::parse::Commands,
        event_w: async_std::channel::Sender<crate::event::Event>,
    ) -> anyhow::Result<usize> {
        let (input_w, input_r) = async_std::channel::unbounded();
        let (resize_w, resize_r) = async_std::channel::unbounded();

        let entry = async_std::sync::Arc::new(async_std::sync::Mutex::new(
            Entry::new(Ok(ast.clone()), self.size, input_w, resize_w),
        ));
        run_commands(
            ast.clone(),
            async_std::sync::Arc::clone(&entry),
            ProcessEnv::new(),
            input_r,
            resize_r,
            event_w,
        );

        self.entries.push(entry);
        Ok(self.entries.len() - 1)
    }

    pub async fn parse_error(
        &mut self,
        e: crate::parse::Error,
        event_w: async_std::channel::Sender<crate::event::Event>,
    ) {
        // XXX would be great to not have to do this
        let (input_w, input_r) = async_std::channel::unbounded();
        let (resize_w, resize_r) = async_std::channel::unbounded();
        input_w.close();
        input_r.close();
        resize_w.close();
        resize_r.close();

        let err_str = format!("{}", e);
        let mut entry = Entry::new(Err(e), self.size, input_w, resize_w);
        entry.vt.process(err_str.replace('\n', "\r\n").as_bytes());
        let status = async_std::process::ExitStatus::from_raw(1 << 8);
        entry.exit_info = Some(ExitInfo::new(status));
        self.entries.push(async_std::sync::Arc::new(
            async_std::sync::Mutex::new(entry),
        ));
        event_w.send(crate::event::Event::PtyClose).await.unwrap();
    }

    pub async fn entry(
        &self,
        idx: usize,
    ) -> async_std::sync::MutexGuardArc<Entry> {
        self.entries[idx].lock_arc().await
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub async fn make_focus_visible(
        &mut self,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
    ) {
        if self.entries.is_empty() || focus.is_none() {
            return;
        }
        let focus = focus.unwrap();

        let mut done = false;
        while focus
            < self
                .visible(repl_lines, Some(focus), scrolling)
                .await
                .map(|(idx, _)| idx)
                .next()
                .unwrap()
        {
            self.scroll_pos += 1;
            done = true;
        }
        if done {
            return;
        }

        while focus
            > self
                .visible(repl_lines, Some(focus), scrolling)
                .await
                .map(|(idx, _)| idx)
                .last()
                .unwrap()
        {
            self.scroll_pos -= 1;
        }
    }

    async fn visible(
        &self,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
    ) -> VisibleEntries {
        let mut iter = VisibleEntries::new();
        if self.entries.is_empty() {
            return iter;
        }

        let mut used_lines = repl_lines;
        for (idx, entry) in
            self.entries.iter().enumerate().rev().skip(self.scroll_pos)
        {
            let entry = entry.lock_arc().await;
            let focused = focus.map_or(false, |focus| idx == focus);
            used_lines += entry.lines(self.size.1, focused && !scrolling);
            if used_lines > usize::from(self.size.0) {
                break;
            }
            iter.add(idx, entry);
        }
        iter
    }
}

struct VisibleEntries {
    entries: std::collections::VecDeque<(
        usize,
        async_std::sync::MutexGuardArc<Entry>,
    )>,
}

impl VisibleEntries {
    fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
        }
    }

    fn add(
        &mut self,
        idx: usize,
        entry: async_std::sync::MutexGuardArc<Entry>,
    ) {
        // push_front because we are adding them in reverse order
        self.entries.push_front((idx, entry));
    }
}

impl std::iter::Iterator for VisibleEntries {
    type Item = (usize, async_std::sync::MutexGuardArc<Entry>);

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.pop_front()
    }
}

impl std::iter::DoubleEndedIterator for VisibleEntries {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.pop_back()
    }
}

pub struct Entry {
    ast: Result<crate::parse::Commands, crate::parse::Error>,
    vt: vt100::Parser,
    audible_bell_state: usize,
    visual_bell_state: usize,
    fullscreen: Option<bool>,
    input: async_std::channel::Sender<Vec<u8>>,
    resize: async_std::channel::Sender<(u16, u16)>,
    start_time: time::OffsetDateTime,
    start_instant: std::time::Instant,
    exit_info: Option<ExitInfo>,
}

impl Entry {
    fn new(
        ast: Result<crate::parse::Commands, crate::parse::Error>,
        size: (u16, u16),
        input: async_std::channel::Sender<Vec<u8>>,
        resize: async_std::channel::Sender<(u16, u16)>,
    ) -> Self {
        Self {
            ast,
            vt: vt100::Parser::new(size.0, size.1, 0),
            audible_bell_state: 0,
            visual_bell_state: 0,
            input,
            resize,
            fullscreen: None,
            start_time: time::OffsetDateTime::now_utc(),
            start_instant: std::time::Instant::now(),
            exit_info: None,
        }
    }

    fn render(
        &self,
        out: &mut impl textmode::Textmode,
        idx: usize,
        entry_count: usize,
        width: u16,
        focused: bool,
        scrolling: bool,
        offset: time::UtcOffset,
    ) {
        set_bgcolor(out, idx, focused);
        out.set_fgcolor(textmode::color::YELLOW);
        let entry_count_width = format!("{}", entry_count + 1).len();
        let idx_str = format!("{}", idx + 1);
        out.write_str(&" ".repeat(entry_count_width - idx_str.len()));
        out.write_str(&idx_str);
        out.write_str(" ");
        out.reset_attributes();

        set_bgcolor(out, idx, focused);
        if let Some(info) = self.exit_info {
            if info.status.signal().is_some() {
                out.set_fgcolor(textmode::color::MAGENTA);
            } else if info.status.success() {
                out.set_fgcolor(textmode::color::DARKGREY);
            } else {
                out.set_fgcolor(textmode::color::RED);
            }
            out.write_str(&crate::format::exit_status(info.status));
        } else {
            out.write_str("     ");
        }
        out.reset_attributes();

        set_bgcolor(out, idx, focused);
        out.write_str("$ ");
        if self.running() {
            out.set_bgcolor(textmode::Color::Rgb(16, 64, 16));
        }
        out.write_str(self.cmd());
        out.reset_attributes();

        set_bgcolor(out, idx, focused);
        let time = self.exit_info.map_or_else(
            || {
                format!(
                    "[{}]",
                    crate::format::time(self.start_time.to_offset(offset))
                )
            },
            |info| {
                format!(
                    "({}) [{}]",
                    crate::format::duration(
                        info.instant - self.start_instant
                    ),
                    crate::format::time(self.start_time.to_offset(offset)),
                )
            },
        );
        let cur_pos = out.screen().cursor_position();
        out.write_str(&" ".repeat(
            usize::from(width) - time.len() - 1 - usize::from(cur_pos.1),
        ));
        out.write_str(&time);
        out.write_str(" ");
        out.reset_attributes();

        if self.binary() {
            let msg = "This appears to be binary data. Fullscreen this entry to view anyway.";
            let len: u16 = msg.len().try_into().unwrap();
            out.move_to(
                out.screen().cursor_position().0 + 1,
                (width - len) / 2,
            );
            out.set_fgcolor(textmode::color::RED);
            out.write_str(msg);
            out.hide_cursor(true);
        } else {
            let last_row = self.output_lines(width, focused && !scrolling);
            if last_row > 5 {
                out.write(b"\r\n");
                out.set_fgcolor(textmode::color::BLUE);
                out.write_str("...");
                out.reset_attributes();
            }
            let mut out_row = out.screen().cursor_position().0 + 1;
            let screen = self.vt.screen();
            let pos = screen.cursor_position();
            let mut wrapped = false;
            let mut cursor_found = None;
            for (idx, row) in screen
                .rows_formatted(0, width)
                .enumerate()
                .take(last_row)
                .skip(last_row.saturating_sub(5))
            {
                let idx: u16 = idx.try_into().unwrap();
                out.reset_attributes();
                if !wrapped {
                    out.move_to(out_row, 0);
                }
                out.write(&row);
                wrapped = screen.row_wrapped(idx);
                if pos.0 == idx {
                    cursor_found = Some(out_row);
                }
                out_row += 1;
            }
            if focused && !scrolling {
                if let Some(row) = cursor_found {
                    out.hide_cursor(screen.hide_cursor());
                    out.move_to(row, pos.1);
                } else {
                    out.hide_cursor(true);
                }
            }
        }
        out.reset_attributes();
    }

    fn render_fullscreen(&mut self, out: &mut impl textmode::Textmode) {
        let screen = self.vt.screen();
        let new_audible_bell_state = screen.audible_bell_count();
        let new_visual_bell_state = screen.visual_bell_count();

        out.write(&screen.state_formatted());

        if self.audible_bell_state != new_audible_bell_state {
            out.write(b"\x07");
            self.audible_bell_state = new_audible_bell_state;
        }

        if self.visual_bell_state != new_visual_bell_state {
            out.write(b"\x1bg");
            self.visual_bell_state = new_visual_bell_state;
        }

        out.reset_attributes();
    }

    pub async fn send_input(&self, bytes: Vec<u8>) {
        if self.running() {
            self.input.send(bytes).await.unwrap();
        }
    }

    pub async fn resize(&self, size: (u16, u16)) {
        if self.running() {
            self.resize.send(size).await.unwrap();
        }
    }

    pub fn cmd(&self) -> &str {
        match &self.ast {
            Ok(ast) => ast.input_string(),
            Err(e) => e.input(),
        }
    }

    pub fn toggle_fullscreen(&mut self) {
        if let Some(fullscreen) = self.fullscreen {
            self.fullscreen = Some(!fullscreen);
        } else {
            self.fullscreen = Some(!self.vt.screen().alternate_screen());
        }
    }

    pub fn set_fullscreen(&mut self, fullscreen: bool) {
        self.fullscreen = Some(fullscreen);
    }

    pub fn running(&self) -> bool {
        self.exit_info.is_none()
    }

    pub fn binary(&self) -> bool {
        self.vt.screen().errors() > 5
    }

    pub fn lines(&self, width: u16, focused: bool) -> usize {
        let lines = self.output_lines(width, focused);
        1 + std::cmp::min(6, lines)
    }

    pub fn output_lines(&self, width: u16, focused: bool) -> usize {
        if let Err(e) = &self.ast {
            return e.error().to_string().lines().count();
        }

        if self.binary() {
            return 1;
        }

        let screen = self.vt.screen();
        let mut last_row = 0;
        for (idx, row) in screen.rows(0, width).enumerate() {
            if !row.is_empty() {
                last_row = idx + 1;
            }
        }
        if focused && self.running() {
            last_row = std::cmp::max(
                last_row,
                usize::from(screen.cursor_position().0) + 1,
            );
        }
        last_row
    }

    pub fn should_fullscreen(&self) -> bool {
        self.fullscreen
            .unwrap_or_else(|| self.vt.screen().alternate_screen())
    }
}

#[derive(Copy, Clone)]
struct ExitInfo {
    status: async_std::process::ExitStatus,
    instant: std::time::Instant,
}

impl ExitInfo {
    fn new(status: async_std::process::ExitStatus) -> Self {
        Self {
            status,
            instant: std::time::Instant::now(),
        }
    }
}

#[derive(Clone)]
pub struct ProcessEnv {
    latest_status: async_std::process::ExitStatus,
}

impl ProcessEnv {
    fn new() -> Self {
        Self {
            latest_status: async_std::process::ExitStatus::from_raw(0),
        }
    }

    fn set_status(&mut self, status: async_std::process::ExitStatus) {
        self.latest_status = status;
    }

    fn latest_status(&self) -> &async_std::process::ExitStatus {
        &self.latest_status
    }
}

fn run_commands(
    ast: crate::parse::Commands,
    entry: async_std::sync::Arc<async_std::sync::Mutex<Entry>>,
    mut env: ProcessEnv,
    input_r: async_std::channel::Receiver<Vec<u8>>,
    resize_r: async_std::channel::Receiver<(u16, u16)>,
    event_w: async_std::channel::Sender<crate::event::Event>,
) {
    async_std::task::spawn(async move {
        let pty = match pty::Pty::new(
            entry.lock_arc().await.vt.screen().size(),
            &entry,
            input_r,
            resize_r,
            event_w,
        ) {
            Ok(pty) => pty,
            Err(e) => {
                let mut entry = entry.lock_arc().await;
                entry.vt.process(
                    format!("nbsh: failed to allocate pty: {}", e).as_bytes(),
                );
                entry.exit_info = Some(ExitInfo::new(
                    async_std::process::ExitStatus::from_raw(1 << 8),
                ));
                return;
            }
        };

        for pipeline in ast.pipelines() {
            let (pipeline_status, done) =
                run_pipeline(pipeline, &pty, &env).await;
            env.set_status(pipeline_status);
            if done {
                break;
            }
        }
        entry.lock_arc().await.exit_info =
            Some(ExitInfo::new(*env.latest_status()));

        pty.close().await;
    });
}

async fn run_pipeline(
    pipeline: &crate::parse::Pipeline,
    pty: &pty::Pty,
    env: &ProcessEnv,
) -> (async_std::process::ExitStatus, bool) {
    let mut cmd = pty_process::Command::new(std::env::current_exe().unwrap());
    cmd.arg("--internal-pipe-runner");
    let (r, w) = nix::unistd::pipe2(nix::fcntl::OFlag::O_CLOEXEC).unwrap();
    unsafe {
        cmd.pre_exec(move || {
            nix::unistd::dup2(r, 3)?;
            Ok(())
        });
    }
    let child = pty.spawn(cmd).unwrap();
    nix::unistd::close(r).unwrap();

    let mut w = unsafe { async_std::fs::File::from_raw_fd(w) };
    w.write_all(pipeline.input_string().as_bytes())
        .await
        .unwrap();
    // todo: write contents of env also
    drop(w);

    let status = child.status_no_drop().await.unwrap();
    (
        status,
        // i'm not sure what exactly the expected behavior here is - in zsh,
        // SIGINT kills the whole command line while SIGTERM doesn't, but i
        // don't know what the precise logic is or how other signals are
        // handled
        status.signal() == Some(signal_hook::consts::signal::SIGINT),
    )
}

fn set_bgcolor(out: &mut impl textmode::Textmode, idx: usize, focus: bool) {
    if focus {
        out.set_bgcolor(textmode::Color::Rgb(0x56, 0x1b, 0x8b));
    } else if idx % 2 == 0 {
        out.set_bgcolor(textmode::Color::Rgb(0x24, 0x21, 0x00));
    } else {
        out.set_bgcolor(textmode::Color::Rgb(0x20, 0x20, 0x20));
    }
}
