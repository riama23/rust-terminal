//! Lo que la pantalla de Review necesita de git, ejecutando el `git` del
//! sistema (sin shell: los argumentos van como tales).
//!
//! **El diff es el árbol de trabajo contra la base común**, no
//! `base...HEAD` como dice el spec: Claude Code normalmente no hace commit y
//! ese diff saldría vacío justo cuando hay trabajo que revisar. Incluye los
//! cambios sin commit y los archivos nuevos sin seguimiento.

use std::io;
use std::path::Path;
use std::process::{Command, Output, Stdio};

use super::diff::{self, DiffFile};

/// Archivos sin seguimiento que se incluyen como máximo, y su tamaño máximo.
const MAX_UNTRACKED: usize = 200;
const MAX_UNTRACKED_BYTES: u64 = 1024 * 1024;

pub fn git(repo: &Path, args: &[&str]) -> io::Result<Output> {
    Command::new("git")
        .arg("-C")
        .arg(repo)
        .args(args)
        // Sin bloqueos opcionales (el agente está escribiendo) ni preguntas.
        .env("GIT_OPTIONAL_LOCKS", "0")
        .env("GIT_TERMINAL_PROMPT", "0")
        .env("LC_ALL", "C")
        .stdin(Stdio::null())
        .output()
}

/// Salida estándar de un comando que tiene que salir bien.
fn stdout_ok(repo: &Path, args: &[&str]) -> Option<String> {
    let out = git(repo, args).ok()?;
    out.status
        .success()
        .then(|| String::from_utf8_lossy(&out.stdout).trim().to_owned())
        .filter(|s| !s.is_empty())
}

/// Rama actual (`None` en HEAD desacoplado o fuera de un repositorio).
pub fn current_branch(repo: &Path) -> Option<String> {
    stdout_ok(repo, &["symbolic-ref", "--short", "-q", "HEAD"])
}

/// La rama contra la que se compara: `origin/HEAD` si existe y, si no, `main`
/// o `master` locales.
pub fn detect_base_branch(repo: &Path) -> Option<String> {
    if let Some(head) = stdout_ok(
        repo,
        &["symbolic-ref", "--short", "-q", "refs/remotes/origin/HEAD"],
    ) {
        return Some(head);
    }
    ["main", "master"].into_iter().find_map(|b| {
        git(
            repo,
            &["rev-parse", "--verify", "-q", &format!("refs/heads/{b}")],
        )
        .ok()
        .filter(|o| o.status.success())
        .map(|_| b.to_owned())
    })
}

/// Ancestro común de `HEAD` y `base`.
pub fn merge_base(repo: &Path, base: &str) -> Option<String> {
    stdout_ok(repo, &["merge-base", "HEAD", base])
}

