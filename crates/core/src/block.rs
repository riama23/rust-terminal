//! Bloques de comandos a partir de las marcas OSC 133 de la shell.
//!
//! Un bloque es prompt → comando → salida → código de salida. Las posiciones
//! son líneas absolutas, como la selección, y se recolocan en el reflow.
//! Nunca se deducen del texto: si la shell no envía marcas, no hay bloques.

use crate::grid::Point;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Block {
    pub id: u64,
    /// Marca A: empieza el prompt.
    pub prompt: Point,
    /// Marca B: empieza lo que escribe el usuario.
    pub input: Option<Point>,
    /// Marca C: empieza la salida del comando.
    pub output: Option<Point>,
    /// Marca D: termina la salida.
    pub end: Option<Point>,
    pub exit_code: Option<i32>,
    /// Texto del comando, tal como lo envía la shell (no leído de la pantalla).
    pub command: Option<String>,
    /// Directorio de trabajo (OSC 7) cuando se mostró el prompt.
    pub cwd: Option<String>,
    /// Cuándo empezó la salida (marca C) y cuándo terminó (marca D).
    pub started_at: Option<std::time::Instant>,
    pub finished_at: Option<std::time::Instant>,
}

impl Block {
    pub fn is_finished(&self) -> bool {
        self.end.is_some()
    }

    /// Cuánto tardó el comando, si ya terminó.
    pub fn duration(&self) -> Option<std::time::Duration> {
        Some(self.finished_at?.duration_since(self.started_at?))
    }

    /// Todos los puntos del bloque, para recolocarlos en el reflow.
    pub(crate) fn points_mut(&mut self) -> impl Iterator<Item = &mut Point> {
        std::iter::once(&mut self.prompt)
            .chain(self.input.as_mut())
            .chain(self.output.as_mut())
            .chain(self.end.as_mut())
    }
}

/// Decodifica `%XX` (de `cmdline_url=` y de las URL `file://` de OSC 7).
pub(crate) fn percent_decode(s: &str) -> String {
    let bytes = s.as_bytes();
    let mut out = Vec::with_capacity(bytes.len());
    let mut i = 0;
    while i < bytes.len() {
        if bytes[i] == b'%'
            && i + 2 < bytes.len()
            && let Ok(b) =
                u8::from_str_radix(std::str::from_utf8(&bytes[i + 1..i + 3]).unwrap_or("x"), 16)
        {
            out.push(b);
            i += 3;
        } else {
            out.push(bytes[i]);
            i += 1;
        }
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// Ruta de una URL `file://host/ruta` de OSC 7.
pub(crate) fn path_from_file_url(url: &str) -> Option<String> {
    let rest = url.strip_prefix("file://")?;
    let path = &rest[rest.find('/')?..];
    Some(percent_decode(path))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn decodes_percent_escapes() {
        assert_eq!(
            percent_decode("ls%20-la%3B%20echo%20%C3%B1"),
            "ls -la; echo ñ"
        );
        assert_eq!(percent_decode("100%"), "100%");
    }

    #[test]
    fn parses_osc7_urls() {
        assert_eq!(
            path_from_file_url("file://mac.local/Users/yo/mi%20dir"),
            Some("/Users/yo/mi dir".to_owned())
        );
        assert_eq!(path_from_file_url("http://x/y"), None);
    }
}
