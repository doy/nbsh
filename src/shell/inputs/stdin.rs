use crate::shell::prelude::*;

pub struct Handler;

impl Handler {
    pub fn new(
        mut input: textmode::blocking::Input,
        event_w: crate::shell::event::Writer,
    ) -> Self {
        std::thread::spawn(move || {
            while let Some(key) = input.read_key().unwrap() {
                event_w.send(Event::Key(key));
            }
        });
        Self
    }
}
