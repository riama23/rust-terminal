//! Lógica pura de la paleta de comandos (⌘⇧P): búsqueda difusa, frecuencia
//! de uso e historial. Sin AppKit y con tests.
//!
//! Tres ámbitos: **Acciones** (las del menú), **Historial** (comandos de los
//! bloques de esta sesión y del historial de zsh) y **Flujos** (comandos con
//! nombre de `workflows.toml`).

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Action,
    History,
    Flow,
}

impl Kind {
    /// Etiqueta de la fila (`ACT` / `CMD`).
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Action => "ACT",
            Kind::History => "CMD",
            Kind::Flow => "FLOW",
        }
    }
}

/// Qué cajas de la paleta se buscan (⇥ las recorre).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    All,
    Actions,
    History,
    Flows,
}

impl Scope {
    pub fn next(self) -> Scope {
        match self {
            Scope::All => Scope::Actions,
            Scope::Actions => Scope::History,
            Scope::History => Scope::Flows,
            Scope::Flows => Scope::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "Todo",
            Scope::Actions => "Acciones",
            Scope::History => "Historial",
            Scope::Flows => "Flujos",
        }
    }

    fn admits(self, kind: Kind) -> bool {
        matches!(
            (self, kind),
            (Scope::All, _)
                | (Scope::Actions, Kind::Action)
                | (Scope::History, Kind::History)
                | (Scope::Flows, Kind::Flow)
        )
    }
}

#[derive(Debug, Clone, PartialEq)]
pub struct Item {
    pub kind: Kind,
    /// Identidad estable (para la frecuencia): `act:<selector>` o `cmd:<texto>`.
    pub key: String,
    pub label: String,
    /// Segunda línea: atajo, o directorio, antigüedad y código de salida.
    pub detail: String,
}

/// Un flujo: un comando con nombre.
#[derive(Debug, Clone, PartialEq)]
pub struct Workflow {
    pub name: String,
    pub command: String,
    pub description: String,
}

/// Lee `workflows.toml`:
///
/// ```toml
/// [[workflow]]
/// name = "Tests del núcleo"
/// command = "cargo test -p rust-terminal-core"
/// description = "opcional"
/// ```
///
/// Devuelve los flujos válidos y un aviso por cada entrada descartada (sin
/// nombre, sin comando o con caracteres de control).
pub fn parse_workflows(text: &str) -> (Vec<Workflow>, Vec<String>) {
    let mut flows = Vec::new();
    let mut problems = Vec::new();
    let value: toml::Table = match text.parse() {
        Ok(v) => v,
        Err(e) => {
            return (
                flows,
                vec![format!("workflows.toml no es TOML válido: {e}")],
            );
        }
    };
    let Some(entries) = value.get("workflow").and_then(|v| v.as_array()) else {
        return (flows, problems);
    };
    for (i, entry) in entries.iter().enumerate() {
        let field = |k: &str| {
            entry
                .get(k)
                .and_then(|v| v.as_str())
                .map(|s| s.trim().to_owned())
                .unwrap_or_default()
        };
        let (name, command, description) = (field("name"), field("command"), field("description"));
        if name.is_empty() || name.chars().count() > 80 || name.chars().any(char::is_control) {
            problems.push(format!(
                "flujo {}: el nombre falta, es muy largo o tiene caracteres de control",
                i + 1
            ));
        } else if command.is_empty() || command.chars().count() > 500 || !insertable(&command) {
            problems.push(format!(
                "flujo «{name}»: el comando falta, es muy largo o tiene caracteres de control"
            ));
        } else if flows.iter().any(|f: &Workflow| f.name == name) {
            problems.push(format!("flujo «{name}»: nombre repetido"));
        } else {
            flows.push(Workflow {
                name,
                command,
                description: description.chars().filter(|c| !c.is_control()).collect(),
            });
        }
    }
    (flows, problems)
}

/// Puntuación de `query` contra `text` (más alta, mejor) o `None` si no
/// aparece como subsecuencia. Sin distinguir mayúsculas ni acentos comunes.
/// Premia las letras seguidas, el inicio de palabra y el principio del texto.
pub fn fuzzy_score(query: &str, text: &str) -> Option<i64> {
    let q: Vec<char> = query
        .chars()
        .filter(|c| !c.is_whitespace())
        .map(fold)
        .collect();
    if q.is_empty() {
        return Some(0);
    }
    let t: Vec<char> = text.chars().map(fold).collect();
    // Se prueba cada posición de la primera letra y gana la mejor: así una
    // coincidencia apretada no la tapa una anterior y dispersa.
    let mut best: Option<i64> = None;
    for start in (0..t.len()).filter(|&i| t[i] == q[0]) {
        if let Some(score) = score_from(&q, &t, start) {
            best = Some(best.map_or(score, |b| b.max(score)));
        }
    }
    best
}

