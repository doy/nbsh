use textmode::Textmode as _;

pub struct Nbsh {
    repl: crate::repl::Repl,
    history: crate::history::History,

    action: async_std::channel::Receiver<Action>,
}

impl Nbsh {
    pub fn new() -> Self {
        let (action_w, action_r) = async_std::channel::unbounded();
        Self {
            repl: crate::repl::Repl::new(action_w.clone()),
            history: crate::history::History::new(action_w),
            action: action_r,
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let mut input = textmode::Input::new().await?;
        let mut output = textmode::Output::new().await?;

        // avoid the guards getting stuck in a task that doesn't run to
        // completion
        let _input_guard = input.take_raw_guard();
        let _output_guard = output.take_screen_guard();

        let Self {
            repl,
            history,
            action,
        } = self;

        let repl =
            async_std::sync::Arc::new(async_std::sync::Mutex::new(repl));
        let history =
            async_std::sync::Arc::new(async_std::sync::Mutex::new(history));
        let input_source = async_std::sync::Arc::new(
            async_std::sync::Mutex::new(InputSource::Repl),
        );

        render(
            &mut output,
            &*repl.lock_arc().await,
            &*history.lock_arc().await,
        )
        .await
        .unwrap();

        let action_history = async_std::sync::Arc::clone(&history);
        let action_repl = async_std::sync::Arc::clone(&repl);
        let action_input_source = async_std::sync::Arc::clone(&input_source);
        async_std::task::spawn(async move {
            while let Ok(action) = action.recv().await {
                match action {
                    Action::Render => {
                        render(
                            &mut output,
                            &*action_repl.lock_arc().await,
                            &*action_history.lock_arc().await,
                        )
                        .await
                        .unwrap();
                    }
                    Action::Run(cmd) => {
                        action_history
                            .lock_arc()
                            .await
                            .run(&cmd)
                            .await
                            .unwrap();
                    }
                    Action::UpdateFocus(new_input_source) => {
                        *action_input_source.lock_arc().await =
                            new_input_source;
                    }
                }
            }
        });

        loop {
            let input_source = *input_source.lock_arc().await;
            match input_source {
                InputSource::Repl => {
                    input.parse_utf8(true);
                    input.parse_ctrl(true);
                    input.parse_meta(true);
                    input.parse_special_keys(true);
                    input.parse_single(false);
                }
                InputSource::History(_) => {
                    input.parse_utf8(false);
                    input.parse_ctrl(false);
                    input.parse_meta(false);
                    input.parse_special_keys(false);
                    input.parse_single(false);
                }
            }
            let key = input.read_key().await.unwrap();
            if let Some(key) = key {
                let quit = match input_source {
                    InputSource::Repl => {
                        repl.lock_arc().await.handle_key(key).await
                    }
                    InputSource::History(idx) => {
                        history.lock_arc().await.handle_key(key, idx).await
                    }
                };
                if quit {
                    break;
                }
            } else {
                break;
            }
        }

        Ok(())
    }
}

async fn render(
    out: &mut textmode::Output,
    repl: &crate::repl::Repl,
    history: &crate::history::History,
) -> anyhow::Result<()> {
    out.clear();
    history.render(out, repl.lines()).await?;
    repl.render(out).await?;
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
