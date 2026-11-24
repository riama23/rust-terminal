//! Árbol de carpetas de la lista de archivos de Review (issue #12). Lógica pura,
//! sin AppKit: convierte las rutas del diff en filas visibles.
//!
//! Las carpetas con un único hijo que también es carpeta se compactan en una
//! sola fila (`crates/app/src`). Primero van las carpetas y después los
//! archivos, cada grupo por orden alfabético. Una carpeta plegada oculta todo
//! lo que contiene, y sus contadores de comentarios suman los de sus archivos.

use std::collections::{BTreeMap, HashSet};

use crate::agent::diff::FileStatus;

/// Un archivo del diff tal y como lo necesita el árbol.
#[derive(Debug, Clone, Copy)]
pub struct TreeFile<'a> {
    pub path: &'a str,
    pub status: FileStatus,
    pub comments: usize,
}

#[derive(Debug, Clone, PartialEq)]
pub enum TreeRow {
    Dir {
        /// Ruta completa (con las carpetas compactadas); es la clave de plegado.
        path: String,
        name: String,
        depth: usize,
        collapsed: bool,
        comments: usize,
    },
    File {
        /// Índice en la lista de archivos del diff (no la posición de la fila).
        index: usize,
        name: String,
        depth: usize,
        status: FileStatus,
        comments: usize,
        selected: bool,
    },
}

#[derive(Default)]
struct Node {
    dirs: BTreeMap<String, Node>,
    files: BTreeMap<String, usize>,
}

impl Node {
    fn insert(&mut self, parts: &[&str], index: usize) {
        match parts {
            [] => {}
            [name] => {
                self.files.insert((*name).to_string(), index);
            }
            [dir, rest @ ..] => self
                .dirs
                .entry((*dir).to_string())
                .or_default()
                .insert(rest, index),
        }
    }

    fn comments(&self, files: &[TreeFile]) -> usize {
        let own: usize = self.files.values().map(|&i| files[i].comments).sum();
        own + self.dirs.values().map(|d| d.comments(files)).sum::<usize>()
    }
}

/// Filas visibles del árbol. `selected` es el índice del archivo seleccionado.
pub fn build(files: &[TreeFile], collapsed: &HashSet<String>, selected: usize) -> Vec<TreeRow> {
    let mut root = Node::default();
    for (i, f) in files.iter().enumerate() {
        let parts: Vec<&str> = f.path.split('/').filter(|p| !p.is_empty()).collect();
        root.insert(&parts, i);
    }
    let mut rows = Vec::new();
    emit(&root, "", 0, files, collapsed, selected, &mut rows);
    rows
}

fn emit(
    node: &Node,
    prefix: &str,
    depth: usize,
    files: &[TreeFile],
    collapsed: &HashSet<String>,
    selected: usize,
    rows: &mut Vec<TreeRow>,
) {
    for (name, child) in &node.dirs {
        // Compacta mientras la carpeta solo contenga otra carpeta.
        let (mut label, mut path, mut cur) = (name.clone(), join(prefix, name), child);
        while cur.files.is_empty() && cur.dirs.len() == 1 {
            let Some((n, next)) = cur.dirs.iter().next() else {
                break;
            };
            label = format!("{label}/{n}");
            path = join(&path, n);
            cur = next;
        }
        let is_collapsed = collapsed.contains(&path);
        rows.push(TreeRow::Dir {
            path: path.clone(),
            name: label,
            depth,
            collapsed: is_collapsed,
            comments: cur.comments(files),
        });
        if !is_collapsed {
            emit(cur, &path, depth + 1, files, collapsed, selected, rows);
        }
    }
    for (name, &index) in &node.files {
        rows.push(TreeRow::File {
            index,
            name: name.clone(),
            depth,
            status: files[index].status,
            comments: files[index].comments,
            selected: index == selected,
        });
    }
}

fn join(prefix: &str, name: &str) -> String {
    if prefix.is_empty() {
        name.to_string()
    } else {
        format!("{prefix}/{name}")
    }
}

/// Despliega las carpetas que contienen `path` (al seleccionar un archivo).
pub fn reveal(collapsed: &mut HashSet<String>, path: &str) {
    collapsed.retain(|dir| !path.starts_with(&format!("{dir}/")));
}

