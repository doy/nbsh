use async_std::io::{ReadExt as _, WriteExt as _};
use futures_lite::future::FutureExt as _;
use pty_process::Command as _;
use textmode::Textmode as _;

pub struct History {
    size: (u16, u16),
    entries: Vec<crate::util::Mutex<HistoryEntry>>,
    escape: bool,
    action: async_std::channel::Sender<crate::action::Action>,
}

impl History {
    pub fn new(
        action: async_std::channel::Sender<crate::action::Action>,
    ) -> Self {
        Self {
            size: (24, 80),
            entries: vec![],
            escape: false,
            action,
        }
    }

    pub async fn run(&mut self, cmd: &str) -> anyhow::Result<usize> {
        let (exe, args) = parse_cmd(cmd);
        let mut process = async_std::process::Command::new(&exe);
        process.args(&args);
        let child = process
            .spawn_pty(Some(&pty_process::Size::new(
                self.size.0,
                self.size.1,
            )))
            .unwrap();
        let (input_w, input_r) = async_std::channel::unbounded();
        let (resize_w, resize_r) = async_std::channel::unbounded();
        let entry = crate::util::mutex(HistoryEntry::new(
            cmd, self.size, input_w, resize_w,
        ));
        let task_entry = async_std::sync::Arc::clone(&entry);
        let task_action = self.action.clone();
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
                    Res::Read(res) => {
                        match res {
                            Ok(bytes) => {
                                task_entry
                                    .lock_arc()
                                    .await
                                    .vt
                                    .process(&buf[..bytes]);
                            }
                            Err(e) => {
                                if e.raw_os_error() != Some(libc::EIO) {
                                    eprintln!("pty read failed: {:?}", e);
                                }
                                task_entry.lock_arc().await.running = false;
                                task_action
                                    .send(crate::action::Action::UpdateFocus(
                                        crate::state::Focus::Readline,
                                    ))
                                    .await
                                    .unwrap();
                                break;
                            }
                        }
                        task_action
                            .send(crate::action::Action::Render)
                            .await
                            .unwrap();
                    }
                    Res::Write(res) => match res {
                        Ok(bytes) => {
                            pty.write(&bytes).await.unwrap();
                        }
                        Err(e) => {
                            panic!(
                                "failed to read from input channel: {}",
                                e
                            );
                        }
                    },
                    Res::Resize(res) => match res {
                        Ok(size) => {
                            child
                                .resize_pty(&pty_process::Size::new(
                                    size.0, size.1,
                                ))
                                .unwrap();
                            task_entry
                                .lock_arc()
                                .await
                                .vt
                                .set_size(size.0, size.1);
                        }
                        Err(e) => {
                            panic!(
                                "failed to read from resize channel: {}",
                                e
                            );
                        }
                    },
                }
            }
        });
        self.entries.push(entry);
        self.action
            .send(crate::action::Action::UpdateFocus(
                crate::state::Focus::History(self.entries.len() - 1),
            ))
            .await
            .unwrap();
        Ok(self.entries.len() - 1)
    }

    pub async fn handle_key(
        &mut self,
        key: textmode::Key,
        idx: usize,
    ) -> bool {
        if self.escape {
            match key {
                textmode::Key::Ctrl(b'e') => {
                    self.send_process_input(idx, &key.into_bytes())
                        .await
                        .unwrap();
                }
                textmode::Key::Char('r') => {
                    self.action
                        .send(crate::action::Action::UpdateFocus(
                            crate::state::Focus::Readline,
                        ))
                        .await
                        .unwrap();
                }
                _ => {}
            }
            self.escape = false;
        } else {
            match key {
                textmode::Key::Ctrl(b'e') => {
                    self.escape = true;
                }
                key => {
                    self.send_process_input(idx, &key.into_bytes())
                        .await
                        .unwrap();
                }
            }
        }
        false
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        repl_lines: usize,
        focus: Option<usize>,
    ) -> anyhow::Result<()> {
        if let Some(idx) = focus {
            let mut entry = self.entries[idx].lock_arc().await;
            let screen = entry.vt.screen();
            if screen.alternate_screen() {
                let new_audible_bell_state = screen.audible_bell_count();
                let new_visual_bell_state = screen.visual_bell_count();

                out.write(&screen.state_formatted());

                if entry.audible_bell_state != new_audible_bell_state {
                    out.write(b"\x07");
                    entry.audible_bell_state = new_audible_bell_state;
                }

                if entry.visual_bell_state != new_visual_bell_state {
                    out.write(b"\x1bg");
                    entry.visual_bell_state = new_visual_bell_state;
                }

                return Ok(());
            }
        }
        let mut used_lines = repl_lines;
        let mut pos = None;
        for (idx, entry) in self.entries.iter().enumerate().rev() {
            let entry = entry.lock_arc().await;
            let screen = entry.vt.screen();
            let mut last_row = 0;
            for (idx, row) in screen.rows(0, self.size.1).enumerate() {
                if !row.is_empty() {
                    last_row = idx + 1;
                }
            }
            if focus == Some(idx) {
                last_row = std::cmp::max(
                    last_row,
                    screen.cursor_position().0 as usize + 1,
                );
            }
            used_lines += 1 + std::cmp::min(6, last_row);
            if used_lines > self.size.0 as usize {
                break;
            }
            if used_lines == 1 {
                used_lines = 2;
                pos = Some((self.size.0 - 1, 0));
            }
            out.move_to(
                (self.size.0 as usize - used_lines).try_into().unwrap(),
                0,
            );
            out.write_str("$ ");
            if entry.running {
                out.set_bgcolor(textmode::Color::Rgb(16, 64, 16));
            }
            out.write_str(&entry.cmd);
            out.reset_attributes();
            if last_row > 5 {
                out.write(b"\r\n");
                out.set_bgcolor(textmode::color::RED);
                out.write(b"...");
                out.reset_attributes();
            }
            let mut end_pos = (0, 0);
            for row in screen
                .rows_formatted(0, self.size.1)
                .take(last_row)
                .skip(last_row.saturating_sub(5))
            {
                out.write(b"\r\n");
                out.write(&row);
                end_pos = out.screen().cursor_position();
            }
            if pos.is_none() {
                pos = Some(end_pos);
            }
            out.reset_attributes();
        }
        if let Some(pos) = pos {
            out.move_to(pos.0, pos.1);
        }
        Ok(())
    }

    pub async fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        for entry in &self.entries {
            let entry = entry.lock_arc().await;
            if entry.running {
                entry.resize.send(size).await.unwrap();
            }
        }
    }

    async fn send_process_input(
        &self,
        idx: usize,
        input: &[u8],
    ) -> anyhow::Result<()> {
        self.entries[idx]
            .lock_arc()
            .await
            .input
            .send(input.to_vec())
            .await
            .unwrap();
        Ok(())
    }
}

struct HistoryEntry {
    cmd: String,
    vt: vt100::Parser,
    audible_bell_state: usize,
    visual_bell_state: usize,
    input: async_std::channel::Sender<Vec<u8>>,
    resize: async_std::channel::Sender<(u16, u16)>,
    running: bool, // option end time
                   // start time
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
            running: true,
        }
    }
}

fn parse_cmd(full_cmd: &str) -> (String, Vec<String>) {
    let mut parts = full_cmd.split(' ');
    let cmd = parts.next().unwrap();
    (
        cmd.to_string(),
        parts.map(std::string::ToString::to_string).collect(),
    )
}
