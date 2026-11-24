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

/// ¿Es un nombre de test seguro para ponerlo en una orden? Solo letras,
/// dígitos y `_ : / . -`: los nombres salen de la salida de un programa, que
/// no es de fiar.
fn safe_test_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 200
        && !name.starts_with('-')
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || "_:/.-".contains(c))
}

/// La orden para repetir solo los tests que fallaron, si el programa lo
/// permite y la salida dice cuáles: `cargo test` (nombres de la lista
/// `failures:`), `pytest` (`--lf`), `jest` (`--onlyFailures`) y `go test`
/// (`--- FAIL:`). `None` en cualquier otro caso.
pub fn failed_only_command(command: &str, output: &str) -> Option<String> {
    let command = command.trim();
    let has = |w: &str| command.split_whitespace().any(|t| t == w);
    let tokens: Vec<&str> = command.split_whitespace().collect();
    // `cargo test …`, también tras variables de entorno (`RUST_LOG=x cargo test`).
    let cargo_test = tokens.windows(2).any(|w| w[0] == "cargo" && w[1] == "test");
    if cargo_test {
        let clean = strip_ansi(output);
        // La lista final: tras la última línea `failures:` hasta una en blanco.
        let mut names: Vec<String> = Vec::new();
        let lines: Vec<&str> = clean.lines().collect();
        if let Some(start) = lines.iter().rposition(|l| l.trim_end() == "failures:") {
            for l in &lines[start + 1..] {
                if l.trim().is_empty() {
                    break;
                }
                let n = l.trim();
                if !safe_test_name(n) {
                    return None;
                }
                names.push(n.to_owned());
            }
        }
        if names.is_empty() {
            return None;
        }
        names.dedup();
        // Los nombres van antes de `--` (los argumentos del ejecutable de test).
        let list = names.join(" ");
        return Some(match command.split_once(" -- ") {
            Some((head, tail)) => format!("{head} {list} -- {tail}"),
            None => format!("{command} {list}"),
        });
    }
    if (has("pytest") || has("py.test")) && !command.contains("--lf") {
        return Some(format!("{command} --lf"));
    }
    if has("jest") && !command.contains("--onlyFailures") {
        return Some(format!("{command} --onlyFailures"));
    }
    if tokens.windows(2).any(|w| w[0] == "go" && w[1] == "test") && !command.contains("-run") {
        let clean = strip_ansi(output);
        let mut names: Vec<&str> = clean
            .lines()
            .filter_map(|l| l.trim().strip_prefix("--- FAIL: "))
            .filter_map(|l| l.split_whitespace().next())
            .collect();
        names.dedup();
        if names.is_empty() || !names.iter().all(|n| safe_test_name(n)) {
            return None;
        }
        // Los subtests (`A/B`) se piden por su test padre.
        let mut parents: Vec<&str> = names
            .iter()
            .map(|n| n.split('/').next().unwrap_or(n))
            .collect();
        parents.dedup();
        return Some(format!("{command} -run '^({})$'", parents.join("|")));
    }
    None
}

/// Un bloque colapsado: su salida ocupa las líneas absolutas `first..=last`.
/// Se enseña solo `first` (con un resumen) y se ocultan las demás.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Collapse {
    pub first: u64,
    pub last: u64,
}

/// Lo que se colapsa de un bloque: su salida desde la línea `output_line`
/// hasta donde acaba (`end` = línea y columna de la marca D). `None` si la
/// salida no pasa de una línea.
pub fn collapse_of(output_line: u64, end: Option<(u64, usize)>) -> Option<Collapse> {
    let (line, col) = end?;
    let last = if col == 0 { line.checked_sub(1)? } else { line };
    (last > output_line).then_some(Collapse {
        first: output_line,
        last,
    })
}

