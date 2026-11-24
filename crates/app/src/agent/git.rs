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

// --- Descartar el worktree y abrir un PR ---

/// ¿`workdir` es un worktree que Claude Code creó dentro de `project`
/// (`<project>/.claude/worktrees/<nombre>`)? Solo esos se pueden borrar desde
/// la app: nunca el proyecto ni una carpeta cualquiera.
pub fn is_discardable_worktree(project: &Path, workdir: &Path) -> bool {
    let (Ok(project), Ok(workdir)) = (project.canonicalize(), workdir.canonicalize()) else {
        return false;
    };
    workdir != project
        && workdir
            .strip_prefix(project.join(".claude").join("worktrees"))
            .is_ok_and(|rest| rest.components().count() == 1)
}

/// Quita el worktree (los cambios sin commit se pierden). **La rama se
/// conserva**: sus commits siguen siendo recuperables.
pub fn remove_worktree(project: &Path, workdir: &Path) -> Result<(), String> {
    if !is_discardable_worktree(project, workdir) {
        return Err("Solo se pueden descartar los worktrees de `.claude/worktrees`.".into());
    }
    let out = git(
        project,
        &["worktree", "remove", "--force", &workdir.to_string_lossy()],
    )
    .map_err(|e| e.to_string())?;
    if out.status.success() {
        Ok(())
    } else {
        Err(String::from_utf8_lossy(&out.stderr).trim().to_owned())
    }
}

/// Archivos con cambios sin commit (incluidos los sin seguimiento).
pub fn dirty_count(repo: &Path) -> usize {
    git(repo, &["status", "--porcelain"])
        .ok()
        .map(|o| String::from_utf8_lossy(&o.stdout).lines().count())
        .unwrap_or(0)
}

/// Commits de `HEAD` que no están en `base`.
pub fn commits_ahead(repo: &Path, base: &str) -> Option<usize> {
    stdout_ok(repo, &["rev-list", "--count", &format!("{base}..HEAD")])?
        .parse()
        .ok()
}

/// `origin/main` → `main`.
fn short_branch(base: &str) -> &str {
    base.strip_prefix("origin/").unwrap_or(base)
}

/// Lo que se ejecutaría para abrir el PR; se enseña antes de confirmar.
#[derive(Debug, Clone, PartialEq)]
pub struct PrPlan {
    pub branch: String,
    pub base: String,
    pub commits: usize,
    /// Archivos con cambios sin commit: no entrarían en el PR.
    pub dirty: usize,
}

impl PrPlan {
    pub fn commands(&self) -> [String; 2] {
        [
            format!("git push -u origin {}", self.branch),
            format!(
                "gh pr create --fill --head {} --base {}",
                self.branch, self.base
            ),
        ]
    }
}

pub fn pr_plan(workdir: &Path, base: Option<&str>) -> Result<PrPlan, String> {
    let branch = current_branch(workdir).ok_or("El agente no está en una rama.")?;
    let base = short_branch(base.ok_or("No se encontró la rama base.")?).to_owned();
    if branch == base {
        return Err(format!(
            "El agente trabaja directamente en `{base}`: no hay rama propia de la que abrir un PR."
        ));
    }
    let commits = commits_ahead(workdir, &base)
        .or_else(|| commits_ahead(workdir, &format!("origin/{base}")))
        .ok_or("No se pudo comparar con la rama base.")?;
    if commits == 0 {
        return Err(
            "La rama no tiene commits nuevos respecto a la base. Pídele a Claude que haga commit \
             de sus cambios."
                .into(),
        );
    }
    Ok(PrPlan {
        branch,
        base,
        commits,
        dirty: dirty_count(workdir),
    })
}

/// El ejecutable de `gh` (una app lanzada desde el Finder no tiene el PATH del usuario).
pub fn find_gh() -> Option<std::path::PathBuf> {
    ["/opt/homebrew/bin/gh", "/usr/local/bin/gh", "/usr/bin/gh"]
        .iter()
        .map(std::path::PathBuf::from)
        .find(|p| p.is_file())
}

