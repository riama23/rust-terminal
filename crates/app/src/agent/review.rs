//! El estado de una revisión: archivos del diff, comentarios en lote y el
//! mensaje que se envía al agente. Puro y con tests.
//!
//! El mensaje se **teclea en la PTY** del agente (no hay otro canal), así que
//! se sanea (sin caracteres de control: un ESC podría cerrar el pegado entre
//! corchetes e inyectar teclas) y solo se envía cuando el agente ha terminado
//! su turno: con un prompt de permiso en pantalla, ese texto lo contestaría.

use super::diff::{DiffFile, LineKind};
use super::model::Status;

pub const MAX_COMMENT_CHARS: usize = 2000;
pub const MAX_COMMENTS: usize = 100;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Side {
    /// Línea del archivo nuevo (añadida o de contexto).
    New,
    /// Línea del archivo antiguo (borrada).
    Old,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Comment {
    pub file: String,
    pub line: u32,
    pub side: Side,
    pub text: String,
}

#[derive(Debug, PartialEq, Eq)]
pub enum ReviewError {
    EmptyComment,
    TooLong,
    TooMany,
    /// La línea no existe en el diff (cambió mientras se revisaba).
    NoSuchLine,
}

impl std::fmt::Display for ReviewError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            ReviewError::EmptyComment => "El comentario está vacío.",
            ReviewError::TooLong => "El comentario es demasiado largo.",
            ReviewError::TooMany => "Hay demasiados comentarios en la revisión.",
            ReviewError::NoSuchLine => "Esa línea ya no está en el diff.",
        })
    }
}

#[derive(Debug, Default)]
pub struct Review {
    pub files: Vec<DiffFile>,
    comments: Vec<Comment>,
}

/// Deja solo texto imprimible: los saltos de línea se conservan y el resto de
/// caracteres de control (ESC, retorno de carro, tabuladores…) se descartan.
pub fn sanitize(text: &str) -> String {
    text.replace("\r\n", "\n")
        .chars()
        .filter_map(|c| match c {
            '\n' => Some('\n'),
            '\t' => Some(' '),
            c if c.is_control() => None,
            c => Some(c),
        })
        .collect::<String>()
        .trim()
        .to_owned()
}

impl Review {
    pub fn new(files: Vec<DiffFile>) -> Self {
        Review {
            files,
            comments: Vec::new(),
        }
    }

    /// Sustituye el diff (se refrescó) conservando los comentarios que siguen
    /// apuntando a una línea que existe. Devuelve cuántos se perdieron.
    pub fn refresh(&mut self, files: Vec<DiffFile>) -> usize {
        self.files = files;
        let before = self.comments.len();
        let files = &self.files;
        self.comments
            .retain(|c| line_exists(files, &c.file, c.side, c.line));
        before - self.comments.len()
    }

    pub fn comments(&self) -> &[Comment] {
        &self.comments
    }

    pub fn add_comment(
        &mut self,
        file: &str,
        side: Side,
        line: u32,
        text: &str,
    ) -> Result<(), ReviewError> {
        let text = sanitize(text);
        if text.is_empty() {
            return Err(ReviewError::EmptyComment);
        }
        if text.chars().count() > MAX_COMMENT_CHARS {
            return Err(ReviewError::TooLong);
        }
        if self.comments.len() >= MAX_COMMENTS {
            return Err(ReviewError::TooMany);
        }
        if !line_exists(&self.files, file, side, line) {
            return Err(ReviewError::NoSuchLine);
        }
        self.comments.push(Comment {
            file: file.to_owned(),
            line,
            side,
            text,
        });
        Ok(())
    }

    pub fn remove_comment(&mut self, index: usize) -> bool {
        if index < self.comments.len() {
            self.comments.remove(index);
            true
        } else {
            false
        }
    }

    pub fn count_for(&self, file: &str) -> usize {
        self.comments.iter().filter(|c| c.file == file).count()
    }

    pub fn comments_at(&self, file: &str, side: Side, line: u32) -> impl Iterator<Item = &Comment> {
        let file = file.to_owned();
        self.comments
            .iter()
            .filter(move |c| c.file == file && c.side == side && c.line == line)
    }

    pub fn totals(&self) -> (usize, usize, usize) {
        (
            self.files.iter().map(|f| f.added).sum(),
            self.files.iter().map(|f| f.removed).sum(),
            self.files.len(),
        )
    }

    /// El mensaje único con todos los comentarios, por archivo y línea.
    pub fn message(&self) -> Option<String> {
        if self.comments.is_empty() {
            return None;
        }
        let mut sorted: Vec<&Comment> = self.comments.iter().collect();
        sorted
            .sort_by(|a, b| (&a.file, a.line, a.side as u8).cmp(&(&b.file, b.line, b.side as u8)));
        let n = sorted.len();
        let mut out = format!(
            "Revisión de tus cambios: {n} {}.\n",
            if n == 1 { "comentario" } else { "comentarios" }
        );
        for c in sorted {
            let old = if c.side == Side::Old {
                " (línea eliminada)"
            } else {
                ""
            };
            out.push_str(&format!("\n{}:{}{old}\n", c.file, c.line));
            for line in c.text.lines() {
                out.push_str(&format!("  {line}\n"));
            }
        }
        Some(out.trim_end().to_owned())
    }

