//! Prueba de extremo a extremo con el `claude` real (ignorada por defecto:
//! usa la red y la cuenta del usuario).
//!
//! ```sh
//! cargo test -p rust-terminal e2e -- --ignored --test-threads=1
//! ```
//!
//! Lanza `claude -p` con los `--settings` que genera el propio código de
//! producción, le pide una orden que necesita permiso y contesta desde el lado
//! de la interfaz (`Broker::resolve`).

use std::fs;
use std::path::PathBuf;
use std::process::{Command, Stdio};
use std::sync::Arc;
use std::sync::mpsc;
use std::time::Duration;

use super::broker::{Broker, BrokerEvent, Decision};
use super::launch::write_agent_files;

struct Outcome {
    file_created: bool,
    command_seen: String,
    output: String,
}

fn run(decision: Decision, name: &str) -> Outcome {
    let base = std::env::temp_dir().join(format!("rt-e2e-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&base);
    let work = base.join("work");
    fs::create_dir_all(&work).unwrap();
    let target: PathBuf = work.join("e2e.txt");

    let (tx, rx) = mpsc::channel();
    let broker = Arc::new(
        Broker::start(Duration::from_secs(60), move |e| {
            let _ = tx.send(e);
        })
        .unwrap(),
    );
    let key = broker.register_agent().unwrap();
    let files = write_agent_files(&base.join("agents"), &broker, &key, None).unwrap();

    // Hilo de "la interfaz": contesta la primera petición de permiso.
    let ui = {
        let broker = broker.clone();
        std::thread::spawn(move || {
            while let Ok(event) = rx.recv_timeout(Duration::from_secs(90)) {
                if let BrokerEvent::Permission(p) = event {
                    let command = p.tool_input["command"].as_str().unwrap_or("").to_owned();
                    assert!(broker.resolve(p.id, decision));
                    return command;
                }
            }
            String::new()
        })
    };

    let out = Command::new("claude")
        .args(["-p", "--permission-mode", "default", "--settings"])
        .arg(&files.settings)
        .arg("--")
        .arg(format!(
            "Usa la herramienta Bash para ejecutar exactamente: touch {}",
            target.display()
        ))
        .current_dir(&work)
        .stdin(Stdio::null())
        .output()
        .expect("no se pudo ejecutar claude");

    let command_seen = ui.join().unwrap();
    let outcome = Outcome {
        file_created: target.exists(),
        command_seen,
        output: String::from_utf8_lossy(&out.stdout).into_owned(),
    };
    let _ = fs::remove_dir_all(&base);
    outcome
}

#[test]
#[ignore = "usa el claude real y la red"]
fn permitir_desde_la_interfaz_ejecuta_la_orden() {
    let o = run(Decision::Allow, "allow");
    assert!(
        o.command_seen.starts_with("touch "),
        "orden vista: {:?}",
        o.command_seen
    );
    assert!(
        o.file_created,
        "el archivo debía crearse. Salida: {}",
        o.output
    );
}

#[test]
#[ignore = "usa el claude real y la red"]
fn denegar_desde_la_interfaz_no_ejecuta_la_orden() {
    let o = run(
        Decision::Deny {
            message: Some("El usuario ha denegado esta orden.".into()),
        },
        "deny",
    );
    assert!(
        o.command_seen.starts_with("touch "),
        "orden vista: {:?}",
        o.command_seen
    );
    assert!(
        !o.file_created,
        "el archivo NO debía crearse. Salida: {}",
        o.output
    );
}
