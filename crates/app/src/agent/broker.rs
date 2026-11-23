//! Broker local entre Claude Code y la interfaz.
//!
//! Un servidor HTTP síncrono en `127.0.0.1` (un hilo por conexión, sin
//! `tokio`). Claude Code le envía los hooks y la status line; la interfaz
//! recibe [`BrokerEvent`]s por un callback y contesta las peticiones de
//! permiso con [`Broker::resolve`].
//!
//! **Autenticación:** token aleatorio por lanzamiento y una clave por agente,
//! ambos en la ruta (`/hook/<token>/<agente>/<Evento>`). Una clave desconocida
//! o un `session_id` que no corresponde al agente se rechazan con 403.
//!
//! **Fail safe** (spike de hooks, `docs/spike-hooks.md`): una respuesta `{}`
//! hace que Claude Code muestre su propio prompt. El broker contesta `{}`, es
//! decir, no decide, cuando:
//! - vence el plazo sin que el usuario responda;
//! - llega una segunda petición del mismo agente mientras hay una pendiente
//!   (no se sabe a cuál se refiere el prompt visible): se liberan las dos;
//! - Claude Code cierra la conexión (se contestó en la sesión: no se responde);
//! - el agente se da de baja.
//!
//! Nunca se permite por defecto.

use std::collections::HashMap;
use std::fs::File;
use std::io::{self, BufReader, Read};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, Sender};
use std::sync::{Arc, Mutex};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::http::{self, HttpError};

/// Identifica a un agente dentro de la app (va en la URL de sus hooks).
pub type AgentKey = String;

/// Conexiones simultáneas máximas; el resto se descarta.
const MAX_CONNECTIONS: usize = 64;
/// Cada cuánto se comprueba si Claude Code cerró la conexión.
const POLL: Duration = Duration::from_millis(250);

#[derive(Debug, Clone, PartialEq)]
pub enum Decision {
    Allow,
    Deny { message: Option<String> },
}

/// Por qué una petición de permiso terminó sin decisión de la interfaz.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Ended {
    TimedOut,
    /// Llegó otra petición del mismo agente: ambigua, se liberan las dos.
    Superseded,
    /// Claude Code cerró la conexión (se respondió en la propia sesión).
    ClientGone,
    /// El agente se dio de baja.
    AgentGone,
}

#[derive(Debug, Clone)]
pub struct PermissionRequest {
    pub id: u64,
    pub agent: AgentKey,
    pub session_id: String,
    pub tool_name: String,
    /// Lo que se aprueba: el `tool_input` del propio `PermissionRequest`
    /// (otro hook puede haber reescrito el del `PreToolUse`).
    pub tool_input: Value,
    pub cwd: String,
    pub suggestions: Value,
}

#[derive(Debug, Clone)]
pub enum BrokerEvent {
    Hook {
        agent: AgentKey,
        event: String,
        payload: Value,
    },
    StatusLine {
        agent: AgentKey,
        payload: Value,
    },
    Permission(PermissionRequest),
    PermissionEnded {
        id: u64,
        agent: AgentKey,
        reason: Ended,
    },
}

enum Outcome {
    Decision(Decision),
    Superseded,
}

struct Pending {
    agent: AgentKey,
    tx: Sender<Outcome>,
}

#[derive(Default)]
struct AgentState {
    session_id: Option<String>,
    pending: Option<u64>,
}

#[derive(Default)]
struct State {
    agents: HashMap<AgentKey, AgentState>,
    pending: HashMap<u64, Pending>,
}

struct Shared {
    token: String,
    deadline: Duration,
    notify: Box<dyn Fn(BrokerEvent) + Send + Sync>,
    state: Mutex<State>,
    next_id: AtomicU64,
    connections: AtomicUsize,
    stop: AtomicBool,
}

pub struct Broker {
    port: u16,
    shared: Arc<Shared>,
}

fn random_hex(bytes: usize) -> io::Result<String> {
    let mut buf = vec![0u8; bytes];
    File::open("/dev/urandom")?.read_exact(&mut buf)?;
    Ok(buf.iter().map(|b| format!("{b:02x}")).collect())
}

/// Comparación sin salida temprana, para no filtrar el token por tiempos.
fn same_secret(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0, |acc, (x, y)| acc | (x ^ y))
            == 0
}

