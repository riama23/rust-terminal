//! Lanzar un agente: del formulario (A4) a la línea de `claude` y sus
//! `--settings`.
//!
//! Principio "no magic": [`display_command`] devuelve el texto exacto que se
//! muestra en el recuadro *Runs exactly* y es el mismo que se ejecuta.

use std::fs;
use std::io;
use std::os::unix::fs::{OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use super::broker::{AgentKey, Broker};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum StartMode {
    /// Empieza proponiendo un plan.
    Plan,
    /// Pregunta antes de cada acción.
    Ask,
    AcceptEdits,
}

impl StartMode {
    fn flag(self) -> &'static str {
        match self {
            StartMode::Plan => "plan",
            StartMode::Ask => "default",
            StartMode::AcceptEdits => "acceptEdits",
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Isolation {
    /// `-w <nombre>`: worktree en `<repo>/.claude/worktrees/<nombre>`.
    NewWorktree(String),
    CurrentFolder,
}

#[derive(Debug, Clone)]
pub struct LaunchSpec {
    pub task: String,
    pub isolation: Isolation,
    pub mode: StartMode,
    pub model: Option<String>,
    pub title: Option<String>,
}

#[derive(Debug, PartialEq, Eq)]
pub enum SpecError {
    EmptyTask,
    BadWorktreeName,
    BadModel,
}

impl std::fmt::Display for SpecError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            SpecError::EmptyTask => "La tarea está vacía.",
            SpecError::BadWorktreeName => {
                "El nombre del worktree solo admite letras, números, «.», «_» y «-», y no puede empezar por «-» ni contener «..»."
            }
            SpecError::BadModel => "El modelo solo admite letras, números, «.», «_», «-» y «[]».",
        })
    }
}

fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && !name.starts_with(['-', '.'])
        && !name.contains("..")
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-'))
}

fn valid_model(model: &str) -> bool {
    !model.is_empty()
        && !model.starts_with('-')
        && model
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '.' | '_' | '-' | '[' | ']'))
}

impl LaunchSpec {
    pub fn validate(&self) -> Result<(), SpecError> {
        if self.task.trim().is_empty() {
            return Err(SpecError::EmptyTask);
        }
        if let Isolation::NewWorktree(name) = &self.isolation
            && !valid_name(name)
        {
            return Err(SpecError::BadWorktreeName);
        }
        if let Some(model) = &self.model
            && !valid_model(model)
        {
            return Err(SpecError::BadModel);
        }
        Ok(())
    }
}

/// Argumentos de `claude` (sin el nombre del programa). La tarea va tras `--`
/// para que un texto que empiece por `-` no se lea como una opción.
pub fn command_args(spec: &LaunchSpec, settings_path: &Path) -> Result<Vec<String>, SpecError> {
    spec.validate()?;
    let mut args = vec![
        "--settings".to_owned(),
        settings_path.to_string_lossy().into_owned(),
    ];
    if let Isolation::NewWorktree(name) = &spec.isolation {
        args.extend(["-w".to_owned(), name.clone()]);
    }
    args.extend(["--permission-mode".to_owned(), spec.mode.flag().to_owned()]);
    if let Some(model) = &spec.model {
        args.extend(["--model".to_owned(), model.clone()]);
    }
    if let Some(title) = spec.title.as_ref().filter(|t| !t.trim().is_empty()) {
        args.extend(["-n".to_owned(), title.clone()]);
    }
    args.extend(["--".to_owned(), spec.task.clone()]);
    Ok(args)
}

/// Cita un argumento para `sh` con comillas simples.
pub fn sh_quote(s: &str) -> String {
    if !s.is_empty()
        && s.chars().all(|c| {
            c.is_ascii_alphanumeric() || matches!(c, '-' | '_' | '.' | '/' | ':' | '=' | '@' | '+')
        })
    {
        return s.to_owned();
    }
    format!("'{}'", s.replace('\'', r"'\''"))
}

/// El comando tal como se enseña al usuario: `claude …`.
pub fn display_command(args: &[String]) -> String {
    std::iter::once("claude".to_owned())
        .chain(args.iter().map(|a| sh_quote(a)))
        .collect::<Vec<_>>()
        .join(" ")
}

/// Eventos que se reciben como hook HTTP. `PermissionRequest` y los de
/// herramientas solo interesan para `Bash` y demás: sin `matcher`, todos.
const HTTP_EVENTS: &[&str] = &[
    "PermissionRequest",
    "PreToolUse",
    "PostToolUse",
    "Notification",
    "Stop",
    "StopFailure",
    "SessionEnd",
    "TaskCreated",
    "TaskCompleted",
    "WorktreeCreate",
    "WorktreeRemove",
];

