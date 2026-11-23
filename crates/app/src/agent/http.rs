//! HTTP/1.1 mínimo para el broker: una petición por conexión, con límites.
//!
//! Solo lo que necesitan los hooks de Claude Code: `POST` con
//! `Content-Length`. Nada de `Transfer-Encoding`, `Expect` ni keep-alive.

use std::io::{self, BufRead, Read, Write};

/// Tamaño máximo de la línea de petición y las cabeceras juntas.
pub const MAX_HEADER_BYTES: usize = 16 * 1024;
/// Tamaño máximo del cuerpo (las respuestas de herramientas pueden ser grandes).
pub const MAX_BODY_BYTES: usize = 8 * 1024 * 1024;

#[derive(Debug)]
pub struct Request {
    pub method: String,
    pub path: String,
    pub body: Vec<u8>,
}

#[derive(Debug)]
pub enum HttpError {
    Malformed(&'static str),
    TooLarge,
    Io(io::Error),
}

impl From<io::Error> for HttpError {
    fn from(e: io::Error) -> Self {
        HttpError::Io(e)
    }
}

/// Lee una línea sin pasar del presupuesto de cabeceras.
fn read_line<R: BufRead>(r: &mut R, line: &mut String, used: &mut usize) -> Result<(), HttpError> {
    line.clear();
    let budget = (MAX_HEADER_BYTES - *used) as u64 + 1;
    let n = r.by_ref().take(budget).read_line(line)?;
    if n == 0 {
        return Err(HttpError::Malformed("conexión cerrada antes de tiempo"));
    }
    *used += n;
    if *used > MAX_HEADER_BYTES {
        return Err(HttpError::TooLarge);
    }
    Ok(())
}

pub fn read_request<R: BufRead>(r: &mut R) -> Result<Request, HttpError> {
    let mut used = 0;
    let mut line = String::new();
    read_line(r, &mut line, &mut used)?;
    let mut parts = line.trim_end().split(' ');
    let method = parts
        .next()
        .filter(|m| !m.is_empty())
        .ok_or(HttpError::Malformed("línea de petición"))?
        .to_owned();
    let path = parts
        .next()
        .ok_or(HttpError::Malformed("línea de petición"))?
        .to_owned();
    let version = parts
        .next()
        .ok_or(HttpError::Malformed("línea de petición"))?;
    if !version.starts_with("HTTP/1.") {
        return Err(HttpError::Malformed("versión de HTTP"));
    }

    let mut content_length = None;
    loop {
        read_line(r, &mut line, &mut used)?;
        let l = line.trim_end_matches(['\r', '\n']);
        if l.is_empty() {
            break;
        }
        let (name, value) = l.split_once(':').ok_or(HttpError::Malformed("cabecera"))?;
        match name.trim().to_ascii_lowercase().as_str() {
            "content-length" => {
                content_length = Some(
                    value
                        .trim()
                        .parse::<usize>()
                        .map_err(|_| HttpError::Malformed("Content-Length"))?,
                );
            }
            "transfer-encoding" => return Err(HttpError::Malformed("Transfer-Encoding")),
            _ => {}
        }
    }

    let len = content_length.unwrap_or(0);
    if len > MAX_BODY_BYTES {
        return Err(HttpError::TooLarge);
    }
    let mut body = vec![0; len];
    r.read_exact(&mut body)?;
    Ok(Request { method, path, body })
}

fn reason(status: u16) -> &'static str {
    match status {
        200 => "OK",
        400 => "Bad Request",
        403 => "Forbidden",
        405 => "Method Not Allowed",
        413 => "Payload Too Large",
        _ => "Error",
    }
}

pub fn write_response<W: Write>(w: &mut W, status: u16, body: &str) -> io::Result<()> {
    write!(
        w,
        "HTTP/1.1 {status} {}\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
        reason(status),
        body.len()
    )?;
    w.flush()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(raw: &str) -> Result<Request, HttpError> {
        read_request(&mut raw.as_bytes())
    }

    #[test]
    fn lee_un_post_con_cuerpo() {
        let r = parse("POST /hook/a/b/Stop HTTP/1.1\r\nHost: x\r\nContent-Length: 5\r\n\r\nhello")
            .unwrap();
        assert_eq!(r.method, "POST");
        assert_eq!(r.path, "/hook/a/b/Stop");
        assert_eq!(r.body, b"hello");
    }

    #[test]
    fn cabeceras_sin_distinguir_mayusculas() {
        let r = parse("POST / HTTP/1.1\r\ncontent-LENGTH: 2\r\n\r\nhi").unwrap();
        assert_eq!(r.body, b"hi");
    }

    #[test]
    fn sin_content_length_el_cuerpo_es_vacio() {
        let r = parse("POST / HTTP/1.1\r\n\r\n").unwrap();
        assert!(r.body.is_empty());
    }

    #[test]
    fn rechaza_transfer_encoding() {
        let e = parse("POST / HTTP/1.1\r\nTransfer-Encoding: chunked\r\n\r\n").unwrap_err();
        assert!(matches!(e, HttpError::Malformed(_)));
    }

    #[test]
    fn rechaza_cuerpo_demasiado_grande() {
        let raw = format!(
            "POST / HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            MAX_BODY_BYTES + 1
        );
        assert!(matches!(parse(&raw).unwrap_err(), HttpError::TooLarge));
    }

    #[test]
    fn rechaza_cabeceras_demasiado_grandes() {
        let raw = format!(
            "POST / HTTP/1.1\r\nX: {}\r\n\r\n",
            "a".repeat(MAX_HEADER_BYTES)
        );
        assert!(matches!(parse(&raw).unwrap_err(), HttpError::TooLarge));
    }

    #[test]
    fn rechaza_peticiones_mal_formadas() {
        for raw in [
            "",
            "POST\r\n\r\n",
            "POST / FTP/1.0\r\n\r\n",
            "POST / HTTP/1.1\r\nsin-dos-puntos\r\n\r\n",
        ] {
            assert!(parse(raw).is_err(), "{raw:?}");
        }
    }

    #[test]
    fn cuerpo_truncado_es_un_error() {
        assert!(parse("POST / HTTP/1.1\r\nContent-Length: 10\r\n\r\nabc").is_err());
    }

    #[test]
    fn escribe_la_respuesta_con_su_longitud() {
        let mut out = Vec::new();
        write_response(&mut out, 200, "{}").unwrap();
        let s = String::from_utf8(out).unwrap();
        assert!(s.starts_with("HTTP/1.1 200 OK\r\n"));
        assert!(s.contains("Content-Length: 2\r\n"));
        assert!(s.ends_with("\r\n\r\n{}"));
    }
}