impl Broker {
    /// Arranca el servidor en un puerto aleatorio de loopback. `notify` se
    /// llama desde los hilos del broker: debe limitarse a encolar y avisar
    /// al hilo principal. `deadline` es lo que se espera a que el usuario
    /// conteste un permiso antes de devolver "sin decisión".
    pub fn start(
        deadline: Duration,
        notify: impl Fn(BrokerEvent) + Send + Sync + 'static,
    ) -> io::Result<Broker> {
        let listener = TcpListener::bind(("127.0.0.1", 0))?;
        let port = listener.local_addr()?.port();
        let shared = Arc::new(Shared {
            token: random_hex(16)?,
            deadline,
            notify: Box::new(notify),
            state: Mutex::new(State::default()),
            next_id: AtomicU64::new(0),
            connections: AtomicUsize::new(0),
            stop: AtomicBool::new(false),
        });
        let accept_shared = shared.clone();
        std::thread::Builder::new()
            .name("broker-accept".into())
            .spawn(move || {
                for stream in listener.incoming() {
                    if accept_shared.stop.load(Ordering::SeqCst) {
                        break;
                    }
                    let Ok(stream) = stream else { continue };
                    if accept_shared.connections.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        accept_shared.connections.fetch_sub(1, Ordering::SeqCst);
                        continue;
                    }
                    let shared = accept_shared.clone();
                    let spawned = std::thread::Builder::new()
                        .name("broker-conn".into())
                        .spawn(move || {
                            handle(&shared, stream);
                            shared.connections.fetch_sub(1, Ordering::SeqCst);
                        });
                    if spawned.is_err() {
                        accept_shared.connections.fetch_sub(1, Ordering::SeqCst);
                    }
                }
            })?;
        Ok(Broker { port, shared })
    }

    pub fn port(&self) -> u16 {
        self.port
    }

    /// Da de alta un agente y devuelve su clave (va en las URLs de sus hooks).
    pub fn register_agent(&self) -> io::Result<AgentKey> {
        let key = random_hex(8)?;
        self.shared
            .state
            .lock()
            .unwrap()
            .agents
            .insert(key.clone(), AgentState::default());
        Ok(key)
    }

    /// Da de baja un agente: sus peticiones pendientes se liberan sin decisión.
    pub fn forget_agent(&self, key: &str) {
        let mut state = self.shared.state.lock().unwrap();
        state.agents.remove(key);
        // Al soltar el emisor, el hilo de la petición contesta `{}`.
        state.pending.retain(|_, p| p.agent != key);
    }

    pub fn hook_url(&self, key: &str, event: &str) -> String {
        format!(
            "http://127.0.0.1:{}/hook/{}/{key}/{event}",
            self.port, self.shared.token
        )
    }

    pub fn statusline_url(&self, key: &str) -> String {
        format!(
            "http://127.0.0.1:{}/statusline/{}/{key}",
            self.port, self.shared.token
        )
    }

    /// Entrega la decisión del usuario. `false` si la petición ya no existe
    /// (venció, la cerró Claude Code o la liberó otra petición).
    pub fn resolve(&self, id: u64, decision: Decision) -> bool {
        let mut state = self.shared.state.lock().unwrap();
        let Some(p) = state.pending.remove(&id) else {
            return false;
        };
        if let Some(a) = state.agents.get_mut(&p.agent) {
            a.pending = None;
        }
        p.tx.send(Outcome::Decision(decision)).is_ok()
    }
}

impl Drop for Broker {
    fn drop(&mut self) {
        self.shared.stop.store(true, Ordering::SeqCst);
        // Despierta el `accept` bloqueado para que el hilo termine.
        let _ = TcpStream::connect(("127.0.0.1", self.port));
        self.shared.state.lock().unwrap().pending.clear();
    }
}

fn permission_json(decision: &Decision) -> String {
    let decision = match decision {
        Decision::Allow => json!({"behavior": "allow"}),
        Decision::Deny { message: None } => json!({"behavior": "deny"}),
        Decision::Deny { message: Some(m) } => json!({"behavior": "deny", "message": m}),
    };
    json!({"hookSpecificOutput": {"hookEventName": "PermissionRequest", "decision": decision}})
        .to_string()
}

fn respond(stream: &mut TcpStream, status: u16, body: &str) {
    let _ = http::write_response(stream, status, body);
    let _ = stream.shutdown(Shutdown::Both);
}

