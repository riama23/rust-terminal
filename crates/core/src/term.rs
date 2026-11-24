//! Estado del terminal: interpreta las acciones de `vte` sobre la cuadrícula.

use std::collections::HashMap;
use std::fmt::Write as _;
use std::sync::OnceLock;
use std::time::{Duration, Instant};

use unicode_width::UnicodeWidthChar;
use vte::{Params, Parser, Perform};

use crate::block::{self, Block};
use crate::grid::{Cell, Color, Flags, Grid, Line, Point};
use crate::links;
use crate::selection::{self, Selection, SelectionRange};

/// Líneas de historial de la pantalla principal (límite inicial, pendiente de medir).
pub const HISTORY_LIMIT: usize = 10_000;

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
    /// Qué eventos de ratón quiere la aplicación (`?1000`, `?1002`, `?1003`).
    pub mouse: MouseMode,
    /// `?1006`: coordenadas del ratón en formato SGR.
    pub mouse_sgr: bool,
    /// `?1004`: avisar a la aplicación al ganar y perder el foco.
    pub focus_events: bool,
    /// `?1007`: en la pantalla alternativa la rueda envía flechas.
    pub alt_scroll: bool,
    /// Banderas activas del protocolo de teclado de kitty (0 = clásico).
    pub kitty_flags: u8,
    /// Nivel de modifyOtherKeys de xterm (`CSI > 4 ; n m`).
    pub modify_other_keys: u8,
}

/// Eventos de ratón que pide la aplicación.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
pub enum MouseMode {
    #[default]
    Off,
    /// `?1000`: pulsar y soltar.
    Click,
    /// `?1002`: además, mover con un botón pulsado.
    Drag,
    /// `?1003`: cualquier movimiento.
    Motion,
}

/// Algo que la app tiene que hacer fuera de la cuadrícula.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum TermEvent {
    /// OSC 52: copiar al portapapeles.
    Clipboard(String),
    /// OSC 52 con `?`: la aplicación quiere leer el portapapeles. La app
    /// decide si responde (con [`Term::clipboard_response`]).
    ClipboardRequest { selector: String },
    /// OSC 9, 777 o 99: notificación del sistema.
    Notification { title: Option<String>, body: String },
}

/// Tiempo máximo que se respeta el modo DEC 2026 sin cerrar.
pub const SYNC_TIMEOUT: Duration = Duration::from_millis(150);

/// Modos que entiende DECRQM (`CSI ? Ps $ p`).
const KNOWN_PRIVATE_MODES: [u16; 14] = [
    1, 7, 25, 47, 1000, 1002, 1003, 1004, 1006, 1007, 1047, 1049, 2004, 2026,
];

impl Default for Modes {
    fn default() -> Self {
        Modes {
            app_cursor: false,
            autowrap: true,
            cursor_visible: true,
            bracketed_paste: false,
            alt_screen: false,
            mouse: MouseMode::Off,
            mouse_sgr: false,
            focus_events: false,
            alt_scroll: true,
            kitty_flags: 0,
            modify_other_keys: 0,
        }
    }
}

/// Atributos con los que se escriben las celdas nuevas.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
struct Pen {
    fg: Color,
    bg: Color,
    flags: Flags,
    /// Enlace OSC 8 activo.
    link: u16,
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
    /// Líneas de historial de la pantalla principal.
    history_limit: usize,
    bell: bool,
    /// Al redimensionar con el cursor en el prompt, borrarlo para que la shell
    /// lo redibuje limpio (ver `Term::resize`).
    clear_prompt_on_resize: bool,
    /// Líneas que la vista está desplazada hacia el historial (0 = abajo del todo).
    display_offset: usize,
    /// La vista cambió entera (desplazamiento, reflow…) y hay que repintar todo.
    view_dirty: bool,
    /// Bloques de comandos (OSC 133), del más antiguo al más reciente.
    blocks: Vec<Block>,
    next_block_id: u64,
    /// Directorio de trabajo comunicado por OSC 7.
    cwd: Option<String>,
    selection: Option<Selection>,
    /// Pilas del protocolo de teclado de kitty de la pantalla activa y de la otra.
    kitty_stack: Vec<u8>,
    inactive_kitty_stack: Vec<u8>,
    /// DEC 2026 activo desde este instante.
    sync_since: Option<Instant>,
    /// Enlaces OSC 8: `links[id - 1]`, con índice inverso para reutilizar ids.
    links: Vec<String>,
    link_ids: HashMap<String, u16>,
    events: Vec<TermEvent>,
}

pub struct Term {
    parser: Parser,
    screen: Screen,
}

impl Term {
    pub fn new(cols: usize, rows: usize) -> Term {
        Term::with_history_limit(cols, rows, HISTORY_LIMIT)
    }

    /// Como [`Term::new`], con un límite de historial propio.
    pub fn with_history_limit(cols: usize, rows: usize, history_limit: usize) -> Term {
        Term {
            parser: Parser::new(),
            screen: Screen::new(cols, rows, history_limit),
        }
    }

    /// Aplica un lote de bytes leídos de la PTY.
    pub fn advance(&mut self, bytes: &[u8]) {
        let start = self.screen.grid.screen_start();
        self.parser.advance(&mut self.screen, bytes);
        if self.screen.main_grid().first_line() > 0 {
            self.screen.prune_evicted();
        }
        let s = &mut self.screen;
        if s.display_offset > 0 {
            // Si se está mirando el historial, la vista no se mueve con la salida nueva.
            let pushed = s.grid.screen_start().saturating_sub(start) as usize;
            let offset = (s.display_offset + pushed).min(s.grid.history_len());
            if offset != s.display_offset || pushed > 0 {
                s.view_dirty = true;
            }
            s.display_offset = offset;
        }
    }

    /// Eventos pendientes para la app (portapapeles, notificaciones).
    pub fn take_events(&mut self) -> Vec<TermEvent> {
        std::mem::take(&mut self.screen.events)
    }

    /// Respuesta a una lectura de portapapeles (OSC 52) con `text`.
    pub fn clipboard_response(selector: &str, text: &str) -> Vec<u8> {
        let selector = if selector.is_empty() { "c" } else { selector };
        format!(
            "\x1b]52;{selector};{}\x1b\\",
            links::base64_encode(text.as_bytes())
        )
        .into_bytes()
    }

    /// DEC 2026 activo (y sin pasar del límite de tiempo): la app debe
    /// seguir mostrando el último frame de este panel.
    pub fn synchronized(&self) -> bool {
        self.screen
            .sync_since
            .is_some_and(|t| t.elapsed() < SYNC_TIMEOUT)
    }

    /// URL de un enlace OSC 8 por su id.
    pub fn link(&self, id: u16) -> Option<&str> {
        let i = (id as usize).checked_sub(1)?;
        self.screen.links.get(i).map(String::as_str)
    }

