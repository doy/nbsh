use async_std::io::{ReadExt as _, WriteExt as _};
use futures_lite::future::FutureExt as _;
use pty_process::Command as _;
use std::os::unix::process::ExitStatusExt as _;
use textmode::Textmode as _;

pub struct History {
    size: (u16, u16),
    entries: Vec<crate::util::Mutex<HistoryEntry>>,
}

impl History {
    pub fn new() -> Self {
        Self {
            size: (24, 80),
            entries: vec![],
        }
    }

    pub async fn handle_key(&self, key: textmode::Key, idx: usize) {
        let entry = self.entries[idx].lock_arc().await;
        if entry.running() {
            entry.input.send(key.into_bytes()).await.unwrap();
        }
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        repl_lines: usize,
        focus: Option<usize>,
    ) -> anyhow::Result<()> {
        let mut used_lines = repl_lines;
        let mut pos = None;
        for (idx, entry) in self.entries.iter().enumerate().rev() {
            let mut entry = entry.lock_arc().await;
            let focused = focus.map_or(false, |focus| idx == focus);
            let last_row = entry.lines(self.size.1, focused);
            used_lines += 1 + std::cmp::min(6, last_row);
            if used_lines > self.size.0 as usize {
                break;
            }
            if focused && used_lines == 1 && entry.running() {
                used_lines = 2;
            }
            out.move_to(
                (self.size.0 as usize - used_lines).try_into().unwrap(),
                0,
            );
            entry.render(out, self.size.1, focused);
            if focused {
                pos = Some(out.screen().cursor_position());
            }
        }
        if let Some(pos) = pos {
            out.move_to(pos.0, pos.1);
        }
        Ok(())
    }

    pub async fn render_fullscreen(
        &self,
        out: &mut textmode::Output,
        idx: usize,
    ) {
        let mut entry = self.entries[idx].lock_arc().await;
        entry.render_fullscreen(out);
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        for entry in &self.entries {
            let entry = entry.lock_arc().await;
            if entry.running() {
                entry.resize.send(size).await.unwrap();
            }
        }
    }

    pub async fn run(
        &mut self,
        cmd: &str,
        action_w: async_std::channel::Sender<crate::action::Action>,
    ) -> anyhow::Result<usize> {
        let (exe, args) = crate::parse::cmd(cmd);
        let (input_w, input_r) = async_std::channel::unbounded();
        let (resize_w, resize_r) = async_std::channel::unbounded();
        let entry = crate::util::mutex(HistoryEntry::new(
            cmd, self.size, input_w, resize_w,
        ));
        if crate::builtins::is(&exe) {
            let code: i32 = crate::builtins::run(&exe, &args).into();
            entry.lock_arc().await.exit_info = Some(ExitInfo::new(
                async_std::process::ExitStatus::from_raw(code << 8),
            ));
            action_w
                .send(crate::action::Action::UpdateFocus(
                    crate::state::Focus::Readline,
                ))
                .await
                .unwrap();
        } else {
            let mut process = async_std::process::Command::new(&exe);
            process.args(&args);
            let child = process
                .spawn_pty(Some(&pty_process::Size::new(
                    self.size.0,
                    self.size.1,
                )))
                .unwrap();
            run_process(
                child,
                async_std::sync::Arc::clone(&entry),
                input_r,
                resize_r,
                action_w,
            );
        }
        self.entries.push(entry);
        Ok(self.entries.len() - 1)
    }

    pub async fn toggle_fullscreen(&mut self, idx: usize) {
        self.entries[idx].lock_arc().await.toggle_fullscreen();
    }

    pub async fn should_fullscreen(&self, idx: usize) -> bool {
        self.entries[idx].lock_arc().await.should_fullscreen()
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }
}

struct HistoryEntry {
    cmd: String,
    vt: vt100::Parser,
    audible_bell_state: usize,
    visual_bell_state: usize,
    fullscreen: Option<bool>,
    input: async_std::channel::Sender<Vec<u8>>,
    resize: async_std::channel::Sender<(u16, u16)>,
    start_time: chrono::DateTime<chrono::Local>,
    start_instant: std::time::Instant,
    exit_info: Option<ExitInfo>,
}

impl HistoryEntry {
    fn new(
        cmd: &str,
        size: (u16, u16),
        input: async_std::channel::Sender<Vec<u8>>,
        resize: async_std::channel::Sender<(u16, u16)>,
    ) -> Self {
        Self {
            cmd: cmd.into(),
            vt: vt100::Parser::new(size.0, size.1, 0),
            audible_bell_state: 0,
            visual_bell_state: 0,
            input,
            resize,
            fullscreen: None,
            start_time: chrono::Local::now(),
            start_instant: std::time::Instant::now(),
            exit_info: None,
        }
    }