fn handle(shared: &Arc<Shared>, mut stream: TcpStream) {
    let _ = stream.set_read_timeout(Some(Duration::from_secs(10)));
    let _ = stream.set_write_timeout(Some(Duration::from_secs(10)));
    let Ok(clone) = stream.try_clone() else {
        return;
    };
    let request = match http::read_request(&mut BufReader::new(clone)) {
        Ok(r) => r,
        Err(HttpError::TooLarge) => return respond(&mut stream, 413, "{}"),
        Err(HttpError::Malformed(_)) => return respond(&mut stream, 400, "{}"),
        Err(HttpError::Io(_)) => return,
    };
    if request.method != "POST" {
        return respond(&mut stream, 405, "{}");
    }

    // /hook/<token>/<agente>/<Evento>  o  /statusline/<token>/<agente>
    let parts: Vec<&str> = request.path.trim_start_matches('/').split('/').collect();
    let (kind, token, agent, event) = match parts.as_slice() {
        ["hook", t, a, e] => ("hook", *t, *a, Some(*e)),
        ["statusline", t, a] => ("statusline", *t, *a, None),
        _ => return respond(&mut stream, 403, "{}"),
    };
    if !same_secret(token, &shared.token) {
        return respond(&mut stream, 403, "{}");
    }
    let Ok(payload) = serde_json::from_slice::<Value>(&request.body) else {
        return respond(&mut stream, 400, "{}");
    };
    let agent = agent.to_owned();

    // El agente debe existir y su `session_id` debe ser el que conocemos.
    // `SessionStart` puede cambiarlo (por ejemplo tras `/clear`).
    let session_id = payload
        .get("session_id")
        .and_then(Value::as_str)
        .map(str::to_owned);
    {
        let mut state = shared.state.lock().unwrap();
        let Some(a) = state.agents.get_mut(&agent) else {
            drop(state);
            return respond(&mut stream, 403, "{}");
        };
        if let Some(id) = &session_id {
            let rebind = event == Some("SessionStart");
            match &a.session_id {
                Some(known) if known != id && !rebind => {
                    drop(state);
                    return respond(&mut stream, 403, "{}");
                }
                _ => a.session_id = Some(id.clone()),
            }
        }
    }

    match (kind, event) {
        ("statusline", _) => {
            (shared.notify)(BrokerEvent::StatusLine { agent, payload });
            respond(&mut stream, 200, "{}");
        }
        ("hook", Some("PermissionRequest")) => permission(shared, stream, agent, payload),
        ("hook", Some(event)) => {
            (shared.notify)(BrokerEvent::Hook {
                agent,
                event: event.to_owned(),
                payload,
            });
            respond(&mut stream, 200, "{}");
        }
        _ => respond(&mut stream, 403, "{}"),
    }
}

/// ¿Cerró Claude Code su extremo? (se contestó el prompt en la propia sesión)
fn client_gone(stream: &TcpStream) -> bool {
    if stream.set_nonblocking(true).is_err() {
        return true;
    }
    let mut byte = [0u8; 1];
    let gone = match stream.peek(&mut byte) {
        Ok(0) => true,
        Ok(_) => false,
        Err(e) => e.kind() != io::ErrorKind::WouldBlock,
    };
    let _ = stream.set_nonblocking(false);
    gone
}

