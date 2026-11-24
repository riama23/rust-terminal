//! Reconocer si un proceso es Claude Code (macOS, sin dependencias).
//!
//! `proc_name` no sirve: con el instalador nativo el ejecutable real es
//! `~/.local/share/claude/versions/2.1.331` y el nombre sale como `2.1.331`.
//! Se mira `argv[0]` (lo que escribió el usuario: `claude`) y la ruta real.

use std::ffi::{c_int, c_uint, c_void};

unsafe extern "C" {
    fn sysctl(
        name: *mut c_int,
        namelen: c_uint,
        oldp: *mut c_void,
        oldlenp: *mut usize,
        newp: *mut c_void,
        newlen: usize,
    ) -> c_int;
}

const CTL_KERN: c_int = 1;
const KERN_PROCARGS2: c_int = 49;

/// `KERN_PROCARGS2` en bruto de un proceso del mismo usuario.
fn procargs_raw(pid: u32) -> Option<Vec<u8>> {
    let mut mib = [CTL_KERN, KERN_PROCARGS2, pid as c_int];
    let mut len: usize = 0;
    // SAFETY: primera llamada, solo pide el tamaño.
    let rc = unsafe {
        sysctl(
            mib.as_mut_ptr(),
            3,
            std::ptr::null_mut(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 || len == 0 {
        return None;
    }
    let mut buf = vec![0u8; len];
    // SAFETY: `buf` tiene `len` bytes y `len` se actualiza con lo escrito.
    let rc = unsafe {
        sysctl(
            mib.as_mut_ptr(),
            3,
            buf.as_mut_ptr().cast(),
            &mut len,
            std::ptr::null_mut(),
            0,
        )
    };
    if rc != 0 {
        return None;
    }
    buf.truncate(len);
    Some(buf)
}

/// `(ruta del ejecutable, argv[0])` de un proceso del mismo usuario.
fn exec_and_argv0(pid: u32) -> Option<(String, String)> {
    parse_procargs(&procargs_raw(pid)?)
}

/// Los argumentos completos (`argv`) de un proceso del mismo usuario.
pub fn args_of(pid: u32) -> Option<Vec<String>> {
    parse_argv(&procargs_raw(pid)?)
}

/// `argv` completo de un `KERN_PROCARGS2`: `argc`, la ruta, ceros de relleno y
/// luego `argc` cadenas terminadas en cero.
fn parse_argv(buf: &[u8]) -> Option<Vec<String>> {
    let argc = i32::from_ne_bytes(buf.get(..4)?.try_into().ok()?);
    let argc = usize::try_from(argc).ok()?.min(256);
    let rest = buf.get(4..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    let rest = &rest[end..];
    let start = rest.iter().position(|b| *b != 0)?;
    let mut args = Vec::new();
    for part in rest[start..].split(|b| *b == 0).take(argc) {
        args.push(String::from_utf8_lossy(part).into_owned());
    }
    (args.len() == argc).then_some(args)
}

/// El destino de un `ssh` en marcha (`user@host` → `host`), si el proceso
/// `pid` es `ssh`.
pub fn ssh_host_of(pid: u32) -> Option<String> {
    let args = args_of(pid)?;
    ssh_destination(&args)
}

/// Opciones de `ssh` que llevan un valor aparte.
const SSH_VALUE_FLAGS: &str = "BbcDEeFIiJLlmOoPpQRSWw";

/// El host de una orden `ssh …` (`args[0]` es el programa). Entiende
/// opciones sueltas y agrupadas (`-tt`, `-p22`), `--`, `usuario@host` y
/// `ssh://usuario@host:puerto`. `None` si no es `ssh` o no hay destino.
pub fn ssh_destination(args: &[String]) -> Option<String> {
    let program = args.first()?.rsplit('/').next()?;
    if program != "ssh" {
        return None;
    }
    let mut it = args[1..].iter();
    let mut target: Option<&str> = None;
    while let Some(a) = it.next() {
        if a == "--" {
            target = it.next().map(String::as_str);
            break;
        }
        let Some(flags) = a.strip_prefix('-').filter(|f| !f.is_empty()) else {
            target = Some(a);
            break;
        };
        // Un grupo de opciones: la primera con valor se lo lleva (el resto del
        // grupo o, si es la última letra, el argumento siguiente).
        let chars: Vec<char> = flags.chars().collect();
        for (i, c) in chars.iter().enumerate() {
            if SSH_VALUE_FLAGS.contains(*c) {
                if i + 1 == chars.len() {
                    it.next();
                }
                break;
            }
        }
    }
    host_of_destination(target?)
}

fn host_of_destination(dest: &str) -> Option<String> {
    let mut d = dest;
    let url = d.strip_prefix("ssh://");
    if let Some(u) = url {
        d = u;
    }
    // Sin el usuario (hasta la última `@` antes de la ruta).
    if let Some((_, host)) = d.rsplit_once('@') {
        d = host;
    }
    let d = d.split('/').next().unwrap_or(d);
    let host = if let Some(rest) = d.strip_prefix('[') {
        // IPv6 entre corchetes.
        rest.split(']').next().unwrap_or(rest)
    } else if url.is_some() {
        d.split(':').next().unwrap_or(d)
    } else {
        d
    };
    (!host.is_empty()).then(|| host.to_owned())
}

/// ¿Coincide `host` con alguno de los patrones? `*` vale por cualquier
/// secuencia y `?` por una letra; sin distinguir mayúsculas.
pub fn host_matches(host: &str, patterns: &[String]) -> bool {
    patterns
        .iter()
        .any(|p| glob(&p.trim().to_lowercase(), &host.to_lowercase()))
}

fn glob(pattern: &str, text: &str) -> bool {
    let (p, t): (Vec<char>, Vec<char>) = (pattern.chars().collect(), text.chars().collect());
    if p.is_empty() {
        return false;
    }
    fn go(p: &[char], t: &[char]) -> bool {
        match p.split_first() {
            None => t.is_empty(),
            Some(('*', rest)) => (0..=t.len()).any(|i| go(rest, &t[i..])),
            Some(('?', rest)) => !t.is_empty() && go(rest, &t[1..]),
            Some((c, rest)) => t.first() == Some(c) && go(rest, &t[1..]),
        }
    }
    go(&p, &t)
}

/// Formato de `KERN_PROCARGS2`: `argc` (i32), la ruta, ceros de relleno y los
/// argumentos, todos terminados en cero.
fn parse_procargs(buf: &[u8]) -> Option<(String, String)> {
    let rest = buf.get(4..)?;
    let end = rest.iter().position(|b| *b == 0)?;
    let exec = String::from_utf8_lossy(&rest[..end]).into_owned();
    let rest = &rest[end..];
    let start = rest.iter().position(|b| *b != 0)?;
    let rest = &rest[start..];
    let end = rest.iter().position(|b| *b == 0).unwrap_or(rest.len());
    Some((exec, String::from_utf8_lossy(&rest[..end]).into_owned()))
}

fn basename(path: &str) -> &str {
    path.rsplit('/').next().unwrap_or(path)
}

/// ¿Es Claude Code? `argv[0]` o la ruta se llaman `claude`, o el ejecutable
/// está en `…/claude/versions/<versión>`.
pub fn looks_like_claude(exec: &str, argv0: &str) -> bool {
    basename(argv0) == "claude" || basename(exec) == "claude" || exec.contains("/claude/versions/")
}

/// ¿El proceso `pid` es Claude Code? `false` si no se puede saber.
pub fn is_claude(pid: u32) -> bool {
    exec_and_argv0(pid).is_some_and(|(exec, argv0)| looks_like_claude(&exec, &argv0))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn recognizes_native_install_and_symlink() {
        let exec = "/Users/u/.local/share/claude/versions/2.1.331";
        assert!(looks_like_claude(exec, "claude"));
        assert!(looks_like_claude(exec, "/Users/u/.local/bin/claude"));
        assert!(looks_like_claude(exec, "2.1.331"));
        assert!(looks_like_claude("/usr/local/bin/claude", "node"));
        assert!(!looks_like_claude("/bin/zsh", "-zsh"));
        assert!(!looks_like_claude("/usr/bin/vim", "vim"));
        assert!(!looks_like_claude("/bin/sleep", "sleep"));
    }

    #[test]
    fn parses_procargs() {
        let mut buf = 2i32.to_ne_bytes().to_vec();
        buf.extend(b"/bin/sleep\0\0\0\0sleep\x00100\0");
        assert_eq!(
            parse_procargs(&buf),
            Some(("/bin/sleep".into(), "sleep".into()))
        );
        assert_eq!(parse_procargs(&[1, 0]), None);
    }

    #[test]
    fn reads_a_real_process() {
        let mut child = std::process::Command::new("/bin/sleep")
            .arg("5")
            .spawn()
            .unwrap();
        let (exec, argv0) = exec_and_argv0(child.id()).expect("sysctl");
        let _ = child.kill();
        let _ = child.wait();
        assert!(exec.ends_with("/sleep"), "{exec}");
        assert_eq!(argv0, "/bin/sleep");
        assert!(!is_claude(std::process::id()));
    }

    fn args(s: &str) -> Vec<String> {
        s.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn finds_the_host_of_an_ssh_command() {
        let host = |s: &str| ssh_destination(&args(s));
        assert_eq!(host("ssh prod-db"), Some("prod-db".into()));
        assert_eq!(
            host("/usr/bin/ssh me@web1.example.com"),
            Some("web1.example.com".into())
        );
        assert_eq!(host("ssh -p 2222 -i ~/.ssh/k me@host"), Some("host".into()));
        assert_eq!(host("ssh -p2222 host"), Some("host".into()));
        assert_eq!(
            host("ssh -tt -o StrictHostKeyChecking=no host cmd"),
            Some("host".into())
        );
        assert_eq!(host("ssh -NfL 8080:localhost:80 jump"), Some("jump".into()));
        assert_eq!(host("ssh -v -- host"), Some("host".into()));
        assert_eq!(host("ssh ssh://me@host:2200/"), Some("host".into()));
        assert_eq!(host("ssh me@[::1]"), Some("::1".into()));
        assert_eq!(host("ssh -J bastion target"), Some("target".into()));
        // No son ssh, o no hay destino.
        assert_eq!(host("scp a b"), None);
        assert_eq!(host("ssh"), None);
        assert_eq!(host("ssh -v"), None);
        assert_eq!(host("sshd -D"), None);
    }

    #[test]
    fn hosts_match_glob_patterns_ignoring_case() {
        let pats: Vec<String> = ["prod-*", "*.prod.example.com", "db1"]
            .map(String::from)
            .to_vec();
        assert!(host_matches("prod-web", &pats));
        assert!(host_matches("PROD-WEB", &pats));
        assert!(host_matches("api.prod.example.com", &pats));
        assert!(host_matches("db1", &pats));
        assert!(!host_matches("db10", &pats));
        assert!(!host_matches("staging-web", &pats));
        assert!(!host_matches("prod", &pats), "`prod-*` pide el guion");
        assert!(!host_matches("x", &[]));
        assert!(!host_matches("x", &["".to_owned()]));
        assert!(host_matches("a1", &["a?".to_owned()]));
    }

    #[test]
    fn full_argv_is_parsed() {
        let mut buf = 3i32.to_ne_bytes().to_vec();
        buf.extend(b"/usr/bin/ssh\0\0\0ssh\0-p\0host\0PATH=/bin\0");
        assert_eq!(parse_argv(&buf), Some(args("ssh -p host")));
        // Menos argumentos de los que dice `argc`: no es de fiar.
        let mut bad = 5i32.to_ne_bytes().to_vec();
        bad.extend(b"/x\0\0a\0b\0");
        assert_eq!(parse_argv(&bad), None);
    }
}
