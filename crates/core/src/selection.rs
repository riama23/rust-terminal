//! Selección de texto y extracción de texto de la cuadrícula.
//!
//! Las posiciones son [`Point`] con línea absoluta, así que una selección
//! sigue siendo válida aunque la pantalla haga scroll.

use crate::grid::{Cell, Flags, Grid, Line, Point};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum SelectionKind {
    /// Arrastrar: de carácter a carácter.
    Simple,
    /// Doble clic: palabras completas.
    Word,
    /// Triple clic: líneas lógicas completas.
    Line,
    /// Option+arrastrar: rectángulo.
    Block,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Selection {
    pub kind: SelectionKind,
    pub anchor: Point,
    pub head: Point,
}

/// Rango normalizado de una selección, con ambos extremos incluidos.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct SelectionRange {
    pub start: Point,
    pub end: Point,
    pub block: bool,
}

impl SelectionRange {
    /// Columnas seleccionadas `[start, end]` de una línea, si hay alguna.
    pub fn cols_on(&self, line: Line, cols: usize) -> Option<(usize, usize)> {
        if line < self.start.line || line > self.end.line || cols == 0 {
            return None;
        }
        if self.block {
            let (a, b) = (
                self.start.col.min(self.end.col),
                self.start.col.max(self.end.col),
            );
            return Some((a.min(cols - 1), b.min(cols - 1)));
        }
        let start = if line == self.start.line {
            self.start.col
        } else {
            0
        };
        let end = if line == self.end.line {
            self.end.col
        } else {
            cols - 1
        };
        (start <= end).then_some((start.min(cols - 1), end.min(cols - 1)))
    }
}

pub fn point(line: Line, col: usize) -> Point {
    Point {
        line,
        col,
        pending: false,
    }
}

/// Caracteres que separan palabras en el doble clic.
fn is_separator(c: char) -> bool {
    c.is_whitespace() || "'\"()[]{}<>|│`,;".contains(c)
}

fn cell_at(grid: &Grid, line: Line, col: usize) -> Cell {
    grid.line(line)
        .and_then(|r| r.cells.get(col))
        .copied()
        .unwrap_or_default()
}

impl Selection {
    pub fn new(kind: SelectionKind, at: Point) -> Selection {
        Selection {
            kind,
            anchor: at,
            head: at,
        }
    }

    pub fn range(&self, grid: &Grid) -> SelectionRange {
        let (mut start, mut end) =
            if (self.anchor.line, self.anchor.col) <= (self.head.line, self.head.col) {
                (self.anchor, self.head)
            } else {
                (self.head, self.anchor)
            };
        let cols = grid.cols();
        match self.kind {
            SelectionKind::Simple | SelectionKind::Block => {}
            SelectionKind::Word => {
                start.col = word_start(grid, start);
                end.col = word_end(grid, end, cols);
            }
            SelectionKind::Line => {
                start = point(logical_line_start(grid, start.line), 0);
                end = point(logical_line_end(grid, end.line), cols - 1);
            }
        }
        // Un carácter ancho se selecciona entero.
        if cell_at(grid, start.line, start.col)
            .flags
            .contains(Flags::WIDE_SPACER)
            && start.col > 0
        {
            start.col -= 1;
        }
        if cell_at(grid, end.line, end.col).flags.contains(Flags::WIDE) {
            end.col += 1;
        }
        SelectionRange {
            start,
            end,
            block: self.kind == SelectionKind::Block,
        }
    }
}

fn word_start(grid: &Grid, p: Point) -> usize {
    let mut col = p.col;
    if is_separator(grid.base_char(cell_at(grid, p.line, col).ch)) {
        return col;
    }
    while col > 0 {
        let prev = cell_at(grid, p.line, col - 1);
        if !prev.flags.contains(Flags::WIDE_SPACER) && is_separator(grid.base_char(prev.ch)) {
            break;
        }
        col -= 1;
    }
    col
}

fn word_end(grid: &Grid, p: Point, cols: usize) -> usize {
    let mut col = p.col;
    if is_separator(grid.base_char(cell_at(grid, p.line, col).ch)) {
        return col;
    }
    while col + 1 < cols {
        let next = cell_at(grid, p.line, col + 1);
        if !next.flags.contains(Flags::WIDE_SPACER) && is_separator(grid.base_char(next.ch)) {
            break;
        }
        col += 1;
    }
    col
}

