pub struct Vt {
    vt: vt100::Parser,
    audible_bell_state: usize,
    visual_bell_state: usize,
    audible_bell: bool,
    visual_bell: bool,
    real_bell_pending: bool,
}

impl Vt {
    pub fn new(size: (u16, u16)) -> Self {
        Self {
            vt: vt100::Parser::new(size.0, size.1, 0),
            audible_bell_state: 0,
            visual_bell_state: 0,
            audible_bell: false,
            visual_bell: false,
            real_bell_pending: false,
        }
    }

    pub fn process(&mut self, bytes: &[u8]) {
        self.vt.process(bytes);
        let screen = self.vt.screen();

        let new_audible_bell_state = screen.audible_bell_count();
        if new_audible_bell_state != self.audible_bell_state {
            self.audible_bell = true;
            self.real_bell_pending = true;
            self.audible_bell_state = new_audible_bell_state;
        }

        let new_visual_bell_state = screen.visual_bell_count();
        if new_visual_bell_state != self.visual_bell_state {
            self.visual_bell = true;
            self.real_bell_pending = true;
            self.visual_bell_state = new_visual_bell_state;
        }
    }

    pub fn screen(&self) -> &vt100::Screen {
        self.vt.screen()
    }

    pub fn size(&self) -> (u16, u16) {
        self.vt.screen().size()
    }

    pub fn set_size(&mut self, size: (u16, u16)) {
        self.vt.set_size(size.0, size.1);
    }

    pub fn is_bell(&self) -> bool {
        self.audible_bell || self.visual_bell
    }

    pub fn bell(&mut self, out: &mut impl textmode::Textmode, focused: bool) {
        if self.real_bell_pending {
            if self.audible_bell {
                out.write(b"\x07");
            }
            if self.visual_bell {
                out.write(b"\x1bg");
            }
            self.real_bell_pending = false;
        }
        if focused {
            self.audible_bell = false;
            self.visual_bell = false;
        }
    }

    pub fn binary(&self) -> bool {
        self.vt.screen().errors() > 5
    }

    pub fn output_lines(&self, focused: bool, running: bool) -> usize {
        if self.binary() {
            return 1;
        }

        let screen = self.vt.screen();
        let mut last_row = 0;
        for (idx, row) in screen.rows(0, screen.size().1).enumerate() {
            if !row.is_empty() {
                last_row = idx + 1;
            }
        }
        if focused && running {
            last_row = std::cmp::max(
                last_row,
                usize::from(screen.cursor_position().0) + 1,
            );
        }
        last_row
    }
}
