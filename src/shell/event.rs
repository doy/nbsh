#[derive(Debug)]
pub enum Event {
    Key(textmode::Key),
    Resize((u16, u16)),
    PtyOutput,
    PtyClose,
    ChildRunPipeline(usize, (usize, usize)),
    ChildSuspend(usize),
    ClockTimer,
}

pub struct Reader {
    pending: async_std::sync::Mutex<Pending>,
    cvar: async_std::sync::Condvar,
}

impl Reader {
    pub fn new(
        input: async_std::channel::Receiver<Event>,
    ) -> async_std::sync::Arc<Self> {
        let this = async_std::sync::Arc::new(Self {
            pending: async_std::sync::Mutex::new(Pending::new()),
            cvar: async_std::sync::Condvar::new(),
        });
        {
            let this = async_std::sync::Arc::clone(&this);
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
    pty_close: bool,
    child_run_pipeline: std::collections::VecDeque<(usize, (usize, usize))>,
    child_suspend: std::collections::VecDeque<usize>,
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
            || self.pty_close
            || !self.child_run_pipeline.is_empty()
            || !self.child_suspend.is_empty()
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
        if self.pty_close {
            self.pty_close = false;
            return Some(Event::PtyClose);
        }
        if let Some((idx, span)) = self.child_run_pipeline.pop_front() {
            return Some(Event::ChildRunPipeline(idx, span));
        }
        if let Some(idx) = self.child_suspend.pop_front() {
            return Some(Event::ChildSuspend(idx));
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
            Some(Event::PtyClose) => self.pty_close = true,
            Some(Event::ChildRunPipeline(idx, span)) => {
                self.child_run_pipeline.push_back((idx, span));
            }
            Some(Event::ChildSuspend(idx)) => {
                self.child_suspend.push_back(idx);
            }
            Some(Event::ClockTimer) => self.clock_timer = true,
            None => self.done = true,
        }
    }
}
