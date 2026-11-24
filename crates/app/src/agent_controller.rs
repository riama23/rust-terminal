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
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSApplication, NSMenu, NSMenuItem, NSStatusItem, NSTextField,
};
use objc2_foundation::{NSPoint, NSRect, NSSize, NSString};
use serde_json::Value;

use crate::agent::alerts::Action;
use crate::agent::broker::{AgentKey, Broker, BrokerEvent, Decision, PermissionRequest};
use crate::agent::hub::Hub;
use crate::agent::launch::{self, LaunchSpec};
use crate::agent::model::Status;
use crate::agent::rules;
use crate::agent_ui::{
    ATTENTION_BAR_HEIGHT, AttentionBar, ChangeRow, DONE_BAR_HEIGHT, DoneBar, INSPECTOR_WIDTH,
    InspectorData, InspectorView, SIDEBAR_WIDTH, SidebarRow, SidebarView,
};
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
    /// La ventana de Review abierta (una a la vez).
    pub(crate) review: RefCell<Option<crate::review_controller::ReviewState>>,
}

impl AgentController {
    pub(crate) fn hub_ref(&self) -> std::cell::Ref<'_, Hub> {
        std::cell::Ref::map(self.inner.borrow(), |i| &i.hub)
    }

    /// Carpeta donde trabaja el agente (el worktree o el proyecto).
    pub(crate) fn workdir_of(&self, key: &str) -> Option<PathBuf> {
        self.inner.borrow().workdirs.get(key).cloned()
    }

    pub(crate) fn project_of(&self, key: &str) -> Option<PathBuf> {
        self.inner.borrow().projects.get(key).cloned()
    }

    pub(crate) fn title_of(&self, key: &str) -> Option<String> {
        self.inner
            .borrow()
            .hub
            .sessions()
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.title.clone())
    }

    pub(crate) fn status_of(&self, key: &str) -> Option<Status> {
        self.inner
            .borrow()
            .hub
            .sessions()
            .iter()
            .find(|s| s.key == key)
            .map(|s| s.status.clone())
    }

    pub(crate) fn pane_of(&self, key: &str) -> Option<PaneId> {
        self.inner.borrow().hub.pane_of(key)
    }
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
    /// Dónde trabaja cada agente: su worktree o, sin él, el proyecto.
    workdirs: HashMap<AgentKey, PathBuf>,
    /// Claves de los paneles normales (`RT_CLAUDE_SETTINGS`): no salen en la
    /// sidebar hasta que un `claude` lanzado a mano manda su primer evento.
    dormant: HashMap<AgentKey, PaneId>,
    /// Agentes adoptados de una clave dormida, con «ya tiene título del prompt».
    manual: HashMap<AgentKey, bool>,
    sidebar: Option<Retained<SidebarView>>,
    sidebar_shown: bool,
    bar: Option<Retained<AttentionBar>>,
    /// Qué ocupa el hueco bajo la terminal (la de atención manda sobre la de terminado).
    bottom: Bottom,
    done_bar: Option<Retained<DoneBar>>,
    inspector: Option<Retained<InspectorView>>,
    inspector_shown: bool,
    /// Último diff calculado de cada agente: cambios y rama.
    changes: HashMap<AgentKey, (Vec<ChangeRow>, Option<String>)>,
    /// Dentro de `refresh_agent_ui`: tocar el contenedor recoloca la terminal,
    /// que vuelve a pedir un refresco. Sin esta guarda, recursión infinita.
    refreshing: bool,
    /// Icono de la barra de menús (existe mientras haya agentes).
    status_item: Option<Retained<NSStatusItem>>,
    /// Lo último que se pintó en el menú, para no reconstruirlo sin cambios.
    status_signature: Vec<String>,
    /// Último número puesto en el badge del Dock.
    badge: usize,
    /// `RT_REVIEW_AUTOSEND` (solo debug): ya se disparó la revisión automática.
    autosend_fired: bool,
}

impl Inner {
    /// Un `claude` lanzado a mano terminó: la clave vuelve a dormir en su
    /// panel por si lanza otro.
    fn end_manual(&mut self, key: &str) {
        let pane = self.hub.pane_of(key);
        self.hub.remove(key);
        self.manual.remove(key);
        self.projects.remove(key);
        self.workdirs.remove(key);
        if let Some(pane) = pane {
            self.dormant.insert(key.to_owned(), pane);
        }
    }

