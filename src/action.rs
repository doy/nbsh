#[derive(Debug)]
pub enum Action {
    Render,
    ForceRedraw,
    Run(String),
    UpdateFocus(crate::state::Focus),
    Resize((u16, u16)),
    Quit,
}

pub struct Debouncer {
    pending: async_std::sync::Mutex<Pending>,
    cvar: async_std::sync::Condvar,
}

impl Debouncer {
    pub async fn recv(&self) -> Option<Action> {
        let mut pending = self
            .cvar
            .wait_until(self.pending.lock().await, |pending| {
                pending.has_event()
            })
            .await;
        pending.get_event()
    }

    async fn send(&self, action: Option<Action>) {
        let mut pending = self.pending.lock().await;
        pending.new_event(&action);
        self.cvar.notify_one();
    }
}

#[derive(Default)]
struct Pending {
    render: Option<()>,
    force_redraw: Option<()>,
    run: std::collections::VecDeque<String>,
    focus: Option<crate::state::Focus>,
    size: Option<(u16, u16)>,
    done: bool,
}

impl Pending {
    fn new() -> Self {
        Self::default()
    }

    fn has_event(&self) -> bool {
        self.done
            || self.render.is_some()
            || self.force_redraw.is_some()
            || !self.run.is_empty()
            || self.focus.is_some()
            || self.size.is_some()
    }

    fn get_event(&mut self) -> Option<Action> {
        if self.size.is_some() {
            return Some(Action::Resize(self.size.take().unwrap()));
        }
        if !self.run.is_empty() {
            return Some(Action::Run(self.run.pop_front().unwrap()));
        }
        if self.focus.is_some() {
            return Some(Action::UpdateFocus(self.focus.take().unwrap()));
        }
        if self.force_redraw.is_some() {
            self.force_redraw.take();
            self.render.take();
            return Some(Action::ForceRedraw);
        }
        if self.render.is_some() {
            self.render.take();
            return Some(Action::Render);
        }
        if self.done {
            return None;
        }
        unreachable!()
    }

    fn new_event(&mut self, action: &Option<Action>) {
        match action {
            Some(Action::Render) => self.render = Some(()),
            Some(Action::ForceRedraw) => self.force_redraw = Some(()),
            Some(Action::Run(cmd)) => self.run.push_back(cmd.to_string()),
            Some(Action::UpdateFocus(focus)) => self.focus = Some(*focus),
            Some(Action::Resize(size)) => self.size = Some(*size),
            Some(Action::Quit) | None => self.done = true,
        }
    }
}

pub fn debounce(
    input: async_std::channel::Receiver<Action>,
) -> async_std::sync::Arc<Debouncer> {
    let debouncer = std::sync::Arc::new(Debouncer {
        pending: async_std::sync::Mutex::new(Pending::new()),
        cvar: async_std::sync::Condvar::new(),
    });
    {
        let debouncer = std::sync::Arc::clone(&debouncer);
        async_std::task::spawn(async move {
            while let Ok(action) = input.recv().await {
                debouncer.send(Some(action)).await;
            }
            debouncer.send(None).await;
        });
    }
    debouncer
}
