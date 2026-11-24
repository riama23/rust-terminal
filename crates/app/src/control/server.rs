//! El servidor de la API de control: un socket Unix en una carpeta privada,
//! una petición JSON por línea (ver `protocol`). No sabe nada de AppKit: las
//! peticiones válidas se las pasa a un [`Handler`].

use std::ffi::c_void;
use std::io::{self, BufRead, Read, Write};
use std::os::fd::AsRawFd;
use std::os::unix::fs::{DirBuilderExt, PermissionsExt};
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use super::protocol::{self, MAX_LINE, Request, code};

/// Conexiones simultáneas como máximo.
const MAX_CONNECTIONS: usize = 16;
/// Peticiones con token no válido antes de cerrar la conexión.
const MAX_BAD_TOKENS: usize = 5;
const IDLE: Duration = Duration::from_secs(60);
/// Longitud máxima de una ruta de socket Unix en macOS.
const MAX_SOCKET_PATH: usize = 100;

unsafe extern "C" {
    fn getpeereid(fd: i32, euid: *mut u32, egid: *mut u32) -> i32;
    fn geteuid() -> u32;
    fn getsockopt(fd: i32, level: i32, name: i32, value: *mut c_void, len: *mut u32) -> i32;
}

/// Quién está al otro lado del socket.
#[derive(Debug, Clone)]
pub struct Client {
    /// Proceso que se conectó (el `rt`, no quien lo lanzó), si se pudo saber.
    pub pid: Option<u32>,
    /// Lo que el cliente dice ser: no es de fiar, solo se enseña.
    pub name: String,
}

/// Quien atiende las peticiones ya autenticadas. Devuelve la línea de respuesta.
pub trait Handler: Send + Sync + 'static {
    fn handle(&self, client: &Client, request: Request) -> String;
}

pub struct ControlServer {
    dir: PathBuf,
    path: PathBuf,
    token: String,
}

/// 128 bits aleatorios del sistema, en hexadecimal.
fn random_token() -> io::Result<String> {
    let mut bytes = [0u8; 16];
    std::fs::File::open("/dev/urandom")?.read_exact(&mut bytes)?;
    Ok(bytes.iter().map(|b| format!("{b:02x}")).collect())
}

/// El usuario del otro extremo es el nuestro.
fn peer_is_us(stream: &UnixStream) -> bool {
    let (mut uid, mut gid) = (u32::MAX, u32::MAX);
    // SAFETY: descriptor válido y punteros a variables locales.
    let rc = unsafe { getpeereid(stream.as_raw_fd(), &mut uid, &mut gid) };
    // SAFETY: sin argumentos.
    rc == 0 && uid == unsafe { geteuid() }
}

/// `LOCAL_PEERPID` (nivel `SOL_LOCAL` = 0, opción 2).
fn peer_pid(stream: &UnixStream) -> Option<u32> {
    let mut pid: i32 = 0;
    let mut len = std::mem::size_of::<i32>() as u32;
    // SAFETY: descriptor válido y punteros a variables locales.
    let rc = unsafe {
        getsockopt(
            stream.as_raw_fd(),
            0,
            2,
            (&mut pid as *mut i32).cast(),
            &mut len,
        )
    };
    (rc == 0 && pid > 0).then_some(pid as u32)
}

