//! La API de control por agentes (v2, experimental): arranque del servidor,
//! reparto de peticiones al hilo principal y permisos en la interfaz.
//!
//! **Leer** (listar sesiones, pantalla, bloques) funciona con el token.
//! **Escribir** (teclear, abrir y cerrar sesiones) pide permiso con un diálogo
//! (una vez, o en esta sesión por agente y grupo; el botón por defecto es
//! *Denegar*) y deja un aviso «control: <agente>» en la barra de pestañas.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::path::PathBuf;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Arc, Mutex, OnceLock};
use std::time::{Duration, Instant};

use objc2::rc::Retained;
use objc2::sel;
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSApplication};
use objc2_foundation::{MainThreadMarker, NSString};
use serde_json::{Value, json};

use crate::control::protocol::{self, Group, MAX_LINES, Method, Request, code};
use crate::control::server::{Client, ControlServer, Handler};
use crate::view::TerminalView;
use crate::window::Waker;

/// Tope del texto que devuelve una lectura.
const MAX_RESULT_BYTES: usize = 2 * 1024 * 1024;
/// Tras negar un permiso, no se vuelve a preguntar al mismo agente durante este tiempo.
const DENY_COOLDOWN: Duration = Duration::from_secs(10);

/// Variables de entorno para las shells: socket, token y el comando `rt`.
static ENV: OnceLock<Vec<(String, String)>> = OnceLock::new();

pub fn env() -> Vec<(String, String)> {
    ENV.get().cloned().unwrap_or_default()
}

struct Job {
    client: Client,
    request: Request,
    reply: Sender<String>,
}

type Queue = Arc<Mutex<Vec<Job>>>;

/// El `Handler` del servidor: pasa cada petición al hilo principal y espera la respuesta.
struct Bridge {
    queue: Queue,
    waker: Arc<Waker>,
}

impl Handler for Bridge {
    fn handle(&self, client: &Client, request: Request) -> String {
        if request.method == Method::Hello {
            return protocol::ok(
                &request.id,
                json!({
                    "protocol": protocol::VERSION,
                    "experimental": true,
                    "app": rust_terminal_core::PRODUCT_NAME,
                    "pid": std::process::id(),
                }),
            );
        }
        let id = request.id.clone();
        // Escribir puede esperar a que el usuario conteste un diálogo.
        let wait = if request.method.write_group().is_some() {
            Duration::from_secs(130)
        } else {
            Duration::from_secs(15)
        };
        let (tx, rx) = channel();
        self.queue.lock().unwrap().push(Job {
            client: client.clone(),
            request,
            reply: tx,
        });
        self.waker.on_main_thread(sel!(controlRequests));
        rx.recv_timeout(wait)
            .unwrap_or_else(|_| protocol::err(&id, code::TIMEOUT, "la app no contestó a tiempo"))
    }
}

struct State {
    _server: ControlServer,
    queue: Queue,
    grants: HashSet<(String, Group)>,
    denied: HashMap<(String, Group), Instant>,
}

thread_local! {
    static STATE: RefCell<Option<State>> = const { RefCell::new(None) };
}

/// El directorio con el comando `rt`: junto al ejecutable (`cargo run`) o en
/// `Contents/Resources/bin` del `.app`.
fn rt_dir() -> Option<PathBuf> {
    let exe = std::env::current_exe().ok()?;
    let dir = exe.parent()?;
    [dir.to_owned(), dir.join("../Resources/bin")]
        .into_iter()
        .find(|d| d.join("rt").is_file())
        .and_then(|d| d.canonicalize().ok())
}

