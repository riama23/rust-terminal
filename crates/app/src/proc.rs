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

/// `(ruta del ejecutable, argv[0])` de un proceso del mismo usuario.
fn exec_and_argv0(pid: u32) -> Option<(String, String)> {
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
    parse_procargs(&buf)
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
}
