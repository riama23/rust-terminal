//! Configuración en TOML: `~/.config/<producto>/config.toml`.
//!
//! Las opciones se definen una sola vez en [`OPTIONS`]: clave, tipo, valor
//! por defecto (en TOML), descripción y cuándo se aplica. De esa tabla salen
//! la configuración por defecto (leyendo esos mismos valores), la validación
//! y la documentación (`docs/configuracion.md`), así que no pueden desfasarse.

use std::path::PathBuf;

use rust_terminal_core::PRODUCT_NAME;

/// Cuándo surte efecto un cambio.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Apply {
    Live,
    NewTabs,
}

/// Tipo de una opción, para validar y documentar.
#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Kind {
    Bool,
    Int { min: i64, max: i64 },
    Float { min: f64, max: f64 },
    Str,
    StrList,
    Color,
    Palette,
    Choice(&'static [&'static str]),
}

pub struct OptionSpec {
    pub section: &'static str,
    pub key: &'static str,
    pub kind: Kind,
    /// Valor por defecto, como literal TOML.
    pub default: &'static str,
    /// Solo se leen al generar la documentación (en el test que la mantiene).
    #[cfg_attr(not(test), allow(dead_code))]
    pub apply: Apply,
    #[cfg_attr(not(test), allow(dead_code))]
    pub description: &'static str,
}

/// Una opción de la sección `[keybindings]` (ver `keybindings.rs`).
macro_rules! kb {
    ($id:literal, $def:literal, $label:literal) => {
        OptionSpec {
            section: "keybindings",
            key: $id,
            kind: Kind::Str,
            default: concat!("\"", $def, "\""),
            apply: Apply::Live,
            description: concat!(
                $label,
                ". Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo."
            ),
        }
    };
}

