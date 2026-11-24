//! `rt`: habla con la API de control de rust-terminal (experimental, protocolo
//! `v0`) por el socket Unix de la app. Pensado para que lo use un agente
//! desde una terminal de la app, que le pasa `RT_CONTROL_SOCKET` y
//! `RT_CONTROL_TOKEN`.

use std::io::{BufRead, BufReader, Read, Write};
use std::os::unix::net::UnixStream;
use std::time::Duration;

use serde_json::{Value, json};

const USAGE: &str = "\
rt — controla las terminales de rust-terminal (experimental)

USO
  rt [--json] <orden> [argumentos]

LEER (sin permiso)
  rt sessions                          lista las sesiones (id, ventana, pestaña, título)
  rt read <sesión> [--lines N]         la pantalla (o las últimas N líneas con historial)
  rt blocks <sesión> [--limit N]       los últimos comandos con su código de salida
  rt output <sesión> <bloque>          la salida de un comando terminado

ESCRIBIR (la app pide permiso al usuario)
  rt send <sesión> [--enter] <texto…>  teclea texto (`-` lo lee de la entrada estándar)
  rt keys <sesión> <tecla>…            pulsa teclas: ctrl+c enter esc up alt+x f5 …
  rt open [--cwd CARPETA]              abre una sesión nueva y escribe su id
  rt close <sesión>                    cierra una sesión

ENTORNO
  RT_CONTROL_SOCKET, RT_CONTROL_TOKEN  los pone la app en sus shells
  RT_CLIENT                            nombre del agente (por defecto, el proceso padre)

CÓDIGOS DE SALIDA  0 bien · 1 error de la app · 2 uso · 3 sin conexión
";

/// Lo que pide el usuario, ya interpretado.
#[derive(Debug, PartialEq)]
struct Command {
    method: &'static str,
    params: Value,
    json: bool,
    /// Cómo mostrar el resultado en texto.
    view: View,
}

#[derive(Debug, PartialEq, Clone, Copy)]
enum View {
    Raw,
    Sessions,
    Text,
    Blocks,
    SessionId,
    Done,
}

fn usage_error(msg: &str) -> String {
    format!("rt: {msg}\n\n{USAGE}")
}

/// Saca `--nombre valor` de la lista de argumentos.
fn take_option(args: &mut Vec<String>, name: &str) -> Result<Option<String>, String> {
    let Some(i) = args.iter().position(|a| a == name) else {
        return Ok(None);
    };
    if i + 1 >= args.len() {
        return Err(format!("`{name}` necesita un valor"));
    }
    let value = args.remove(i + 1);
    args.remove(i);
    Ok(Some(value))
}

fn take_flag(args: &mut Vec<String>, name: &str) -> bool {
    match args.iter().position(|a| a == name) {
        Some(i) => {
            args.remove(i);
            true
        }
        None => false,
    }
}

fn number(value: Option<String>, name: &str) -> Result<Option<u64>, String> {
    value
        .map(|v| {
            v.parse::<u64>()
                .map_err(|_| format!("`{name}` debe ser un número"))
        })
        .transpose()
}

