//! Informes de fallos: abre la página de issue nueva del repositorio con un
//! diagnóstico ya rellenado. El usuario lo revisa en el navegador (puede
//! quitar lo que no quiera compartir) y lo envía.

use rust_terminal_core::PRODUCT_NAME;

/// Página de issue nueva del repositorio.
pub const NEW_ISSUE_URL: &str = "https://github.com/riama23/rust-terminal/issues/new";

/// Longitud máxima de la URL: GitHub y los navegadores rechazan URLs muy largas.
const MAX_URL: usize = 7_500;

/// Datos para el diagnóstico.
#[derive(Debug, Default)]
pub struct Diagnostics {
    pub macos: String,
    pub window: String,
    pub font: String,
    pub pane: String,
    pub last_command: Option<String>,
    pub unhandled: Vec<(String, u32)>,
    pub screen: String,
}

fn body(d: &Diagnostics, screen: &str) -> String {
    let mut b = String::new();
    b.push_str("## Qué pasó\n\n<!-- Describe el fallo: qué esperabas y qué ocurrió. -->\n\n");
    b.push_str("## Cómo reproducirlo\n\n1. \n\n");
    b.push_str("---\n\n### Diagnóstico\n\n");
    b.push_str(
        "_Generado automáticamente. Revisa que no incluya nada privado antes de enviarlo._\n\n",
    );
    b.push_str(&format!(
        "- **Versión:** {PRODUCT_NAME} {} (commit {})\n",
        env!("CARGO_PKG_VERSION"),
        env!("RT_GIT_COMMIT")
    ));
    b.push_str(&format!("- **macOS:** {}\n", d.macos));
    b.push_str(&format!("- **Ventana:** {}\n", d.window));
    b.push_str(&format!("- **Fuente:** {}\n", d.font));
    b.push_str(&format!("- **Panel:** {}\n", d.pane));
    if let Some(cmd) = &d.last_command {
        b.push_str(&format!("- **Último comando:** {cmd}\n"));
    }
    b.push_str("\n<details><summary>Secuencias no soportadas recientes</summary>\n\n```\n");
    if d.unhandled.is_empty() {
        b.push_str("(ninguna)\n");
    }
    for (s, n) in &d.unhandled {
        if *n > 1 {
            b.push_str(&format!("{s}  (×{n})\n"));
        } else {
            b.push_str(&format!("{s}\n"));
        }
    }
    b.push_str("```\n\n</details>\n\n");
    b.push_str("<details><summary>Texto visible del panel</summary>\n\n```text\n");
    b.push_str(screen);
    if !screen.ends_with('\n') {
        b.push('\n');
    }
    b.push_str("```\n\n</details>\n");
    b
}

/// Codifica para la consulta de una URL.
fn encode(s: &str) -> String {
    let mut out = String::with_capacity(s.len() * 3);
    for b in s.bytes() {
        match b {
            b'A'..=b'Z' | b'a'..=b'z' | b'0'..=b'9' | b'-' | b'_' | b'.' | b'~' => {
                out.push(b as char)
            }
            _ => out.push_str(&format!("%{b:02X}")),
        }
    }
    out
}

/// URL de la issue nueva. Si es demasiado larga, recorta el texto del panel
/// quedándose con las últimas líneas (lo más reciente).
pub fn issue_url(d: &Diagnostics) -> String {
    let title = encode("Fallo: ");
    let mut lines: Vec<&str> = d.screen.lines().collect();
    let mut dropped = 0;
    loop {
        let screen = if dropped > 0 {
            format!(
                "[… {dropped} líneas anteriores recortadas]\n{}",
                lines.join("\n")
            )
        } else {
            lines.join("\n")
        };
        let url = format!(
            "{NEW_ISSUE_URL}?title={title}&body={}",
            encode(&body(d, &screen))
        );
        if url.len() <= MAX_URL || lines.is_empty() {
            return url;
        }
        lines.remove(0);
        dropped += 1;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn decode(s: &str) -> String {
        let bytes = s.as_bytes();
        let mut out = Vec::new();
        let mut i = 0;
        while i < bytes.len() {
            if bytes[i] == b'%' {
                out.push(u8::from_str_radix(&s[i + 1..i + 3], 16).unwrap());
                i += 3;
            } else {
                out.push(bytes[i]);
                i += 1;
            }
        }
        String::from_utf8(out).unwrap()
    }

    fn diag(screen: &str) -> Diagnostics {
        Diagnostics {
            macos: "27.0".into(),
            window: "1 pestaña".into(),
            font: "SF Mono 13 pt".into(),
            pane: "100×30".into(),
            last_command: Some("`false` → código 1".into()),
            unhandled: vec![("OSC 1337".into(), 3)],
            screen: screen.into(),
        }
    }

    #[test]
    fn url_contains_the_diagnostics() {
        let url = issue_url(&diag("$ false\n$ ñ"));
        assert!(url.starts_with(NEW_ISSUE_URL));
        let body = decode(url.split("&body=").nth(1).unwrap());
        assert!(body.contains("**macOS:** 27.0"));
        assert!(body.contains("`false` → código 1"));
        assert!(body.contains("OSC 1337  (×3)"));
        assert!(body.contains("$ false\n$ ñ"));
        assert!(body.contains(env!("RT_GIT_COMMIT")));
    }

    #[test]
    fn long_screens_are_trimmed_keeping_the_end() {
        let screen: Vec<String> = (0..2000).map(|i| format!("línea número {i}")).collect();
        let url = issue_url(&diag(&screen.join("\n")));
        assert!(url.len() <= MAX_URL);
        let body = decode(url.split("&body=").nth(1).unwrap());
        assert!(body.contains("línea número 1999"));
        assert!(!body.contains("línea número 0\n"));
        assert!(body.contains("líneas anteriores recortadas"));
    }
}
