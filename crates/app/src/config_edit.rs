//! Edición de `config.toml` línea a línea, para la ventana de ajustes.
//!
//! Conserva comentarios, orden y el resto del archivo: solo se cambia (o se
//! añade) la línea `clave = valor` de la sección pedida. Puro y con tests.

/// Un texto como cadena TOML básica (`"…"` con escapes).
pub fn toml_string(s: &str) -> String {
    let mut out = String::from("\"");
    for c in s.chars() {
        match c {
            '"' => out.push_str("\\\""),
            '\\' => out.push_str("\\\\"),
            '\n' => out.push_str("\\n"),
            '\r' => out.push_str("\\r"),
            '\t' => out.push_str("\\t"),
            c if c.is_control() => out.push_str(&format!("\\u{:04x}", c as u32)),
            c => out.push(c),
        }
    }
    out.push('"');
    out
}

/// `[nombre]` → `nombre` (con comentario final permitido).
fn section_name(line: &str) -> Option<&str> {
    let t = line.trim();
    if t.starts_with("[[") {
        return Some("\0array");
    }
    let rest = t.strip_prefix('[')?;
    let end = rest.find(']')?;
    Some(rest[..end].trim())
}

fn is_key_line(line: &str, key: &str) -> bool {
    let t = line.trim_start();
    t.strip_prefix(key)
        .is_some_and(|rest| rest.trim_start().starts_with('='))
}

/// Líneas que ocupa el valor que empieza en `lines[i]`: más de una si es un
/// array que se cierra en otra línea.
fn value_span(lines: &[&str], i: usize) -> usize {
    let value = lines[i].split_once('=').map_or("", |(_, v)| v);
    if !value.trim_start().starts_with('[') {
        return 1;
    }
    let mut depth: i32 = 0;
    for (n, l) in lines[i..].iter().enumerate() {
        let part = if n == 0 { value } else { l };
        for c in part.chars() {
            match c {
                '[' => depth += 1,
                ']' => depth -= 1,
                '#' => break,
                _ => {}
            }
        }
        if depth <= 0 {
            return n + 1;
        }
    }
    1
}

/// Pone `section.key = literal` (`literal` ya es TOML válido).
pub fn set_value(text: &str, section: &str, key: &str, literal: &str) -> String {
    edit(text, section, key, Some(literal))
}

/// Quita `section.key` (vuelve al valor por defecto).
pub fn remove_value(text: &str, section: &str, key: &str) -> String {
    edit(text, section, key, None)
}

fn edit(text: &str, section: &str, key: &str, literal: Option<&str>) -> String {
    let lines: Vec<&str> = text.lines().collect();
    let mut out: Vec<String> = Vec::new();
    let mut i = 0;
    let mut in_section = false;
    let mut done = false;
    // Dónde acaba la sección buscada (para añadir la clave al final de ella).
    let mut insert_at: Option<usize> = None;
    while i < lines.len() {
        let line = lines[i];
        if let Some(name) = section_name(line) {
            if in_section && insert_at.is_none() {
                insert_at = Some(out.len());
            }
            in_section = name == section;
            out.push(line.to_owned());
            i += 1;
            continue;
        }
        if in_section && !done && is_key_line(line, key) {
            let span = value_span(&lines, i);
            if let Some(l) = literal {
                out.push(format!("{key} = {l}"));
            }
            done = true;
            i += span;
            continue;
        }
        out.push(line.to_owned());
        i += 1;
    }
    if done || literal.is_none() {
        return join(out, text);
    }
    let entry = format!("{key} = {}", literal.unwrap_or_default());
    match (in_section, insert_at) {
        // La sección existe: la clave va tras su última línea con contenido.
        (true, _) | (false, Some(_)) => {
            let end = insert_at.unwrap_or(out.len());
            let mut pos = end;
            while pos > 0 && out[pos - 1].trim().is_empty() {
                pos -= 1;
            }
            let header_pos = out
                .iter()
                .position(|l| section_name(l) == Some(section))
                .unwrap_or(0);
            let pos = pos.max(header_pos + 1);
            out.insert(pos, entry);
        }
        // No existe: sección nueva al final.
        (false, None) => {
            if out.last().is_some_and(|l| !l.trim().is_empty()) {
                out.push(String::new());
            }
            out.push(format!("[{section}]"));
            out.push(entry);
        }
    }
    join(out, text)
}

