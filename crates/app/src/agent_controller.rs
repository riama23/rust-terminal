//! Los agentes de una ventana: lanzarlos, recibir los eventos del broker,
//! pintar la sidebar y la attention bar, y contestar los permisos.
//!
//! El estado vive en [`AgentController`] (un ivar de la `TerminalView`); la
//! lógica pura (estados, orden, reglas) está en `agent::*` y se prueba allí.
//! Aquí solo hay pegamento con AppKit.

use std::cell::RefCell;
use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use objc2::rc::{Retained, Weak};
use objc2::{MainThreadOnly, Message, sel};
use objc2_app_kit::{NSAlert, NSAlertFirstButtonReturn, NSTextField};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};

use crate::agent::broker::{AgentKey, Broker, BrokerEvent, Decision, PermissionRequest};
use crate::agent::hub::Hub;
use crate::agent::launch::{self, LaunchSpec};
use crate::agent::model::Status;
use crate::agent::rules;
use crate::agent_ui::{ATTENTION_BAR_HEIGHT, AttentionBar, SIDEBAR_WIDTH, SidebarRow, SidebarView};
use crate::container::ContainerView;
use crate::palette::dusk;
use crate::view::TerminalView;
use crate::workspace::PaneId;

/// Cuánto espera el broker a que el usuario conteste un permiso antes de
/// devolver "sin decisión" (y dejar el prompt de Claude Code).
const PERMISSION_DEADLINE: Duration = Duration::from_secs(300);

#[derive(Default)]
pub struct AgentController {
    inner: RefCell<Inner>,
}

#[derive(Default)]
struct Inner {
    hub: Hub,
    broker: Option<Broker>,
    queue: Arc<Mutex<Vec<BrokerEvent>>>,
    /// Carpeta privada con los `settings` y scripts de cada agente.
    dir: Option<PathBuf>,
    /// Proyecto desde el que se lanzó cada agente (donde va *Siempre permitir*).
    projects: HashMap<AgentKey, PathBuf>,
    sidebar: Option<Retained<SidebarView>>,
    sidebar_shown: bool,
    bar: Option<Retained<AttentionBar>>,
    bar_shown: bool,
    /// Dentro de `refresh_agent_ui`: tocar el contenedor recoloca la terminal,
    /// que vuelve a pedir un refresco. Sin esta guarda, recursión infinita.
    refreshing: bool,
}

fn status_color(status: &Status) -> [u8; 3] {
    match status {
        Status::NeedsYou { .. } => dusk::ATTENTION,
        Status::Working | Status::PlanReady => dusk::ACCENT,
        Status::Error => dusk::ERROR,
        Status::Done => dusk::OK,
        Status::Ended => dusk::MUTED,
    }
}

/// Las filas de la sidebar para el estado actual.
pub fn sidebar_rows(hub: &Hub, focused: PaneId) -> Vec<SidebarRow> {
    hub.sessions()
        .iter()
        .map(|s| {
            let label = s.status.label();
            SidebarRow {
                key: s.key.clone(),
                title: s.title.clone(),
                icon: s.status.icon(),
                detail: match &s.telemetry.branch {
                    Some(branch) => format!("{branch} · {label}"),
                    None => label,
                },
                color: status_color(&s.status),
                ring: matches!(s.status, Status::NeedsYou { .. }),
                selected: hub.pane_of(&s.key) == Some(focused),
            }
        })
        .collect()
}

impl TerminalView {
    fn container(&self) -> Option<Retained<ContainerView>> {
        // SAFETY: `superview` solo lee el árbol de vistas, desde el hilo principal.
        unsafe { self.superview() }?
            .downcast::<ContainerView>()
            .ok()
    }

