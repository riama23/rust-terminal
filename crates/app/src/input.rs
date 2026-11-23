//! Codificación de teclas a bytes para la PTY (codificación clásica de xterm).
//!
//! No depende de AppKit: la vista traduce `NSEvent` a [`Key`] y [`Mods`].

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Key {
    Up,
    Down,
    Right,
    Left,
    Home,
    End,
    PageUp,
    PageDown,
    Insert,
    Delete,
    /// F1–F12.
    F(u8),
    Enter,
    Tab,
    Backspace,
    Escape,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Mods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
}

impl Mods {
    /// Parámetro de modificadores de xterm: 1 + shift + 2·alt + 4·ctrl.
    fn param(self) -> u8 {
        1 + self.shift as u8 + 2 * self.alt as u8 + 4 * self.ctrl as u8
    }
}

/// Traduce un `keyCode` de macOS (posición física, independiente del layout)
/// a una tecla especial.
pub fn key_from_keycode(code: u16) -> Option<Key> {
    Some(match code {
        36 | 76 => Key::Enter,
        48 => Key::Tab,
        51 => Key::Backspace,
        53 => Key::Escape,
        114 => Key::Insert,
        117 => Key::Delete,
        115 => Key::Home,
        119 => Key::End,
        116 => Key::PageUp,
        121 => Key::PageDown,
        123 => Key::Left,
        124 => Key::Right,
        125 => Key::Down,
        126 => Key::Up,
        122 => Key::F(1),
        120 => Key::F(2),
        99 => Key::F(3),
        118 => Key::F(4),
        96 => Key::F(5),
        97 => Key::F(6),
        98 => Key::F(7),
        100 => Key::F(8),
        101 => Key::F(9),
        109 => Key::F(10),
        103 => Key::F(11),
        111 => Key::F(12),
        _ => return None,
    })
}

pub fn encode_key(key: Key, mods: Mods, app_cursor: bool) -> Vec<u8> {
    let m = mods.param();
    // Teclas con letra final: flechas, Home y End (SS3 en modo DECCKM).
    let letter = |c: char| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[1;{m}{c}").into_bytes()
        } else if app_cursor {
            format!("\x1bO{c}").into_bytes()
        } else {
            format!("\x1b[{c}").into_bytes()
        }
    };
    // Teclas con número y `~`.
    let tilde = |n: u8| -> Vec<u8> {
        if m > 1 {
            format!("\x1b[{n};{m}~").into_bytes()
        } else {
            format!("\x1b[{n}~").into_bytes()
        }
    };
    match key {
        Key::Up => letter('A'),
        Key::Down => letter('B'),
        Key::Right => letter('C'),
        Key::Left => letter('D'),
        Key::Home => letter('H'),
        Key::End => letter('F'),
        Key::Insert => tilde(2),
        Key::Delete => tilde(3),
        Key::PageUp => tilde(5),
        Key::PageDown => tilde(6),
        Key::F(n @ 1..=4) => {
            let c = (b'P' + n - 1) as char;
            if m > 1 {
                format!("\x1b[1;{m}{c}").into_bytes()
            } else {
                format!("\x1bO{c}").into_bytes()
            }
        }
        Key::F(n @ 5..=12) => tilde([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize]),
        Key::F(_) => Vec::new(),
        Key::Enter => b"\r".to_vec(),
        Key::Tab if mods.shift => b"\x1b[Z".to_vec(),
        Key::Tab => b"\t".to_vec(),
        Key::Backspace if mods.ctrl => vec![0x08],
        Key::Backspace => vec![0x7f],
        Key::Escape => vec![0x1b],
    }
}

/// Byte de control para Ctrl+`c`, con las equivalencias habituales de xterm.
pub fn ctrl_byte(c: char) -> Option<u8> {
    Some(match c.to_ascii_lowercase() {
        c @ 'a'..='z' => c as u8 - b'a' + 1,
        '@' | ' ' | '2' => 0x00,
        '[' | '3' => 0x1b,
        '\\' | '4' => 0x1c,
        ']' | '5' => 0x1d,
        '^' | '6' => 0x1e,
        '_' | '-' | '/' | '7' => 0x1f,
        '?' | '8' => 0x7f,
        _ => return None,
    })
}

/// Prepara texto pegado: saltos de línea como CR y, con bracketed paste,
/// envuelto en `ESC[200~ … ESC[201~` sin marcadores de fin dentro.
pub fn encode_paste(text: &str, bracketed: bool) -> Vec<u8> {
    let text = text.replace("\r\n", "\r").replace('\n', "\r");
    if bracketed {
        let text = text.replace("\x1b[201~", "");
        format!("\x1b[200~{text}\x1b[201~").into_bytes()
    } else {
        text.into_bytes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const NONE: Mods = Mods {
        shift: false,
        alt: false,
        ctrl: false,
    };

    #[test]
    fn arrows_follow_decckm() {
        assert_eq!(encode_key(Key::Up, NONE, false), b"\x1b[A");
        assert_eq!(encode_key(Key::Up, NONE, true), b"\x1bOA");
        assert_eq!(encode_key(Key::End, NONE, true), b"\x1bOF");
    }

    #[test]
    fn modifiers_use_csi_1_mod() {
        let ctrl = Mods { ctrl: true, ..NONE };
        let shift_alt = Mods {
            shift: true,
            alt: true,
            ..NONE
        };
        assert_eq!(encode_key(Key::Right, ctrl, true), b"\x1b[1;5C");
        assert_eq!(encode_key(Key::Left, shift_alt, false), b"\x1b[1;4D");
        assert_eq!(encode_key(Key::Delete, ctrl, false), b"\x1b[3;5~");
        assert_eq!(encode_key(Key::F(1), ctrl, false), b"\x1b[1;5P");
    }

    #[test]
    fn function_and_editing_keys() {
        assert_eq!(encode_key(Key::F(1), NONE, false), b"\x1bOP");
        assert_eq!(encode_key(Key::F(4), NONE, false), b"\x1bOS");
        assert_eq!(encode_key(Key::F(5), NONE, false), b"\x1b[15~");
        assert_eq!(encode_key(Key::F(12), NONE, false), b"\x1b[24~");
        assert_eq!(encode_key(Key::PageUp, NONE, false), b"\x1b[5~");
        assert_eq!(encode_key(Key::Backspace, NONE, false), b"\x7f");
        assert_eq!(
            encode_key(
                Key::Tab,
                Mods {
                    shift: true,
                    ..NONE
                },
                false
            ),
            b"\x1b[Z"
        );
    }

    #[test]
    fn ctrl_bytes() {
        assert_eq!(ctrl_byte('c'), Some(0x03));
        assert_eq!(ctrl_byte('R'), Some(0x12));
        assert_eq!(ctrl_byte('['), Some(0x1b));
        assert_eq!(ctrl_byte(' '), Some(0x00));
        assert_eq!(ctrl_byte('ñ'), None);
    }

    #[test]
    fn paste_encoding() {
        assert_eq!(encode_paste("a\nb\r\nc", false), b"a\rb\rc");
        assert_eq!(
            encode_paste("x\x1b[201~y", true),
            b"\x1b[200~xy\x1b[201~".to_vec()
        );
    }
}
