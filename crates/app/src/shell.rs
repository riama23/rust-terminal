//! Integración con la shell: inyección para zsh mediante `ZDOTDIR`.
//!
//! Los scripts van embebidos en el binario y se escriben al arrancar en
//! `~/Library/Caches/rust-terminal/zsh/`. El `.zshenv` inyectado restaura el
//! `ZDOTDIR` del usuario y carga su configuración, así que esta no cambia.
//! `RT_NO_SHELL_INTEGRATION=1` la desactiva (hasta que haya configuración).

use std::path::{Path, PathBuf};

const ZSHENV: &str = include_str!("../shell/zsh/.zshenv");
const ZSH_INTEGRATION: &str = include_str!("../shell/zsh/rt-integration.zsh");

/// Escribe los scripts de zsh en `dir`.
pub fn install_zsh(dir: &Path) -> std::io::Result<()> {
    std::fs::create_dir_all(dir)?;
    std::fs::write(dir.join(".zshenv"), ZSHENV)?;
    std::fs::write(dir.join("rt-integration.zsh"), ZSH_INTEGRATION)?;
    Ok(())
}

/// Variables de entorno para lanzar `zsh_dir` como ZDOTDIR, conservando el
/// ZDOTDIR original del usuario si lo tenía.
pub fn zsh_env(dir: &Path) -> Vec<(String, String)> {
    let mut env = vec![("ZDOTDIR".to_owned(), dir.display().to_string())];
    if let Some(orig) = std::env::var_os("ZDOTDIR") {
        env.push((
            "RT_ORIG_ZDOTDIR".to_owned(),
            orig.to_string_lossy().into_owned(),
        ));
    }
    env
}

/// Prepara la integración para `shell` y devuelve las variables de entorno
/// que hay que añadir. Vacío si la shell no es zsh, si está desactivada o si
/// no se pudieron escribir los scripts.
pub fn integration_env(shell: &str) -> Vec<(String, String)> {
    let is_zsh = Path::new(shell).file_name().is_some_and(|n| n == "zsh");
    if !is_zsh || std::env::var_os("RT_NO_SHELL_INTEGRATION").is_some() {
        return Vec::new();
    }
    let Some(dir) = cache_dir() else {
        return Vec::new();
    };
    match install_zsh(&dir) {
        Ok(()) => zsh_env(&dir),
        Err(e) => {
            eprintln!("integración con zsh desactivada: {e}");
            Vec::new()
        }
    }
}

fn cache_dir() -> Option<PathBuf> {
    let home = std::env::var_os("HOME")?;
    Some(PathBuf::from(home).join("Library/Caches/rust-terminal/zsh"))
}

#[cfg(test)]
mod tests {
    //! Tests herméticos: una zsh real en una PTY, con un HOME temporal (sin
    //! la configuración del usuario) y un `precmd` que imita a Powerlevel10k.

    use std::io::{Read, Write};
    use std::sync::mpsc;
    use std::time::{Duration, Instant};

    use rust_terminal_core::{Pty, PtyWriter, Term};

    use super::*;

    const ZSHRC: &str = r#"
PS1='$ '
RPS1='R'
# Como p10k: un precmd que reescribe el prompt y devuelve 0.
fake_theme() { PS1='$ '; return 0 }
precmd_functions+=(fake_theme)
"#;

    struct Session {
        term: Term,
        writer: PtyWriter,
        rx: mpsc::Receiver<Vec<u8>>,
        _dirs: (tempdir::Dir, tempdir::Dir),
    }

    mod tempdir {
        use std::path::{Path, PathBuf};

        pub struct Dir(PathBuf);

        impl Dir {
            pub fn new(name: &str) -> Dir {
                let path = std::env::temp_dir().join(format!(
                    "rt-test-{name}-{}-{:?}",
                    std::process::id(),
                    std::thread::current().id()
                ));
                let _ = std::fs::remove_dir_all(&path);
                std::fs::create_dir_all(&path).unwrap();
                Dir(path)
            }

            pub fn path(&self) -> &Path {
                &self.0
            }
        }

