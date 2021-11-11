#[derive(Debug)]
pub enum Action {
    Render,
    Run(String),
    UpdateFocus(crate::state::Focus),
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
    render: bool,
    run: std::collections::VecDeque<String>,
    focus: Option<crate::state::Focus>,
    done: bool,
}

impl Pending {
    fn new() -> Self {
        Self::default()
    }

    fn has_event(&self) -> bool {
        self.render || !self.run.is_empty() || self.focus.is_some()
    }

    fn get_event(&mut self) -> Option<Action> {
        if !self.run.is_empty() {
            return Some(Action::Run(self.run.pop_front().unwrap()));
        }
        if self.focus.is_some() {
            return Some(Action::UpdateFocus(self.focus.take().unwrap()));
        }
        if self.render {
            self.render = false;
            return Some(Action::Render);
        }
        if self.done {
            return None;
        }
        unreachable!()
    }

    fn new_event(&mut self, action: &Option<Action>) {
        match action {
            Some(Action::Run(cmd)) => self.run.push_back(cmd.to_string()),
            Some(Action::UpdateFocus(focus)) => self.focus = Some(*focus),
            Some(Action::Render) => self.render = true,
            None => self.done = true,
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
