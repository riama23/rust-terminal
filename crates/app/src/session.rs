//! Una sesión por panel: la shell en su PTY, su `Term` y el hilo lector.
//!
//! El lector parsea lotes de la PTY con el bloqueo de `Term` tomado y lo
//! suelta en cada lote. Después pide un frame, pero solo si el panel está a la
//! vista: la salida de una pestaña oculta se procesa sin pintar.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::sync::{Arc, Mutex};

use objc2::sel;
use rust_terminal_core::{Pty, PtyReader, PtyWriter, Term};

use crate::window::Waker;
use crate::workspace::PaneId;

pub struct Session {
    pub id: PaneId,
    pub term: Mutex<Term>,
    writer: Mutex<PtyWriter>,
    pid: u32,
    shell_name: String,
    /// El panel está en la pestaña activa.
    pub visible: AtomicBool,
    /// La shell terminó (el lector llegó al final).
    pub exited: AtomicBool,
}

impl Session {
    /// Lanza `shell` como shell de login en una PTY nueva y arranca su lector.
    pub fn spawn(
        id: PaneId,
        shell: &str,
        env: &[(String, String)],
        cwd: Option<&Path>,
        cols: u16,
        rows: u16,
        waker: Arc<Waker>,
    ) -> std::io::Result<Arc<Session>> {
        // Todos los `spawn` ocurren en el hilo principal: así ningún otro
        // `fork` puede heredar un maestro antes de que tenga `CLOEXEC`.
        let pty = Pty::spawn_in(shell, &["-l"], env.iter().cloned(), cwd, cols, rows)?;
        let (reader, writer, child) = pty.split()?;
        let session = Arc::new(Session {
            id,
            term: Mutex::new(Term::new(cols.into(), rows.into())),
            writer: Mutex::new(writer),
            pid: child.id(),
            shell_name: shell.rsplit('/').next().unwrap_or(shell).to_owned(),
            visible: AtomicBool::new(true),
            exited: AtomicBool::new(false),
        });
        start_reader(session.clone(), reader, child, waker);
        Ok(session)
    }

    /// Escribe en la PTY (teclas, pegado o respuestas a consultas).
    pub fn write(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
        }
    }

    /// Cambia el tamaño del núcleo y de la PTY si es distinto.
    pub fn resize(&self, cols: usize, rows: usize) {
        let mut term = self.term.lock().unwrap();
        if term.grid().cols() != cols || term.grid().rows() != rows {
            term.resize(cols, rows);
            let _ = self.writer.lock().unwrap().resize(cols as u16, rows as u16);
        }
    }

    /// Título para la pestaña: OSC 0/2, si no el directorio (OSC 7) y si no
    /// el nombre de la shell.
    pub fn title(&self) -> String {
        let term = self.term.lock().unwrap();
        if let Some(t) = term.title().filter(|t| !t.is_empty()) {
            return t.to_owned();
        }
        if let Some(cwd) = term.cwd() {
            let home = std::env::var("HOME").unwrap_or_default();
            if !home.is_empty() && cwd == home {
                return "~".to_owned();
            }
            if let Some(name) = Path::new(cwd).file_name() {
                return name.to_string_lossy().into_owned();
            }
        }
        self.shell_name.clone()
    }

    /// Directorio actual según OSC 7.
    pub fn cwd(&self) -> Option<PathBuf> {
        self.term.lock().unwrap().cwd().map(PathBuf::from)
    }

    /// Hay un programa en marcha: el grupo en primer plano no es la shell, o
    /// los bloques dicen que un comando no ha terminado.
    pub fn is_busy(&self) -> bool {
        if self.exited.load(SeqCst) {
            return false;
        }
        let fg = self.writer.lock().unwrap().foreground_pgrp();
        if fg.is_some_and(|pgrp| pgrp != self.pid) {
            return true;
        }
        let term = self.term.lock().unwrap();
        term.blocks()
            .last()
            .is_some_and(|b| b.output.is_some() && !b.is_finished())
    }

    /// Cierra la sesión: la shell recibe SIGHUP y el lector termina solo.
    pub fn hangup(&self) {
        rust_terminal_core::hangup(self.pid);
    }
}

fn start_reader(session: Arc<Session>, mut reader: PtyReader, mut child: Child, waker: Arc<Waker>) {
    std::thread::Builder::new()
        .name(format!("pty-reader-{}", session.id))
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let (responses, title_changed) = {
                    let mut term = session.term.lock().unwrap();
                    term.advance(&buf[..n]);
                    (term.take_responses(), term.take_title_changed())
                };
                if !responses.is_empty() {
                    session.write(&responses);
                }
                if title_changed {
                    waker.on_main_thread(sel!(sessionsChanged));
                }
                if session.visible.load(SeqCst) {
                    waker.request_frame();
                }
            }
            let _ = child.wait();
            session.exited.store(true, SeqCst);
            waker.on_main_thread(sel!(sessionsChanged));
        })
        .expect("hilo lector");
}
