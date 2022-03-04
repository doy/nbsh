use crate::shell::prelude::*;

mod clock;
mod git;
pub use git::Info as GitInfo;
mod signals;
mod stdin;

pub struct Handler {
    _clock: clock::Handler,
    git: git::Handler,
    _signals: signals::Handler,
    _stdin: stdin::Handler,
}

impl Handler {
    pub fn new(
        input: textmode::blocking::Input,
        event_w: crate::shell::event::Writer,
    ) -> Result<Self> {
        Ok(Self {
            _clock: clock::Handler::new(event_w.clone()),
            git: git::Handler::new(event_w.clone()),
            _signals: signals::Handler::new(event_w.clone())?,
            _stdin: stdin::Handler::new(input, event_w),
        })
    }

    pub fn new_dir(&self, path: std::path::PathBuf) {
        self.git.new_dir(path);
    }
}
