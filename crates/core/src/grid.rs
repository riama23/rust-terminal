//! Cuadrícula de celdas: la pantalla visible y su historial (scrollback).
//!
//! Las líneas se identifican con un número absoluto ([`Line`]) que no cambia
//! al hacer scroll: la más antigua guardada es [`Grid::first_line`] y la
//! primera fila de la pantalla es [`Grid::screen_start`]. Al descartar líneas
//! antiguas por el límite del historial, `first_line` avanza.

use std::borrow::Cow;
use std::collections::{HashMap, VecDeque};

/// Primer código del área de uso privado suplementaria B: las celdas con un
/// grupo de caracteres (base + combinantes) guardan aquí su índice.
const CLUSTER_BASE: u32 = 0x10_0000;
/// Cuántos grupos distintos caben (hasta U+10FFFD).
const CLUSTER_MAX: usize = 0xFFFE;

/// Número absoluto de línea (historial + pantalla).
pub type Line = u64;

/// Color de primer plano o de fondo de una celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum Color {
    /// El color por defecto del tema (lo decide el renderizador).
    #[default]
    Default,
    /// Índice de la paleta de 256 colores (0–15 son los ANSI).
    Indexed(u8),
    Rgb(u8, u8, u8),
}

/// Atributos de una celda, como bitset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u8);

impl Flags {
    pub const BOLD: Flags = Flags(1 << 0);
    pub const DIM: Flags = Flags(1 << 1);
    pub const ITALIC: Flags = Flags(1 << 2);
    pub const UNDERLINE: Flags = Flags(1 << 3);
    pub const INVERSE: Flags = Flags(1 << 4);
    pub const STRIKEOUT: Flags = Flags(1 << 5);
    /// Primera mitad de un carácter de doble ancho.
    pub const WIDE: Flags = Flags(1 << 6);
    /// Segunda mitad de un carácter de doble ancho: no se dibuja por sí misma.
    /// En la última columna de una fila con salto suave, y sin `WIDE` delante,
    /// marca el hueco que dejó un carácter ancho que no cabía.
    pub const WIDE_SPACER: Flags = Flags(1 << 7);

    pub const fn empty() -> Flags {
        Flags(0)
    }

    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
    }

    pub const fn intersects(self, other: Flags) -> bool {
        self.0 & other.0 != 0
    }

    pub fn insert(&mut self, other: Flags) {
        self.0 |= other.0;
    }

    pub fn remove(&mut self, other: Flags) {
        self.0 &= !other.0;
    }
}

impl std::ops::BitOr for Flags {
    type Output = Flags;

    fn bitor(self, rhs: Flags) -> Flags {
        Flags(self.0 | rhs.0)
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Cell {
    pub ch: char,
    pub fg: Color,
    pub bg: Color,
    pub flags: Flags,
    /// Enlace OSC 8 (índice en la tabla de `Term`; 0 = sin enlace). Ocupa el
    /// relleno que ya tenía la celda: sigue midiendo 16 bytes.
    pub link: u16,
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            flags: Flags::empty(),
            link: 0,
        }
    }
}

impl Cell {
    /// Celda en blanco que conserva el fondo dado (borrado con *background color erase*).
    pub fn blank(bg: Color) -> Cell {
        Cell {
            bg,
            ..Cell::default()
        }
    }

    fn is_leading_spacer(&self) -> bool {
        self.flags.contains(Flags::WIDE_SPACER)
    }
}

/// Una fila. Las del historial pueden ser más cortas que el ancho actual: se
/// recortan las celdas por defecto del final para ahorrar memoria.
#[derive(Debug, Clone, PartialEq, Eq, Default)]
pub struct Row {
    pub cells: Vec<Cell>,
    /// La línea continúa en la fila siguiente (salto suave por ancho).
    pub wrapped: bool,
    /// Todas las celdas desde aquí son `Cell::default()` (cota superior: puede
    /// ser mayor de lo necesario, nunca menor). Evita recorrer y limpiar la
    /// fila entera al pasarla al historial.
    pub(crate) used: usize,
}

