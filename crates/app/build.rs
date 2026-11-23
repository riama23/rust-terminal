//! Incrusta el commit de git en el binario (para los informes de fallos).

use std::process::Command;

fn main() {
    let git = |args: &[&str]| {
        Command::new("git")
            .args(args)
            .output()
            .ok()
            .filter(|o| o.status.success())
            .map(|o| String::from_utf8_lossy(&o.stdout).trim().to_owned())
    };
    let commit = git(&["rev-parse", "--short", "HEAD"]).unwrap_or_else(|| "desconocido".to_owned());
    let dirty =
        git(&["status", "--porcelain", "--untracked-files=no"]).is_some_and(|s| !s.is_empty());
    let version = if dirty {
        format!("{commit}+cambios")
    } else {
        commit
    };
    println!("cargo:rustc-env=RT_GIT_COMMIT={version}");
    // Volver a ejecutar cuando cambie el commit o el índice.
    println!("cargo:rerun-if-changed=../../.git/HEAD");
    println!("cargo:rerun-if-changed=../../.git/index");
}
