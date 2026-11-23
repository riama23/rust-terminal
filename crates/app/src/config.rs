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

pub const OPTIONS: &[OptionSpec] = &[
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
];

pub type Rgb = [u8; 3];

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ClipboardRead {
    Deny,
    Ask,
    Allow,
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
            ("shell", "program", Value::Str(s)) => {
                self.shell_program = (!s.is_empty()).then_some(s)
            }
            ("shell", "args", Value::StrList(a)) => self.shell_args = a,
            ("shell", "integration", Value::Bool(b)) => self.shell_integration = b,
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
            (s, k, _) => unreachable!("opción {s}.{k} sin asignar en Config::set"),
        }
    }

    /// Lee un archivo de configuración. Lo que no es válido se ignora (queda
    /// el valor por defecto) y se describe en la lista de errores.
    pub fn parse(text: &str) -> (Config, Vec<String>) {
        let mut config = Config::default();
        let mut errors = Vec::new();
        let table: toml::Table = match text.parse() {
            Ok(t) => t,
            Err(e) => {
                let e: toml::de::Error = e;
                let line = e
                    .span()
                    .map(|s| text[..s.start.min(text.len())].matches('\n').count() + 1);
                let msg = e.message().to_owned();
                errors.push(match line {
                    Some(l) => format!("línea {l}: {msg}"),
                    None => msg,
                });
                return (config, errors);
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
                    Ok(v) => config.set(section, key, v),
                    Err(e) => errors.push(format!("`{section}.{key}` {e}")),
                }
            }
        }
        (config, errors)
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
    pub fn load() -> (Config, Vec<String>) {
        match std::fs::read_to_string(Config::path()) {
            Ok(text) => Config::parse(&text),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => (Config::default(), Vec::new()),
            Err(e) => (Config::default(), vec![format!("no se pudo leer: {e}")]),
        }
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
        let (c, errors) = Config::parse(&text);
        assert!(errors.is_empty(), "{errors:?}");
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
        );
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
        );
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
        let (c, errors) = Config::parse("[font]\nsize = 14\nfamily = \n");
        assert_eq!(c, Config::default());
        assert!(errors[0].starts_with("línea 3"), "{errors:?}");
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
}
