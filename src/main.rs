#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::too_many_arguments)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::unused_self)]

mod action;
mod builtins;
mod env;
mod format;
mod history;
mod parse;
mod readline;
mod state;
mod util;

use async_std::stream::StreamExt as _;

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

async fn resize(
    action_w: &async_std::channel::Sender<crate::action::Action>,
) {
    let size = terminal_size::terminal_size().map_or(
        (24, 80),
        |(terminal_size::Width(w), terminal_size::Height(h))| (h, w),
    );
    action_w
        .send(crate::action::Action::Resize(size))
        .await
        .unwrap();
}

async fn async_main() -> anyhow::Result<()> {
    let mut input = textmode::Input::new().await?;
    let mut output = textmode::Output::new().await?;

    // avoid the guards getting stuck in a task that doesn't run to
    // completion
    let _input_guard = input.take_raw_guard();
    let _output_guard = output.take_screen_guard();

    let (action_w, action_r) = async_std::channel::unbounded();

    let state = state::State::new(get_offset());
    state.render(&mut output, true).await.unwrap();

    let state = util::mutex(state);

    {
        let mut signals = signal_hook_async_std::Signals::new(&[
            signal_hook::consts::signal::SIGWINCH,
        ])?;
        let action_w = action_w.clone();
        async_std::task::spawn(async move {
            while signals.next().await.is_some() {
                resize(&action_w).await;
            }
        });
    }

    resize(&action_w).await;

    {
        let state = async_std::sync::Arc::clone(&state);
        let action_w = action_w.clone();
        async_std::task::spawn(async move {
            while let Some(key) = input.read_key().await.unwrap() {
                if let Some(action) =
                    state.lock_arc().await.handle_key(key).await
                {
                    action_w.send(action).await.unwrap();
                }
            }
        });
    }

    // redraw the clock every second
    {
        let action_w = action_w.clone();
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
            action_w.send(crate::action::Action::Render).await.unwrap();
            while interval.next().await.is_some() {
                action_w.send(crate::action::Action::Render).await.unwrap();
            }
        });
    }

    let action_reader = action::Reader::new(action_r);
    while let Some(action) = action_reader.recv().await {
        state
            .lock_arc()
            .await
            .handle_action(action, &mut output, &action_w)
            .await;
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
