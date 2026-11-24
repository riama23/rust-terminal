//! Atajos de teclado configurables (v1.6). Puro y con tests.
//!
//! Cada acción tiene un identificador (`new_tab`), el selector del menú al
//! que está ligada y su atajo por defecto. En `config.toml`, sección
//! `[keybindings]`, se reasigna con `new_tab = "cmd+t"` (o `""` para
//! quitarlo). Al arrancar y al recargar, [`crate::apply_keybindings`] los pone
//! como equivalentes de teclado del menú.

use std::collections::HashMap;

pub const CMD: u8 = 1;
pub const SHIFT: u8 = 2;
pub const CTRL: u8 = 4;
pub const ALT: u8 = 8;

/// Una acción reasignable: identificador, selector del menú, atajo por
/// defecto y descripción (para la documentación y la ventana de ajustes).
pub struct Action {
    pub id: &'static str,
    pub selector: &'static str,
    pub default: &'static str,
    pub label: &'static str,
}

macro_rules! actions {
    ($(($id:literal, $sel:literal, $def:literal, $label:literal)),* $(,)?) => {
        pub const ACTIONS: &[Action] = &[
            $(Action { id: $id, selector: $sel, default: $def, label: $label }),*
        ];
    };
}

actions![
    (
        "command_palette",
        "commandPalette:",
        "cmd+shift+p",
        "Paleta de comandos"
    ),
    ("new_agent", "newAgent:", "cmd+n", "Nuevo agente"),
    (
        "review_changes",
        "reviewChanges:",
        "cmd+shift+r",
        "Revisar cambios del agente"
    ),
    ("new_tab", "newTab:", "cmd+t", "Nueva pestaña"),
    (
        "split_vertical",
        "splitVertical:",
        "cmd+d",
        "Dividir en vertical"
    ),
    (
        "split_horizontal",
        "splitHorizontal:",
        "cmd+shift+d",
        "Dividir en horizontal"
    ),
    ("close_pane", "closePane:", "cmd+w", "Cerrar el panel"),
    ("close_tab", "closeTab:", "cmd+shift+w", "Cerrar la pestaña"),
    (
        "copy_last_output",
        "copyLastOutput:",
        "cmd+shift+c",
        "Copiar la salida del último comando"
    ),
    (
        "previous_block",
        "previousBlock:",
        "cmd+up",
        "Bloque anterior"
    ),
    ("next_block", "nextBlock:", "cmd+down", "Bloque siguiente"),
    (
        "block_rerun",
        "blockRerun:",
        "",
        "Relanzar el bloque seleccionado"
    ),
    (
        "block_rerun_failed",
        "blockRerunFailed:",
        "",
        "Relanzar solo los tests fallidos del bloque"
    ),
    (
        "block_copy",
        "blockCopy:",
        "",
        "Copiar la salida del bloque seleccionado"
    ),
    (
        "block_stop",
        "blockStop:",
        "",
        "Detener el bloque en ejecución"
    ),
    (
        "block_collapse",
        "blockCollapse:",
        "",
        "Colapsar o expandir la salida del bloque"
    ),
    (
        "block_open",
        "blockOpen:",
        "cmd+o",
        "Abrir el error del bloque en el editor"
    ),
    (
        "previous_tab",
        "previousTab:",
        "cmd+shift+[",
        "Pestaña anterior"
    ),
    ("next_tab", "nextTab:", "cmd+shift+]", "Pestaña siguiente"),
    (
        "focus_left",
        "focusLeft:",
        "cmd+alt+left",
        "Panel de la izquierda"
    ),
    (
        "focus_right",
        "focusRight:",
        "cmd+alt+right",
        "Panel de la derecha"
    ),
    ("focus_up", "focusUp:", "cmd+alt+up", "Panel de arriba"),
    ("focus_down", "focusDown:", "cmd+alt+down", "Panel de abajo"),
    (
        "toggle_zoom",
        "toggleZoom:",
        "cmd+shift+return",
        "Ampliar o restaurar el panel"
    ),
    (
        "resize_left",
        "resizeLeft:",
        "cmd+ctrl+left",
        "Mover el divisor a la izquierda"
    ),
    (
        "resize_right",
        "resizeRight:",
        "cmd+ctrl+right",
        "Mover el divisor a la derecha"
    ),
    (
        "resize_up",
        "resizeUp:",
        "cmd+ctrl+up",
        "Mover el divisor hacia arriba"
    ),
    (
        "resize_down",
        "resizeDown:",
        "cmd+ctrl+down",
        "Mover el divisor hacia abajo"
    ),
    ("open_settings", "openSettings:", "cmd+,", "Ajustes"),
    (
        "report_issue",
        "reportIssue:",
        "cmd+shift+i",
        "Informar de un fallo"
    ),
];

