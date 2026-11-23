//! *Siempre permitir*: la regla que se propone y su escritura en
//! `.claude/settings.local.json` (personal, no el `settings.json` del equipo).
//!
//! Solo se ofrece una regla **exacta**: nunca más amplia que lo que el usuario
//! está viendo. Si la orden contiene algo que Claude Code interpretaría como
//! comodín o que no cabe en una línea, no hay regla y el botón se desactiva.

use std::fs;
use std::io;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

/// La regla exacta para esta petición, o `None` si no se puede expresar sin
/// ampliar el permiso.
pub fn rule_for(tool_name: &str, tool_input: &Value) -> Option<String> {
    if tool_name.is_empty() || tool_name.contains(['(', ')', '*', '\n']) {
        return None;
    }
    if tool_name != "Bash" {
        // Herramientas sin argumento relevante (o MCP): la regla es el nombre.
        // Las que tocan archivos no se ofrecen: un nombre suelto autorizaría
        // cualquier ruta.
        let touches_files = matches!(
            tool_name,
            "Edit" | "Write" | "MultiEdit" | "NotebookEdit" | "Read"
        );
        return (!touches_files).then(|| tool_name.to_owned());
    }
    let command = tool_input.get("command")?.as_str()?.trim();
    if command.is_empty() || command.contains(['*', '\n', '\r']) || command.len() > 500 {
        return None;
    }
    Some(format!("Bash({command})"))
}

/// Dónde se guarda: `<proyecto>/.claude/settings.local.json`.
pub fn local_settings_path(project_dir: &Path) -> PathBuf {
    project_dir.join(".claude").join("settings.local.json")
}

#[derive(Debug, PartialEq, Eq)]
pub enum Added {
    Added,
    AlreadyPresent,
}

#[derive(Debug)]
pub enum RuleError {
    /// El archivo existe pero no es un JSON de objeto: no se toca.
    Unreadable(String),
    Io(io::Error),
}

impl std::fmt::Display for RuleError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        match self {
            RuleError::Unreadable(why) => write!(f, "{why}"),
            RuleError::Io(e) => write!(f, "{e}"),
        }
    }
}

impl From<io::Error> for RuleError {
    fn from(e: io::Error) -> Self {
        RuleError::Io(e)
    }
}