fn logical_line_start(grid: &Grid, mut line: Line) -> Line {
    while line > grid.first_line() && grid.line(line - 1).is_some_and(|r| r.wrapped) {
        line -= 1;
    }
    line
}

fn logical_line_end(grid: &Grid, mut line: Line) -> Line {
    while grid.line(line).is_some_and(|r| r.wrapped) && grid.line(line + 1).is_some() {
        line += 1;
    }
    line
}

/// Texto de un rango (extremos incluidos). Une las filas con salto suave sin
/// salto de línea, quita los espacios finales y no duplica caracteres anchos.
pub fn text_in_range(grid: &Grid, range: SelectionRange) -> String {
    let cols = grid.cols();
    let mut out = String::new();
    let first = range.start.line.max(grid.first_line());
    for line in first..=range.end.line {
        let Some(row) = grid.line(line) else { break };
        let Some((a, b)) = range.cols_on(line, cols.max(row.cells.len())) else {
            continue;
        };
        let mut text = String::new();
        for c in row
            .cells
            .iter()
            .take(b + 1)
            .skip(a)
            .filter(|c| !c.flags.contains(Flags::WIDE_SPACER))
        {
            text.push_str(&grid.char_text(c.ch));
        }
        let continues = row.wrapped && !range.block && line != range.end.line;
        if !continues {
            text.truncate(text.trim_end_matches(' ').len());
        }
        out.push_str(&text);
        if line != range.end.line && !continues {
            out.push('\n');
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Term;

    fn term(cols: usize, rows: usize, input: &str) -> Term {
        let mut t = Term::new(cols, rows);
        t.advance(input.as_bytes());
        t
    }

    fn select(t: &Term, kind: SelectionKind, a: (Line, usize), b: (Line, usize)) -> String {
        let mut s = Selection::new(kind, point(a.0, a.1));
        s.head = point(b.0, b.1);
        text_in_range(t.grid(), s.range(t.grid()))
    }

    #[test]
    fn simple_selection_across_lines() {
        let t = term(10, 3, "hola mundo\r\nadiós");
        assert_eq!(
            select(&t, SelectionKind::Simple, (0, 5), (1, 2)),
            "mundo\nadi"
        );
        // Al revés da lo mismo.
        assert_eq!(
            select(&t, SelectionKind::Simple, (1, 2), (0, 5)),
            "mundo\nadi"
        );
    }

    #[test]
    fn wrapped_lines_are_joined_without_newline() {
        let t = term(4, 3, "abcdefg");
        assert_eq!(select(&t, SelectionKind::Simple, (0, 0), (1, 3)), "abcdefg");
    }

    #[test]
    fn word_selection() {
        let t = term(30, 1, "ls -la /usr/local/bin (x)");
        assert_eq!(
            select(&t, SelectionKind::Word, (0, 9), (0, 9)),
            "/usr/local/bin"
        );
        assert_eq!(select(&t, SelectionKind::Word, (0, 23), (0, 23)), "x");
    }

    #[test]
    fn line_selection_takes_whole_logical_line() {
        let t = term(4, 3, "abcdefg\r\nzz");
        assert_eq!(select(&t, SelectionKind::Line, (1, 1), (1, 1)), "abcdefg");
    }

    #[test]
    fn block_selection() {
        let t = term(10, 3, "abcdef\r\nghijkl\r\nmnopqr");
        assert_eq!(
            select(&t, SelectionKind::Block, (0, 1), (2, 3)),
            "bcd\nhij\nnop"
        );
    }

    #[test]
    fn wide_chars_are_selected_whole_and_once() {
        let t = term(10, 1, "a日本b");
        // Empezar en la segunda mitad de 日 lo incluye entero.
        assert_eq!(select(&t, SelectionKind::Simple, (0, 2), (0, 3)), "日本");
    }

    #[test]
    fn selection_reaches_into_history() {
        let t = term(10, 2, "uno\r\ndos\r\ntres");
        let start = t.grid().first_line();
        assert_eq!(
            select(&t, SelectionKind::Simple, (start, 0), (2, 3)),
            "uno\ndos\ntres"
        );
    }
}
