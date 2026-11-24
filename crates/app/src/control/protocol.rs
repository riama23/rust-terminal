//! El protocolo de la API de control: una petición JSON por línea y una
//! respuesta JSON por línea. Puro y con tests.
//!
//! ```text
//! → {"v":0,"id":1,"token":"…","client":"claude","method":"sessions.list","params":{}}
//! ← {"v":0,"id":1,"ok":true,"result":{…}}
//! ← {"v":0,"id":1,"ok":false,"error":{"code":"denied","message":"…"}}
//! ```

use serde_json::{Value, json};

/// Versión del protocolo. `0` = experimental: puede cambiar sin aviso.
pub const VERSION: u64 = 0;
/// Tamaño máximo de una petición (una línea).
pub const MAX_LINE: usize = 1024 * 1024;
/// Texto máximo de `input.send_text`.
pub const MAX_TEXT: usize = 64 * 1024;
/// Teclas máximas de `input.send_keys`.
pub const MAX_KEYS: usize = 256;
/// Líneas máximas de `screen.read`.
pub const MAX_LINES: usize = 20_000;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Method {
    Hello,
    SessionsList,
    ScreenRead,
    BlocksList,
    BlocksOutput,
    InputSendText,
    InputSendKeys,
    SessionsOpen,
    SessionsClose,
}

/// A qué grupo de permisos pertenece un método que escribe.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Group {
    /// Teclear en una sesión.
    Input,
    /// Abrir o cerrar sesiones.
    Sessions,
}

impl Method {
    pub fn parse(name: &str) -> Option<Method> {
        Some(match name {
            "hello" => Method::Hello,
            "sessions.list" => Method::SessionsList,
            "screen.read" => Method::ScreenRead,
            "blocks.list" => Method::BlocksList,
            "blocks.output" => Method::BlocksOutput,
            "input.send_text" => Method::InputSendText,
            "input.send_keys" => Method::InputSendKeys,
            "sessions.open" => Method::SessionsOpen,
            "sessions.close" => Method::SessionsClose,
            _ => return None,
        })
    }

    #[cfg_attr(not(test), allow(dead_code))]
    pub fn name(self) -> &'static str {
        match self {
            Method::Hello => "hello",
            Method::SessionsList => "sessions.list",
            Method::ScreenRead => "screen.read",
            Method::BlocksList => "blocks.list",
            Method::BlocksOutput => "blocks.output",
            Method::InputSendText => "input.send_text",
            Method::InputSendKeys => "input.send_keys",
            Method::SessionsOpen => "sessions.open",
            Method::SessionsClose => "sessions.close",
        }
    }

    /// El grupo de permisos si el método **escribe**; `None` si solo lee.
    pub fn write_group(self) -> Option<Group> {
        match self {
            Method::InputSendText | Method::InputSendKeys => Some(Group::Input),
            Method::SessionsOpen | Method::SessionsClose => Some(Group::Sessions),
            _ => None,
        }
    }
}

#[derive(Debug, Clone)]
pub struct Request {
    pub id: Value,
    pub client: String,
    pub method: Method,
    pub params: Value,
}

/// Códigos de error estables.
pub mod code {
    pub const BAD_REQUEST: &str = "bad_request";
    pub const UNAUTHORIZED: &str = "unauthorized";
    pub const UNKNOWN_METHOD: &str = "unknown_method";
    pub const NOT_FOUND: &str = "not_found";
    pub const DENIED: &str = "denied";
    pub const TOO_LARGE: &str = "too_large";
    pub const TIMEOUT: &str = "timeout";
    pub const BUSY: &str = "busy";
}

pub fn ok(id: &Value, result: Value) -> String {
    let mut s = json!({"v": VERSION, "id": id, "ok": true, "result": result}).to_string();
    s.push('\n');
    s
}

pub fn err(id: &Value, code: &str, message: &str) -> String {
    let mut s = json!({
        "v": VERSION, "id": id, "ok": false,
        "error": {"code": code, "message": message}
    })
    .to_string();
    s.push('\n');
    s
}

/// Comparación sin atajos por la longitud del prefijo común (el token no
/// debe poder adivinarse midiendo tiempos).
fn same_token(a: &str, b: &str) -> bool {
    let (a, b) = (a.as_bytes(), b.as_bytes());
    let mut diff = a.len() ^ b.len();
    for i in 0..a.len().max(b.len()) {
        diff |= usize::from(a.get(i).copied().unwrap_or(0) ^ b.get(i).copied().unwrap_or(0));
    }
    diff == 0
}