/// Puntuación de la subsecuencia que empieza en `start`, o `None` si no
/// cabe o queda demasiado dispersa (más del doble de la consulta + 6 letras).
fn score_from(q: &[char], t: &[char], start: usize) -> Option<i64> {
    let (mut qi, mut score, mut prev): (usize, i64, Option<usize>) = (0, 0, None);
    let mut last = start;
    for (ti, &c) in t.iter().enumerate().skip(start) {
        if qi < q.len() && c == q[qi] {
            score += 10;
            if prev.is_some_and(|p| p + 1 == ti) {
                score += 15;
            }
            if ti == 0 || !t[ti - 1].is_alphanumeric() {
                score += 12;
            }
            if ti == qi {
                score += 5;
            }
            prev = Some(ti);
            last = ti;
            qi += 1;
        }
    }
    let span = last + 1 - start;
    if qi < q.len() || span > q.len() * 2 + 6 {
        return None;
    }
    // Los textos largos puntúan un poco menos a igualdad de aciertos.
    Some(score - (t.len() as i64 / 8))
}

fn fold(c: char) -> char {
    let lower = c.to_lowercase().next().unwrap_or(c);
    match lower {
        'á' | 'à' | 'ä' | 'â' => 'a',
        'é' | 'è' | 'ë' | 'ê' => 'e',
        'í' | 'ì' | 'ï' | 'î' => 'i',
        'ó' | 'ò' | 'ö' | 'ô' => 'o',
        'ú' | 'ù' | 'ü' | 'û' => 'u',
        'ñ' => 'n',
        other => other,
    }
}

/// Frecuencia de uso: cuántas veces y cuándo se usó cada elemento.
#[derive(Debug, Clone, Default, PartialEq)]
pub struct Frecency {
    entries: HashMap<String, (u32, u64)>,
}

/// Cuántas entradas se guardan como máximo.
const MAX_ENTRIES: usize = 500;

impl Frecency {
    pub fn record(&mut self, key: &str, now: u64) {
        let e = self.entries.entry(key.to_owned()).or_insert((0, now));
        e.0 += 1;
        e.1 = now;
        if self.entries.len() > MAX_ENTRIES {
            // Fuera las menos recientes.
            let mut by_age: Vec<(String, u64)> = self
                .entries
                .iter()
                .map(|(k, (_, t))| (k.clone(), *t))
                .collect();
            by_age.sort_by_key(|(_, t)| *t);
            for (k, _) in by_age.into_iter().take(self.entries.len() - MAX_ENTRIES) {
                self.entries.remove(&k);
            }
        }
    }

    /// Puntos por uso: crece con las veces y decae con los días.
    pub fn bonus(&self, key: &str, now: u64) -> i64 {
        let Some(&(count, last)) = self.entries.get(key) else {
            return 0;
        };
        let days = now.saturating_sub(last) as f64 / 86_400.0;
        let decay = 1.0 / (1.0 + days / 7.0);
        (f64::from(count.min(50)).sqrt() * 20.0 * decay) as i64
    }

    /// Una línea por entrada: `veces<TAB>última<TAB>clave`. Las claves con
    /// saltos de línea no se guardan.
    pub fn serialize(&self) -> String {
        let mut rows: Vec<_> = self
            .entries
            .iter()
            .filter(|(k, _)| !k.contains(['\n', '\r', '\t']))
            .collect();
        rows.sort_by(|a, b| a.0.cmp(b.0));
        rows.iter()
            .map(|(k, (c, t))| format!("{c}\t{t}\t{k}\n"))
            .collect()
    }

    pub fn parse(text: &str) -> Frecency {
        let mut f = Frecency::default();
        for line in text.lines() {
            let mut p = line.splitn(3, '\t');
            if let (Some(c), Some(t), Some(k)) = (p.next(), p.next(), p.next())
                && let (Ok(c), Ok(t)) = (c.parse::<u32>(), t.parse::<u64>())
                && !k.is_empty()
            {
                f.entries.insert(k.to_owned(), (c, t));
            }
        }
        f
    }
}

/// Los elementos que coinciden con `query`, de mejor a peor. Con la consulta
/// vacía manda la frecuencia y, a igualdad, el orden de entrada (los más
/// recientes primero en el historial). `limit` recorta el resultado.
pub fn rank(
    items: &[Item],
    query: &str,
    scope: Scope,
    frecency: &Frecency,
    now: u64,
    limit: usize,
) -> Vec<usize> {
    let mut scored: Vec<(usize, i64)> = items
        .iter()
        .enumerate()
        .filter(|(_, it)| scope.admits(it.kind))
        .filter_map(|(i, it)| {
            let s = fuzzy_score(query, &it.label)?;
            Some((i, s + frecency.bonus(&it.key, now)))
        })
        .collect();
    // Estable: a igual puntuación se respeta el orden original.
    scored.sort_by_key(|(_, s)| std::cmp::Reverse(*s));
    scored.into_iter().take(limit).map(|(i, _)| i).collect()
}

