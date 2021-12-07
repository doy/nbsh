#[derive(Debug)]
pub enum Action {
    Render,
    ForceRedraw,
    Run(String),
    UpdateFocus(crate::state::Focus),
    UpdateScene(crate::state::Scene),
    CheckUpdateScene,
    Resize((u16, u16)),
    Quit,
}

pub struct Reader {
    pending: async_std::sync::Mutex<Pending>,
    cvar: async_std::sync::Condvar,
}

impl Reader {
    pub fn new(
        input: async_std::channel::Receiver<Action>,
    ) -> async_std::sync::Arc<Self> {
        let this = std::sync::Arc::new(Self {
            pending: async_std::sync::Mutex::new(Pending::new()),
            cvar: async_std::sync::Condvar::new(),
        });
        {
            let this = std::sync::Arc::clone(&this);
            async_std::task::spawn(async move {
                while let Ok(action) = input.recv().await {
                    this.new_action(Some(action)).await;
                }
                this.new_action(None).await;
            });
        }
        this
    }

    pub async fn recv(&self) -> Option<Action> {
        let mut pending = self
            .cvar
            .wait_until(self.pending.lock().await, |pending| {
                pending.has_action()
            })
            .await;
        pending.get_action()
    }

    async fn new_action(&self, action: Option<Action>) {
        let mut pending = self.pending.lock().await;
        pending.new_action(&action);
        self.cvar.notify_one();
    }
}

#[derive(Default)]
struct Pending {
    render: Option<()>,
    force_redraw: Option<()>,
    run: std::collections::VecDeque<String>,
    focus: Option<crate::state::Focus>,
    scene: Option<crate::state::Scene>,
    check_scene: Option<()>,
    size: Option<(u16, u16)>,
    done: bool,
}

impl Pending {
    fn new() -> Self {
        Self::default()
    }

    fn has_action(&self) -> bool {
        self.done
            || self.render.is_some()
            || self.force_redraw.is_some()
            || !self.run.is_empty()
            || self.focus.is_some()
            || self.scene.is_some()
            || self.check_scene.is_some()
            || self.size.is_some()
    }

    fn get_action(&mut self) -> Option<Action> {
        if self.size.is_some() {
            return Some(Action::Resize(self.size.take().unwrap()));
        }
        if !self.run.is_empty() {
            return Some(Action::Run(self.run.pop_front().unwrap()));
        }
        if self.focus.is_some() {
            return Some(Action::UpdateFocus(self.focus.take().unwrap()));
        }
        if self.scene.is_some() {
            return Some(Action::UpdateScene(self.scene.take().unwrap()));
        }
        if self.check_scene.take().is_some() {
            return Some(Action::CheckUpdateScene);
        }
        if self.force_redraw.take().is_some() {
            self.render.take();
            return Some(Action::ForceRedraw);
        }
        if self.render.take().is_some() {
            return Some(Action::Render);
        }
        if self.done {
            return None;
        }
        unreachable!()
    }

    fn new_action(&mut self, action: &Option<Action>) {
        match action {
            Some(Action::Render) => self.render = Some(()),
            Some(Action::ForceRedraw) => self.force_redraw = Some(()),
            Some(Action::Run(cmd)) => self.run.push_back(cmd.to_string()),
            Some(Action::UpdateFocus(focus)) => self.focus = Some(*focus),
            Some(Action::UpdateScene(scene)) => self.scene = Some(*scene),
            Some(Action::CheckUpdateScene) => self.check_scene = Some(()),
            Some(Action::Resize(size)) => self.size = Some(*size),
            Some(Action::Quit) | None => self.done = true,
        }
    }
}