fn join(lines: Vec<String>, original: &str) -> String {
    let mut s = lines.join("\n");
    if !s.is_empty() && (original.ends_with('\n') || original.is_empty() || !s.ends_with('\n')) {
        s.push('\n');
    }
    s
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn replaces_an_existing_line_keeping_the_rest() {
        let text =
            "# mi config\n[font]\nsize = 13.0 # grande\nfamily = \"X\"\n\n[window]\npadding = 4\n";
        let out = set_value(text, "font", "size", "15.0");
        assert_eq!(
            out,
            "# mi config\n[font]\nsize = 15.0\nfamily = \"X\"\n\n[window]\npadding = 4\n"
        );
    }

    #[test]
    fn adds_a_key_at_the_end_of_an_existing_section() {
        let text = "[font]\nsize = 13.0\n\n[window]\npadding = 4\n";
        let out = set_value(text, "font", "family", "\"Menlo\"");
        assert_eq!(
            out,
            "[font]\nsize = 13.0\nfamily = \"Menlo\"\n\n[window]\npadding = 4\n"
        );
        // En la última sección también.
        let out = set_value(text, "window", "dim_unfocused", "false");
        assert!(
            out.ends_with("[window]\npadding = 4\ndim_unfocused = false\n"),
            "{out}"
        );
    }

    #[test]
    fn creates_the_section_when_missing() {
        assert_eq!(
            set_value("", "blocks", "visual", "false"),
            "[blocks]\nvisual = false\n"
        );
        assert_eq!(
            set_value("[font]\nsize = 13\n", "blocks", "visual", "false"),
            "[font]\nsize = 13\n\n[blocks]\nvisual = false\n"
        );
    }

    #[test]
    fn a_multiline_array_is_replaced_as_a_whole() {
        let text =
            "[colors]\npalette = [\n  \"#000000\",\n  \"#111111\",\n]\nbackground = \"#101014\"\n";
        let out = set_value(text, "colors", "palette", "[\"#ffffff\"]");
        assert_eq!(
            out,
            "[colors]\npalette = [\"#ffffff\"]\nbackground = \"#101014\"\n"
        );
    }

    #[test]
    fn removing_goes_back_to_the_default() {
        let text = "[font]\nsize = 13.0\nfamily = \"X\"\n";
        assert_eq!(
            remove_value(text, "font", "size"),
            "[font]\nfamily = \"X\"\n"
        );
        assert_eq!(remove_value(text, "font", "nada"), text);
        assert_eq!(remove_value("", "font", "size"), "");
    }

    #[test]
    fn only_the_right_section_is_touched() {
        // Una clave igual en otra sección no se toca.
        let text = "[a]\nkey = 1\n[b]\nkey = 2\n";
        assert_eq!(
            set_value(text, "b", "key", "9"),
            "[a]\nkey = 1\n[b]\nkey = 9\n"
        );
        // Un prefijo no es la clave.
        let text = "[a]\nkeyboard = 1\n";
        assert_eq!(
            set_value(text, "a", "key", "2"),
            "[a]\nkeyboard = 1\nkey = 2\n"
        );
    }

    #[test]
    fn strings_are_escaped_and_results_parse() {
        let lit = toml_string("a \"b\" \\ c\nd");
        let text = set_value("[x]\n", "x", "k", &lit);
        let table: toml::Table = text.parse().unwrap();
        assert_eq!(table["x"]["k"].as_str(), Some("a \"b\" \\ c\nd"));
    }
}
