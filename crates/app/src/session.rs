//! Una sesión por panel: la shell en su PTY, su `Term` y el hilo lector.
//!
//! El lector parsea lotes de la PTY con el bloqueo de `Term` tomado y lo
//! suelta en cada lote. Después pide un frame, pero solo si el panel está a la
//! vista: la salida de una pestaña oculta se procesa sin pintar.

use std::io::{Read, Write};
use std::path::{Path, PathBuf};
use std::process::Child;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use objc2::sel;
use rust_terminal_core::{Pty, PtyReader, PtyWriter, Term, TermEvent};

use crate::window::Waker;
use crate::workspace::PaneId;

/// Algo que la sesión le pide a la app (en el hilo principal).
#[derive(Debug, Clone, PartialEq)]
pub enum AppEvent {
    Term(TermEvent),
    Bell,
    /// Terminó un comando largo (bloques OSC 133).
    CommandFinished {
        command: String,
        exit_code: Option<i32>,
        seconds: u64,
    },
    /// Claude Code pasó a ser (o dejó de ser) el programa en primer plano.
    Claude(bool),
    /// Un programa distinto de la shell ocupa (o dejó de ocupar) el primer
    /// plano: la pestaña muestra su spinner.
    Busy(bool),
}

pub struct Session {
    pub id: PaneId,
    pub term: Mutex<Term>,
    /// Cola hacia el hilo escritor: escribir en la PTY puede bloquearse (un
    /// pegado grande con la aplicación ocupada) y no debe parar ni la interfaz
    /// ni el lector.
    input: mpsc::Sender<Vec<u8>>,
    /// Manejador para el tamaño y las consultas, que no se bloquean.
    control: Mutex<PtyWriter>,
    /// El panel se cerró: el hilo escritor abandona lo pendiente.
    closed: Arc<AtomicBool>,
    pid: u32,
    shell_name: String,
    /// El panel está en la pestaña activa.
    pub visible: AtomicBool,
    /// La shell terminó (el lector llegó al final).
    pub exited: AtomicBool,
    /// Hay un programa en primer plano que no es la shell (lo pone el lector).
    pub busy: AtomicBool,
    /// Eventos pendientes para la app.
    pending: Mutex<Vec<AppEvent>>,
    /// Último bloque terminado que ya se revisó (para avisar una sola vez).
    last_finished: AtomicU64,
    /// Segundos a partir de los que se avisa de un comando largo (0 = nunca).
    pub long_command_secs: AtomicU64,
}

/// Qué lanzar en una sesión nueva (de la configuración).
pub struct SpawnOptions<'a> {
    pub shell: &'a str,
    pub args: &'a [String],
    pub env: &'a [(String, String)],
    pub cwd: Option<&'a Path>,
    pub history_limit: usize,
    pub long_command_secs: u64,
}

impl Session {
    /// Lanza `shell` como shell de login en una PTY nueva y arranca su lector.
    pub fn spawn(
        id: PaneId,
        opts: SpawnOptions,
        cols: u16,
        rows: u16,
        waker: Arc<Waker>,
    ) -> std::io::Result<Arc<Session>> {
        // Todos los `spawn` ocurren en el hilo principal: así ningún otro
        // `fork` puede heredar un maestro antes de que tenga `CLOEXEC`.
        let shell = opts.shell;
        let args: Vec<&str> = opts.args.iter().map(String::as_str).collect();
        let pty = Pty::spawn_in(shell, &args, opts.env.iter().cloned(), opts.cwd, cols, rows)?;
        let (reader, writer, child) = pty.split()?;
        let control = writer.try_clone()?;
        let closed = Arc::new(AtomicBool::new(false));
        let input = start_writer(id, writer, closed.clone());
        let session = Arc::new(Session {
            id,
            term: Mutex::new(Term::with_history_limit(
                cols.into(),
                rows.into(),
                opts.history_limit,
            )),
            input,
            control: Mutex::new(control),
            closed,
            pid: child.id(),
            shell_name: shell.rsplit('/').next().unwrap_or(shell).to_owned(),
            visible: AtomicBool::new(true),
            exited: AtomicBool::new(false),
            busy: AtomicBool::new(false),
            pending: Mutex::new(Vec::new()),
            last_finished: AtomicU64::new(0),
            long_command_secs: AtomicU64::new(opts.long_command_secs),
        });
        start_reader(session.clone(), reader, child, waker);
        Ok(session)
    }

    /// Escribe en la PTY (teclas, pegado o respuestas a consultas).
    pub fn write(&self, bytes: &[u8]) {
        if !bytes.is_empty() {
            let _ = self.input.send(bytes.to_vec());
        }
    }