    fn render(
        &mut self,
        out: &mut textmode::Output,
        width: u16,
        focused: bool,
    ) {
        out.set_bgcolor(textmode::Color::Rgb(32, 32, 32));
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
        if focused {
            out.set_fgcolor(textmode::color::BLACK);
            out.set_bgcolor(textmode::color::CYAN);
        } else {
            out.set_bgcolor(textmode::Color::Rgb(32, 32, 32));
        }
        out.write_str("$ ");
        out.reset_attributes();
        out.set_bgcolor(textmode::Color::Rgb(32, 32, 32));
        if self.running() {
            out.set_bgcolor(textmode::Color::Rgb(16, 64, 16));
        }
        out.write_str(&self.cmd);
        out.reset_attributes();
        out.set_bgcolor(textmode::Color::Rgb(32, 32, 32));
        let time = if let Some(info) = self.exit_info {
            format!(
                "[{} ({:6})]",
                self.start_time.time().format("%H:%M:%S"),
                crate::format::duration(info.instant - self.start_instant)
            )
        } else {
            format!("[{}]", self.start_time.time().format("%H:%M:%S"))
        };
        let cur_pos = out.screen().cursor_position();
        out.write_str(
            &" ".repeat(width as usize - time.len() - 1 - cur_pos.1 as usize),
        );
        out.write_str(&time);
        out.write_str(" ");
        out.reset_attributes();

        if self.binary() {
            let msg = b"This appears to be binary data. Fullscreen this entry to view anyway.";
            let len: u16 = msg.len().try_into().unwrap();
            out.move_to(
                out.screen().cursor_position().0 + 1,
                (width - len) / 2,
            );
            out.set_fgcolor(textmode::color::RED);
            out.write(msg);
            out.write(b"\x1b[?25l");
            out.reset_attributes();
        } else {
            let last_row = self.lines(width, focused);
            if last_row > 5 {
                out.write(b"\r\n");
                out.set_fgcolor(textmode::color::BLUE);
                out.write(b"...");
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
                out.write(b"\x1b[m");
                if !wrapped {
                    out.write(format!("\x1b[{}H", out_row + 1).as_bytes());
                }
                out.write(&row);
                wrapped = screen.row_wrapped(idx);
                if pos.0 == idx {
                    cursor_found = Some(out_row);
                }
                out_row += 1;
            }
            if focused {
                if let Some(row) = cursor_found {
                    if screen.hide_cursor() {
                        out.write(b"\x1b[?25l");
                    } else {
                        out.write(b"\x1b[?25h");
                        out.move_to(row, pos.1);
                    }
                } else {
                    out.write(b"\x1b[?25l");
                }
            }
        }
        out.reset_attributes();
    }

    fn render_fullscreen(&mut self, out: &mut textmode::Output) {
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

    fn toggle_fullscreen(&mut self) {
        if let Some(fullscreen) = self.fullscreen {
            self.fullscreen = Some(!fullscreen);
        } else {
            self.fullscreen = Some(!self.vt.screen().alternate_screen());
        }
    }

    fn running(&self) -> bool {
        self.exit_info.is_none()
    }

    fn binary(&self) -> bool {
        self.vt.screen().errors() > 5
    }

    fn lines(&self, width: u16, focused: bool) -> usize {
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
                screen.cursor_position().0 as usize + 1,
            );
        }
        last_row
    }

    fn should_fullscreen(&self) -> bool {
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

fn run_process(
    mut child: pty_process::async_std::Child,
    entry: crate::util::Mutex<HistoryEntry>,
    input_r: async_std::channel::Receiver<Vec<u8>>,
    resize_r: async_std::channel::Receiver<(u16, u16)>,
    action_w: async_std::channel::Sender<crate::action::Action>,
) {
    async_std::task::spawn(async move {
        loop {
            enum Res {
                Read(Result<usize, std::io::Error>),
                Write(Result<Vec<u8>, async_std::channel::RecvError>),
                Resize(Result<(u16, u16), async_std::channel::RecvError>),
            }
            let mut buf = [0_u8; 4096];
            let mut pty = child.pty();
            let read = async { Res::Read(pty.read(&mut buf).await) };
            let write = async { Res::Write(input_r.recv().await) };
            let resize = async { Res::Resize(resize_r.recv().await) };
            match read.race(write).race(resize).await {
                Res::Read(res) => match res {
                    Ok(bytes) => {
                        let mut entry = entry.lock_arc().await;
                        let pre_alternate_screen =
                            entry.vt.screen().alternate_screen();
                        entry.vt.process(&buf[..bytes]);
                        let post_alternate_screen =
                            entry.vt.screen().alternate_screen();
                        if entry.fullscreen.is_none()
                            && pre_alternate_screen != post_alternate_screen
                        {
                            action_w
                                .send(crate::action::Action::CheckUpdateScene)
                                .await
                                .unwrap();
                        }
                        action_w
                            .send(crate::action::Action::Render)
                            .await
                            .unwrap();
                    }
                    Err(e) => {
                        if e.raw_os_error() != Some(libc::EIO) {
                            eprintln!("pty read failed: {:?}", e);
                        }
                        // XXX not sure if this is safe - are we sure
                        // the child exited?
                        entry.lock_arc().await.exit_info = Some(
                            ExitInfo::new(child.status().await.unwrap()),
                        );
                        action_w
                            .send(crate::action::Action::UpdateFocus(
                                crate::state::Focus::Readline,
                            ))
                            .await
                            .unwrap();
                        break;
                    }
                },
                Res::Write(res) => match res {
                    Ok(bytes) => {
                        pty.write(&bytes).await.unwrap();
                    }
                    Err(e) => {
                        panic!("failed to read from input channel: {}", e);
                    }
                },
                Res::Resize(res) => match res {
                    Ok(size) => {
                        child
                            .resize_pty(&pty_process::Size::new(
                                size.0, size.1,
                            ))
                            .unwrap();
                        entry.lock_arc().await.vt.set_size(size.0, size.1);
                    }
                    Err(e) => {
                        panic!("failed to read from resize channel: {}", e);
                    }
                },
            }
        }
    });
}
