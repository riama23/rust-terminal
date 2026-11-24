//! Lógica pura de los bloques visuales (v1.5): etiquetas, orden de relanzar,
//! primera ubicación `ruta:línea:columna` de una salida y orden del editor.
//! Sin AppKit y con tests.

use std::path::{Path, PathBuf};
use std::time::Duration;

/// Un archivo, una línea y (si la hay) una columna dentro de una salida.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Location {
    pub path: PathBuf,
    pub line: u32,
    pub col: Option<u32>,
}

/// `0,3 s`, `12 s`, `1 min 5 s`.
pub fn format_duration(d: Duration) -> String {
    let ms = d.as_millis();
    if ms < 1000 {
        return format!("{ms} ms");
    }
    let secs = d.as_secs();
    if secs < 10 {
        return format!("{:.1} s", d.as_secs_f64()).replace('.', ",");
    }
    if secs < 60 {
        return format!("{secs} s");
    }
    let (m, s) = (secs / 60, secs % 60);
    if m < 60 {
        return format!("{m} min {s} s");
    }
    format!("{} h {} min", m / 60, m % 60)
}

/// Los bytes que reproducen el comando en la PTY (comando e Intro), o `None`
/// si no es seguro reenviarlo: vacío o con caracteres de control (un salto de
/// línea dentro se ejecutaría como varios comandos).
pub fn rerun_bytes(command: &str) -> Option<Vec<u8>> {
    let command = command.trim();
    if command.is_empty() || command.chars().any(char::is_control) {
        return None;
    }
    let mut bytes = command.as_bytes().to_vec();
    bytes.push(b'\r');
    Some(bytes)
}

/// Quita las secuencias de escape ANSI (CSI) de un texto.
fn strip_ansi(text: &str) -> String {
    let mut out = String::with_capacity(text.len());
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.peek() == Some(&'[') {
                chars.next();
                for n in chars.by_ref() {
                    if ('\u{40}'..='\u{7e}').contains(&n) {
                        break;
                    }
                }
            }
        } else {
            out.push(c);
        }
    }
    out
}

/// Primer `ruta:línea[:columna]` de `output` que apunta a un archivo que
/// existe (relativo a `cwd`, o absoluto). `exists` se inyecta para los tests.
pub fn first_location(
    output: &str,
    cwd: Option<&Path>,
    exists: impl Fn(&Path) -> bool,
) -> Option<Location> {
    let clean = strip_ansi(output);
    for line in clean.lines() {
        for raw in line.split(|c: char| c.is_whitespace() || "()[]<>\"'`,;=".contains(c)) {
            let Some(loc) = parse_token(raw) else {
                continue;
            };
            let path = if loc.path.is_absolute() {
                loc.path.clone()
            } else if let Some(cwd) = cwd {
                cwd.join(&loc.path)
            } else {
                continue;
            };
            if exists(&path) {
                return Some(Location { path, ..loc });
            }
        }
    }
    None
}

/// `src/main.rs:12:5` → ruta, línea y columna. Quita el `:` final de los
/// mensajes (`main.rs:12:`).
fn parse_token(raw: &str) -> Option<Location> {
    let token = raw.trim_end_matches(':');
    let mut parts = token.rsplitn(3, ':');
    let last = parts.next()?;
    let (path, line, col) = match (
        last.parse::<u32>(),
        parts.next().map(|p| p.parse::<u32>()),
        parts.next(),
    ) {
        // ruta:línea:columna
        (Ok(col), Some(Ok(line)), Some(path)) => (path, line, Some(col)),
        // ruta:línea
        (Ok(line), Some(Err(_)), _) | (Ok(line), None, _) => {
            // `rsplitn` partió mal si había dos números; reconstruye la ruta.
            let path = token.strip_suffix(&format!(":{last}"))?;
            (path, line, None)
        }
        _ => return None,
    };
    if line == 0 || path.is_empty() || !looks_like_path(path) {
        return None;
    }
    Some(Location {
        path: PathBuf::from(path),
        line,
        col,
    })
}

/// Evita falsos positivos como `12:30:45` o `http://…:8080`.
fn looks_like_path(p: &str) -> bool {
    if p.contains("://") || p.chars().all(|c| c.is_ascii_digit() || c == ':') {
        return false;
    }
    p.contains('/')
        || p.rsplit_once('.').is_some_and(|(stem, ext)| {
            !stem.is_empty()
                && !ext.is_empty()
                && ext.len() <= 6
                && ext.chars().all(char::is_alphanumeric)
        })
}

/// Argumentos para abrir `loc` con la plantilla de `blocks.editor`
/// (`code -g {path}:{line}:{col}`). Vacía: `open -t <ruta>` (el editor de
/// texto por defecto, sin línea). Cada palabra de la plantilla es un
/// argumento: nada pasa por una shell.
pub fn editor_args(template: &str, loc: &Location) -> Vec<String> {
    let path = loc.path.to_string_lossy();
    if template.trim().is_empty() {
        return vec!["open".into(), "-t".into(), path.into_owned()];
    }
    template
        .split_whitespace()
        .map(|w| {
            w.replace("{path}", &path)
                .replace("{line}", &loc.line.to_string())
                .replace("{col}", &loc.col.unwrap_or(1).to_string())
        })
        .collect()
}

/// Hacia dónde mover la selección de bloque.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Step {
    Previous,
    Next,
}