/// `1 min`, `3 h`, `2 d`: hace cuánto.
pub fn age_label(secs: u64) -> String {
    match secs {
        0..=59 => "ahora".to_owned(),
        60..=3599 => format!("hace {} min", secs / 60),
        3600..=86_399 => format!("hace {} h", secs / 3600),
        _ => format!("hace {} d", secs / 86_400),
    }
}

/// Historial de zsh (`: 1700000000:0;comando` con `EXTENDED_HISTORY`, o una
/// línea por comando). Los comandos de varias líneas (`\` al final) se
/// descartan: no se pueden reenviar con seguridad. Devuelve `(hora, comando)`
/// del más reciente al más antiguo, sin repetidos.
pub fn parse_zsh_history(text: &str) -> Vec<(Option<u64>, String)> {
    let mut out: Vec<(Option<u64>, String)> = Vec::new();
    let mut seen = std::collections::HashSet::new();
    let mut lines = text.lines().peekable();
    while let Some(line) = lines.next() {
        let mut ts = None;
        let mut cmd = line;
        if let Some(rest) = line.strip_prefix(": ")
            && let Some((meta, c)) = rest.split_once(';')
        {
            ts = meta.split(':').next().and_then(|t| t.trim().parse().ok());
            cmd = c;
        }
        let mut multiline = false;
        let mut owned = cmd.to_owned();
        while owned.ends_with('\\') {
            multiline = true;
            owned.pop();
            match lines.next() {
                Some(next) => owned.push_str(next),
                None => break,
            }
        }
        let owned = owned.trim().to_owned();
        if multiline || owned.is_empty() || !insertable(&owned) {
            continue;
        }
        // Un comando repetido se queda con su última aparición.
        if !seen.insert(owned.clone()) {
            out.retain(|(_, c)| *c != owned);
        }
        out.push((ts, owned));
    }
    out.reverse();
    out
}