        impl Drop for Dir {
            fn drop(&mut self) {
                let _ = std::fs::remove_dir_all(&self.0);
            }
        }
    }

    impl Session {
        fn start(zshrc: &str, cols: u16, rows: u16) -> Session {
            let home = tempdir::Dir::new("home");
            let integ = tempdir::Dir::new("integ");
            std::fs::write(home.path().join(".zshrc"), zshrc).unwrap();
            install_zsh(integ.path()).unwrap();
            let mut env = vec![
                ("HOME".to_owned(), home.path().display().to_string()),
                ("ZDOTDIR".to_owned(), integ.path().display().to_string()),
            ];
            env.push(("LANG".to_owned(), "en_US.UTF-8".to_owned()));
            let pty = Pty::spawn("/bin/zsh", &["-i"], env, cols, rows).unwrap();
            let (mut reader, writer, _child) = pty.split().unwrap();
            let (tx, rx) = mpsc::channel();
            std::thread::spawn(move || {
                let mut buf = [0u8; 8192];
                while let Ok(n) = reader.read(&mut buf) {
                    if n == 0 || tx.send(buf[..n].to_vec()).is_err() {
                        break;
                    }
                }
            });
            Session {
                term: Term::new(cols.into(), rows.into()),
                writer,
                rx,
                _dirs: (home, integ),
            }
        }

        /// Procesa la salida hasta que `cond` se cumpla (o falla a los 10 s).
        fn wait(&mut self, what: &str, cond: impl Fn(&Term) -> bool) {
            let deadline = Instant::now() + Duration::from_secs(10);
            while !cond(&self.term) {
                let left = deadline.saturating_duration_since(Instant::now());
                match self.rx.recv_timeout(left) {
                    Ok(bytes) => {
                        self.term.advance(&bytes);
                        let responses = self.term.take_responses();
                        self.writer.write_all(&responses).unwrap();
                    }
                    Err(_) => panic!(
                        "esperando {what}; pantalla:\n{}\nbloques: {:#?}",
                        self.term.snapshot_text(),
                        self.term.blocks()
                    ),
                }
            }
        }

        fn wait_prompt(&mut self, n: usize) {
            self.wait(&format!("prompt {n}"), |t| {
                t.blocks().len() >= n && t.blocks()[n - 1].input.is_some()
            });
        }

        fn run(&mut self, cmd: &str) {
            let n = self.term.blocks().len();
            self.writer.write_all(cmd.as_bytes()).unwrap();
            self.writer.write_all(b"\r").unwrap();
            self.wait_prompt(n + 1);
        }
    }

    #[test]
    fn blocks_with_exit_codes_and_commands() {
        let mut s = Session::start(ZSHRC, 60, 20);
        s.wait_prompt(1);
        s.run("true");
        s.run("false");
        s.run("echo 'hola ñ; ok'");
        let b = s.term.blocks();
        assert_eq!(b[0].command.as_deref(), Some("true"));
        assert_eq!(b[0].exit_code, Some(0));
        // El precmd del tema devuelve 0, pero el bloque ve el `false`.
        assert_eq!(b[1].exit_code, Some(1));
        assert_eq!(b[2].command.as_deref(), Some("echo 'hola ñ; ok'"));
        assert_eq!(s.term.block_output(&b[2]).as_deref(), Some("hola ñ; ok"));
        assert!(b[0].cwd.is_some());
    }

    #[test]
    fn output_without_newline_has_no_prompt_sp_marker() {
        let mut s = Session::start(ZSHRC, 60, 20);
        s.wait_prompt(1);
        s.run("printf foo");
        let b = &s.term.blocks()[0];
        assert_eq!(s.term.block_output(b).as_deref(), Some("foo"));
    }

