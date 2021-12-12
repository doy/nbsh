#[derive(Debug)]
pub enum Event {
    Key(textmode::Key),
    Resize((u16, u16)),
    ProcessOutput,
    ProcessAlternateScreen,
    ProcessExit,
    ClockTimer,
    Quit,
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
        pending.new_event(&event);
        self.cvar.notify_one();
    }
}

#[derive(Default)]
struct Pending {
    key: std::collections::VecDeque<textmode::Key>,
    size: Option<(u16, u16)>,
    process_output: bool,
    process_alternate_screen: bool,
    process_exit: bool,
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
            || self.process_output
            || self.process_alternate_screen
            || self.process_exit
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
        if self.process_exit {
            self.process_exit = false;
            return Some(Event::ProcessExit);
        }
        if self.process_alternate_screen {
            self.process_alternate_screen = false;
            return Some(Event::ProcessAlternateScreen);
        }
        if self.clock_timer {
            self.clock_timer = false;
            return Some(Event::ClockTimer);
        }
        // process_output should be last because it will often be the case
        // that there is ~always new process output (cat on large files, yes,
        // etc) and that shouldn't prevent other events from happening
        if self.process_output {
            self.process_output = false;
            return Some(Event::ProcessOutput);
        }
        unreachable!()
    }

    fn new_event(&mut self, event: &Option<Event>) {
        match event {
            Some(Event::Key(key)) => self.key.push_back(key.clone()),
            Some(Event::Resize(size)) => self.size = Some(*size),
            Some(Event::ProcessOutput) => self.process_output = true,
            Some(Event::ProcessAlternateScreen) => {
                self.process_alternate_screen = true;
            }
            Some(Event::ProcessExit) => self.process_exit = true,
            Some(Event::ClockTimer) => self.clock_timer = true,
            Some(Event::Quit) | None => self.done = true,
        }
    }
}