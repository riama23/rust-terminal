//! Estado del terminal: interpreta las acciones de `vte` sobre la cuadrícula.

use std::fmt::Write as _;
use std::sync::OnceLock;

use unicode_width::UnicodeWidthChar;
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

/// Modos que afectan a la entrada o al renderizado y que tiene que consultar
/// quien usa el núcleo.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Modes {
    /// DECCKM (`?1`): las flechas se envían como SS3 en lugar de CSI.
    pub app_cursor: bool,
    /// DECAWM (`?7`).
    pub autowrap: bool,
    /// DECTCEM (`?25`).
    pub cursor_visible: bool,
    /// `?2004`: el texto pegado va entre `ESC[200~` y `ESC[201~`.
    pub bracketed_paste: bool,
    /// Pantalla alternativa activa (`?47`, `?1047`, `?1049`).
    pub alt_screen: bool,
}

impl Default for Modes {
    fn default() -> Self {
        Modes {
            app_cursor: false,
            autowrap: true,
            cursor_visible: true,
            bracketed_paste: false,
            alt_screen: false,
        }
    }
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
    line_drawing: bool,
}

/// Respuesta a DA1: VT220 con color ANSI, como xterm.
const DA1_RESPONSE: &[u8] = b"\x1b[?62;22c";
/// Respuesta a DA2: tipo 1 (VT220), versión 10, sin cartucho.
const DA2_RESPONSE: &[u8] = b"\x1b[>1;10;0c";

