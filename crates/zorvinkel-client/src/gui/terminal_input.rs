//! Translates egui keyboard events into the bytes a terminal program expects.
use eframe::egui::{Event, Key, Modifiers};

/// Bytes for the events of one frame. Text typed with the keyboard arrives as `Event::Text`;
/// keys without text and Ctrl combinations arrive as `Event::Key`.
pub fn bytes_for_events(events: &[Event], application_cursor: bool) -> Vec<u8> {
    let mut bytes = Vec::new();
    for event in events {
        match event {
            Event::Text(text) | Event::Paste(text) => {
                bytes.extend(text.replace('\n', "\r").bytes())
            }
            Event::Key {
                key,
                pressed: true,
                modifiers,
                ..
            } => bytes.extend(key_bytes(*key, *modifiers, application_cursor)),
            _ => {}
        }
    }
    bytes
}

fn key_bytes(key: Key, modifiers: Modifiers, application_cursor: bool) -> Vec<u8> {
    if modifiers.ctrl && !modifiers.alt {
        return control_byte(key).map(|byte| vec![byte]).unwrap_or_default();
    }
    let cursor = |letter: u8| {
        let introducer = if application_cursor { b'O' } else { b'[' };
        vec![0x1b, introducer, letter]
    };
    match key {
        Key::Enter => b"\r".to_vec(),
        Key::Tab if modifiers.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backspace => vec![0x7f],
        Key::Escape => vec![0x1b],
        Key::ArrowUp => cursor(b'A'),
        Key::ArrowDown => cursor(b'B'),
        Key::ArrowRight => cursor(b'C'),
        Key::ArrowLeft => cursor(b'D'),
        Key::Home => cursor(b'H'),
        Key::End => cursor(b'F'),
        Key::Insert => b"\x1b[2~".to_vec(),
        Key::Delete => b"\x1b[3~".to_vec(),
        Key::PageUp => b"\x1b[5~".to_vec(),
        Key::PageDown => b"\x1b[6~".to_vec(),
        Key::F1 => b"\x1bOP".to_vec(),
        Key::F2 => b"\x1bOQ".to_vec(),
        Key::F3 => b"\x1bOR".to_vec(),
        Key::F4 => b"\x1bOS".to_vec(),
        Key::F5 => b"\x1b[15~".to_vec(),
        Key::F6 => b"\x1b[17~".to_vec(),
        Key::F7 => b"\x1b[18~".to_vec(),
        Key::F8 => b"\x1b[19~".to_vec(),
        Key::F9 => b"\x1b[20~".to_vec(),
        Key::F10 => b"\x1b[21~".to_vec(),
        Key::F11 => b"\x1b[23~".to_vec(),
        Key::F12 => b"\x1b[24~".to_vec(),
        _ => Vec::new(),
    }
}

/// Ctrl+A to Ctrl+Z as control characters. Ctrl+V is left to the paste event.
fn control_byte(key: Key) -> Option<u8> {
    let name = key.name();
    let letter = name.chars().next().filter(|_| name.len() == 1)?;
    (letter.is_ascii_uppercase() && key != Key::V).then(|| letter as u8 - b'A' + 1)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn key(key: Key, modifiers: Modifiers) -> Event {
        Event::Key {
            key,
            physical_key: None,
            pressed: true,
            repeat: false,
            modifiers,
        }
    }
    fn plain(key_pressed: Key) -> Vec<u8> {
        bytes_for_events(&[key(key_pressed, Modifiers::NONE)], false)
    }

    #[test]
    fn typed_text_is_sent_as_typed() {
        let events = [Event::Text("ls -l".into())];
        assert_eq!(bytes_for_events(&events, false), b"ls -l");
    }
    #[test]
    fn pasted_lines_end_with_carriage_returns() {
        let events = [Event::Paste("a\nb".into())];
        assert_eq!(bytes_for_events(&events, false), b"a\rb");
    }
    #[test]
    fn editing_keys_use_the_usual_sequences() {
        assert_eq!(plain(Key::Enter), b"\r");
        assert_eq!(plain(Key::Backspace), [0x7f]);
        assert_eq!(plain(Key::Delete), b"\x1b[3~");
        assert_eq!(plain(Key::PageUp), b"\x1b[5~");
        assert_eq!(plain(Key::F5), b"\x1b[15~");
    }
    #[test]
    fn arrows_follow_the_application_cursor_mode() {
        let up = [key(Key::ArrowUp, Modifiers::NONE)];
        assert_eq!(bytes_for_events(&up, false), b"\x1b[A");
        assert_eq!(bytes_for_events(&up, true), b"\x1bOA");
    }
    #[test]
    fn control_letters_become_control_characters() {
        let ctrl = Modifiers {
            ctrl: true,
            ..Modifiers::NONE
        };
        assert_eq!(bytes_for_events(&[key(Key::C, ctrl)], false), [3]);
        assert_eq!(bytes_for_events(&[key(Key::D, ctrl)], false), [4]);
        assert_eq!(bytes_for_events(&[key(Key::V, ctrl)], false), b"");
    }
    #[test]
    fn released_keys_and_unknown_keys_send_nothing() {
        let released = Event::Key {
            key: Key::Enter,
            physical_key: None,
            pressed: false,
            repeat: false,
            modifiers: Modifiers::NONE,
        };
        assert!(bytes_for_events(&[released], false).is_empty());
        assert!(plain(Key::A).is_empty(), "letters arrive as text events");
    }
}
