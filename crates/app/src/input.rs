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

// --- Protocolo de teclado de kitty ---

/// Banderas del protocolo de kitty (`CSI > flags u`).
pub mod kitty {
    /// Es el comportamiento base en cuanto hay alguna bandera activa.
    #[cfg_attr(not(test), allow(dead_code))]
    pub const DISAMBIGUATE: u8 = 1;
    pub const EVENT_TYPES: u8 = 2;
    pub const ALTERNATE_KEYS: u8 = 4;
    pub const ALL_AS_ESCAPES: u8 = 8;
    pub const ASSOCIATED_TEXT: u8 = 16;
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KeyKind {
    Press,
    Repeat,
    Release,
}

/// Tecla para el protocolo de kitty.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum KittyKey {
    /// Tecla de texto: el carácter sin Shift del layout actual.
    Char(char),
    /// Tecla especial.
    Special(Key),
    /// Tecla modificadora sola (Shift, Ctrl…), con su código de kitty.
    Modifier(u32),
}

/// Modificadores completos para kitty (incluye Cmd y Bloq Mayús).
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct KittyMods {
    pub shift: bool,
    pub alt: bool,
    pub ctrl: bool,
    pub super_: bool,
    pub caps_lock: bool,
}

impl KittyMods {
    fn value(self) -> u32 {
        1 + self.shift as u32
            + 2 * self.alt as u32
            + 4 * self.ctrl as u32
            + 8 * self.super_ as u32
            + 64 * self.caps_lock as u32
    }

