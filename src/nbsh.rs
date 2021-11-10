use textmode::Textmode as _;

pub struct Nbsh {
    repl: crate::repl::Repl,
    history: crate::history::History,
}

impl Nbsh {
    pub fn new() -> Self {
        Self {
            repl: crate::repl::Repl::new(),
            history: crate::history::History::new(),
        }
    }

    pub async fn run(self) -> anyhow::Result<()> {
        let mut input = textmode::Input::new().await?;
        let mut output = textmode::Output::new().await?;

        // avoid the guards getting stuck in a task that doesn't run to
        // completion
        let _input_guard = input.take_raw_guard();
        let _output_guard = output.take_screen_guard();

        let (run_w, run_r) = async_std::channel::unbounded();
        let (render_w, render_r) = async_std::channel::unbounded();

        self.render(&mut output).await.unwrap();

        let locked_self =
            async_std::sync::Arc::new(async_std::sync::Mutex::new(self));

        let readline_self = std::sync::Arc::clone(&locked_self);
        let readline_render = render_w.clone();
        let readline_task = async_std::task::spawn(async move {
            loop {
                let key = input.read_key().await.unwrap();
                let mut self_ = readline_self.lock_arc().await;
                let (last, cmd) = self_.handle_key(key);
                if last {
                    break;
                }
                if let Some(cmd) = cmd {
                    run_w.send(cmd).await.unwrap();
                }
                readline_render.send(()).await.unwrap();
            }
        });

        let history_self = std::sync::Arc::clone(&locked_self);
        let history_render = render_w.clone();
        async_std::task::spawn(async move {
            while let Ok(cmd) = run_r.recv().await {
                let mut self_ = history_self.lock_arc().await;
                self_
                    .history
                    .run(&cmd, history_render.clone())
                    .await
                    .unwrap();
            }
        });

        let render_self = std::sync::Arc::clone(&locked_self);
        async_std::task::spawn(async move {
            while let Ok(()) = render_r.recv().await {
                while let Ok(()) = render_r.try_recv() {}
                let self_ = render_self.lock_arc().await;
                self_.render(&mut output).await.unwrap();
            }
        });

        readline_task.await;

        Ok(())
    }

    fn handle_key(
        &mut self,
        key: Option<textmode::Key>,
    ) -> (bool, Option<String>) {
        let mut cmd = None;
        match key {
            Some(textmode::Key::String(s)) => self.repl.add_input(&s),
            Some(textmode::Key::Char(c)) => {
                self.repl.add_input(&c.to_string());
            }
            Some(textmode::Key::Ctrl(b'c')) => self.repl.clear_input(),
            Some(textmode::Key::Ctrl(b'd')) | None => return (true, None),
            Some(textmode::Key::Ctrl(b'm')) => {
                cmd = Some(self.repl.input());
                self.repl.clear_input();
            }
            Some(textmode::Key::Backspace) => self.repl.backspace(),
            _ => {}
        }
        (false, cmd)
    }

    async fn render(&self, out: &mut textmode::Output) -> anyhow::Result<()> {
        out.clear();
        self.history.render(out, self.repl.lines()).await?;
        self.repl.render(out).await?;
        out.refresh().await?;
        Ok(())
    }
}