/// Alterna el plegado de una carpeta.
pub fn toggle(collapsed: &mut HashSet<String>, dir: &str) {
    if !collapsed.remove(dir) {
        collapsed.insert(dir.to_string());
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn f(path: &str, comments: usize) -> TreeFile<'_> {
        TreeFile {
            path,
            status: FileStatus::Modified,
            comments,
        }
    }

    fn names(rows: &[TreeRow]) -> Vec<String> {
        rows.iter()
            .map(|r| match r {
                TreeRow::Dir { name, depth, .. } => format!("{}{name}/", " ".repeat(*depth)),
                TreeRow::File { name, depth, .. } => format!("{}{name}", " ".repeat(*depth)),
            })
            .collect()
    }

    #[test]
    fn carpetas_primero_y_cada_grupo_ordenado() {
        let files = [
            f("z.rs", 0),
            f("src/b.rs", 0),
            f("a.rs", 0),
            f("src/a.rs", 0),
        ];
        let rows = build(&files, &HashSet::new(), 0);
        assert_eq!(names(&rows), ["src/", " a.rs", " b.rs", "a.rs", "z.rs"]);
    }

    #[test]
    fn compacta_las_carpetas_con_un_unico_hijo_carpeta() {
        let files = [f("crates/app/src/a.rs", 0), f("crates/app/src/b.rs", 0)];
        let rows = build(&files, &HashSet::new(), 0);
        assert_eq!(names(&rows), ["crates/app/src/", " a.rs", " b.rs"]);
    }

    #[test]
    fn no_compacta_si_la_carpeta_tiene_archivos_o_varias_carpetas() {
        let files = [
            f("a/x.rs", 0),
            f("a/b/y.rs", 0),
            f("c/d/z.rs", 0),
            f("c/e/w.rs", 0),
        ];
        let rows = build(&files, &HashSet::new(), 0);
        assert_eq!(
            names(&rows),
            [
                "a/", " b/", "  y.rs", " x.rs", "c/", " d/", "  z.rs", " e/", "  w.rs"
            ]
        );
    }

    #[test]
    fn una_carpeta_plegada_oculta_su_contenido() {
        let files = [f("src/a.rs", 0), f("src/sub/b.rs", 0), f("README.md", 0)];
        let collapsed: HashSet<String> = ["src".to_string()].into();
        let rows = build(&files, &collapsed, 0);
        assert_eq!(names(&rows), ["src/", "README.md"]);
        assert!(matches!(
            rows[0],
            TreeRow::Dir {
                collapsed: true,
                ..
            }
        ));
    }

    #[test]
    fn la_clave_de_plegado_de_una_carpeta_compactada_es_su_ruta_completa() {
        let files = [f("crates/app/a.rs", 0)];
        let collapsed: HashSet<String> = ["crates/app".to_string()].into();
        let rows = build(&files, &collapsed, 0);
        assert_eq!(names(&rows), ["crates/app/"]);
        assert!(matches!(&rows[0], TreeRow::Dir { path, .. } if path == "crates/app"));
    }

    #[test]
    fn el_contador_de_una_carpeta_suma_sus_descendientes_aunque_este_plegada() {
        let files = [f("src/a.rs", 2), f("src/sub/b.rs", 3), f("c.rs", 5)];
        let collapsed: HashSet<String> = ["src".to_string()].into();
        let rows = build(&files, &collapsed, 0);
        assert!(matches!(rows[0], TreeRow::Dir { comments: 5, .. }));
    }

    #[test]
    fn las_filas_de_archivo_llevan_el_indice_del_diff_y_la_seleccion() {
        let files = [f("b/x.rs", 0), f("a.rs", 0)];
        let rows = build(&files, &HashSet::new(), 1);
        let sel: Vec<(usize, bool)> = rows
            .iter()
            .filter_map(|r| match r {
                TreeRow::File {
                    index, selected, ..
                } => Some((*index, *selected)),
                TreeRow::Dir { .. } => None,
            })
            .collect();
        assert_eq!(sel, [(0, false), (1, true)]);
    }

    #[test]
    fn ignora_barras_sobrantes_y_no_falla_sin_archivos() {
        assert!(build(&[], &HashSet::new(), 0).is_empty());
        let files = [f("/a//b.rs", 0)];
        assert_eq!(names(&build(&files, &HashSet::new(), 0)), ["a/", " b.rs"]);
    }

    #[test]
    fn reveal_despliega_solo_las_carpetas_que_contienen_el_archivo() {
        let mut collapsed: HashSet<String> = [
            "src".to_string(),
            "src/sub".to_string(),
            "docs".to_string(),
            "src2".to_string(),
        ]
        .into();
        reveal(&mut collapsed, "src/sub/b.rs");
        let mut left: Vec<_> = collapsed.into_iter().collect();
        left.sort();
        assert_eq!(left, ["docs", "src2"]);
    }

    #[test]
    fn toggle_alterna() {
        let mut c = HashSet::new();
        toggle(&mut c, "src");
        assert!(c.contains("src"));
        toggle(&mut c, "src");
        assert!(c.is_empty());
    }
}