fn parse_args(mut args: Vec<String>, stdin: &mut dyn FnMut() -> String) -> Result<Command, String> {
    let json = take_flag(&mut args, "--json");
    if args.is_empty() {
        return Err(usage_error("falta la orden"));
    }
    let name = args.remove(0);
    let one_session = |args: &[String]| -> Result<String, String> {
        match args {
            [s] => Ok(s.clone()),
            _ => Err(usage_error(&format!(
                "`rt {name}` necesita una sesión (mira `rt sessions`)"
            ))),
        }
    };
    let (method, params, view) = match name.as_str() {
        "sessions" => ("sessions.list", json!({}), View::Sessions),
        "read" => {
            let lines = number(take_option(&mut args, "--lines")?, "--lines")?;
            let mut p = json!({ "session": one_session(&args)? });
            if let Some(n) = lines {
                p["lines"] = json!(n);
            }
            ("screen.read", p, View::Text)
        }
        "blocks" => {
            let limit = number(take_option(&mut args, "--limit")?, "--limit")?;
            let mut p = json!({ "session": one_session(&args)? });
            if let Some(n) = limit {
                p["limit"] = json!(n);
            }
            ("blocks.list", p, View::Blocks)
        }
        "output" => match args.as_slice() {
            [s, b] => {
                let block = b
                    .parse::<u64>()
                    .map_err(|_| usage_error("el bloque debe ser un número"))?;
                (
                    "blocks.output",
                    json!({"session": s, "block": block}),
                    View::Text,
                )
            }
            _ => return Err(usage_error("`rt output <sesión> <bloque>`")),
        },
        "send" => {
            let enter = take_flag(&mut args, "--enter");
            if args.len() < 2 {
                return Err(usage_error("`rt send <sesión> [--enter] <texto…>`"));
            }
            let session = args.remove(0);
            let text = if args == ["-"] {
                stdin()
            } else {
                args.join(" ")
            };
            (
                "input.send_text",
                json!({"session": session, "text": text, "enter": enter}),
                View::Done,
            )
        }
        "keys" => {
            if args.len() < 2 {
                return Err(usage_error("`rt keys <sesión> <tecla>…`"));
            }
            let session = args.remove(0);
            (
                "input.send_keys",
                json!({"session": session, "keys": args}),
                View::Done,
            )
        }
        "open" => {
            let cwd = take_option(&mut args, "--cwd")?;
            if !args.is_empty() {
                return Err(usage_error("`rt open [--cwd CARPETA]`"));
            }
            let p = cwd.map_or_else(|| json!({}), |c| json!({ "cwd": c }));
            ("sessions.open", p, View::SessionId)
        }
        "close" => (
            "sessions.close",
            json!({ "session": one_session(&args)? }),
            View::Done,
        ),
        "hello" | "version" => ("hello", json!({}), View::Raw),
        "help" | "--help" | "-h" => return Err(USAGE.to_owned()),
        other => return Err(usage_error(&format!("orden desconocida `{other}`"))),
    };
    Ok(Command {
        method,
        params,
        json,
        view,
    })
}

/// El nombre del agente: `RT_CLIENT` o el proceso padre (sin la ruta).
fn client_name() -> String {
    if let Ok(n) = std::env::var("RT_CLIENT")
        && !n.trim().is_empty()
    {
        return n;
    }
    let ppid = std::os::unix::process::parent_id();
    std::process::Command::new("/bin/ps")
        .args(["-o", "comm=", "-p", &ppid.to_string()])
        .output()
        .ok()
        .and_then(|o| String::from_utf8(o.stdout).ok())
        .map(|s| s.trim().rsplit('/').next().unwrap_or("").to_owned())
        .filter(|s| !s.is_empty())
        .unwrap_or_else(|| "rt".to_owned())
}

fn request_line(cmd: &Command, token: &str, client: &str) -> String {
    let mut line = json!({
        "v": 0, "id": 1, "token": token, "client": client,
        "method": cmd.method, "params": cmd.params,
    })
    .to_string();
    line.push('\n');
    line
}