    /// Cambia el tamaño del núcleo y de la PTY si es distinto.
    pub fn resize(&self, cols: usize, rows: usize) {
        let mut term = self.term.lock().unwrap();
        if term.grid().cols() != cols || term.grid().rows() != rows {
            term.resize(cols, rows);
            let _ = self
                .control
                .lock()
                .unwrap()
                .resize(cols as u16, rows as u16);
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
        let fg = self.control.lock().unwrap().foreground_pgrp();
        if fg.is_some_and(|pgrp| pgrp != self.pid) {
            return true;
        }
        let term = self.term.lock().unwrap();
        term.blocks()
            .last()
            .is_some_and(|b| b.output.is_some() && !b.is_finished())
    }

    /// Eventos pendientes para la app.
    pub fn take_events(&self) -> Vec<AppEvent> {
        std::mem::take(&mut self.pending.lock().unwrap())
    }

    /// Recoge del núcleo lo que la app tiene que hacer (con el bloqueo tomado).
    fn collect_events(&self, term: &mut Term) -> Vec<AppEvent> {
        let mut events: Vec<AppEvent> =
            term.take_events().into_iter().map(AppEvent::Term).collect();
        if term.take_bell() {
            events.push(AppEvent::Bell);
        }
        if let Some(b) = term.last_finished_block()
            && b.id > self.last_finished.swap(b.id, SeqCst)
            && let threshold = self.long_command_secs.load(SeqCst)
            && threshold > 0
            && let Some(d) = b
                .duration()
                .filter(|d| *d >= Duration::from_secs(threshold))
        {
            events.push(AppEvent::CommandFinished {
                command: b.command.clone().unwrap_or_default(),
                exit_code: b.exit_code,
                seconds: d.as_secs(),
            });
        }
        events
    }

    /// Aviso de foco (`?1004`) si la aplicación lo pidió.
    pub fn send_focus(&self, gained: bool) {
        if self.term.lock().unwrap().modes().focus_events {
            self.write(crate::input::encode_focus(gained));
        }
    }

    /// Cierra la sesión: la shell recibe SIGHUP y el lector termina solo.
    pub fn hangup(&self) {
        self.closed.store(true, SeqCst);
        rust_terminal_core::hangup(self.pid);
    }
}

/// Hilo escritor: vacía la cola en la PTY en orden, en trozos pequeños y
/// esperando a que haya sitio. Termina cuando la sesión se cierra (`closed`),
/// se libera la cola o la PTY deja de aceptar datos; así nunca se queda
/// bloqueado para siempre en un pegado enorme a un panel que ya no existe.
fn start_writer(
    id: PaneId,
    mut writer: PtyWriter,
    closed: Arc<AtomicBool>,
) -> mpsc::Sender<Vec<u8>> {
    const CHUNK: usize = 1024;
    let (tx, rx) = mpsc::channel::<Vec<u8>>();
    std::thread::Builder::new()
        .name(format!("pty-writer-{id}"))
        .spawn(move || {
            for bytes in rx {
                let mut rest = &bytes[..];
                while !rest.is_empty() {
                    if closed.load(SeqCst) {
                        return;
                    }
                    if !writer.wait_writable(Duration::from_millis(100)) {
                        continue;
                    }
                    match writer.write(&rest[..rest.len().min(CHUNK)]) {
                        Ok(n) => rest = &rest[n..],
                        Err(e) if e.kind() == std::io::ErrorKind::Interrupted => {}
                        Err(_) => return,
                    }
                }
            }
        })
        .expect("hilo escritor");
    tx
}

fn start_reader(session: Arc<Session>, mut reader: PtyReader, mut child: Child, waker: Arc<Waker>) {
    std::thread::Builder::new()
        .name(format!("pty-reader-{}", session.id))
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            // Primer plano de la PTY: solo se mira el proceso si cambia el
            // grupo, y solo se avisa si cambia que sea Claude Code.
            let mut last_fg: Option<u32> = None;
            let mut claude_fg = false;
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let (responses, title_changed, mut events, block_running) = {
                    let mut term = session.term.lock().unwrap();
                    term.advance(&buf[..n]);
                    let events = session.collect_events(&mut term);
                    // Con las marcas OSC 133, un comando sin terminar: un
                    // proceso silencioso (`sleep`) no escribe nada que haga
                    // mirar el primer plano.
                    let running = term
                        .blocks()
                        .last()
                        .is_some_and(|b| b.output.is_some() && !b.is_finished());
                    (
                        term.take_responses(),
                        term.take_title_changed(),
                        events,
                        running,
                    )
                };
                let fg = session.control.lock().unwrap().foreground_pgrp();
                let fg_busy = fg.is_some_and(|p| p != session.pid);
                let busy = fg_busy || block_running;
                if busy != session.busy.swap(busy, SeqCst) {
                    events.push(AppEvent::Busy(busy));
                }
                if fg != last_fg {
                    last_fg = fg;
                    let claude = fg.is_some_and(|p| p != session.pid && crate::proc::is_claude(p));
                    if claude != claude_fg {
                        claude_fg = claude;
                        events.push(AppEvent::Claude(claude));
                    }
                }
                if !events.is_empty() {
                    session.pending.lock().unwrap().extend(events);
                    waker.on_main_thread(sel!(sessionEvents));
                }
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
