//! Núcleo del emulador: PTY, parser y estado de la pantalla.
//!
//! No depende de nada de interfaz ni de macOS más allá de POSIX; el
//! renderizado vive en otro crate.

/// Nombre del producto (provisional hasta que se decida). Fija la ruta de
/// configuración, el nombre del `.app` y la respuesta a XTVERSION.
pub const PRODUCT_NAME: &str = "rust-terminal";

/// Celdas que ocupa un carácter (0, 1 o 2), con la misma tabla que usa el
/// núcleo al imprimir.
pub fn char_width(c: char) -> usize {
    unicode_width::UnicodeWidthChar::width(c).unwrap_or(0)
}

pub mod block;
pub mod grid;
pub mod links;
pub mod pty;
pub mod selection;
pub mod term;

pub use block::Block;
pub use grid::{Cell, Color, Flags, Grid, Line, Point, Row};
pub use pty::{Pty, PtyReader, PtyWriter, hangup};
pub use selection::{Selection, SelectionKind, SelectionRange};
pub use term::{Cursor, Modes, MouseMode, Term, TermEvent, recent_unhandled};
