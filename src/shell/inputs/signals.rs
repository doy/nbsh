use crate::shell::prelude::*;

pub struct Handler;

impl Handler {
    pub fn new(event_w: crate::shell::event::Writer) -> Result<Self> {
        let signals = tokio::signal::unix::signal(
            tokio::signal::unix::SignalKind::window_change(),
        )?;
        tokio::spawn(Self::task(signals, event_w));
        Ok(Self)
    }

    async fn task(
        mut signals: tokio::signal::unix::Signal,
        event_w: crate::shell::event::Writer,
    ) {
        event_w.send(resize_event());
        while signals.recv().await.is_some() {
            event_w.send(resize_event());
        }
    }
}

fn resize_event() -> Event {
    Event::Resize(terminal_size::terminal_size().map_or(
        (24, 80),
        |(terminal_size::Width(w), terminal_size::Height(h))| (h, w),
    ))
}