pub const OPTIONS: &[OptionSpec] = &[
    OptionSpec {
        section: "appearance",
        key: "theme",
        kind: Kind::Choice(&["dusk", "paper", "system"]),
        default: r#""dusk""#,
        apply: Apply::Live,
        description: "Tema de la interfaz: `dusk` (oscuro), `paper` (claro) o `system` (sigue al sistema). Los colores que pongas en `[colors]` mandan sobre los del tema.",
    },
    OptionSpec {
        section: "font",
        key: "family",
        kind: Kind::Str,
        default: r#""""#,
        apply: Apply::Live,
        description: "Familia de la fuente (por ejemplo \"JetBrains Mono\"). Vacío: la monoespaciada del sistema (SF Mono). Si no existe, se usa la del sistema.",
    },
    OptionSpec {
        section: "font",
        key: "size",
        kind: Kind::Float {
            min: 6.0,
            max: 72.0,
        },
        default: "13.0",
        apply: Apply::Live,
        description: "Tamaño de la fuente en puntos.",
    },
    OptionSpec {
        section: "colors",
        key: "background",
        kind: Kind::Color,
        default: r##""#1c1c1e""##,
        apply: Apply::Live,
        description: "Color de fondo.",
    },
    OptionSpec {
        section: "colors",
        key: "foreground",
        kind: Kind::Color,
        default: r##""#d8d8d8""##,
        apply: Apply::Live,
        description: "Color del texto.",
    },
    OptionSpec {
        section: "colors",
        key: "cursor",
        kind: Kind::Color,
        default: r##""#d8d8d8""##,
        apply: Apply::Live,
        description: "Color del cursor.",
    },
    OptionSpec {
        section: "colors",
        key: "selection",
        kind: Kind::Color,
        default: r##""#264f78""##,
        apply: Apply::Live,
        description: "Fondo del texto seleccionado.",
    },
    OptionSpec {
        section: "colors",
        key: "palette",
        kind: Kind::Palette,
        default: r##"["#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5", "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff"]"##,
        apply: Apply::Live,
        description: "Los 16 colores ANSI: negro, rojo, verde, amarillo, azul, magenta, cian, blanco y sus versiones brillantes.",
    },
    OptionSpec {
        section: "window",
        key: "dim_unfocused",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::Live,
        description: "Atenúa los paneles sin foco cuando hay varios en la pestaña.",
    },
    OptionSpec {
        section: "window",
        key: "pane_headers",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::Live,
        description: "Cabecera con el título sobre cada panel cuando hay varios en la pestaña.",
    },
    OptionSpec {
        section: "window",
        key: "padding",
        kind: Kind::Float {
            min: 0.0,
            max: 100.0,
        },
        default: "6.0",
        apply: Apply::Live,
        description: "Margen interior de cada panel, en puntos.",
    },
    OptionSpec {
        section: "shell",
        key: "program",
        kind: Kind::Str,
        default: r#""""#,
        apply: Apply::NewTabs,
        description: "Ruta de la shell. Vacío: la de `$SHELL` (o `/bin/zsh`).",
    },
    OptionSpec {
        section: "shell",
        key: "args",
        kind: Kind::StrList,
        default: r#"["-l"]"#,
        apply: Apply::NewTabs,
        description: "Argumentos de la shell. Por defecto, shell de login.",
    },
    OptionSpec {
        section: "shell",
        key: "integration",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::NewTabs,
        description: "Inyecta la integración con zsh (bloques de comandos). También se desactiva con `RT_NO_SHELL_INTEGRATION=1`.",
    },
    OptionSpec {
        section: "agents",
        key: "track_manual_claude",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::NewTabs,
        description: "Muestra en la sidebar los `claude` que lanzas a mano en un panel. Con zsh, una función `claude` le pasa los hooks de la app; con otras shells solo se ve que está en marcha.",
    },
    OptionSpec {
        section: "blocks",
        key: "visual",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::Live,
        description: "Tiñe los bloques fallidos y el seleccionado y muestra la barra de acciones del bloque (⌘↑ y ⌘↓ seleccionan). Con `false`, solo quedan las marcas del margen.",
    },
    OptionSpec {
        section: "blocks",
        key: "editor",
        kind: Kind::Str,
        default: r#""""#,
        apply: Apply::Live,
        description: "Orden para «Abrir en el editor» (⌘O) con `{path}`, `{line}` y `{col}`, por ejemplo `code -g {path}:{line}:{col}`. Vacío: `open -t` (el editor de texto por defecto, sin línea).",
    },
    OptionSpec {
        section: "scrollback",
        key: "lines",
        kind: Kind::Int {
            min: 0,
            max: 1_000_000,
        },
        default: "10000",
        apply: Apply::NewTabs,
        description: "Líneas del historial por panel. Cada línea de 170 columnas ocupa unos 2,7 KB.",
    },
    OptionSpec {
        section: "clipboard",
        key: "write",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::Live,
        description: "Permite que los programas copien al portapapeles con OSC 52 (vim, tmux, ssh…).",
    },
    OptionSpec {
        section: "clipboard",
        key: "read",
        kind: Kind::Choice(&["deny", "ask", "allow"]),
        default: r#""deny""#,
        apply: Apply::Live,
        description: "Si los programas pueden leer el portapapeles con OSC 52: `deny` (nunca), `ask` (preguntar cada vez) o `allow` (siempre). Leerlo permite a un programa remoto ver lo que has copiado.",
    },
    OptionSpec {
        section: "notifications",
        key: "enabled",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::Live,
        description: "Notificaciones del sistema con la ventana en segundo plano (las de los programas y las de comandos largos).",
    },
    OptionSpec {
        section: "notifications",
        key: "long_command_seconds",
        kind: Kind::Int {
            min: 0,
            max: 86_400,
        },
        default: "10",
        apply: Apply::Live,
        description: "Avisa cuando termina un comando que tardó al menos estos segundos (requiere la integración con zsh). 0: nunca.",
    },
    OptionSpec {
        section: "bell",
        key: "dock_bounce",
        kind: Kind::Bool,
        default: "true",
        apply: Apply::Live,
        description: "Con la ventana en segundo plano, la campana (BEL) hace rebotar el icono del Dock.",
    },
    kb!("command_palette", "cmd+shift+p", "Paleta de comandos"),
    kb!("new_agent", "cmd+n", "Nuevo agente"),
    kb!(
        "review_changes",
        "cmd+shift+r",
        "Revisar cambios del agente"
    ),
    kb!("new_tab", "cmd+t", "Nueva pestaña"),
    kb!("split_vertical", "cmd+d", "Dividir en vertical"),
    kb!("split_horizontal", "cmd+shift+d", "Dividir en horizontal"),
    kb!("close_pane", "cmd+w", "Cerrar el panel"),
    kb!("close_tab", "cmd+shift+w", "Cerrar la pestaña"),
    kb!(
        "copy_last_output",
        "cmd+shift+c",
        "Copiar la salida del último comando"
    ),
    kb!("previous_block", "cmd+up", "Bloque anterior"),
    kb!("next_block", "cmd+down", "Bloque siguiente"),
    kb!("block_rerun", "", "Relanzar el bloque seleccionado"),
    kb!(
        "block_rerun_failed",
        "",
        "Relanzar solo los tests fallidos del bloque"
    ),
    kb!("block_copy", "", "Copiar la salida del bloque seleccionado"),
    kb!("block_stop", "", "Detener el bloque en ejecución"),
    kb!(
        "block_collapse",
        "",
        "Colapsar o expandir la salida del bloque"
    ),
    kb!(
        "block_open",
        "cmd+o",
        "Abrir el error del bloque en el editor"
    ),
    kb!("previous_tab", "cmd+shift+[", "Pestaña anterior"),
    kb!("next_tab", "cmd+shift+]", "Pestaña siguiente"),
    kb!("focus_left", "cmd+alt+left", "Panel de la izquierda"),
    kb!("focus_right", "cmd+alt+right", "Panel de la derecha"),
    kb!("focus_up", "cmd+alt+up", "Panel de arriba"),
    kb!("focus_down", "cmd+alt+down", "Panel de abajo"),
    kb!(
        "toggle_zoom",
        "cmd+shift+return",
        "Ampliar o restaurar el panel"
    ),
    kb!(
        "resize_left",
        "cmd+ctrl+left",
        "Mover el divisor a la izquierda"
    ),
    kb!(
        "resize_right",
        "cmd+ctrl+right",
        "Mover el divisor a la derecha"
    ),
    kb!("resize_up", "cmd+ctrl+up", "Mover el divisor hacia arriba"),
    kb!(
        "resize_down",
        "cmd+ctrl+down",
        "Mover el divisor hacia abajo"
    ),
    kb!("open_settings", "cmd+,", "Ajustes"),
    kb!("report_issue", "cmd+shift+i", "Informar de un fallo"),
];