/// El bloque al que lleva ⌘↑/⌘↓. `ids` son los comandos ejecutados, del más
/// antiguo al más reciente. Con un bloque seleccionado, el anterior o el
/// siguiente a él; sin selección, ⌘↑ elige el más reciente y ⌘↓ ninguno.
pub fn step_block(ids: &[u64], selected: Option<u64>, step: Step) -> Option<u64> {
    match (
        selected.and_then(|id| ids.iter().position(|b| *b == id)),
        step,
    ) {
        (Some(i), Step::Previous) => i.checked_sub(1).map(|j| ids[j]),
        (Some(i), Step::Next) => ids.get(i + 1).copied(),
        (None, Step::Previous) => ids.last().copied(),
        (None, Step::Next) => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn stepping_between_blocks() {
        let ids = [1, 2, 3];
        assert_eq!(step_block(&ids, None, Step::Previous), Some(3));
        assert_eq!(step_block(&ids, None, Step::Next), None);
        assert_eq!(step_block(&ids, Some(2), Step::Previous), Some(1));
        assert_eq!(step_block(&ids, Some(2), Step::Next), Some(3));
        assert_eq!(step_block(&ids, Some(1), Step::Previous), None);
        assert_eq!(step_block(&ids, Some(3), Step::Next), None);
        // Una selección que ya no existe se trata como ninguna.
        assert_eq!(step_block(&ids, Some(9), Step::Previous), Some(3));
        assert_eq!(step_block(&[], None, Step::Previous), None);
    }

    fn all(_: &Path) -> bool {
        true
    }

    #[test]
    fn durations_read_naturally() {
        assert_eq!(format_duration(Duration::from_millis(40)), "40 ms");
        assert_eq!(format_duration(Duration::from_millis(1250)), "1,2 s");
        assert_eq!(format_duration(Duration::from_secs(12)), "12 s");
        assert_eq!(format_duration(Duration::from_secs(65)), "1 min 5 s");
        assert_eq!(format_duration(Duration::from_secs(3720)), "1 h 2 min");
    }

    #[test]
    fn rerun_refuses_anything_that_could_run_more_than_the_command() {
        assert_eq!(rerun_bytes(" cargo test "), Some(b"cargo test\r".to_vec()));
        assert_eq!(rerun_bytes(""), None);
        assert_eq!(rerun_bytes("   "), None);
        assert_eq!(rerun_bytes("echo a\nrm -rf x"), None);
        assert_eq!(rerun_bytes("echo \u{1b}[31m"), None);
        assert_eq!(
            rerun_bytes("echo ñ; ls"),
            Some("echo ñ; ls\r".as_bytes().to_vec())
        );
    }

    #[test]
    fn finds_the_first_location_in_rust_python_and_js_output() {
        let cwd = Path::new("/p");
        let rust = "error[E0308]: mismatched types\n --> src/main.rs:12:5\n  |";
        assert_eq!(
            first_location(rust, Some(cwd), all),
            Some(Location {
                path: "/p/src/main.rs".into(),
                line: 12,
                col: Some(5)
            })
        );
        let py = "  File \"app/x.py\", line 3, in <module>\nTraceback:\nfoo.py:7: error";
        assert_eq!(
            first_location(py, Some(cwd), all),
            Some(Location {
                path: "/p/foo.py".into(),
                line: 7,
                col: None
            })
        );
        let js = "at Object.<anonymous> (/abs/dir/test.js:40:9)";
        assert_eq!(
            first_location(js, Some(cwd), all),
            Some(Location {
                path: "/abs/dir/test.js".into(),
                line: 40,
                col: Some(9)
            })
        );
        let colored = "\u{1b}[1;31msrc/lib.rs:8:1\u{1b}[0m: warning";
        assert_eq!(
            first_location(colored, Some(cwd), all).map(|l| l.line),
            Some(8)
        );
    }

    #[test]
    fn ignores_times_urls_and_missing_files() {
        let cwd = Path::new("/p");
        assert_eq!(first_location("12:30:45 started", Some(cwd), all), None);
        assert_eq!(
            first_location("see http://localhost:8080/x", Some(cwd), all),
            None
        );
        assert_eq!(first_location("v1.2:3 done", Some(cwd), |_| false), None);
        // Un archivo que no existe se salta y se sigue con el siguiente.
        let out = "nope.rs:1:1 y real.rs:2:2";
        let only_real = |p: &Path| p.ends_with("real.rs");
        assert_eq!(
            first_location(out, Some(cwd), only_real).map(|l| l.path),
            Some("/p/real.rs".into())
        );
        // Sin directorio, solo valen las rutas absolutas.
        assert_eq!(first_location("a.rs:1", None, all), None);
        assert!(first_location("/a/b.rs:1", None, all).is_some());
    }

    #[test]
    fn editor_template_is_split_without_a_shell() {
        let loc = Location {
            path: "/p/a b.rs".into(),
            line: 3,
            col: None,
        };
        assert_eq!(editor_args("", &loc), ["open", "-t", "/p/a b.rs"]);
        assert_eq!(
            editor_args("code -g {path}:{line}:{col}", &loc),
            ["code", "-g", "/p/a b.rs:3:1"]
        );
        // Una ruta con `;` o `$()` sigue siendo un solo argumento.
        let evil = Location {
            path: "/p/$(x);y.rs".into(),
            line: 1,
            col: Some(2),
        };
        assert_eq!(
            editor_args("zed {path}:{line}", &evil),
            ["zed", "/p/$(x);y.rs:1"]
        );
    }
}
