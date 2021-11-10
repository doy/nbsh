use async_std::io::ReadExt as _;
use pty_process::Command as _;
use textmode::Textmode as _;

pub struct History {
    entries: Vec<async_std::sync::Arc<async_std::sync::Mutex<HistoryEntry>>>,
    action: async_std::channel::Sender<crate::nbsh::Action>,
}

impl History {
    pub fn new(
        action: async_std::channel::Sender<crate::nbsh::Action>,
    ) -> Self {
        Self {
            entries: vec![],
            action,
        }
    }

    pub async fn run(&mut self, cmd: &str) -> anyhow::Result<usize> {
        let (exe, args) = parse_cmd(cmd);
        let mut process = async_process::Command::new(&exe);
        process.args(&args);
        let child = process
            .spawn_pty(Some(&pty_process::Size::new(24, 80)))
            .unwrap();
        let entry = async_std::sync::Arc::new(async_std::sync::Mutex::new(
            HistoryEntry::new(cmd),
        ));
        let task_entry = async_std::sync::Arc::clone(&entry);
        let task_action = self.action.clone();
        async_std::task::spawn(async move {
            loop {
                let mut buf = [0_u8; 4096];
                match child.pty().read(&mut buf).await {
                    Ok(bytes) => {
                        task_entry.lock_arc().await.vt.process(&buf[..bytes]);
                    }
                    Err(e) => {
                        if e.raw_os_error() != Some(libc::EIO) {
                            eprintln!("pty read failed: {:?}", e);
                        }
                        task_entry.lock_arc().await.running = false;
                        task_action
                            .send(crate::nbsh::Action::Render)
                            .await
                            .unwrap();
                        break;
                    }
                }
                task_action.send(crate::nbsh::Action::Render).await.unwrap();
            }
        });
        self.entries.push(entry);
        self.action.send(crate::nbsh::Action::Render).await.unwrap();
        Ok(self.entries.len() - 1)
    }

    pub async fn handle_key(
        &mut self,
        key: textmode::Key,
        idx: usize,
    ) -> bool {
        if let textmode::Key::Bytes(b) = key {
            self.send_process_input(idx, &b).await.unwrap();
        } else {
            unreachable!();
        }
        false
    }

    pub async fn render(
        &self,
        out: &mut textmode::Output,
        repl_lines: usize,
    ) -> anyhow::Result<()> {
        let mut used_lines = repl_lines;
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
            out.move_to((24 - used_lines).try_into().unwrap(), 0);
            out.write_str("$ ");
            if entry.running {
                out.set_bgcolor(vt100::Color::Rgb(16, 64, 16));
            }
            out.write_str(&entry.cmd);
            out.reset_attributes();
            out.write(b"\r\n");
            if last_row > 5 {
                out.set_bgcolor(textmode::color::RED);
                out.write(b"...");
                out.reset_attributes();
                out.write(b"\r\n");
            }
            for row in screen
                .rows_formatted(0, 80)
                .take(last_row)
                .skip(last_row.saturating_sub(5))
            {
                out.write(&row);
                out.write(b"\r\n");
            }
            out.reset_attributes();
        }
        Ok(())
    }

    async fn send_process_input(
        &self,
        idx: usize,
        input: &[u8],
    ) -> anyhow::Result<()> {
        todo!()
    }
}

struct HistoryEntry {
    cmd: String,
    vt: vt100::Parser,
    running: bool, // option end time
                   // start time
}

impl HistoryEntry {
    fn new(cmd: &str) -> Self {
        Self {
            cmd: cmd.into(),
            vt: vt100::Parser::new(24, 80, 0),
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