/// Cómo se reparten las filas de la vista cuando hay bloques colapsados: las
/// ocultas se saltan al pintar y las de debajo suben. La terminal no se entera
/// (su rejilla no cambia): es solo la forma de dibujarla y de leer el ratón.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct RowMap {
    /// Fila de pantalla de cada fila de la vista (`None`: oculta).
    screen: Vec<Option<usize>>,
    /// Fila de la vista de cada fila de pantalla visible.
    view: Vec<usize>,
    /// `(fila de la vista, líneas ocultas)` donde va el resumen.
    pub summaries: Vec<(usize, usize)>,
}

impl RowMap {
    /// Sin nada oculto (el caso normal).
    pub fn is_identity(&self) -> bool {
        self.summaries.is_empty()
    }

    /// Fila de pantalla donde se pinta la fila `view_row` de la vista.
    pub fn screen_of(&self, view_row: usize) -> Option<usize> {
        if self.screen.is_empty() {
            return Some(view_row);
        }
        self.screen.get(view_row).copied().unwrap_or(Some(view_row))
    }

    /// Fila de la vista que hay en la fila de pantalla `screen_row`.
    pub fn view_of(&self, screen_row: usize) -> usize {
        if self.is_identity() {
            return screen_row;
        }
        match self.view.get(screen_row) {
            Some(v) => *v,
            // Por debajo de lo dibujado: las filas ocultas dejan hueco al final.
            None => screen_row + (self.screen.len() - self.view.len()),
        }
    }

    /// Fila de pantalla de la primera fila visible desde `view_row` en adelante.
    pub fn screen_at_or_after(&self, view_row: usize) -> Option<usize> {
        (view_row..self.screen.len().max(view_row + 1)).find_map(|r| self.screen_of(r))
    }

    /// Fila de pantalla de la última fila visible hasta `view_row`.
    pub fn screen_at_or_before(&self, view_row: usize) -> Option<usize> {
        (0..=view_row).rev().find_map(|r| self.screen_of(r))
    }
}