    fn alert(&self, message: &str, info: &str) {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(message));
        alert.setInformativeText(&NSString::from_str(info));
        alert.addButtonWithTitle(&NSString::from_str("Aceptar"));
        alert.runModal();
    }

    // --- Lanzar ---

    /// Abre una pestaña nueva con `claude` y registra el agente.
    pub fn launch_agent(&self, spec: LaunchSpec, project: PathBuf) -> Result<(), String> {
        spec.validate().map_err(|e| e.to_string())?;
        let waker = self
            .waker_arc()
            .ok_or("La ventana aún no está lista.".to_owned())?;
        let state = self.agents();

        // Broker y carpeta de archivos, la primera vez.
        let (broker_key, files) = {
            let mut inner = state.inner.borrow_mut();
            if inner.broker.is_none() {
                let queue = inner.queue.clone();
                let broker = Broker::start(PERMISSION_DEADLINE, move |event| {
                    queue.lock().unwrap().push(event);
                    waker.on_main_thread(sel!(agentEvents));
                })
                .map_err(|e| format!("No se pudo arrancar el broker: {e}"))?;
                inner.broker = Some(broker);
                let base = std::env::temp_dir();
                launch::remove_stale_dirs(&base, launch::process_alive);
                inner.dir = Some(launch::agents_dir(&base, std::process::id()));
            }
            let broker = inner.broker.as_ref().expect("recién creado");
            let key = broker.register_agent().map_err(|e| e.to_string())?;
            let home = std::env::var_os("HOME")
                .map(PathBuf::from)
                .unwrap_or_default();
            let user_statusline = launch::find_user_statusline(&home, &project);
            let dir = inner.dir.clone().expect("recién creado");
            let files = launch::write_agent_files(&dir, broker, &key, user_statusline.as_deref())
                .map_err(|e| {
                broker.forget_agent(&key);
                format!("No se pudieron escribir los ajustes del agente: {e}")
            })?;
            (key, files)
        };

        let args = launch::command_args(&spec, &files.settings).map_err(|e| e.to_string())?;
        // Por la shell de login para tener el PATH del usuario (claude suele
        // estar en ~/.local/bin).
        let mut command = vec![
            self.user_shell(),
            "-l".to_owned(),
            "-i".to_owned(),
            "-c".to_owned(),
            "exec claude \"$@\"".to_owned(),
            "claude".to_owned(),
        ];
        command.extend(args);

        let Some(pane) = self.open_agent_tab(command, project.clone()) else {
            self.discard_agent(&broker_key);
            return Err("No se pudo abrir la pestaña del agente.".into());
        };
        if let (Some(path), Some(session)) = (std::env::var_os("RT_DUMP_PANE"), self.session(pane))
        {
            dump_pane_text(session, PathBuf::from(path));
        }
        let title = spec
            .title
            .clone()
            .filter(|t| !t.trim().is_empty())
            .unwrap_or_else(|| first_words(&spec.task, 40));
        {
            let mut inner = state.inner.borrow_mut();
            inner.projects.insert(broker_key.clone(), project);
            inner.hub.add(broker_key, title, pane);
        }
        self.refresh_agent_ui();
        Ok(())
    }

    fn discard_agent(&self, key: &str) {
        let mut inner = self.agents().inner.borrow_mut();
        if let Some(broker) = &inner.broker {
            broker.forget_agent(key);
        }
        if let Some(dir) = &inner.dir {
            launch::remove_agent_files(dir, key);
        }
        inner.projects.remove(key);
        inner.hub.remove(key);
    }

    /// Al salir: da de baja a todos y borra la carpeta con sus tokens.
    pub fn shutdown_agents(&self) {
        let mut inner = self.agents().inner.borrow_mut();
        let keys: Vec<AgentKey> = inner.hub.sessions().iter().map(|s| s.key.clone()).collect();
        if let Some(broker) = &inner.broker {
            for key in &keys {
                broker.forget_agent(key);
            }
        }
        if let Some(dir) = inner.dir.take() {
            let _ = std::fs::remove_dir_all(dir);
        }
    }

    /// Da de baja los agentes cuyo panel ya no existe.
    pub fn reconcile_agents(&self) {
        let gone: Vec<AgentKey> = {
            let inner = self.agents().inner.borrow();
            inner
                .hub
                .sessions()
                .iter()
                .filter(|s| {
                    inner
                        .hub
                        .pane_of(&s.key)
                        .is_none_or(|p| !self.pane_exists(p))
                })
                .map(|s| s.key.clone())
                .collect()
        };
        for key in &gone {
            self.discard_agent(key);
        }
        if !gone.is_empty() {
            self.refresh_agent_ui();
        }
    }

    // --- Eventos del broker ---

    /// Hay eventos pendientes del broker (hilo principal).
    pub fn agent_events(&self) {
        let events = {
            let inner = self.agents().inner.borrow();
            std::mem::take(&mut *inner.queue.lock().unwrap())
        };
        let mut changed = false;
        {
            let mut inner = self.agents().inner.borrow_mut();
            for event in &events {
                changed |= inner.hub.apply(event);
            }
        }
        if changed {
            self.refresh_agent_ui();
        }
    }

    // --- Interfaz ---

    /// Pone la sidebar y la barra según el estado y el panel con el foco.
    pub fn refresh_agent_ui(&self) {
        let state = self.agents();
        if std::mem::replace(&mut state.inner.borrow_mut().refreshing, true) {
            return;
        }
        self.refresh_agent_ui_inner();
        state.inner.borrow_mut().refreshing = false;
        self.snapshot_agent_ui();
        self.auto_decide_for_tests();
    }

    /// `RT_AUTO_DECIDE=allow|deny`: contesta solo la petición pendiente, por la
    /// misma vía que los botones. **Solo en compilaciones debug**: una variable
    /// de entorno que auto-permite no puede existir en el binario de uso.
    fn auto_decide_for_tests(&self) {
        if !cfg!(debug_assertions) {
            return;
        }
        let Some(mode) = std::env::var("RT_AUTO_DECIDE").ok() else {
            return;
        };
        let Some(req) = self.pending_permission() else {
            return;
        };
        match mode.as_str() {
            "allow" => self.decide(&req, Decision::Allow),
            "deny" => self.decide(
                &req,
                Decision::Deny {
                    message: Some("Denegado automáticamente (RT_AUTO_DECIDE).".into()),
                },
            ),
            _ => {}
        }
    }

    /// `RT_SNAPSHOT_DIR=/ruta`: vuelca la sidebar y la barra a PNG tras cada
    /// refresco (verificación sin depender de la pantalla).
    fn snapshot_agent_ui(&self) {
        let Some(dir) = std::env::var_os("RT_SNAPSHOT_DIR") else {
            return;
        };
        let dir = PathBuf::from(dir);
        let _ = std::fs::create_dir_all(&dir);
        let inner = self.agents().inner.borrow();
        if inner.sidebar_shown
            && let Some(s) = &inner.sidebar
        {
            crate::agent_ui::snapshot_png(s, &dir.join("sidebar.png").to_string_lossy());
        }
        if inner.bar_shown
            && let Some(b) = &inner.bar
        {
            crate::agent_ui::snapshot_png(b, &dir.join("bar.png").to_string_lossy());
        }
    }

    fn refresh_agent_ui_inner(&self) {
        let Some(container) = self.container() else {
            return;
        };
        let focused = self.focused_pane();
        let state = self.agents();

        // Sidebar: solo existe mientras haya agentes.
        let (rows, show_sidebar, pending) = {
            let inner = state.inner.borrow();
            (
                sidebar_rows(&inner.hub, focused),
                !inner.hub.is_empty(),
                inner.hub.pending_for_pane(focused).cloned(),
            )
        };
        if show_sidebar {
            let sidebar = self.ensure_sidebar();
            sidebar.set_rows(rows);
            if !std::mem::replace(&mut state.inner.borrow_mut().sidebar_shown, true) {
                container.set_left(Some((&sidebar, SIDEBAR_WIDTH)));
            }
        } else if std::mem::replace(&mut state.inner.borrow_mut().sidebar_shown, false) {
            container.set_left(None);
        }

        // Attention bar: la petición pendiente del panel con el foco.
        match pending {
            Some(req) => {
                let bar = self.ensure_bar();
                let detail = detail_of(&req);
                let can_always = rules::rule_for(&req.tool_name, &req.tool_input).is_some();
                bar.set_request(&req.tool_name, &detail, can_always);
                if !std::mem::replace(&mut state.inner.borrow_mut().bar_shown, true) {
                    container.set_bottom(Some((&bar, ATTENTION_BAR_HEIGHT)));
                }
            }
            None => {
                if std::mem::replace(&mut state.inner.borrow_mut().bar_shown, false) {
                    container.set_bottom(None);
                }
            }
        }
    }

    fn ensure_sidebar(&self) -> Retained<SidebarView> {
        let state = self.agents();
        if let Some(s) = state.inner.borrow().sidebar.clone() {
            return s;
        }
        let sidebar = SidebarView::new(self.mtm());
        let me: Retained<TerminalView> = self.retain();
        let weak: Weak<TerminalView> = Weak::from_retained(&me);
        sidebar.set_on_select(move |key| {
            if let Some(view) = weak.load() {
                view.focus_agent(key);
            }
        });
        state.inner.borrow_mut().sidebar = Some(sidebar.clone());
        sidebar
    }

    fn ensure_bar(&self) -> Retained<AttentionBar> {
        let state = self.agents();
        if let Some(b) = state.inner.borrow().bar.clone() {
            return b;
        }
        let bar = AttentionBar::new(
            self.mtm(),
            self.as_ref(),
            sel!(allowOnce:),
            sel!(alwaysAllow:),
            sel!(denyPermission:),
        );
        state.inner.borrow_mut().bar = Some(bar.clone());
        bar
    }

    /// Clic en una fila de la sidebar: lleva el foco al panel del agente.
    pub fn focus_agent(&self, key: &str) {
        let pane = self.agents().inner.borrow().hub.pane_of(key);
        if let Some(pane) = pane
            && self.focus_pane_by_id(pane)
        {
            if let Some(window) = self.window() {
                window.makeFirstResponder(Some(self));
            }
            self.refresh_agent_ui();
        }
    }

    // --- Permisos ---

    /// La petición pendiente del panel con el foco, si la hay.
    pub fn pending_permission(&self) -> Option<PermissionRequest> {
        let focused = self.focused_pane();
        self.agents()
            .inner
            .borrow()
            .hub
            .pending_for_pane(focused)
            .cloned()
    }

    /// Entrega la decisión al broker y retira la barra.
    fn decide(&self, req: &PermissionRequest, decision: Decision) {
        let state = self.agents();
        let delivered = {
            let inner = state.inner.borrow();
            inner
                .broker
                .as_ref()
                .is_some_and(|b| b.resolve(req.id, decision))
        };
        if delivered {
            state.inner.borrow_mut().hub.resolved(&req.agent, req.id);
        }
        // Si no se entregó (venció o se contestó en la sesión), el evento
        // `PermissionEnded` ya viene de camino y retira la barra.
        self.refresh_agent_ui();
    }

    pub fn allow_once(&self) {
        if let Some(req) = self.pending_permission() {
            self.decide(&req, Decision::Allow);
        }
    }

    /// ⌘⌫ / *Denegar…*: pide un motivo opcional y deniega.
    pub fn deny_with_reason(&self) {
        let Some(req) = self.pending_permission() else {
            return;
        };
        let mtm = self.mtm();
        let alert = NSAlert::new(mtm);
        alert.setMessageText(&NSString::from_str("Denegar esta acción"));
        alert.setInformativeText(&NSString::from_str(
            "Claude recibirá el motivo. Es opcional.",
        ));
        let field = NSTextField::initWithFrame(
            NSTextField::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(380.0, 24.0)),
        );
        alert.setAccessoryView(Some(&field));
        alert.addButtonWithTitle(&NSString::from_str("Denegar"));
        alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
        alert.window().setInitialFirstResponder(Some(&field));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return;
        }
        let text = field.stringValue().to_string();
        let text = text.trim();
        let message = (!text.is_empty()).then(|| text.to_owned());
        self.decide(&req, Decision::Deny { message });
    }

    /// *Siempre permitir*: enseña la regla exacta y el archivo antes de escribir.
    pub fn always_allow(&self) {
        let Some(req) = self.pending_permission() else {
            return;
        };
        let Some(rule) = rules::rule_for(&req.tool_name, &req.tool_input) else {
            return;
        };
        let project = self
            .agents()
            .inner
            .borrow()
            .projects
            .get(&req.agent)
            .cloned()
            .unwrap_or_else(|| PathBuf::from(&req.cwd));
        let path = rules::local_settings_path(&project);

        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str("¿Permitir siempre esta orden?"));
        alert.setInformativeText(&NSString::from_str(&format!(
            "Se añadirá esta regla a permissions.allow:\n\n{rule}\n\nArchivo: {}",
            path.display()
        )));
        alert.addButtonWithTitle(&NSString::from_str("Permitir siempre"));
        alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
        if alert.runModal() != NSAlertFirstButtonReturn {
            return;
        }
        match rules::add_allow_rule(&path, &rule) {
            Ok(_) => self.decide(&req, Decision::Allow),
            Err(e) => self.alert("No se pudo guardar la regla", &e.to_string()),
        }
    }

    /// ⌘N: el formulario de nuevo agente.
    pub fn new_agent_sheet(&self) {
        self.show_launch_error("El formulario todavía no está hecho.");
    }

    /// Muestra un error de lanzamiento.
    pub fn show_launch_error(&self, message: &str) {
        self.alert("No se pudo lanzar el agente", message);
    }
}