    #[test]
    fn user_prompt_eol_mark_is_kept_visible() {
        let zshrc = format!("{ZSHRC}\nPROMPT_EOL_MARK='<fin>'\n");
        let mut s = Session::start(&zshrc, 60, 20);
        s.wait_prompt(1);
        s.run("printf foo");
        s.run("printf bar");
        let b = s.term.blocks();
        assert_eq!(s.term.block_output(&b[0]).as_deref(), Some("foo"));
        assert_eq!(s.term.block_output(&b[1]).as_deref(), Some("bar"));
        // La marca del usuario sigue apareciendo tras la salida, una sola vez.
        let text = s.term.snapshot_text();
        assert!(
            text.contains("foo<fin>") && text.contains("bar<fin>"),
            "{text}"
        );
        assert!(!text.contains("<fin><fin>"), "{text}");
    }

    #[test]
    fn without_prompt_sp_exit_code_comes_from_precmd() {
        let zshrc = format!("{ZSHRC}\nunsetopt prompt_sp\n");
        let mut s = Session::start(&zshrc, 60, 20);
        s.wait_prompt(1);
        s.run("false");
        assert_eq!(s.term.blocks()[0].exit_code, Some(1));
    }

    #[test]
    fn empty_enter_creates_no_block() {
        let mut s = Session::start(ZSHRC, 60, 20);
        s.wait_prompt(1);
        s.writer.write_all(b"\r").unwrap();
        // Esperamos a que zsh dibuje el segundo prompt (dos "$" en pantalla).
        s.wait("segundo prompt", |t| {
            t.snapshot_text().matches('$').count() >= 2
        });
        s.run("true");
        let b = s.term.blocks();
        assert_eq!(b.iter().filter(|b| b.exit_code.is_some()).count(), 1);
        assert_eq!(b[0].command.as_deref(), Some("true"));
    }

    #[test]
    fn injected_zshenv_prints_nothing() {
        let home = tempdir::Dir::new("home-quiet");
        let integ = tempdir::Dir::new("integ-quiet");
        install_zsh(integ.path()).unwrap();
        let out = std::process::Command::new("/bin/zsh")
            .args(["-c", "print -n"])
            .env("HOME", home.path())
            .env("ZDOTDIR", integ.path())
            .output()
            .unwrap();
        assert!(out.stdout.is_empty() && out.stderr.is_empty(), "{out:?}");
    }

    #[test]
    fn user_zdotdir_is_restored() {
        let home = tempdir::Dir::new("home-zd");
        let integ = tempdir::Dir::new("integ-zd");
        let user = tempdir::Dir::new("user-zd");
        install_zsh(integ.path()).unwrap();
        std::fs::write(user.path().join(".zshenv"), "export FROM_USER_ZSHENV=1\n").unwrap();
        let out = std::process::Command::new("/bin/zsh")
            .args(["-c", "print -rn -- \"$ZDOTDIR|$FROM_USER_ZSHENV\""])
            .env("HOME", home.path())
            .env("ZDOTDIR", integ.path())
            .env("RT_ORIG_ZDOTDIR", user.path())
            .output()
            .unwrap();
        assert_eq!(
            String::from_utf8_lossy(&out.stdout),
            format!("{}|1", user.path().display())
        );
    }

    #[test]
    fn resize_at_prompt_leaves_a_single_prompt() {
        for zshrc in [ZSHRC, "PS1=$'linea1\\n$ '\n"] {
            let mut s = Session::start(zshrc, 40, 10);
            s.wait_prompt(1);
            s.run("echo antes");
            s.writer.write_all(b"abc").unwrap();
            s.wait("eco de abc", |t| t.snapshot_text().contains("$ abc"));
            for (cols, rows) in [(20u16, 10u16), (50, 10)] {
                s.term.resize(cols.into(), rows.into());
                s.writer.resize(cols, rows).unwrap();
                s.wait("prompt redibujado", |t| t.snapshot_text().contains("$ abc"));
                std::thread::sleep(Duration::from_millis(300));
                while let Ok(bytes) = s.rx.try_recv() {
                    s.term.advance(&bytes);
                }
                let text = s.term.snapshot_text();
                assert_eq!(text.matches("$ abc").count(), 1, "pantalla:\n{text}");
                assert!(text.contains("antes"), "pantalla:\n{text}");
            }
        }
    }
}