/// Añade `rule` a `permissions.allow` conservando el resto del archivo. Si el
/// archivo existe y no se puede leer como un objeto JSON, no lo sobrescribe.
pub fn add_allow_rule(path: &Path, rule: &str) -> Result<Added, RuleError> {
    let mut root = match fs::read_to_string(path) {
        Ok(text) if text.trim().is_empty() => json!({}),
        Ok(text) => serde_json::from_str::<Value>(&text)
            .ok()
            .filter(Value::is_object)
            .ok_or_else(|| {
                RuleError::Unreadable(format!(
                    "{} no es un JSON válido; no se ha modificado.",
                    path.display()
                ))
            })?,
        Err(e) if e.kind() == io::ErrorKind::NotFound => json!({}),
        Err(e) => return Err(e.into()),
    };

    let permissions = root
        .as_object_mut()
        .expect("comprobado arriba")
        .entry("permissions")
        .or_insert_with(|| json!({}));
    let Some(permissions) = permissions.as_object_mut() else {
        return Err(RuleError::Unreadable(
            "`permissions` no es un objeto; no se ha modificado.".into(),
        ));
    };
    let allow = permissions.entry("allow").or_insert_with(|| json!([]));
    let Some(allow) = allow.as_array_mut() else {
        return Err(RuleError::Unreadable(
            "`permissions.allow` no es una lista; no se ha modificado.".into(),
        ));
    };
    if allow.iter().any(|v| v.as_str() == Some(rule)) {
        return Ok(Added::AlreadyPresent);
    }
    allow.push(json!(rule));

    if let Some(dir) = path.parent() {
        fs::create_dir_all(dir)?;
    }
    // Escritura atómica: un corte a medias no deja el archivo cortado.
    let tmp = path.with_extension("json.tmp");
    fs::write(
        &tmp,
        serde_json::to_string_pretty(&root).map_err(io::Error::other)? + "\n",
    )?;
    fs::rename(&tmp, path)?;
    Ok(Added::Added)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tmp(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rt-rules-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        d
    }

    #[test]
    fn bash_exacto() {
        assert_eq!(
            rule_for("Bash", &json!({"command": "  pnpm db:migrate "})).as_deref(),
            Some("Bash(pnpm db:migrate)")
        );
    }

    #[test]
    fn nunca_se_propone_una_regla_mas_amplia() {
        for command in ["rm -rf *", "ls\nrm x", "", "   "] {
            assert_eq!(
                rule_for("Bash", &json!({"command": command})),
                None,
                "{command:?}"
            );
        }
        assert_eq!(rule_for("Bash", &json!({"command": "x".repeat(501)})), None);
        assert_eq!(rule_for("Bash", &json!({})), None);
        assert_eq!(rule_for("Bash", &json!({"command": 5})), None);
    }

    #[test]
    fn herramientas_de_archivos_no_tienen_regla_suelta() {
        for tool in ["Edit", "Write", "Read", "MultiEdit", "NotebookEdit"] {
            assert_eq!(rule_for(tool, &json!({"file_path": "/x"})), None, "{tool}");
        }
        assert_eq!(
            rule_for("WebSearch", &json!({})).as_deref(),
            Some("WebSearch")
        );
        assert_eq!(
            rule_for("mcp__pencil__draw", &json!({})).as_deref(),
            Some("mcp__pencil__draw")
        );
    }

    #[test]
    fn nombres_raros_no_se_aceptan() {
        for tool in ["", "Bash(x)", "mcp__*", "a\nb"] {
            assert_eq!(rule_for(tool, &json!({})), None, "{tool:?}");
        }
    }

    #[test]
    fn crea_el_archivo_y_los_directorios() {
        let dir = tmp("crea");
        let path = local_settings_path(&dir);
        assert_eq!(add_allow_rule(&path, "Bash(ls)").unwrap(), Added::Added);
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["permissions"]["allow"], json!(["Bash(ls)"]));
        assert!(!path.with_extension("json.tmp").exists());
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn conserva_el_resto_y_no_duplica() {
        let dir = tmp("conserva");
        let path = local_settings_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(
            &path,
            r#"{"model":"opus","permissions":{"allow":["Bash(git status)"],"deny":["Bash(rm *)"]}}"#,
        )
        .unwrap();
        assert_eq!(add_allow_rule(&path, "Bash(ls)").unwrap(), Added::Added);
        assert_eq!(
            add_allow_rule(&path, "Bash(ls)").unwrap(),
            Added::AlreadyPresent
        );
        let v: Value = serde_json::from_str(&fs::read_to_string(&path).unwrap()).unwrap();
        assert_eq!(v["model"], "opus");
        assert_eq!(v["permissions"]["deny"], json!(["Bash(rm *)"]));
        assert_eq!(
            v["permissions"]["allow"],
            json!(["Bash(git status)", "Bash(ls)"])
        );
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn no_pisa_un_archivo_que_no_entiende() {
        let dir = tmp("roto");
        let path = local_settings_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        for contenido in [
            "{roto",
            "[]",
            r#"{"permissions":[]}"#,
            r#"{"permissions":{"allow":"x"}}"#,
        ] {
            fs::write(&path, contenido).unwrap();
            assert!(matches!(
                add_allow_rule(&path, "Bash(ls)"),
                Err(RuleError::Unreadable(_))
            ));
            assert_eq!(
                fs::read_to_string(&path).unwrap(),
                contenido,
                "no debe tocarse"
            );
        }
        fs::remove_dir_all(dir).unwrap();
    }

    #[test]
    fn un_archivo_vacio_se_trata_como_objeto_vacio() {
        let dir = tmp("vacio");
        let path = local_settings_path(&dir);
        fs::create_dir_all(path.parent().unwrap()).unwrap();
        fs::write(&path, "  \n").unwrap();
        assert_eq!(add_allow_rule(&path, "Bash(ls)").unwrap(), Added::Added);
        fs::remove_dir_all(dir).unwrap();
    }
}
