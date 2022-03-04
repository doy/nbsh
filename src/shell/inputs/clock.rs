use crate::shell::prelude::*;

pub struct Handler;

impl Handler {
    pub fn new(event_w: crate::shell::event::Writer) -> Self {
        tokio::spawn(Self::task(event_w));
        Self
    }

    async fn task(event_w: crate::shell::event::Writer) {
        let now_clock = time::OffsetDateTime::now_utc();
        let now_instant = tokio::time::Instant::now();
        let mut interval = tokio::time::interval_at(
            now_instant
                + std::time::Duration::from_nanos(
                    1_000_000_000_u64
                        .saturating_sub(now_clock.nanosecond().into()),
                ),
            std::time::Duration::from_secs(1),
        );
        loop {
            interval.tick().await;
            event_w.send(Event::ClockTimer);
        }
    }
}