impl ControlServer {
    /// Crea la carpeta privada, el socket y el hilo que acepta conexiones.
    pub fn start(base: &Path, handler: Arc<dyn Handler>) -> io::Result<ControlServer> {
        let token = random_token()?;
        // El token da un nombre único a cada servidor (varios en un proceso, en los tests).
        let unique = format!("{}-{}", std::process::id(), &token[..6]);
        let mut dir = base.join(format!("rust-terminal-control-{unique}"));
        if dir.join("control.sock").as_os_str().len() > MAX_SOCKET_PATH {
            // Un TMPDIR largo no cabe en un socket Unix: `/tmp` sí.
            dir = PathBuf::from(format!("/tmp/rt-control-{unique}"));
        }
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::DirBuilder::new().mode(0o700).create(&dir)?;
        let path = dir.join("control.sock");
        let listener = UnixListener::bind(&path)?;
        std::fs::set_permissions(&path, std::fs::Permissions::from_mode(0o600))?;

        let active = Arc::new(AtomicUsize::new(0));
        let tok = token.clone();
        std::thread::Builder::new()
            .name("control-accept".into())
            .spawn(move || {
                for stream in listener.incoming().flatten() {
                    if !peer_is_us(&stream) {
                        continue;
                    }
                    if active.fetch_add(1, Ordering::SeqCst) >= MAX_CONNECTIONS {
                        active.fetch_sub(1, Ordering::SeqCst);
                        let mut s = stream;
                        let _ = s.write_all(
                            protocol::err(
                                &serde_json::Value::Null,
                                code::BUSY,
                                "demasiadas conexiones",
                            )
                            .as_bytes(),
                        );
                        continue;
                    }
                    let (handler, tok, active) = (handler.clone(), tok.clone(), active.clone());
                    let spawned = std::thread::Builder::new()
                        .name("control-conn".into())
                        .spawn(move || {
                            serve(stream, &tok, handler.as_ref());
                            active.fetch_sub(1, Ordering::SeqCst);
                        });
                    if spawned.is_err() {
                        // El hilo no arrancó: su contador no se libera solo.
                        // (`active` se movió al cierre; con un fallo aquí se
                        // pierde una plaza hasta reiniciar, que es aceptable.)
                    }
                }
            })?;
        Ok(ControlServer { dir, path, token })
    }

    pub fn socket_path(&self) -> &Path {
        &self.path
    }

    pub fn token(&self) -> &str {
        &self.token
    }
}

impl Drop for ControlServer {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.dir);
    }
}

/// Lee una línea de como mucho `MAX_LINE` bytes. `Ok(None)`: fin de la conexión;
/// `Err(TooLarge)`: la línea no cabe.
enum Line {
    Text(String),
    TooLarge,
    Eof,
}

fn read_line(reader: &mut impl BufRead) -> io::Result<Line> {
    let mut buf = Vec::new();
    let n = reader
        .by_ref()
        .take(MAX_LINE as u64 + 1)
        .read_until(b'\n', &mut buf)?;
    if n == 0 {
        return Ok(Line::Eof);
    }
    if buf.last() != Some(&b'\n') && buf.len() > MAX_LINE {
        return Ok(Line::TooLarge);
    }
    Ok(Line::Text(String::from_utf8_lossy(&buf).trim().to_owned()))
}