    /// El primer evento de una clave dormida convierte el panel en un agente
    /// de la sidebar. `true` si pasó. `StatusLine` no adopta (llega también
    /// tras `SessionEnd`).
    fn adopt(&mut self, event: &BrokerEvent) -> bool {
        let (key, cwd) = match event {
            BrokerEvent::Hook {
                agent,
                event: name,
                payload,
            } if name != "SessionEnd" => (agent, payload.get("cwd").and_then(Value::as_str)),
            BrokerEvent::Permission(p) => (&p.agent, Some(p.cwd.as_str())),
            _ => return false,
        };
        let Some(pane) = self.dormant.remove(key) else {
            return false;
        };
        // Lo había detectado ya el proceso: la entrada completa lo sustituye.
        if let Some(old) = self.hub.key_of_pane(pane).cloned() {
            self.hub.remove(&old);
        }
        let dir = cwd.filter(|c| !c.is_empty()).map(PathBuf::from);
        let title = dir
            .as_deref()
            .map_or_else(|| "Claude".to_owned(), folder_title);
        self.hub.add_running(key.clone(), title, pane, false);
        self.manual.insert(key.clone(), false);
        if let Some(dir) = dir {
            self.projects.insert(key.clone(), dir.clone());
            self.workdirs.insert(key.clone(), dir);
        }
        true
    }
}

/// Nombre de carpeta para el título de un agente lanzado a mano.
fn folder_title(dir: &std::path::Path) -> String {
    dir.file_name()
        .map_or_else(|| "Claude".to_owned(), |n| n.to_string_lossy().into_owned())
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Default)]
enum Bottom {
    #[default]
    None,
    Attention,
    Done,
}