    /// Modificadores que convierten una tecla de texto en un atajo.
    fn is_shortcut(self) -> bool {
        self.alt || self.ctrl || self.super_
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct KeyEvent {
    pub key: KittyKey,
    pub mods: KittyMods,
    pub kind: KeyKind,
    /// Carácter con Shift (para las teclas alternativas).
    pub shifted: Option<char>,
    /// Texto que produce la tecla, si produce alguno.
    pub text: Option<String>,
}

/// Códigos de kitty de las teclas modificadoras (izquierda, derecha).
pub mod modifier_codes {
    pub const LEFT_SHIFT: u32 = 57441;
    pub const LEFT_CONTROL: u32 = 57442;
    pub const LEFT_ALT: u32 = 57443;
    pub const LEFT_SUPER: u32 = 57444;
    pub const RIGHT_SHIFT: u32 = 57447;
    pub const RIGHT_CONTROL: u32 = 57448;
    pub const RIGHT_ALT: u32 = 57449;
    pub const RIGHT_SUPER: u32 = 57450;
    pub const CAPS_LOCK: u32 = 57358;
}

/// Codifica una tecla con el protocolo de kitty. `None` significa que no
/// hace falta escape: la tecla se envía como texto o con la codificación
/// clásica, como si el protocolo no estuviera activo.
pub fn encode_kitty(ev: &KeyEvent, flags: u8) -> Option<Vec<u8>> {
    let all = flags & kitty::ALL_AS_ESCAPES != 0;
    let events = flags & kitty::EVENT_TYPES != 0;
    if ev.kind == KeyKind::Release && !events {
        return Some(Vec::new());
    }
    let m = ev.mods.value();
    let e = match ev.kind {
        KeyKind::Press => 1,
        KeyKind::Repeat => 2,
        KeyKind::Release => 3,
    };
    let mods_field = |force: bool| -> String {
        if events && e != 1 {
            format!("{m}:{e}")
        } else if m != 1 || force {
            m.to_string()
        } else {
            String::new()
        }
    };

    let code: u32 = match ev.key {
        KittyKey::Modifier(code) => {
            if !all {
                return Some(Vec::new());
            }
            code
        }
        KittyKey::Char(c) => {
            if !all && !ev.mods.is_shortcut() && ev.kind != KeyKind::Release {
                return None;
            }
            c as u32
        }
        KittyKey::Special(key) => {
            let legacy_ok = !all && ev.kind != KeyKind::Release;
            match key {
                Key::Enter | Key::Tab | Key::Backspace
                    if legacy_ok && (m == 1 || (key == Key::Tab && m == 2)) =>
                {
                    // Sin modificadores siguen siendo \r, \t y DEL (y Shift+Tab, CSI Z).
                    return None;
                }
                Key::Enter => 13,
                Key::Tab => 9,
                Key::Backspace => 127,
                Key::Escape => 27,
                other => {
                    // Teclas con letra o con `~`: la forma clásica con el
                    // campo de modificadores (y tipo de evento) de kitty.
                    let (number, end) = special_form(other);
                    let mf = mods_field(false);
                    let s = match (number, mf.is_empty()) {
                        (1, true) => format!("\x1b[{end}"),
                        (n, true) => format!("\x1b[{n}{end}"),
                        (n, false) => format!("\x1b[{n};{mf}{end}"),
                    };
                    return Some(s.into_bytes());
                }
            }
        }
    };

    let mut first = code.to_string();
    if flags & kitty::ALTERNATE_KEYS != 0
        && ev.mods.shift
        && let Some(shifted) = ev.shifted.filter(|&s| s as u32 != code)
    {
        first.push_str(&format!(":{}", shifted as u32));
    }
    let text = (all && flags & kitty::ASSOCIATED_TEXT != 0 && ev.kind != KeyKind::Release)
        .then(|| ev.text.as_deref())
        .flatten()
        .filter(|t| !t.is_empty() && !t.chars().any(char::is_control))
        .map(|t| {
            t.chars()
                .map(|c| (c as u32).to_string())
                .collect::<Vec<_>>()
                .join(":")
        });
    let s = match text {
        Some(text) => format!("\x1b[{first};{};{text}u", mods_field(false)),
        None => {
            let mf = mods_field(false);
            if mf.is_empty() {
                format!("\x1b[{first}u")
            } else {
                format!("\x1b[{first};{mf}u")
            }
        }
    };
    Some(s.into_bytes())
}

/// Número y final de una tecla especial con la forma de kitty. F3 usa
/// `13~` porque `CSI R` se confunde con la respuesta de posición del cursor.
fn special_form(key: Key) -> (u32, char) {
    match key {
        Key::Up => (1, 'A'),
        Key::Down => (1, 'B'),
        Key::Right => (1, 'C'),
        Key::Left => (1, 'D'),
        Key::Home => (1, 'H'),
        Key::End => (1, 'F'),
        Key::F(1) => (1, 'P'),
        Key::F(2) => (1, 'Q'),
        Key::F(3) => (13, '~'),
        Key::F(4) => (1, 'S'),
        Key::F(n @ 5..=12) => ([15, 17, 18, 19, 20, 21, 23, 24][(n - 5) as usize], '~'),
        Key::Insert => (2, '~'),
        Key::Delete => (3, '~'),
        Key::PageUp => (5, '~'),
        Key::PageDown => (6, '~'),
        _ => (1, 'u'),
    }
}

/// modifyOtherKeys de xterm (`CSI 27 ; mod ; código ~`) para una tecla de
/// texto con Ctrl o Alt. `None` si la forma clásica basta.
pub fn encode_modify_other_keys(c: char, mods: Mods, level: u8) -> Option<Vec<u8>> {
    if level == 0 || !(mods.ctrl || mods.alt) {
        return None;
    }
    // En el nivel 1 solo se codifican las combinaciones sin byte de control propio.
    if level == 1 && mods.ctrl && !mods.alt && ctrl_byte(c).is_some() {
        return None;
    }
    Some(format!("\x1b[27;{};{}~", mods.param(), c as u32).into_bytes())
}

/// Option como Meta: ESC delante de lo que escribe la tecla sin Option.
pub fn encode_meta(text: &str) -> Vec<u8> {
    let mut out = vec![0x1b];
    out.extend_from_slice(text.as_bytes());
    out
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

    fn ev(key: KittyKey, mods: KittyMods) -> KeyEvent {
        KeyEvent {
            key,
            mods,
            kind: KeyKind::Press,
            shifted: None,
            text: None,
        }
    }

    const SHIFT: KittyMods = KittyMods {
        shift: true,
        alt: false,
        ctrl: false,
        super_: false,
        caps_lock: false,
    };

    fn k(e: &KeyEvent, flags: u8) -> Option<String> {
        encode_kitty(e, flags).map(|b| String::from_utf8(b).unwrap())
    }

    #[test]
    fn kitty_disambiguate() {
        let f = kitty::DISAMBIGUATE;
        // Texto sin atajos: se envía como texto.
        assert_eq!(k(&ev(KittyKey::Char('a'), KittyMods::default()), f), None);
        assert_eq!(k(&ev(KittyKey::Char('a'), SHIFT), f), None);
        // Atajos y Escape, con CSI u.
        let ctrl = KittyMods {
            ctrl: true,
            ..Default::default()
        };
        assert_eq!(k(&ev(KittyKey::Char('c'), ctrl), f).unwrap(), "\x1b[99;5u");
        let alt = KittyMods {
            alt: true,
            ..Default::default()
        };
        assert_eq!(k(&ev(KittyKey::Char('a'), alt), f).unwrap(), "\x1b[97;3u");
        assert_eq!(
            k(&ev(KittyKey::Special(Key::Escape), KittyMods::default()), f).unwrap(),
            "\x1b[27u"
        );
        // Enter sin modificadores sigue siendo \r; con Shift, CSI u (Claude Code).
        assert_eq!(
            k(&ev(KittyKey::Special(Key::Enter), KittyMods::default()), f),
            None
        );
        assert_eq!(
            k(&ev(KittyKey::Special(Key::Enter), SHIFT), f).unwrap(),
            "\x1b[13;2u"
        );
        // Flechas y F3.
        assert_eq!(
            k(&ev(KittyKey::Special(Key::Up), KittyMods::default()), f).unwrap(),
            "\x1b[A"
        );
        assert_eq!(
            k(&ev(KittyKey::Special(Key::Up), ctrl), f).unwrap(),
            "\x1b[1;5A"
        );
        assert_eq!(
            k(&ev(KittyKey::Special(Key::F(3)), KittyMods::default()), f).unwrap(),
            "\x1b[13~"
        );
    }

    #[test]
    fn kitty_event_types() {
        let f = kitty::DISAMBIGUATE | kitty::EVENT_TYPES;
        let mut e = ev(KittyKey::Char('a'), KittyMods::default());
        e.kind = KeyKind::Release;
        assert_eq!(k(&e, f).unwrap(), "\x1b[97;1:3u");
        let mut up = ev(KittyKey::Special(Key::Up), KittyMods::default());
        up.kind = KeyKind::Repeat;
        assert_eq!(k(&up, f).unwrap(), "\x1b[1;1:2A");
        // Sin la bandera de eventos, soltar no envía nada.
        assert_eq!(k(&e, kitty::DISAMBIGUATE).unwrap(), "");
    }

    #[test]
    fn kitty_all_keys_alternates_and_text() {
        let f = kitty::DISAMBIGUATE | kitty::ALL_AS_ESCAPES;
        assert_eq!(
            k(&ev(KittyKey::Char('a'), KittyMods::default()), f).unwrap(),
            "\x1b[97u"
        );
        assert_eq!(
            k(&ev(KittyKey::Special(Key::Enter), KittyMods::default()), f).unwrap(),
            "\x1b[13u"
        );
        let mut e = ev(KittyKey::Char('a'), SHIFT);
        e.shifted = Some('A');
        e.text = Some("A".into());
        assert_eq!(k(&e, f | kitty::ALTERNATE_KEYS).unwrap(), "\x1b[97:65;2u");
        assert_eq!(
            k(&e, f | kitty::ALTERNATE_KEYS | kitty::ASSOCIATED_TEXT).unwrap(),
            "\x1b[97:65;2;65u"
        );
        let mut plain = ev(KittyKey::Char('a'), KittyMods::default());
        plain.text = Some("a".into());
        assert_eq!(
            k(&plain, f | kitty::ASSOCIATED_TEXT).unwrap(),
            "\x1b[97;;97u"
        );
        // Las teclas modificadoras solas solo se envían con "todas como escapes".
        let shift_key = ev(KittyKey::Modifier(modifier_codes::LEFT_SHIFT), SHIFT);
        assert_eq!(k(&shift_key, f).unwrap(), "\x1b[57441;2u");
        assert_eq!(k(&shift_key, kitty::DISAMBIGUATE).unwrap(), "");
    }

    #[test]
    fn modify_other_keys_and_meta() {
        let ctrl = Mods { ctrl: true, ..NONE };
        assert_eq!(
            encode_modify_other_keys('a', ctrl, 2).unwrap(),
            b"\x1b[27;5;97~"
        );
        assert_eq!(encode_modify_other_keys('a', ctrl, 1), None);
        assert_eq!(
            encode_modify_other_keys('1', ctrl, 1).unwrap(),
            b"\x1b[27;5;49~"
        );
        assert_eq!(encode_modify_other_keys('a', NONE, 2), None);
        assert_eq!(encode_meta("b"), b"\x1bb");
    }
}