/// Un atajo ya interpretado: la tecla como la espera `NSMenuItem` y los
/// modificadores (`CMD | SHIFT | CTRL | ALT`).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Shortcut {
    pub key: String,
    pub mods: u8,
}

/// El carácter de AppKit para una tecla con nombre.
fn named_key(name: &str) -> Option<&'static str> {
    Some(match name {
        "left" => "\u{F702}",
        "right" => "\u{F703}",
        "up" => "\u{F700}",
        "down" => "\u{F701}",
        "return" | "enter" => "\r",
        "tab" => "\t",
        "space" => " ",
        "escape" | "esc" => "\u{1b}",
        "backspace" => "\u{8}",
        "delete" => "\u{7f}",
        "home" => "\u{F729}",
        "end" => "\u{F72B}",
        "pageup" => "\u{F72C}",
        "pagedown" => "\u{F72D}",
        _ => return None,
    })
}

/// `"cmd+shift+d"` → atajo. `""` quita el atajo (`Ok(None)`).
pub fn parse(text: &str) -> Result<Option<Shortcut>, String> {
    let text = text.trim().to_lowercase();
    if text.is_empty() {
        return Ok(None);
    }
    let mut mods = 0u8;
    let mut key: Option<String> = None;
    // Una `+` final es la propia tecla (`cmd++`).
    let parts: Vec<&str> = if text.ends_with("++") || text == "+" {
        let head = text.trim_end_matches('+');
        head.split('+')
            .filter(|p| !p.is_empty())
            .chain(["+"])
            .collect()
    } else {
        text.split('+').collect()
    };
    for part in parts {
        let part = part.trim();
        match part {
            "cmd" | "command" | "super" => mods |= CMD,
            "shift" => mods |= SHIFT,
            "ctrl" | "control" => mods |= CTRL,
            "alt" | "opt" | "option" => mods |= ALT,
            "" => return Err("atajo mal formado".to_owned()),
            other => {
                if key.is_some() {
                    return Err(format!("dos teclas (`{other}`) en un atajo"));
                }
                let k = if let Some(k) = named_key(other) {
                    k.to_owned()
                } else if let Some(n) = other.strip_prefix('f').and_then(|n| n.parse::<u32>().ok())
                    && (1..=12).contains(&n)
                {
                    char::from_u32(0xF703 + n)
                        .map(String::from)
                        .unwrap_or_default()
                } else if other.chars().count() == 1 {
                    other.to_owned()
                } else {
                    return Err(format!("tecla desconocida `{other}`"));
                };
                key = Some(k);
            }
        }
    }
    let key = key.ok_or("falta la tecla")?;
    let is_fn = key
        .chars()
        .next()
        .is_some_and(|c| ('\u{F704}'..='\u{F70F}').contains(&c));
    // Una tecla normal sin Cmd, Ctrl u Opción robaría lo que se escribe.
    if !is_fn && mods & (CMD | CTRL | ALT) == 0 {
        return Err("necesita ⌘, ⌃ u ⌥ (si no, impediría escribir)".to_owned());
    }
    Ok(Some(Shortcut { key, mods }))
}

/// `⌘⇧T`, `⌘⌥←`: el atajo para enseñarlo.
pub fn describe(s: &Shortcut) -> String {
    let mut out = String::new();
    if s.mods & CTRL != 0 {
        out.push('⌃');
    }
    if s.mods & ALT != 0 {
        out.push('⌥');
    }
    if s.mods & SHIFT != 0 {
        out.push('⇧');
    }
    if s.mods & CMD != 0 {
        out.push('⌘');
    }
    out.push_str(match s.key.as_str() {
        "\u{F702}" => "←",
        "\u{F703}" => "→",
        "\u{F700}" => "↑",
        "\u{F701}" => "↓",
        "\r" => "↵",
        "\t" => "⇥",
        " " => "Espacio",
        "\u{1b}" => "⎋",
        "\u{8}" => "⌫",
        "\u{7f}" => "⌦",
        other => return format!("{out}{}", other.to_uppercase()),
    });
    out
}

/// El atajo de la acción `id` según `bindings` (o su valor por defecto).
pub fn shortcut_for(bindings: &HashMap<String, String>, id: &str) -> Option<Shortcut> {
    let text = bindings
        .get(id)
        .map(String::as_str)
        .or_else(|| ACTIONS.iter().find(|a| a.id == id).map(|a| a.default))?;
    parse(text).ok().flatten()
}