pub type Rgb = [u8; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRead {
    Deny,
    Ask,
    Allow,
}

/// Qué tema pide la configuración.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Appearance {
    Dusk,
    Paper,
    System,
}

/// Colores del tema.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Theme {
    pub background: Rgb,
    pub foreground: Rgb,
    pub cursor: Rgb,
    pub selection: Rgb,
    pub palette: [Rgb; 16],
}

/// Colores de la terminal con el tema Paper (claro). La paleta ANSI se
/// deriva de los colores de estado del spec y cada color cumple 4,5:1 sobre
/// el fondo (lo comprueba un test); el «blanco» (7 y 15) es un gris oscuro
/// porque un blanco no se leería sobre un fondo casi blanco.
pub fn paper_theme() -> Theme {
    let hex = |s: &str| parse_color(s).expect("color de Paper válido");
    Theme {
        background: hex("#fbfbfa"),
        foreground: hex("#1c1d21"),
        cursor: hex("#2f5fd0"),
        selection: hex("#cfe0ff"),
        palette: [
            hex("#1c1d21"),
            hex("#c8372d"),
            hex("#1b7f45"),
            hex("#8a6100"),
            hex("#2f5fd0"),
            hex("#8e3fb0"),
            hex("#0b7a8a"),
            hex("#555a63"),
            hex("#6b7079"),
            hex("#b02a20"),
            hex("#17713c"),
            hex("#7a5400"),
            hex("#2450b8"),
            hex("#7b2f9b"),
            hex("#08656f"),
            hex("#3a3e47"),
        ],
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Config {
    /// `None`: la monoespaciada del sistema.
    pub font_family: Option<String>,
    pub font_size: f64,
    pub theme: Theme,
    pub padding: f64,
    /// `None`: `$SHELL`.
    pub shell_program: Option<String>,
    pub shell_args: Vec<String>,
    pub shell_integration: bool,
    pub track_manual_claude: bool,
    pub dim_unfocused: bool,
    pub pane_headers: bool,
    pub block_visual: bool,
    pub block_editor: String,
    /// Atajos por acción (`[keybindings]`).
    pub keybindings: std::collections::HashMap<String, String>,
    pub appearance: Appearance,
    /// Claves de `[colors]` que el usuario puso en el archivo: no las pisa el tema.
    pub colors_set: Vec<String>,
    pub scrollback_lines: usize,
    pub clipboard_write: bool,
    pub clipboard_read: ClipboardRead,
    pub notifications: bool,
    /// 0: nunca.
    pub long_command_seconds: u64,
    pub bell_dock_bounce: bool,
}

/// Valor ya validado.
enum Value {
    Bool(bool),
    Int(i64),
    Float(f64),
    Str(String),
    StrList(Vec<String>),
    Color(Rgb),
    Palette([Rgb; 16]),
}

/// El literal TOML para `input` escrito en la ventana de ajustes, ya
/// validado contra el tipo y los límites de la opción.
pub fn literal_for(spec: &OptionSpec, input: &str) -> Result<String, String> {
    use toml::Value as T;
    let input = input.trim();
    let value = match spec.kind {
        Kind::Bool => match input {
            "true" | "sí" | "si" | "on" => T::Boolean(true),
            "false" | "no" | "off" => T::Boolean(false),
            _ => return Err("debe ser true o false".to_owned()),
        },
        Kind::Int { .. } => T::Integer(
            input
                .parse::<i64>()
                .map_err(|_| "debe ser un número entero".to_owned())?,
        ),
        Kind::Float { .. } => T::Float(
            input
                .replace(',', ".")
                .parse::<f64>()
                .map_err(|_| "debe ser un número".to_owned())?,
        ),
        Kind::Str | Kind::Color | Kind::Choice(_) => T::String(input.to_owned()),
        Kind::StrList => T::Array(
            input
                .split(',')
                .map(str::trim)
                .filter(|s| !s.is_empty())
                .map(|s| T::String(s.to_owned()))
                .collect(),
        ),
        Kind::Palette => return Err("la paleta de 16 colores se edita en config.toml".to_owned()),
    };
    match validate(spec.kind, &value)? {
        Value::Str(s) if spec.section == "keybindings" => {
            crate::keybindings::parse(&s)?;
        }
        _ => {}
    }
    Ok(match &value {
        T::Boolean(b) => b.to_string(),
        T::Integer(i) => i.to_string(),
        T::Float(f) => format!("{f:?}"),
        T::String(s) => crate::config_edit::toml_string(s),
        T::Array(a) => format!(
            "[{}]",
            a.iter()
                .filter_map(|x| x.as_str())
                .map(crate::config_edit::toml_string)
                .collect::<Vec<_>>()
                .join(", ")
        ),
        _ => unreachable!("tipos validados arriba"),
    })
}

/// El valor actual de una opción como texto para la ventana de ajustes: el
/// del archivo si está puesto y, si no, el por defecto.
pub fn display_value(spec: &OptionSpec, table: &toml::Table) -> String {
    use toml::Value as T;
    let from_file = table
        .get(spec.section)
        .and_then(|s| s.as_table())
        .and_then(|s| s.get(spec.key));
    let default: toml::Value = format!("v = {}", spec.default)
        .parse::<toml::Table>()
        .ok()
        .and_then(|mut t| t.remove("v"))
        .unwrap_or_else(|| T::String(String::new()));
    match from_file.unwrap_or(&default) {
        T::Boolean(b) => b.to_string(),
        T::Integer(i) => i.to_string(),
        T::Float(f) => format!("{f}"),
        T::String(s) => s.clone(),
        T::Array(a) => a
            .iter()
            .filter_map(|x| x.as_str())
            .collect::<Vec<_>>()
            .join(", "),
        other => other.to_string(),
    }
}

/// ¿Está esta opción puesta en el archivo?
pub fn is_set(spec: &OptionSpec, table: &toml::Table) -> bool {
    table
        .get(spec.section)
        .and_then(|s| s.as_table())
        .is_some_and(|s| s.contains_key(spec.key))
}

fn parse_color(s: &str) -> Option<Rgb> {
    let hex = s.strip_prefix('#')?;
    if hex.len() != 6 || !hex.chars().all(|c| c.is_ascii_hexdigit()) {
        return None;
    }
    let v = u32::from_str_radix(hex, 16).ok()?;
    Some([(v >> 16) as u8, (v >> 8) as u8, v as u8])
}

fn validate(kind: Kind, v: &toml::Value) -> Result<Value, String> {
    use toml::Value as T;
    match (kind, v) {
        (Kind::Bool, T::Boolean(b)) => Ok(Value::Bool(*b)),
        (Kind::Int { min, max }, T::Integer(i)) => {
            if (min..=max).contains(i) {
                Ok(Value::Int(*i))
            } else {
                Err(format!("debe estar entre {min} y {max}"))
            }
        }
        (Kind::Float { min, max }, T::Float(_) | T::Integer(_)) => {
            let f = v
                .as_float()
                .unwrap_or_else(|| v.as_integer().unwrap_or(0) as f64);
            if f >= min && f <= max {
                Ok(Value::Float(f))
            } else {
                Err(format!("debe estar entre {min} y {max}"))
            }
        }
        (Kind::Str, T::String(s)) => Ok(Value::Str(s.clone())),
        (Kind::StrList, T::Array(a)) => a
            .iter()
            .map(|x| x.as_str().map(str::to_owned))
            .collect::<Option<Vec<_>>>()
            .map(Value::StrList)
            .ok_or_else(|| "debe ser una lista de textos".to_owned()),
        (Kind::Color, T::String(s)) => parse_color(s)
            .map(Value::Color)
            .ok_or_else(|| format!("\"{s}\" no es un color \"#rrggbb\"")),
        (Kind::Palette, T::Array(a)) => {
            if a.len() != 16 {
                return Err(format!("debe tener 16 colores (tiene {})", a.len()));
            }
            let mut out = [[0u8; 3]; 16];
            for (i, c) in a.iter().enumerate() {
                out[i] = c
                    .as_str()
                    .and_then(parse_color)
                    .ok_or_else(|| format!("el color {} no es \"#rrggbb\"", i + 1))?;
            }
            Ok(Value::Palette(out))
        }
        (Kind::Choice(choices), T::String(s)) => {
            if choices.contains(&s.as_str()) {
                Ok(Value::Str(s.clone()))
            } else {
                Err(format!("debe ser uno de: {}", choices.join(", ")))
            }
        }
        (kind, _) => Err(format!("debe ser {}", kind_name(kind))),
    }
}

fn kind_name(kind: Kind) -> String {
    match kind {
        Kind::Bool => "`true` o `false`".to_owned(),
        Kind::Int { min, max } => format!("un entero ({min}–{max})"),
        Kind::Float { min, max } => format!("un número ({min}–{max})"),
        Kind::Str => "un texto".to_owned(),
        Kind::StrList => "una lista de textos".to_owned(),
        Kind::Color => "un color `\"#rrggbb\"`".to_owned(),
        Kind::Palette => "una lista de 16 colores `\"#rrggbb\"`".to_owned(),
        Kind::Choice(c) => format!(
            "uno de: {}",
            c.iter()
                .map(|x| format!("`\"{x}\"`"))
                .collect::<Vec<_>>()
                .join(", ")
        ),
    }
}

impl Config {
    /// Asigna una opción ya validada. Cada entrada de [`OPTIONS`] debe tener
    /// su rama (un test lo comprueba).
    fn set(&mut self, section: &str, key: &str, v: Value) {
        match (section, key, v) {
            ("font", "family", Value::Str(s)) => self.font_family = (!s.is_empty()).then_some(s),
            ("font", "size", Value::Float(f)) => self.font_size = f,
            ("colors", "background", Value::Color(c)) => self.theme.background = c,
            ("colors", "foreground", Value::Color(c)) => self.theme.foreground = c,
            ("colors", "cursor", Value::Color(c)) => self.theme.cursor = c,
            ("colors", "selection", Value::Color(c)) => self.theme.selection = c,
            ("colors", "palette", Value::Palette(p)) => self.theme.palette = p,
            ("window", "padding", Value::Float(f)) => self.padding = f,
            ("window", "dim_unfocused", Value::Bool(b)) => self.dim_unfocused = b,
            ("window", "pane_headers", Value::Bool(b)) => self.pane_headers = b,
            ("blocks", "visual", Value::Bool(b)) => self.block_visual = b,
            ("blocks", "editor", Value::Str(s)) => self.block_editor = s,
            ("shell", "program", Value::Str(s)) => {
                self.shell_program = (!s.is_empty()).then_some(s)
            }
            ("shell", "args", Value::StrList(a)) => self.shell_args = a,
            ("shell", "integration", Value::Bool(b)) => self.shell_integration = b,
            ("agents", "track_manual_claude", Value::Bool(b)) => self.track_manual_claude = b,
            ("scrollback", "lines", Value::Int(i)) => self.scrollback_lines = i as usize,
            ("clipboard", "write", Value::Bool(b)) => self.clipboard_write = b,
            ("clipboard", "read", Value::Str(s)) => {
                self.clipboard_read = match s.as_str() {
                    "ask" => ClipboardRead::Ask,
                    "allow" => ClipboardRead::Allow,
                    _ => ClipboardRead::Deny,
                }
            }
            ("notifications", "enabled", Value::Bool(b)) => self.notifications = b,
            ("notifications", "long_command_seconds", Value::Int(i)) => {
                self.long_command_seconds = i as u64
            }
            ("bell", "dock_bounce", Value::Bool(b)) => self.bell_dock_bounce = b,
            ("appearance", "theme", Value::Str(s)) => {
                self.appearance = match s.as_str() {
                    "paper" => Appearance::Paper,
                    "system" => Appearance::System,
                    _ => Appearance::Dusk,
                }
            }
            ("keybindings", id, Value::Str(s)) => {
                self.keybindings.insert(id.to_owned(), s);
            }
            (s, k, _) => unreachable!("opción {s}.{k} sin asignar en Config::set"),
        }
    }

    /// Lee un archivo de configuración. Una opción no válida se ignora (queda
    /// su valor por defecto) y se describe en la lista de errores. Un error de
    /// sintaxis TOML devuelve `Err`: entonces se conserva la configuración anterior.
    pub fn parse(text: &str) -> Result<(Config, Vec<String>), String> {
        let mut config = Config::default();
        let mut errors = Vec::new();
        let table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                let e: toml::de::Error = e;
                let line = e
                    .span()
                    .map(|s| text[..s.start.min(text.len())].matches('\n').count() + 1);
                let msg = e.message().trim().to_owned();
                return Err(match line {
                    Some(l) => format!("línea {l}: {msg}"),
                    None => msg,
                });
            }
        };
        for (section, value) in &table {
            let Some(keys) = value.as_table() else {
                errors.push(format!("`{section}` debe ser una sección (`[{section}]`)"));
                continue;
            };
            for (key, v) in keys {
                let Some(spec) = OPTIONS
                    .iter()
                    .find(|o| o.section == section && o.key == key)
                else {
                    errors.push(format!("opción desconocida `{section}.{key}`"));
                    continue;
                };
                match validate(spec.kind, v) {
                    // Un atajo mal escrito se ignora y se explica por qué.
                    Ok(Value::Str(s)) if section == "keybindings" => {
                        match crate::keybindings::parse(&s) {
                            Ok(_) => config.set(section, key, Value::Str(s)),
                            Err(e) => errors.push(format!("`{section}.{key}` {e}")),
                        }
                    }
                    Ok(v) => {
                        if section == "colors" {
                            config.colors_set.push(key.clone());
                        }
                        config.set(section, key, v)
                    }
                    Err(e) => errors.push(format!("`{section}.{key}` {e}")),
                }
            }
        }
        errors.extend(crate::keybindings::conflicts(&config.keybindings));
        Ok((config, errors))
    }

    pub fn path() -> PathBuf {
        let base = std::env::var_os("XDG_CONFIG_HOME")
            .map(PathBuf::from)
            .filter(|p| p.is_absolute())
            .unwrap_or_else(|| {
                PathBuf::from(std::env::var_os("HOME").unwrap_or_default()).join(".config")
            });
        base.join(PRODUCT_NAME).join("config.toml")
    }

    /// Lee la configuración del usuario. Sin archivo, la de por defecto.
    pub fn load() -> Result<(Config, Vec<String>), String> {
        match std::fs::read_to_string(Config::path()) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => {
                Ok((Config::default(), Vec::new()))
            }
            Err(e) => Err(format!("no se pudo leer: {e}")),
        }
    }

    /// ¿Se usa el tema claro? `system_dark`: el sistema está en modo oscuro.
    pub fn use_paper(&self, system_dark: bool) -> bool {
        match self.appearance {
            Appearance::Dusk => false,
            Appearance::Paper => true,
            Appearance::System => !system_dark,
        }
    }

    /// Los colores de la terminal para el tema activo. Con Paper, los que el
    /// usuario no puso en `[colors]` son los de Paper.
    pub fn effective_theme(&self, paper: bool) -> Theme {
        if !paper {
            return self.theme;
        }
        let mut t = paper_theme();
        let set = |k: &str| self.colors_set.iter().any(|s| s == k);
        if set("background") {
            t.background = self.theme.background;
        }
        if set("foreground") {
            t.foreground = self.theme.foreground;
        }
        if set("cursor") {
            t.cursor = self.theme.cursor;
        }
        if set("selection") {
            t.selection = self.theme.selection;
        }
        if set("palette") {
            t.palette = self.theme.palette;
        }
        t
    }

    /// Shell a lanzar: la configurada o `$SHELL`.
    pub fn shell(&self) -> String {
        self.shell_program
            .clone()
            .or_else(|| std::env::var("SHELL").ok())
            .unwrap_or_else(|| "/bin/zsh".to_owned())
    }
}

