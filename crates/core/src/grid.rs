//! Cuadrícula de celdas de la pantalla visible.

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

/// Atributos de texto de una celda, como bitset.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Flags(u8);

impl Flags {
    pub const BOLD: Flags = Flags(1 << 0);
    pub const DIM: Flags = Flags(1 << 1);
    pub const ITALIC: Flags = Flags(1 << 2);
    pub const UNDERLINE: Flags = Flags(1 << 3);
    pub const INVERSE: Flags = Flags(1 << 4);
    pub const STRIKEOUT: Flags = Flags(1 << 5);

    pub const fn empty() -> Flags {
        Flags(0)
    }

    pub const fn contains(self, other: Flags) -> bool {
        self.0 & other.0 == other.0
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
}

impl Default for Cell {
    fn default() -> Self {
        Cell {
            ch: ' ',
            fg: Color::Default,
            bg: Color::Default,
            flags: Flags::empty(),
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
}

/// Filas de tamaño fijo. Las filas son `Vec` separados para que desplazar
/// una región sea rotar punteros, no copiar celdas.
#[derive(Debug, Clone)]
pub struct Grid {
    cols: usize,
    rows: Vec<Vec<Cell>>,
}

impl Grid {
    pub fn new(cols: usize, rows: usize) -> Grid {
        assert!(cols > 0 && rows > 0, "la cuadrícula no puede estar vacía");
        Grid {
            cols,
            rows: vec![vec![Cell::default(); cols]; rows],
        }
    }

    pub fn cols(&self) -> usize {
        self.cols
    }

    pub fn rows(&self) -> usize {
        self.rows.len()
    }

    pub fn cell(&self, row: usize, col: usize) -> &Cell {
        &self.rows[row][col]
    }

    pub fn cell_mut(&mut self, row: usize, col: usize) -> &mut Cell {
        &mut self.rows[row][col]
    }

    pub fn row(&self, row: usize) -> &[Cell] {
        &self.rows[row]
    }

    /// Cambia el tamaño recortando o rellenando por la derecha y por abajo.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        assert!(cols > 0 && rows > 0, "la cuadrícula no puede estar vacía");
        for row in &mut self.rows {
            row.resize(cols, Cell::default());
        }
        self.rows.resize_with(rows, || vec![Cell::default(); cols]);
        self.cols = cols;
    }

    /// Rellena `[start, end)` de una fila con `template`.
    pub fn clear_cols(&mut self, row: usize, start: usize, end: usize, template: Cell) {
        let end = end.min(self.cols);
        if start < end {
            self.rows[row][start..end].fill(template);
        }
    }

    /// Rellena las filas `[start, end)` completas con `template`.
    pub fn clear_rows(&mut self, start: usize, end: usize, template: Cell) {
        let end = end.min(self.rows.len());
        for row in self.rows.iter_mut().take(end).skip(start) {
            row.fill(template);
        }
    }

    /// Sube `n` filas el contenido de la región `[top, bottom]` (inclusive).
    /// Las filas que entran por abajo quedan en blanco.
    pub fn scroll_up(&mut self, top: usize, bottom: usize, n: usize, template: Cell) {
        let n = n.min(bottom + 1 - top);
        self.rows[top..=bottom].rotate_left(n);
        self.clear_rows(bottom + 1 - n, bottom + 1, template);
    }

    /// Baja `n` filas el contenido de la región `[top, bottom]` (inclusive).
    /// Las filas que entran por arriba quedan en blanco.
    pub fn scroll_down(&mut self, top: usize, bottom: usize, n: usize, template: Cell) {
        let n = n.min(bottom + 1 - top);
        self.rows[top..=bottom].rotate_right(n);
        self.clear_rows(top, top + n, template);
    }

    /// Inserta `n` celdas en blanco en `col`, empujando el resto a la derecha.
    pub fn insert_cells(&mut self, row: usize, col: usize, n: usize, template: Cell) {
        let n = n.min(self.cols - col);
        let line = &mut self.rows[row][col..];
        line.rotate_right(n);
        line[..n].fill(template);
    }

    /// Borra `n` celdas en `col`, trayendo el resto hacia la izquierda.
    pub fn delete_cells(&mut self, row: usize, col: usize, n: usize, template: Cell) {
        let n = n.min(self.cols - col);
        let line = &mut self.rows[row][col..];
        line.rotate_left(n);
        let len = line.len();
        line[len - n..].fill(template);
    }
}