/// Borra las carpetas de sockets de ejecuciones que murieron sin limpiar
/// (`rust-terminal-control-<pid>-<id>` con un `pid` que ya no existe).
fn remove_stale_dirs(base: &std::path::Path) {
    let Ok(entries) = std::fs::read_dir(base) else {
        return;
    };
    for entry in entries.flatten() {
        let name = entry.file_name();
        let name = name.to_string_lossy();
        let Some(rest) = name.strip_prefix("rust-terminal-control-") else {
            continue;
        };
        let Some(pid) = rest.split('-').next().and_then(|p| p.parse::<u32>().ok()) else {
            continue;
        };
        if pid != std::process::id() && !crate::agent::launch::process_alive(pid) {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// Al salir: cierra el servidor y borra su carpeta (socket incluido).
pub fn shutdown() {
    STATE.with(|s| *s.borrow_mut() = None);
}

/// Arranca el servidor (si `control.enabled`) y prepara el entorno de las
/// shells. Una vez, antes de lanzar la primera.
pub fn start(waker: Arc<Waker>, enabled: bool) {
    if !enabled || ENV.get().is_some() {
        return;
    }
    remove_stale_dirs(&std::env::temp_dir());
    let queue: Queue = Arc::new(Mutex::new(Vec::new()));
    let handler = Arc::new(Bridge {
        queue: queue.clone(),
        waker,
    });
    let server = match ControlServer::start(&std::env::temp_dir(), handler) {
        Ok(s) => s,
        Err(e) => {
            eprintln!("API de control: no se pudo arrancar: {e}");
            return;
        }
    };
    let mut env = vec![
        (
            "RT_CONTROL_SOCKET".to_owned(),
            server.socket_path().display().to_string(),
        ),
        ("RT_CONTROL_TOKEN".to_owned(), server.token().to_owned()),
    ];
    if let Some(dir) = rt_dir() {
        env.push(("RT_CTL".to_owned(), dir.join("rt").display().to_string()));
        let path = std::env::var("PATH").unwrap_or_default();
        env.push(("PATH".to_owned(), format!("{}:{path}", dir.display())));
    }
    // Solo en depuración: `RT_CONTROL_DUMP=archivo` anota socket y token para las pruebas.
    #[cfg(debug_assertions)]
    if let Some(path) = std::env::var_os("RT_CONTROL_DUMP") {
        let text: String = env.iter().map(|(k, v)| format!("{k}={v}\n")).collect();
        let _ = std::fs::write(path, text);
    }
    let _ = ENV.set(env);
    STATE.with(|s| {
        *s.borrow_mut() = Some(State {
            _server: server,
            queue,
            grants: HashSet::new(),
            denied: HashMap::new(),
        });
    });
}

fn views(mtm: MainThreadMarker) -> Vec<Retained<TerminalView>> {
    let mut views: Vec<_> = NSApplication::sharedApplication(mtm)
        .windows()
        .iter()
        .filter_map(|w| crate::container::terminal_view(&w))
        .collect();
    views.sort_by_key(|v| v.view_id());
    views
}

/// Atiende lo que haya en la cola (hilo principal).
pub fn process(mtm: MainThreadMarker) {
    let jobs: Vec<Job> = STATE.with(|s| {
        s.borrow()
            .as_ref()
            .map(|st| std::mem::take(&mut *st.queue.lock().unwrap()))
            .unwrap_or_default()
    });
    for job in jobs {
        let response = execute(mtm, &job);
        let _ = job.reply.send(response);
    }
}

/// `"1-3"` → (vista 1, panel 3).
fn parse_session(id: &str) -> Option<(u32, u64)> {
    let (v, p) = id.split_once('-')?;
    Some((v.parse().ok()?, p.parse().ok()?))
}

fn find_session(
    views: &[Retained<TerminalView>],
    id: &str,
) -> Option<(Retained<TerminalView>, u64, Arc<crate::session::Session>)> {
    let (view_id, pane) = parse_session(id)?;
    let view = views.iter().find(|v| v.view_id() == view_id)?;
    let session = view.session(pane)?;
    Some((view.clone(), pane, session))
}

fn str_param<'a>(job: &'a Job, name: &str) -> Option<&'a str> {
    job.request.params.get(name).and_then(Value::as_str)
}

fn limited(text: String) -> (String, bool) {
    if text.len() <= MAX_RESULT_BYTES {
        return (text, false);
    }
    // Se queda con el final (lo más reciente), cortando en un límite de carácter.
    let mut cut = text.len() - MAX_RESULT_BYTES;
    while !text.is_char_boundary(cut) {
        cut += 1;
    }
    (text[cut..].to_owned(), true)
}

fn execute(mtm: MainThreadMarker, job: &Job) -> String {
    let id = &job.request.id;
    let views = views(mtm);
    let fail = |c: &str, m: &str| protocol::err(id, c, m);
    let not_found = || fail(code::NOT_FOUND, "no existe esa sesión");
    let need_session = || fail(code::BAD_REQUEST, "falta `session`");

    match job.request.method {
        Method::Hello => protocol::ok(id, json!({"protocol": protocol::VERSION})),
        Method::SessionsList => {
            let mut list = Vec::new();
            for view in &views {
                for p in view.pane_infos() {
                    let Some(s) = view.session(p.pane) else {
                        continue;
                    };
                    let (cols, rows) = {
                        let term = s.term.lock().unwrap();
                        (term.grid().cols(), term.grid().rows())
                    };
                    list.push(json!({
                        "id": format!("{}-{}", view.view_id(), p.pane),
                        "window": if view.is_quick() { "quick" } else { "main" },
                        "tab": p.tab,
                        "active": p.active_tab && p.focused,
                        "focused": p.focused,
                        "title": s.title(),
                        "cwd": s.cwd().map(|c| c.display().to_string()),
                        "busy": s.busy.load(std::sync::atomic::Ordering::SeqCst),
                        "ssh_host": s.ssh_host(),
                        "agent": view.agents().hub_ref().session_of_pane(p.pane).is_some(),
                        "cols": cols,
                        "rows": rows,
                    }));
                }
            }
            protocol::ok(id, json!({ "sessions": list }))
        }
        Method::ScreenRead => {
            let Some(sid) = str_param(job, "session") else {
                return need_session();
            };
            let Some((_, _, session)) = find_session(&views, sid) else {
                return not_found();
            };
            let lines = job
                .request
                .params
                .get("lines")
                .and_then(Value::as_u64)
                .map(|n| (n as usize).clamp(1, MAX_LINES));
            let term = session.term.lock().unwrap();
            let text = match lines {
                Some(n) => term.recent_text(n),
                None => term.snapshot_text(),
            };
            let cursor = term.cursor();
            let result = json!({
                "cols": term.grid().cols(),
                "rows": term.grid().rows(),
                "cursor": {"row": cursor.row, "col": cursor.col},
                "alt_screen": term.modes().alt_screen,
            });
            drop(term);
            let (text, truncated) = limited(text);
            let mut result = result;
            result["text"] = json!(text);
            result["truncated"] = json!(truncated);
            protocol::ok(id, result)
        }
        Method::BlocksList => {
            let Some(sid) = str_param(job, "session") else {
                return need_session();
            };
            let Some((_, _, session)) = find_session(&views, sid) else {
                return not_found();
            };
            let limit = job
                .request
                .params
                .get("limit")
                .and_then(Value::as_u64)
                .map_or(20, |n| (n as usize).clamp(1, 200));
            let term = session.term.lock().unwrap();
            // Solo comandos ejecutados (con salida), los más recientes al final.
            let all: Vec<_> = term
                .blocks()
                .iter()
                .filter(|b| b.output.is_some())
                .collect();
            let start = all.len().saturating_sub(limit);
            let blocks: Vec<Value> = all[start..]
                .iter()
                .map(|b| {
                    json!({
                        "id": b.id,
                        "command": b.command,
                        "cwd": b.cwd,
                        "running": !b.is_finished(),
                        "exit_code": b.exit_code,
                        "duration_ms": b.duration().map(|d| d.as_millis() as u64),
                    })
                })
                .collect();
            protocol::ok(id, json!({ "blocks": blocks }))
        }
        Method::BlocksOutput => {
            let (Some(sid), Some(block)) = (
                str_param(job, "session"),
                job.request.params.get("block").and_then(Value::as_u64),
            ) else {
                return fail(code::BAD_REQUEST, "faltan `session` y `block`");
            };
            let Some((_, _, session)) = find_session(&views, sid) else {
                return not_found();
            };
            let term = session.term.lock().unwrap();
            let Some(b) = term.blocks().iter().find(|b| b.id == block) else {
                return fail(code::NOT_FOUND, "no existe ese bloque");
            };
            let Some(text) = term.block_output(b) else {
                return fail(code::NOT_FOUND, "el bloque no ha terminado");
            };
            drop(term);
            let (text, truncated) = limited(text);
            protocol::ok(id, json!({"text": text, "truncated": truncated}))
        }
        Method::InputSendText | Method::InputSendKeys => {
            let Some(sid) = str_param(job, "session") else {
                return need_session();
            };
            let Some((view, _, session)) = find_session(&views, sid) else {
                return not_found();
            };
            let (bytes, what) = if job.request.method == Method::InputSendText {
                let Some(text) = str_param(job, "text") else {
                    return fail(code::BAD_REQUEST, "falta `text`");
                };
                let enter = job.request.params.get("enter").and_then(Value::as_bool) == Some(true);
                match protocol::text_to_bytes(text, enter) {
                    Ok(b) => (
                        b,
                        format!(
                            "Teclear en «{}»:\n\n{}",
                            session.title(),
                            preview(text, enter)
                        ),
                    ),
                    Err(e) => return fail(code::BAD_REQUEST, &e),
                }
            } else {
                let keys: Vec<String> = job
                    .request
                    .params
                    .get("keys")
                    .and_then(Value::as_array)
                    .map(|a| {
                        a.iter()
                            .filter_map(|k| k.as_str().map(str::to_owned))
                            .collect()
                    })
                    .unwrap_or_default();
                match protocol::keys_to_bytes(&keys) {
                    Ok(b) => (
                        b,
                        format!(
                            "Pulsar teclas en «{}»:\n\n{}",
                            session.title(),
                            keys.join(" ")
                        ),
                    ),
                    Err(e) => return fail(code::BAD_REQUEST, &e),
                }
            };
            if let Err(resp) = ensure_permission(mtm, job, Group::Input, &what) {
                return resp;
            }
            session.write(&bytes);
            view.flash_control_note(&job.client.name);
            protocol::ok(id, json!({"sent": bytes.len()}))
        }
        Method::SessionsOpen => {
            let cwd = str_param(job, "cwd").map(PathBuf::from);
            if let Some(c) = &cwd
                && !c.is_dir()
            {
                return fail(code::BAD_REQUEST, "`cwd` no es una carpeta");
            }
            let what = format!(
                "Abrir una sesión nueva{}",
                cwd.as_ref()
                    .map(|c| format!(" en {}", c.display()))
                    .unwrap_or_default()
            );
            let Some(view) = views.iter().find(|v| !v.is_quick()).or(views.first()) else {
                return not_found();
            };
            if let Err(resp) = ensure_permission(mtm, job, Group::Sessions, &what) {
                return resp;
            }
            match view.control_open_tab(cwd) {
                Some(pane) => {
                    view.flash_control_note(&job.client.name);
                    protocol::ok(
                        id,
                        json!({"session": format!("{}-{}", view.view_id(), pane)}),
                    )
                }
                None => fail(code::NOT_FOUND, "no se pudo abrir la sesión"),
            }
        }
        Method::SessionsClose => {
            let Some(sid) = str_param(job, "session") else {
                return need_session();
            };
            let Some((view, pane, session)) = find_session(&views, sid) else {
                return not_found();
            };
            let what = format!("Cerrar la sesión «{}»", session.title());
            if let Err(resp) = ensure_permission(mtm, job, Group::Sessions, &what) {
                return resp;
            }
            view.control_close_pane(pane);
            protocol::ok(id, json!({"closed": sid}))
        }
    }
}

/// El texto que se va a teclear, para enseñarlo en el diálogo (con los
/// saltos de línea visibles y recortado).
fn preview(text: &str, enter: bool) -> String {
    let mut shown: String = text
        .chars()
        .take(300)
        .map(|c| match c {
            '\n' | '\r' => '↵',
            '\t' => '⇥',
            c => c,
        })
        .collect();
    if text.chars().count() > 300 {
        shown.push('…');
    }
    if enter {
        shown.push_str(" ↵");
    }
    shown
}

/// Quién es el proceso que se conectó, para enseñarlo en el diálogo.
fn process_label(pid: Option<u32>) -> String {
    let Some(pid) = pid else {
        return "proceso desconocido".to_owned();
    };
    let args = crate::proc::args_of(pid).unwrap_or_default();
    let cmd: String = args.iter().take(3).cloned().collect::<Vec<_>>().join(" ");
    format!(
        "pid {pid}{}",
        if cmd.is_empty() {
            String::new()
        } else {
            format!(" ({cmd})")
        }
    )
}

/// Pide (o recuerda) el permiso de `job.client` para un grupo de acciones.
/// `Err` es la respuesta de «denegado».
fn ensure_permission(
    mtm: MainThreadMarker,
    job: &Job,
    group: Group,
    what: &str,
) -> Result<(), String> {
    let key = (job.client.name.clone(), group);
    let denied = |why: &str| protocol::err(&job.request.id, code::DENIED, why);
    let verdict = STATE.with(|s| {
        let st = s.borrow();
        let st = st.as_ref()?;
        if st.grants.contains(&key) {
            return Some(Ok(()));
        }
        // Un agente al que se acaba de decir que no, no vuelve a preguntar de golpe.
        if st
            .denied
            .get(&key)
            .is_some_and(|t| t.elapsed() < DENY_COOLDOWN)
        {
            return Some(Err("permiso denegado hace un momento".to_owned()));
        }
        None
    });
    match verdict {
        Some(Ok(())) => return Ok(()),
        Some(Err(why)) => return Err(denied(&why)),
        None => {}
    }

    // Solo en depuración (`RT_CONTROL_AUTO=allow|deny`): contesta sin diálogo,
    // para las pruebas. En una compilación `release` no existe.
    #[cfg(debug_assertions)]
    if let Ok(auto) = std::env::var("RT_CONTROL_AUTO") {
        return if auto == "allow" {
            Ok(())
        } else {
            Err(denied("denegado (RT_CONTROL_AUTO)"))
        };
    }

    let alert = NSAlert::new(mtm);
    alert.setMessageText(&NSString::from_str(&format!(
        "«{}» quiere controlar tu terminal",
        job.client.name
    )));
    alert.setInformativeText(&NSString::from_str(&format!(
        "{what}\n\nProceso: {}\nEl nombre del agente lo declara él mismo; fíate del proceso.",
        process_label(job.client.pid)
    )));
    // El primero es el botón por defecto (Intro): por seguridad, Denegar.
    alert.addButtonWithTitle(&NSString::from_str("Denegar"));
    alert.addButtonWithTitle(&NSString::from_str("Permitir una vez"));
    alert.addButtonWithTitle(&NSString::from_str("Permitir en esta sesión"));
    #[allow(deprecated)]
    NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
    let answer = alert.runModal();
    let choice = answer - NSAlertFirstButtonReturn;
    STATE.with(|s| {
        if let Some(st) = s.borrow_mut().as_mut() {
            match choice {
                2 => {
                    st.grants.insert(key.clone());
                }
                1 => {}
                _ => {
                    st.denied.insert(key.clone(), Instant::now());
                }
            }
        }
    });
    if choice == 1 || choice == 2 {
        Ok(())
    } else {
        Err(denied("el usuario denegó el permiso"))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn session_ids_round_trip() {
        assert_eq!(parse_session("1-3"), Some((1, 3)));
        assert_eq!(parse_session("12-999"), Some((12, 999)));
        for bad in ["", "1", "a-b", "1-", "-2", "1-2-3x", "1 -2"] {
            assert_eq!(parse_session(bad), None, "{bad}");
        }
    }

    #[test]
    fn long_results_keep_the_end() {
        let (t, cut) = limited("abc".to_owned());
        assert_eq!((t.as_str(), cut), ("abc", false));
        let big = format!("{}FIN", "ñ".repeat(MAX_RESULT_BYTES));
        let (t, cut) = limited(big);
        assert!(cut && t.ends_with("FIN") && t.len() <= MAX_RESULT_BYTES);
        assert!(std::str::from_utf8(t.as_bytes()).is_ok());
    }

    #[test]
    fn the_preview_shows_newlines_and_trims() {
        assert_eq!(preview("ls\nrm x", true), "ls↵rm x ↵");
        let long = "a".repeat(400);
        let p = preview(&long, false);
        assert_eq!(p.chars().count(), 301);
        assert!(p.ends_with('…'));
    }

    #[test]
    fn stale_socket_folders_of_dead_processes_are_removed() {
        let base = std::env::temp_dir().join(format!("rt-stale-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&base);
        std::fs::create_dir_all(&base).unwrap();
        let dead = base.join("rust-terminal-control-999999-abcdef");
        let mine = base.join(format!(
            "rust-terminal-control-{}-abcdef",
            std::process::id()
        ));
        let other = base.join("otra-cosa");
        for d in [&dead, &mine, &other] {
            std::fs::create_dir_all(d).unwrap();
        }
        remove_stale_dirs(&base);
        assert!(!dead.exists(), "el de un proceso muerto se borra");
        assert!(mine.exists(), "el propio no");
        assert!(other.exists(), "lo que no es nuestro tampoco");
        let _ = std::fs::remove_dir_all(&base);
    }
}