/// Estado sobre el que actúa el parser. Está separado de `Term` para poder
/// tomar prestados a la vez el parser y el estado en `advance`.
struct Screen {
    /// Pantalla activa.
    grid: Grid,
    /// La otra pantalla (la principal mientras la alternativa está activa).
    inactive_grid: Grid,
    cursor: Cursor,
    pen: Pen,
    /// Región de scroll `[top, bottom]`, inclusive.
    top: usize,
    bottom: usize,
    /// Cursor guardado (DECSC) de la pantalla activa y de la inactiva.
    saved: Option<Saved>,
    inactive_saved: Option<Saved>,
    modes: Modes,
    /// G0 apunta al juego de gráficos de línea DEC (`ESC ( 0`).
    line_drawing: bool,
    title: Option<String>,
    title_changed: bool,
    /// Bytes que hay que devolver a la aplicación por la PTY.
    responses: Vec<u8>,
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
            screen: Screen::new(cols, rows),
        }
    }

    /// Aplica un lote de bytes leídos de la PTY.
    pub fn advance(&mut self, bytes: &[u8]) {
        self.parser.advance(&mut self.screen, bytes);
    }

    pub fn grid(&self) -> &Grid {
        &self.screen.grid
    }

    /// Llama a `f` con cada fila de la pantalla activa que cambió desde la
    /// última llamada.
    pub fn drain_damage(&mut self, f: impl FnMut(usize, &[Cell])) {
        self.screen.grid.drain_damage(f);
    }

    pub fn cursor(&self) -> Cursor {
        self.screen.cursor
    }

    pub fn modes(&self) -> Modes {
        self.screen.modes
    }

    pub fn title(&self) -> Option<&str> {
        self.screen.title.as_deref()
    }

    /// Devuelve si el título cambió desde la última llamada.
    pub fn take_title_changed(&mut self) -> bool {
        std::mem::take(&mut self.screen.title_changed)
    }

    /// Bytes pendientes de enviar a la aplicación (respuestas a DA, DSR…).
    pub fn take_responses(&mut self) -> Vec<u8> {
        std::mem::take(&mut self.screen.responses)
    }

    /// Devuelve si sonó la campana desde la última llamada y la rearma.
    pub fn take_bell(&mut self) -> bool {
        std::mem::take(&mut self.screen.bell)
    }

    /// Cambia el tamaño de ambas pantallas. La región de scroll vuelve a ser
    /// la pantalla entera.
    pub fn resize(&mut self, cols: usize, rows: usize) {
        let s = &mut self.screen;
        s.grid.resize(cols, rows);
        s.inactive_grid.resize(cols, rows);
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
                let line: String = grid
                    .row(r)
                    .iter()
                    .filter(|c| !c.flags.contains(Flags::WIDE_SPACER))
                    .map(|c| c.ch)
                    .collect();
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

/// Registra en stderr una secuencia que el núcleo no interpreta, si la
/// variable de entorno `RT_LOG_UNHANDLED` está definida.
fn log_unhandled(args: std::fmt::Arguments) {
    static ENABLED: OnceLock<bool> = OnceLock::new();
    if *ENABLED.get_or_init(|| std::env::var_os("RT_LOG_UNHANDLED").is_some()) {
        eprintln!("[sin soporte] {args}");
    }
}

fn describe_params(params: &Params) -> String {
    let mut out = String::new();
    for (i, p) in params.iter().enumerate() {
        if i > 0 {
            out.push(';');
        }
        for (j, v) in p.iter().enumerate() {
            if j > 0 {
                out.push(':');
            }
            let _ = write!(out, "{v}");
        }
    }
    out
}

/// Traduce un carácter del juego de gráficos de línea DEC (G0 = `ESC ( 0`).
fn dec_line_drawing(c: char) -> char {
    match c {
        '_' => ' ',
        '`' => '◆',
        'a' => '▒',
        'b' => '␉',
        'c' => '␌',
        'd' => '␍',
        'e' => '␊',
        'f' => '°',
        'g' => '±',
        'h' => '␤',
        'i' => '␋',
        'j' => '┘',
        'k' => '┐',
        'l' => '┌',
        'm' => '└',
        'n' => '┼',
        'o' => '⎺',
        'p' => '⎻',
        'q' => '─',
        'r' => '⎼',
        's' => '⎽',
        't' => '├',
        'u' => '┤',
        'v' => '┴',
        'w' => '┬',
        'x' => '│',
        'y' => '≤',
        'z' => '≥',
        '{' => 'π',
        '|' => '≠',
        '}' => '£',
        '~' => '·',
        c => c,
    }
}

impl Screen {
    fn new(cols: usize, rows: usize) -> Screen {
        Screen {
            grid: Grid::new(cols, rows),
            inactive_grid: Grid::new(cols, rows),
            cursor: Cursor::default(),
            pen: Pen::default(),
            top: 0,
            bottom: rows - 1,
            saved: None,
            inactive_saved: None,
            modes: Modes::default(),
            line_drawing: false,
            title: None,
            title_changed: false,
            responses: Vec::new(),
            bell: false,
        }
    }

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

    /// Escribe una celda deshaciendo el par ancho que pudiera romper: si se
    /// pisa una mitad de un carácter de doble ancho, la otra queda en blanco.
    fn write_cell(&mut self, row: usize, col: usize, cell: Cell) {
        let old = self.grid.cell(row, col).flags;
        if old.contains(Flags::WIDE) && col + 1 < self.cols() {
            let next = self.grid.cell_mut(row, col + 1);
            next.ch = ' ';
            next.flags.remove(Flags::WIDE_SPACER);
        }
        if old.contains(Flags::WIDE_SPACER) && col > 0 {
            let prev = self.grid.cell_mut(row, col - 1);
            prev.ch = ' ';
            prev.flags.remove(Flags::WIDE);
        }
        *self.grid.cell_mut(row, col) = cell;
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
            line_drawing: self.line_drawing,
        });
    }

    fn restore_cursor(&mut self) {
        match self.saved {
            Some(saved) => {
                self.pen = saved.pen;
                self.line_drawing = saved.line_drawing;
                self.move_to(saved.cursor.row, saved.cursor.col);
                self.cursor.wrap_pending = saved.cursor.wrap_pending;
            }
            None => {
                self.pen = Pen::default();
                self.line_drawing = false;
                self.move_to(0, 0);
            }
        }
    }

    /// Cambia entre la pantalla principal y la alternativa. Cada pantalla
    /// conserva su propio cursor guardado.
    fn switch_screen(&mut self, alt: bool, clear_alt: bool) {
        if self.modes.alt_screen == alt {
            return;
        }
        std::mem::swap(&mut self.grid, &mut self.inactive_grid);
        std::mem::swap(&mut self.saved, &mut self.inactive_saved);
        self.modes.alt_screen = alt;
        if alt && clear_alt {
            let rows = self.rows();
            self.grid.clear_rows(0, rows, Cell::default());
        }
        self.grid.mark_all_dirty();
    }

    fn set_private_mode(&mut self, params: &Params, on: bool) {
        for p in params.iter() {
            match p[0] {
                1 => self.modes.app_cursor = on,
                7 => {
                    self.modes.autowrap = on;
                    if !on {
                        self.cursor.wrap_pending = false;
                    }
                }
                // Parpadeo del cursor: no se implementa (rompería el reposo sin frames).
                12 => {}
                25 => self.modes.cursor_visible = on,
                47 => self.switch_screen(on, false),
                1047 => self.switch_screen(on, true),
                1049 => {
                    if on {
                        self.save_cursor();
                        self.switch_screen(true, true);
                    } else {
                        self.switch_screen(false, false);
                        self.restore_cursor();
                    }
                }
                2004 => self.modes.bracketed_paste = on,
                n => log_unhandled(format_args!(
                    "modo privado ?{n} {}",
                    if on { "h" } else { "l" }
                )),
            }
        }
    }

    fn device_status_report(&mut self, kind: u16) {
        match kind {
            5 => self.responses.extend_from_slice(b"\x1b[0n"),
            6 => {
                let Cursor { row, col, .. } = self.cursor;
                let _ = write!(Bytes(&mut self.responses), "\x1b[{};{}R", row + 1, col + 1);
            }
            n => log_unhandled(format_args!("DSR {n}")),
        }
    }

    fn reset(&mut self) {
        let title = self.title.take();
        *self = Screen::new(self.cols(), self.rows());
        self.title = title;
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
                n => log_unhandled(format_args!("SGR {n}")),
            }
            i += 1;
        }
    }
}