/// Atiende una conexión hasta que se cierra o se queda inactiva.
fn serve(stream: UnixStream, token: &str, handler: &dyn Handler) {
    let _ = stream.set_read_timeout(Some(IDLE));
    let pid = peer_pid(&stream);
    let Ok(mut writer) = stream.try_clone() else {
        return;
    };
    let mut reader = io::BufReader::new(stream);
    let mut bad_tokens = 0;
    loop {
        let line = match read_line(&mut reader) {
            Ok(Line::Text(l)) if l.is_empty() => continue,
            Ok(Line::Text(l)) => l,
            Ok(Line::TooLarge) => {
                let _ = writer.write_all(
                    protocol::err(
                        &serde_json::Value::Null,
                        code::TOO_LARGE,
                        "la petición es demasiado grande",
                    )
                    .as_bytes(),
                );
                return;
            }
            Ok(Line::Eof) | Err(_) => return,
        };
        let response = match protocol::parse_request(&line, token) {
            Ok(request) => {
                let client = Client {
                    pid,
                    name: request.client.clone(),
                };
                handler.handle(&client, request)
            }
            Err(response) => {
                if response.contains(code::UNAUTHORIZED) {
                    bad_tokens += 1;
                    if bad_tokens >= MAX_BAD_TOKENS {
                        let _ = writer.write_all(response.as_bytes());
                        return;
                    }
                }
                response
            }
        };
        if writer.write_all(response.as_bytes()).is_err() {
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};
    use std::io::BufReader;

    struct Echo;

    impl Handler for Echo {
        fn handle(&self, client: &Client, request: Request) -> String {
            protocol::ok(
                &request.id,
                json!({"method": request.method.name(), "client": client.name, "pid": client.pid}),
            )
        }
    }

    fn base(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rt-ctl-test-{name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&d);
        std::fs::create_dir_all(&d).unwrap();
        d
    }

    fn ask(stream: &mut UnixStream, reader: &mut impl BufRead, line: &str) -> Value {
        stream.write_all(format!("{line}\n").as_bytes()).unwrap();
        let mut out = String::new();
        reader.read_line(&mut out).unwrap();
        serde_json::from_str(out.trim()).unwrap()
    }

    #[test]
    fn the_socket_and_its_folder_are_private() {
        let b = base("perms");
        let s = ControlServer::start(&b, Arc::new(Echo)).unwrap();
        let mode = |p: &Path| std::fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(s.socket_path().parent().unwrap()), 0o700);
        assert_eq!(mode(s.socket_path()), 0o600);
        assert_eq!(s.token().len(), 32);
        assert!(s.token().chars().all(|c| c.is_ascii_hexdigit()));
        // Cada arranque tiene su token.
        let other = ControlServer::start(&b, Arc::new(Echo)).unwrap();
        assert_ne!(other.token(), s.token());
        drop(other);
        drop(s);
        assert!(
            !b.join(format!("rust-terminal-control-{}", std::process::id()))
                .exists(),
            "al soltarlo se borra"
        );
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn requests_with_the_token_are_served_and_others_refused() {
        let b = base("auth");
        let s = ControlServer::start(&b, Arc::new(Echo)).unwrap();
        let mut c = UnixStream::connect(s.socket_path()).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let tok = s.token().to_owned();
        let good =
            format!(r#"{{"v":0,"id":1,"token":"{tok}","client":"claude","method":"hello"}}"#);
        let v = ask(&mut c, &mut r, &good);
        assert_eq!(v["ok"], true, "{v}");
        assert_eq!(v["result"]["client"], "claude");
        // El pid del cliente es el de este proceso (el test es el cliente).
        assert_eq!(v["result"]["pid"], std::process::id());
        // Varias peticiones por la misma conexión.
        assert_eq!(ask(&mut c, &mut r, &good)["ok"], true);
        // Token malo.
        let bad = r#"{"v":0,"id":2,"token":"nope","method":"hello"}"#;
        let v = ask(&mut c, &mut r, bad);
        assert_eq!(
            (v["ok"].clone(), v["error"]["code"].clone()),
            (json!(false), json!("unauthorized"))
        );
        // Tras varios intentos con un token malo se cierra la conexión.
        for _ in 0..MAX_BAD_TOKENS {
            let _ = c.write_all(format!("{bad}\n").as_bytes());
        }
        let mut sink = String::new();
        let mut closed = false;
        for _ in 0..(MAX_BAD_TOKENS + 2) {
            sink.clear();
            if r.read_line(&mut sink).unwrap_or(0) == 0 {
                closed = true;
                break;
            }
        }
        assert!(closed, "la conexión debe cerrarse");
        drop(s);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn an_oversized_line_is_refused_and_the_connection_closed() {
        let b = base("big");
        let s = ControlServer::start(&b, Arc::new(Echo)).unwrap();
        let mut c = UnixStream::connect(s.socket_path()).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        let big = "x".repeat(MAX_LINE + 10);
        // El servidor puede cerrar antes de leerlo todo: no importa si la escritura falla.
        let _ = c.write_all(big.as_bytes());
        let _ = c.write_all(b"\n");
        let mut out = String::new();
        let n = r.read_line(&mut out).unwrap_or(0);
        if n > 0 {
            let v: Value = serde_json::from_str(out.trim()).unwrap();
            assert_eq!(v["error"]["code"], "too_large");
        }
        drop(s);
        let _ = std::fs::remove_dir_all(&b);
    }

    #[test]
    fn garbage_does_not_kill_the_server() {
        let b = base("junk");
        let s = ControlServer::start(&b, Arc::new(Echo)).unwrap();
        let mut c = UnixStream::connect(s.socket_path()).unwrap();
        let mut r = BufReader::new(c.try_clone().unwrap());
        assert_eq!(
            ask(&mut c, &mut r, "{{{{ no json")["error"]["code"],
            "bad_request"
        );
        // Líneas vacías se saltan y el servidor sigue respondiendo.
        let tok = s.token().to_owned();
        c.write_all(b"\n\n").unwrap();
        let good = format!(r#"{{"v":0,"id":9,"token":"{tok}","method":"hello"}}"#);
        assert_eq!(ask(&mut c, &mut r, &good)["id"], 9);
        // Y una conexión nueva funciona.
        let mut c2 = UnixStream::connect(s.socket_path()).unwrap();
        let mut r2 = BufReader::new(c2.try_clone().unwrap());
        assert_eq!(ask(&mut c2, &mut r2, &good)["ok"], true);
        drop(s);
        let _ = std::fs::remove_dir_all(&b);
    }
}