impl Default for Config {
    /// Los valores por defecto salen de los literales de [`OPTIONS`].
    fn default() -> Config {
        let mut config = Config {
            font_family: None,
            font_size: 0.0,
            theme: Theme {
                background: [0; 3],
                foreground: [0; 3],
                cursor: [0; 3],
                selection: [0; 3],
                palette: [[0; 3]; 16],
            },
            padding: 0.0,
            shell_program: None,
            shell_args: Vec::new(),
            shell_integration: false,
            track_manual_claude: false,
            dim_unfocused: false,
            pane_headers: false,
            block_visual: false,
            block_editor: String::new(),
            keybindings: Default::default(),
            appearance: Appearance::Dusk,
            colors_set: Vec::new(),
            scrollback_lines: 0,
            clipboard_write: false,
            clipboard_read: ClipboardRead::Deny,
            notifications: false,
            long_command_seconds: 0,
            bell_dock_bounce: false,
        };
        for spec in OPTIONS {
            let value: toml::Value = format!("v = {}", spec.default)
                .parse::<toml::Table>()
                .expect("valor por defecto en TOML válido")
                .remove("v")
                .expect("valor por defecto");
            let v = validate(spec.kind, &value).expect("valor por defecto válido");
            config.set(spec.section, spec.key, v);
        }
        config
    }
}

