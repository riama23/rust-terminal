//! Parser de diffs unificados (`git diff`) para la pantalla de Review.
//!
//! Puro: recibe el texto y devuelve archivos, bloques y líneas con su número
//! antiguo y nuevo. Tolera lo que git puede emitir (renombrados, binarios,
//! archivos nuevos o borrados, rutas entrecomilladas, «\ No newline»).

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FileStatus {
    Added,
    Modified,
    Deleted,
    Renamed,
}

impl FileStatus {
    /// Etiqueta de una letra de la lista de archivos (A/M/D/R).
    pub fn tag(self) -> &'static str {
        match self {
            FileStatus::Added => "A",
            FileStatus::Modified => "M",
            FileStatus::Deleted => "D",
            FileStatus::Renamed => "R",
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum LineKind {
    Context,
    Add,
    Del,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Line {
    pub kind: LineKind,
    /// Número en el archivo antiguo (no hay en las añadidas).
    pub old: Option<u32>,
    /// Número en el archivo nuevo (no hay en las borradas).
    pub new: Option<u32>,
    pub text: String,
}

impl Line {
    /// El signo que se pinta delante: nunca solo color.
    pub fn sign(&self) -> char {
        match self.kind {
            LineKind::Context => ' ',
            LineKind::Add => '+',
            LineKind::Del => '-',
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Hunk {
    /// La línea `@@ -a,b +c,d @@ …` tal cual.
    pub header: String,
    pub lines: Vec<Line>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct DiffFile {
    /// Ruta actual (o la antigua si se borró).
    pub path: String,
    pub old_path: Option<String>,
    pub status: FileStatus,
    pub binary: bool,
    pub hunks: Vec<Hunk>,
    pub added: usize,
    pub removed: usize,
}

/// Quita las comillas de estilo C que git pone a rutas con caracteres raros.
fn unquote(path: &str) -> String {
    let p = path.trim();
    let Some(inner) = p.strip_prefix('"').and_then(|s| s.strip_suffix('"')) else {
        return p.to_owned();
    };
    let mut out = Vec::new();
    let b = inner.as_bytes();
    let mut i = 0;
    while i < b.len() {
        if b[i] == b'\\' && i + 1 < b.len() {
            i += 1;
            match b[i] {
                b'n' => out.push(b'\n'),
                b't' => out.push(b'\t'),
                b'"' => out.push(b'"'),
                b'\\' => out.push(b'\\'),
                d if d.is_ascii_digit() => {
                    // Escape octal de tres cifras (bytes UTF-8).
                    let end = (i + 3).min(b.len());
                    let oct = std::str::from_utf8(&b[i..end]).unwrap_or("0");
                    out.push(u8::from_str_radix(oct, 8).unwrap_or(b'?'));
                    i = end - 1;
                }
                other => out.push(other),
            }
        } else {
            out.push(b[i]);
        }
        i += 1;
    }
    String::from_utf8_lossy(&out).into_owned()
}

/// `a/ruta` o `b/ruta` → `ruta`; `/dev/null` → `None`.
fn side_path(raw: &str) -> Option<String> {
    let p = unquote(raw.split('\t').next().unwrap_or(raw));
    if p == "/dev/null" {
        return None;
    }
    Some(
        p.strip_prefix("a/")
            .or_else(|| p.strip_prefix("b/"))
            .unwrap_or(&p)
            .to_owned(),
    )
}

/// `@@ -10,3 +12,4 @@` → (10, 12).
fn hunk_starts(header: &str) -> Option<(u32, u32)> {
    let rest = header.strip_prefix("@@ -")?;
    let (old, rest) = rest.split_once(' ')?;
    let new = rest.strip_prefix('+')?.split([' ', ',']).next()?;
    let old = old.split(',').next()?;
    Some((old.parse().ok()?, new.parse().ok()?))
}

pub fn parse(diff: &str) -> Vec<DiffFile> {
    let mut files: Vec<DiffFile> = Vec::new();
    let mut hunk: Option<(Hunk, u32, u32)> = None;

    let flush = |files: &mut Vec<DiffFile>, hunk: &mut Option<(Hunk, u32, u32)>| {
        if let (Some((h, _, _)), Some(f)) = (hunk.take(), files.last_mut()) {
            f.hunks.push(h);
        }
    };

    for line in diff.lines() {
        if line.starts_with("diff --git ") {
            flush(&mut files, &mut hunk);
            // La ruta definitiva sale de `---`/`+++`; esto es un valor de reserva.
            let guess = line
                .rsplit_once(" b/")
                .map_or_else(String::new, |(_, p)| unquote(p));
            files.push(DiffFile {
                path: guess,
                old_path: None,
                status: FileStatus::Modified,
                binary: false,
                hunks: Vec::new(),
                added: 0,
                removed: 0,
            });
            continue;
        }
        let Some(file) = files.last_mut() else {
            continue;
        };
        if let Some((h, old, new)) = hunk.as_mut() {
            // Dentro de un bloque.
            let (kind, text) = match line.as_bytes().first() {
                Some(b'+') => (LineKind::Add, &line[1..]),
                Some(b'-') => (LineKind::Del, &line[1..]),
                Some(b' ') => (LineKind::Context, &line[1..]),
                Some(b'\\') => continue, // «\ No newline at end of file»
                None => (LineKind::Context, ""),
                _ => {
                    flush(&mut files, &mut hunk);
                    // Cae a las cabeceras del siguiente bloque o archivo.
                    if let Some((o, n)) = hunk_starts(line) {
                        hunk = Some((
                            Hunk {
                                header: line.to_owned(),
                                lines: Vec::new(),
                            },
                            o,
                            n,
                        ));
                    }
                    continue;
                }
            };
            let (o, n) = match kind {
                LineKind::Context => (Some(*old), Some(*new)),
                LineKind::Add => (None, Some(*new)),
                LineKind::Del => (Some(*old), None),
            };
            match kind {
                LineKind::Context => {
                    *old += 1;
                    *new += 1;
                }
                LineKind::Add => {
                    *new += 1;
                    file.added += 1;
                }
                LineKind::Del => {
                    *old += 1;
                    file.removed += 1;
                }
            }
            h.lines.push(Line {
                kind,
                old: o,
                new: n,
                text: text.to_owned(),
            });
            continue;
        }
        if let Some((o, n)) = hunk_starts(line) {
            hunk = Some((
                Hunk {
                    header: line.to_owned(),
                    lines: Vec::new(),
                },
                o,
                n,
            ));
        } else if line.starts_with("new file mode") {
            file.status = FileStatus::Added;
        } else if line.starts_with("deleted file mode") {
            file.status = FileStatus::Deleted;
        } else if let Some(p) = line.strip_prefix("rename from ") {
            file.status = FileStatus::Renamed;
            file.old_path = Some(unquote(p));
        } else if let Some(p) = line.strip_prefix("rename to ") {
            file.path = unquote(p);
        } else if let Some(p) = line.strip_prefix("--- ") {
            match side_path(p) {
                Some(path) => {
                    if file.status == FileStatus::Deleted {
                        file.path = path;
                    }
                }
                None => file.status = FileStatus::Added,
            }
        } else if let Some(p) = line.strip_prefix("+++ ") {
            match side_path(p) {
                Some(path) => file.path = path,
                None => file.status = FileStatus::Deleted,
            }
        } else if line.starts_with("Binary files ") || line.starts_with("GIT binary patch") {
            file.binary = true;
        }
    }
    flush(&mut files, &mut hunk);
    files
}

#[cfg(test)]
mod tests {
    use super::*;

    const SIMPLE: &str = "\
diff --git a/src/lib.rs b/src/lib.rs
index 1111111..2222222 100644
--- a/src/lib.rs
+++ b/src/lib.rs
@@ -1,4 +1,5 @@
 fn a() {}
-fn b() {}
+fn b() { 1 }
+fn c() {}
 fn d() {}
@@ -20,2 +21,2 @@ impl X {
 ctx
-viejo
+nuevo
";

    #[test]
    fn un_archivo_modificado_con_dos_bloques() {
        let files = parse(SIMPLE);
        assert_eq!(files.len(), 1);
        let f = &files[0];
        assert_eq!(
            (f.path.as_str(), f.status),
            ("src/lib.rs", FileStatus::Modified)
        );
        assert_eq!((f.added, f.removed), (3, 2));
        assert_eq!(f.hunks.len(), 2);
        assert_eq!(f.hunks[1].header, "@@ -20,2 +21,2 @@ impl X {");
    }

    #[test]
    fn los_numeros_de_linea_cuentan_bien() {
        let f = &parse(SIMPLE)[0];
        let l = &f.hunks[0].lines;
        assert_eq!((l[0].old, l[0].new), (Some(1), Some(1)));
        assert_eq!(
            (l[1].kind, l[1].old, l[1].new),
            (LineKind::Del, Some(2), None)
        );
        assert_eq!(
            (l[2].kind, l[2].old, l[2].new),
            (LineKind::Add, None, Some(2))
        );
        assert_eq!((l[3].old, l[3].new), (None, Some(3)));
        assert_eq!((l[4].old, l[4].new), (Some(3), Some(4)));
        let l2 = &f.hunks[1].lines;
        assert_eq!((l2[0].old, l2[0].new), (Some(20), Some(21)));
        assert_eq!(l2[2].text, "nuevo");
    }

    #[test]
    fn cada_linea_lleva_su_signo() {
        let f = &parse(SIMPLE)[0];
        let signs: String = f.hunks[0].lines.iter().map(Line::sign).collect();
        assert_eq!(signs, " -++ ");
    }

    #[test]
    fn archivo_nuevo_y_borrado() {
        let d = "\
diff --git a/nuevo.txt b/nuevo.txt
new file mode 100644
index 0000000..e69de29
--- /dev/null
+++ b/nuevo.txt
@@ -0,0 +1,2 @@
+uno
+dos
diff --git a/viejo.txt b/viejo.txt
deleted file mode 100644
index e69de29..0000000
--- a/viejo.txt
+++ /dev/null
@@ -1 +0,0 @@
-adiós
";
        let files = parse(d);
        assert_eq!(files.len(), 2);
        assert_eq!(
            (files[0].path.as_str(), files[0].status, files[0].added),
            ("nuevo.txt", FileStatus::Added, 2)
        );
        assert_eq!(files[0].hunks[0].lines[1].new, Some(2));
        assert_eq!(
            (files[1].path.as_str(), files[1].status, files[1].removed),
            ("viejo.txt", FileStatus::Deleted, 1)
        );
    }

    #[test]
    fn renombrado_con_cambios() {
        let d = "\
diff --git a/a.rs b/b.rs
similarity index 90%
rename from a.rs
rename to b.rs
index 1..2 100644
--- a/a.rs
+++ b/b.rs
@@ -1 +1 @@
-x
+y
";
        let f = &parse(d)[0];
        assert_eq!(
            (f.status, f.path.as_str(), f.old_path.as_deref()),
            (FileStatus::Renamed, "b.rs", Some("a.rs"))
        );
    }

    #[test]
    fn renombrado_puro_sin_bloques() {
        let d = "diff --git a/a b/b\nsimilarity index 100%\nrename from a\nrename to b\n";
        let f = &parse(d)[0];
        assert_eq!(
            (f.status, f.path.as_str(), f.hunks.len()),
            (FileStatus::Renamed, "b", 0)
        );
    }

    #[test]
    fn binarios() {
        let d = "diff --git a/img.png b/img.png\nindex 1..2 100644\nBinary files a/img.png and b/img.png differ\n";
        let f = &parse(d)[0];
        assert!(f.binary && f.hunks.is_empty());
        assert_eq!(f.path, "img.png");
    }

    #[test]
    fn rutas_con_espacios_y_acentos_entrecomilladas() {
        let d = "\
diff --git \"a/mi carpeta/ni\\303\\261o.txt\" \"b/mi carpeta/ni\\303\\261o.txt\"
index 1..2 100644
--- \"a/mi carpeta/ni\\303\\261o.txt\"
+++ \"b/mi carpeta/ni\\303\\261o.txt\"
@@ -1 +1 @@
-a
+b
";
        assert_eq!(parse(d)[0].path, "mi carpeta/niño.txt");
    }

    #[test]
    fn la_marca_de_sin_salto_final_no_es_una_linea() {
        let d = "\
diff --git a/f b/f
--- a/f
+++ b/f
@@ -1 +1 @@
-a
\\ No newline at end of file
+b
\\ No newline at end of file
";
        let f = &parse(d)[0];
        assert_eq!(f.hunks[0].lines.len(), 2);
        assert_eq!((f.added, f.removed), (1, 1));
    }

    #[test]
    fn una_linea_de_contexto_vacia_sin_espacio_no_rompe_la_numeracion() {
        // Algunas herramientas recortan el espacio de las líneas de contexto vacías.
        let d = "diff --git a/f b/f\n--- a/f\n+++ b/f\n@@ -1,3 +1,3 @@\n a\n\n-c\n+d\n";
        let l = &parse(d)[0].hunks[0].lines;
        assert_eq!(l[1].kind, LineKind::Context);
        assert_eq!((l[2].old, l[3].new), (Some(3), Some(3)));
    }

    #[test]
    fn entradas_raras_no_entran_en_panico() {
        for d in [
            "",
            "basura\nsin formato",
            "@@ -1 +1 @@\n+x",
            "diff --git a/x b/x\n@@ rota @@\n+y",
            "--- a/x\n+++ b/x",
        ] {
            let _ = parse(d);
        }
        assert!(parse("").is_empty());
    }
}