/// Lo que se aprueba, en una línea: la orden de `Bash` o el resto de campos.
fn detail_of(req: &PermissionRequest) -> String {
    if let Some(command) = req.tool_input.get("command").and_then(|v| v.as_str()) {
        return command.to_owned();
    }
    if let Some(path) = req.tool_input.get("file_path").and_then(|v| v.as_str()) {
        return path.to_owned();
    }
    if req.tool_input.is_null() {
        String::new()
    } else {
        req.tool_input.to_string()
    }
}

/// `RT_DUMP_PANE=/ruta`: vuelca cada 2 s el texto visible del panel del agente
/// (verificación sin pantalla). El hilo termina cuando el panel se cierra.
fn dump_pane_text(session: Arc<crate::session::Session>, path: PathBuf) {
    use std::sync::atomic::Ordering::SeqCst;
    let _ = std::thread::Builder::new()
        .name("dump-pane".into())
        .spawn(move || {
            while !session.exited.load(SeqCst) {
                let text = {
                    let term = session.term.lock().unwrap();
                    (0..term.grid().rows())
                        .map(|i| {
                            let line: String = term
                                .visible_row(i)
                                .iter()
                                .filter(|c| {
                                    !c.flags.contains(rust_terminal_core::Flags::WIDE_SPACER)
                                })
                                .map(|c| c.ch)
                                .collect();
                            line.trim_end().to_owned()
                        })
                        .collect::<Vec<_>>()
                        .join("\n")
                };
                let _ = std::fs::write(&path, text);
                std::thread::sleep(Duration::from_secs(2));
            }
        });
}