/// Documentación de todas las opciones, en Markdown (la genera y comprueba
/// el test `docs_are_up_to_date`).
#[cfg(test)]
pub fn docs_markdown() -> String {
    let mut out = String::new();
    out.push_str("# Configuración\n\n");
    out.push_str("> Archivo generado a partir de `crates/app/src/config.rs`. No lo edites a mano: `RT_UPDATE_DOCS=1 cargo test -p rust-terminal config` lo regenera.\n\n");
    out.push_str(&format!(
        "La configuración se lee de `~/.config/{PRODUCT_NAME}/config.toml` (o de `$XDG_CONFIG_HOME/{PRODUCT_NAME}/config.toml`). El archivo es opcional y todas las opciones tienen un valor por defecto.\n\n"
    ));
    out.push_str("Los cambios se aplican al guardar el archivo, sin reiniciar. Las opciones marcadas como **pestañas nuevas** solo afectan a las pestañas y paneles que se abran después. Si hay un error, se muestra un aviso y se ignora solo la opción incorrecta.\n\n");
    let mut section = "";
    for spec in OPTIONS {
        if spec.section != section {
            section = spec.section;
            out.push_str(&format!("## `[{section}]`\n\n"));
        }
        out.push_str(&format!("### `{}`\n\n{}\n\n", spec.key, spec.description));
        out.push_str(&format!("- **Tipo:** {}\n", kind_name(spec.kind)));
        out.push_str(&format!("- **Por defecto:** `{}`\n", spec.default));
        out.push_str(&format!(
            "- **Se aplica:** {}\n\n",
            match spec.apply {
                Apply::Live => "en vivo",
                Apply::NewTabs => "pestañas nuevas",
            }
        ));
    }
    out.push_str("## Ejemplo\n\n```toml\n[font]\nfamily = \"JetBrains Mono\"\nsize = 14\n\n[colors]\nbackground = \"#101014\"\n\n[notifications]\nlong_command_seconds = 30\n```\n");
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn defaults_come_from_the_option_table() {
        let c = Config::default();
        assert_eq!(c.font_size, 13.0);
        assert_eq!(c.font_family, None);
        assert_eq!(c.theme.background, [0x1c, 0x1c, 0x1e]);
        assert_eq!(c.theme.palette[1], [0xcd, 0x31, 0x31]);
        assert_eq!(c.shell_args, ["-l"]);
        assert!(c.shell_integration && c.clipboard_write && c.notifications);
        assert_eq!(c.clipboard_read, ClipboardRead::Deny);
        assert_eq!(c.scrollback_lines, 10_000);
        assert_eq!(c.long_command_seconds, 10);
    }

    #[test]
    fn every_option_is_assignable_and_documented() {
        // Una configuración con todos los valores por defecto escritos.
        let mut text = String::new();
        let mut section = "";
        for spec in OPTIONS {
            if spec.section != section {
                section = spec.section;
                text.push_str(&format!("[{section}]\n"));
            }
            text.push_str(&format!("{} = {}\n", spec.key, spec.default));
        }
        let (mut c, errors) = Config::parse(&text).unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        // Escribir todos los valores por defecto cuenta como poner los colores.
        c.colors_set.clear();
        assert_eq!(c, Config::default());
        let docs = docs_markdown();
        for spec in OPTIONS {
            assert!(docs.contains(&format!("### `{}`", spec.key)));
        }
    }

    #[test]
    fn parses_values() {
        let (c, errors) = Config::parse(
            r##"
[font]
family = "JetBrains Mono"
size = 15

[colors]
background = "#000000"

[shell]
program = "/bin/bash"
integration = false

[clipboard]
read = "ask"
"##,
        )
        .unwrap();
        assert!(errors.is_empty(), "{errors:?}");
        assert_eq!(c.font_family.as_deref(), Some("JetBrains Mono"));
        assert_eq!(c.font_size, 15.0);
        assert_eq!(c.theme.background, [0, 0, 0]);
        assert_eq!(c.shell(), "/bin/bash");
        assert!(!c.shell_integration);
        assert_eq!(c.clipboard_read, ClipboardRead::Ask);
        // Lo no indicado conserva su valor por defecto.
        assert_eq!(c.theme.foreground, Config::default().theme.foreground);
    }

    #[test]
    fn invalid_values_are_reported_and_ignored() {
        let (c, errors) = Config::parse(
            r##"
[font]
size = 500
colour = "x"

[colors]
background = "negro"
palette = ["#000000"]

[clipboard]
read = "sometimes"

[nope]
x = 1
"##,
        )
        .unwrap();
        assert_eq!(c, Config::default());
        assert_eq!(errors.len(), 6, "{errors:#?}");
        assert!(
            errors
                .iter()
                .any(|e| e.contains("`font.size` debe estar entre 6 y 72"))
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("opción desconocida `font.colour`"))
        );
        assert!(
            errors
                .iter()
                .any(|e| e.contains("\"negro\" no es un color"))
        );
        assert!(errors.iter().any(|e| e.contains("16 colores")));
        assert!(errors.iter().any(|e| e.contains("deny, ask, allow")));
        assert!(errors.iter().any(|e| e.contains("`nope.x`")));
    }

    #[test]
    fn syntax_errors_report_the_line() {
        let error = Config::parse("[font]\nsize = 14\nfamily = \n").unwrap_err();
        assert!(error.starts_with("línea 3"), "{error}");
    }

    /// `docs/configuracion.md` está al día con la tabla de opciones.
    #[test]
    fn docs_are_up_to_date() {
        let path =
            std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("../../docs/configuracion.md");
        let expected = docs_markdown();
        if std::env::var_os("RT_UPDATE_DOCS").is_some() {
            std::fs::write(&path, &expected).unwrap();
        }
        let actual = std::fs::read_to_string(&path).unwrap_or_default();
        assert!(
            actual == expected,
            "docs/configuracion.md no está al día: RT_UPDATE_DOCS=1 cargo test -p rust-terminal config"
        );
    }

    fn spec(section: &str, key: &str) -> &'static OptionSpec {
        OPTIONS
            .iter()
            .find(|o| o.section == section && o.key == key)
            .unwrap()
    }

    #[test]
    fn settings_input_becomes_a_validated_literal() {
        assert_eq!(literal_for(spec("font", "size"), "15").unwrap(), "15.0");
        assert_eq!(literal_for(spec("font", "size"), "14,5").unwrap(), "14.5");
        assert!(
            literal_for(spec("font", "size"), "500")
                .unwrap_err()
                .contains("entre")
        );
        assert!(literal_for(spec("font", "size"), "grande").is_err());
        assert_eq!(
            literal_for(spec("blocks", "visual"), "false").unwrap(),
            "false"
        );
        assert!(literal_for(spec("blocks", "visual"), "quizá").is_err());
        assert_eq!(
            literal_for(spec("font", "family"), "JetBrains \"Mono\"").unwrap(),
            r#""JetBrains \"Mono\"""#
        );
        assert_eq!(
            literal_for(spec("shell", "args"), "-l, -i").unwrap(),
            r#"["-l", "-i"]"#
        );
        assert!(literal_for(spec("colors", "background"), "rojo").is_err());
        assert_eq!(
            literal_for(spec("colors", "background"), "#101014").unwrap(),
            "\"#101014\""
        );
        assert!(literal_for(spec("colors", "palette"), "x").is_err());
        // Un atajo mal escrito no se acepta.
        assert!(literal_for(spec("keybindings", "new_tab"), "t").is_err());
        assert_eq!(
            literal_for(spec("keybindings", "new_tab"), "cmd+shift+t").unwrap(),
            "\"cmd+shift+t\""
        );
    }

    #[test]
    fn display_value_prefers_the_file_over_the_default() {
        let table: toml::Table = "[font]\nsize = 15.5\n".parse().unwrap();
        assert_eq!(display_value(spec("font", "size"), &table), "15.5");
        assert!(is_set(spec("font", "size"), &table));
        assert_eq!(display_value(spec("blocks", "visual"), &table), "true");
        assert!(!is_set(spec("blocks", "visual"), &table));
        assert_eq!(display_value(spec("shell", "args"), &table), "-l");
    }

    #[test]
    fn keybinding_options_match_the_action_table() {
        for a in crate::keybindings::ACTIONS {
            let o = spec("keybindings", a.id);
            assert_eq!(o.default, format!("\"{}\"", a.default), "{}", a.id);
        }
        let n = OPTIONS
            .iter()
            .filter(|o| o.section == "keybindings")
            .count();
        assert_eq!(n, crate::keybindings::ACTIONS.len());
    }

    /// Contraste WCAG entre dos colores.
    fn contrast(a: Rgb, b: Rgb) -> f64 {
        let lum = |c: Rgb| {
            let f = |v: u8| {
                let v = f64::from(v) / 255.0;
                if v <= 0.03928 {
                    v / 12.92
                } else {
                    ((v + 0.055) / 1.055).powf(2.4)
                }
            };
            0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
        };
        let (la, lb) = (lum(a), lum(b));
        (la.max(lb) + 0.05) / (la.min(lb) + 0.05)
    }

    #[test]
    fn every_paper_color_reads_on_the_paper_background() {
        let t = paper_theme();
        for (i, c) in t.palette.iter().enumerate() {
            let r = contrast(*c, t.background);
            assert!(r >= 4.5, "color ANSI {i} {c:?}: contraste {r:.2} < 4,5");
        }
        assert!(contrast(t.foreground, t.background) >= 4.5);
        assert!(contrast(t.cursor, t.background) >= 3.0);
        // El texto sobre la selección también se lee.
        assert!(contrast(t.foreground, t.selection) >= 4.5);
    }

    #[test]
    fn paper_tokens_text_is_readable() {
        let p = &crate::palette::PAPER_TOKENS;
        for (name, c) in [
            ("text", p.text),
            ("muted", p.muted),
            ("accent", p.accent),
            ("ok", p.ok),
            ("error", p.error),
            ("attention", p.attention),
            ("warn", p.warn),
            ("prod", p.prod),
        ] {
            assert!(contrast(c, p.bg) >= 4.5, "{name}: {:.2}", contrast(c, p.bg));
        }
        // `ghost` es decorativo (el spec lo exime).
        let d = &crate::palette::DUSK;
        for (name, c) in [
            ("text", d.text),
            ("muted", d.muted),
            ("accent", d.accent),
            ("ok", d.ok),
            ("error", d.error),
        ] {
            assert!(contrast(c, d.bg) >= 4.5, "dusk {name}");
        }
    }

    #[test]
    fn the_theme_choice_and_user_colors() {
        let (c, errs) =
            Config::parse("[appearance]\ntheme = \"paper\"\n[colors]\nbackground = \"#101010\"\n")
                .unwrap();
        assert!(errs.is_empty(), "{errs:?}");
        assert!(c.use_paper(true) && c.use_paper(false));
        let t = c.effective_theme(true);
        // El fondo lo puso el usuario; el resto es de Paper.
        assert_eq!(t.background, [0x10, 0x10, 0x10]);
        assert_eq!(t.foreground, paper_theme().foreground);
        // Con Dusk no se toca nada.
        let (d, _) = Config::parse("").unwrap();
        assert!(!d.use_paper(false));
        assert_eq!(d.effective_theme(false), d.theme);
        // Follow system.
        let (s, _) = Config::parse("[appearance]\ntheme = \"system\"\n").unwrap();
        assert!(!s.use_paper(true), "sistema oscuro → Dusk");
        assert!(s.use_paper(false), "sistema claro → Paper");
        assert!(
            Config::parse("[appearance]\ntheme = \"rosa\"\n")
                .unwrap()
                .1
                .len()
                == 1
        );
    }
}
