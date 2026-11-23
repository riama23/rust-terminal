//! Estado del terminal: interpreta las acciones de `vte` sobre la cuadrícula.

use vte::{Params, Parser, Perform};

use crate::grid::{Cell, Color, Flags, Grid};

/// Posición del cursor. `wrap_pending` indica que se escribió en la última
/// columna y el siguiente carácter imprimible salta de línea antes.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub struct Cursor {
    pub row: usize,
    pub col: usize,
    pub wrap_pending: bool,
}

/// Atributos con los que se escriben las celdas nuevas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Pen {
    fg: Color,
    bg: Color,
    flags: Flags,
}

#[derive(Debug, Clone, Copy)]
struct Saved {
    cursor: Cursor,
    pen: Pen,
}

/// Estado sobre el que actúa el parser. Está separado de `Term` para poder
/// tomar prestados a la vez el parser y el estado en `advance`.
struct Screen {
    grid: Grid,
    cursor: Cursor,
    pen: Pen,
    /// Región de scroll `[top, bottom]`, inclusive.
    top: usize,
    bottom: usize,
    saved: Option<Saved>,
    bell: bool,
}

pub struct Term {
    parser: Parser,
    screen: Screen,
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Term {
        Term {
            parser: Parser::new(),
            screen: Screen {
                grid: Grid::new(cols, rows),
                cursor: Cursor::default(),
                pen: Pen::default(),
                top: 0,
                bottom: rows - 1,
                saved: None,
                bell: false,
            },
        }
    }

