#![warn(clippy::pedantic)]
#![warn(clippy::nursery)]
#![allow(clippy::missing_const_for_fn)]
#![allow(clippy::unused_self)]

mod history;
mod readline;
mod state;
mod util;

async fn async_main() -> anyhow::Result<()> {
    let mut input = textmode::Input::new().await?;
    let mut output = textmode::Output::new().await?;

    // avoid the guards getting stuck in a task that doesn't run to
    // completion
    let _input_guard = input.take_raw_guard();
    let _output_guard = output.take_screen_guard();

    let (action_w, action_r) = async_std::channel::unbounded();

    let state = util::mutex(state::State::new(action_w));

    state.lock_arc().await.render(&mut output).await.unwrap();

    {
        let state = async_std::sync::Arc::clone(&state);
        async_std::task::spawn(async move {
            while let Ok(action) = action_r.recv().await {
                state
                    .lock_arc()
                    .await
                    .handle_action(action, &mut output)
                    .await;
            }
        });
    }

    loop {
        let key = input.read_key().await.unwrap();
        if state.lock_arc().await.handle_input(key).await {
            break;
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