fn permission(shared: &Arc<Shared>, mut stream: TcpStream, agent: AgentKey, payload: Value) {
    let text = |k: &str| {
        payload
            .get(k)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let id = shared.next_id.fetch_add(1, Ordering::SeqCst) + 1;
    let (tx, rx) = mpsc::channel();

    {
        let mut state = shared.state.lock().unwrap();
        let previous = state.agents.get_mut(&agent).and_then(|a| a.pending.take());
        if let Some(prev) = previous {
            // Dos peticiones a la vez: no sabemos a cuál se refiere el prompt
            // visible. Se liberan las dos sin decisión.
            if let Some(p) = state.pending.remove(&prev) {
                let _ = p.tx.send(Outcome::Superseded);
            }
            drop(state);
            return respond(&mut stream, 200, "{}");
        }
        if let Some(a) = state.agents.get_mut(&agent) {
            a.pending = Some(id);
        }
        state.pending.insert(
            id,
            Pending {
                agent: agent.clone(),
                tx,
            },
        );
    }

    (shared.notify)(BrokerEvent::Permission(PermissionRequest {
        id,
        agent: agent.clone(),
        session_id: text("session_id"),
        tool_name: text("tool_name"),
        tool_input: payload.get("tool_input").cloned().unwrap_or(Value::Null),
        cwd: text("cwd"),
        suggestions: payload
            .get("permission_suggestions")
            .cloned()
            .unwrap_or(Value::Null),
    }));

    let cleanup = |shared: &Shared| {
        let mut state = shared.state.lock().unwrap();
        state.pending.remove(&id);
        if let Some(a) = state.agents.get_mut(&agent)
            && a.pending == Some(id)
        {
            a.pending = None;
        }
    };
    let ended = |reason| {
        (shared.notify)(BrokerEvent::PermissionEnded {
            id,
            agent: agent.clone(),
            reason,
        });
    };

    let start = Instant::now();
    loop {
        match rx.recv_timeout(POLL) {
            Ok(Outcome::Decision(d)) => return respond(&mut stream, 200, &permission_json(&d)),
            Ok(Outcome::Superseded) => {
                ended(Ended::Superseded);
                return respond(&mut stream, 200, "{}");
            }
            Err(RecvTimeoutError::Disconnected) => {
                ended(Ended::AgentGone);
                return respond(&mut stream, 200, "{}");
            }
            Err(RecvTimeoutError::Timeout) => {
                if start.elapsed() >= shared.deadline {
                    cleanup(shared);
                    ended(Ended::TimedOut);
                    return respond(&mut stream, 200, "{}");
                }
                if client_gone(&stream) {
                    cleanup(shared);
                    ended(Ended::ClientGone);
                    return;
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Write;
    use std::sync::mpsc::Receiver;

    struct Rig {
        broker: Broker,
        events: Receiver<BrokerEvent>,
    }

    fn rig(deadline: Duration) -> Rig {
        let (tx, events) = mpsc::channel();
        let broker = Broker::start(deadline, move |e| {
            let _ = tx.send(e);
        })
        .unwrap();
        Rig { broker, events }
    }

    /// POST crudo; devuelve (código, cuerpo).
    fn post(port: u16, path: &str, body: &str) -> (u16, String) {
        let mut s = TcpStream::connect(("127.0.0.1", port)).unwrap();
        write!(
            s,
            "POST {path} HTTP/1.1\r\nHost: x\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        let code = out[9..12].parse().unwrap();
        let body = out.split("\r\n\r\n").nth(1).unwrap_or("").to_owned();
        (code, body)
    }

    fn path_of(url: &str) -> String {
        url.split_once("127.0.0.1:")
            .map(|(_, rest)| rest.split_once('/').map(|(_, p)| format!("/{p}")).unwrap())
            .unwrap()
    }

    fn perm_body(session: &str, input: &str) -> String {
        json!({
            "session_id": session, "hook_event_name": "PermissionRequest",
            "tool_name": "Bash", "tool_input": {"command": input}, "cwd": "/tmp"
        })
        .to_string()
    }

    fn next_permission(rig: &Rig) -> PermissionRequest {
        loop {
            match rig.events.recv_timeout(Duration::from_secs(5)).unwrap() {
                BrokerEvent::Permission(p) => return p,
                _ => continue,
            }
        }
    }

    #[test]
    fn token_incorrecto_da_403_y_no_genera_evento() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let url = r
            .broker
            .hook_url(&key, "Stop")
            .replace(&r.shared_token(), "0000");
        let (code, _) = post(r.broker.port(), &path_of(&url), r#"{"session_id":"s"}"#);
        assert_eq!(code, 403);
        assert!(r.events.recv_timeout(Duration::from_millis(200)).is_err());
    }

    #[test]
    fn agente_desconocido_da_403() {
        let r = rig(Duration::from_secs(5));
        let url = r.broker.hook_url("noexiste", "Stop");
        let (code, _) = post(r.broker.port(), &path_of(&url), r#"{"session_id":"s"}"#);
        assert_eq!(code, 403);
    }

    #[test]
    fn solo_acepta_post() {
        let r = rig(Duration::from_secs(5));
        let mut s = TcpStream::connect(("127.0.0.1", r.broker.port())).unwrap();
        write!(s, "GET / HTTP/1.1\r\n\r\n").unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 405"));
    }

    #[test]
    fn json_invalido_da_400() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let url = r.broker.hook_url(&key, "Stop");
        assert_eq!(post(r.broker.port(), &path_of(&url), "no es json").0, 400);
    }

    #[test]
    fn cuerpo_gigante_da_413() {
        let r = rig(Duration::from_secs(5));
        let mut s = TcpStream::connect(("127.0.0.1", r.broker.port())).unwrap();
        write!(
            s,
            "POST /x HTTP/1.1\r\nContent-Length: {}\r\n\r\n",
            http::MAX_BODY_BYTES + 1
        )
        .unwrap();
        let mut out = String::new();
        s.read_to_string(&mut out).unwrap();
        assert!(out.starts_with("HTTP/1.1 413"));
    }

    #[test]
    fn un_hook_normal_llega_a_la_interfaz_y_contesta_vacio() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let url = r.broker.hook_url(&key, "Stop");
        let (code, body) = post(r.broker.port(), &path_of(&url), r#"{"session_id":"s1"}"#);
        assert_eq!((code, body.as_str()), (200, "{}"));
        match r.events.recv_timeout(Duration::from_secs(2)).unwrap() {
            BrokerEvent::Hook {
                agent,
                event,
                payload,
            } => {
                assert_eq!((agent, event.as_str()), (key, "Stop"));
                assert_eq!(payload["session_id"], "s1");
            }
            other => panic!("evento inesperado: {other:?}"),
        }
    }

    #[test]
    fn la_status_line_llega_como_su_propio_evento() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let url = r.broker.statusline_url(&key);
        let (code, _) = post(
            r.broker.port(),
            &path_of(&url),
            r#"{"session_id":"s1","version":"2.1.330"}"#,
        );
        assert_eq!(code, 200);
        assert!(matches!(
            r.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            BrokerEvent::StatusLine { .. }
        ));
    }

    #[test]
    fn un_session_id_distinto_se_rechaza_salvo_en_session_start() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let port = r.broker.port();
        let stop = path_of(&r.broker.hook_url(&key, "Stop"));
        let start = path_of(&r.broker.hook_url(&key, "SessionStart"));
        assert_eq!(post(port, &stop, r#"{"session_id":"A"}"#).0, 200);
        assert_eq!(
            post(port, &stop, r#"{"session_id":"B"}"#).0,
            403,
            "suplantación"
        );
        assert_eq!(post(port, &start, r#"{"session_id":"B"}"#).0, 200, "/clear");
        assert_eq!(post(port, &stop, r#"{"session_id":"B"}"#).0, 200);
        assert_eq!(post(port, &stop, r#"{"session_id":"A"}"#).0, 403);
    }

    #[test]
    fn permitir_desde_la_interfaz_contesta_allow() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let port = r.broker.port();
        let path = path_of(&r.broker.hook_url(&key, "PermissionRequest"));
        let client = std::thread::spawn(move || post(port, &path, &perm_body("s1", "touch x")));
        let req = next_permission(&r);
        assert_eq!(req.tool_name, "Bash");
        assert_eq!(req.tool_input["command"], "touch x");
        assert_eq!(req.session_id, "s1");
        assert!(r.broker.resolve(req.id, Decision::Allow));
        let (code, body) = client.join().unwrap();
        assert_eq!(code, 200);
        let v: Value = serde_json::from_str(&body).unwrap();
        assert_eq!(
            v["hookSpecificOutput"]["hookEventName"],
            "PermissionRequest"
        );
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "allow");
    }

    #[test]
    fn denegar_lleva_el_mensaje() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let port = r.broker.port();
        let path = path_of(&r.broker.hook_url(&key, "PermissionRequest"));
        let client = std::thread::spawn(move || post(port, &path, &perm_body("s1", "rm x")));
        let req = next_permission(&r);
        let deny = Decision::Deny {
            message: Some("no, usa otra ruta".into()),
        };
        assert!(r.broker.resolve(req.id, deny));
        let v: Value = serde_json::from_str(&client.join().unwrap().1).unwrap();
        assert_eq!(v["hookSpecificOutput"]["decision"]["behavior"], "deny");
        assert_eq!(
            v["hookSpecificOutput"]["decision"]["message"],
            "no, usa otra ruta"
        );
    }

    #[test]
    fn si_vence_el_plazo_no_se_decide() {
        let r = rig(Duration::from_millis(400));
        let key = r.broker.register_agent().unwrap();
        let path = path_of(&r.broker.hook_url(&key, "PermissionRequest"));
        let (code, body) = post(r.broker.port(), &path, &perm_body("s1", "ls"));
        assert_eq!((code, body.as_str()), (200, "{}"));
        let req = next_permission(&r);
        assert!(matches!(
            r.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            BrokerEvent::PermissionEnded {
                reason: Ended::TimedOut,
                ..
            }
        ));
        assert!(
            !r.broker.resolve(req.id, Decision::Allow),
            "ya no se puede contestar"
        );
    }

    #[test]
    fn dos_peticiones_del_mismo_agente_se_liberan_sin_decision() {
        let r = rig(Duration::from_secs(5));
        let key = r.broker.register_agent().unwrap();
        let port = r.broker.port();
        let path = path_of(&r.broker.hook_url(&key, "PermissionRequest"));
        let p1 = path.clone();
        let first = std::thread::spawn(move || post(port, &p1, &perm_body("s1", "a")));
        let req = next_permission(&r);
        let (code, body) = post(port, &path, &perm_body("s1", "b"));
        assert_eq!((code, body.as_str()), (200, "{}"));
        assert_eq!(first.join().unwrap().1, "{}");
        assert!(matches!(
            r.events.recv_timeout(Duration::from_secs(2)).unwrap(),
            BrokerEvent::PermissionEnded {
                reason: Ended::Superseded,
                ..
            }
        ));
        assert!(!r.broker.resolve(req.id, Decision::Allow));
    }

    #[test]
    fn agentes_distintos_no_se_interfieren() {
        let r = rig(Duration::from_secs(5));
        let (k1, k2) = (
            r.broker.register_agent().unwrap(),
            r.broker.register_agent().unwrap(),
        );
        let port = r.broker.port();
        let (p1, p2) = (
            path_of(&r.broker.hook_url(&k1, "PermissionRequest")),
            path_of(&r.broker.hook_url(&k2, "PermissionRequest")),
        );
        let a = std::thread::spawn(move || post(port, &p1, &perm_body("s1", "a")));
        let b = std::thread::spawn(move || post(port, &p2, &perm_body("s2", "b")));
        let (x, y) = (next_permission(&r), next_permission(&r));
        assert_ne!(x.agent, y.agent);
        assert!(r.broker.resolve(x.id, Decision::Allow));
        assert!(r.broker.resolve(y.id, Decision::Deny { message: None }));
        for t in [a, b] {
            assert!(t.join().unwrap().1.contains("behavior"));
        }
    }

    #[test]
    fn si_claude_cierra_la_conexion_se_retira_la_peticion() {
        let r = rig(Duration::from_secs(30));
        let key = r.broker.register_agent().unwrap();
        let path = path_of(&r.broker.hook_url(&key, "PermissionRequest"));
        let body = perm_body("s1", "ls");
        let mut s = TcpStream::connect(("127.0.0.1", r.broker.port())).unwrap();
        write!(
            s,
            "POST {path} HTTP/1.1\r\nContent-Length: {}\r\n\r\n{body}",
            body.len()
        )
        .unwrap();
        let req = next_permission(&r);
        drop(s); // el usuario contestó en la sesión
        assert!(matches!(
            r.events.recv_timeout(Duration::from_secs(3)).unwrap(),
            BrokerEvent::PermissionEnded {
                reason: Ended::ClientGone,
                ..
            }
        ));
        assert!(!r.broker.resolve(req.id, Decision::Allow));
    }

    #[test]
    fn dar_de_baja_al_agente_libera_su_peticion() {
        let r = rig(Duration::from_secs(30));
        let key = r.broker.register_agent().unwrap();
        let port = r.broker.port();
        let path = path_of(&r.broker.hook_url(&key, "PermissionRequest"));
        let client = std::thread::spawn(move || post(port, &path, &perm_body("s1", "ls")));
        next_permission(&r);
        r.broker.forget_agent(&key);
        assert_eq!(client.join().unwrap().1, "{}");
    }

    impl Rig {
        fn shared_token(&self) -> String {
            self.broker.shared.token.clone()
        }
    }
}