fn status_color(status: &Status) -> [u8; 3] {
    match status {
        Status::NeedsYou { .. } => dusk::ATTENTION,
        Status::Working | Status::Running | Status::PlanReady => dusk::ACCENT,
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
        self.ensure_broker()?;
        let state = self.agents();

        // Clave y archivos del agente.
        let (broker_key, files) = {
            let inner = state.inner.borrow_mut();
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
            launch::agent_shell(&self.user_shell()),
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
            let workdir = match &spec.isolation {
                launch::Isolation::NewWorktree(name) => {
                    project.join(".claude").join("worktrees").join(name)
                }
                launch::Isolation::CurrentFolder => project.clone(),
            };
            inner.workdirs.insert(broker_key.clone(), workdir);
            inner.projects.insert(broker_key.clone(), project);
            inner.hub.add(broker_key, title, pane);
        }
        self.refresh_agent_ui();
        Ok(())
    }

    /// Arranca el broker y la carpeta de archivos la primera vez.
    fn ensure_broker(&self) -> Result<(), String> {
        let state = self.agents();
        if state.inner.borrow().broker.is_some() {
            return Ok(());
        }
        let waker = self
            .waker_arc()
            .ok_or("La ventana aún no está lista.".to_owned())?;
        crate::notify::install_agent_support(waker.clone());
        let queue = state.inner.borrow().queue.clone();
        let broker = Broker::start(PERMISSION_DEADLINE, move |event| {
            queue.lock().unwrap().push(event);
            waker.on_main_thread(sel!(agentEvents));
        })
        .map_err(|e| format!("No se pudo arrancar el broker: {e}"))?;
        let mut inner = state.inner.borrow_mut();
        inner.broker = Some(broker);
        let base = std::env::temp_dir();
        launch::remove_stale_dirs(&base, launch::process_alive);
        inner.dir = Some(launch::agents_dir(&base, std::process::id()));
        Ok(())
    }

    /// Variable de entorno de un panel normal: apunta a los `settings` con los
    /// hooks de la app, que la función `claude` de zsh añade a un `claude`
    /// lanzado a mano. `None` si algo falla (el panel sigue como siempre).
    pub(crate) fn manual_claude_env(
        &self,
        pane: PaneId,
        cwd: Option<&std::path::Path>,
    ) -> Option<(String, String)> {
        self.ensure_broker().ok()?;
        let state = self.agents();
        let home = std::env::var_os("HOME")
            .map(PathBuf::from)
            .unwrap_or_default();
        let project = cwd.map_or_else(|| home.clone(), std::path::Path::to_path_buf);
        let user_statusline = launch::find_user_statusline(&home, &project);
        let (key, settings) = {
            let inner = state.inner.borrow();
            let broker = inner.broker.as_ref()?;
            let dir = inner.dir.clone()?;
            let key = broker.register_agent().ok()?;
            match launch::write_agent_files(&dir, broker, &key, user_statusline.as_deref()) {
                Ok(files) => (key, files.settings),
                Err(_) => {
                    broker.forget_agent(&key);
                    return None;
                }
            }
        };
        state.inner.borrow_mut().dormant.insert(key, pane);
        Some((
            "RT_CLAUDE_SETTINGS".to_owned(),
            settings.to_string_lossy().into_owned(),
        ))
    }

    /// Claude Code pasó a ser (o dejó de ser) el programa en primer plano de
    /// un panel. Respaldo para lo que no manda hooks: «En marcha», sin más.
    pub fn claude_foreground(&self, pane: PaneId, claude: bool) {
        let state = self.agents();
        let changed = {
            let mut inner = state.inner.borrow_mut();
            let existing = inner.hub.key_of_pane(pane).cloned();
            match (claude, existing) {
                (true, None) => {
                    let title = self
                        .session(pane)
                        .and_then(|s| s.cwd())
                        .map_or_else(|| "Claude".to_owned(), |c| folder_title(&c));
                    inner
                        .hub
                        .add_running(format!("pane-{pane}"), title, pane, true);
                    true
                }
                (false, Some(key)) => {
                    let limited = inner
                        .hub
                        .sessions()
                        .iter()
                        .any(|s| s.key == key && s.limited);
                    if limited {
                        inner.hub.remove(&key);
                        true
                    } else if inner.manual.contains_key(&key) {
                        inner.end_manual(&key);
                        true
                    } else {
                        false
                    }
                }
                _ => false,
            }
        };
        if changed {
            self.refresh_agent_ui();
        }
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
        inner.workdirs.remove(key);
        inner.manual.remove(key);
        inner.dormant.remove(key);
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
        // Claves dormidas de paneles que ya no existen.
        let stale: Vec<AgentKey> = {
            let inner = self.agents().inner.borrow();
            inner
                .dormant
                .iter()
                .filter(|(_, p)| !self.pane_exists(**p))
                .map(|(k, _)| k.clone())
                .collect()
        };
        for key in &stale {
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
        let active = NSApplication::sharedApplication(self.mtm()).isActive();
        let done_before: std::collections::HashSet<AgentKey> = {
            let inner = self.agents().inner.borrow();
            inner
                .hub
                .sessions()
                .iter()
                .filter(|s| s.status == Status::Done)
                .map(|s| s.key.clone())
                .collect()
        };
        let mut changed = false;
        let mut alerts = Vec::new();
        {
            let mut inner = self.agents().inner.borrow_mut();
            for event in &events {
                changed |= inner.adopt(event);
                if let BrokerEvent::Hook {
                    agent,
                    event: name,
                    payload,
                } = event
                    && inner.manual.contains_key(agent)
                {
                    match name.as_str() {
                        // La sesión acabó: sale de la sidebar.
                        "SessionEnd" => {
                            inner.end_manual(agent);
                            changed = true;
                            continue;
                        }
                        // Recién abierta: sigue «En marcha» hasta que haga algo.
                        "SessionStart" => continue,
                        // El primer prompt da título a la fila.
                        "UserPromptSubmit" if inner.manual.get(agent) == Some(&false) => {
                            let prompt = payload.get("prompt").and_then(Value::as_str);
                            if let Some(prompt) = prompt.filter(|p| !p.trim().is_empty()) {
                                inner.hub.set_title(agent, first_words(prompt, 40));
                                inner.manual.insert(agent.clone(), true);
                            }
                        }
                        _ => {}
                    }
                }
                let (c, alert) = inner.hub.apply_with_alert(event, active);
                changed |= c;
                alerts.extend(alert);
            }
        }
        for alert in &alerts {
            crate::notify::notify_agent(alert);
        }
        // Un turno que acaba de terminar: se calcula qué cambió.
        let newly_done: Vec<AgentKey> = {
            let inner = self.agents().inner.borrow();
            inner
                .hub
                .sessions()
                .iter()
                .filter(|s| s.status == Status::Done && !done_before.contains(&s.key))
                .map(|s| s.key.clone())
                .collect()
        };
        for key in newly_done {
            self.load_changes_async(&key);
        }
        self.autosend_for_tests();
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
        self.update_dock_and_menu_bar();
        self.snapshot_agent_ui();
        self.auto_decide_for_tests();
    }

    /// Badge del Dock (sesiones que necesitan al usuario, no salida sin leer)
    /// e icono de la barra de menús con la lista de agentes.
    fn update_dock_and_menu_bar(&self) {
        let state = self.agents();
        let (attention, total, rows) = {
            let inner = state.inner.borrow();
            let rows: Vec<(String, String)> = inner
                .hub
                .sessions()
                .iter()
                .map(|s| {
                    (
                        s.key.clone(),
                        format!("{} {} — {}", s.status.icon(), s.title, s.status.label()),
                    )
                })
                .collect();
            (inner.hub.needing_attention(), rows.len(), rows)
        };
        let mtm = self.mtm();

        // Dock.
        if std::mem::replace(&mut state.inner.borrow_mut().badge, attention) != attention {
            let tile = NSApplication::sharedApplication(mtm).dockTile();
            let label = (attention > 0).then(|| NSString::from_str(&attention.to_string()));
            tile.setBadgeLabel(label.as_deref());
        }

        // Barra de menús: solo mientras haya agentes.
        if total == 0 {
            if let Some(item) = state.inner.borrow_mut().status_item.take() {
                objc2_app_kit::NSStatusBar::systemStatusBar().removeStatusItem(&item);
            }
            state.inner.borrow_mut().status_signature.clear();
            return;
        }
        let title = if attention > 0 {
            format!("! {attention}")
        } else {
            format!("◐ {total}")
        };
        let mut signature: Vec<String> = rows.iter().map(|(k, t)| format!("{k}\u{1}{t}")).collect();
        signature.push(title.clone());
        if state.inner.borrow().status_signature == signature
            && state.inner.borrow().status_item.is_some()
        {
            return;
        }
        let item = state.inner.borrow().status_item.clone().unwrap_or_else(|| {
            objc2_app_kit::NSStatusBar::systemStatusBar()
                .statusItemWithLength(objc2_app_kit::NSVariableStatusItemLength)
        });
        if let Some(button) = item.button(mtm) {
            button.setTitle(&NSString::from_str(&title));
        }
        let menu = NSMenu::new(mtm);
        for (key, text) in &rows {
            let entry = self.status_menu_item(text, sel!(focusAgentFromMenu:), "");
            // SAFETY: la clave es una cadena; el menú retiene el objeto.
            unsafe { entry.setRepresentedObject(Some(&NSString::from_str(key))) };
            menu.addItem(&entry);
        }
        menu.addItem(&NSMenuItem::separatorItem(mtm));
        let new_agent = self.status_menu_item("Nuevo agente…  (⌃⌥N)", sel!(newAgent:), "");
        menu.addItem(&new_agent);
        item.setMenu(Some(&menu));
        let mut inner = state.inner.borrow_mut();
        inner.status_item = Some(item);
        inner.status_signature = signature;
    }

    fn status_menu_item(
        &self,
        title: &str,
        action: objc2::runtime::Sel,
        key: &str,
    ) -> Retained<NSMenuItem> {
        // SAFETY: título, acción y atajo válidos; el destino es esta vista.
        let item = unsafe {
            NSMenuItem::initWithTitle_action_keyEquivalent(
                NSMenuItem::alloc(self.mtm()),
                &NSString::from_str(title),
                Some(action),
                &NSString::from_str(key),
            )
        };
        // SAFETY: la vista implementa la acción y vive más que el menú.
        unsafe { item.setTarget(Some(self)) };
        item
    }

    /// Respuestas a notificaciones (hilo principal).
    pub fn notification_responses(&self) {
        for response in crate::notify::take_responses() {
            match response.action {
                Action::Open => self.focus_agent_and_activate(&response.agent),
                Action::AllowOnce | Action::Deny(_) => {
                    let decision = match response.action {
                        Action::Deny(d) => d,
                        _ => Decision::Allow,
                    };
                    // Solo la petición concreta de la notificación: una vieja no
                    // puede contestar otra distinta.
                    let req = response.request_id.and_then(|id| {
                        self.agents()
                            .inner
                            .borrow()
                            .hub
                            .pending_request(&response.agent, id)
                            .cloned()
                    });
                    if let Some(req) = req {
                        self.decide(&req, decision);
                    }
                }
            }
        }
    }

    /// ⌃⌥N desde otra aplicación: trae la ventana al frente y abre el formulario.
    pub fn global_new_agent(&self) {
        #[allow(deprecated)]
        NSApplication::sharedApplication(self.mtm()).activateIgnoringOtherApps(true);
        if let Some(window) = self.window() {
            window.makeKeyAndOrderFront(None);
        }
        self.new_agent_sheet();
    }

    /// Lleva la app y el panel del agente al frente.
    pub fn focus_agent_and_activate(&self, key: &str) {
        let mtm = self.mtm();
        #[allow(deprecated)]
        NSApplication::sharedApplication(mtm).activateIgnoringOtherApps(true);
        if let Some(window) = self.window() {
            window.makeKeyAndOrderFront(None);
        }
        self.focus_agent(key);
    }

    /// `RT_REVIEW_AUTOSEND=1` (solo debug): cuando el agente termina su turno,
    /// abre la revisión; `review_ready` la envía. Verifica el envío sin pulsar.
    fn autosend_for_tests(&self) {
        if !cfg!(debug_assertions) || std::env::var_os("RT_REVIEW_AUTOSEND").is_none() {
            return;
        }
        let ready = {
            let mut inner = self.agents().inner.borrow_mut();
            let done = inner
                .hub
                .sessions()
                .iter()
                .any(|s| s.status == Status::Done);
            done && !std::mem::replace(&mut inner.autosend_fired, true)
        };
        if ready {
            // Antes de que haya turno terminado el envío debe rechazarse: se prueba aparte.
            self.open_review();
        }
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
        // Lo que hay puesto en el Dock y en la barra de menús, leído de AppKit.
        let mtm = self.mtm();
        let tile = NSApplication::sharedApplication(mtm).dockTile();
        let title = inner
            .status_item
            .as_ref()
            .and_then(|i| i.button(mtm))
            .map(|b| b.title().to_string())
            .unwrap_or_default();
        let menu = inner
            .status_item
            .as_ref()
            .and_then(|i| i.menu(mtm))
            .map(|m| {
                m.itemArray()
                    .iter()
                    .map(|i| i.title().to_string())
                    .collect::<Vec<_>>()
                    .join(" | ")
            })
            .unwrap_or_default();
        let badge = tile.badgeLabel().map(|l| l.to_string()).unwrap_or_default();
        let _ = std::fs::write(
            dir.join("status.txt"),
            format!(
                "dock_badge={badge}\nstatus_title={title}\nmenu={menu}\nhotkey={}\n",
                crate::hotkey::status_text().unwrap_or_default()
            ),
        );
        if inner.sidebar_shown
            && let Some(s) = &inner.sidebar
        {
            crate::agent_ui::snapshot_png(s, &dir.join("sidebar.png").to_string_lossy());
        }
        if inner.bottom == Bottom::Attention
            && let Some(b) = &inner.bar
        {
            crate::agent_ui::snapshot_png(b, &dir.join("bar.png").to_string_lossy());
        }
        if inner.bottom == Bottom::Done
            && let Some(b) = &inner.done_bar
        {
            crate::agent_ui::snapshot_png(b, &dir.join("done.png").to_string_lossy());
        }
        if inner.inspector_shown
            && let Some(i) = &inner.inspector
        {
            crate::agent_ui::snapshot_png(i, &dir.join("inspector.png").to_string_lossy());
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

        // Datos del agente del panel con el foco (para la barra y el inspector).
        let info = {
            let inner = state.inner.borrow();
            inner.hub.session_of_pane(focused).map(|s| {
                let changes = inner.changes.get(&s.key).cloned();
                let workdir = inner.workdirs.get(&s.key).cloned().unwrap_or_default();
                (s.clone(), changes, workdir)
            })
        };

        // Barra inferior: un permiso pendiente manda sobre el aviso de terminado.
        let done_summary = info.as_ref().and_then(|(s, changes, _)| {
            if s.status != Status::Done {
                return None;
            }
            let (rows, _) = changes.as_ref()?;
            (!rows.is_empty()).then(|| done_text(rows))
        });
        let want = if pending.is_some() {
            Bottom::Attention
        } else if done_summary.is_some() {
            Bottom::Done
        } else {
            Bottom::None
        };
        match (&pending, &done_summary) {
            (Some(req), _) => {
                let bar = self.ensure_bar();
                let can_always = rules::rule_for(&req.tool_name, &req.tool_input).is_some();
                bar.set_request(&req.tool_name, &req.detail(), can_always);
            }
            (None, Some(text)) => self.ensure_done_bar().set_summary(text),
            (None, None) => {}
        }
        let previous = std::mem::replace(&mut state.inner.borrow_mut().bottom, want);
        if previous != want {
            match want {
                Bottom::Attention => {
                    container.set_bottom(Some((&self.ensure_bar(), ATTENTION_BAR_HEIGHT)))
                }
                Bottom::Done => {
                    container.set_bottom(Some((&self.ensure_done_bar(), DONE_BAR_HEIGHT)))
                }
                Bottom::None => container.set_bottom(None),
            }
        }

        // Inspector: a la derecha mientras el panel con el foco sea un agente y
        // quepa (las columnas se pliegan en ventanas estrechas).
        let sidebar_on = state.inner.borrow().sidebar_shown;
        let fits = crate::agent_ui::inspector_fits(container.bounds().size.width, sidebar_on);
        match info.filter(|_| fits) {
            Some((s, changes, workdir)) => {
                let inspector = self.ensure_inspector();
                inspector.set_data(InspectorData {
                    todos: s.todos.clone(),
                    changes: changes.as_ref().map(|(rows, _)| rows.clone()),
                    context_percent: s.telemetry.context_percent,
                    worktree: workdir.display().to_string(),
                    branch: s
                        .telemetry
                        .branch
                        .clone()
                        .or_else(|| changes.and_then(|(_, b)| b)),
                });
                if !std::mem::replace(&mut state.inner.borrow_mut().inspector_shown, true) {
                    container.set_right(Some((&inspector, INSPECTOR_WIDTH)));
                }
            }
            None => {
                if std::mem::replace(&mut state.inner.borrow_mut().inspector_shown, false) {
                    container.set_right(None);
                }
            }
        }
    }

    fn ensure_done_bar(&self) -> Retained<DoneBar> {
        let state = self.agents();
        if let Some(b) = state.inner.borrow().done_bar.clone() {
            return b;
        }
        let bar = DoneBar::new(
            self.mtm(),
            self.as_ref(),
            sel!(reviewChanges:),
            sel!(openPullRequest:),
        );
        state.inner.borrow_mut().done_bar = Some(bar.clone());
        bar
    }

    fn ensure_inspector(&self) -> Retained<InspectorView> {
        let state = self.agents();
        if let Some(i) = state.inner.borrow().inspector.clone() {
            return i;
        }
        let inspector = InspectorView::new(self.mtm());
        state.inner.borrow_mut().inspector = Some(inspector.clone());
        inspector
    }

    /// Guarda el diff calculado de un agente (inspector y barra de terminado).
    pub fn set_agent_changes(&self, key: &str, rows: Vec<ChangeRow>, branch: Option<String>) {
        self.agents()
            .inner
            .borrow_mut()
            .changes
            .insert(key.to_owned(), (rows, branch));
        self.refresh_agent_ui();
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
        let project = default_project_dir(self.focused_cwd());
        let Some(form) = crate::agent_sheet::run(self.mtm(), &project) else {
            return;
        };
        if let Err(e) = self.launch_agent(form.to_spec(), project) {
            self.show_launch_error(&e);
        }
    }

    /// Muestra un error de lanzamiento.
    pub fn show_launch_error(&self, message: &str) {
        self.alert("No se pudo lanzar el agente", message);
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

/// El resumen de la barra de terminado, corto para que quepa en ventanas estrechas.
fn done_text(rows: &[ChangeRow]) -> String {
    let (added, removed): (usize, usize) = rows
        .iter()
        .fold((0, 0), |(a, r), c| (a + c.added, r + c.removed));
    let noun = if rows.len() == 1 {
        "archivo"
    } else {
        "archivos"
    };
    format!(
        "Terminó el turno · {} {noun} (+{added} −{removed})",
        rows.len()
    )
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
    use serde_json::json;

    use crate::agent::broker::Ended;

    fn hook(agent: &str, event: &str, payload: Value) -> BrokerEvent {
        BrokerEvent::Hook {
            agent: agent.into(),
            event: event.into(),
            payload,
        }
    }

    fn row(inner: &Inner) -> Option<(String, Status, bool)> {
        inner
            .hub
            .sessions()
            .first()
            .map(|s| (s.title.clone(), s.status.clone(), s.limited))
    }

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
        assert_eq!(mk(json!({"command": "ls -la"})).detail(), "ls -la");
        assert_eq!(mk(json!({"file_path": "/a/b.rs"})).detail(), "/a/b.rs");
        assert_eq!(mk(json!({"q": 1})).detail(), r#"{"q":1}"#);
        assert_eq!(mk(Value::Null).detail(), "");
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

    #[test]
    fn el_resumen_de_terminado_es_corto_y_concuerda_en_numero() {
        let row = |a, r| ChangeRow {
            tag: "M",
            path: "x".into(),
            added: a,
            removed: r,
        };
        assert_eq!(
            done_text(&[row(3, 1)]),
            "Terminó el turno · 1 archivo (+3 −1)"
        );
        assert_eq!(
            done_text(&[row(3, 1), row(10, 0)]),
            "Terminó el turno · 2 archivos (+13 −1)"
        );
    }

    #[test]
    fn a_dormant_key_becomes_an_agent_on_its_first_event() {
        let mut inner = Inner::default();
        inner.dormant.insert("k".into(), 7);
        // Un `StatusLine` suelto (llega también tras `SessionEnd`) no adopta.
        let status = BrokerEvent::StatusLine {
            agent: "k".into(),
            payload: json!({}),
        };
        assert!(!inner.adopt(&status));
        assert!(inner.hub.is_empty());

        let start = hook("k", "SessionStart", json!({"cwd": "/work/mi-proyecto"}));
        assert!(inner.adopt(&start));
        assert_eq!(
            row(&inner),
            Some(("mi-proyecto".into(), Status::Running, false))
        );
        assert_eq!(inner.hub.pane_of("k"), Some(7));
        assert_eq!(inner.workdirs["k"], PathBuf::from("/work/mi-proyecto"));
        assert!(!inner.dormant.contains_key("k"));
        // Ya adoptada: otro evento no vuelve a adoptar.
        assert!(!inner.adopt(&start));
    }

    #[test]
    fn a_permission_also_adopts() {
        let mut inner = Inner::default();
        inner.dormant.insert("k".into(), 3);
        assert!(inner.adopt(&perm("k", 1, "Bash")));
        assert_eq!(inner.hub.pane_of("k"), Some(3));
    }

    #[test]
    fn the_full_entry_replaces_the_one_seen_by_process() {
        let mut inner = Inner::default();
        inner.dormant.insert("k".into(), 7);
        inner
            .hub
            .add_running("pane-7".into(), "Claude".into(), 7, true);
        assert_eq!(row(&inner).map(|r| r.2), Some(true));
        assert!(inner.adopt(&hook("k", "PreToolUse", json!({"cwd": "/a/b"}))));
        assert_eq!(inner.hub.sessions().len(), 1);
        assert_eq!(row(&inner), Some(("b".into(), Status::Running, false)));
    }

    #[test]
    fn ending_returns_the_key_to_its_pane() {
        let mut inner = Inner::default();
        inner.dormant.insert("k".into(), 7);
        inner.adopt(&hook("k", "SessionStart", json!({"cwd": "/a/b"})));
        inner.end_manual("k");
        assert!(inner.hub.is_empty());
        assert_eq!(inner.dormant.get("k"), Some(&7));
        // El siguiente `claude` del panel vuelve a adoptarse.
        assert!(inner.adopt(&hook("k", "SessionStart", json!({"cwd": "/a/c"}))));
        assert_eq!(row(&inner).map(|r| r.0), Some("c".into()));
    }
}