    /// Enlace bajo un punto: primero OSC 8 y, si no hay, una URL en el texto
    /// de su línea lógica. Devuelve la URL y su rango en esa línea.
    pub fn link_at(&self, at: Point) -> Option<(String, SelectionRange)> {
        let grid = &self.screen.grid;
        let cell = grid.line(at.line)?.cells.get(at.col)?;
        if cell.link != 0 {
            let url = self.link(cell.link)?.to_owned();
            // Extensión del enlace en la fila (celdas contiguas con el mismo id).
            let row = &grid.line(at.line)?.cells;
            let start = (0..=at.col)
                .rev()
                .take_while(|&c| row[c].link == cell.link)
                .last()?;
            let end = (at.col..row.len())
                .take_while(|&c| row[c].link == cell.link)
                .last()?;
            let range = SelectionRange {
                start: selection::point(at.line, start),
                end: selection::point(at.line, end),
                block: false,
            };
            return Some((url, range));
        }
        // Línea lógica que contiene el punto.
        let mut first = at.line;
        while first > grid.first_line() && grid.line(first - 1).is_some_and(|r| r.wrapped) {
            first -= 1;
        }
        let mut chars = Vec::new();
        let mut positions = Vec::new();
        let mut line = first;
        while let Some(row) = grid.line(line) {
            for (col, c) in row.cells.iter().enumerate() {
                if !c.flags.contains(Flags::WIDE_SPACER) {
                    chars.push(grid.base_char(c.ch));
                    positions.push((line, col));
                }
            }
            if !row.wrapped {
                break;
            }
            line += 1;
        }
        let here = positions
            .iter()
            .position(|&(l, c)| l == at.line && (c == at.col || c + 1 == at.col))?;
        let (a, b) = links::find_urls(&chars)
            .into_iter()
            .find(|&(a, b)| a <= here && here < b)?;
        let url: String = chars[a..b].iter().collect();
        let (sl, sc) = positions[a];
        let (el, ec) = positions[b - 1];
        Some((
            url,
            SelectionRange {
                start: selection::point(sl, sc),
                end: selection::point(el, ec),
                block: false,
            },
        ))
    }

    /// Bloques de comandos (OSC 133), del más antiguo al más reciente.
    pub fn blocks(&self) -> &[Block] {
        &self.screen.blocks
    }

    /// Directorio actual según OSC 7.
    pub fn cwd(&self) -> Option<&str> {
        self.screen.cwd.as_deref()
    }

    /// Activa o desactiva el borrado del prompt al redimensionar.
    pub fn set_clear_prompt_on_resize(&mut self, on: bool) {
        self.screen.clear_prompt_on_resize = on;
    }

    /// Texto de la salida de un bloque terminado (sin el prompt siguiente).
    pub fn block_output(&self, block: &Block) -> Option<String> {
        let (start, end) = (block.output?, block.end?);
        if (end.line, end.col) <= (start.line, start.col) {
            return Some(String::new());
        }
        // El final es exclusivo: la última celda es la anterior a `end`.
        let last = if end.col > 0 {
            selection::point(end.line, end.col - 1)
        } else {
            selection::point(end.line - 1, self.screen.grid.cols() - 1)
        };
        let range = SelectionRange {
            start: selection::point(start.line, start.col),
            end: last,
            block: false,
        };
        let mut text = selection::text_in_range(&self.screen.grid, range);
        while text.ends_with('\n') {
            text.pop();
        }
        Some(text)
    }

    /// Último bloque con la salida terminada.
    pub fn last_finished_block(&self) -> Option<&Block> {
        self.screen.blocks.iter().rev().find(|b| b.is_finished())
    }

    /// Línea absoluta de la fila `row` de la vista.
    pub fn visible_line(&self, row: usize) -> Line {
        self.screen.grid.screen_start() - self.screen.display_offset as Line + row as Line
    }

    /// Desplaza la vista para que `line` quede arriba (o lo más cerca posible).
    pub fn scroll_to_line(&mut self, line: Line) {
        let s = &mut self.screen;
        let offset = s.grid.screen_start().saturating_sub(line) as usize;
        let offset = offset.min(s.grid.history_len());
        if offset != s.display_offset {
            s.display_offset = offset;
            s.view_dirty = true;
        }
    }

    pub fn selection(&self) -> Option<Selection> {
        self.screen.selection
    }

    pub fn set_selection(&mut self, selection: Option<Selection>) {
        self.screen.selection = selection;
    }

    /// Rango normalizado de la selección actual.
    pub fn selection_range(&self) -> Option<SelectionRange> {
        self.screen.selection.map(|s| s.range(&self.screen.grid))
    }

    pub fn selection_text(&self) -> Option<String> {
        let range = self.selection_range()?;
        Some(selection::text_in_range(&self.screen.grid, range))
    }

    /// Líneas que la vista está desplazada hacia el historial.
    pub fn display_offset(&self) -> usize {
        self.screen.display_offset
    }

    /// Desplaza la vista `delta` líneas (positivo = hacia el historial).
    pub fn scroll_display(&mut self, delta: isize) {
        let s = &mut self.screen;
        let max = s.grid.history_len() as isize;
        let offset = (s.display_offset as isize + delta).clamp(0, max) as usize;
        if offset != s.display_offset {
            s.display_offset = offset;
            s.view_dirty = true;
        }
    }

    pub fn scroll_to_bottom(&mut self) {
        self.scroll_display(isize::MIN / 2);
    }

    /// Fila `i` de la vista (teniendo en cuenta el desplazamiento). Las filas
    /// del historial pueden ser más cortas que el ancho.
    pub fn visible_row(&self, i: usize) -> &[Cell] {
        let s = &self.screen;
        if s.display_offset == 0 {
            return s.grid.row(i);
        }
        let line = s.grid.screen_start() - s.display_offset as u64 + i as u64;
        s.grid.line(line).map_or(&[], |r| &r.cells)
    }

    pub fn grid(&self) -> &Grid {
        &self.screen.grid
    }