    /// Tras enviar, la revisión empieza de cero.
    pub fn clear_comments(&mut self) {
        self.comments.clear();
    }
}

fn line_exists(files: &[DiffFile], file: &str, side: Side, line: u32) -> bool {
    files.iter().filter(|f| f.path == file).any(|f| {
        f.hunks.iter().flat_map(|h| &h.lines).any(|l| match side {
            Side::New => l.new == Some(line) && l.kind != LineKind::Del,
            Side::Old => l.old == Some(line) && l.kind == LineKind::Del,
        })
    })
}

/// ¿Se puede enviar ahora el mensaje a un agente con este estado?
pub fn can_send(status: &Status) -> Result<(), &'static str> {
    match status {
        Status::Done => Ok(()),
        Status::NeedsYou { .. } | Status::PlanReady => {
            Err("El agente espera una respuesta. Contéstala primero: ese texto la contestaría.")
        }
        Status::Working => {
            Err("El agente está trabajando. Envía los comentarios cuando termine su turno.")
        }
        Status::Error => Err("La sesión terminó con un error."),
        Status::Ended => Err("La sesión ya terminó."),
    }
}

const PASTE_START: &[u8] = b"\x1b[200~";
const PASTE_END: &[u8] = b"\x1b[201~";

/// Los bytes que se escriben en la PTY: el mensaje y, aparte, Intro. Con
/// pegado entre corchetes los saltos de línea no envían nada; sin él, el
/// mensaje va en una sola línea para que un salto no lo envíe a medias.
pub fn pty_payload(message: &str, bracketed_paste: bool) -> Vec<u8> {
    let clean = sanitize(message);
    let mut out = Vec::new();
    if bracketed_paste {
        out.extend_from_slice(PASTE_START);
        out.extend_from_slice(clean.as_bytes());
        out.extend_from_slice(PASTE_END);
    } else {
        out.extend_from_slice(clean.replace('\n', " ").as_bytes());
    }
    out.push(b'\r');
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::diff::parse;

    fn files() -> Vec<DiffFile> {
        parse(
            "diff --git a/src/a.rs b/src/a.rs\n--- a/src/a.rs\n+++ b/src/a.rs\n@@ -1,3 +1,3 @@\n uno\n-dos\n+DOS\n tres\n\
             diff --git a/b.rs b/b.rs\n--- a/b.rs\n+++ b/b.rs\n@@ -5 +5 @@\n-x\n+y\n",
        )
    }

    fn review() -> Review {
        Review::new(files())
    }

    #[test]
    fn comenta_lineas_nuevas_y_antiguas_que_existen() {
        let mut r = review();
        assert!(
            r.add_comment("src/a.rs", Side::New, 2, "¿y si es vacío?")
                .is_ok()
        );
        assert!(r.add_comment("src/a.rs", Side::Old, 2, "¿seguro?").is_ok());
        assert_eq!(r.comments().len(), 2);
        assert_eq!(r.count_for("src/a.rs"), 2);
        assert_eq!(r.count_for("b.rs"), 0);
    }

    #[test]
    fn no_se_comenta_una_linea_que_no_existe_o_del_lado_equivocado() {
        let mut r = review();
        assert_eq!(
            r.add_comment("src/a.rs", Side::New, 99, "x"),
            Err(ReviewError::NoSuchLine)
        );
        assert_eq!(
            r.add_comment("nada.rs", Side::New, 1, "x"),
            Err(ReviewError::NoSuchLine)
        );
        // La línea 2 antigua es una borrada; la línea 2 nueva, una añadida.
        // La 3 antigua es contexto: solo existe como línea nueva.
        assert_eq!(
            r.add_comment("src/a.rs", Side::Old, 3, "x"),
            Err(ReviewError::NoSuchLine)
        );
        assert!(r.add_comment("src/a.rs", Side::New, 3, "x").is_ok());
    }

    #[test]
    fn valida_el_texto() {
        let mut r = review();
        assert_eq!(
            r.add_comment("src/a.rs", Side::New, 1, "  \n\t "),
            Err(ReviewError::EmptyComment)
        );
        assert_eq!(
            r.add_comment("src/a.rs", Side::New, 1, "\x1b\x07"),
            Err(ReviewError::EmptyComment)
        );
        let largo = "x".repeat(MAX_COMMENT_CHARS + 1);
        assert_eq!(
            r.add_comment("src/a.rs", Side::New, 1, &largo),
            Err(ReviewError::TooLong)
        );
        for _ in 0..MAX_COMMENTS {
            r.add_comment("src/a.rs", Side::New, 1, "c").unwrap();
        }
        assert_eq!(
            r.add_comment("src/a.rs", Side::New, 1, "uno más"),
            Err(ReviewError::TooMany)
        );
    }

    #[test]
    fn sanear_quita_los_controles_y_conserva_los_saltos() {
        assert_eq!(
            sanitize("hola\x1b[201~\nmundo\r\n\x07fin\x7f"),
            "hola[201~\nmundo\nfin"
        );
        assert_eq!(sanitize("a\tb"), "a b");
        assert_eq!(sanitize("  ñandú ✓  "), "ñandú ✓");
        assert!(!sanitize("\x1b[200~\x03\x04").contains('\x1b'));
    }

    #[test]
    fn el_mensaje_agrupa_por_archivo_y_linea_con_referencias() {
        let mut r = review();
        r.add_comment("src/a.rs", Side::New, 2, "Maneja el vacío.\nY el nulo.")
            .unwrap();
        r.add_comment("b.rs", Side::New, 5, "¿Por qué y?").unwrap();
        r.add_comment("src/a.rs", Side::Old, 2, "¿Se puede borrar?")
            .unwrap();
        let m = r.message().unwrap();
        let esperado = "Revisión de tus cambios: 3 comentarios.\n\
                        \n\
                        b.rs:5\n  ¿Por qué y?\n\
                        \n\
                        src/a.rs:2\n  Maneja el vacío.\n  Y el nulo.\n\
                        \n\
                        src/a.rs:2 (línea eliminada)\n  ¿Se puede borrar?";
        assert_eq!(m, esperado);
    }

    #[test]
    fn sin_comentarios_no_hay_mensaje_y_uno_va_en_singular() {
        let mut r = review();
        assert_eq!(r.message(), None);
        r.add_comment("b.rs", Side::New, 5, "ok").unwrap();
        assert!(
            r.message()
                .unwrap()
                .starts_with("Revisión de tus cambios: 1 comentario.\n")
        );
    }

    #[test]
    fn un_diff_nuevo_conserva_solo_los_comentarios_que_siguen_valiendo() {
        let mut r = review();
        r.add_comment("src/a.rs", Side::New, 2, "a").unwrap();
        r.add_comment("b.rs", Side::New, 5, "b").unwrap();
        let solo_b: Vec<DiffFile> = files().into_iter().filter(|f| f.path == "b.rs").collect();
        assert_eq!(r.refresh(solo_b), 1);
        assert_eq!(r.comments().len(), 1);
        assert_eq!(r.comments()[0].file, "b.rs");
    }

    #[test]
    fn quitar_y_vaciar() {
        let mut r = review();
        r.add_comment("b.rs", Side::New, 5, "b").unwrap();
        assert!(!r.remove_comment(3));
        assert!(r.remove_comment(0));
        r.add_comment("b.rs", Side::New, 5, "b").unwrap();
        r.clear_comments();
        assert_eq!(r.message(), None);
    }

    #[test]
    fn totales_del_encabezado() {
        assert_eq!(review().totals(), (2, 2, 2));
    }

    #[test]
    fn solo_se_envia_cuando_el_agente_termino_su_turno() {
        assert!(can_send(&Status::Done).is_ok());
        for s in [
            Status::NeedsYou {
                tool: "Bash".into(),
            },
            Status::PlanReady,
            Status::Working,
            Status::Error,
            Status::Ended,
        ] {
            assert!(can_send(&s).is_err(), "{s:?}");
        }
        assert!(
            can_send(&Status::NeedsYou {
                tool: String::new()
            })
            .unwrap_err()
            .contains("Contéstala")
        );
    }

    #[test]
    fn el_payload_envuelve_el_pegado_y_termina_con_intro_aparte() {
        let p = pty_payload("a\nb", true);
        assert_eq!(p, b"\x1b[200~a\nb\x1b[201~\r");
        let sin = pty_payload("a\nb", false);
        assert_eq!(sin, b"a b\r");
    }

    #[test]
    fn un_comentario_hostil_no_puede_cerrar_el_pegado_ni_meter_teclas() {
        let mut r = review();
        r.add_comment("b.rs", Side::New, 5, "mal\x1b[201~\rrm -rf ~\x03")
            .unwrap();
        let p = pty_payload(&r.message().unwrap(), true);
        let cuerpo = &p[PASTE_START.len()..p.len() - PASTE_END.len() - 1];
        assert!(!cuerpo.contains(&0x1b), "ningún ESC dentro del pegado");
        assert!(!cuerpo.contains(&b'\r') && !cuerpo.contains(&0x03));
        assert_eq!(
            p.iter().filter(|&&b| b == 0x1b).count(),
            2,
            "solo los dos del envoltorio"
        );
    }
}