/// Interpreta una línea. `Err` es la respuesta ya lista para devolver.
pub fn parse_request(line: &str, token: &str) -> Result<Request, String> {
    let v: Value = serde_json::from_str(line).map_err(|e| {
        err(
            &Value::Null,
            code::BAD_REQUEST,
            &format!("JSON no válido: {e}"),
        )
    })?;
    let id = v.get("id").cloned().unwrap_or(Value::Null);
    // Sin ser un objeto no hay nada más que mirar.
    let Some(obj) = v.as_object() else {
        return Err(err(
            &id,
            code::BAD_REQUEST,
            "la petición debe ser un objeto JSON",
        ));
    };
    // El token se comprueba antes que nada: sin él, ni siquiera se explica qué falta.
    let supplied = obj.get("token").and_then(Value::as_str).unwrap_or("");
    if !same_token(supplied, token) {
        return Err(err(&id, code::UNAUTHORIZED, "token no válido"));
    }
    if obj.get("v").and_then(Value::as_u64) != Some(VERSION) {
        return Err(err(
            &id,
            code::BAD_REQUEST,
            &format!("versión del protocolo no soportada (esta app habla la {VERSION})"),
        ));
    }
    let Some(name) = obj.get("method").and_then(Value::as_str) else {
        return Err(err(&id, code::BAD_REQUEST, "falta `method`"));
    };
    let Some(method) = Method::parse(name) else {
        return Err(err(
            &id,
            code::UNKNOWN_METHOD,
            &format!("método desconocido `{name}`"),
        ));
    };
    let client: String = obj
        .get("client")
        .and_then(Value::as_str)
        .unwrap_or("desconocido")
        .chars()
        .filter(|c| !c.is_control())
        .take(60)
        .collect();
    Ok(Request {
        id,
        client: if client.trim().is_empty() {
            "desconocido".into()
        } else {
            client
        },
        method,
        params: obj.get("params").cloned().unwrap_or(Value::Null),
    })
}

/// El texto de `input.send_text` como bytes de PTY: letras, tabulador y
/// saltos de línea (que pasan a Intro). Cualquier otro carácter de control
/// se rechaza (para eso están las teclas).
pub fn text_to_bytes(text: &str, enter: bool) -> Result<Vec<u8>, String> {
    if text.len() > MAX_TEXT {
        return Err(format!("el texto pasa de {MAX_TEXT} bytes"));
    }
    let mut out = Vec::with_capacity(text.len() + 1);
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        match c {
            '\r' => {
                // `\r\n` cuenta como un solo salto.
                if chars.peek() == Some(&'\n') {
                    chars.next();
                }
                out.push(b'\r');
            }
            '\n' => out.push(b'\r'),
            '\t' => out.push(b'\t'),
            c if c.is_control() => {
                return Err(format!(
                    "el texto tiene un carácter de control (U+{:04X}); usa `input.send_keys`",
                    c as u32
                ));
            }
            c => {
                let mut buf = [0u8; 4];
                out.extend_from_slice(c.encode_utf8(&mut buf).as_bytes());
            }
        }
    }
    if enter {
        out.push(b'\r');
    }
    Ok(out)
}