/// ¿Se puede escribir tal cual en la shell? Sin caracteres de control (un
/// salto de línea ejecutaría varios comandos).
pub fn insertable(command: &str) -> bool {
    let c = command.trim();
    !c.is_empty() && !c.chars().any(char::is_control)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn item(kind: Kind, key: &str, label: &str) -> Item {
        Item {
            kind,
            key: key.into(),
            label: label.into(),
            detail: String::new(),
        }
    }

    #[test]
    fn fuzzy_matches_subsequences_and_prefers_tight_ones() {
        assert!(fuzzy_score("npt", "Nueva pestaña").is_some());
        // Letras muy dispersas en un texto largo no cuentan como coincidencia…
        assert_eq!(
            fuzzy_score("marca", "sudo mv composer.phar /usr/local/bin/composer"),
            None
        );
        // …pero una coincidencia apretada más adelante sí.
        assert!(fuzzy_score("ab", "a xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx ab").is_some());
        assert_eq!(fuzzy_score("zz", "Nueva pestaña"), None);
        assert_eq!(fuzzy_score("", "lo que sea"), Some(0));
        // Sin mayúsculas ni acentos.
        assert!(fuzzy_score("pestana", "Nueva PESTAÑA").is_some());
        // Letras seguidas y principio de palabra puntúan más.
        let tight = fuzzy_score("cargo", "cargo test").unwrap();
        let loose = fuzzy_score("cargo", "c_a_r_g_o").unwrap();
        assert!(tight > loose);
        let word = fuzzy_score("pes", "Nueva pestaña").unwrap();
        let mid = fuzzy_score("est", "Nueva pestaña").unwrap();
        assert!(word > mid);
    }

    #[test]
    fn ranking_uses_frecency_and_scope() {
        let items = vec![
            item(Kind::Action, "act:newTab:", "Nueva pestaña"),
            item(Kind::Action, "act:newAgent:", "Nuevo agente…"),
            item(Kind::History, "cmd:cargo test", "cargo test"),
            item(Kind::History, "cmd:cargo build", "cargo build"),
        ];
        let mut f = Frecency::default();
        let now = 1_000_000;
        // Sin uso: respeta el orden original entre empates.
        let r = rank(&items, "nu", Scope::All, &f, now, 10);
        assert_eq!(r, vec![0, 1]);
        // Usar «Nuevo agente» muchas veces lo sube.
        for _ in 0..5 {
            f.record("act:newAgent:", now);
        }
        assert_eq!(rank(&items, "nu", Scope::All, &f, now, 10)[0], 1);
        // El ámbito filtra.
        assert_eq!(
            rank(&items, "cargo", Scope::Actions, &f, now, 10),
            Vec::<usize>::new()
        );
        assert_eq!(
            rank(&items, "cargo", Scope::History, &f, now, 10),
            vec![2, 3]
        );
        // El límite recorta.
        assert_eq!(rank(&items, "", Scope::All, &f, now, 2).len(), 2);
    }

    #[test]
    fn frecency_decays_and_round_trips() {
        let mut f = Frecency::default();
        f.record("a", 0);
        f.record("a", 0);
        let fresh = f.bonus("a", 0);
        let old = f.bonus("a", 60 * 86_400);
        assert!(fresh > old && old >= 0);
        assert_eq!(f.bonus("nunca", 0), 0);
        f.record("cmd:echo hola", 5);
        let back = Frecency::parse(&f.serialize());
        assert_eq!(back, f);
        // Las claves con saltos de línea o tabuladores no se guardan, y la basura se ignora.
        f.record("cmd:a\nb", 5);
        assert!(!f.serialize().contains("a\nb"));
        assert_eq!(Frecency::parse("x\ty\tz\n\n3\t4\t\n").entries.len(), 0);
    }

    #[test]
    fn frecency_keeps_a_bounded_number_of_entries() {
        let mut f = Frecency::default();
        for i in 0..(MAX_ENTRIES as u64 + 50) {
            f.record(&format!("k{i}"), i);
        }
        assert_eq!(f.entries.len(), MAX_ENTRIES);
        assert!(
            f.entries
                .contains_key(&format!("k{}", MAX_ENTRIES as u64 + 49))
        );
        assert!(!f.entries.contains_key("k0"));
    }

    #[test]
    fn zsh_history_is_parsed_newest_first_without_repeats_or_multiline() {
        let text = ": 100:0;ls\n: 200:0;cargo test\n: 300:0;ls\n: 400:0;echo uno \\\n: 401:0;resto\n: 500:0;make\n";
        let h = parse_zsh_history(text);
        let cmds: Vec<&str> = h.iter().map(|(_, c)| c.as_str()).collect();
        assert_eq!(cmds, ["make", "ls", "cargo test"]);
        // `ls` conserva la hora de su última aparición.
        assert_eq!(h[1].0, Some(300));
        // Sin formato extendido también vale.
        assert_eq!(
            parse_zsh_history("a\nb\n")
                .iter()
                .map(|(_, c)| c.as_str())
                .collect::<Vec<_>>(),
            ["b", "a"]
        );
    }

    #[test]
    fn nothing_with_control_characters_is_insertable() {
        assert!(insertable("git status"));
        assert!(insertable("echo ñ; ls"));
        assert!(!insertable(""));
        assert!(!insertable("a\nb"));
        assert!(!insertable("a\u{1b}[31m"));
        assert!(!insertable("a\rb"));
    }

    #[test]
    fn ages_are_readable() {
        assert_eq!(age_label(5), "ahora");
        assert_eq!(age_label(120), "hace 2 min");
        assert_eq!(age_label(7200), "hace 2 h");
        assert_eq!(age_label(3 * 86_400), "hace 3 d");
    }

    #[test]
    fn workflows_are_parsed_and_bad_ones_reported() {
        let text = r#"
[[workflow]]
name = "Tests del núcleo"
command = "cargo test -p rust-terminal-core"
description = "Rápidos"

[[workflow]]
name = "Sin comando"

[[workflow]]
name = "Con salto"
command = "echo a\nrm -rf x"

[[workflow]]
name = "Tests del núcleo"
command = "otro"

[[workflow]]
name = "Limpiar"
command = "cargo clean"
"#;
        let (flows, problems) = parse_workflows(text);
        let names: Vec<&str> = flows.iter().map(|f| f.name.as_str()).collect();
        assert_eq!(names, ["Tests del núcleo", "Limpiar"]);
        assert_eq!(flows[0].description, "Rápidos");
        assert_eq!(problems.len(), 3, "{problems:?}");
        // Sin sección o con TOML roto no hay flujos, y lo segundo avisa.
        assert_eq!(parse_workflows(""), (vec![], vec![]));
        let (none, errs) = parse_workflows("[[workflow");
        assert!(none.is_empty() && errs.len() == 1);
    }

    #[test]
    fn flows_have_their_own_scope() {
        let items = vec![
            item(Kind::Flow, "flow:Tests", "Tests"),
            item(Kind::History, "cmd:cargo test", "cargo test"),
        ];
        let f = Frecency::default();
        assert_eq!(rank(&items, "t", Scope::Flows, &f, 0, 10), vec![0]);
        assert_eq!(rank(&items, "t", Scope::History, &f, 0, 10), vec![1]);
        assert_eq!(Scope::History.next(), Scope::Flows);
        assert_eq!(Scope::Flows.next(), Scope::All);
        assert_eq!(Kind::Flow.tag(), "FLOW");
    }
}