/// Adaptador para usar `write!` sobre un `Vec<u8>` sin pasar por `io::Write`.
struct Bytes<'a>(&'a mut Vec<u8>);

impl std::fmt::Write for Bytes<'_> {
    fn write_str(&mut self, s: &str) -> std::fmt::Result {
        self.0.extend_from_slice(s.as_bytes());
        Ok(())
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
        let c = if self.line_drawing {
            dec_line_drawing(c)
        } else {
            c
        };
        // Los caracteres de ancho 0 (combinantes, ZWJ…) se descartan por ahora.
        let width = match c.width() {
            Some(w @ 1..=2) => w,
            _ => return,
        };
        let cols = self.cols();
        if width > cols {
            return;
        }
        if self.cursor.wrap_pending {
            self.cursor.col = 0;
            self.linefeed();
        }
        // Un carácter ancho que no cabe en la última columna salta de línea antes.
        if width == 2 && self.cursor.col + 1 == cols {
            if self.modes.autowrap {
                let blank = self.blank();
                self.write_cell(self.cursor.row, self.cursor.col, blank);
                self.cursor.col = 0;
                self.linefeed();
            } else {
                self.cursor.col = cols - 2;
            }
        }

        let Pen { fg, bg, flags } = self.pen;
        let Cursor { row, col, .. } = self.cursor;
        let mut cell_flags = flags;
        if width == 2 {
            cell_flags.insert(Flags::WIDE);
        }
        self.write_cell(
            row,
            col,
            Cell {
                ch: c,
                fg,
                bg,
                flags: cell_flags,
            },
        );
        if width == 2 {
            self.write_cell(
                row,
                col + 1,
                Cell {
                    ch: ' ',
                    fg,
                    bg,
                    flags: flags | Flags::WIDE_SPACER,
                },
            );
        }

        let next = col + width;
        if next >= cols {
            self.cursor.col = cols - 1;
            self.cursor.wrap_pending = self.modes.autowrap;
        } else {
            self.cursor.col = next;
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
            // SO/SI (cambio a G1/G0): sin soporte de G1.
            0x0E | 0x0F => {}
            b => log_unhandled(format_args!("C0 0x{b:02x}")),
        }
    }

    fn esc_dispatch(&mut self, intermediates: &[u8], _ignore: bool, byte: u8) {
        match (intermediates, byte) {
            ([], b'7') => self.save_cursor(),
            ([], b'8') => self.restore_cursor(),
            ([], b'D') => self.linefeed(),
            ([], b'E') => {
                self.cursor.col = 0;
                self.linefeed();
            }
            ([], b'M') => self.reverse_index(),
            ([], b'c') => self.reset(),
            // ST: cierre de OSC/DCS, que `vte` ya ha despachado.
            ([], b'\\') => {}
            // Teclado numérico en modo aplicación/numérico: sin efecto por ahora.
            ([], b'=' | b'>') => {}
            ([b'('], b'0') => self.line_drawing = true,
            ([b'('], b'B') => self.line_drawing = false,
            _ => log_unhandled(format_args!(
                "ESC {}{}",
                String::from_utf8_lossy(intermediates),
                byte as char
            )),
        }
    }

    fn osc_dispatch(&mut self, params: &[&[u8]], _bell_terminated: bool) {
        match params.first() {
            Some(&(b"0" | b"2")) => {
                let title = params[1..]
                    .iter()
                    .map(|p| String::from_utf8_lossy(p))
                    .collect::<Vec<_>>()
                    .join(";");
                self.title = Some(title);
                self.title_changed = true;
            }
            // Título del icono: no aplica en macOS.
            Some(&b"1") => {}
            Some(kind) => log_unhandled(format_args!("OSC {}", String::from_utf8_lossy(kind))),
            None => {}
        }
    }

    fn hook(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        log_unhandled(format_args!(
            "DCS {}{}{action}",
            String::from_utf8_lossy(intermediates),
            describe_params(params)
        ));
    }

    fn csi_dispatch(&mut self, params: &Params, intermediates: &[u8], _ignore: bool, action: char) {
        match (intermediates, action) {
            ([], _) => {}
            ([b'?'], 'h') => return self.set_private_mode(params, true),
            ([b'?'], 'l') => return self.set_private_mode(params, false),
            ([b'>'], 'c') => return self.responses.extend_from_slice(DA2_RESPONSE),
            _ => {
                return log_unhandled(format_args!(
                    "CSI {}{}{action}",
                    String::from_utf8_lossy(intermediates),
                    describe_params(params)
                ));
            }
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
            'L' | 'M' => {}
            'S' => {
                let blank = self.blank();
                self.grid.scroll_up(self.top, self.bottom, n, blank);
            }
            'T' => {
                let blank = self.blank();
                self.grid.scroll_down(self.top, self.bottom, n, blank);
            }
            'c' if raw(params, 0) == 0 => self.responses.extend_from_slice(DA1_RESPONSE),
            'n' => self.device_status_report(raw(params, 0)),
            'r' => self.set_scroll_region(params),
            'm' => self.sgr(params),
            's' => self.save_cursor(),
            'u' => self.restore_cursor(),
            _ => log_unhandled(format_args!("CSI {}{action}", describe_params(params))),
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

    fn dirty_rows(t: &mut Term) -> Vec<usize> {
        let mut rows = Vec::new();
        t.drain_damage(|i, _| rows.push(i));
        rows
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
    fn autowrap_disabled_overwrites_last_column() {
        let t = term(3, 2, "\x1b[?7labcde");
        assert_eq!(t.snapshot_text(), "abe");
        assert_eq!(pos(&t), (0, 2));
        assert!(!t.modes().autowrap);
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
            "a\x1b[?9999h\x1b[>4;1m\x1b]777;x\x07\x1bPqdcs\x1b\\\x1b[99zb\x1b(Bc",
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

    #[test]
    fn wide_chars_take_two_cells() {
        let t = term(10, 1, "a日b");
        assert_eq!(t.snapshot_text(), "a日b");
        assert!(t.grid().cell(0, 1).flags.contains(Flags::WIDE));
        assert!(t.grid().cell(0, 2).flags.contains(Flags::WIDE_SPACER));
        assert_eq!(t.grid().cell(0, 3).ch, 'b');
        assert_eq!(pos(&t), (0, 4));
    }

    #[test]
    fn emoji_is_wide() {
        let t = term(10, 1, "🙂x");
        assert_eq!(t.grid().cell(0, 2).ch, 'x');
    }

    #[test]
    fn wide_char_at_last_column_wraps_first() {
        let t = term(4, 2, "abc日");
        assert_eq!(t.snapshot_text(), "abc\n日");
        assert_eq!(pos(&t), (1, 2));
    }

    #[test]
    fn wide_char_filling_last_two_columns_sets_wrap_pending() {
        let t = term(4, 2, "ab日");
        assert_eq!(pos(&t), (0, 3));
        assert!(t.cursor().wrap_pending);
    }

    #[test]
    fn overwriting_half_of_wide_char_clears_the_other_half() {
        // Pisar la primera mitad.
        let t = term(6, 1, "日本\x1b[1;1Hx");
        assert_eq!(t.snapshot_text(), "x 本");
        assert!(!t.grid().cell(0, 1).flags.contains(Flags::WIDE_SPACER));
        // Pisar la segunda mitad.
        let t = term(6, 1, "日本\x1b[1;2Hx");
        assert_eq!(t.snapshot_text(), " x本");
        assert!(!t.grid().cell(0, 0).flags.contains(Flags::WIDE));
    }

    #[test]
    fn zero_width_chars_are_dropped() {
        let t = term(5, 1, "e\u{301}x");
        assert_eq!(t.snapshot_text(), "ex");
    }

    #[test]
    fn alt_screen_1049_round_trip() {
        let mut t = term(10, 3, "main\r\nprompt$ ");
        let before = pos(&t);
        t.advance(b"\x1b[?1049h");
        assert!(t.modes().alt_screen);
        assert_eq!(t.snapshot_text(), "");
        t.advance(b"\x1b[2;2Hvim");
        assert_eq!(t.snapshot_text(), "\n vim");
        t.advance(b"\x1b[?1049l");
        assert!(!t.modes().alt_screen);
        assert_eq!(t.snapshot_text(), "main\nprompt$");
        assert_eq!(pos(&t), before);
    }

    #[test]
    fn alt_screen_47_keeps_contents_and_marks_dirty() {
        let mut t = term(5, 2, "\x1b[?47hx\x1b[?47l");
        dirty_rows(&mut t);
        t.advance(b"\x1b[?47h");
        assert_eq!(t.snapshot_text(), "x");
        assert_eq!(dirty_rows(&mut t), [0, 1]);
    }

    #[test]
    fn private_modes() {
        let mut t = Term::new(5, 1);
        assert_eq!(t.modes(), Modes::default());
        t.advance(b"\x1b[?1;2004h\x1b[?25l");
        let m = t.modes();
        assert!(m.app_cursor && m.bracketed_paste && !m.cursor_visible);
        t.advance(b"\x1b[?1;2004l\x1b[?25h");
        assert_eq!(t.modes(), Modes::default());
    }

    #[test]
    fn device_attributes_and_status_reports() {
        let mut t = term(10, 5, "\x1b[c\x1b[>c\x1b[5n\x1b[3;4H\x1b[6n");
        assert_eq!(
            t.take_responses(),
            b"\x1b[?62;22c\x1b[>1;10;0c\x1b[0n\x1b[3;4R".to_vec()
        );
        assert!(t.take_responses().is_empty());
    }

    #[test]
    fn title_from_osc() {
        let mut t = term(5, 1, "\x1b]2;mi;título\x07");
        assert_eq!(t.title(), Some("mi;título"));
        assert!(t.take_title_changed());
        assert!(!t.take_title_changed());
        t.advance(b"\x1b]0;otro\x1b\\");
        assert_eq!(t.title(), Some("otro"));
    }

    #[test]
    fn dec_line_drawing_charset() {
        let t = term(5, 1, "\x1b(0lqk\x1b(Bq");
        assert_eq!(t.snapshot_text(), "┌─┐q");
    }

    #[test]
    fn damage_tracks_changed_rows() {
        let mut t = Term::new(5, 3);
        dirty_rows(&mut t);
        t.advance(b"\x1b[2;1Hx");
        assert_eq!(dirty_rows(&mut t), [1]);
        assert!(dirty_rows(&mut t).is_empty());
    }
}
