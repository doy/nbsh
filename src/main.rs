#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::too_many_lines)]
#![allow(clippy::unused_self)]

mod action;
mod builtins;
mod format;
mod history;
mod parse;
mod readline;
mod state;
mod util;

use async_std::stream::StreamExt as _;

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

    let state = state::State::new();
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
                chrono::Local::now().timestamp_subsec_nanos().into(),
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