/// Una tecla con nombre (`ctrl+c`, `enter`, `up`, `alt+x`, `f5`, `a`) a bytes de PTY.
pub fn key_to_bytes(key: &str) -> Result<Vec<u8>, String> {
    let k = key.trim().to_lowercase();
    if let Some(rest) = k.strip_prefix("ctrl+") {
        return match rest.as_bytes() {
            [b @ b'a'..=b'z'] => Ok(vec![b - b'a' + 1]),
            b"[" => Ok(vec![0x1b]),
            b"\\" => Ok(vec![0x1c]),
            b"]" => Ok(vec![0x1d]),
            b"space" | b"@" => Ok(vec![0]),
            _ => Err(format!("tecla desconocida `{key}`")),
        };
    }
    if let Some(rest) = k.strip_prefix("alt+") {
        let mut out = vec![0x1b];
        out.extend(key_to_bytes(rest)?);
        return Ok(out);
    }
    let named: Option<&[u8]> = match k.as_str() {
        "enter" | "return" => Some(b"\r"),
        "tab" => Some(b"\t"),
        "esc" | "escape" => Some(b"\x1b"),
        "backspace" => Some(b"\x7f"),
        "space" => Some(b" "),
        "up" => Some(b"\x1b[A"),
        "down" => Some(b"\x1b[B"),
        "right" => Some(b"\x1b[C"),
        "left" => Some(b"\x1b[D"),
        "home" => Some(b"\x1b[H"),
        "end" => Some(b"\x1b[F"),
        "pageup" => Some(b"\x1b[5~"),
        "pagedown" => Some(b"\x1b[6~"),
        "delete" => Some(b"\x1b[3~"),
        "shift+tab" => Some(b"\x1b[Z"),
        _ => None,
    };
    if let Some(b) = named {
        return Ok(b.to_vec());
    }
    if let Some(n) = k.strip_prefix('f').and_then(|n| n.parse::<u8>().ok()) {
        let seq: &[u8] = match n {
            1 => b"\x1bOP",
            2 => b"\x1bOQ",
            3 => b"\x1bOR",
            4 => b"\x1bOS",
            5 => b"\x1b[15~",
            6 => b"\x1b[17~",
            7 => b"\x1b[18~",
            8 => b"\x1b[19~",
            9 => b"\x1b[20~",
            10 => b"\x1b[21~",
            11 => b"\x1b[23~",
            12 => b"\x1b[24~",
            _ => return Err(format!("tecla desconocida `{key}`")),
        };
        return Ok(seq.to_vec());
    }
    // Una letra, un número o un signo suelto.
    let mut chars = key.chars();
    match (chars.next(), chars.next()) {
        (Some(c), None) if !c.is_control() => {
            let mut buf = [0u8; 4];
            Ok(c.encode_utf8(&mut buf).as_bytes().to_vec())
        }
        _ => Err(format!("tecla desconocida `{key}`")),
    }
}

