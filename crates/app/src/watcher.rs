//! Vigila el archivo de configuración con kqueue para recargarlo en caliente.
//!
//! El hilo duerme en `kevent` hasta que cambia el directorio (los editores
//! suelen guardar escribiendo un archivo nuevo y renombrándolo) o el propio
//! archivo. No hay sondeo: en reposo no gasta nada. Solo avisa si el
//! contenido del archivo cambió de verdad.

use std::path::{Path, PathBuf};
use std::time::Duration;

use rustix::event::kqueue::{Event, EventFilter, EventFlags, VnodeEvents, kevent, kqueue};
use rustix::fd::AsRawFd;
use rustix::fs::{Mode, OFlags, open};

/// Contenido actual del archivo (`None` si no existe o no se puede leer).
fn snapshot(path: &Path) -> Option<String> {
    std::fs::read_to_string(path).ok()
}

/// Arranca el hilo vigilante. `on_change` se llama (desde ese hilo) cada vez
/// que el contenido de `path` cambia.
pub fn watch(path: PathBuf, on_change: impl Fn() + Send + 'static) {
    std::thread::Builder::new()
        .name("config-watcher".into())
        .spawn(move || {
            let Some(dir) = path.parent().map(Path::to_path_buf) else {
                return;
            };
            // El directorio tiene que existir para poder vigilarlo.
            let _ = std::fs::create_dir_all(&dir);
            let mut last = snapshot(&path);
            loop {
                if wait_for_change(&dir, &path).is_err() {
                    // Si algo falla (p. ej. el directorio desapareció), se
                    // reintenta más tarde sin consumir CPU de forma continua.
                    std::thread::sleep(Duration::from_secs(5));
                    let _ = std::fs::create_dir_all(&dir);
                }
                // Los editores escriben en varios pasos: se espera a que acaben.
                std::thread::sleep(Duration::from_millis(50));
                let now = snapshot(&path);
                if now != last {
                    last = now;
                    on_change();
                }
            }
        })
        .expect("hilo vigilante de la configuración");
}

/// Bloquea hasta el siguiente evento del directorio o del archivo.
fn wait_for_change(dir: &Path, file: &Path) -> rustix::io::Result<()> {
    let dir_fd = open(
        dir,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )?;
    let file_fd = open(file, OFlags::RDONLY | OFlags::CLOEXEC, Mode::empty()).ok();
    let kq = kqueue()?;
    let flags = EventFlags::ADD | EventFlags::CLEAR;
    let mut changes = vec![Event::new(
        EventFilter::Vnode {
            vnode: dir_fd.as_raw_fd(),
            flags: VnodeEvents::WRITE | VnodeEvents::DELETE | VnodeEvents::RENAME,
        },
        flags,
        std::ptr::null_mut(),
    )];
    if let Some(fd) = &file_fd {
        changes.push(Event::new(
            EventFilter::Vnode {
                vnode: fd.as_raw_fd(),
                flags: VnodeEvents::WRITE
                    | VnodeEvents::EXTEND
                    | VnodeEvents::DELETE
                    | VnodeEvents::RENAME
                    | VnodeEvents::ATTRIBUTES,
            },
            flags,
            std::ptr::null_mut(),
        ));
    }
    let mut events: Vec<Event> = Vec::with_capacity(4);
    // SAFETY: `dir_fd` y `file_fd` viven más que `kq` (se declaran antes y
    // se liberan después).
    unsafe { kevent(&kq, &changes, &mut events, None)? };
    drop(kq);
    Ok(())
}

#[cfg(test)]
mod tests {
    use std::sync::mpsc;

    use super::*;

    #[test]
    fn notices_writes_and_atomic_replacements() {
        let dir = std::env::temp_dir().join(format!("rt-watch-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        std::fs::create_dir_all(&dir).unwrap();
        let file = dir.join("config.toml");
        std::fs::write(&file, "a").unwrap();
        let (tx, rx) = mpsc::channel();
        watch(file.clone(), move || {
            let _ = tx.send(());
        });
        std::thread::sleep(Duration::from_millis(200));

        // Escritura en el sitio.
        std::fs::write(&file, "b").unwrap();
        rx.recv_timeout(Duration::from_secs(5))
            .expect("cambio en el sitio");

        // Guardado atómico: archivo nuevo y renombrado.
        let tmp = dir.join(".config.toml.tmp");
        std::fs::write(&tmp, "c").unwrap();
        std::fs::rename(&tmp, &file).unwrap();
        rx.recv_timeout(Duration::from_secs(5)).expect("renombrado");

        // Un archivo ajeno en el directorio no avisa.
        std::fs::write(dir.join("otro"), "x").unwrap();
        assert!(rx.recv_timeout(Duration::from_millis(500)).is_err());
        let _ = std::fs::remove_dir_all(&dir);
    }
}
