//! La pantalla de Review de un agente: abrir, comentar y enviar.
//!
//! El diff se calcula en un hilo (un `git diff` grande no debe congelar la
//! interfaz) y llega al hilo principal con el selector `reviewReady`.

use std::path::{Path, PathBuf};
use std::sync::Mutex;
use std::sync::atomic::{AtomicBool, Ordering::SeqCst};
use std::time::Duration;

use objc2::rc::{Retained, Weak};
use objc2::{MainThreadOnly, Message, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSBackingStoreType, NSTextField, NSWindow,
    NSWindowStyleMask,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::agent::broker::AgentKey;
use crate::agent::diff::DiffFile;
use crate::agent::git;
use crate::agent::model::Status;
use crate::agent::review::{self, Review, ReviewError, Side};
use crate::review_ui::{FileRow, ReviewActions, ReviewRoot};
use crate::view::TerminalView;

pub struct ReviewState {
    window: Retained<NSWindow>,
    root: Retained<ReviewRoot>,
    agent: AgentKey,
    title: String,
    review: Review,
    selected: usize,
    branch: Option<String>,
    base: Option<String>,
}

struct Loaded {
    files: Vec<DiffFile>,
    branch: Option<String>,
    base: Option<String>,
}

#[derive(Clone, Copy, PartialEq)]
enum Purpose {
    /// Abrir (o refrescar) la ventana de Review.
    Review,
    /// Solo actualizar el inspector y la barra de terminado.
    Inspector,
}

type LoadResult = (AgentKey, Purpose, Result<Loaded, String>);

static RESULTS: Mutex<Vec<LoadResult>> = Mutex::new(Vec::new());

/// Resultado de subir la rama y abrir el PR (URL o error).
static PR_RESULTS: Mutex<Vec<(AgentKey, Result<String, String>)>> = Mutex::new(Vec::new());
static PR_RUNNING: AtomicBool = AtomicBool::new(false);

fn load(workdir: &Path) -> Result<Loaded, String> {
    let base = git::detect_base_branch(workdir);
    let files = git::collect_diff(workdir, base.as_deref()).map_err(|e| e.to_string())?;
    Ok(Loaded {
        files,
        branch: git::current_branch(workdir),
        base,
    })
}

impl TerminalView {
    pub(crate) fn info(&self, message: &str, info: &str) {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(message));
        alert.setInformativeText(&NSString::from_str(info));
        alert.addButtonWithTitle(&NSString::from_str("Aceptar"));
        alert.runModal();
    }

    /// ⌘⇧R: abre la revisión del agente del panel con el foco.
    pub fn open_review(&self) {
        let focused = self.focused_pane();
        let found = {
            let hub = self.agents().hub_ref();
            hub.session_of_pane(focused)
                .map(|s| (s.key.clone(), s.title.clone()))
        };
        let Some((key, _)) = found else {
            self.info(
                "Esta pestaña no es un agente",
                "La revisión de cambios está disponible en las pestañas de agentes de Claude Code.",
            );
            return;
        };
        let Some(workdir) = self.agents().workdir_of(&key) else {
            return;
        };
        let Some(waker) = self.waker_arc() else {
            return;
        };
        let spawned = std::thread::Builder::new()
            .name("review-diff".into())
            .spawn(move || {
                let result = load(&workdir);
                RESULTS.lock().unwrap().push((key, Purpose::Review, result));
                waker.on_main_thread(sel!(reviewReady));
            });
        if spawned.is_err() {
            self.info(
                "No se pudo abrir la revisión",
                "No se pudo crear el hilo que calcula el diff.",
            );
        }
    }

    /// Calcula en segundo plano qué cambió un agente (inspector y barra de terminado).
    pub fn load_changes_async(&self, key: &str) {
        let Some(workdir) = self.agents().workdir_of(key) else {
            return;
        };
        let Some(waker) = self.waker_arc() else {
            return;
        };
        let key = key.to_owned();
        let _ = std::thread::Builder::new()
            .name("changes-diff".into())
            .spawn(move || {
                let result = load(&workdir);
                RESULTS
                    .lock()
                    .unwrap()
                    .push((key, Purpose::Inspector, result));
                waker.on_main_thread(sel!(reviewReady));
            });
    }

    /// Pregunta con dos botones; `true` si se pulsa el primero.
    fn confirm(&self, message: &str, info: &str, yes: &str, destructive: bool) -> bool {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(message));
        alert.setInformativeText(&NSString::from_str(info));
        let first = alert.addButtonWithTitle(&NSString::from_str(yes));
        first.setHasDestructiveAction(destructive);
        alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
        alert.runModal() == NSAlertFirstButtonReturn
    }

    fn focused_agent_key(&self) -> Option<AgentKey> {
        let focused = self.focused_pane();
        let hub = self.agents().hub_ref();
        hub.session_of_pane(focused).map(|s| s.key.clone())
    }

    fn review_agent_key(&self) -> Option<AgentKey> {
        self.agents()
            .review
            .borrow()
            .as_ref()
            .map(|s| s.agent.clone())
    }

    /// «Abrir PR…» de la barra de terminado.
    pub fn open_pr_focused(&self) {
        match self.focused_agent_key() {
            Some(key) => self.open_pr_for(key),
            None => self.info(
                "Esta pestaña no es un agente",
                "Abrir un PR está disponible en las pestañas de agentes de Claude Code.",
            ),
        }
    }

    /// «Abrir PR…» de la cabecera de Review.
    pub fn open_pr_review(&self) {
        if let Some(key) = self.review_agent_key() {
            self.open_pr_for(key);
        }
    }

    /// Confirma con las órdenes exactas y sube la rama y abre el PR en un hilo.
    fn open_pr_for(&self, key: AgentKey) {
        if PR_RUNNING.load(SeqCst) {
            self.info("Ya se está abriendo un PR", "Espera a que termine.");
            return;
        }
        let Some(workdir) = self.agents().workdir_of(&key) else {
            self.info(
                "No se puede abrir el PR",
                "Este agente no tiene carpeta de trabajo.",
            );
            return;
        };
        let base = git::detect_base_branch(&workdir);
        let plan = match git::pr_plan(&workdir, base.as_deref()) {
            Ok(p) => p,
            Err(e) => {
                self.info("No se puede abrir el PR", &e);
                return;
            }
        };
        let [push, create] = plan.commands();
        let mut info = format!(
            "Se ejecutará en {}:\n\n    {push}\n    {create}\n\n{} commit{} de «{}» hacia «{}».",
            workdir.display(),
            plan.commits,
            if plan.commits == 1 { "" } else { "s" },
            plan.branch,
            plan.base
        );
        if plan.dirty > 0 {
            info.push_str(&format!(
                "\n\nAtención: {} archivo{} con cambios sin commit NO entrarán en el PR.",
                plan.dirty,
                if plan.dirty == 1 { "" } else { "s" }
            ));
        }
        if !self.confirm(
            &format!("¿Subir «{}» y abrir un PR?", plan.branch),
            &info,
            "Subir y abrir PR",
            false,
        ) {
            return;
        }
        let Some(waker) = self.waker_arc() else {
            return;
        };
        PR_RUNNING.store(true, SeqCst);
        let spawned = std::thread::Builder::new()
            .name("open-pr".into())
            .spawn(move || {
                let result = git::open_pr(&workdir, &plan);
                PR_RESULTS.lock().unwrap().push((key, result));
                PR_RUNNING.store(false, SeqCst);
                waker.on_main_thread(sel!(reviewReady));
            });
        if spawned.is_err() {
            PR_RUNNING.store(false, SeqCst);
            self.info("No se pudo abrir el PR", "No se pudo crear el hilo.");
        }
    }

    /// El resultado de abrir un PR (hilo principal).
    fn pr_finished(&self, result: Result<String, String>) {
        match result {
            Ok(url) if url.starts_with("https://") => {
                let alert = NSAlert::new(self.mtm());
                alert.setMessageText(&NSString::from_str("PR abierto"));
                alert.setInformativeText(&NSString::from_str(&url));
                alert.addButtonWithTitle(&NSString::from_str("Abrir en el navegador"));
                alert.addButtonWithTitle(&NSString::from_str("Cerrar"));
                if alert.runModal() == NSAlertFirstButtonReturn {
                    let _ = std::process::Command::new("/usr/bin/open")
                        .arg(&url)
                        .status();
                }
            }
            Ok(_) => self.info(
                "PR abierto",
                "La rama se subió y `gh` terminó bien, pero no devolvió la dirección del PR.",
            ),
            Err(e) => self.info("No se pudo abrir el PR", &e),
        }
    }

    /// «Descartar worktree…» de la cabecera de Review.
    pub fn discard_worktree_review(&self) {
        let Some(key) = self.review_agent_key() else {
            return;
        };
        let (Some(workdir), Some(project)) = (
            self.agents().workdir_of(&key),
            self.agents().project_of(&key),
        ) else {
            return;
        };
        let busy = matches!(
            self.agents().status_of(&key),
            Some(Status::Working | Status::Running | Status::NeedsYou { .. } | Status::PlanReady)
        );
        if busy {
            self.info(
                "El agente sigue activo",
                "Espera a que termine su turno antes de descartar su worktree.",
            );
            return;
        }
        if !git::is_discardable_worktree(&project, &workdir) {
            self.info(
                "No se puede descartar",
                "Solo se pueden descartar los worktrees que Claude Code crea en `.claude/worktrees`. \
                 Este agente trabaja en la carpeta del proyecto.",
            );
            return;
        }
        let dirty = git::dirty_count(&workdir);
        let branch = git::current_branch(&workdir);
        let mut info = format!("Se eliminará la carpeta {}.", workdir.display());
        if dirty > 0 {
            info.push_str(&format!(
                "\n\n{dirty} archivo{} con cambios sin commit se perderá{}.",
                if dirty == 1 { "" } else { "s" },
                if dirty == 1 { "" } else { "n" }
            ));
        }
        if let Some(b) = &branch {
            info.push_str(&format!(
                "\n\nLa rama «{b}» se conserva con sus commits (se borra a mano con `git branch -D {b}`)."
            ));
        }
        if !self.confirm(
            "¿Descartar este worktree?",
            &info,
            "Descartar worktree",
            true,
        ) {
            return;
        }
        match git::remove_worktree(&project, &workdir) {
            Ok(()) => {
                if let Some(state) = self.agents().review.borrow_mut().take() {
                    state.window.orderOut(None);
                }
                self.info(
                    "Worktree descartado",
                    "La carpeta se eliminó. Cierra la pestaña del agente cuando quieras.",
                );
            }
            Err(e) => self.info("No se pudo descartar el worktree", &e),
        }
    }

    /// Hay diffs calculados (hilo principal).
    pub fn review_ready(&self) {
        let prs = std::mem::take(&mut *PR_RESULTS.lock().unwrap());
        for (_, result) in prs {
            self.pr_finished(result);
        }
        let results = std::mem::take(&mut *RESULTS.lock().unwrap());
        for (key, purpose, result) in results {
            match (purpose, result) {
                (Purpose::Inspector, Ok(l)) => {
                    let rows = l
                        .files
                        .iter()
                        .map(|f| crate::agent_ui::ChangeRow {
                            tag: f.status.tag(),
                            path: f.path.clone(),
                            added: f.added,
                            removed: f.removed,
                        })
                        .collect();
                    self.set_agent_changes(&key, rows, l.branch);
                }
                // Sin repositorio: el inspector sigue en «Calculando…».
                (Purpose::Inspector, Err(_)) => {}
                (Purpose::Review, result) => self.review_loaded(key, result),
            }
        }
    }

    /// El diff de la ventana de Review está calculado.
    fn review_loaded(&self, key: AgentKey, result: Result<Loaded, String>) {
        let loaded = match result {
            Ok(l) => l,
            Err(e) => {
                self.info("No se pudo calcular el diff", &e);
                return;
            }
        };
        let title = self.agents().title_of(&key).unwrap_or_default();
        let mut slot = self.agents().review.borrow_mut();
        match slot.as_mut() {
            Some(state) if state.agent == key => {
                let lost = state.review.refresh(loaded.files);
                state.branch = loaded.branch;
                state.base = loaded.base;
                state.selected = state
                    .selected
                    .min(state.review.files.len().saturating_sub(1));
                drop(slot);
                if lost > 0 {
                    self.info(
                        "Algunos comentarios se quitaron",
                        &format!(
                            "{lost} comentario(s) apuntaban a líneas que ya no están en el diff."
                        ),
                    );
                }
            }
            _ => {
                let mut review = Review::new(loaded.files);
                if cfg!(debug_assertions) && std::env::var_os("RT_REVIEW_DEMO").is_some() {
                    seed_demo_comments(&mut review);
                }
                let selected = if cfg!(debug_assertions) {
                    std::env::var("RT_REVIEW_SELECT")
                        .ok()
                        .and_then(|s| s.parse().ok())
                        .unwrap_or(0)
                } else {
                    0
                };
                let (window, root) = self.build_review_window(&title);
                *slot = Some(ReviewState {
                    window,
                    root,
                    agent: key,
                    title,
                    review,
                    selected,
                    branch: loaded.branch,
                    base: loaded.base,
                });
                drop(slot);
            }
        }
        self.review_render();
        if cfg!(debug_assertions) && std::env::var_os("RT_REVIEW_AUTOSEND").is_some() {
            self.review_send();
            return;
        }
        if let Some(state) = self.agents().review.borrow().as_ref() {
            #[allow(deprecated)]
            NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
            state.window.makeKeyAndOrderFront(None);
        }
    }

    fn build_review_window(&self, title: &str) -> (Retained<NSWindow>, Retained<ReviewRoot>) {
        let mtm = self.mtm();
        let me: Retained<TerminalView> = self.retain();
        let root = ReviewRoot::new(
            mtm,
            me.as_ref(),
            ReviewActions {
                refresh: sel!(reviewRefresh:),
                send: sel!(reviewSend:),
                pr: sel!(reviewOpenPR:),
                discard: sel!(reviewDiscard:),
            },
        );
        let frame = NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1180.0, 740.0));
        root.setFrame(frame);
        // SAFETY: ventana propia, que no se libera al cerrarse.
        let window = unsafe {
            NSWindow::initWithContentRect_styleMask_backing_defer(
                NSWindow::alloc(mtm),
                frame,
                NSWindowStyleMask::Titled
                    | NSWindowStyleMask::Closable
                    | NSWindowStyleMask::Miniaturizable
                    | NSWindowStyleMask::Resizable,
                NSBackingStoreType::Buffered,
                false,
            )
        };
        // SAFETY: la ventana no es de un controlador de ventana.
        unsafe { window.setReleasedWhenClosed(false) };
        window.setTitle(&NSString::from_str(&format!("Review · {title}")));
        window.setMinSize(NSSize::new(760.0, 420.0));
        window.setContentView(Some(&root));
        window.center();

        let weak: Weak<TerminalView> = Weak::from_retained(&me);
        let w = weak.clone();
        root.ivars_pub().list.set_on_select(move |i| {
            if let Some(v) = w.load() {
                v.review_select(i);
            }
        });
        let w = weak.clone();
        root.ivars_pub().diff.set_on_add(move |side, line| {
            if let Some(v) = w.load() {
                v.review_add(side, line);
            }
        });
        let w = weak;
        root.ivars_pub().diff.set_on_delete(move |index| {
            if let Some(v) = w.load() {
                v.review_delete(index);
            }
        });
        (window, root)
    }

    /// El tema cambió: si hay una ventana de Review abierta, se vuelve a pintar.
    pub(crate) fn review_refresh_look(&self) {
        if self.agents().review.borrow().is_some() {
            self.review_render();
        }
    }

    /// Vuelca el modelo a las vistas.
    fn review_render(&self) {
        let slot = self.agents().review.borrow();
        let Some(state) = slot.as_ref() else { return };
        let r = &state.review;
        let files: Vec<FileRow> = r
            .files
            .iter()
            .enumerate()
            .map(|(i, f)| FileRow {
                path: f.path.clone(),
                status: f.status,
                comments: r.count_for(&f.path),
                selected: i == state.selected,
            })
            .collect();
        let iv = state.root.ivars_pub();
        iv.list.set_rows(files);
        let rows = r
            .files
            .get(state.selected)
            .map(|f| review::rows(f, r))
            .unwrap_or_default();
        iv.diff.set_rows(rows, state.root.diff_width());

        let (added, removed, count) = r.totals();
        let branch = state.branch.as_deref().unwrap_or("(sin rama)");
        let base = state.base.as_deref().unwrap_or("HEAD");
        let n = r.comments().len();
        let subtitle = format!(
            "{branch} → {base} · +{added} −{removed} · {count} {}",
            if count == 1 { "archivo" } else { "archivos" }
        );
        let send_title = match n {
            0 => "Enviar comentarios a Claude".to_owned(),
            1 => "Enviar 1 comentario a Claude".to_owned(),
            n => format!("Enviar {n} comentarios a Claude"),
        };
        state
            .root
            .set_header(&state.title, &subtitle, &send_title, n > 0);

        if let Some(dir) = std::env::var_os("RT_SNAPSHOT_DIR") {
            let dir = PathBuf::from(dir);
            let _ = std::fs::create_dir_all(&dir);
            crate::agent_ui::snapshot_png(&state.root, &dir.join("review.png").to_string_lossy());
        }
    }

    pub fn review_select(&self, index: usize) {
        if let Some(state) = self.agents().review.borrow_mut().as_mut()
            && index < state.review.files.len()
        {
            state.selected = index;
        }
        self.review_render();
    }

    /// Clic en el «+» de una línea: pide el comentario y lo añade al lote.
    pub fn review_add(&self, side: Side, line: u32) {
        let file = {
            let slot = self.agents().review.borrow();
            let Some(state) = slot.as_ref() else { return };
            let Some(f) = state.review.files.get(state.selected) else {
                return;
            };
            f.path.clone()
        };
        let Some(text) = self.ask_comment(&format!("{file}:{line}")) else {
            return;
        };
        let result = {
            let mut slot = self.agents().review.borrow_mut();
            let Some(state) = slot.as_mut() else { return };
            state.review.add_comment(&file, side, line, &text)
        };
        match result {
            Ok(()) => self.review_render(),
            Err(e) => self.info("No se pudo añadir el comentario", &e.to_string()),
        }
    }

    fn ask_comment(&self, location: &str) -> Option<String> {
        let mtm = self.mtm();
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str(&format!("Comentario en {location}")));
        alert.setInformativeText(&NSString::from_str(
            "Se añade a la revisión; no se envía hasta que pulses «Enviar».",
        ));
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(420.0, 60.0)),
        );
        field.setUsesSingleLineMode(false);
        alert.setAccessoryView(Some(&field));
        alert.addButtonWithTitle(&NSString::from_str("Añadir a la revisión"));
        alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
        alert.window().setInitialFirstResponder(Some(&field));
        (alert.runModal() == NSAlertFirstButtonReturn).then(|| field.stringValue().to_string())
    }

    pub fn review_delete(&self, index: usize) {
        if let Some(state) = self.agents().review.borrow_mut().as_mut() {
            state.review.remove_comment(index);
        }
        self.review_render();
    }

    pub fn review_refresh(&self) {
        let key = self
            .agents()
            .review
            .borrow()
            .as_ref()
            .map(|s| s.agent.clone());
        let Some(key) = key else { return };
        let Some(workdir) = self.agents().workdir_of(&key) else {
            return;
        };
        let Some(waker) = self.waker_arc() else {
            return;
        };
        let _ = std::thread::Builder::new()
            .name("review-diff".into())
            .spawn(move || {
                let result = load(&workdir);
                RESULTS.lock().unwrap().push((key, Purpose::Review, result));
                waker.on_main_thread(sel!(reviewReady));
            });
    }

    /// Envía todos los comentarios al agente como un único mensaje.
    pub fn review_send(&self) {
        let (key, message) = {
            let slot = self.agents().review.borrow();
            let Some(state) = slot.as_ref() else { return };
            let Some(m) = state.review.message() else {
                return;
            };
            (state.agent.clone(), m)
        };
        // Solo con el turno terminado: con un permiso en pantalla, el texto lo contestaría.
        let status = self.agents().status_of(&key);
        match status.as_ref().map(review::can_send) {
            Some(Ok(())) => {}
            Some(Err(why)) => {
                self.info("No se pueden enviar los comentarios ahora", why);
                return;
            }
            None => {
                self.info(
                    "No se pueden enviar los comentarios",
                    "La sesión ya no existe.",
                );
                return;
            }
        }
        let Some(session) = self.agents().pane_of(&key).and_then(|p| self.session(p)) else {
            self.info(
                "No se pueden enviar los comentarios",
                "El panel del agente ya no existe.",
            );
            return;
        };
        let bracketed = session.term.lock().unwrap().modes().bracketed_paste;
        session.write(&review::pty_paste(&message, bracketed));
        // Intro aparte y un instante después, como lo espera la interfaz de Claude Code.
        let for_submit = session.clone();
        let _ = std::thread::Builder::new()
            .name("review-submit".into())
            .spawn(move || {
                std::thread::sleep(Duration::from_millis(400));
                for_submit.write(review::SUBMIT);
            });

        if let Some(state) = self.agents().review.borrow_mut().as_mut() {
            state.review.clear_comments();
            state.window.orderOut(None);
        }
        self.review_render();
        self.focus_agent_and_activate(&key);
    }
}

/// Solo en depuración (`RT_REVIEW_DEMO`): dos comentarios de ejemplo para
/// poder ver cómo se pintan sin interactuar.
fn seed_demo_comments(review: &mut Review) {
    let Some(file) = review.files.first() else {
        return;
    };
    let path = file.path.clone();
    let lines: Vec<(Side, u32)> = file
        .hunks
        .iter()
        .flat_map(|h| &h.lines)
        .filter_map(|l| match (l.kind, l.old, l.new) {
            (crate::agent::diff::LineKind::Del, Some(o), _) => Some((Side::Old, o)),
            (_, _, Some(n)) => Some((Side::New, n)),
            _ => None,
        })
        .collect();
    for (i, (side, n)) in lines
        .into_iter()
        .filter(|(_, n)| *n > 0)
        .take(2)
        .enumerate()
    {
        let text = if i == 0 {
            "Maneja el caso vacío.\nY el nulo."
        } else {
            "¿Seguro que se puede borrar?"
        };
        let r: Result<(), ReviewError> = review.add_comment(&path, side, n, text);
        let _ = r;
    }
}
