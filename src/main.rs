#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::future_not_send)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::struct_excessive_bools)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::unused_self)]

mod builtins;
mod env;
mod event;
mod format;
mod history;
mod parse;
mod readline;
mod state;
mod util;

use async_std::stream::StreamExt as _;
use textmode::Textmode as _;

// the time crate is currently unable to get the local offset on unix due to
// soundness concerns, so we have to do it manually/:
//
// https://github.com/time-rs/time/issues/380
fn get_offset() -> time::UtcOffset {
    let offset_str =
        std::process::Command::new("date").args(&["+%:z"]).output();
    if let Ok(offset_str) = offset_str {
        let offset_str = String::from_utf8(offset_str.stdout).unwrap();
        time::UtcOffset::parse(
            offset_str.trim(),
            &time::format_description::parse("[offset_hour]:[offset_minute]")
                .unwrap(),
        )
        .unwrap_or(time::UtcOffset::UTC)
    } else {
        time::UtcOffset::UTC
    }
}

async fn async_main() -> anyhow::Result<()> {
    let mut input = textmode::Input::new().await?;
    let mut output = textmode::Output::new().await?;

    // avoid the guards getting stuck in a task that doesn't run to
    // completion
    let _input_guard = input.take_raw_guard();
    let _output_guard = output.take_screen_guard();

    let (event_w, event_r) = async_std::channel::unbounded();

    {
        let signals = signal_hook_async_std::Signals::new(&[
            signal_hook::consts::signal::SIGWINCH,
        ])?;
        let event_w = event_w.clone();
        async_std::task::spawn(async move {
            let mut signals = async_std::stream::once(
                signal_hook::consts::signal::SIGWINCH,
            )
            .chain(signals);
            while signals.next().await.is_some() {
                event_w
                    .send(crate::event::Event::Resize(
                        terminal_size::terminal_size().map_or(
                            (24, 80),
                            |(
                                terminal_size::Width(w),
                                terminal_size::Height(h),
                            )| { (h, w) },
                        ),
                    ))
                    .await
                    .unwrap();
            }
        });
    }

    {
        let event_w = event_w.clone();
        async_std::task::spawn(async move {
            while let Some(key) = input.read_key().await.unwrap() {
                event_w.send(event::Event::Key(key)).await.unwrap();
            }
        });
    }

    // redraw the clock every second
    {
        let event_w = event_w.clone();
        async_std::task::spawn(async move {
            let first_sleep = 1_000_000_000_u64.saturating_sub(
                time::OffsetDateTime::now_utc().nanosecond().into(),
            );
            async_std::task::sleep(std::time::Duration::from_nanos(
                first_sleep,
            ))
            .await;
            let mut interval = async_std::stream::interval(
                std::time::Duration::from_secs(1),
            );
            event_w.send(crate::event::Event::ClockTimer).await.unwrap();
            while interval.next().await.is_some() {
                event_w.send(crate::event::Event::ClockTimer).await.unwrap();
            }
        });
    }

    let mut state = state::State::new(get_offset());
    let event_reader = event::Reader::new(event_r);
    while let Some(event) = event_reader.recv().await {
        match state.handle_event(event, &event_w).await {
            Some(state::Action::Refresh) => {
                state.render(&mut output).await?;
                output.refresh().await?;
            }
            Some(state::Action::HardRefresh) => {
                state.render(&mut output).await?;
                output.hard_refresh().await?;
            }
            Some(state::Action::Resize(rows, cols)) => {
                output.set_size(rows, cols);
                state.render(&mut output).await?;
                output.hard_refresh().await?;
            }
            Some(state::Action::Quit) => break,
            None => {}
        }
    }

    Ok(())
}

fn main() {
    match async_std::task::block_on(async_main()) {
        Ok(_) => (),
        Err(e) => {
            eprintln!("nbsh: {}", e);
            std::process::exit(1);
        }
    };
}
