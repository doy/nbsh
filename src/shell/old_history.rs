use crate::shell::prelude::*;

use tokio::io::AsyncBufReadExt as _;

use pest::Parser as _;

#[derive(pest_derive::Parser)]
#[grammar = "history.pest"]
struct HistoryLine;

pub struct History {
    entries: std::sync::Arc<std::sync::Mutex<Vec<Entry>>>,
}

impl History {
    pub fn new() -> Self {
        let entries = std::sync::Arc::new(std::sync::Mutex::new(vec![]));
        tokio::spawn(Self::task(std::sync::Arc::clone(&entries)));
        Self { entries }
    }

    pub fn entry_count(&self) -> usize {
        self.entries.lock().unwrap().len()
    }

    async fn task(entries: std::sync::Arc<std::sync::Mutex<Vec<Entry>>>) {
        // TODO: we should actually read this in reverse order, because we
        // want to populate the most recent entries first
        let mut stream = tokio_stream::wrappers::LinesStream::new(
            tokio::io::BufReader::new(
                tokio::fs::File::open(crate::dirs::history_file())
                    .await
                    .unwrap(),
            )
            .lines(),
        );
        while let Some(line) = stream.next().await {
            let line = if let Ok(line) = line {
                line
            } else {
                continue;
            };
            let entry = if let Ok(entry) = line.parse() {
                entry
            } else {
                continue;
            };
            entries.lock().unwrap().push(entry);
        }
    }
}

pub struct Entry {
    cmdline: String,
    start_time: Option<time::OffsetDateTime>,
    duration: Option<std::time::Duration>,
}

impl Entry {
    pub fn render(
        &self,
        out: &mut impl textmode::Textmode,
        offset: time::UtcOffset,
    ) {
        let size = out.screen().size();
        let mut time = "".to_string();
        if let Some(duration) = self.duration {
            time.push_str(&crate::format::duration(duration));
        }
        if let Some(start_time) = self.start_time {
            time.push_str(&crate::format::time(start_time.to_offset(offset)));
        }

        out.write_str("       $ ");
        let start = usize::from(out.screen().cursor_position().1);
        let end = usize::from(size.1) - time.len() - 2;
        let max_len = end - start;
        let cmd = if self.cmdline.len() > max_len {
            &self.cmdline[..(max_len - 4)]
        } else {
            &self.cmdline
        };
        out.write_str(cmd);
        if self.cmdline.len() > max_len {
            out.write_str(" ");
            out.set_fgcolor(textmode::color::BLUE);
            out.write_str("...");
        }
        out.reset_attributes();

        out.set_bgcolor(textmode::Color::Rgb(0x20, 0x20, 0x20));
        let cur_pos = out.screen().cursor_position();
        out.write_str(&" ".repeat(
            usize::from(size.1) - time.len() - 1 - usize::from(cur_pos.1),
        ));
        out.write_str(&time);
        out.write_str(" ");
        out.reset_attributes();
    }

    pub fn cmd(&self) -> &str {
        &self.cmdline
    }
}

impl std::str::FromStr for Entry {
    type Err = anyhow::Error;

    fn from_str(line: &str) -> std::result::Result<Self, Self::Err> {
        let mut parsed =
            HistoryLine::parse(Rule::line, line).map_err(|e| anyhow!(e))?;
        let line = parsed.next().unwrap();
        assert!(matches!(line.as_rule(), Rule::line));

        let mut start_time = None;
        let mut duration = None;
        let mut cmdline = None;
        for part in line.into_inner() {
            match part.as_rule() {
                Rule::time => {
                    start_time =
                        Some(time::OffsetDateTime::from_unix_timestamp(
                            part.as_str().parse()?,
                        )?);
                }
                Rule::duration => {
                    if part.as_str() == "0" {
                        continue;
                    }
                    let mut dur_parts = part.as_str().split('.');
                    let secs: u64 = dur_parts.next().unwrap().parse()?;
                    let nsec_str = dur_parts.next().unwrap_or("0");
                    let nsec_str = &nsec_str[..9.min(nsec_str.len())];
                    let nsecs: u64 = nsec_str.parse()?;
                    duration = Some(std::time::Duration::from_nanos(
                        secs * 1_000_000_000
                            + nsecs
                                * (10u64.pow(
                                    (9 - nsec_str.len()).try_into().unwrap(),
                                )),
                    ));
                }
                Rule::command => {
                    cmdline = Some(part.as_str().to_string());
                }
                Rule::line => unreachable!(),
                Rule::EOI => break,
            }
        }

        Ok(Self {
            cmdline: cmdline.unwrap(),
            start_time,
            duration,
        })
    }
}

#[test]
fn test_parse() {
    let entry: Entry =
        ": 1646779848:1234.56;vim ~/.zsh_history".parse().unwrap();
    assert_eq!(entry.cmdline, "vim ~/.zsh_history");
    assert_eq!(
        entry.duration,
        Some(std::time::Duration::from_nanos(1_234_560_000_000))
    );
    assert_eq!(
        entry.start_time,
        Some(time::macros::datetime!(2022-03-08 22:50:48).assume_utc())
    );

    let entry: Entry = ": 1646779848:1;vim ~/.zsh_history".parse().unwrap();
    assert_eq!(entry.cmdline, "vim ~/.zsh_history");
    assert_eq!(entry.duration, Some(std::time::Duration::from_secs(1)));
    assert_eq!(
        entry.start_time,
        Some(time::macros::datetime!(2022-03-08 22:50:48).assume_utc())
    );

    let entry: Entry = "vim ~/.zsh_history".parse().unwrap();
    assert_eq!(entry.cmdline, "vim ~/.zsh_history");
    assert_eq!(entry.duration, None);
    assert_eq!(entry.start_time, None);
}
