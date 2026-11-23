//! Núcleo del emulador: PTY, parser y estado de la pantalla.
//!
//! No depende de nada de interfaz ni de macOS más allá de POSIX; el
//! renderizado vive en otro crate.

pub mod block;
pub mod grid;
pub mod pty;
pub mod selection;
pub mod term;

pub use block::Block;
pub use grid::{Cell, Color, Flags, Grid, Line, Point, Row};
pub use pty::{Pty, PtyReader, PtyWriter};
pub use selection::{Selection, SelectionKind, SelectionRange};
pub use term::{Cursor, Modes, Term};