    /// Aplica un lote de bytes leídos de la PTY.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    pub fn grid(&self) -> &Grid {
        &self.screen.grid
    }

    pub fn cursor(&self) -> Cursor {
        self.screen.cursor
    }

    /// Devuelve si sonó la campana desde la última llamada y la rearma.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.screen.bell)
    }

    /// Cambia el tamaño. La región de scroll vuelve a ser la pantalla entera.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let s = &mut self.screen;
        s.grid.resize(cols, rows);
        s.top = 0;
        s.bottom = rows - 1;
        s.cursor.row = s.cursor.row.min(rows - 1);
        s.cursor.col = s.cursor.col.min(cols - 1);
        s.cursor.wrap_pending = false;
    }

    /// La pantalla como texto: una línea por fila, sin espacios finales y sin
    /// filas vacías al final.
    pub fn snapshot_text(&self) -> String {
        let grid = &self.screen.grid;
        let mut lines: Vec<String> = (0..grid.rows())
            .map(|r| {
                let line: String = grid.row(r).iter().map(|c| c.ch).collect();
                line.trim_end_matches(' ').to_owned()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }
}

/// Primer valor del parámetro `i`, con `default` si falta o es 0.
fn param(params: &Params, i: usize, default: usize) -> usize {
    match params.iter().nth(i).and_then(|p| p.first()) {
        Some(&0) | None => default,
        Some(&v) => v as usize,
    }
}

/// Valor crudo del parámetro `i` (0 si falta), para los que tienen 0 como selector.
fn raw(params: &Params, i: usize) -> u16 {
    params
        .iter()
        .nth(i)
        .and_then(|p| p.first())
        .copied()
        .unwrap_or(0)
}

fn byte(v: u16) -> u8 {
    v.min(255) as u8
}

impl Screen {
    fn cols(&self) -> usize {
        self.grid.cols()
    }

    fn rows(&self) -> usize {
        self.grid.rows()
    }

    /// Celda para borrar: espacio con el fondo actual.
    fn blank(&self) -> Cell {
        Cell::blank(self.pen.bg)
    }

    fn move_to(&mut self, row: usize, col: usize) {
        self.cursor.row = row.min(self.rows() - 1);
        self.cursor.col = col.min(self.cols() - 1);
        self.cursor.wrap_pending = false;
    }

    fn linefeed(&mut self) {
        self.cursor.wrap_pending = false;
        if self.cursor.row == self.bottom {
            let blank = self.blank();
            self.grid.scroll_up(self.top, self.bottom, 1, blank);
        } else if self.cursor.row + 1 < self.rows() {
            self.cursor.row += 1;
        }
    }

    fn reverse_index(&mut self) {
        self.cursor.wrap_pending = false;
        if self.cursor.row == self.top {
            let blank = self.blank();
            self.grid.scroll_down(self.top, self.bottom, 1, blank);
        } else if self.cursor.row > 0 {
            self.cursor.row -= 1;
        }
    }

    /// Límites verticales para CUU/CUD: la región si el cursor está dentro.
    fn vertical_bounds(&self) -> (usize, usize) {
        let row = self.cursor.row;
        let top = if row >= self.top { self.top } else { 0 };
        let bottom = if row <= self.bottom {
            self.bottom
        } else {
            self.rows() - 1
        };
        (top, bottom)
    }

    fn erase_display(&mut self, mode: u16) {
        let blank = self.blank();
        let Cursor { row, col, .. } = self.cursor;
        match mode {
            0 => {
                self.grid.clear_cols(row, col, self.cols(), blank);
                self.grid.clear_rows(row + 1, self.rows(), blank);
            }
            1 => {
                self.grid.clear_rows(0, row, blank);
                self.grid.clear_cols(row, 0, col + 1, blank);
            }
            2 => self.grid.clear_rows(0, self.rows(), blank),
            // 3 borra el scrollback, que llega en M4.
            _ => {}
        }
    }

    fn erase_line(&mut self, mode: u16) {
        let blank = self.blank();
        let Cursor { row, col, .. } = self.cursor;
        match mode {
            0 => self.grid.clear_cols(row, col, self.cols(), blank),
            1 => self.grid.clear_cols(row, 0, col + 1, blank),
            2 => self.grid.clear_cols(row, 0, self.cols(), blank),
            _ => {}
        }
    }

    fn set_scroll_region(&mut self, params: &Params) {
        let top = param(params, 0, 1) - 1;
        let bottom = param(params, 1, self.rows()).min(self.rows()) - 1;
        if top < bottom {
            self.top = top;
            self.bottom = bottom;
            self.move_to(0, 0);
        }
    }

    fn save_cursor(&mut self) {
        self.saved = Some(Saved {
            cursor: self.cursor,
            pen: self.pen,
        });
    }

    fn restore_cursor(&mut self) {
        match self.saved {
            Some(saved) => {
                self.pen = saved.pen;
                self.move_to(saved.cursor.row, saved.cursor.col);
                self.cursor.wrap_pending = saved.cursor.wrap_pending;
            }
            None => {
                self.pen = Pen::default();
                self.move_to(0, 0);
            }
        }
    }

    fn reset(&mut self) {
        let (cols, rows) = (self.cols(), self.rows());
        *self = Screen {
            grid: Grid::new(cols, rows),
            cursor: Cursor::default(),
            pen: Pen::default(),
            top: 0,
            bottom: rows - 1,
            saved: None,
            bell: false,
        };
    }

    fn sgr(&mut self, params: &Params) {
        let params: Vec<&[u16]> = params.iter().collect();
        if params.is_empty() {
            self.pen = Pen::default();
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p[0] {
                0 => self.pen = Pen::default(),
                1 => self.pen.flags.insert(Flags::BOLD),
                2 => self.pen.flags.insert(Flags::DIM),
                3 => self.pen.flags.insert(Flags::ITALIC),
                4 | 21 => self.pen.flags.insert(Flags::UNDERLINE),
                7 => self.pen.flags.insert(Flags::INVERSE),
                9 => self.pen.flags.insert(Flags::STRIKEOUT),
                22 => self.pen.flags.remove(Flags::BOLD | Flags::DIM),
                23 => self.pen.flags.remove(Flags::ITALIC),
                24 => self.pen.flags.remove(Flags::UNDERLINE),
                27 => self.pen.flags.remove(Flags::INVERSE),
                29 => self.pen.flags.remove(Flags::STRIKEOUT),
                n @ 30..=37 => self.pen.fg = Color::Indexed((n - 30) as u8),
                39 => self.pen.fg = Color::Default,
                n @ 40..=47 => self.pen.bg = Color::Indexed((n - 40) as u8),
                49 => self.pen.bg = Color::Default,
                n @ 90..=97 => self.pen.fg = Color::Indexed((n - 90 + 8) as u8),
                n @ 100..=107 => self.pen.bg = Color::Indexed((n - 100 + 8) as u8),
                n @ (38 | 48) => {
                    let color = if p.len() > 1 {
                        extended_color_colon(p)
                    } else {
                        let (color, used) = extended_color_semicolon(&params[i + 1..]);
                        i += used;
                        color
                    };
                    if let Some(color) = color {
                        if n == 38 {
                            self.pen.fg = color;
                        } else {
                            self.pen.bg = color;
                        }
                    }
                }
                _ => {}
            }
            i += 1;
        }
    }
}

/// `38:5:n`, `38:2:r:g:b` o `38:2:<espacio de color>:r:g:b`.
fn extended_color_colon(p: &[u16]) -> Option<Color> {
    match p.get(1)? {
        5 => Some(Color::Indexed(byte(*p.get(2)?))),
        2 => {
            let rgb = if p.len() >= 6 { &p[3..6] } else { p.get(2..5)? };
            Some(Color::Rgb(byte(rgb[0]), byte(rgb[1]), byte(rgb[2])))
        }
        _ => None,
    }
}

/// `38;5;n` o `38;2;r;g;b`. Devuelve el color y cuántos parámetros consumió.
fn extended_color_semicolon(rest: &[&[u16]]) -> (Option<Color>, usize) {
    let get = |i: usize| rest.get(i).map(|p| p[0]);
    match get(0) {
        Some(5) => match get(1) {
            Some(n) => (Some(Color::Indexed(byte(n))), 2),
            None => (None, 1),
        },
        Some(2) => match (get(1), get(2), get(3)) {
            (Some(r), Some(g), Some(b)) => (Some(Color::Rgb(byte(r), byte(g), byte(b))), 4),
            _ => (None, rest.len()),
        },
        Some(_) => (None, 1),
        None => (None, 0),
    }
}

impl Perform for Screen {
    fn print(&mut self, c: char) {
        if self.cursor.wrap_pending {
            self.cursor.col = 0;
            self.linefeed();
        }
        let Pen { fg, bg, flags } = self.pen;
        *self.grid.cell_mut(self.cursor.row, self.cursor.col) = Cell {
            ch: c,
            fg,
            bg,
            flags,
        };
        if self.cursor.col + 1 == self.cols() {
            self.cursor.wrap_pending = true;
        } else {
            self.cursor.col += 1;
        }
    }

    fn execute(&mut self, byte: u8) {
        match byte {
            0x07 => self.bell = true,
            0x08 => {
                self.cursor.col = self.cursor.col.saturating_sub(1);
                self.cursor.wrap_pending = false;
            }
            0x09 => {
                let next = (self.cursor.col / 8 + 1) * 8;
                self.cursor.col = next.min(self.cols() - 1);
                self.cursor.wrap_pending = false;
            }
            0x0A..=0x0C => self.linefeed(),
            0x0D => {
                self.cursor.col = 0;
                self.cursor.wrap_pending = false;
            }
            _ => {}
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        if !intermediates.is_empty() {
            return;
        }
        match byte {
            b'7' => self.save_cursor(),
            b'8' => self.restore_cursor(),
            b'D' => self.linefeed(),
            b'E' => {
                self.cursor.col = 0;
                self.linefeed();
            }
            b'M' => self.reverse_index(),
            b'c' => self.reset(),
            _ => {}
        }
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        // Los modos privados (`CSI ? …`) y el resto de variantes con
        // intermedios llegan en hitos posteriores.
        if !intermediates.is_empty() {
            return;
        }
        let Cursor { row, col, .. } = self.cursor;
        let n = param(params, 0, 1);
        match action {
            'A' => {
                let (top, _) = self.vertical_bounds();
                self.move_to(row.saturating_sub(n).max(top), col);
            }
            'B' => {
                let (_, bottom) = self.vertical_bounds();
                self.move_to((row + n).min(bottom), col);
            }
            'C' => self.move_to(row, col + n),
            'D' => self.move_to(row, col.saturating_sub(n)),
            'E' => {
                let (_, bottom) = self.vertical_bounds();
                self.move_to((row + n).min(bottom), 0);
            }
            'F' => {
                let (top, _) = self.vertical_bounds();
                self.move_to(row.saturating_sub(n).max(top), 0);
            }
            'G' | '`' => self.move_to(row, n - 1),
            'd' => self.move_to(n - 1, col),
            'H' | 'f' => self.move_to(n - 1, param(params, 1, 1) - 1),
            'J' => self.erase_display(raw(params, 0)),
            'K' => self.erase_line(raw(params, 0)),
            '@' => {
                let blank = self.blank();
                self.grid.insert_cells(row, col, n, blank);
                self.cursor.wrap_pending = false;
            }
            'P' => {
                let blank = self.blank();
                self.grid.delete_cells(row, col, n, blank);
                self.cursor.wrap_pending = false;
            }
            'X' => {
                let blank = self.blank();
                self.grid.clear_cols(row, col, col + n, blank);
                self.cursor.wrap_pending = false;
            }
            'L' | 'M' if (self.top..=self.bottom).contains(&row) => {
                let blank = self.blank();
                if action == 'L' {
                    self.grid.scroll_down(row, self.bottom, n, blank);
                } else {
                    self.grid.scroll_up(row, self.bottom, n, blank);
                }
                self.move_to(row, 0);
            }
            'S' => {
                let blank = self.blank();
                self.grid.scroll_up(self.top, self.bottom, n, blank);
            }
            'T' => {
                let blank = self.blank();
                self.grid.scroll_down(self.top, self.bottom, n, blank);
            }
            'r' => self.set_scroll_region(params),
            'm' => self.sgr(params),
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            _ => {}
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn term(cols: usize, rows: usize, input: &str) -> Term {
        let mut t = Term::new(cols, rows);
        t.advance(input.as_bytes());
        t
    }

    fn pos(t: &Term) -> (usize, usize) {
        let c = t.cursor();
        (c.row, c.col)
    }

    #[test]
    fn plain_text() {
        let t = term(10, 3, "hola");
        assert_eq!(t.snapshot_text(), "hola");
        assert_eq!(pos(&t), (0, 4));
    }

    #[test]
    fn cr_lf() {
        let t = term(10, 3, "ab\r\ncd\nx");
        assert_eq!(t.snapshot_text(), "ab\ncd\n  x");
        assert_eq!(pos(&t), (2, 3));
    }

    #[test]
    fn backspace_and_tab() {
        let t = term(20, 2, "abc\x08X\tY");
        assert_eq!(t.snapshot_text(), "abX     Y");
        let t = term(10, 1, "\t\t\tZ");
        assert_eq!(pos(&t), (0, 9));
        assert_eq!(t.grid().cell(0, 9).ch, 'Z');
    }

    #[test]
    fn autowrap_with_wrap_pending() {
        let mut t = term(80, 3, &"a".repeat(80));
        assert_eq!(pos(&t), (0, 79));
        assert!(t.cursor().wrap_pending);
        assert_eq!(t.snapshot_text(), "a".repeat(80));

        t.advance(b"b");
        assert_eq!(pos(&t), (1, 1));
        assert!(!t.cursor().wrap_pending);
        assert_eq!(t.grid().cell(1, 0).ch, 'b');
    }

    #[test]
    fn cr_cancels_wrap_pending() {
        let t = term(3, 2, "abc\rX");
        assert_eq!(t.snapshot_text(), "Xbc");
    }

    #[test]
    fn cursor_position_and_relative_moves_are_clamped() {
        let mut t = term(10, 5, "\x1b[3;4H");
        assert_eq!(pos(&t), (2, 3));
        t.advance(b"\x1b[H");
        assert_eq!(pos(&t), (0, 0));
        t.advance(b"\x1b[99;99H");
        assert_eq!(pos(&t), (4, 9));
        t.advance(b"\x1b[2A\x1b[3D");
        assert_eq!(pos(&t), (2, 6));
        t.advance(b"\x1b[100A\x1b[100D");
        assert_eq!(pos(&t), (0, 0));
        t.advance(b"\x1b[B\x1b[C");
        assert_eq!(pos(&t), (1, 1));
        t.advance(b"\x1b[100B\x1b[100C");
        assert_eq!(pos(&t), (4, 9));
        t.advance(b"\x1b[5G\x1b[2d");
        assert_eq!(pos(&t), (1, 4));
    }

    #[test]
    fn erase_in_line() {
        let base = "abcdef\x1b[1;3H";
        assert_eq!(term(10, 1, &format!("{base}\x1b[K")).snapshot_text(), "ab");
        assert_eq!(
            term(10, 1, &format!("{base}\x1b[1K")).snapshot_text(),
            "   def"
        );
        assert_eq!(term(10, 1, &format!("{base}\x1b[2K")).snapshot_text(), "");
    }

    #[test]
    fn erase_in_display() {
        let base = "aaa\r\nbbb\r\nccc\x1b[2;2H";
        assert_eq!(
            term(5, 3, &format!("{base}\x1b[J")).snapshot_text(),
            "aaa\nb"
        );
        assert_eq!(
            term(5, 3, &format!("{base}\x1b[1J")).snapshot_text(),
            "\n  b\nccc"
        );
        assert_eq!(term(5, 3, &format!("{base}\x1b[2J")).snapshot_text(), "");
    }

    #[test]
    fn erase_uses_current_background() {
        let t = term(4, 1, "\x1b[44m\x1b[2K");
        assert_eq!(t.grid().cell(0, 3).bg, Color::Indexed(4));
    }

    #[test]
    fn sgr_basic_colors_and_flags() {
        let t = term(10, 1, "\x1b[1;4;31;42mA\x1b[0mB\x1b[95;103mC");
        let a = t.grid().cell(0, 0);
        assert_eq!(a.fg, Color::Indexed(1));
        assert_eq!(a.bg, Color::Indexed(2));
        assert!(a.flags.contains(Flags::BOLD | Flags::UNDERLINE));
        assert_eq!(
            *t.grid().cell(0, 1),
            Cell {
                ch: 'B',
                ..Cell::default()
            }
        );
        let c = t.grid().cell(0, 2);
        assert_eq!(c.fg, Color::Indexed(13));
        assert_eq!(c.bg, Color::Indexed(11));
    }

    #[test]
    fn sgr_empty_resets() {
        let t = term(10, 1, "\x1b[1;31m\x1b[mA");
        assert_eq!(
            *t.grid().cell(0, 0),
            Cell {
                ch: 'A',
                ..Cell::default()
            }
        );
    }

    #[test]
    fn sgr_256_and_truecolor() {
        let t = term(
            10,
            1,
            "\x1b[38;5;208;48;2;10;20;30mA\x1b[38:2::1:2:3;48:5:17mB\x1b[38:2:4:5:6mC",
        );
        let a = t.grid().cell(0, 0);
        assert_eq!(a.fg, Color::Indexed(208));
        assert_eq!(a.bg, Color::Rgb(10, 20, 30));
        let b = t.grid().cell(0, 1);
        assert_eq!(b.fg, Color::Rgb(1, 2, 3));
        assert_eq!(b.bg, Color::Indexed(17));
        assert_eq!(t.grid().cell(0, 2).fg, Color::Rgb(4, 5, 6));
    }

    #[test]
    fn sgr_unset_attributes() {
        let t = term(10, 1, "\x1b[1;3;7m\x1b[22;27mA");
        let flags = t.grid().cell(0, 0).flags;
        assert!(flags.contains(Flags::ITALIC));
        assert!(!flags.contains(Flags::BOLD));
        assert!(!flags.contains(Flags::INVERSE));
    }

    #[test]
    fn linefeed_on_last_row_scrolls() {
        let t = term(5, 3, "1\r\n2\r\n3\r\n4");
        assert_eq!(t.snapshot_text(), "2\n3\n4");
        assert_eq!(pos(&t), (2, 1));
    }

    #[test]
    fn scroll_region_limits_scrolling() {
        // Región en las filas 2–3 (1-based) de 4; las filas 1 y 4 no se mueven.
        let mut t = term(5, 4, "top\r\na\r\nb\r\nbot");
        t.advance(b"\x1b[2;3r");
        assert_eq!(pos(&t), (0, 0));
        t.advance(b"\x1b[3;1H\nc");
        assert_eq!(t.snapshot_text(), "top\nb\nc\nbot");
        // RI en la primera fila de la región baja el contenido.
        t.advance(b"\x1b[2;1H\x1bM");
        assert_eq!(t.snapshot_text(), "top\n\nb\nbot");
    }

    #[test]
    fn scroll_up_and_down() {
        let mut t = term(3, 3, "a\r\nb\r\nc");
        t.advance(b"\x1b[S");
        assert_eq!(t.snapshot_text(), "b\nc");
        t.advance(b"\x1b[2T");
        assert_eq!(t.snapshot_text(), "\n\nb");
    }

    #[test]
    fn insert_and_delete_lines() {
        let mut t = term(3, 4, "a\r\nb\r\nc\r\nd\x1b[2;2H\x1b[L");
        assert_eq!(t.snapshot_text(), "a\n\nb\nc");
        assert_eq!(pos(&t), (1, 0));
        t.advance(b"\x1b[2M");
        assert_eq!(t.snapshot_text(), "a\nc");
    }

    #[test]
    fn insert_and_delete_lines_respect_region() {
        let mut t = term(3, 4, "a\r\nb\r\nc\r\nd\x1b[1;3r\x1b[1;1H\x1b[L");
        assert_eq!(t.snapshot_text(), "\na\nb\nd");
        // Fuera de la región no hace nada.
        t.advance(b"\x1b[4;1H\x1b[M");
        assert_eq!(t.snapshot_text(), "\na\nb\nd");
    }

    #[test]
    fn insert_delete_and_erase_chars() {
        assert_eq!(
            term(6, 1, "abcdef\x1b[1;2H\x1b[2@").snapshot_text(),
            "a  bcd"
        );
        assert_eq!(term(6, 1, "abcdef\x1b[1;2H\x1b[2P").snapshot_text(), "adef");
        assert_eq!(
            term(6, 1, "abcdef\x1b[1;2H\x1b[2X").snapshot_text(),
            "a  def"
        );
    }

    #[test]
    fn save_and_restore_cursor() {
        let t = term(10, 3, "\x1b[2;3H\x1b[31m\x1b7\x1b[H\x1b[0mx\x1b8y");
        assert_eq!(t.grid().cell(1, 2).ch, 'y');
        assert_eq!(t.grid().cell(1, 2).fg, Color::Indexed(1));
    }

    #[test]
    fn bell_is_reported_once() {
        let mut t = term(5, 1, "\x07");
        assert!(t.take_bell());
        assert!(!t.take_bell());
    }

    #[test]
    fn unknown_sequences_are_ignored() {
        let t = term(
            10,
            2,
            "a\x1b[?1049h\x1b[>4;1m\x1b]0;título\x07\x1bPqdcs\x1b\\\x1b[99zb\x1b(Bc",
        );
        assert_eq!(t.snapshot_text(), "abc");
    }

    #[test]
    fn invalid_utf8_does_not_panic() {
        let mut t = Term::new(5, 2);
        t.advance(&[b'a', 0xFF, 0xC3, b'b']);
        assert_eq!(t.grid().cell(0, 0).ch, 'a');
    }

    #[test]
    fn resize_clamps_cursor() {
        let mut t = term(10, 5, "\x1b[5;10H");
        t.resize(4, 2);
        assert_eq!(pos(&t), (1, 3));
        assert_eq!(t.grid().cols(), 4);
        assert_eq!(t.grid().rows(), 2);
    }
}