fn first_words(text: &str, max: usize) -> String {
    let line = text.lines().next().unwrap_or("").trim();
    if line.chars().count() <= max {
        return line.to_owned();
    }
    let cut: String = line.chars().take(max.saturating_sub(1)).collect();
    format!("{}…", cut.trim_end())
}

/// Carpeta del proyecto para un agente nuevo: la del panel con el foco o `$HOME`.
pub fn default_project_dir(cwd: Option<PathBuf>) -> PathBuf {
    cwd.filter(|p| Path::new(p).is_dir())
        .or_else(|| std::env::var_os("HOME").map(PathBuf::from))
        .unwrap_or_else(|| PathBuf::from("/"))
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    use crate::agent::broker::Ended;

    fn perm(agent: &str, id: u64, tool: &str) -> BrokerEvent {
        BrokerEvent::Permission(PermissionRequest {
            id,
            agent: agent.into(),
            session_id: "s".into(),
            tool_name: tool.into(),
            tool_input: json!({"command": "pnpm db:migrate"}),
            cwd: "/".into(),
            suggestions: Value::Null,
        })
    }

    #[test]
    fn las_filas_siguen_el_orden_y_marcan_la_atencion_y_la_seleccion() {
        let mut hub = Hub::default();
        hub.add("a".into(), "Primera".into(), 10);
        hub.add("b".into(), "Segunda".into(), 20);
        hub.apply(&perm("b", 1, "Bash"));
        let rows = sidebar_rows(&hub, 10);
        assert_eq!(rows[0].key, "b");
        assert!(rows[0].ring && !rows[0].selected);
        assert_eq!(rows[0].detail, "Necesita tu atención · Bash");
        assert_eq!(rows[0].icon, "!");
        assert!(!rows[1].ring && rows[1].selected);
        assert_eq!(rows[1].detail, "Trabajando");
    }

    #[test]
    fn la_rama_va_delante_del_estado() {
        let mut hub = Hub::default();
        hub.add("a".into(), "T".into(), 1);
        hub.apply(&BrokerEvent::StatusLine {
            agent: "a".into(),
            payload: json!({"worktree": {"branch": "worktree-x"}}),
        });
        assert_eq!(sidebar_rows(&hub, 1)[0].detail, "worktree-x · Trabajando");
    }

    #[test]
    fn terminada_la_peticion_vuelve_la_fila_a_la_normalidad() {
        let mut hub = Hub::default();
        hub.add("a".into(), "T".into(), 1);
        hub.apply(&perm("a", 1, "Bash"));
        hub.apply(&BrokerEvent::PermissionEnded {
            id: 1,
            agent: "a".into(),
            reason: Ended::ClientGone,
        });
        assert!(!sidebar_rows(&hub, 1)[0].ring);
    }

    #[test]
    fn el_detalle_es_la_orden_el_archivo_o_el_json() {
        let mk = |input: Value| PermissionRequest {
            id: 1,
            agent: "a".into(),
            session_id: "s".into(),
            tool_name: "X".into(),
            tool_input: input,
            cwd: "/".into(),
            suggestions: Value::Null,
        };
        assert_eq!(detail_of(&mk(json!({"command": "ls -la"}))), "ls -la");
        assert_eq!(detail_of(&mk(json!({"file_path": "/a/b.rs"}))), "/a/b.rs");
        assert_eq!(detail_of(&mk(json!({"q": 1}))), r#"{"q":1}"#);
        assert_eq!(detail_of(&mk(Value::Null)), "");
    }

    #[test]
    fn el_titulo_se_acorta_por_palabras_con_puntos_suspensivos() {
        assert_eq!(
            first_words("Arregla el bug\nsegunda línea", 40),
            "Arregla el bug"
        );
        assert_eq!(first_words("abcdefghij", 5), "abcd…");
        assert_eq!(first_words("", 5), "");
    }
}
