//! Prueba de humo del núcleo (M1): sin ventana ni hilos. Ejecuta la shell
//! del usuario en una PTY, le manda un comando y muestra la pantalla final.
//! Se sustituye por la app de AppKit en M2.

use std::io::{self, Read, Write};

use rust_terminal_core::{Pty, Term};

const COLS: u16 = 80;
const ROWS: u16 = 24;

fn main() -> io::Result<()> {
    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
    let mut pty = Pty::spawn(&shell, &[], std::iter::empty::<(&str, &str)>(), COLS, ROWS)?;
    pty.write_all(b"echo hola; exit\n")?;

    let mut term = Term::new(COLS.into(), ROWS.into());
    let mut buf = [0u8; 64 * 1024];
    loop {
        let n = pty.read(&mut buf)?;
        if n == 0 {
            break;
        }
        term.advance(&buf[..n]);
    }
    let status = pty.child().wait()?;

    println!("{}", term.snapshot_text());
    eprintln!("[{shell} terminó: {status}]");
    Ok(())
}