/// Varias teclas en orden.
pub fn keys_to_bytes(keys: &[String]) -> Result<Vec<u8>, String> {
    if keys.is_empty() {
        return Err("no hay teclas".to_owned());
    }
    if keys.len() > MAX_KEYS {
        return Err(format!("más de {MAX_KEYS} teclas"));
    }
    let mut out = Vec::new();
    for k in keys {
        out.extend(key_to_bytes(k)?);
    }
    Ok(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    const TOKEN: &str = "abc123";

    fn line(extra: &str) -> String {
        format!(r#"{{"v":0,"id":7,"token":"{TOKEN}",{extra}}}"#)
    }

    fn error_code(resp: &str) -> String {
        let v: Value = serde_json::from_str(resp.trim()).unwrap();
        assert_eq!(v["ok"], false, "{resp}");
        v["error"]["code"].as_str().unwrap().to_owned()
    }

    #[test]
    fn a_good_request_is_parsed() {
        let r = parse_request(
            &line(r#""client":"claude","method":"screen.read","params":{"session":"1-2"}"#),
            TOKEN,
        )
        .unwrap();
        assert_eq!(r.method, Method::ScreenRead);
        assert_eq!(r.client, "claude");
        assert_eq!(r.id, json!(7));
        assert_eq!(r.params["session"], "1-2");
    }

    #[test]
    fn the_token_is_checked_first_and_exactly() {
        for bad in [
            r#"{"v":0,"id":1,"method":"hello"}"#.to_owned(),
            r#"{"v":0,"id":1,"token":"","method":"hello"}"#.to_owned(),
            r#"{"v":0,"id":1,"token":"abc12","method":"hello"}"#.to_owned(),
            r#"{"v":0,"id":1,"token":"abc1234","method":"hello"}"#.to_owned(),
            r#"{"v":0,"id":1,"token":7,"method":"hello"}"#.to_owned(),
        ] {
            assert_eq!(
                error_code(&parse_request(&bad, TOKEN).unwrap_err()),
                "unauthorized",
                "{bad}"
            );
        }
        // Sin token válido no se revela ni que el método no existe.
        let r = parse_request(r#"{"v":0,"id":1,"token":"x","method":"no.existe"}"#, TOKEN);
        assert_eq!(error_code(&r.unwrap_err()), "unauthorized");
    }

    #[test]
    fn malformed_requests_are_explained() {
        let e = |l: &str| error_code(&parse_request(l, TOKEN).unwrap_err());
        // Ni siquiera con un token válido se acepta algo que no sea una petición.
        assert_eq!(e("no es json"), "bad_request");
        assert_eq!(e("[1,2]"), "bad_request");
        assert_eq!(e("\"hola\""), "bad_request");
    }

    #[test]
    fn versions_and_methods_are_validated() {
        let e = |l: &str| error_code(&parse_request(l, TOKEN).unwrap_err());
        assert_eq!(
            e(&format!(
                r#"{{"v":1,"id":1,"token":"{TOKEN}","method":"hello"}}"#
            )),
            "bad_request"
        );
        assert_eq!(
            e(&format!(r#"{{"id":1,"token":"{TOKEN}","method":"hello"}}"#)),
            "bad_request"
        );
        assert_eq!(e(&line(r#""method":"rm.rf""#)), "unknown_method");
        assert_eq!(e(&line(r#""params":{}"#)), "bad_request");
    }

    #[test]
    fn client_names_are_cleaned() {
        let r = parse_request(&line(r#""client":"a\u001b[31mb","method":"hello""#), TOKEN).unwrap();
        assert_eq!(r.client, "a[31mb");
        let r = parse_request(&line(r#""client":"   ","method":"hello""#), TOKEN).unwrap();
        assert_eq!(r.client, "desconocido");
        let long = "x".repeat(500);
        let r = parse_request(
            &line(&format!(r#""client":"{long}","method":"hello""#)),
            TOKEN,
        )
        .unwrap();
        assert_eq!(r.client.len(), 60);
    }

    #[test]
    fn only_writing_methods_need_a_permission_group() {
        assert_eq!(Method::InputSendText.write_group(), Some(Group::Input));
        assert_eq!(Method::InputSendKeys.write_group(), Some(Group::Input));
        assert_eq!(Method::SessionsOpen.write_group(), Some(Group::Sessions));
        assert_eq!(Method::SessionsClose.write_group(), Some(Group::Sessions));
        for m in [
            Method::Hello,
            Method::SessionsList,
            Method::ScreenRead,
            Method::BlocksList,
            Method::BlocksOutput,
        ] {
            assert_eq!(m.write_group(), None, "{m:?}");
        }
        // Los nombres hacen ida y vuelta.
        for m in [Method::Hello, Method::InputSendKeys, Method::SessionsClose] {
            assert_eq!(Method::parse(m.name()), Some(m));
        }
    }

    #[test]
    fn text_becomes_pty_bytes_without_control_characters() {
        assert_eq!(text_to_bytes("ls -la", false).unwrap(), b"ls -la");
        assert_eq!(text_to_bytes("ls", true).unwrap(), b"ls\r");
        assert_eq!(text_to_bytes("a\nb\r\nc", false).unwrap(), b"a\rb\rc");
        assert_eq!(text_to_bytes("ñ\t", false).unwrap(), "ñ\t".as_bytes());
        // Una secuencia de escape o un Ctrl-C no pasan por el texto.
        assert!(text_to_bytes("a\u{1b}[2Jb", false).is_err());
        assert!(text_to_bytes("a\u{3}", false).is_err());
        assert!(text_to_bytes("a\0", false).is_err());
        assert!(text_to_bytes(&"x".repeat(MAX_TEXT + 1), false).is_err());
    }

    #[test]
    fn keys_map_to_the_bytes_a_terminal_sends() {
        let k = |s: &str| key_to_bytes(s).unwrap();
        assert_eq!(k("ctrl+c"), [3]);
        assert_eq!(k("Ctrl+D"), [4]);
        assert_eq!(k("enter"), b"\r");
        assert_eq!(k("esc"), [0x1b]);
        assert_eq!(k("up"), b"\x1b[A");
        assert_eq!(k("alt+x"), b"\x1bx");
        assert_eq!(k("alt+left"), b"\x1b\x1b[D");
        assert_eq!(k("f1"), b"\x1bOP");
        assert_eq!(k("f12"), b"\x1b[24~");
        assert_eq!(k("q"), b"q");
        assert_eq!(k("ñ"), "ñ".as_bytes());
        assert_eq!(k("shift+tab"), b"\x1b[Z");
        for bad in ["ctrl+1", "banana", "f13", "", "ab"] {
            assert!(key_to_bytes(bad).is_err(), "{bad}");
        }
        let keys: Vec<String> = ["ctrl+c", "enter"].map(String::from).to_vec();
        assert_eq!(keys_to_bytes(&keys).unwrap(), [3, b'\r']);
        assert!(keys_to_bytes(&[]).is_err());
        assert!(keys_to_bytes(&vec!["a".to_owned(); MAX_KEYS + 1]).is_err());
    }

    #[test]
    fn responses_are_one_json_line() {
        let ok = ok(&json!(3), json!({"x": 1}));
        assert!(ok.ends_with('\n') && ok.matches('\n').count() == 1);
        let v: Value = serde_json::from_str(ok.trim()).unwrap();
        assert_eq!(
            (v["v"].clone(), v["ok"].clone(), v["result"]["x"].clone()),
            (json!(0), json!(true), json!(1))
        );
        assert_eq!(error_code(&err(&Value::Null, code::DENIED, "no")), "denied");
    }
}
