use textmode::Textmode as _;

pub async fn run() -> anyhow::Result<()> {
    let mut input = textmode::Input::new().await?;
    let mut output = textmode::Output::new().await?;

    // avoid the guards getting stuck in a task that doesn't run to
    // completion
    let _input_guard = input.take_raw_guard();
    let _output_guard = output.take_screen_guard();

    let (action_w, action_r) = async_std::channel::unbounded();

    let repl = async_std::sync::Arc::new(async_std::sync::Mutex::new(
        crate::repl::Repl::new(action_w.clone()),
    ));
    let history = async_std::sync::Arc::new(async_std::sync::Mutex::new(
        crate::history::History::new(action_w),
    ));
    let input_source = async_std::sync::Arc::new(
        async_std::sync::Mutex::new(InputSource::Repl),
    );

    render(
        &mut output,
        &*repl.lock_arc().await,
        &*history.lock_arc().await,
        &*input_source.lock_arc().await,
    )
    .await
    .unwrap();

    {
        let repl = async_std::sync::Arc::clone(&repl);
        let history = async_std::sync::Arc::clone(&history);
        let input_source = async_std::sync::Arc::clone(&input_source);
        async_std::task::spawn(async move {
            while let Ok(action) = action_r.recv().await {
                handle_action(
                    action,
                    &mut output,
                    async_std::sync::Arc::clone(&repl),
                    async_std::sync::Arc::clone(&history),
                    async_std::sync::Arc::clone(&input_source),
                )
                .await;
            }
        });
    }

    loop {
        let quit = handle_input(
            &mut input,
            async_std::sync::Arc::clone(&repl),
            async_std::sync::Arc::clone(&history),
            async_std::sync::Arc::clone(&input_source),
        )
        .await;
        if quit {
            break;
        }
    }

    Ok(())
}

async fn handle_action(
    action: Action,
    output: &mut textmode::Output,
    repl: async_std::sync::Arc<async_std::sync::Mutex<crate::repl::Repl>>,
    history: async_std::sync::Arc<
        async_std::sync::Mutex<crate::history::History>,
    >,
    input_source: async_std::sync::Arc<async_std::sync::Mutex<InputSource>>,
) {
    match action {
        Action::Render => {
            render(
                output,
                &*repl.lock_arc().await,
                &*history.lock_arc().await,
                &*input_source.lock_arc().await,
            )
            .await
            .unwrap();
        }
        Action::Run(ref cmd) => {
            history.lock_arc().await.run(cmd).await.unwrap();
        }
        Action::UpdateFocus(new_input_source) => {
            *input_source.lock_arc().await = new_input_source;
        }
    }
}

async fn handle_input(
    input: &mut textmode::Input,
    repl: async_std::sync::Arc<async_std::sync::Mutex<crate::repl::Repl>>,
    history: async_std::sync::Arc<
        async_std::sync::Mutex<crate::history::History>,
    >,
    input_source: async_std::sync::Arc<async_std::sync::Mutex<InputSource>>,
) -> bool {
    let key = input.read_key().await.unwrap();
    if let Some(key) = key {
        let input_source = *input_source.lock_arc().await;
        let quit = match input_source {
            InputSource::Repl => repl.lock_arc().await.handle_key(key).await,
            InputSource::History(idx) => {
                history.lock_arc().await.handle_key(key, idx).await
            }
        };
        if quit {
            return true;
        }
    } else {
        return true;
    }
    false
}

async fn render(
    out: &mut textmode::Output,
    repl: &crate::repl::Repl,
    history: &crate::history::History,
    input_source: &InputSource,
) -> anyhow::Result<()> {
    out.clear();
    if let InputSource::Repl = input_source {
        history.render(out, repl.lines()).await?;
        repl.render(out).await?;
    } else {
        history.render(out, 0).await?;
    }
    out.refresh().await?;
    Ok(())
}

#[derive(Copy, Clone, Debug)]
pub enum InputSource {
    Repl,
    History(usize),
}

pub enum Action {
    Render,
    Run(String),
    UpdateFocus(InputSource),
}
