use std::num::NonZeroU16;

const SCROLLBACK_LINES: usize = 2000;

/// Terminal dimensions in character cells; both are at least one.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TerminalSize {
    cols: NonZeroU16,
    rows: NonZeroU16,
}
impl TerminalSize {
    pub fn new(cols: u16, rows: u16) -> Option<Self> {
        Some(Self {
            cols: NonZeroU16::new(cols)?,
            rows: NonZeroU16::new(rows)?,
        })
    }
    pub fn cols(self) -> u16 {
        self.cols.get()
    }
    pub fn rows(self) -> u16 {
        self.rows.get()
    }
}

/// Terminal emulation: turns the byte stream of the remote program into a cell grid.
pub struct Screen {
    parser: vt100::Parser,
}
impl Screen {
    pub fn new(size: TerminalSize) -> Self {
        Self {
            parser: vt100::Parser::new(size.rows, size.cols, SCROLLBACK_LINES),
        }
    }
    pub fn process(&mut self, bytes: &[u8]) {
        self.parser.process(bytes);
    }
    pub fn resize(&mut self, size: TerminalSize) {
        self.parser.screen_mut().set_size(size.rows, size.cols);
    }
    /// Starts from an empty screen again, e.g. before `tmux` redraws after a reconnect.
    pub fn reset(&mut self, size: TerminalSize) {
        *self = Self::new(size);
    }
    pub fn view(&self) -> &vt100::Screen {
        self.parser.screen()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn size(cols: u16, rows: u16) -> TerminalSize {
        TerminalSize::new(cols, rows).unwrap()
    }

    #[test]
    fn size_must_not_be_empty() {
        assert!(TerminalSize::new(0, 24).is_none());
        assert!(TerminalSize::new(80, 0).is_none());
    }
    #[test]
    fn renders_text_and_control_sequences() {
        let mut screen = Screen::new(size(20, 4));
        screen.process(b"hello\r\n\x1b[31mred\x1b[0m");
        assert_eq!(screen.view().contents(), "hello\nred");
        assert_eq!(screen.view().cursor_position(), (1, 3));
    }
    #[test]
    fn reset_clears_the_screen() {
        let mut screen = Screen::new(size(20, 4));
        screen.process(b"old");
        screen.reset(size(10, 2));
        assert_eq!(screen.view().contents(), "");
        assert_eq!(screen.view().size().0.get(), 2);
        assert_eq!(screen.view().size().1.get(), 10);
    }
    #[test]
    fn resize_changes_the_grid() {
        let mut screen = Screen::new(size(20, 4));
        screen.resize(size(40, 10));
        assert_eq!(screen.view().size().0.get(), 10);
        assert_eq!(screen.view().size().1.get(), 40);
    }
}
