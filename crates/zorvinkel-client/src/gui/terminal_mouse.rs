//! Mouse support of the terminal: reports to programs that ask for it, and the local selection.
use zorvinkel_terminal::vt100::{MouseProtocolEncoding, MouseProtocolMode};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum MouseEvent {
    /// Only the left button is reported.
    Press,
    Release,
    /// Moved with the button held down.
    Drag,
    WheelUp,
    WheelDown,
}

/// The escape sequence that reports `event` at a zero-based cell to a program, or nothing when
/// the program did not ask for this kind of event.
pub fn report(
    mode: MouseProtocolMode,
    encoding: MouseProtocolEncoding,
    event: MouseEvent,
    row: u16,
    column: u16,
) -> Vec<u8> {
    let wanted = match event {
        MouseEvent::Press | MouseEvent::WheelUp | MouseEvent::WheelDown => {
            mode != MouseProtocolMode::None
        }
        MouseEvent::Release => matches!(
            mode,
            MouseProtocolMode::PressRelease
                | MouseProtocolMode::ButtonMotion
                | MouseProtocolMode::AnyMotion
        ),
        MouseEvent::Drag => matches!(
            mode,
            MouseProtocolMode::ButtonMotion | MouseProtocolMode::AnyMotion
        ),
    };
    if !wanted {
        return Vec::new();
    }
    let (code, release) = match event {
        MouseEvent::Press => (0_u8, false),
        MouseEvent::Release => (0_u8, true),
        MouseEvent::Drag => (32_u8, false),
        MouseEvent::WheelUp => (64_u8, false),
        MouseEvent::WheelDown => (65_u8, false),
    };
    let (x, y) = (u32::from(column) + 1, u32::from(row) + 1);
    match encoding {
        MouseProtocolEncoding::Sgr => {
            let end = if release { 'm' } else { 'M' };
            format!("\x1b[<{code};{x};{y}{end}").into_bytes()
        }
        MouseProtocolEncoding::Default | MouseProtocolEncoding::Utf8 => {
            // Without SGR a release does not say which button was released.
            let code = if release { 3 } else { code };
            let mut bytes = b"\x1b[M".to_vec();
            for value in [u32::from(code) + 32, x + 32, y + 32] {
                if encoding == MouseProtocolEncoding::Utf8 {
                    let mut buffer = [0; 4];
                    let character = char::from_u32(value).unwrap_or(' ');
                    bytes.extend(character.encode_utf8(&mut buffer).as_bytes());
                } else if let Ok(byte) = u8::try_from(value) {
                    bytes.push(byte);
                } else {
                    // Outside the range of this encoding: better no report than a wrong one.
                    return Vec::new();
                }
            }
            bytes
        }
    }
}

/// A text selection between two cells, kept in screen coordinates `(row, column)`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    anchor: (u16, u16),
    head: (u16, u16),
}
impl Selection {
    pub fn start(cell: (u16, u16)) -> Self {
        Self {
            anchor: cell,
            head: cell,
        }
    }
    pub fn extend_to(&mut self, cell: (u16, u16)) {
        self.head = cell;
    }
    /// A click without dragging selects nothing.
    pub fn is_empty(&self) -> bool {
        self.anchor == self.head
    }
    /// First and last selected cell in reading order, both included.
    pub fn bounds(&self) -> ((u16, u16), (u16, u16)) {
        if self.anchor <= self.head {
            (self.anchor, self.head)
        } else {
            (self.head, self.anchor)
        }
    }
    pub fn contains(&self, row: u16, column: u16) -> bool {
        let (first, last) = self.bounds();
        (row, column) >= first && (row, column) <= last
    }
    /// The arguments of `contents_between`: its end column is exclusive.
    pub fn text_range(&self, columns: u16) -> (u16, u16, u16, u16) {
        let ((first_row, first_column), (last_row, last_column)) = self.bounds();
        (
            first_row,
            first_column,
            last_row,
            (last_column + 1).min(columns),
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use MouseProtocolEncoding::{Default as X10, Sgr, Utf8};
    use MouseProtocolMode::{AnyMotion, ButtonMotion, None as Off, Press, PressRelease};

    #[test]
    fn nothing_is_reported_while_the_program_did_not_ask() {
        let wheel = MouseEvent::WheelUp;
        assert!(report(Off, Sgr, wheel, 0, 0).is_empty());
        assert!(report(Press, Sgr, MouseEvent::Release, 0, 0).is_empty());
        assert!(report(PressRelease, Sgr, MouseEvent::Drag, 0, 0).is_empty());
    }
    #[test]
    fn sgr_reports_one_based_cells_and_release_as_lowercase_m() {
        let press = report(PressRelease, Sgr, MouseEvent::Press, 4, 9);
        assert_eq!(press, b"\x1b[<0;10;5M");
        let release = report(PressRelease, Sgr, MouseEvent::Release, 4, 9);
        assert_eq!(release, b"\x1b[<0;10;5m");
        assert_eq!(
            report(ButtonMotion, Sgr, MouseEvent::Drag, 0, 0),
            b"\x1b[<32;1;1M"
        );
        assert_eq!(
            report(AnyMotion, Sgr, MouseEvent::WheelDown, 0, 0),
            b"\x1b[<65;1;1M"
        );
    }
    #[test]
    fn the_default_encoding_adds_32_and_reports_release_without_a_button() {
        assert_eq!(
            report(PressRelease, X10, MouseEvent::Press, 0, 0),
            [0x1b, b'[', b'M', 32, 33, 33]
        );
        assert_eq!(
            report(PressRelease, X10, MouseEvent::Release, 0, 0),
            [0x1b, b'[', b'M', 32 + 3, 33, 33]
        );
    }
    #[test]
    fn the_default_encoding_stops_at_cell_223_but_utf8_goes_on() {
        assert!(report(Press, X10, MouseEvent::Press, 0, 300).is_empty());
        assert!(!report(Press, Utf8, MouseEvent::Press, 0, 300).is_empty());
    }

    #[test]
    fn selection_covers_the_reading_order_whichever_way_it_was_dragged() {
        let mut selection = Selection::start((2, 5));
        assert!(selection.is_empty());
        selection.extend_to((1, 3));
        assert_eq!(selection.bounds(), ((1, 3), (2, 5)));
        assert!(selection.contains(1, 3) && selection.contains(2, 5));
        assert!(selection.contains(1, 70), "the rest of the first row");
        assert!(selection.contains(2, 0), "the start of the last row");
        assert!(!selection.contains(1, 2) && !selection.contains(2, 6));
    }
    #[test]
    fn text_range_includes_the_last_cell_and_stays_inside_the_row() {
        let mut selection = Selection::start((0, 2));
        selection.extend_to((1, 79));
        assert_eq!(selection.text_range(80), (0, 2, 1, 80));
        selection.extend_to((0, 6));
        assert_eq!(selection.text_range(80), (0, 2, 0, 7));
    }
}