/// Cambios del árbol de trabajo contra la base común con `base` (o contra
/// `HEAD` si no hay base). Incluye archivos nuevos sin seguimiento.
pub fn collect_diff(repo: &Path, base: Option<&str>) -> io::Result<Vec<DiffFile>> {
    let from = base
        .and_then(|b| merge_base(repo, b))
        .unwrap_or_else(|| "HEAD".to_owned());
    let tracked = git(
        repo,
        &[
            "diff",
            "--no-color",
            "--no-ext-diff",
            "--find-renames",
            "-U3",
            &from,
            "--",
        ],
    )?;
    if !tracked.status.success() {
        return Err(io::Error::other(
            String::from_utf8_lossy(&tracked.stderr).trim().to_owned(),
        ));
    }
    let mut files = diff::parse(&String::from_utf8_lossy(&tracked.stdout));

    let listing = git(repo, &["ls-files", "--others", "--exclude-standard", "-z"])?;
    let untracked = String::from_utf8_lossy(&listing.stdout);
    for rel in untracked
        .split('\0')
        .filter(|s| !s.is_empty())
        .take(MAX_UNTRACKED)
    {
        let path = repo.join(rel);
        if !path.is_file() || std::fs::metadata(&path).is_ok_and(|m| m.len() > MAX_UNTRACKED_BYTES)
        {
            continue;
        }
        // `--no-index` sale con código 1 cuando hay diferencias: es lo normal.
        let out = git(
            repo,
            &[
                "diff",
                "--no-index",
                "--no-color",
                "-U3",
                "--",
                "/dev/null",
                rel,
            ],
        )?;
        files.extend(diff::parse(&String::from_utf8_lossy(&out.stdout)));
    }
    files.sort_by(|a, b| a.path.cmp(&b.path));
    Ok(files)
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::agent::diff::FileStatus;
    use std::fs;
    use std::path::PathBuf;

    fn run(repo: &Path, args: &[&str]) {
        let out = Command::new("git")
            .arg("-C")
            .arg(repo)
            .args([
                "-c",
                "user.name=Test",
                "-c",
                "user.email=t@t",
                "-c",
                "commit.gpgsign=false",
            ])
            .args(args)
            .output()
            .unwrap();
        assert!(
            out.status.success(),
            "git {args:?}: {}",
            String::from_utf8_lossy(&out.stderr)
        );
    }

    fn repo(name: &str) -> PathBuf {
        let d = std::env::temp_dir().join(format!("rt-git-{name}-{}", std::process::id()));
        let _ = fs::remove_dir_all(&d);
        fs::create_dir_all(&d).unwrap();
        run(&d, &["init", "-q", "-b", "main"]);
        fs::write(d.join("a.txt"), "1\n2\n3\n").unwrap();
        fs::write(d.join("viejo.txt"), "adiós\n").unwrap();
        run(&d, &["add", "."]);
        run(&d, &["commit", "-q", "-m", "inicial"]);
        d
    }

    #[test]
    fn detecta_la_rama_actual_y_la_base() {
        let d = repo("ramas");
        assert_eq!(current_branch(&d).as_deref(), Some("main"));
        assert_eq!(detect_base_branch(&d).as_deref(), Some("main"));
        run(&d, &["checkout", "-q", "-b", "worktree-x"]);
        assert_eq!(current_branch(&d).as_deref(), Some("worktree-x"));
        assert_eq!(detect_base_branch(&d).as_deref(), Some("main"));
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn fuera_de_un_repositorio_no_hay_nada() {
        let d = std::env::temp_dir().join(format!("rt-git-fuera-{}", std::process::id()));
        fs::create_dir_all(&d).unwrap();
        assert_eq!(current_branch(&d), None);
        assert_eq!(detect_base_branch(&d), None);
        assert!(collect_diff(&d, None).is_err());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn el_diff_incluye_lo_que_aun_no_tiene_commit() {
        let d = repo("sincommit");
        fs::write(d.join("a.txt"), "1\ndos\n3\n4\n").unwrap();
        fs::remove_file(d.join("viejo.txt")).unwrap();
        fs::write(d.join("nuevo.txt"), "hola\nmundo\n").unwrap();
        let files = collect_diff(&d, Some("main")).unwrap();
        let by = |p: &str| {
            files
                .iter()
                .find(|f| f.path == p)
                .unwrap_or_else(|| panic!("falta {p}: {files:?}"))
        };
        assert_eq!(by("a.txt").status, FileStatus::Modified);
        assert_eq!((by("a.txt").added, by("a.txt").removed), (2, 1));
        assert_eq!(by("viejo.txt").status, FileStatus::Deleted);
        assert_eq!(
            (by("nuevo.txt").status, by("nuevo.txt").added),
            (FileStatus::Added, 2)
        );
        assert_eq!(files.len(), 3);
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn compara_contra_la_base_comun_y_suma_commits_y_cambios_sueltos() {
        let d = repo("base");
        run(&d, &["checkout", "-q", "-b", "worktree-x"]);
        fs::write(d.join("a.txt"), "1\n2\n3\ncommit\n").unwrap();
        run(&d, &["commit", "-q", "-am", "en la rama"]);
        // Mientras tanto `main` avanza: no debe aparecer como cambio nuestro.
        run(&d, &["checkout", "-q", "main"]);
        fs::write(d.join("solo-main.txt"), "x\n").unwrap();
        run(&d, &["add", "."]);
        run(&d, &["commit", "-q", "-m", "main avanza"]);
        run(&d, &["checkout", "-q", "worktree-x"]);
        fs::write(d.join("suelto.txt"), "s\n").unwrap();

        let files = collect_diff(&d, Some("main")).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert_eq!(paths, ["a.txt", "suelto.txt"], "sin solo-main.txt");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn sin_base_compara_contra_head() {
        let d = repo("head");
        fs::write(d.join("a.txt"), "cambiado\n").unwrap();
        let files = collect_diff(&d, None).unwrap();
        assert_eq!(files.len(), 1);
        assert_eq!(files[0].path, "a.txt");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn sin_cambios_no_hay_archivos_y_se_respeta_gitignore() {
        let d = repo("limpio");
        assert!(collect_diff(&d, Some("main")).unwrap().is_empty());
        fs::write(d.join(".gitignore"), "ignorado.log\n").unwrap();
        run(&d, &["add", ".gitignore"]);
        run(&d, &["commit", "-q", "-m", "ignore"]);
        fs::write(d.join("ignorado.log"), "ruido\n").unwrap();
        assert!(collect_diff(&d, Some("main")).unwrap().is_empty());
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn archivos_sin_seguimiento_enormes_o_con_nombres_raros() {
        let d = repo("raros");
        fs::write(
            d.join("grande.bin"),
            vec![b'x'; (MAX_UNTRACKED_BYTES + 1) as usize],
        )
        .unwrap();
        fs::write(d.join("con espacio ñ.txt"), "hola\n").unwrap();
        fs::create_dir_all(d.join("sub dir")).unwrap();
        fs::write(d.join("sub dir/x.txt"), "x\n").unwrap();
        let files = collect_diff(&d, Some("main")).unwrap();
        let paths: Vec<&str> = files.iter().map(|f| f.path.as_str()).collect();
        assert!(!paths.contains(&"grande.bin"), "{paths:?}");
        assert!(paths.contains(&"con espacio ñ.txt"), "{paths:?}");
        assert!(paths.contains(&"sub dir/x.txt"), "{paths:?}");
        fs::remove_dir_all(d).unwrap();
    }

    #[test]
    fn los_argumentos_no_pasan_por_una_shell() {
        let d = repo("inyeccion");
        let marca = d.join("pwned");
        let raro = format!("$(touch {})", marca.display());
        assert_eq!(merge_base(&d, &raro), None);
        assert!(!marca.exists());
        fs::remove_dir_all(d).unwrap();
    }
}