/// `SessionStart` no llega por HTTP (spike de hooks): va como `command` con
/// `curl`, sin imprimir nada (la salida de este hook se añadiría al contexto).
fn session_start_command(url: &str) -> String {
    format!(
        "curl -s -m 5 -o /dev/null -X POST -H 'Content-Type: application/json' --data-binary @- {}",
        sh_quote(url)
    )
}

/// El `settings.json` de un agente: hooks hacia el broker y la status line.
pub fn settings_json(broker: &Broker, key: &str, statusline_script: &Path) -> Value {
    let mut hooks = serde_json::Map::new();
    for event in HTTP_EVENTS {
        hooks.insert(
            (*event).to_owned(),
            json!([{"hooks": [{"type": "http", "url": broker.hook_url(key, event)}]}]),
        );
    }
    hooks.insert(
        "SessionStart".to_owned(),
        json!([{"hooks": [{"type": "command",
                           "command": session_start_command(&broker.hook_url(key, "SessionStart"))}]}]),
    );
    json!({
        "hooks": hooks,
        "statusLine": {"type": "command", "command": statusline_script.to_string_lossy()},
    })
}

/// Contenido del script de status line. Reenvía el JSON al broker (sin
/// esperar más de 2 s) y, si el usuario tenía una status line propia, la
/// ejecuta con la misma entrada y devuelve su salida sin cambios.
pub fn statusline_script(url: &str, user_command: Option<&str>) -> String {
    let mut s = String::from("#!/bin/sh\ninput=$(cat)\n");
    s.push_str(&format!(
        "printf '%s' \"$input\" | curl -s -m 2 -o /dev/null -X POST -H 'Content-Type: application/json' --data-binary @- {} >/dev/null 2>&1 &\n",
        sh_quote(url)
    ));
    if let Some(cmd) = user_command {
        s.push_str(&format!(
            "printf '%s' \"$input\" | sh -c {}\n",
            sh_quote(cmd)
        ));
    }
    s.push_str("wait\n");
    s
}

/// La `statusLine` del usuario que `--settings` taparía: la del proyecto
/// (`.local` sobre compartida) y, si no hay, la de `~/.claude/settings.json`.
pub fn find_user_statusline(home: &Path, project_dir: &Path) -> Option<String> {
    let candidates = [
        project_dir.join(".claude/settings.local.json"),
        project_dir.join(".claude/settings.json"),
        home.join(".claude/settings.json"),
    ];
    candidates.iter().find_map(|path| {
        let v: Value = serde_json::from_str(&fs::read_to_string(path).ok()?).ok()?;
        let sl = v.get("statusLine")?;
        (sl.get("type").and_then(Value::as_str) == Some("command"))
            .then(|| sl.get("command")?.as_str().map(str::to_owned))?
    })
}

/// Lo que hay en disco para un agente.
#[derive(Debug)]
pub struct AgentFiles {
    pub dir: PathBuf,
    pub settings: PathBuf,
}

/// Escribe el script de status line y el `settings.json` del agente en `dir`
/// (privado: contienen el token del broker).
pub fn write_agent_files(
    dir: &Path,
    broker: &Broker,
    key: &AgentKey,
    user_statusline: Option<&str>,
) -> io::Result<AgentFiles> {
    fs::create_dir_all(dir)?;
    fs::set_permissions(dir, fs::Permissions::from_mode(0o700))?;
    let script = dir.join(format!("statusline-{key}.sh"));
    write_private(
        &script,
        &statusline_script(&broker.statusline_url(key), user_statusline),
        0o700,
    )?;
    let settings = dir.join(format!("settings-{key}.json"));
    let json = serde_json::to_string_pretty(&settings_json(broker, key, &script))
        .map_err(io::Error::other)?;
    write_private(&settings, &json, 0o600)?;
    Ok(AgentFiles {
        dir: dir.to_owned(),
        settings,
    })
}

fn write_private(path: &Path, content: &str, mode: u32) -> io::Result<()> {
    use std::io::Write;
    let _ = fs::remove_file(path);
    let mut f = fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(mode)
        .open(path)?;
    f.write_all(content.as_bytes())
}