impl Row {
    fn blank(cols: usize, template: Cell) -> Row {
        Row {
            cells: vec![template; cols],
            wrapped: false,
            used: if template == Cell::default() { 0 } else { cols },
        }
    }

    /// Deja la fila en blanco con `template`.
    fn clear(&mut self, template: Cell) {
        if template == Cell::default() {
            let used = self.used.min(self.cells.len());
            self.cells[..used].fill(template);
            self.used = 0;
        } else {
            self.cells.fill(template);
            self.used = self.cells.len();
        }
        self.wrapped = false;
    }

    /// Copia de la fila sin las celdas por defecto del final (salvo si tiene
    /// salto suave), con la capacidad justa.
    fn compacted(&self) -> Row {
        let len = if self.wrapped {
            self.cells.len()
        } else {
            self.cells[..self.used.min(self.cells.len())]
                .iter()
                .rposition(|c| *c != Cell::default())
                .map_or(0, |i| i + 1)
        };
        Row {
            cells: self.cells[..len].to_vec(),
            wrapped: self.wrapped,
            used: len,
        }
    }

    /// Recorta las celdas por defecto del final (no las de filas con salto
    /// suave, que se unen con la siguiente en el reflow).
    fn compact(&mut self) {
        if !self.wrapped {
            let len = self
                .cells
                .iter()
                .rposition(|c| *c != Cell::default())
                .map_or(0, |i| i + 1);
            self.cells.truncate(len);
        }
        self.cells.shrink_to_fit();
    }
}

/// Un punto (línea absoluta, columna) que el reflow debe recolocar.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Point {
    pub line: Line,
    pub col: usize,
    /// El cursor está en la última columna con el salto de línea pendiente:
    /// en realidad apunta a la posición siguiente.
    pub pending: bool,
}

/// Pantalla visible más historial.
#[derive(Debug, Clone)]
pub struct Grid {
    cols: usize,
    /// Filas de la pantalla. Van en caja para que desplazar una región mueva
    /// punteros de 8 bytes y no estructuras `Row` enteras (bench-core,
    /// `y\r\n` en región: de 66 a 91 MiB/s).
    #[allow(clippy::vec_box)]
    rows: Vec<Box<Row>>,
    dirty: Vec<bool>,
    history: VecDeque<Row>,
    history_limit: usize,
    /// Número absoluto de `history[0]` (o de la primera fila si no hay historial).
    first_line: Line,
    /// Grupos de caracteres (base + combinantes) de las celdas de esta
    /// cuadrícula; la celda guarda `CLUSTER_BASE + índice`.
    clusters: Vec<String>,
    cluster_ids: HashMap<String, char>,
}

impl Grid {
    /// Cuadrícula sin historial.
    pub fn new(cols: usize, rows: usize) -> Grid {
        Grid::with_history(cols, rows, 0)
    }

    pub fn with_history(cols: usize, rows: usize, history_limit: usize) -> Grid {
        assert!(cols > 0 && rows > 0, "la cuadrícula no puede estar vacía");
        Grid {
            cols,
            rows: (0..rows)
                .map(|_| Box::new(Row::blank(cols, Cell::default())))
                .collect(),
            dirty: vec![true; rows],
            history: VecDeque::new(),
            history_limit,
            first_line: 0,
            clusters: Vec::new(),
            cluster_ids: HashMap::new(),
        }
    }

    // --- Grupos de caracteres ---

    /// Código para una celda con el texto `text` (base + combinantes). Si la
    /// tabla está llena, se queda solo el carácter base.
    pub fn cluster(&mut self, text: &str) -> char {
        if let Some(&c) = self.cluster_ids.get(text) {
            return c;
        }
        if self.clusters.len() >= CLUSTER_MAX {
            return text.chars().next().unwrap_or(' ');
        }
        let c = char::from_u32(CLUSTER_BASE + self.clusters.len() as u32)
            .expect("código privado válido");
        self.clusters.push(text.to_owned());
        self.cluster_ids.insert(text.to_owned(), c);
        c
    }

