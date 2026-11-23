//! Enlaces en texto plano y utilidades para OSC (base64 de OSC 52).
//!
//! La detección de URL solo reconoce un patrón (`esquema://…`); no deduce
//! nada sobre comandos ni salidas.

const SCHEMES: [&str; 5] = ["https://", "http://", "file://", "ftp://", "mailto:"];

fn is_url_char(c: char) -> bool {
    !c.is_whitespace() && !c.is_control() && !"<>\"'`{}|\\^│".contains(c)
}

/// Rangos `[inicio, fin)` (índices de carácter) de las URL de `chars`.
pub fn find_urls(chars: &[char]) -> Vec<(usize, usize)> {
    let mut out = Vec::new();
    let mut i = 0;
    while i < chars.len() {
        let boundary = i == 0 || !chars[i - 1].is_alphanumeric();
        let scheme = boundary
            .then(|| {
                SCHEMES.iter().find(|s| {
                    s.chars()
                        .enumerate()
                        .all(|(k, c)| chars.get(i + k).is_some_and(|x| x.eq_ignore_ascii_case(&c)))
                })
            })
            .flatten();
        let Some(scheme) = scheme else {
            i += 1;
            continue;
        };
        let body_start = i + scheme.len();
        let mut end = body_start;
        while end < chars.len() && is_url_char(chars[end]) {
            end += 1;
        }
        // Puntuación final que casi nunca es parte de la URL.
        while let Some(&last) = chars[body_start..end].last() {
            let unbalanced = |open: char, close: char| {
                last == close && {
                    let s = &chars[i..end];
                    s.iter().filter(|&&c| c == close).count()
                        > s.iter().filter(|&&c| c == open).count()
                }
            };
            if ".,;:!?".contains(last) || unbalanced('(', ')') || unbalanced('[', ']') {
                end -= 1;
            } else {
                break;
            }
        }
        if end > body_start {
            out.push((i, end));
            i = end;
        } else {
            i = body_start;
        }
    }
    out
}

/// Decodifica base64 estándar (con o sin relleno). `None` si no es válido.
pub fn base64_decode(input: &str) -> Option<Vec<u8>> {
    fn value(c: u8) -> Option<u32> {
        Some(match c {
            b'A'..=b'Z' => c - b'A',
            b'a'..=b'z' => c - b'a' + 26,
            b'0'..=b'9' => c - b'0' + 52,
            b'+' => 62,
            b'/' => 63,
            _ => return None,
        } as u32)
    }
    let bytes: Vec<u8> = input
        .bytes()
        .filter(|b| !b.is_ascii_whitespace() && *b != b'=')
        .collect();
    let mut out = Vec::with_capacity(bytes.len() * 3 / 4);
    for chunk in bytes.chunks(4) {
        let mut acc = 0u32;
        for (k, &b) in chunk.iter().enumerate() {
            acc |= value(b)? << (18 - 6 * k);
        }
        let n = match chunk.len() {
            4 => 3,
            3 => 2,
            2 => 1,
            _ => return None,
        };
        out.extend_from_slice(&acc.to_be_bytes()[1..1 + n]);
    }
    Some(out)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn urls(s: &str) -> Vec<String> {
        let chars: Vec<char> = s.chars().collect();
        find_urls(&chars)
            .into_iter()
            .map(|(a, b)| chars[a..b].iter().collect())
            .collect()
    }

    #[test]
    fn finds_urls_and_trims_punctuation() {
        assert_eq!(
            urls("ver https://example.com/a?b=1, y (http://x.org/p_(1)). fin"),
            ["https://example.com/a?b=1", "http://x.org/p_(1)"]
        );
        assert_eq!(
            urls("file:///tmp/a.txt mailto:yo@ejemplo.es"),
            ["file:///tmp/a.txt", "mailto:yo@ejemplo.es"]
        );
        assert!(urls("nohttps://x y https:// solo").is_empty());
        assert_eq!(urls("\"https://a.b/c\""), ["https://a.b/c"]);
    }

    #[test]
    fn decodes_base64() {
        assert_eq!(base64_decode("aG9sYQ==").unwrap(), b"hola");
        assert_eq!(base64_decode("w7FhbmR1").unwrap(), "ñandu".as_bytes());
        assert_eq!(base64_decode("YQ").unwrap(), b"a");
        assert!(base64_decode("@@@@").is_none());
    }
}
