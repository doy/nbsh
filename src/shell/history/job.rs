use crate::shell::prelude::*;

pub struct Job {}

impl Job {
    pub fn new(
        cmdline: &str,
        env: Env,
        pts: &pty_process::Pts,
        event_w: crate::shell::event::Writer,
    ) -> Result<Self> {
        let (child, fh) = spawn_command(cmdline, &env, pts)?;
        let state = std::sync::Arc::new(std::sync::Mutex::new(
            State::Running((0, 0)),
        ));
        tokio::spawn(Self::task(
            child,
            fh,
            std::sync::Arc::clone(&state),
            env,
            event_w,
        ));
        Ok(Self {
            state,
            start_time,
            start_instant,
        })
    }

    pub fn start_time(&self) -> &time::OffsetDateTime {
        &self.start_time
    }

    pub fn start_instant(&self) -> &std::time::Instant {
        &self.start_instant
    }

    pub fn with_state<T>(&self, f: impl FnOnce(&State) -> T) -> T {
        let state = self.state.lock().unwrap();
        f(&state)
    }

    pub fn with_state_mut<T>(&self, f: impl FnOnce(&mut State) -> T) -> T {
        let mut state = self.state.lock().unwrap();
        f(&mut state)
    }

    pub fn lock_state(&self) -> std::sync::MutexGuard<State> {
        self.state.lock().unwrap()
    }

    pub fn running(&self) -> bool {
        self.with_state(|state| matches!(state, State::Running(..)))
    }

    pub fn set_span(&self, new_span: (usize, usize)) {
        self.with_state_mut(|state| {
            if let State::Running(span) = state {
                *span = new_span;
            }
        });
    }
}