/// Borra los archivos de un agente al darlo de baja.
pub fn remove_agent_files(dir: &Path, key: &str) {
    let _ = fs::remove_file(dir.join(format!("statusline-{key}.sh")));
    let _ = fs::remove_file(dir.join(format!("settings-{key}.json")));
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::time::Duration;

    fn spec() -> LaunchSpec {
        LaunchSpec {
            task: "Arregla el escape de comillas".into(),
            isolation: Isolation::NewWorktree("fix-escaped-quotes".into()),
            mode: StartMode::Plan,
            model: Some("opus".into()),
            title: Some("Escape de comillas".into()),
        }
    }

    fn tmpdir(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rt-launch-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn la_tabla_a4_se_traduce_a_flags() {
        let args = command_args(&spec(), Path::new("/tmp/s.json")).unwrap();
        assert_eq!(
            args,
            [
                "--settings",
                "/tmp/s.json",
                "-w",
                "fix-escaped-quotes",
                "--permission-mode",
                "plan",
                "--model",
                "opus",
                "-n",
                "Escape de comillas",
                "--",
                "Arregla el escape de comillas"
            ]
        );
    }

    #[test]
    fn modos_de_inicio() {
        for (mode, flag) in [
            (StartMode::Plan, "plan"),
            (StartMode::Ask, "default"),
            (StartMode::AcceptEdits, "acceptEdits"),
        ] {
            let s = LaunchSpec { mode, ..spec() };
            let args = command_args(&s, Path::new("/s")).unwrap();
            let i = args.iter().position(|a| a == "--permission-mode").unwrap();
            assert_eq!(args[i + 1], flag);
        }
    }

    #[test]
    fn carpeta_actual_no_lleva_worktree_y_lo_opcional_se_omite() {
        let s = LaunchSpec {
            isolation: Isolation::CurrentFolder,
            model: None,
            title: Some("  ".into()),
            ..spec()
        };
        let args = command_args(&s, Path::new("/s")).unwrap();
        for flag in ["-w", "--model", "-n"] {
            assert!(!args.iter().any(|a| a == flag), "{flag} sobra");
        }
    }

    #[test]
    fn una_tarea_que_empieza_por_guion_va_tras_el_separador() {
        let s = LaunchSpec {
            task: "--dangerously-skip-permissions".into(),
            ..spec()
        };
        let args = command_args(&s, Path::new("/s")).unwrap();
        let sep = args.iter().position(|a| a == "--").unwrap();
        assert_eq!(args[sep + 1], "--dangerously-skip-permissions");
        assert_eq!(sep + 2, args.len(), "nada después de la tarea");
    }

    #[test]
    fn validacion() {
        let bad = |s: LaunchSpec| s.validate().unwrap_err();
        assert_eq!(
            bad(LaunchSpec {
                task: " \n".into(),
                ..spec()
            }),
            SpecError::EmptyTask
        );
        for name in [
            "",
            "-x",
            ".oculto",
            "a..b",
            "con espacio",
            "a/b",
            "a;b",
            "$(x)",
        ] {
            let s = LaunchSpec {
                isolation: Isolation::NewWorktree(name.into()),
                ..spec()
            };
            assert_eq!(bad(s), SpecError::BadWorktreeName, "{name:?}");
        }
        for model in ["", "--x", "a b", "a;b"] {
            let s = LaunchSpec {
                model: Some(model.into()),
                ..spec()
            };
            assert_eq!(bad(s), SpecError::BadModel, "{model:?}");
        }
        assert!(
            LaunchSpec {
                model: Some("opus[1m]".into()),
                ..spec()
            }
            .validate()
            .is_ok()
        );
    }

    #[test]
    fn el_comando_mostrado_cita_lo_que_hace_falta() {
        let args = command_args(&spec(), Path::new("/tmp/s.json")).unwrap();
        assert_eq!(
            display_command(&args),
            "claude --settings /tmp/s.json -w fix-escaped-quotes --permission-mode plan --model opus -n 'Escape de comillas' -- 'Arregla el escape de comillas'"
        );
        assert_eq!(sh_quote("it's"), r"'it'\''s'");
        assert_eq!(sh_quote(""), "''");
        assert_eq!(sh_quote("$(rm -rf ~)"), "'$(rm -rf ~)'");
    }

    fn broker() -> Broker {
        Broker::start(Duration::from_secs(1), |_| {}).unwrap()
    }

    #[test]
    fn el_settings_apunta_el_broker_y_pone_session_start_como_command() {
        let b = broker();
        let key = b.register_agent().unwrap();
        let v = settings_json(&b, &key, Path::new("/tmp/sl.sh"));
        let perm = &v["hooks"]["PermissionRequest"][0]["hooks"][0];
        assert_eq!(perm["type"], "http");
        assert_eq!(perm["url"], b.hook_url(&key, "PermissionRequest"));
        let start = &v["hooks"]["SessionStart"][0]["hooks"][0];
        assert_eq!(start["type"], "command");
        let cmd = start["command"].as_str().unwrap();
        assert!(cmd.contains("-o /dev/null"), "no debe imprimir nada: {cmd}");
        assert!(cmd.contains(&b.hook_url(&key, "SessionStart")));
        assert_eq!(v["statusLine"]["command"], "/tmp/sl.sh");
    }

    #[test]
    fn el_script_de_status_line_reenvia_y_conserva_la_del_usuario() {
        let con = statusline_script(
            "http://127.0.0.1:1/statusline/t/k",
            Some("~/bin/mi linea 'x'"),
        );
        assert!(con.contains("--data-binary @- http://127.0.0.1:1/statusline/t/k"));
        assert!(con.contains(r"sh -c '~/bin/mi linea '\''x'\'''"));
        assert!(con.trim_end().ends_with("wait"));
        let sin = statusline_script("http://x/y", None);
        assert!(!sin.contains("sh -c"));
    }

    #[test]
    fn el_script_de_status_line_funciona_de_verdad() {
        // Sin broker alcanzable: la salida del usuario debe pasar intacta.
        let dir = tmpdir("sl");
        let script = dir.join("sl.sh");
        fs::write(
            &script,
            statusline_script("http://127.0.0.1:9/x", Some("cat | tr a-z A-Z")),
        )
        .unwrap();
        let out = std::process::Command::new("sh")
            .arg(&script)
            .stdin(std::process::Stdio::piped())
            .stdout(std::process::Stdio::piped())
            .spawn()
            .and_then(|mut c| {
                use std::io::Write;
                c.stdin
                    .take()
                    .unwrap()
                    .write_all(b"hola {\"a\":1}")
                    .unwrap();
                c.wait_with_output()
            })
            .unwrap();
        assert_eq!(String::from_utf8_lossy(&out.stdout), "HOLA {\"A\":1}");
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn busca_la_status_line_del_usuario_con_la_precedencia_de_claude() {
        let home = tmpdir("home");
        let proj = tmpdir("proj");
        fs::create_dir_all(home.join(".claude")).unwrap();
        fs::create_dir_all(proj.join(".claude")).unwrap();
        assert_eq!(find_user_statusline(&home, &proj), None);
        let put = |p: PathBuf, cmd: &str| {
            fs::write(
                p,
                json!({"statusLine": {"type": "command", "command": cmd}}).to_string(),
            )
            .unwrap()
        };
        put(home.join(".claude/settings.json"), "usuario");
        assert_eq!(
            find_user_statusline(&home, &proj).as_deref(),
            Some("usuario")
        );
        put(proj.join(".claude/settings.json"), "proyecto");
        assert_eq!(
            find_user_statusline(&home, &proj).as_deref(),
            Some("proyecto")
        );
        put(proj.join(".claude/settings.local.json"), "local");
        assert_eq!(find_user_statusline(&home, &proj).as_deref(), Some("local"));
        // Un JSON roto o una statusLine que no es de tipo command no cuentan.
        fs::write(proj.join(".claude/settings.local.json"), "{roto").unwrap();
        assert_eq!(
            find_user_statusline(&home, &proj).as_deref(),
            Some("proyecto")
        );
        fs::write(
            proj.join(".claude/settings.json"),
            json!({"statusLine": {"type": "static", "text": "x"}}).to_string(),
        )
        .unwrap();
        assert_eq!(
            find_user_statusline(&home, &proj).as_deref(),
            Some("usuario")
        );
        fs::remove_dir_all(home).unwrap();
        fs::remove_dir_all(proj).unwrap();
    }

    #[test]
    fn los_archivos_del_agente_son_privados_y_se_borran() {
        let dir = tmpdir("files");
        let b = broker();
        let key = b.register_agent().unwrap();
        let files = write_agent_files(&dir, &b, &key, None).unwrap();
        let mode = |p: &Path| fs::metadata(p).unwrap().permissions().mode() & 0o777;
        assert_eq!(mode(&dir), 0o700);
        assert_eq!(mode(&files.settings), 0o600);
        assert_eq!(mode(&dir.join(format!("statusline-{key}.sh"))), 0o700);
        let parsed: Value =
            serde_json::from_str(&fs::read_to_string(&files.settings).unwrap()).unwrap();
        assert!(parsed["hooks"]["Stop"].is_array());
        // Reescribir no falla aunque ya existan.
        write_agent_files(&dir, &b, &key, None).unwrap();
        remove_agent_files(&dir, &key);
        assert!(!files.settings.exists());
        fs::remove_dir_all(dir).unwrap();
    }
}