    /// Texto de un carácter de celda: el propio carácter o su grupo.
    pub fn char_text(&self, ch: char) -> Cow<'_, str> {
        let code = ch as u32;
        if code >= CLUSTER_BASE
            && let Some(s) = self.clusters.get((code - CLUSTER_BASE) as usize)
        {
            return Cow::Borrowed(s);
        }
        Cow::Owned(ch.to_string())
    }

    /// Carácter base (el primero) de un carácter de celda.
    pub fn base_char(&self, ch: char) -> char {
        self.char_text(ch).chars().next().unwrap_or(' ')
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.rows[row].cells[col]
    }

    pub fn cell_mut(&mut self, row: usize, col: usize) -> &mut Cell {
        self.dirty[row] = true;
        let r = &mut self.rows[row];
        r.used = r.used.max(col + 1);
        &mut r.cells[col]
    }

    pub fn row(&self, row: usize) -> &[Cell] {
        &self.rows[row].cells
    }

    pub fn is_wrapped(&self, row: usize) -> bool {
        self.rows[row].wrapped
    }

    pub fn set_wrapped(&mut self, row: usize, wrapped: bool) {
        self.rows[row].wrapped = wrapped;
    }

    // --- Historial y líneas absolutas ---

    pub fn history_len(&self) -> usize {
        self.history.len()
    }

    /// Número absoluto de la línea más antigua guardada.
    pub fn first_line(&self) -> Line {
        self.first_line
    }

    /// Número absoluto de la primera fila de la pantalla.
    pub fn screen_start(&self) -> Line {
        self.first_line + self.history.len() as Line
    }

    /// Una línea por su número absoluto, del historial o de la pantalla.
    pub fn line(&self, line: Line) -> Option<&Row> {
        let i = line.checked_sub(self.first_line)? as usize;
        if i < self.history.len() {
            self.history.get(i)
        } else {
            self.rows.get(i - self.history.len()).map(|r| &**r)
        }
    }

    pub fn clear_history(&mut self) {
        self.first_line = self.screen_start();
        self.history.clear();
    }

    /// Memoria aproximada que ocupan las celdas del historial, en bytes.
    /// Comprueba el invariante de `Row::used` en la pantalla (para tests).
    #[cfg(test)]
    pub(crate) fn check_used(&self) {
        for (i, r) in self.rows.iter().enumerate() {
            assert!(
                r.cells[r.used.min(r.cells.len())..]
                    .iter()
                    .all(|c| *c == Cell::default()),
                "fila {i}: hay contenido más allá de used={}",
                r.used
            );
        }
    }

    pub fn history_bytes(&self) -> usize {
        self.history
            .iter()
            .map(|r| r.cells.capacity() * std::mem::size_of::<Cell>() + std::mem::size_of::<Row>())
            .sum()
    }

    // --- Daños ---

    pub fn mark_all_dirty(&mut self) {
        self.dirty.fill(true);
    }

    pub fn is_dirty(&self, row: usize) -> bool {
        self.dirty[row]
    }

    pub fn any_dirty(&self) -> bool {
        self.dirty.iter().any(|d| *d)
    }

    pub fn clear_damage(&mut self) {
        self.dirty.fill(false);
    }

    /// Llama a `f` con cada fila sucia y la marca como limpia.
    pub fn drain_damage(&mut self, mut f: impl FnMut(usize, &[Cell])) {
        for (i, (row, dirty)) in self.rows.iter().zip(&mut self.dirty).enumerate() {
            if std::mem::take(dirty) {
                f(i, &row.cells);
            }
        }
    }

    // --- Edición ---

    /// Rellena `[start, end)` de una fila con `template`.
    pub fn clear_cols(&mut self, row: usize, start: usize, end: usize, template: Cell) {
        let end = end.min(self.cols);
        if start < end {
            let r = &mut self.rows[row];
            r.cells[start..end].fill(template);
            if template != Cell::default() {
                r.used = r.used.max(end);
            }
            self.dirty[row] = true;
        }
    }

    /// Rellena las filas `[start, end)` completas con `template`.
    pub fn clear_rows(&mut self, start: usize, end: usize, template: Cell) {
        let end = end.min(self.rows.len());
        for row in self.rows.iter_mut().take(end).skip(start) {
            row.clear(template);
        }
        if start < end {
            self.dirty[start..end].fill(true);
        }
    }

    /// Sube `n` filas el contenido de la región `[top, bottom]` (inclusive).
    /// Las filas que entran por abajo quedan en blanco. Si la región empieza
    /// arriba del todo y hay historial, las que salen se guardan en él.
    pub fn scroll_up(&mut self, top: usize, bottom: usize, n: usize, template: Cell) {
        let n = n.min(bottom + 1 - top);
        if top == 0 && self.history_limit > 0 {
            for i in 0..n {
                // Al historial va una copia compacta (una sola reserva, del
                // tamaño justo); la fila original se reutiliza en blanco.
                let row = &mut self.rows[i];
                self.history.push_back(row.compacted());
                row.clear(template);
            }
            while self.history.len() > self.history_limit {
                self.history.pop_front();
                self.first_line += 1;
            }
            self.rows[..=bottom].rotate_left(n);
        } else {
            self.rows[top..=bottom].rotate_left(n);
            self.clear_rows(bottom + 1 - n, bottom + 1, template);
        }
        self.dirty[top..=bottom].fill(true);
    }

    /// Baja `n` filas el contenido de la región `[top, bottom]` (inclusive).
    /// Las filas que entran por arriba quedan en blanco.
    pub fn scroll_down(&mut self, top: usize, bottom: usize, n: usize, template: Cell) {
        let n = n.min(bottom + 1 - top);
        self.rows[top..=bottom].rotate_right(n);
        self.clear_rows(top, top + n, template);
        self.dirty[top..=bottom].fill(true);
    }

    /// Inserta `n` celdas en blanco en `col`, empujando el resto a la derecha.
    pub fn insert_cells(&mut self, row: usize, col: usize, n: usize, template: Cell) {
        let n = n.min(self.cols - col);
        let line = &mut self.rows[row].cells[col..];
        line.rotate_right(n);
        line[..n].fill(template);
        self.rows[row].used = self.cols;
        self.dirty[row] = true;
    }

    /// Borra `n` celdas en `col`, trayendo el resto hacia la izquierda.
    pub fn delete_cells(&mut self, row: usize, col: usize, n: usize, template: Cell) {
        let n = n.min(self.cols - col);
        let line = &mut self.rows[row].cells[col..];
        line.rotate_left(n);
        let len = line.len();
        line[len - n..].fill(template);
        self.rows[row].used = self.cols;
        self.dirty[row] = true;
    }

    // --- Cambio de tamaño ---

    /// Cambia el tamaño recortando o rellenando, sin reajustar líneas (para
    /// la pantalla alternativa).
    pub fn resize_truncate(&mut self, cols: usize, rows: usize) {
        assert!(cols > 0 && rows > 0, "la cuadrícula no puede estar vacía");
        for row in &mut self.rows {
            row.cells.resize(cols, Cell::default());
            row.used = row.used.min(cols);
            row.wrapped = false;
            // Un carácter ancho cortado por el borde derecho deja de serlo.
            if let Some(last) = row.cells.last_mut()
                && last.flags.contains(Flags::WIDE)
            {
                *last = Cell::default();
            }
        }
        self.rows
            .resize_with(rows, || Box::new(Row::blank(cols, Cell::default())));
        self.cols = cols;
        self.dirty = vec![true; rows];
    }

    /// Cambia el tamaño reajustando las líneas largas al nuevo ancho (reflow)
    /// y mueve líneas entre la pantalla y el historial para que `points[0]`
    /// (el cursor) siga en pantalla. Recoloca todos los `points`.
    ///
    /// Las filas por debajo del cursor que estén vacías se descartan.
    pub fn resize_reflow(&mut self, cols: usize, rows: usize, points: &mut [Point]) {
        assert!(cols > 0 && rows > 0, "la cuadrícula no puede estar vacía");
        let cursor_line = points.first().map(|p| p.line);

        // 1. Todas las filas, sin las vacías del final que estén por debajo del cursor.
        let mut all: Vec<Row> = self.history.drain(..).collect();
        let old_first = self.first_line;
        all.extend(self.rows.drain(..).map(|r| *r));
        let cursor_idx = cursor_line.map(|l| (l - old_first) as usize);
        while all.len() > cursor_idx.map_or(0, |c| c + 1)
            && all
                .last()
                .is_some_and(|r| !r.wrapped && r.cells.iter().all(|c| *c == Cell::default()))
        {
            all.pop();
        }

        // 2. Líneas lógicas: se unen las filas con salto suave. Cada punto se
        //    traduce a (línea lógica, desplazamiento dentro de ella).
        struct Logical {
            cells: Vec<Cell>,
        }
        let mut logical: Vec<Logical> = Vec::new();
        let mut point_pos: Vec<Option<(usize, usize)>> = vec![None; points.len()];
        let mut current: Vec<Cell> = Vec::new();
        for (idx, mut row) in all.into_iter().enumerate() {
            let line = old_first + idx as Line;
            let wrapped = row.wrapped;
            // El hueco de un carácter ancho que no cabía no es contenido.
            if wrapped && row.cells.last().is_some_and(|c| c.is_leading_spacer()) {
                let n = row.cells.len();
                let prev_is_wide = n >= 2 && row.cells[n - 2].flags.contains(Flags::WIDE);
                if !prev_is_wide {
                    row.cells.pop();
                }
            }
            if !wrapped {
                let len = row
                    .cells
                    .iter()
                    .rposition(|c| *c != Cell::default())
                    .map_or(0, |i| i + 1);
                row.cells.truncate(len);
            }
            for (pi, p) in points.iter().enumerate() {
                if p.line == line {
                    point_pos[pi] =
                        Some((logical.len(), current.len() + p.col + p.pending as usize));
                }
            }
            current.extend(row.cells);
            if !wrapped {
                logical.push(Logical {
                    cells: std::mem::take(&mut current),
                });
            }
        }
        if !current.is_empty() {
            logical.push(Logical { cells: current });
        }

        // 3. Se vuelven a partir al nuevo ancho.
        let mut out: Vec<Row> = Vec::new();
        let mut new_points: Vec<Option<Point>> = vec![None; points.len()];
        for (li, l) in logical.iter().enumerate() {
            let start_row = out.len();
            let mut row = Row::default();
            // Posición (fila relativa, columna) de cada celda de la línea lógica.
            let mut positions: Vec<(usize, usize)> = Vec::with_capacity(l.cells.len());
            let mut r = 0;
            for cell in &l.cells {
                let width = if cell.flags.contains(Flags::WIDE) {
                    2
                } else {
                    1
                };
                let is_spacer = cell.flags.contains(Flags::WIDE_SPACER);
                if !is_spacer && row.cells.len() + width > cols {
                    if width == 2 && cols >= 2 && row.cells.len() < cols {
                        row.cells.push(Cell {
                            flags: Flags::WIDE_SPACER,
                            ..Cell::default()
                        });
                    }
                    row.wrapped = true;
                    out.push(std::mem::take(&mut row));
                    r += 1;
                }
                if is_spacer && row.cells.len() >= cols {
                    // La segunda mitad acompaña a su primera mitad.
                    positions.push((r, cols - 1));
                    continue;
                }
                positions.push((r, row.cells.len()));
                row.cells.push(*cell);
            }
            out.push(row);

            for (pi, pos) in point_pos.iter().enumerate() {
                let Some((pl, off)) = *pos else { continue };
                if pl != li {
                    continue;
                }
                let (rr, cc, pending) = match positions.get(off) {
                    Some(&(r, c)) => (r, c, false),
                    None => {
                        // Más allá del contenido (p. ej. el cursor tras un espacio).
                        let extra = off - positions.len();
                        let (lr, lc) = positions.last().map_or((0, 0), |&(r, c)| (r, c + 1));
                        if lc + extra < cols {
                            (lr, lc + extra, false)
                        } else if extra == 0 {
                            // Justo tras una fila llena: salto de línea pendiente.
                            (lr, cols - 1, true)
                        } else {
                            (lr, cols - 1, false)
                        }
                    }
                };
                new_points[pi] = Some(Point {
                    line: (start_row + rr) as Line,
                    col: cc,
                    pending,
                });
            }
        }

        // 4. Qué filas quedan en pantalla: las últimas, pero sin dejar fuera el cursor.
        let total = out.len();
        let mut start = total.saturating_sub(rows);
        if let Some(Some(c)) = new_points.first() {
            let c = c.line as usize;
            if c < start {
                start = c;
            } else if c >= start + rows {
                start = c + 1 - rows;
            }
        }
        let mut screen: Vec<Row> = out.split_off(start.min(out.len()));
        screen.truncate(rows);
        for row in &mut screen {
            row.used = row.cells.len();
            row.cells.resize(cols, Cell::default());
        }
        screen.resize_with(rows, || Row::blank(cols, Cell::default()));
        let mut history = out;
        for row in &mut history {
            row.compact();
        }
        let dropped = history.len().saturating_sub(self.history_limit);
        history.drain(..dropped);

        let new_first = old_first;
        let history_offset = start - history.len();
        for (p, np) in points.iter_mut().zip(new_points) {
            *p = match np {
                Some(np) if (np.line as usize) >= history_offset => Point {
                    line: new_first + np.line - history_offset as Line,
                    ..np
                },
                // Punto descartado o fuera de lo guardado: a la línea más antigua.
                _ => Point {
                    line: new_first,
                    col: 0,
                    pending: false,
                },
            };
        }

        self.history = history.into();
        self.rows = screen.into_iter().map(Box::new).collect();
        self.cols = cols;
        self.first_line = new_first;
        self.dirty = vec![true; rows];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn dirty_rows(grid: &mut Grid) -> Vec<usize> {
        let mut rows = Vec::new();
        grid.drain_damage(|i, _| rows.push(i));
        rows
    }

    #[test]
    fn cell_is_compact() {
        // Base para estimar la memoria del historial.
        assert_eq!(std::mem::size_of::<Cell>(), 16);
    }

    #[test]
    fn new_grid_is_fully_dirty_then_clean() {
        let mut g = Grid::new(3, 3);
        assert_eq!(dirty_rows(&mut g), [0, 1, 2]);
        assert!(dirty_rows(&mut g).is_empty());
    }

    #[test]
    fn mutations_mark_only_affected_rows() {
        let mut g = Grid::new(3, 4);
        dirty_rows(&mut g);
        g.cell_mut(1, 0).ch = 'x';
        assert_eq!(dirty_rows(&mut g), [1]);
        g.clear_cols(2, 0, 1, Cell::default());
        assert_eq!(dirty_rows(&mut g), [2]);
        g.scroll_up(1, 2, 1, Cell::default());
        assert_eq!(dirty_rows(&mut g), [1, 2]);
    }

    #[test]
    fn scrolled_out_rows_go_to_history_and_are_compacted() {
        let mut g = Grid::with_history(10, 2, 100);
        g.cell_mut(0, 0).ch = 'a';
        g.scroll_up(0, 1, 1, Cell::default());
        assert_eq!(g.history_len(), 1);
        assert_eq!(g.line(0).unwrap().cells.len(), 1);
        assert_eq!(g.screen_start(), 1);
    }

    #[test]
    fn history_limit_evicts_oldest() {
        let mut g = Grid::with_history(4, 1, 3);
        for i in 0..5 {
            g.cell_mut(0, 0).ch = char::from(b'0' + i);
            g.scroll_up(0, 0, 1, Cell::default());
        }
        assert_eq!(g.history_len(), 3);
        assert_eq!(g.first_line(), 2);
        assert_eq!(g.line(2).unwrap().cells[0].ch, '2');
        assert!(g.line(1).is_none());
    }

    #[test]
    fn scroll_region_not_at_top_does_not_feed_history() {
        let mut g = Grid::with_history(4, 3, 10);
        g.scroll_up(1, 2, 1, Cell::default());
        assert_eq!(g.history_len(), 0);
    }
}