/// El atajo ligado a un selector del menú.
pub fn shortcut_for_selector(
    bindings: &HashMap<String, String>,
    selector: &str,
) -> Option<Shortcut> {
    let action = ACTIONS.iter().find(|a| a.selector == selector)?;
    shortcut_for(bindings, action.id)
}

/// Avisos de atajos repetidos entre acciones (gana el primero del menú).
pub fn conflicts(bindings: &HashMap<String, String>) -> Vec<String> {
    let mut seen: HashMap<(String, u8), &Action> = HashMap::new();
    let mut out = Vec::new();
    for a in ACTIONS {
        if let Some(s) = shortcut_for(bindings, a.id) {
            let k = (s.key.clone(), s.mods);
            match seen.get(&k) {
                Some(prev) => out.push(format!(
                    "`keybindings.{}` repite el atajo {} de `keybindings.{}`",
                    a.id,
                    describe(&s),
                    prev.id
                )),
                None => {
                    seen.insert(k, a);
                }
            }
        }
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn parses_modifiers_and_keys() {
        let s = parse("Cmd+Shift+D").unwrap().unwrap();
        assert_eq!((s.key.as_str(), s.mods), ("d", CMD | SHIFT));
        let s = parse("cmd+alt+left").unwrap().unwrap();
        assert_eq!((s.key.as_str(), s.mods), ("\u{F702}", CMD | ALT));
        assert_eq!(parse("cmd+shift+return").unwrap().unwrap().key, "\r");
        assert_eq!(parse("cmd+,").unwrap().unwrap().key, ",");
        assert_eq!(parse("cmd++").unwrap().unwrap().key, "+");
        assert_eq!(parse("cmd+shift+[").unwrap().unwrap().key, "[");
        assert_eq!(parse("f5").unwrap().unwrap().mods, 0);
        assert_eq!(parse("ctrl+f12").unwrap().unwrap().mods, CTRL);
        assert_eq!(parse("").unwrap(), None);
        assert_eq!(parse("  ").unwrap(), None);
    }

    #[test]
    fn rejects_what_would_break_typing_or_is_malformed() {
        assert!(parse("d").unwrap_err().contains("impediría"));
        assert!(parse("shift+d").is_err());
        assert!(parse("cmd+shift").unwrap_err().contains("falta la tecla"));
        assert!(parse("cmd+a+b").unwrap_err().contains("dos teclas"));
        assert!(parse("cmd+banana").unwrap_err().contains("desconocida"));
        assert!(parse("cmd+").is_err());
        assert!(parse("cmd+f13").is_err());
    }

    #[test]
    fn describes_like_the_menus_do() {
        let d = |t: &str| describe(&parse(t).unwrap().unwrap());
        assert_eq!(d("cmd+shift+t"), "⇧⌘T");
        assert_eq!(d("cmd+alt+left"), "⌥⌘←");
        assert_eq!(d("ctrl+alt+n"), "⌃⌥N");
        assert_eq!(d("cmd+shift+return"), "⇧⌘↵");
        assert_eq!(d("cmd+,"), "⌘,");
    }

    #[test]
    fn every_default_parses_and_there_are_no_conflicts_by_default() {
        for a in ACTIONS {
            assert!(parse(a.default).is_ok(), "{}: {}", a.id, a.default);
            assert!(a.selector.ends_with(':'), "{}", a.id);
        }
        assert_eq!(conflicts(&HashMap::new()), Vec::<String>::new());
        // Ids y selectores únicos.
        let mut ids: Vec<_> = ACTIONS.iter().map(|a| a.id).collect();
        ids.sort();
        ids.dedup();
        assert_eq!(ids.len(), ACTIONS.len());
        let mut sels: Vec<_> = ACTIONS.iter().map(|a| a.selector).collect();
        sels.sort();
        sels.dedup();
        assert_eq!(sels.len(), ACTIONS.len());
    }

    #[test]
    fn overrides_and_conflicts() {
        let mut b = HashMap::new();
        b.insert("new_tab".to_owned(), "cmd+shift+t".to_owned());
        assert_eq!(shortcut_for(&b, "new_tab").unwrap().mods, CMD | SHIFT);
        // Quitar un atajo.
        b.insert("close_pane".to_owned(), String::new());
        assert_eq!(shortcut_for(&b, "close_pane"), None);
        // Repetir uno ya usado.
        b.insert("new_agent".to_owned(), "cmd+d".to_owned());
        let c = conflicts(&b);
        assert_eq!(c.len(), 1, "{c:?}");
        assert!(c[0].contains("new_agent") && c[0].contains("split_vertical"));
        assert_eq!(
            shortcut_for_selector(&b, "newTab:").map(|s| s.key),
            Some("t".to_owned())
        );
        assert_eq!(shortcut_for_selector(&b, "noExiste:"), None);
    }
}
