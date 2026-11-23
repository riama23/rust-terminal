//! Núcleo del emulador: PTY, parser y estado de la pantalla.
//!
//! No depende de nada de interfaz ni de macOS más allá de POSIX; el
//! renderizado vive en otro crate.

pub mod grid;
pub mod pty;
pub mod term;

pub use grid::{Cell, Color, Flags, Grid};
pub use pty::{Pty, PtyReader, PtyWriter};
pub use term::{Cursor, Modes, Term};
