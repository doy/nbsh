use crate::shell::prelude::*;

#[derive(Debug)]
enum Request {
    Input(Vec<u8>),
    Resize(u16, u16),
}

pub struct Pty {
    vt: std::sync::Arc<std::sync::Mutex<super::pty::Vt>>,
    request_w: tokio::sync::mpsc::UnboundedSender<Request>,
}

impl Pty {
    pub fn new(
        size: (u16, u16),
        event_w: crate::shell::event::Writer,
    ) -> Result<(Self, pty_process::Pts)> {
        let (request_w, request_r) = tokio::sync::mpsc::unbounded_channel();

        let pty = pty_process::Pty::new()?;
        pty.resize(pty_process::Size::new(size.0, size.1))?;
        let pts = pty.pts()?;

        let vt = std::sync::Arc::new(std::sync::Mutex::new(
            super::pty::Vt::new(size),
        ));

        tokio::task::spawn(pty_task(
            pty,
            std::sync::Arc::clone(&vt),
            request_r,
            event_w,
        ));

        Ok((Self { vt, request_w }, pts))
    }

    pub fn with_vt<T>(&self, f: impl FnOnce(&super::pty::Vt) -> T) -> T {
        let vt = self.vt.lock().unwrap();
        f(&*vt)
    }

    pub fn with_vt_mut<T>(
        &self,
        f: impl FnOnce(&mut super::pty::Vt) -> T,
    ) -> T {
        let mut vt = self.vt.lock().unwrap();
        f(&mut *vt)
    }

    pub fn lock_vt(&self) -> std::sync::MutexGuard<super::pty::Vt> {
        self.vt.lock().unwrap()
    }

    pub fn fullscreen(&self) -> bool {
        self.with_vt(|vt| vt.screen().alternate_screen())
    }

    pub fn input(&self, bytes: Vec<u8>) {
        #[allow(clippy::let_underscore_drop)]
        let _ = self.request_w.send(Request::Input(bytes));
    }

    pub fn resize(&self, size: (u16, u16)) {
        #[allow(clippy::let_underscore_drop)]
        let _ = self.request_w.send(Request::Resize(size.0, size.1));
    }
}

pub struct Vt {
    vt: vt100::Parser,
    audible_bell_state: usize,
    visual_bell_state: usize,
    audible_bell: bool,
    visual_bell: bool,
    real_bell_pending: bool,
}

impl Vt {
    pub fn new(size: (u16, u16)) -> Self {
        Self {
            vt: vt100::Parser::new(size.0, size.1, 0),
            audible_bell_state: 0,
            visual_bell_state: 0,
            audible_bell: false,
            visual_bell: false,
            real_bell_pending: false,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.vt.process(bytes);
        let screen = self.vt.screen();

        let new_audible_bell_state = screen.audible_bell_count();
        if new_audible_bell_state != self.audible_bell_state {
            self.audible_bell = true;
            self.real_bell_pending = true;
            self.audible_bell_state = new_audible_bell_state;
        }

        let new_visual_bell_state = screen.visual_bell_count();
        if new_visual_bell_state != self.visual_bell_state {
            self.visual_bell = true;
            self.real_bell_pending = true;
            self.visual_bell_state = new_visual_bell_state;
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.vt.screen()
    }

    pub fn set_size(&mut self, size: (u16, u16)) {
        self.vt.set_size(size.0, size.1);
    }

    pub fn is_bell(&self) -> bool {
        self.audible_bell || self.visual_bell
    }

    pub fn bell(&mut self, out: &mut impl textmode::Textmode, focused: bool) {
        if self.real_bell_pending {
            if self.audible_bell {
                out.write(b"\x07");
            }
            if self.visual_bell {
                out.write(b"\x1bg");
            }
            self.real_bell_pending = false;
        }
        if focused {
            self.audible_bell = false;
            self.visual_bell = false;
        }
    }

    pub fn binary(&self) -> bool {
        self.vt.screen().errors() > 5
    }

    pub fn output_lines(&self, focused: bool, running: bool) -> usize {
        if self.binary() {
            return 1;
        }

        let screen = self.vt.screen();
        let mut last_row = 0;
        for (idx, row) in screen.rows(0, screen.size().1).enumerate() {
            if !row.is_empty() {
                last_row = idx + 1;
            }
        }
        if focused && running {
            last_row = std::cmp::max(
                last_row,
                usize::from(screen.cursor_position().0) + 1,
            );
        }
        last_row
    }
}

async fn pty_task(
    pty: pty_process::Pty,
    vt: std::sync::Arc<std::sync::Mutex<super::pty::Vt>>,
    request_r: tokio::sync::mpsc::UnboundedReceiver<Request>,
    event_w: crate::shell::event::Writer,
) {
    enum Res {
        Read(Result<bytes::Bytes, std::io::Error>),
        Request(Request),
    }

    let (pty_r, mut pty_w) = pty.into_split();
    let mut stream: futures_util::stream::SelectAll<_> = [
        tokio_util::io::ReaderStream::new(pty_r)
            .map(Res::Read)
            .boxed(),
        tokio_stream::wrappers::UnboundedReceiverStream::new(request_r)
            .map(Res::Request)
            .boxed(),
    ]
    .into_iter()
    .collect();
    while let Some(res) = stream.next().await {
        match res {
            Res::Read(res) => match res {
                Ok(bytes) => {
                    vt.lock().unwrap().process(&bytes);
                    event_w.send(Event::PtyOutput);
                }
                Err(e) => {
                    // this means that there are no longer any open pts fds.
                    // we could alternately signal this through an explicit
                    // channel at ChildExit time, but this seems reliable
                    // enough.
                    if e.raw_os_error() == Some(libc::EIO) {
                        return;
                    }
                    panic!("pty read failed: {:?}", e);
                }
            },
            Res::Request(Request::Input(bytes)) => {
                pty_w.write(&bytes).await.unwrap();
            }
            Res::Request(Request::Resize(row, col)) => {
                pty_w.resize(pty_process::Size::new(row, col)).unwrap();
                vt.lock().unwrap().set_size((row, col));
            }
        }
    }
}
