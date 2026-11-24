//! Lógica pura de la paleta de comandos (⌘⇧P): búsqueda difusa, frecuencia
//! de uso e historial. Sin AppKit y con tests.
//!
//! Dos ámbitos: **Acciones** (las del menú) e **Historial** (comandos de los
//! bloques de esta sesión y del historial de zsh). Los *flujos* del spec
//! todavía no existen.

use std::collections::HashMap;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Kind {
    Action,
    History,
}

impl Kind {
    /// Etiqueta de la fila (`ACT` / `CMD`).
    pub fn tag(self) -> &'static str {
        match self {
            Kind::Action => "ACT",
            Kind::History => "CMD",
        }
    }
}

/// Qué cajas de la paleta se buscan (⇥ las recorre).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Scope {
    All,
    Actions,
    History,
}

impl Scope {
    pub fn next(self) -> Scope {
        match self {
            Scope::All => Scope::Actions,
            Scope::Actions => Scope::History,
            Scope::History => Scope::All,
        }
    }

    pub fn label(self) -> &'static str {
        match self {
            Scope::All => "Todo",
            Scope::Actions => "Acciones",
            Scope::History => "Historial",
        }
    }

    fn admits(self, kind: Kind) -> bool {
        matches!(
            (self, kind),
            (Scope::All, _) | (Scope::Actions, Kind::Action) | (Scope::History, Kind::History)
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
    let (mut qi, mut score, mut prev_match): (usize, i64, Option<usize>) = (0, 0, None);
    for (ti, &c) in t.iter().enumerate() {
        if qi < q.len() && c == q[qi] {
            score += 10;
            if prev_match == Some(ti.wrapping_sub(1)) && ti > 0 {
                score += 15;
            }
            if ti == 0 || !t[ti - 1].is_alphanumeric() {
                score += 12;
            }
            if ti == qi {
                score += 5;
            }
            prev_match = Some(ti);
            qi += 1;
        }
    }
    // Los textos largos puntúan un poco menos a igualdad de aciertos.
    (qi == q.len()).then(|| score - (t.len() as i64 / 8))
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
}