/// Sube la rama y abre el PR. Devuelve la URL.
pub fn open_pr(workdir: &Path, plan: &PrPlan) -> Result<String, String> {
    let gh =
        find_gh().ok_or("No se encontró `gh` (GitHub CLI). Instálalo con `brew install gh`.")?;
    let push = git(workdir, &["push", "-u", "origin", &plan.branch]).map_err(|e| e.to_string())?;
    if !push.status.success() {
        return Err(format!(
            "git push falló:\n{}",
            String::from_utf8_lossy(&push.stderr).trim()
        ));
    }
    let out = Command::new(gh)
        .current_dir(workdir)
        .args([
            "pr",
            "create",
            "--fill",
            "--head",
            &plan.branch,
            "--base",
            &plan.base,
        ])
        .env("GH_PROMPT_DISABLED", "1")
        .stdin(Stdio::null())
        .output()
        .map_err(|e| e.to_string())?;
    if !out.status.success() {
        return Err(format!(
            "gh pr create falló:\n{}",
            String::from_utf8_lossy(&out.stderr).trim()
        ));
    }
    Ok(String::from_utf8_lossy(&out.stdout)
        .lines()
        .rev()
        .find(|l| l.starts_with("http"))
        .unwrap_or("")
        .to_owned())
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

    #[test]
    fn only_claude_worktrees_can_be_discarded() {
        let d = repo("discard");
        let wt = d.join(".claude").join("worktrees").join("tarea");
        run(
            &d,
            &[
                "worktree",
                "add",
                "-q",
                "-b",
                "worktree-tarea",
                &wt.to_string_lossy(),
            ],
        );
        assert!(is_discardable_worktree(&d, &wt));
        assert!(!is_discardable_worktree(&d, &d));
        assert!(!is_discardable_worktree(&d, &d.join(".claude")));
        assert!(!is_discardable_worktree(&d, Path::new("/tmp")));
        assert!(!is_discardable_worktree(&d, &d.join("no-existe")));
        // Una carpeta cualquiera dentro del proyecto tampoco.
        fs::create_dir_all(d.join("src")).unwrap();
        assert!(!is_discardable_worktree(&d, &d.join("src")));
        assert!(remove_worktree(&d, &d).is_err());

        fs::write(wt.join("sin-commit.txt"), "x").unwrap();
        remove_worktree(&d, &wt).unwrap();
        assert!(!wt.exists());
        // La rama se conserva.
        assert!(
            git(
                &d,
                &["rev-parse", "--verify", "-q", "refs/heads/worktree-tarea"]
            )
            .unwrap()
            .status
            .success()
        );
        let _ = fs::remove_dir_all(&d);
    }

    #[test]
    fn pr_plan_needs_an_own_branch_with_commits() {
        let d = repo("prplan");
        // En la rama base: no hay PR posible.
        assert!(
            pr_plan(&d, Some("main"))
                .unwrap_err()
                .contains("directamente")
        );
        run(&d, &["checkout", "-q", "-b", "worktree-x"]);
        assert!(
            pr_plan(&d, Some("main"))
                .unwrap_err()
                .contains("sin commits")
                || pr_plan(&d, Some("main"))
                    .unwrap_err()
                    .contains("no tiene commits")
        );
        fs::write(d.join("nuevo.txt"), "hola\n").unwrap();
        run(&d, &["add", "."]);
        run(&d, &["commit", "-q", "-m", "nuevo"]);
        fs::write(d.join("sin-commit.txt"), "x").unwrap();
        let plan = pr_plan(&d, Some("origin/main")).unwrap();
        assert_eq!(
            plan,
            PrPlan {
                branch: "worktree-x".into(),
                base: "main".into(),
                commits: 1,
                dirty: 1
            }
        );
        assert_eq!(plan.commands()[0], "git push -u origin worktree-x");
        assert!(plan.commands()[1].ends_with("--head worktree-x --base main"));
        assert!(pr_plan(&d, None).is_err());
        let _ = fs::remove_dir_all(&d);
    }
}
