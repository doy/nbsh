#[derive(Debug, serde::Serialize, serde::Deserialize)]
pub enum Event {
    #[serde(
        serialize_with = "serialize_key",
        deserialize_with = "deserialize_key"
    )]
    Key(textmode::Key),
    Resize((u16, u16)),
    PtyOutput,
    PtyClose(crate::env::Env),
    ChildSuspend(usize),
    PipelineExit(crate::env::Env),
    ClockTimer,
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::needless_pass_by_value)]
fn serialize_key<S>(_key: &textmode::Key, _s: S) -> Result<S::Ok, S::Error>
where
    S: serde::Serializer,
{
    todo!()
}

#[allow(clippy::trivially_copy_pass_by_ref, clippy::needless_pass_by_value)]
fn deserialize_key<'de, D>(_d: D) -> Result<textmode::Key, D::Error>
where
    D: serde::Deserializer<'de>,
{
    todo!()
}

pub struct Reader {
    pending: async_std::sync::Mutex<Pending>,
    cvar: async_std::sync::Condvar,
}

impl Reader {
    pub fn new(
        input: async_std::channel::Receiver<Event>,
    ) -> async_std::sync::Arc<Self> {
        let this = std::sync::Arc::new(Self {
            pending: async_std::sync::Mutex::new(Pending::new()),
            cvar: async_std::sync::Condvar::new(),
        });
        {
            let this = std::sync::Arc::clone(&this);
            async_std::task::spawn(async move {
                while let Ok(event) = input.recv().await {
                    this.new_event(Some(event)).await;
                }
                this.new_event(None).await;
            });
        }
        this
    }

    pub async fn recv(&self) -> Option<Event> {
        let mut pending = self
            .cvar
            .wait_until(self.pending.lock().await, |pending| {
                pending.has_event()
            })
            .await;
        pending.get_event()
    }

    async fn new_event(&self, event: Option<Event>) {
        let mut pending = self.pending.lock().await;
        pending.new_event(event);
        self.cvar.notify_one();
    }
}

#[derive(Default)]
struct Pending {
    key: std::collections::VecDeque<textmode::Key>,
    size: Option<(u16, u16)>,
    pty_output: bool,
    pty_close: std::collections::VecDeque<crate::env::Env>,
    child_suspend: std::collections::VecDeque<usize>,
    pipeline_exit: std::collections::VecDeque<crate::env::Env>,
    clock_timer: bool,
    done: bool,
}

impl Pending {
    fn new() -> Self {
        Self::default()
    }

    fn has_event(&self) -> bool {
        self.done
            || !self.key.is_empty()
            || self.size.is_some()
            || self.pty_output
            || !self.pty_close.is_empty()
            || !self.child_suspend.is_empty()
            || !self.pipeline_exit.is_empty()
            || self.clock_timer
    }

    fn get_event(&mut self) -> Option<Event> {
        if self.done {
            return None;
        }
        if let Some(key) = self.key.pop_front() {
            return Some(Event::Key(key));
        }
        if let Some(size) = self.size.take() {
            return Some(Event::Resize(size));
        }
        if let Some(env) = self.pty_close.pop_front() {
            return Some(Event::PtyClose(env));
        }
        if let Some(idx) = self.child_suspend.pop_front() {
            return Some(Event::ChildSuspend(idx));
        }
        if let Some(env) = self.pipeline_exit.pop_front() {
            return Some(Event::PipelineExit(env));
        }
        if self.clock_timer {
            self.clock_timer = false;
            return Some(Event::ClockTimer);
        }
        // process_output should be last because it will often be the case
        // that there is ~always new process output (cat on large files, yes,
        // etc) and that shouldn't prevent other events from happening
        if self.pty_output {
            self.pty_output = false;
            return Some(Event::PtyOutput);
        }
        unreachable!()
    }

    fn new_event(&mut self, event: Option<Event>) {
        match event {
            Some(Event::Key(key)) => self.key.push_back(key),
            Some(Event::Resize(size)) => self.size = Some(size),
            Some(Event::PtyOutput) => self.pty_output = true,
            Some(Event::PtyClose(env)) => self.pty_close.push_back(env),
            Some(Event::ChildSuspend(idx)) => {
                self.child_suspend.push_back(idx);
            }
            Some(Event::PipelineExit(env)) => {
                self.pipeline_exit.push_back(env);
            }
            Some(Event::ClockTimer) => self.clock_timer = true,
            None => self.done = true,
        }
    }
}
