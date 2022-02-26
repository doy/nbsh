use crate::shell::prelude::*;

pub struct Pty {
    pts: pty_process::Pts,
    close_w: tokio::sync::mpsc::UnboundedSender<()>,
}

impl Pty {
    pub fn new(
        size: (u16, u16),
        entry: &crate::mutex::Mutex<super::Entry>,
        input_r: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
        resize_r: tokio::sync::mpsc::UnboundedReceiver<(u16, u16)>,
        event_w: crate::shell::event::Writer,
    ) -> anyhow::Result<Self> {
        let (close_w, close_r) = tokio::sync::mpsc::unbounded_channel();

        let pty = pty_process::Pty::new()?;
        pty.resize(pty_process::Size::new(size.0, size.1))?;
        let pts = pty.pts()?;

        tokio::task::spawn(pty_task(
            pty,
            crate::mutex::clone(entry),
            input_r,
            resize_r,
            close_r,
            event_w,
        ));

        Ok(Self { pts, close_w })
    }

    pub fn spawn(
        &self,
        mut cmd: pty_process::Command,
    ) -> anyhow::Result<tokio::process::Child> {
        Ok(cmd.spawn(&self.pts)?)
    }

    pub async fn close(&self) {
        self.close_w.send(()).unwrap();
    }
}

async fn pty_task(
    pty: pty_process::Pty,
    entry: crate::mutex::Mutex<super::Entry>,
    input_r: tokio::sync::mpsc::UnboundedReceiver<Vec<u8>>,
    resize_r: tokio::sync::mpsc::UnboundedReceiver<(u16, u16)>,
    close_r: tokio::sync::mpsc::UnboundedReceiver<()>,
    event_w: crate::shell::event::Writer,
) {
    enum Res {
        Read(Result<bytes::Bytes, std::io::Error>),
        Write(Vec<u8>),
        Resize((u16, u16)),
        Close(()),
    }

    let (pty_r, mut pty_w) = pty.into_split();
    let mut stream: futures_util::stream::SelectAll<_> = [
        tokio_util::io::ReaderStream::new(pty_r)
            .map(Res::Read)
            .boxed(),
        tokio_stream::wrappers::UnboundedReceiverStream::new(input_r)
            .map(Res::Write)
            .boxed(),
        tokio_stream::wrappers::UnboundedReceiverStream::new(resize_r)
            .map(Res::Resize)
            .boxed(),
        tokio_stream::wrappers::UnboundedReceiverStream::new(close_r)
            .map(Res::Close)
            .boxed(),
    ]
    .into_iter()
    .collect();
    while let Some(res) = stream.next().await {
        match res {
            Res::Read(res) => match res {
                Ok(bytes) => {
                    entry.clone().lock_owned().await.process(&bytes);
                    event_w.send(Event::PtyOutput);
                }
                Err(e) => {
                    panic!("pty read failed: {:?}", e);
                }
            },
            Res::Write(bytes) => {
                pty_w.write(&bytes).await.unwrap();
            }
            Res::Resize(size) => pty_w
                .resize(pty_process::Size::new(size.0, size.1))
                .unwrap(),
            Res::Close(()) => {
                event_w.send(Event::PtyClose);
                return;
            }
        }
    }
}
