use crate::shell::prelude::*;

mod entry;
pub use entry::{Entry, ExitInfo};
mod pty;

pub struct History {
    size: (u16, u16),
    entries: Vec<Entry>,
    scroll_pos: usize,
}

impl History {
    pub fn new() -> Self {
        Self {
            size: (24, 80),
            entries: vec![],
            scroll_pos: 0,
        }
    }

    pub fn render(
        &self,
        out: &mut impl textmode::Textmode,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
        offset: time::UtcOffset,
    ) {
        let mut cursor = None;
        for (idx, used_lines, mut vt) in
            self.visible(repl_lines, focus, scrolling).rev()
        {
            let focused = focus.map_or(false, |focus| idx == focus);
            out.move_to(
                (usize::from(self.size.0) - used_lines).try_into().unwrap(),
                0,
            );
            self.entries[idx].render(
                out,
                self.entry_count(),
                &mut *vt,
                focused,
                scrolling,
                offset,
            );
            if focused && !scrolling {
                cursor = Some((
                    out.screen().cursor_position(),
                    out.screen().hide_cursor(),
                ));
            }
        }
        if let Some((pos, hide)) = cursor {
            out.move_to(pos.0, pos.1);
            out.hide_cursor(hide);
        }
    }

    pub fn entry(&self, idx: usize) -> &Entry {
        &self.entries[idx]
    }

    pub fn entry_mut(&mut self, idx: usize) -> &mut Entry {
        &mut self.entries[idx]
    }

    pub fn resize(&mut self, size: (u16, u16)) {
        self.size = size;
        for entry in &self.entries {
            entry.resize(size);
        }
    }

    pub fn run(
        &mut self,
        cmdline: String,
        env: Env,
        event_w: crate::shell::event::Writer,
    ) {
        self.entries
            .push(Entry::new(cmdline, env, self.size, event_w).unwrap());
    }

    pub fn entry_count(&self) -> usize {
        self.entries.len()
    }

    pub fn make_focus_visible(
        &mut self,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
    ) {
        if self.entries.is_empty() || focus.is_none() {
            return;
        }
        let focus = focus.unwrap();

        let mut done = false;
        while focus
            < self
                .visible(repl_lines, Some(focus), scrolling)
                .map(|(idx, ..)| idx)
                .next()
                .unwrap()
        {
            self.scroll_pos += 1;
            done = true;
        }
        if done {
            return;
        }

        while focus
            > self
                .visible(repl_lines, Some(focus), scrolling)
                .map(|(idx, ..)| idx)
                .last()
                .unwrap()
        {
            self.scroll_pos -= 1;
        }
    }

    fn visible(
        &self,
        repl_lines: usize,
        focus: Option<usize>,
        scrolling: bool,
    ) -> VisibleEntries {
        let mut iter = VisibleEntries::new();
        let mut used_lines = repl_lines;
        for (idx, entry) in
            self.entries.iter().enumerate().rev().skip(self.scroll_pos)
        {
            let focused = focus.map_or(false, |focus| idx == focus);
            used_lines +=
                entry.lines(self.entry_count(), focused && !scrolling);
            if used_lines > usize::from(self.size.0) {
                break;
            }
            iter.add(idx, used_lines, entry.lock_vt());
        }
        iter
    }
}

struct VisibleEntries<'a> {
    entries: std::collections::VecDeque<(
        usize,
        usize,
        std::sync::MutexGuard<'a, pty::Vt>,
    )>,
}

impl<'a> VisibleEntries<'a> {
    fn new() -> Self {
        Self {
            entries: std::collections::VecDeque::new(),
        }
    }

    fn add(
        &mut self,
        idx: usize,
        offset: usize,
        vt: std::sync::MutexGuard<'a, pty::Vt>,
    ) {
        // push_front because we are adding them in reverse order
        self.entries.push_front((idx, offset, vt));
    }
}

impl<'a> std::iter::Iterator for VisibleEntries<'a> {
    type Item = (usize, usize, std::sync::MutexGuard<'a, pty::Vt>);

    fn next(&mut self) -> Option<Self::Item> {
        self.entries.pop_front()
    }
}

impl<'a> std::iter::DoubleEndedIterator for VisibleEntries<'a> {
    fn next_back(&mut self) -> Option<Self::Item> {
        self.entries.pop_back()
    }
}
