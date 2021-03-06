use futures::stream::Stream as _;
use snafu::ResultExt as _;
use std::io::Write as _;

#[derive(Debug, snafu::Snafu)]
pub enum Error {
    #[snafu(display("failed to write to the terminal: {}", source))]
    WriteToTerminal { source: std::io::Error },

    #[snafu(display("end of input"))]
    EOF,

    #[snafu(display(
        "failed to put the terminal into raw mode: {}",
        source
    ))]
    IntoRawMode { source: crossterm::ErrorKind },

    #[snafu(display("{}", source))]
    KeyReader { source: crate::key_reader::Error },
}

pub type Result<T> = std::result::Result<T, Error>;

pub fn readline() -> Readline {
    Readline::new()
}

pub struct Readline {
    reader: crate::key_reader::KeyReader,
    state: ReadlineState,
    raw_screen: Option<crossterm::screen::RawScreen>,
}

struct ReadlineState {
    prompt: String,
    echo: bool,
    output: bool,
    manage_screen: bool,

    buffer: String,
    cursor: usize,
    wrote_prompt: bool,
}

impl Readline {
    pub fn new() -> Self {
        Self {
            reader: crate::key_reader::KeyReader::new(),
            state: ReadlineState {
                prompt: String::from("$ "),
                echo: true,
                output: true,
                manage_screen: true,
                buffer: String::new(),
                cursor: 0,
                wrote_prompt: false,
            },
            raw_screen: None,
        }
    }

    #[allow(dead_code)]
    pub fn prompt(mut self, prompt: &str) -> Self {
        self.state.prompt = prompt.to_string();
        self
    }

    #[allow(dead_code)]
    pub fn echo(mut self, echo: bool) -> Self {
        self.state.echo = echo;
        self
    }

    #[allow(dead_code)]
    pub fn disable_output(mut self, disable: bool) -> Self {
        self.state.output = !disable;
        self
    }

    pub fn set_raw(mut self, raw: bool) -> Self {
        self.state.manage_screen = raw;
        self
    }

    #[allow(dead_code)]
    pub fn cursor_pos(&self) -> usize {
        self.state.cursor
    }
}

impl ReadlineState {
    fn process_event(
        &mut self,
        event: &crossterm::input::InputEvent,
    ) -> Result<futures::Async<String>> {
        match event {
            crossterm::input::InputEvent::Keyboard(e) => {
                return self.process_keyboard_event(*e);
            }
            _ => {}
        }

        Ok(futures::Async::NotReady)
    }

