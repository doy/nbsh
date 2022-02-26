#[derive(Debug)]
pub enum Event {
    Key(textmode::Key),
    Resize((u16, u16)),
    PtyOutput,
    PtyClose,
    ChildRunPipeline(usize, (usize, usize)),
    ChildSuspend(usize),
    GitInfo(Option<super::git::Info>),
    ClockTimer,
}

pub fn channel() -> (Writer, Reader) {
    let (event_w, event_r) = tokio::sync::mpsc::unbounded_channel();
    (Writer::new(event_w), Reader::new(event_r))
}

#[derive(Clone)]
pub struct Writer(tokio::sync::mpsc::UnboundedSender<Event>);

impl Writer {
    pub fn new(event_w: tokio::sync::mpsc::UnboundedSender<Event>) -> Self {
        Self(event_w)
    }

    pub fn send(&self, event: Event) {
        // the only time this should ever error is when the application is
        // shutting down, at which point we don't actually care about any
        // further dropped messages
        #[allow(clippy::let_underscore_drop)]
        let _ = self.0.send(event);
    }
}

pub struct Reader(std::sync::Arc<InnerReader>);

impl Reader {
    pub fn new(input: tokio::sync::mpsc::UnboundedReceiver<Event>) -> Self {
        Self(InnerReader::new(input))
    }

    pub async fn recv(&self) -> Option<Event> {
        self.0.recv().await
    }
}

struct InnerReader {
    pending: tokio::sync::Mutex<Pending>,
    cvar: tokio::sync::Notify,
}

impl InnerReader {
    fn new(
        mut input: tokio::sync::mpsc::UnboundedReceiver<Event>,
    ) -> std::sync::Arc<Self> {
        let this = Self {
            pending: tokio::sync::Mutex::new(Pending::new()),
            cvar: tokio::sync::Notify::new(),
        };
        let this = std::sync::Arc::new(this);
        {
            let this = this.clone();
            tokio::task::spawn(async move {
                while let Some(event) = input.recv().await {
                    this.new_event(Some(event)).await;
                }
                this.new_event(None).await;
            });
        }
        this
    }

    async fn recv(&self) -> Option<Event> {
        loop {
            let mut pending = self.pending.lock().await;
            if pending.has_event() {
                return pending.get_event();
            }
            drop(pending);
            self.cvar.notified().await;
        }
    }

    async fn new_event(&self, event: Option<Event>) {
        let mut pending = self.pending.lock().await;
        pending.new_event(event);
        self.cvar.notify_one();
    }
}

#[allow(clippy::option_option)]
#[derive(Default)]
struct Pending {
    key: std::collections::VecDeque<textmode::Key>,
    size: Option<(u16, u16)>,
    pty_output: bool,
    pty_close: bool,
    child_run_pipeline: std::collections::VecDeque<(usize, (usize, usize))>,
    child_suspend: std::collections::VecDeque<usize>,
    git_info: Option<Option<super::git::Info>>,
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
            || self.git_info.is_some()
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
        if let Some(info) = self.git_info.take() {
            return Some(Event::GitInfo(info));
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
            Some(Event::GitInfo(info)) => self.git_info = Some(info),
            Some(Event::ClockTimer) => self.clock_timer = true,
            None => self.done = true,
        }
    }
}
