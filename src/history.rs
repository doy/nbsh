use async_std::io::{ReadExt as _, WriteExt as _};
use pty_process::Command as _;
use textmode::Textmode as _;

pub struct History {
    entries: Vec<crate::util::Mutex<HistoryEntry>>,
    action: async_std::channel::Sender<crate::action::Action>,
}

impl History {
    pub fn new(
        action: async_std::channel::Sender<crate::action::Action>,
    ) -> Self {
        Self {
            entries: vec![],
            action,
        }
    }

    pub async fn run(&mut self, cmd: &str) -> anyhow::Result<usize> {
        let (exe, args) = parse_cmd(cmd);
        let mut process = async_std::process::Command::new(&exe);
        process.args(&args);
        let child = process
            .spawn_pty(Some(&pty_process::Size::new(24, 80)))
            .unwrap();
        let (input_w, input_r) = async_std::channel::unbounded();
        let entry = crate::util::mutex(HistoryEntry::new(
            cmd,
            child.id().try_into().unwrap(),
            input_w,
        ));
        let task_entry = async_std::sync::Arc::clone(&entry);
        let task_action = self.action.clone();
        async_std::task::spawn(async move {
            loop {
                enum Res {
                    Read(Result<usize, std::io::Error>),
                    Write(Result<Vec<u8>, async_std::channel::RecvError>),
                }
                let mut buf = [0_u8; 4096];
                let mut pty = child.pty();
                let read = async { Res::Read(pty.read(&mut buf).await) };
                let write = async { Res::Write(input_r.recv().await) };
                match futures_lite::future::race(read, write).await {
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
        match key {
            textmode::Key::Ctrl(b'c') => {
                let pid = self.entries[idx].lock_arc().await.pid;
                nix::sys::signal::kill(pid, nix::sys::signal::Signal::SIGINT)
                    .unwrap();
            }
            textmode::Key::Ctrl(b'z') => {
                self.action
                    .send(crate::action::Action::UpdateFocus(
                        crate::state::Focus::Readline,
                    ))
                    .await
                    .unwrap();
            }
            key => {
                self.send_process_input(idx, &key.into_bytes())
                    .await
                    .unwrap();
            }
        }
        false
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        repl_lines: usize,
    ) -> anyhow::Result<()> {
        let mut used_lines = repl_lines;
        let mut pos = None;
        for entry in self.entries.iter().rev() {
            let entry = entry.lock_arc().await;
            let screen = entry.vt.screen();
            let mut last_row = 0;
            for (idx, row) in screen.rows(0, 80).enumerate() {
                if !row.is_empty() {
                    last_row = idx + 1;
                }
            }
            used_lines += 1 + std::cmp::min(6, last_row);
            if used_lines > 24 {
                break;
            }
            if used_lines == 1 {
                used_lines = 2;
                pos = Some((23, 0));
            }
            out.move_to((24 - used_lines).try_into().unwrap(), 0);
            out.write_str("$ ");
            if entry.running {
                out.set_bgcolor(vt100::Color::Rgb(16, 64, 16));
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
                .rows_formatted(0, 80)
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
    pid: nix::unistd::Pid,
    vt: vt100::Parser,
    input: async_std::channel::Sender<Vec<u8>>,
    running: bool, // option end time
                   // start time
}

impl HistoryEntry {
    fn new(
        cmd: &str,
        pid: i32,
        input: async_std::channel::Sender<Vec<u8>>,
    ) -> Self {
        Self {
            cmd: cmd.into(),
            pid: nix::unistd::Pid::from_raw(pid),
            vt: vt100::Parser::new(24, 80, 0),
            input,
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