    fn process_keyboard_event(
        &mut self,
        event: crossterm::input::KeyEvent,
    ) -> Result<futures::Async<String>> {
        match event {
            crossterm::input::KeyEvent::Char('\n') => {
                self.echo_char('\n').context(WriteToTerminal)?;
                return Ok(futures::Async::Ready(self.buffer.clone()));
            }
            crossterm::input::KeyEvent::Char('\t') => {
                // TODO
            }
            crossterm::input::KeyEvent::Char(c) => {
                if self.cursor != self.buffer.len() {
                    self.echo(b"\x1b[@").context(WriteToTerminal)?;
                }
                self.echo_char(c).context(WriteToTerminal)?;
                self.buffer.insert(self.cursor, c);
                self.cursor += 1;
            }
            crossterm::input::KeyEvent::Ctrl(c) => match c {
                'a' => {
                    if self.cursor != 0 {
                        self.echo(
                            &format!("\x1b[{}D", self.cursor).into_bytes(),
                        )
                        .context(WriteToTerminal)?;
                        self.cursor = 0;
                    }
                }
                'c' => {
                    self.buffer = String::new();
                    self.cursor = 0;
                    self.echo_char('\n').context(WriteToTerminal)?;
                    self.prompt().context(WriteToTerminal)?;
                }
                'd' => {
                    if self.buffer.is_empty() {
                        self.echo_char('\n').context(WriteToTerminal)?;
                        return EOF.fail();
                    }
                }
                'e' => {
                    if self.cursor != self.buffer.len() {
                        self.echo(
                            &format!(
                                "\x1b[{}C",
                                self.buffer.len() - self.cursor
                            )
                            .into_bytes(),
                        )
                        .context(WriteToTerminal)?;
                        self.cursor = self.buffer.len();
                    }
                }
                'u' => {
                    if self.cursor != 0 {
                        self.echo(
                            std::iter::repeat(b'\x08')
                                .take(self.cursor)
                                .chain(
                                    format!("\x1b[{}P", self.cursor)
                                        .into_bytes(),
                                )
                                .collect::<Vec<_>>()
                                .as_ref(),
                        )
                        .context(WriteToTerminal)?;
                        self.buffer = self.buffer.split_off(self.cursor);
                        self.cursor = 0;
                    }
                }
                _ => {}
            },
            crossterm::input::KeyEvent::Backspace => {
                if self.cursor != 0 {
                    self.cursor -= 1;
                    self.buffer.remove(self.cursor);
                    if self.cursor == self.buffer.len() {
                        self.echo(b"\x08 \x08").context(WriteToTerminal)?;
                    } else {
                        self.echo(b"\x08\x1b[P").context(WriteToTerminal)?;
                    }
                }
            }
            crossterm::input::KeyEvent::Left => {
                if self.cursor != 0 {
                    self.cursor -= 1;
                    self.write(b"\x1b[D").context(WriteToTerminal)?;
                }
            }
            crossterm::input::KeyEvent::Right => {
                if self.cursor != self.buffer.len() {
                    self.cursor += 1;
                    self.write(b"\x1b[C").context(WriteToTerminal)?;
                }
            }
            crossterm::input::KeyEvent::Delete => {
                if self.cursor != self.buffer.len() {
                    self.buffer.remove(self.cursor);
                    self.echo(b"\x1b[P").context(WriteToTerminal)?;
                }
            }
            _ => {}
        }

        Ok(futures::Async::NotReady)
    }

    fn write(&self, buf: &[u8]) -> std::io::Result<()> {
        if !self.output {
            return Ok(());
        }

        let stdout = std::io::stdout();
        let mut stdout = stdout.lock();
        stdout.write_all(buf)?;
        stdout.flush()
    }

    fn prompt(&self) -> std::io::Result<()> {
        self.write(self.prompt.as_bytes())
    }

    fn echo(&self, bytes: &[u8]) -> std::io::Result<()> {
        let bytes: Vec<_> = bytes
            .iter()
            // replace \n with \r\n
            .fold(vec![], |mut acc, &c| {
                if c == b'\n' {
                    acc.push(b'\r');
                    acc.push(b'\n');
                } else {
                    if self.echo {
                        acc.push(c);
                    }
                }
                acc
            });
        self.write(&bytes)
    }

    fn echo_char(&self, c: char) -> std::io::Result<()> {
        let mut buf = [0_u8; 4];
        self.echo(c.encode_utf8(&mut buf[..]).as_bytes())
    }
}

impl std::fmt::Display for Readline {
    fn fmt(
        &self,
        f: &mut std::fmt::Formatter,
    ) -> std::result::Result<(), std::fmt::Error> {
        write!(f, "{}{}", self.state.prompt, self.state.buffer)
    }
}

#[must_use = "futures do nothing unless polled"]
impl futures::future::Future for Readline {
    type Item = String;
    type Error = Error;

    fn poll(&mut self) -> futures::Poll<Self::Item, Self::Error> {
        if !self.state.wrote_prompt {
            self.state.prompt().context(WriteToTerminal)?;
            self.state.wrote_prompt = true;
        }

        if self.state.manage_screen && self.raw_screen.is_none() {
            self.raw_screen = Some(
                crossterm::screen::RawScreen::into_raw_mode()
                    .context(IntoRawMode)?,
            );
        }

        loop {
            if let Some(event) =
                futures::try_ready!(self.reader.poll().context(KeyReader))
            {
                let a = self.state.process_event(&event)?;
                if a.is_ready() {
                    return Ok(a);
                }
            } else {
                eprintln!("EEEOOOFFF");
                return Err(Error::EOF);
            }
        }
    }
}