    /// Llama a `f` con cada fila de la vista que cambió desde la última
    /// llamada. Las filas del historial pueden ser más cortas que el ancho.
    pub fn drain_damage(&mut self, mut f: impl FnMut(usize, &[Cell])) {
        let s = &mut self.screen;
        if s.display_offset == 0 && !s.view_dirty {
            s.grid.drain_damage(f);
            return;
        }
        if s.view_dirty || s.grid.any_dirty() {
            for i in 0..s.grid.rows() {
                f(i, self.visible_row(i));
            }
            let s = &mut self.screen;
            s.grid.clear_damage();
            s.view_dirty = false;
        }
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
        if s.modes.alt_screen {
            // La alternativa se recorta; la principal (inactiva) se reajusta
            // usando su cursor guardado como referencia.
            s.grid.resize_truncate(cols, rows);
            let main = &mut s.inactive_grid;
            let mut points: Vec<Point> = s
                .inactive_saved
                .iter()
                .map(|sv| Point {
                    line: main.screen_start() + sv.cursor.row as u64,
                    col: sv.cursor.col,
                    pending: sv.cursor.wrap_pending,
                })
                .collect();
            main.resize_reflow(cols, rows, &mut points);
            if let (Some(sv), Some(p)) = (s.inactive_saved.as_mut(), points.first()) {
                sv.cursor.row = (p.line.saturating_sub(main.screen_start()) as usize).min(rows - 1);
                sv.cursor.col = p.col.min(cols - 1);
                sv.cursor.wrap_pending = p.pending;
            }
            s.cursor.row = s.cursor.row.min(rows - 1);
            s.cursor.col = s.cursor.col.min(cols - 1);
            s.cursor.wrap_pending = false;
        } else {
            // Si el cursor está en el prompt, la shell lo redibuja al recibir
            // SIGWINCH: se borra desde la marca A para no dejar restos al reajustar.
            if s.clear_prompt_on_resize
                && let Some(b) = s.blocks.last().filter(|b| b.output.is_none())
                && b.prompt.line >= s.grid.screen_start()
            {
                let row = (b.prompt.line - s.grid.screen_start()) as usize;
                if row <= s.cursor.row {
                    let total = s.grid.rows();
                    s.grid.clear_rows(row, total, Cell::default());
                    s.cursor = Cursor {
                        row,
                        col: 0,
                        wrap_pending: false,
                    };
                }
            }
            let start = s.grid.screen_start();
            let mut points = vec![Point {
                line: start + s.cursor.row as u64,
                col: s.cursor.col,
                pending: s.cursor.wrap_pending,
            }];
            if let Some(sv) = s.saved {
                points.push(Point {
                    line: start + sv.cursor.row as u64,
                    col: sv.cursor.col,
                    pending: sv.cursor.wrap_pending,
                });
            }
            let fixed = points.len();
            for b in &mut s.blocks {
                points.extend(b.points_mut().map(|p| *p));
            }
            s.grid.resize_reflow(cols, rows, &mut points);
            let mut moved = points[fixed..].iter();
            for b in &mut s.blocks {
                for p in b.points_mut() {
                    if let Some(np) = moved.next() {
                        *p = *np;
                    }
                }
            }
            let start = s.grid.screen_start();
            s.cursor.row = (points[0].line.saturating_sub(start) as usize).min(rows - 1);
            s.cursor.col = points[0].col.min(cols - 1);
            s.cursor.wrap_pending = points[0].pending && s.modes.autowrap;
            if let (Some(sv), Some(p)) = (s.saved.as_mut(), points.get(1)) {
                sv.cursor.row = (p.line.saturating_sub(start) as usize).min(rows - 1);
                sv.cursor.col = p.col.min(cols - 1);
                sv.cursor.wrap_pending = p.pending;
            }
            s.inactive_grid.resize_truncate(cols, rows);
        }
        s.top = 0;
        s.bottom = rows - 1;
        s.display_offset = s.display_offset.min(s.grid.history_len());
        s.view_dirty = true;
        s.selection = None;
    }

    /// La pantalla como texto: una línea por fila, sin espacios finales y sin
    /// filas vacías al final.
    pub fn snapshot_text(&self) -> String {
        let grid = &self.screen.grid;
        let mut lines: Vec<String> = (0..grid.rows())
            .map(|r| {
                let mut line = String::new();
                for c in grid
                    .row(r)
                    .iter()
                    .filter(|c| !c.flags.contains(Flags::WIDE_SPACER))
                {
                    line.push_str(&grid.char_text(c.ch));
                }
                line.trim_end_matches(' ').to_owned()
            })
            .collect();
        while lines.last().is_some_and(String::is_empty) {
            lines.pop();
        }
        lines.join("\n")
    }

    /// Las últimas `max_lines` líneas (historial incluido) como texto, sin
    /// espacios finales y sin líneas vacías al final. Es lo que lee la API de
    /// control (v2).
    pub fn recent_text(&self, max_lines: usize) -> String {
        let grid = &self.screen.grid;
        let max_lines = max_lines.max(1);
        let total = grid.history_len() + grid.rows();
        let last = grid.first_line() + total as Line - 1;
        // Se lee con margen (las filas vacías de debajo del cursor no cuentan).
        let want = (max_lines + grid.rows()) as Line;
        let first = last.saturating_sub(want - 1).max(grid.first_line());
        let range = SelectionRange {
            start: selection::point(first, 0),
            end: selection::point(last, grid.cols().saturating_sub(1)),
            block: false,
        };
        let text = selection::text_in_range(grid, range);
        let mut lines: Vec<&str> = text.lines().map(|l| l.trim_end_matches(' ')).collect();
        while lines.last().is_some_and(|l| l.is_empty()) {
            lines.pop();
        }
        let skip = lines.len().saturating_sub(max_lines);
        lines[skip..].join("\n")
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
    let text = args.to_string();
    if *ENABLED.get_or_init(|| std::env::var_os("RT_LOG_UNHANDLED").is_some()) {
        eprintln!("[sin soporte] {text}");
    }
    record_unhandled(text);
}

/// Cuántas secuencias no soportadas se recuerdan para los informes de fallos.
const RECENT_UNHANDLED: usize = 30;

fn recent() -> &'static std::sync::Mutex<std::collections::VecDeque<(String, u32)>> {
    static RECENT: OnceLock<std::sync::Mutex<std::collections::VecDeque<(String, u32)>>> =
        OnceLock::new();
    RECENT.get_or_init(Default::default)
}

/// Guarda la secuencia (las repetidas seguidas solo suman a su contador).
fn record_unhandled(text: String) {
    let Ok(mut list) = recent().lock() else {
        return;
    };
    match list.back_mut() {
        Some((last, n)) if *last == text => *n += 1,
        _ => {
            if list.len() == RECENT_UNHANDLED {
                list.pop_front();
            }
            list.push_back((text, 1));
        }
    }
}