/// El mapa de filas para una vista de `rows` filas que empieza en la línea
/// absoluta `top`.
pub fn row_map(collapses: &[Collapse], top: u64, rows: usize) -> RowMap {
    let mut hidden = vec![false; rows];
    let mut summaries = Vec::new();
    for c in collapses {
        if c.last <= c.first {
            continue;
        }
        let mut count = 0;
        for line in c.first + 1..=c.last {
            if line >= top && ((line - top) as usize) < rows {
                hidden[(line - top) as usize] = true;
            }
            count += 1;
        }
        if c.first >= top && ((c.first - top) as usize) < rows {
            summaries.push(((c.first - top) as usize, count));
        }
    }
    if summaries.is_empty() && !hidden.iter().any(|h| *h) {
        return RowMap::default();
    }
    let mut screen = Vec::with_capacity(rows);
    let mut view = Vec::new();
    for (r, h) in hidden.iter().enumerate() {
        if *h {
            screen.push(None);
        } else {
            screen.push(Some(view.len()));
            view.push(r);
        }
    }
    RowMap {
        screen,
        view,
        summaries,
    }
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

    #[test]
    fn collapsing_hides_rows_and_moves_the_rest_up() {
        // Vista de 10 filas desde la línea 100; salida en 102..=106 colapsada.
        let map = row_map(
            &[Collapse {
                first: 102,
                last: 106,
            }],
            100,
            10,
        );
        assert!(!map.is_identity());
        assert_eq!(map.summaries, vec![(2, 4)]);
        // Filas 0..=2 igual; 3..=6 ocultas; la 7 sube a la pantalla 3.
        assert_eq!(map.screen_of(2), Some(2));
        assert_eq!(map.screen_of(3), None);
        assert_eq!(map.screen_of(6), None);
        assert_eq!(map.screen_of(7), Some(3));
        assert_eq!(map.screen_of(9), Some(5));
        // Al revés: del ratón a la vista.
        assert_eq!(map.view_of(2), 2);
        assert_eq!(map.view_of(3), 7);
        assert_eq!(map.view_of(5), 9);
        // Bajo lo dibujado, sigue la cuenta.
        assert_eq!(map.view_of(6), 10);
        assert_eq!(map.screen_at_or_after(3), Some(3));
        assert_eq!(map.screen_at_or_before(5), Some(2));
    }

    #[test]
    fn what_a_block_collapses() {
        // Salida de la 11 a la 14; la marca D en la 15, columna 0.
        assert_eq!(
            collapse_of(11, Some((15, 0))),
            Some(Collapse {
                first: 11,
                last: 14
            })
        );
        // D a mitad de línea: esa línea cuenta.
        assert_eq!(collapse_of(11, Some((15, 3))).map(|c| c.last), Some(15));
        // Una línea o menos, o sin terminar: nada que colapsar.
        assert_eq!(collapse_of(11, Some((12, 0))), None);
        assert_eq!(collapse_of(11, None), None);
        assert_eq!(collapse_of(0, Some((0, 0))), None);
    }

    #[test]
    fn collapses_outside_the_view_or_too_short_change_nothing() {
        assert!(row_map(&[], 0, 5).is_identity());
        assert!(row_map(&[Collapse { first: 3, last: 3 }], 0, 5).is_identity());
        assert!(
            row_map(
                &[Collapse {
                    first: 50,
                    last: 60
                }],
                0,
                5
            )
            .is_identity()
        );
        let id = RowMap::default();
        assert_eq!((id.screen_of(4), id.view_of(4)), (Some(4), 4));
        // Una salida que empieza por encima de la vista oculta sus filas visibles.
        let map = row_map(
            &[Collapse {
                first: 95,
                last: 102,
            }],
            100,
            6,
        );
        assert_eq!(map.screen_of(0), None);
        assert_eq!(map.screen_of(2), None);
        assert_eq!(map.screen_of(3), Some(0));
        assert!(map.summaries.is_empty());
    }

    #[test]
    fn failed_only_for_cargo_pytest_jest_and_go() {
        let cargo = "running 2 tests\ntest a ... FAILED\n\nfailures:\n\n---- a stdout ----\npanic\n\nfailures:\n    tests::a\n    tests::b_c\n\ntest result: FAILED";
        assert_eq!(
            failed_only_command("cargo test -p core", cargo).as_deref(),
            Some("cargo test -p core tests::a tests::b_c")
        );
        // Los nombres van antes de `--`.
        assert_eq!(
            failed_only_command("cargo test -- --nocapture", cargo).as_deref(),
            Some("cargo test tests::a tests::b_c -- --nocapture")
        );
        assert_eq!(
            failed_only_command("RUST_LOG=x cargo test", cargo).as_deref(),
            Some("RUST_LOG=x cargo test tests::a tests::b_c")
        );
        assert_eq!(
            failed_only_command("pytest tests/", "").as_deref(),
            Some("pytest tests/ --lf")
        );
        assert_eq!(failed_only_command("pytest --lf", ""), None);
        assert_eq!(
            failed_only_command("npx jest src", "").as_deref(),
            Some("npx jest src --onlyFailures")
        );
        let go = "--- FAIL: TestA (0.00s)\n    --- FAIL: TestA/sub (0.00s)\n--- FAIL: TestB (0.01s)\nFAIL";
        assert_eq!(
            failed_only_command("go test ./...", go).as_deref(),
            Some("go test ./... -run '^(TestA|TestB)$'")
        );
        assert_eq!(failed_only_command("ls", ""), None);
        assert_eq!(failed_only_command("cargo test", "ok"), None);
    }

    #[test]
    fn failed_only_refuses_hostile_test_names() {
        // Una salida con nombres que inyectarían órdenes no produce nada.
        for bad in [
            "a; rm -rf ~",
            "$(id)",
            "a`b`",
            "x y z",
            "--flag",
            "a&b",
            "a|b",
        ] {
            let out = format!("failures:\n    {bad}\n\n");
            assert_eq!(failed_only_command("cargo test", &out), None, "{bad}");
        }
        let go = "--- FAIL: Test$(x) (0s)";
        assert_eq!(failed_only_command("go test", go), None);
    }
}