/// Texto legible del resultado.
fn render(view: View, result: &Value) -> String {
    let s = |v: &Value| v.as_str().unwrap_or("").to_owned();
    match view {
        View::Raw => serde_json::to_string_pretty(result).unwrap_or_default(),
        View::Done => String::new(),
        View::SessionId => s(&result["session"]),
        View::Text => {
            let mut t = s(&result["text"]);
            if result["truncated"] == json!(true) {
                t.push_str("\n[…recortado: solo el final]");
            }
            t
        }
        View::Sessions => {
            let rows: Vec<String> = result["sessions"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|x| {
                            let mut flags = Vec::new();
                            if x["focused"] == json!(true) {
                                flags.push("foco");
                            }
                            if x["agent"] == json!(true) {
                                flags.push("agente");
                            }
                            if x["busy"] == json!(true) {
                                flags.push("ocupada");
                            }
                            let host = x["ssh_host"]
                                .as_str()
                                .map(|h| format!(" ssh:{h}"))
                                .unwrap_or_default();
                            format!(
                                "{}\t{}\t{}\t{}\t{}{}{}",
                                s(&x["id"]),
                                s(&x["window"]),
                                x["tab"],
                                s(&x["title"]),
                                x["cwd"].as_str().unwrap_or("-"),
                                host,
                                if flags.is_empty() {
                                    String::new()
                                } else {
                                    format!("  [{}]", flags.join(", "))
                                },
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            rows.join("\n")
        }
        View::Blocks => {
            let rows: Vec<String> = result["blocks"]
                .as_array()
                .map(|a| {
                    a.iter()
                        .map(|b| {
                            let state = if b["running"] == json!(true) {
                                "…".to_owned()
                            } else {
                                match b["exit_code"].as_i64() {
                                    Some(0) => "ok".to_owned(),
                                    Some(c) => format!("exit {c}"),
                                    None => "?".to_owned(),
                                }
                            };
                            format!(
                                "{}\t{}\t{}",
                                b["id"],
                                state,
                                b["command"].as_str().unwrap_or("")
                            )
                        })
                        .collect()
                })
                .unwrap_or_default();
            rows.join("\n")
        }
    }
}

fn run() -> Result<i32, String> {
    let args: Vec<String> = std::env::args().skip(1).collect();
    let mut read_stdin = || {
        let mut s = String::new();
        let _ = std::io::stdin().read_to_string(&mut s);
        s.trim_end_matches('\n').to_owned()
    };
    let cmd = match parse_args(args, &mut read_stdin) {
        Ok(c) => c,
        Err(msg) => {
            eprintln!("{}", msg.trim_end());
            return Ok(2);
        }
    };
    let (Ok(socket), Ok(token)) = (
        std::env::var("RT_CONTROL_SOCKET"),
        std::env::var("RT_CONTROL_TOKEN"),
    ) else {
        eprintln!(
            "rt: no hay conexión con la app. Hay que ejecutarlo dentro de una terminal de rust-terminal \
             (con `control.enabled = true`): ahí están RT_CONTROL_SOCKET y RT_CONTROL_TOKEN."
        );
        return Ok(3);
    };
    let mut stream = UnixStream::connect(&socket)
        .map_err(|e| format!("no se pudo conectar con {socket}: {e} (¿sigue abierta la app?)"))?;
    // Escribir puede esperar a que el usuario conteste un diálogo.
    let _ = stream.set_read_timeout(Some(Duration::from_secs(140)));
    stream
        .write_all(request_line(&cmd, &token, &client_name()).as_bytes())
        .map_err(|e| e.to_string())?;
    let mut line = String::new();
    BufReader::new(stream)
        .read_line(&mut line)
        .map_err(|e| format!("sin respuesta: {e}"))?;
    let response: Value =
        serde_json::from_str(line.trim()).map_err(|e| format!("respuesta no válida: {e}"))?;
    if response["ok"] != json!(true) {
        eprintln!(
            "rt: {}: {}",
            response["error"]["code"].as_str().unwrap_or("error"),
            response["error"]["message"].as_str().unwrap_or("")
        );
        return Ok(1);
    }
    let result = &response["result"];
    let out = if cmd.json {
        serde_json::to_string_pretty(result).unwrap_or_default()
    } else {
        render(cmd.view, result)
    };
    if !out.is_empty() {
        println!("{out}");
    }
    Ok(0)
}

fn main() {
    match run() {
        Ok(code) => std::process::exit(code),
        Err(e) => {
            eprintln!("rt: {e}");
            std::process::exit(3);
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn parse(args: &[&str]) -> Result<Command, String> {
        parse_args(args.iter().map(|s| (*s).to_owned()).collect(), &mut || {
            "desde stdin".to_owned()
        })
    }

    #[test]
    fn commands_become_requests() {
        let c = parse(&["sessions"]).unwrap();
        assert_eq!((c.method, c.json), ("sessions.list", false));
        let c = parse(&["--json", "read", "1-2", "--lines", "50"]).unwrap();
        assert_eq!((c.method, c.json), ("screen.read", true));
        assert_eq!(c.params, json!({"session": "1-2", "lines": 50}));
        let c = parse(&["blocks", "1-2", "--limit", "5"]).unwrap();
        assert_eq!(c.params, json!({"session": "1-2", "limit": 5}));
        let c = parse(&["output", "1-2", "7"]).unwrap();
        assert_eq!(c.params, json!({"session": "1-2", "block": 7}));
        let c = parse(&["send", "1-2", "--enter", "ls", "-la"]).unwrap();
        assert_eq!(
            c.params,
            json!({"session": "1-2", "text": "ls -la", "enter": true})
        );
        let c = parse(&["send", "1-2", "-"]).unwrap();
        assert_eq!(c.params["text"], "desde stdin");
        let c = parse(&["keys", "1-2", "ctrl+c", "enter"]).unwrap();
        assert_eq!(
            c.params,
            json!({"session": "1-2", "keys": ["ctrl+c", "enter"]})
        );
        let c = parse(&["open", "--cwd", "/tmp"]).unwrap();
        assert_eq!(
            (c.method, c.params),
            ("sessions.open", json!({"cwd": "/tmp"}))
        );
        assert_eq!(parse(&["close", "1-2"]).unwrap().method, "sessions.close");
    }

    #[test]
    fn bad_usage_is_explained() {
        for bad in [
            &[][..],
            &["nada"],
            &["read"],
            &["read", "1-2", "3-4"],
            &["read", "1-2", "--lines"],
            &["read", "1-2", "--lines", "muchas"],
            &["output", "1-2"],
            &["output", "1-2", "x"],
            &["send", "1-2"],
            &["keys", "1-2"],
            &["open", "algo"],
        ] {
            assert!(parse(bad).is_err(), "{bad:?}");
        }
    }

    #[test]
    fn the_request_carries_the_token_and_the_client() {
        let c = parse(&["hello"]).unwrap();
        let line = request_line(&c, "tok", "claude");
        assert!(line.ends_with('\n'));
        let v: Value = serde_json::from_str(line.trim()).unwrap();
        assert_eq!(
            (
                v["v"].clone(),
                v["token"].clone(),
                v["client"].clone(),
                v["method"].clone()
            ),
            (json!(0), json!("tok"), json!("claude"), json!("hello"))
        );
    }

    #[test]
    fn results_are_rendered_for_humans() {
        let sessions = json!({"sessions": [
            {"id": "1-1", "window": "main", "tab": 0, "title": "zsh", "cwd": "/p", "focused": true, "agent": false, "busy": false, "ssh_host": null},
            {"id": "1-2", "window": "main", "tab": 1, "title": "claude", "cwd": "/q", "focused": false, "agent": true, "busy": true, "ssh_host": "prod-db"},
        ]});
        let out = render(View::Sessions, &sessions);
        assert!(out.contains("1-1\tmain\t0\tzsh\t/p  [foco]"), "{out}");
        assert!(out.contains("ssh:prod-db  [agente, ocupada]"), "{out}");
        let blocks = json!({"blocks": [
            {"id": 3, "command": "ls", "running": false, "exit_code": 0},
            {"id": 4, "command": "false", "running": false, "exit_code": 1},
            {"id": 5, "command": "sleep 9", "running": true, "exit_code": null},
        ]});
        assert_eq!(
            render(View::Blocks, &blocks),
            "3\tok\tls\n4\texit 1\tfalse\n5\t…\tsleep 9"
        );
        let t = render(View::Text, &json!({"text": "hola", "truncated": true}));
        assert!(t.starts_with("hola") && t.contains("recortado"));
        assert_eq!(render(View::SessionId, &json!({"session": "1-9"})), "1-9");
        assert_eq!(render(View::Done, &json!({"sent": 3})), "");
    }
}