/// Últimas secuencias que el núcleo no interpretó (de todas las sesiones),
/// con cuántas veces seguidas llegaron. Para los informes de fallos.
pub fn recent_unhandled() -> Vec<(String, u32)> {
    recent()
        .lock()
        .map(|l| l.iter().cloned().collect())
        .unwrap_or_default()
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
    fn new(cols: usize, rows: usize, history_limit: usize) -> Screen {
        Screen {
            history_limit,
            grid: Grid::with_history(cols, rows, history_limit),
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
            clear_prompt_on_resize: true,
            display_offset: 0,
            view_dirty: true,
            blocks: Vec::new(),
            next_block_id: 0,
            cwd: None,
            selection: None,
            kitty_stack: Vec::new(),
            inactive_kitty_stack: Vec::new(),
            sync_since: None,
            links: Vec::new(),
            link_ids: HashMap::new(),
            events: Vec::new(),
        }
    }

    /// Posición del cursor como punto absoluto.
    fn cursor_point(&self) -> Point {
        Point {
            line: self.grid.screen_start() + self.cursor.row as Line,
            col: self.cursor.col,
            pending: self.cursor.wrap_pending,
        }
    }

    /// Marca de integración con la shell (OSC 133).
    fn shell_mark(&mut self, params: &[&[u8]]) {
        if self.modes.alt_screen {
            return;
        }
        let here = self.cursor_point();
        let kind = params.get(1).and_then(|p| p.first()).copied();
        match kind {
            Some(b'A') => {
                match self.blocks.last_mut() {
                    // Prompt sin comando (Enter vacío o prompt redibujado): se sustituye.
                    Some(b) if b.output.is_none() => {
                        self.blocks.pop();
                    }
                    // Comando que terminó sin marca D: se cierra sin código.
                    Some(b) if !b.is_finished() => b.end = Some(here),
                    _ => {}
                }
                self.next_block_id += 1;
                self.blocks.push(Block {
                    id: self.next_block_id,
                    prompt: here,
                    input: None,
                    output: None,
                    end: None,
                    exit_code: None,
                    command: None,
                    cwd: self.cwd.clone(),
                    started_at: None,
                    finished_at: None,
                });
            }
            Some(b'B') => {
                if let Some(b) = self.blocks.last_mut().filter(|b| b.output.is_none()) {
                    b.input = Some(here);
                }
            }
            Some(b'C') => {
                if let Some(b) = self.blocks.last_mut().filter(|b| b.output.is_none()) {
                    b.output = Some(here);
                    b.started_at = Some(Instant::now());
                    b.command = params[2..].iter().find_map(|p| {
                        std::str::from_utf8(p)
                            .ok()?
                            .strip_prefix("cmdline_url=")
                            .map(block::percent_decode)
                    });
                }
            }
            Some(b'D') => {
                if let Some(b) = self
                    .blocks
                    .last_mut()
                    .filter(|b| b.output.is_some() && !b.is_finished())
                {
                    b.end = Some(here);
                    b.finished_at = Some(Instant::now());
                    b.exit_code = params
                        .get(2)
                        .and_then(|p| std::str::from_utf8(p).ok())
                        .and_then(|p| p.parse().ok());
                }
            }
            _ => log_unhandled(format_args!("OSC 133 {:?}", kind.map(char::from))),
        }
    }

    /// Descarta lo que apunta a líneas que ya salieron del historial.
    fn prune_evicted(&mut self) {
        let first = self.main_grid().first_line();
        // Los bloques sin terminar se conservan: aún esperan su marca D.
        self.blocks
            .retain(|b| b.end.is_none_or(|e| e.line >= first));
        if self
            .selection
            .is_some_and(|s| s.anchor.line.min(s.head.line) < first)
        {
            self.selection = None;
        }
    }

    /// La pantalla principal, esté activa o no.
    fn main_grid(&mut self) -> &mut Grid {
        if self.modes.alt_screen {
            &mut self.inactive_grid
        } else {
            &mut self.grid
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

    /// Une un carácter de ancho 0 a la celda escrita justo antes.
    fn combine(&mut self, c: char) {
        let Cursor {
            row,
            col,
            wrap_pending,
        } = self.cursor;
        let mut target = if wrap_pending {
            col
        } else if col > 0 {
            col - 1
        } else {
            return;
        };
        if target > 0
            && self
                .grid
                .cell(row, target)
                .flags
                .contains(Flags::WIDE_SPACER)
        {
            target -= 1;
        }
        let base = self.grid.cell(row, target).ch;
        if base == ' '
            && self
                .grid
                .cell(row, target)
                .flags
                .contains(Flags::WIDE_SPACER)
        {
            return;
        }
        let mut text = self.grid.char_text(base).into_owned();
        // Límite por si llega una ristra interminable de combinantes.
        if text.chars().count() >= 16 {
            return;
        }
        text.push(c);
        let id = self.grid.cluster(&text);
        self.grid.cell_mut(row, target).ch = id;
    }

    /// Escribe una celda deshaciendo el par ancho que pudiera romper: si se
    /// pisa una mitad de un carácter de doble ancho, la otra queda en blanco.
    fn write_cell(&mut self, row: usize, col: usize, cell: Cell) {
        let old = self.grid.cell(row, col).flags;
        if old.contains(Flags::WIDE)
            && col + 1 < self.cols()
            && self
                .grid
                .cell(row, col + 1)
                .flags
                .contains(Flags::WIDE_SPACER)
        {
            let next = self.grid.cell_mut(row, col + 1);
            next.ch = ' ';
            next.flags.remove(Flags::WIDE_SPACER);
        }
        if old.contains(Flags::WIDE_SPACER)
            && col > 0
            && self.grid.cell(row, col - 1).flags.contains(Flags::WIDE)
        {
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
            3 => {
                self.main_grid().clear_history();
                self.display_offset = 0;
                self.view_dirty = true;
            }
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
        std::mem::swap(&mut self.kitty_stack, &mut self.inactive_kitty_stack);
        self.modes.kitty_flags = self.kitty_stack.last().copied().unwrap_or(0);
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
                1000 | 1002 | 1003 => {
                    self.modes.mouse = match (on, p[0]) {
                        (false, _) => MouseMode::Off,
                        (true, 1000) => MouseMode::Click,
                        (true, 1002) => MouseMode::Drag,
                        _ => MouseMode::Motion,
                    };
                }
                1004 => self.modes.focus_events = on,
                1006 => self.modes.mouse_sgr = on,
                1007 => self.modes.alt_scroll = on,
                2026 => self.sync_since = on.then(Instant::now),
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

    /// DECRQM (`CSI ? Ps $ p`): 1 = activo, 2 = inactivo, 0 = desconocido.
    fn report_private_mode(&mut self, mode: u16) {
        let m = self.modes;
        let state = match mode {
            1 => Some(m.app_cursor),
            7 => Some(m.autowrap),
            25 => Some(m.cursor_visible),
            47 | 1047 | 1049 => Some(m.alt_screen),
            1000 => Some(m.mouse == MouseMode::Click),
            1002 => Some(m.mouse == MouseMode::Drag),
            1003 => Some(m.mouse == MouseMode::Motion),
            1004 => Some(m.focus_events),
            1006 => Some(m.mouse_sgr),
            1007 => Some(m.alt_scroll),
            2004 => Some(m.bracketed_paste),
            2026 => Some(self.sync_since.is_some()),
            _ => None,
        };
        debug_assert!(state.is_none() || KNOWN_PRIVATE_MODES.contains(&mode));
        let value = match state {
            Some(true) => 1,
            Some(false) => 2,
            None => 0,
        };
        let _ = write!(Bytes(&mut self.responses), "\x1b[?{mode};{value}$y");
    }

    /// Protocolo de teclado de kitty: `CSI > f u` (apilar), `CSI < n u`
    /// (desapilar), `CSI = f ; m u` (cambiar) y `CSI ? u` (consultar).
    fn kitty_keyboard(&mut self, kind: u8, params: &Params) {
        const MAX_DEPTH: usize = 16;
        match kind {
            b'>' => {
                if self.kitty_stack.len() >= MAX_DEPTH {
                    self.kitty_stack.remove(0);
                }
                self.kitty_stack.push(raw(params, 0) as u8 & 0x1f);
            }
            b'<' => {
                let n = param(params, 0, 1).min(self.kitty_stack.len());
                self.kitty_stack.truncate(self.kitty_stack.len() - n);
            }
            b'=' => {
                let flags = raw(params, 0) as u8 & 0x1f;
                let current = self.kitty_stack.last().copied().unwrap_or(0);
                let new = match param(params, 1, 1) {
                    2 => current | flags,
                    3 => current & !flags,
                    _ => flags,
                };
                match self.kitty_stack.last_mut() {
                    Some(top) => *top = new,
                    None => self.kitty_stack.push(new),
                }
            }
            _ => {
                let flags = self.kitty_stack.last().copied().unwrap_or(0);
                let _ = write!(Bytes(&mut self.responses), "\x1b[?{flags}u");
            }
        }
        self.modes.kitty_flags = self.kitty_stack.last().copied().unwrap_or(0);
    }

    /// OSC 8: `OSC 8 ; params ; URI`. Un URI vacío cierra el enlace.
    fn hyperlink(&mut self, params: &[&[u8]]) {
        let uri = params
            .get(2..)
            .map(|p| {
                p.iter()
                    .map(|b| String::from_utf8_lossy(b))
                    .collect::<Vec<_>>()
                    .join(";")
            })
            .unwrap_or_default();
        if uri.is_empty() {
            self.pen.link = 0;
            return;
        }
        let id = match self.link_ids.get(&uri) {
            Some(&id) => id,
            None if self.links.len() < u16::MAX as usize => {
                self.links.push(uri.clone());
                let id = self.links.len() as u16;
                self.link_ids.insert(uri, id);
                id
            }
            None => 0,
        };
        self.pen.link = id;
    }

    /// OSC 9, 777 y 99: notificaciones.
    fn notification(&mut self, params: &[&[u8]]) {
        let text = |i: usize| -> String {
            params
                .get(i..)
                .map(|p| {
                    p.iter()
                        .map(|b| String::from_utf8_lossy(b))
                        .collect::<Vec<_>>()
                        .join(";")
                })
                .unwrap_or_default()
        };
        let event = match params[0] {
            // OSC 9 ; texto (iTerm2). `OSC 9 ; 4 ; …` es una barra de progreso (ConEmu).
            b"9" if params.get(1) != Some(&&b"4"[..]) => Some(TermEvent::Notification {
                title: None,
                body: text(1),
            }),
            // OSC 777 ; notify ; título ; cuerpo (rxvt, Ghostty).
            b"777" if params.get(1) == Some(&&b"notify"[..]) => Some(TermEvent::Notification {
                title: params
                    .get(2)
                    .map(|t| String::from_utf8_lossy(t).into_owned()),
                body: text(3),
            }),
            // OSC 99 ; metadatos ; texto (kitty, forma sencilla).
            b"99" => {
                let meta = params
                    .get(1)
                    .map(|m| String::from_utf8_lossy(m))
                    .unwrap_or_default();
                let body = text(2);
                if meta.contains("p=title") {
                    Some(TermEvent::Notification {
                        title: Some(body),
                        body: String::new(),
                    })
                } else {
                    Some(TermEvent::Notification { title: None, body })
                }
            }
            _ => None,
        };
        if let Some(event) = event.filter(
            |e| !matches!(e, TermEvent::Notification { title: None, body } if body.is_empty()),
        ) {
            self.events.push(event);
        }
    }

    /// OSC 52 ; destino ; datos en base64 (o `?` para leer).
    fn clipboard(&mut self, params: &[&[u8]]) {
        let Some(data) = params.get(2).and_then(|d| std::str::from_utf8(d).ok()) else {
            return;
        };
        if data == "?" {
            let selector = params
                .get(1)
                .map(|s| String::from_utf8_lossy(s).into_owned())
                .unwrap_or_default();
            self.events.push(TermEvent::ClipboardRequest { selector });
            return;
        }
        if let Some(text) = links::base64_decode(data).and_then(|b| String::from_utf8(b).ok()) {
            self.events.push(TermEvent::Clipboard(text));
        }
    }

    fn reset(&mut self) {
        let title = self.title.take();
        *self = Screen::new(self.cols(), self.rows(), self.history_limit);
        self.title = title;
    }

    fn sgr(&mut self, params: &Params) {
        let params: Vec<&[u16]> = params.iter().collect();
        // El enlace OSC 8 no forma parte de SGR: sobrevive a un reset.
        let reset = Pen {
            link: self.pen.link,
            ..Pen::default()
        };
        if params.is_empty() {
            self.pen = reset;
            return;
        }
        let mut i = 0;
        while i < params.len() {
            let p = params[i];
            match p[0] {
                0 => self.pen = reset,
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
        // ASCII imprimible (lo más común): ancho 1 sin consultar la tabla.
        let width = match if (' '..='~').contains(&c) {
            Some(1)
        } else {
            c.width()
        } {
            Some(w @ 1..=2) => w,
            // Ancho 0 (combinantes, ZWJ, selectores de variante): se une a la
            // celda anterior sin mover el cursor, como en xterm.
            Some(0) => return self.combine(c),
            _ => return,
        };
        let cols = self.cols();
        if width > cols {
            return;
        }
        if self.cursor.wrap_pending {
            self.grid.set_wrapped(self.cursor.row, true);
            self.cursor.col = 0;
            self.linefeed();
        }
        // Un carácter ancho que no cabe en la última columna salta de línea antes.
        if width == 2 && self.cursor.col + 1 == cols {
            if self.modes.autowrap {
                // El hueco se marca para que el reflow no lo tome por contenido.
                let spacer = Cell {
                    flags: Flags::WIDE_SPACER,
                    ..self.blank()
                };
                self.write_cell(self.cursor.row, self.cursor.col, spacer);
                self.grid.set_wrapped(self.cursor.row, true);
                self.cursor.col = 0;
                self.linefeed();
            } else {
                self.cursor.col = cols - 2;
            }
        }

        let Pen {
            fg,
            bg,
            flags,
            link,
        } = self.pen;
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
                link,
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
                    link,
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
            Some(&b"133") => self.shell_mark(params),
            Some(&b"8") => self.hyperlink(params),
            Some(&b"52") => self.clipboard(params),
            Some(&(b"9" | b"777" | b"99")) => self.notification(params),
            Some(&b"7") => {
                if let Some(path) = params
                    .get(1)
                    .and_then(|p| std::str::from_utf8(p).ok())
                    .and_then(block::path_from_file_url)
                {
                    self.cwd = Some(path);
                }
            }
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
            ([b'?', b'$'], 'p') => return self.report_private_mode(raw(params, 0)),
            // XTVERSION.
            ([b'>'], 'q') => {
                let _ = write!(
                    Bytes(&mut self.responses),
                    "\x1bP>|{} {}\x1b\\",
                    crate::PRODUCT_NAME,
                    env!("CARGO_PKG_VERSION")
                );
                return;
            }
            ([k @ (b'>' | b'<' | b'=' | b'?')], 'u') => return self.kitty_keyboard(*k, params),
            // modifyOtherKeys: `CSI > 4 ; n m`.
            ([b'>'], 'm') => {
                if raw(params, 0) == 4 {
                    self.modes.modify_other_keys = raw(params, 1).min(2) as u8;
                }
                return;
            }
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

    fn history_text(t: &Term) -> Vec<String> {
        let g = t.grid();
        (g.first_line()..g.screen_start())
            .map(|l| {
                g.line(l)
                    .unwrap()
                    .cells
                    .iter()
                    .filter(|c| !c.flags.contains(Flags::WIDE_SPACER))
                    .map(|c| c.ch)
                    .collect::<String>()
                    .trim_end()
                    .to_owned()
            })
            .collect()
    }

    /// Texto de historial + pantalla, para comparar contenidos tras un reflow.
    fn all_text(t: &Term) -> String {
        let mut lines = history_text(t);
        lines.push(t.snapshot_text());
        lines.join("\n").trim_end().to_owned()
    }

    #[test]
    fn scrolled_lines_go_to_history() {
        let t = term(10, 2, "uno\r\ndos\r\ntres\r\ncuatro");
        assert_eq!(history_text(&t), ["uno", "dos"]);
        assert_eq!(t.snapshot_text(), "tres\ncuatro");
    }

    #[test]
    fn alt_screen_does_not_feed_history() {
        let t = term(10, 2, "\x1b[?1049ha\r\nb\r\nc\r\nd");
        assert!(history_text(&t).is_empty());
    }

    #[test]
    fn ed3_clears_history() {
        let mut t = term(10, 1, "a\r\nb\r\nc");
        assert_eq!(t.grid().history_len(), 2);
        t.advance(b"\x1b[3J");
        assert_eq!(t.grid().history_len(), 0);
    }

    #[test]
    fn autowrap_marks_rows_as_wrapped() {
        let t = term(3, 3, "abcdef\r\ng");
        assert!(t.grid().is_wrapped(0));
        assert!(!t.grid().is_wrapped(1));
    }

    #[test]
    fn display_offset_scrolls_view_and_stays_stable() {
        let mut t = term(5, 2, "1\r\n2\r\n3\r\n4");
        t.scroll_display(1);
        assert_eq!(t.display_offset(), 1);
        assert_eq!(t.visible_row(0)[0].ch, '2');
        // Llega salida: la vista sigue mostrando lo mismo.
        t.advance(b"\r\n5");
        assert_eq!(t.display_offset(), 2);
        assert_eq!(t.visible_row(0)[0].ch, '2');
        t.scroll_to_bottom();
        assert_eq!(t.display_offset(), 0);
        assert_eq!(t.visible_row(0)[0].ch, '4');
        // No se puede subir más allá del historial.
        t.scroll_display(100);
        assert_eq!(t.display_offset(), t.grid().history_len());
    }

    #[test]
    fn scrolled_view_reports_full_damage() {
        let mut t = term(5, 2, "1\r\n2\r\n3");
        dirty_rows(&mut t);
        t.scroll_display(1);
        assert_eq!(dirty_rows(&mut t), [0, 1]);
        assert!(dirty_rows(&mut t).is_empty());
    }

    #[test]
    fn reflow_narrower_wraps_long_lines() {
        let mut t = term(10, 4, "abcdefgh\r\nxy");
        t.resize(4, 4);
        assert_eq!(t.snapshot_text(), "abcd\nefgh\nxy");
        assert!(t.grid().is_wrapped(0));
        assert_eq!(pos(&t), (2, 2));
    }

    #[test]
    fn reflow_wider_joins_wrapped_lines() {
        let mut t = term(4, 4, "abcdefgh\r\nxy");
        assert_eq!(t.snapshot_text(), "abcd\nefgh\nxy");
        t.resize(10, 4);
        assert_eq!(t.snapshot_text(), "abcdefgh\nxy");
        assert_eq!(pos(&t), (1, 2));
    }

    #[test]
    fn reflow_round_trip_preserves_content() {
        let input = "primera línea bastante larga\r\n日本語の文章とテキスト\r\ncorta\r\n\x1b[31mcolor\x1b[0m fin";
        let mut t = term(20, 6, input);
        let before = all_text(&t);
        let cursor = pos(&t);
        for cols in [7, 5, 3, 11, 20] {
            t.resize(cols, 6);
        }
        assert_eq!(all_text(&t), before);
        assert_eq!(pos(&t), cursor);
        assert_eq!(t.grid().cell(t.cursor().row, 0).fg, Color::Indexed(1));
    }

    #[test]
    fn reflow_keeps_wide_char_whole_at_the_edge() {
        // "ab日" en 3 columnas: el ancho no cabe en la última y salta.
        let mut t = term(4, 3, "ab日c");
        t.resize(3, 3);
        assert_eq!(t.snapshot_text(), "ab\n日c");
        t.resize(4, 3);
        assert_eq!(t.snapshot_text(), "ab日\nc");
    }

    #[test]
    fn reflow_moves_lines_between_history_and_screen() {
        let mut t = term(10, 3, "1111\r\n2222\r\n3333\r\n4444");
        assert_eq!(history_text(&t), ["1111"]);
        // Más estrecho: las líneas ocupan el doble y suben al historial.
        t.resize(2, 3);
        assert_eq!(t.snapshot_text(), "33\n44\n44");
        assert!(t.cursor().wrap_pending);
        // Más ancho otra vez: vuelven del historial.
        t.resize(10, 3);
        assert_eq!(t.snapshot_text(), "2222\n3333\n4444");
        assert_eq!(history_text(&t), ["1111"]);
    }

    #[test]
    fn taller_window_pulls_history_down() {
        let mut t = term(10, 2, "a\r\nb\r\nc");
        t.resize(10, 3);
        assert_eq!(t.snapshot_text(), "a\nb\nc");
        assert_eq!(pos(&t), (2, 1));
    }

    #[test]
    fn reflow_while_in_alt_screen_keeps_main_screen() {
        let mut t = term(8, 3, "abcdefgh\r\n$ \x1b[?1049h\x1b[Hvim");
        t.resize(4, 3);
        assert_eq!(t.snapshot_text(), "vim");
        t.advance(b"\x1b[?1049l");
        assert!(all_text(&t).contains("abcd\nefgh"));
    }

    const A: &str = "\x1b]133;A\x07";
    const B: &str = "\x1b]133;B\x07";

    fn c(cmd: &str) -> String {
        format!("\x1b]133;C;cmdline_url={}\x07", cmd.replace(' ', "%20"))
    }

    fn d(code: i32) -> String {
        format!("\x1b]133;D;{code}\x07")
    }

    #[test]
    fn blocks_from_shell_marks() {
        let input = format!(
            "{A}$ {B}echo hola\r\n{}hola\r\n{}{A}$ {B}false\r\n{}{}{A}$ {B}",
            c("echo hola"),
            d(0),
            c("false"),
            d(1)
        );
        let t = term(20, 10, &input);
        let blocks = t.blocks();
        assert_eq!(blocks.len(), 3);
        assert_eq!(blocks[0].command.as_deref(), Some("echo hola"));
        assert_eq!(blocks[0].exit_code, Some(0));
        assert_eq!(t.block_output(&blocks[0]).as_deref(), Some("hola"));
        assert_eq!(blocks[1].exit_code, Some(1));
        assert_eq!(t.block_output(&blocks[1]).as_deref(), Some(""));
        assert!(!blocks[2].is_finished());
        assert_eq!(t.last_finished_block().unwrap().id, blocks[1].id);
    }

    #[test]
    fn output_without_trailing_newline() {
        let input = format!("{A}$ {B}printf foo\r\n{}foo{}{A}$ ", c("printf foo"), d(0));
        let t = term(20, 5, &input);
        assert_eq!(t.block_output(&t.blocks()[0]).as_deref(), Some("foo"));
    }

    #[test]
    fn empty_enter_does_not_create_a_block() {
        // Enter sin comando: no hay C ni D, solo otro A.
        let input = format!("{A}$ {B}\r\n{A}$ {B}\r\n{A}$ {B}");
        let t = term(20, 5, &input);
        assert_eq!(t.blocks().len(), 1);
        assert!(t.blocks()[0].exit_code.is_none());
    }

    #[test]
    fn marks_are_ignored_in_alt_screen() {
        let input = format!("\x1b[?1049h{A}x{}", d(0));
        let t = term(20, 5, &input);
        assert!(t.blocks().is_empty());
    }

    #[test]
    fn cwd_from_osc7_is_attached_to_blocks() {
        let input = format!("\x1b]7;file://host/tmp/mi%20dir\x07{A}$ ");
        let t = term(20, 5, &input);
        assert_eq!(t.cwd(), Some("/tmp/mi dir"));
        assert_eq!(t.blocks()[0].cwd.as_deref(), Some("/tmp/mi dir"));
    }

    #[test]
    fn block_points_follow_reflow() {
        let input = format!("{A}$ {B}cmd\r\n{}abcdefghij\r\n{}{A}$ {B}", c("cmd"), d(0));
        let mut t = term(12, 6, &input);
        t.resize(5, 6);
        assert_eq!(
            t.block_output(&t.blocks()[0]).as_deref(),
            Some("abcdefghij")
        );
        t.resize(30, 6);
        assert_eq!(
            t.block_output(&t.blocks()[0]).as_deref(),
            Some("abcdefghij")
        );
    }

    #[test]
    fn resize_at_prompt_clears_prompt_for_redraw() {
        let input = format!("salida\r\n{A}$ {B}texto");
        let mut t = term(20, 4, &input);
        t.resize(10, 4);
        // La línea del prompt queda vacía y el cursor al principio: la shell la redibuja.
        assert_eq!(t.snapshot_text(), "salida");
        assert_eq!(pos(&t), (1, 0));
    }

    #[test]
    fn evicted_blocks_are_pruned() {
        let mut input = format!("{A}$ {B}x\r\n{}", c("x"));
        input.push_str(&"l\r\n".repeat(HISTORY_LIMIT + 50));
        input.push_str(&format!("{}{A}$ ", d(0)));
        let t = term(10, 5, &input);
        // El primer bloque sigue (su final está en el historial), pero su
        // salida está recortada; el prompt es inalcanzable.
        assert!(t.blocks()[0].prompt.line < t.grid().first_line());
        let mut t = t;
        t.advance("m\r\n".repeat(HISTORY_LIMIT + 50).as_bytes());
        assert_eq!(t.blocks().len(), 1);
        assert!(!t.blocks()[0].is_finished());
    }

    #[test]
    fn scroll_to_line_and_visible_line() {
        let mut t = term(10, 2, "1\r\n2\r\n3\r\n4\r\n5");
        t.scroll_to_line(1);
        assert_eq!(t.visible_line(0), 1);
        assert_eq!(t.visible_row(0)[0].ch, '2');
    }

    #[test]
    fn selection_is_cleared_on_resize() {
        let mut t = term(10, 2, "hola");
        t.set_selection(Some(Selection::new(
            crate::SelectionKind::Simple,
            selection::point(0, 0),
        )));
        assert_eq!(t.selection_text().as_deref(), Some("h"));
        t.resize(5, 2);
        assert!(t.selection().is_none());
    }

    #[test]
    fn mouse_focus_and_scroll_modes() {
        let mut t = term(5, 1, "\x1b[?1002;1006;1004h");
        let m = t.modes();
        assert_eq!(m.mouse, MouseMode::Drag);
        assert!(m.mouse_sgr && m.focus_events && m.alt_scroll);
        t.advance(b"\x1b[?1003h");
        assert_eq!(t.modes().mouse, MouseMode::Motion);
        t.advance(b"\x1b[?1003l\x1b[?1007l");
        assert_eq!(t.modes().mouse, MouseMode::Off);
        assert!(!t.modes().alt_scroll);
    }

    #[test]
    fn decrqm_reports_modes() {
        let mut t = term(
            5,
            1,
            "\x1b[?2026h\x1b[?2026$p\x1b[?25$p\x1b[?1000$p\x1b[?9999$p",
        );
        assert_eq!(
            t.take_responses(),
            b"\x1b[?2026;1$y\x1b[?25;1$y\x1b[?1000;2$y\x1b[?9999;0$y".to_vec()
        );
    }

    #[test]
    fn xtversion() {
        let mut t = term(5, 1, "\x1b[>q");
        let r = String::from_utf8(t.take_responses()).unwrap();
        assert!(
            r.starts_with("\x1bP>|rust-terminal ") && r.ends_with("\x1b\\"),
            "{r:?}"
        );
    }

    #[test]
    fn synchronized_output_with_timeout() {
        let mut t = term(5, 1, "\x1b[?2026h");
        assert!(t.synchronized());
        t.advance(b"\x1b[?2026l");
        assert!(!t.synchronized());
        t.advance(b"\x1b[?2026h");
        std::thread::sleep(SYNC_TIMEOUT + Duration::from_millis(20));
        assert!(!t.synchronized());
    }

    #[test]
    fn kitty_keyboard_stack() {
        let mut t = term(5, 1, "\x1b[?u");
        assert_eq!(t.take_responses(), b"\x1b[?0u");
        t.advance(b"\x1b[>1u\x1b[>27u\x1b[?u");
        assert_eq!(t.modes().kitty_flags, 27);
        assert_eq!(t.take_responses(), b"\x1b[?27u");
        t.advance(b"\x1b[=8;3u");
        assert_eq!(t.modes().kitty_flags, 27 & !8);
        t.advance(b"\x1b[=8;2u");
        assert_eq!(t.modes().kitty_flags, 27);
        t.advance(b"\x1b[<u");
        assert_eq!(t.modes().kitty_flags, 1);
        t.advance(b"\x1b[<5u");
        assert_eq!(t.modes().kitty_flags, 0);
    }

    #[test]
    fn kitty_stack_is_per_screen() {
        let mut t = term(5, 1, "\x1b[>1u\x1b[?1049h");
        assert_eq!(t.modes().kitty_flags, 0);
        t.advance(b"\x1b[>8u\x1b[?1049l");
        assert_eq!(t.modes().kitty_flags, 1);
    }

    #[test]
    fn modify_other_keys() {
        let mut t = term(5, 1, "\x1b[>4;2m");
        assert_eq!(t.modes().modify_other_keys, 2);
        t.advance(b"\x1b[>4m");
        assert_eq!(t.modes().modify_other_keys, 0);
    }

    #[test]
    fn osc8_hyperlinks_survive_sgr_reset() {
        let t = term(
            30,
            1,
            "a\x1b]8;;https://ej.es\x07li\x1b[0mnk\x1b]8;;\x07 b\x1b]8;id=x;https://ej.es\x07c",
        );
        let row = t.grid().row(0);
        assert_eq!(row[0].link, 0);
        assert!(row[1].link != 0 && row[1].link == row[4].link);
        assert_eq!(row[5].link, 0);
        // La misma URL reutiliza el id.
        assert_eq!(row[7].link, row[1].link);
        assert_eq!(t.link(row[1].link), Some("https://ej.es"));
        let (url, range) = t.link_at(selection::point(0, 3)).unwrap();
        assert_eq!(url, "https://ej.es");
        assert_eq!((range.start.col, range.end.col), (1, 4));
    }

    #[test]
    fn plain_urls_are_found_across_wrapped_rows() {
        let t = term(10, 3, "ver https://ej.es/abc fin");
        let (url, range) = t.link_at(selection::point(1, 2)).unwrap();
        assert_eq!(url, "https://ej.es/abc");
        assert_eq!((range.start.line, range.start.col), (0, 4));
        assert!(t.link_at(selection::point(0, 0)).is_none());
    }

    #[test]
    fn clipboard_write_and_read_request() {
        let mut t = term(5, 1, "\x1b]52;c;aG9sYSDDsQ==\x07\x1b]52;c;?\x07");
        assert_eq!(
            t.take_events(),
            [
                TermEvent::Clipboard("hola ñ".to_owned()),
                TermEvent::ClipboardRequest {
                    selector: "c".to_owned()
                },
            ]
        );
        // El núcleo no responde por su cuenta: decide la app.
        assert!(t.take_responses().is_empty());
        assert_eq!(
            Term::clipboard_response("c", "hola"),
            b"\x1b]52;c;aG9sYQ==\x1b\\".to_vec()
        );
    }

    #[test]
    fn notifications() {
        let mut t = term(
            5,
            1,
            "\x1b]9;Hecho\x07\x1b]9;4;1;50\x07\x1b]777;notify;Claude;Listo\x07\x1b]99;;Kitty\x07",
        );
        assert_eq!(
            t.take_events(),
            [
                TermEvent::Notification {
                    title: None,
                    body: "Hecho".into()
                },
                TermEvent::Notification {
                    title: Some("Claude".into()),
                    body: "Listo".into()
                },
                TermEvent::Notification {
                    title: None,
                    body: "Kitty".into()
                },
            ]
        );
    }

    #[test]
    fn blocks_record_duration() {
        let input = format!("{A}$ {B}x\r\n{}{}{A}$ ", c("x"), d(0));
        let t = term(20, 5, &input);
        assert!(t.blocks()[0].duration().is_some());
    }

    #[test]
    fn custom_history_limit() {
        let mut t = Term::with_history_limit(5, 1, 3);
        t.advance("1\r\n2\r\n3\r\n4\r\n5\r\n6".as_bytes());
        assert_eq!(t.grid().history_len(), 3);
        // Un reset conserva el límite.
        t.advance(b"\x1bc1\r\n2\r\n3\r\n4\r\n5");
        assert_eq!(t.grid().history_len(), 3);
        let mut none = Term::with_history_limit(5, 1, 0);
        none.advance(b"1\r\n2");
        assert_eq!(none.grid().history_len(), 0);
    }

    #[test]
    fn unhandled_sequences_are_recorded() {
        let _t = term(5, 1, "\x1b[?31337h\x1b[?31337h");
        let recent = recent_unhandled();
        assert!(
            recent.iter().any(|(s, n)| s.contains("31337") && *n >= 2),
            "{recent:?}"
        );
    }

    #[test]
    fn combining_marks_join_the_previous_cell() {
        let t = term(10, 1, "e\u{301}x");
        assert_eq!(t.snapshot_text(), "e\u{301}x");
        // El cursor no se mueve con la marca: la "x" va en la columna 1.
        assert_eq!(t.grid().cell(0, 1).ch, 'x');
        assert_eq!(t.grid().char_text(t.grid().cell(0, 0).ch), "e\u{301}");
        assert_eq!(t.grid().base_char(t.grid().cell(0, 0).ch), 'e');
    }

    #[test]
    fn emoji_sequences_keep_their_codepoints() {
        // Corazón con selector de variante y familia con ZWJ: el texto se
        // conserva entero y las columnas siguen las de wcwidth.
        let input = "❤\u{fe0f}👨\u{200d}👩";
        let t = term(10, 1, input);
        assert_eq!(t.snapshot_text(), input);
        assert_eq!(pos(&t), (0, 5));
    }

    #[test]
    fn combining_mark_after_wrap_and_on_wide_char() {
        // Tras escribir en la última columna (salto pendiente).
        let t = term(3, 2, "abc\u{301}");
        assert_eq!(t.snapshot_text(), "abc\u{301}");
        // Sobre un carácter ancho, la marca va a su primera mitad.
        let t = term(5, 1, "日\u{3099}x");
        assert_eq!(t.grid().char_text(t.grid().cell(0, 0).ch), "日\u{3099}");
        assert_eq!(t.grid().cell(0, 2).ch, 'x');
    }

    #[test]
    fn combined_cells_survive_selection_reflow_and_history() {
        let mut t = term(6, 2, "ae\u{301}iou\r\nxx\r\nyy");
        let sel = SelectionRange {
            start: selection::point(0, 0),
            end: selection::point(0, 4),
            block: false,
        };
        assert_eq!(selection::text_in_range(t.grid(), sel), "ae\u{301}iou");
        t.resize(3, 2);
        t.resize(6, 2);
        let first = t.grid().first_line();
        let row = &t.grid().line(first).unwrap().cells;
        assert_eq!(t.grid().char_text(row[1].ch), "e\u{301}");
    }

    #[test]
    fn row_used_invariant_holds_under_mixed_operations() {
        let mut t = Term::new(12, 5);
        let steps: &[&str] = &[
            "hola mundo largo que salta\r\n",
            "\x1b[44m\x1b[2K azul\x1b[0m\r\n",
            "\x1b[2;3H\x1b[3@xyz\x1b[2P",
            "\x1b[2;4r\x1b[4;1H\n\n\n\x1b[r",
            "\x1b[1;1H\x1b[2L\x1b[1M",
            "\x1b[45m\x1b[J\x1b[0m",
            "日本語テキスト🙂 e\u{301}\r\n",
            "\x1b[3;5H\x1b[K\x1b[1K\x1b[5X",
            "\x1b[S\x1b[2T",
        ];
        for (i, step) in steps.iter().enumerate() {
            t.advance(step.as_bytes());
            t.grid().check_used();
            if i % 3 == 2 {
                t.resize(7 + i, 4 + i % 3);
                t.grid().check_used();
            }
        }
        t.advance(b"\x1b[?1049hfoo\x1b[41m\x1b[2Jbar\x1b[?1049l");
        t.grid().check_used();
    }

    #[test]
    fn recent_text_reads_history_and_the_screen() {
        let mut t = Term::with_history_limit(10, 3, 100);
        for i in 1..=8 {
            t.advance(format!("linea{i}\r\n").as_bytes());
        }
        // Pantalla de 3 filas: lo último queda en pantalla, lo anterior en el historial.
        let all = t.recent_text(100);
        assert!(all.starts_with("linea1\n"), "{all}");
        assert!(all.ends_with("linea8"), "{all}");
        // Solo las últimas.
        assert_eq!(t.recent_text(2), "linea7\nlinea8");
        assert_eq!(t.recent_text(1), "linea8");
        // Más de lo que hay no falla, y sin nada devuelve vacío.
        assert_eq!(Term::new(10, 3).recent_text(5), "");
    }
}
