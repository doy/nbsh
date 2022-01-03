use async_std::io::{ReadExt as _, WriteExt as _};
use futures_lite::future::FutureExt as _;

pub struct Pty {
    pty: async_std::sync::Arc<pty_process::Pty>,
}

impl Pty {
    pub fn new(
        size: (u16, u16),
        entry: &async_std::sync::Arc<async_std::sync::Mutex<super::Entry>>,
        input_r: async_std::channel::Receiver<Vec<u8>>,
        resize_r: async_std::channel::Receiver<(u16, u16)>,
        close_r: async_std::channel::Receiver<()>,
        event_w: async_std::channel::Sender<crate::event::Event>,
    ) -> anyhow::Result<Self> {
        let pty = pty_process::Pty::new()?;
        pty.resize(pty_process::Size::new(size.0, size.1))?;
        let pty = async_std::sync::Arc::new(pty);

        {
            let entry = async_std::sync::Arc::clone(entry);
            let pty = async_std::sync::Arc::clone(&pty);
            async_std::task::spawn(async move {
                loop {
                    enum Res {
                        Read(Result<usize, std::io::Error>),
                        Write(Result<Vec<u8>, async_std::channel::RecvError>),
                        Resize(
                            Result<(u16, u16), async_std::channel::RecvError>,
                        ),
                        Close(Result<(), async_std::channel::RecvError>),
                    }
                    let mut buf = [0_u8; 4096];
                    let read =
                        async { Res::Read((&*pty).read(&mut buf).await) };
                    let write = async { Res::Write(input_r.recv().await) };
                    let resize = async { Res::Resize(resize_r.recv().await) };
                    let close = async { Res::Close(close_r.recv().await) };
                    match read.race(write).race(resize).or(close).await {
                        Res::Read(res) => match res {
                            Ok(bytes) => {
                                let mut entry = entry.lock_arc().await;
                                let pre_alternate_screen =
                                    entry.vt.screen().alternate_screen();
                                entry.vt.process(&buf[..bytes]);
                                let post_alternate_screen =
                                    entry.vt.screen().alternate_screen();
                                if entry.fullscreen.is_none()
                                    && pre_alternate_screen
                                        != post_alternate_screen
                                {
                                    event_w.send(
                                        crate::event::Event::ProcessAlternateScreen,
                                    )
                                    .await
                                    .unwrap();
                                }
                                event_w
                                    .send(crate::event::Event::ProcessOutput)
                                    .await
                                    .unwrap();
                            }
                            Err(e) => {
                                if e.raw_os_error() != Some(libc::EIO) {
                                    panic!("pty read failed: {:?}", e);
                                }
                            }
                        },
                        Res::Write(res) => {
                            match res {
                                Ok(bytes) => {
                                    (&*pty).write(&bytes).await.unwrap();
                                }
                                Err(e) => {
                                    panic!("failed to read from input channel: {}", e);
                                }
                            }
                        }
                        Res::Resize(res) => match res {
                            Ok(size) => {
                                pty.resize(pty_process::Size::new(
                                    size.0, size.1,
                                ))
                                .unwrap();
                                entry
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
                        Res::Close(res) => match res {
                            Ok(()) => {
                                event_w
                                    .send(crate::event::Event::ProcessExit)
                                    .await
                                    .unwrap();
                                return;
                            }
                            Err(e) => {
                                panic!(
                                    "failed to read from close channel: {}",
                                    e
                                );
                            }
                        },
                    }
                }
            });
        }

        Ok(Self { pty })
    }

    pub fn spawn(
        &self,
        mut cmd: pty_process::Command,
    ) -> anyhow::Result<async_std::process::Child> {
        Ok(cmd.spawn(&self.pty)?)
    }
}
