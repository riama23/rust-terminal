//! La vista de una ventana: pestañas, paneles y sus sesiones.
//!
//! - **Teclado:** va al panel con el foco. `NSTextInputClient` resuelve las
//!   teclas muertas y Option.
//! - **Ratón:** barra de pestañas, divisores, foco, selección y scroll.
//! - **Layout:** se calcula aquí (hilo principal) y se publica para el render
//!   como un [`WindowFrame`].

use std::cell::{Cell, OnceCell, RefCell};
use std::collections::HashMap;
use std::sync::Arc;
use std::sync::atomic::Ordering::SeqCst;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, Bool, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSAlert, NSAlertFirstButtonReturn, NSAppearance, NSAppearanceCustomization,
    NSAppearanceNameAqua, NSAppearanceNameDarkAqua, NSApplication, NSCursor, NSEvent,
    NSEventModifierFlags, NSMenuItem, NSPasteboard, NSPasteboardTypeString,
    NSRequestUserAttentionType, NSResponder, NSTextInputClient, NSView, NSWorkspace,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSPoint,
    NSRange, NSRect, NSSize, NSString, NSUInteger,
};
use rust_terminal_core::selection;
use rust_terminal_core::{MouseMode, Selection, SelectionKind, SelectionRange, Term, TermEvent};

use crate::config::{ClipboardRead, Config};
use crate::font::FontSpec;
use crate::font::Metrics;
use crate::input::{
    self, KeyEvent, KeyKind, KittyKey, KittyMods, Mods, MouseAction, MouseButton, kitty,
};
use crate::notify;
use crate::renderer::{Layout, TabBar, TabItem};
use crate::report;
use crate::session::{AppEvent, Session, SpawnOptions};
use crate::window::{PaneSlot, Waker, WindowFrame, WindowShared};
use crate::workspace::{Axis, Direction, Divider, PaneId, Rect, Workspace};

/// Geometría calculada en el último layout, en píxeles, para el ratón.
#[derive(Default)]
struct Geometry {
    scale: f64,
    metrics: Option<Metrics>,
    pad: f64,
    tab_bar: Option<TabBar>,
    panes: Vec<(PaneId, Rect)>,
    dividers: Vec<Divider>,
    /// Cabeceras de los paneles (vacío con un solo panel).
    headers: Vec<(PaneId, Rect)>,
}

/// Qué se está arrastrando con el ratón.
enum Drag {
    Divider(Divider),
    Select(PaneId),
    /// La aplicación del panel recibe el ratón (`?1000` y similares).
    Report(PaneId, MouseButton),
}

pub struct ViewIvars {
    window: Arc<WindowShared>,
    waker: OnceCell<Arc<Waker>>,
    /// Configuración vigente (se recarga en caliente).
    config: RefCell<Config>,
    /// Entorno base de las shells (locale); la integración se añade por sesión.
    base_env: Vec<(String, String)>,
    /// `-e comando …`: lo ejecuta el primer panel en lugar de la shell.
    initial_command: RefCell<Option<Vec<String>>>,
    workspace: RefCell<Option<Workspace>>,
    sessions: RefCell<HashMap<PaneId, Arc<Session>>>,
    next_id: Cell<PaneId>,
    geometry: RefCell<Geometry>,
    drag: RefCell<Option<Drag>>,
    /// Texto en composición (tecla muerta o método de entrada).
    marked: RefCell<String>,
    /// Métricas para la escala actual, para no recalcularlas en cada resize.
    metrics: RefCell<Option<(f64, FontSpec, Metrics)>>,
    /// Desplazamiento del trackpad acumulado que aún no llega a una línea.
    scroll_accum: Cell<f64>,
    /// El ratón se movió desde `mouseDown` (si no, un clic simple deselecciona).
    drag_moved: Cell<bool>,
    /// Enlace bajo el ratón mientras se mantiene Cmd.
    hover_link: RefCell<Option<(PaneId, SelectionRange)>>,
    /// Última posición del ratón en píxeles (para Cmd sin mover el ratón).
    last_mouse: Cell<Option<(f64, f64)>>,
    /// Última celda reportada a la aplicación (para no repetir movimientos).
    last_reported: Cell<Option<(PaneId, usize, usize)>>,
    /// La ventana es la activa y qué panel tenía el foco (avisos `?1004`).
    window_key: Cell<bool>,
    last_focus: Cell<Option<PaneId>>,
    /// Fotograma del spinner de las pestañas y si su reloj está en marcha.
    spinner_phase: Cell<usize>,
    spinner_running: Cell<bool>,
    /// Opacidad del fondo (1; el quick terminal usa menos) y si esta vista
    /// es la del quick terminal.
    opacity: Cell<f32>,
    quick: Cell<bool>,
    /// Texto a la derecha de la barra de pestañas («fijado» en el quick terminal).
    tab_note: RefCell<Option<String>>,
    /// Bloque seleccionado con ⌘↑/⌘↓: panel e identificador.
    pub(crate) selected_block: Cell<Option<(PaneId, u64)>>,
    /// Bloques colapsados (panel e identificador).
    pub(crate) collapsed_blocks: RefCell<std::collections::HashSet<(PaneId, u64)>>,
    /// Agentes de Claude Code de esta ventana (sidebar y attention bar).
    pub(crate) agents: crate::agent_controller::AgentController,
    /// La paleta de comandos (⌘⇧P).
    pub(crate) palette: crate::palette_controller::PaletteController,
    /// La ventana de ajustes (⌘,).
    pub(crate) settings: crate::settings_controller::SettingsController,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y `TerminalView`
    // no implementa `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTTerminalView"]
    #[ivars = ViewIvars]
    pub struct TerminalView;

    unsafe impl NSObjectProtocol for TerminalView {}

    impl TerminalView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        /// ⌘↵ y ⌘⌫ contestan el permiso pendiente, solo si la barra está visible.
        #[unsafe(method(performKeyEquivalent:))]
        fn perform_key_equivalent(&self, event: &NSEvent) -> Bool {
            if self.handle_agent_key(event) {
                return Bool::YES;
            }
            // SAFETY: llamada a la implementación de NSView.
            unsafe { msg_send![super(self), performKeyEquivalent: event] }
        }

        #[unsafe(method(allowOnce:))]
        fn allow_once_action(&self, _sender: Option<&AnyObject>) {
            self.allow_once();
        }

        #[unsafe(method(alwaysAllow:))]
        fn always_allow_action(&self, _sender: Option<&AnyObject>) {
            self.always_allow();
        }

        #[unsafe(method(denyPermission:))]
        fn deny_permission_action(&self, _sender: Option<&AnyObject>) {
            self.deny_with_reason();
        }

        /// Elegir un agente en el menú de la barra de menús.
        #[unsafe(method(focusAgentFromMenu:))]
        fn focus_agent_from_menu(&self, sender: Option<&AnyObject>) {
            let key = sender
                .and_then(|s| s.downcast_ref::<NSMenuItem>())
                .and_then(|i| i.representedObject())
                .and_then(|o| o.downcast::<NSString>().ok())
                .map(|s| s.to_string());
            if let Some(key) = key {
                self.focus_agent_and_activate(&key);
            }
        }

        /// ⌘⇧R: revisar los cambios del agente del panel con el foco.
        #[unsafe(method(reviewChanges:))]
        fn review_changes_action(&self, _sender: Option<&AnyObject>) {
            self.open_review();
        }

        #[unsafe(method(reviewSend:))]
        fn review_send_action(&self, _sender: Option<&AnyObject>) {
            self.review_send();
        }

        /// «Abrir PR…» de la barra de terminado: el agente del panel con el foco.
        #[unsafe(method(openPullRequest:))]
        fn open_pull_request_action(&self, _sender: Option<&AnyObject>) {
            self.open_pr_focused();
        }

        #[unsafe(method(reviewOpenPR:))]
        fn review_open_pr_action(&self, _sender: Option<&AnyObject>) {
            self.open_pr_review();
        }

        #[unsafe(method(reviewDiscard:))]
        fn review_discard_action(&self, _sender: Option<&AnyObject>) {
            self.discard_worktree_review();
        }

        #[unsafe(method(reviewRefresh:))]
        fn review_refresh_action(&self, _sender: Option<&AnyObject>) {
            self.review_refresh();
        }

        #[unsafe(method(newAgent:))]
        fn new_agent_action(&self, _sender: Option<&AnyObject>) {
            self.new_agent_sheet();
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let kind = if event.isARepeat() {
                KeyKind::Repeat
            } else {
                KeyKind::Press
            };
            self.handle_key(event, kind);
        }

        /// Soltar una tecla: solo importa con el nivel 2 del protocolo de kitty.
        #[unsafe(method(keyUp:))]
        fn key_up(&self, event: &NSEvent) {
            let wants = self
                .focused_session()
                .is_some_and(|s| s.term.lock().unwrap().modes().kitty_flags & kitty::EVENT_TYPES != 0);
            if wants {
                self.handle_key(event, KeyKind::Release);
            }
        }

        /// Teclas modificadoras solas: solo con "todas las teclas como escapes" de kitty.
        #[unsafe(method(flagsChanged:))]
        fn flags_changed(&self, event: &NSEvent) {
            self.handle_modifier_key(event);
            let cmd = event.modifierFlags().contains(NSEventModifierFlags::Command);
            self.update_hover(cmd);
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: llamada a la implementación de NSView.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.update_layout();
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn did_change_backing(&self) {
            self.update_layout();
        }

        // --- Edición ---

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            // SAFETY: lectura del portapapeles general en el hilo principal.
            let text = unsafe {
                NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString)
            };
            if let (Some(text), Some(s)) = (text, self.focused_session()) {
                let bracketed = s.term.lock().unwrap().modes().bracketed_paste;
                self.send_input(&input::encode_paste(&text.to_string(), bracketed));
            }
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            let text = self
                .focused_session()
                .and_then(|s| s.term.lock().unwrap().selection_text());
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                copy_to_pasteboard(&text);
            }
        }

        /// Copia la salida del último comando terminado (bloques OSC 133).
        #[unsafe(method(copyLastOutput:))]
        fn copy_last_output(&self, _sender: Option<&AnyObject>) {
            let text = self.focused_session().and_then(|s| {
                let term = s.term.lock().unwrap();
                term.last_finished_block().and_then(|b| term.block_output(b))
            });
            if let Some(text) = text {
                copy_to_pasteboard(&text);
            }
        }

        // --- Bloques ---

        #[unsafe(method(previousBlock:))]
        fn previous_block(&self, _sender: Option<&AnyObject>) {
            self.step_block(crate::blocks::Step::Previous);
        }

        #[unsafe(method(nextBlock:))]
        fn next_block(&self, _sender: Option<&AnyObject>) {
            self.step_block(crate::blocks::Step::Next);
        }

        #[unsafe(method(blockStop:))]
        fn block_stop_action(&self, _sender: Option<&AnyObject>) {
            self.block_stop();
        }

        #[unsafe(method(blockRerun:))]
        fn block_rerun_action(&self, _sender: Option<&AnyObject>) {
            self.block_rerun();
        }

        #[unsafe(method(blockRerunFailed:))]
        fn block_rerun_failed_action(&self, _sender: Option<&AnyObject>) {
            self.block_rerun_failed();
        }

        #[unsafe(method(blockCopy:))]
        fn block_copy_action(&self, _sender: Option<&AnyObject>) {
            self.block_copy();
        }

        /// ⌘O: el primer `archivo:línea` de la salida del bloque, en el editor.
        #[unsafe(method(blockCollapse:))]
        fn block_collapse_action(&self, _sender: Option<&AnyObject>) {
            self.block_toggle_collapse();
        }

        #[unsafe(method(blockOpen:))]
        fn block_open_action(&self, _sender: Option<&AnyObject>) {
            self.block_open_in_editor();
        }

        // --- Pestañas y paneles ---

        /// Cmd+Shift+I: abre una issue nueva con el diagnóstico rellenado.
        #[unsafe(method(reportIssue:))]
        fn report_issue(&self, _sender: Option<&AnyObject>) {
            let url = report::issue_url(&self.diagnostics());
            open_url(&url);
        }

        #[unsafe(method(newTab:))]
        fn new_tab_action(&self, _sender: Option<&AnyObject>) {
            self.new_tab();
        }

        #[unsafe(method(splitVertical:))]
        fn split_vertical(&self, _sender: Option<&AnyObject>) {
            self.split(Axis::Columns);
        }

        #[unsafe(method(splitHorizontal:))]
        fn split_horizontal(&self, _sender: Option<&AnyObject>) {
            self.split(Axis::Rows);
        }

        /// Cmd+W: cierra el panel con el foco (y la pestaña si era el último).
        #[unsafe(method(closePane:))]
        fn close_pane_action(&self, _sender: Option<&AnyObject>) {
            if let Some(s) = self.focused_session()
                && self.confirm_close(std::slice::from_ref(&s), "este panel")
            {
                self.close_pane(s.id);
            }
        }

        #[unsafe(method(closeTab:))]
        fn close_tab_action(&self, _sender: Option<&AnyObject>) {
            let index = self.with_workspace(|w| w.active);
            self.close_tab(index);
        }

        #[unsafe(method(selectTab:))]
        fn select_tab_action(&self, sender: Option<&AnyObject>) {
            let tag = sender
                .and_then(|s| s.downcast_ref::<NSMenuItem>())
                .map_or(1, |item| item.tag());
            self.with_workspace_mut(|w| w.select((tag.max(1) - 1) as usize));
            self.update_layout();
        }

        #[unsafe(method(nextTab:))]
        fn next_tab(&self, _sender: Option<&AnyObject>) {
            self.with_workspace_mut(Workspace::next);
            self.update_layout();
        }

        #[unsafe(method(previousTab:))]
        fn previous_tab(&self, _sender: Option<&AnyObject>) {
            self.with_workspace_mut(Workspace::previous);
            self.update_layout();
        }

        /// ⌘⇧↵: el panel con el foco ocupa toda la pestaña.
        /// Vuelve a mirar el primer plano de todos los paneles.
        #[unsafe(method(recheckForeground))]
        fn recheck_foreground(&self) {
            for s in self.all_sessions() {
                s.recheck_foreground();
            }
            self.process_events();
        }

        /// Siguiente fotograma del spinner de las pestañas ocupadas.
        #[unsafe(method(spinnerTick))]
        fn spinner_tick(&self) {
            self.spin();
        }

        /// ⌘⇧P: la paleta de comandos.
        /// El sistema cambió entre claro y oscuro: con `theme = "system"`, el tema lo sigue.
        #[unsafe(method(viewDidChangeEffectiveAppearance))]
        fn view_did_change_effective_appearance(&self) {
            if self.apply_theme() {
                self.theme_changed();
            }
        }

        /// ⌘,: la ventana de ajustes.
        #[unsafe(method(openSettings:))]
        fn open_settings_action(&self, _sender: Option<&AnyObject>) {
            self.open_settings();
        }

        #[unsafe(method(settingsToggle:))]
        fn settings_toggle_action(&self, sender: Option<&AnyObject>) {
            if let Some(s) = sender {
                self.settings_toggle(s);
            }
        }

        #[unsafe(method(settingsCommit:))]
        fn settings_commit_action(&self, sender: Option<&AnyObject>) {
            if let Some(s) = sender {
                self.settings_commit(s);
            }
        }

        #[unsafe(method(settingsReset:))]
        fn settings_reset_action(&self, sender: Option<&AnyObject>) {
            if let Some(s) = sender {
                self.settings_reset(s);
            }
        }

        #[unsafe(method(settingsOpenFile:))]
        fn settings_open_file_action(&self, _sender: Option<&AnyObject>) {
            self.settings_open_file();
        }

        /// ⌘⌥P: fija o suelta el terminal rápido.
        #[unsafe(method(quickPin:))]
        fn quick_pin_action(&self, _sender: Option<&AnyObject>) {
            if self.is_quick() {
                crate::quick::toggle_pin();
            }
        }

        /// ⌘⌥W: el terminal rápido pasa a ser una ventana normal.
        #[unsafe(method(quickToWindow:))]
        fn quick_to_window_action(&self, _sender: Option<&AnyObject>) {
            if self.is_quick() {
                crate::quick::promote(self.mtm());
            }
        }

        #[unsafe(method(commandPalette:))]
        fn command_palette_action(&self, _sender: Option<&AnyObject>) {
            self.toggle_palette();
        }

        #[unsafe(method(toggleZoom:))]
        fn toggle_zoom_action(&self, _sender: Option<&AnyObject>) {
            self.with_workspace_mut(|w| w.tab_mut().toggle_zoom());
            self.update_layout();
        }

        #[unsafe(method(resizeLeft:))]
        fn resize_left_action(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Left);
        }

        #[unsafe(method(resizeRight:))]
        fn resize_right_action(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Right);
        }

        #[unsafe(method(resizeUp:))]
        fn resize_up_action(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Up);
        }

        #[unsafe(method(resizeDown:))]
        fn resize_down_action(&self, _sender: Option<&AnyObject>) {
            self.resize_pane(Direction::Down);
        }

        #[unsafe(method(focusLeft:))]
        fn focus_left(&self, _sender: Option<&AnyObject>) {
            self.move_focus(Direction::Left);
        }

        #[unsafe(method(focusRight:))]
        fn focus_right(&self, _sender: Option<&AnyObject>) {
            self.move_focus(Direction::Right);
        }

        #[unsafe(method(focusUp:))]
        fn focus_up(&self, _sender: Option<&AnyObject>) {
            self.move_focus(Direction::Up);
        }

        #[unsafe(method(focusDown:))]
        fn focus_down(&self, _sender: Option<&AnyObject>) {
            self.move_focus(Direction::Down);
        }

        // --- Ratón ---

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.handle_scroll(event);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.handle_mouse_down(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.handle_mouse_dragged(event);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            let drag = self.ivars().drag.borrow_mut().take();
            match drag {
                Some(Drag::Select(id)) if !self.ivars().drag_moved.get() => {
                    if let Some(s) = self.session(id) {
                        s.term.lock().unwrap().set_selection(None);
                        self.request_frame();
                    }
                }
                Some(Drag::Report(id, button)) => {
                    self.report_mouse(id, event, button, MouseAction::Release);
                }
                _ => {}
            }
        }

        #[unsafe(method(rightMouseDown:))]
        fn right_mouse_down(&self, event: &NSEvent) {
            self.other_button(event, MouseButton::Right, MouseAction::Press);
        }

        #[unsafe(method(rightMouseUp:))]
        fn right_mouse_up(&self, event: &NSEvent) {
            self.other_button(event, MouseButton::Right, MouseAction::Release);
        }

        #[unsafe(method(otherMouseDown:))]
        fn other_mouse_down(&self, event: &NSEvent) {
            self.other_button(event, MouseButton::Middle, MouseAction::Press);
        }

        #[unsafe(method(otherMouseUp:))]
        fn other_mouse_up(&self, event: &NSEvent) {
            self.other_button(event, MouseButton::Middle, MouseAction::Release);
        }

        #[unsafe(method(rightMouseDragged:))]
        fn right_mouse_dragged(&self, event: &NSEvent) {
            self.handle_mouse_dragged(event);
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            self.handle_mouse_moved(event);
        }
    }

    unsafe impl NSTextInputClient for TerminalView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _range: NSRange) {
            let was_composing = !self.ivars().marked.borrow().is_empty();
            self.ivars().marked.borrow_mut().clear();
            if was_composing {
                self.update_layout();
            }
            let text = text_of(string);
            if !text.is_empty() {
                self.send_input(text.as_bytes());
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Las teclas con comando (Enter, flechas…) se codifican antes, en `handle_key`.
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, _selected: NSRange, _replacement: NSRange) {
            *self.ivars().marked.borrow_mut() = text_of(string);
            // El texto en composición se dibuja en el cursor (sin enviarlo aún).
            self.update_layout();
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.ivars().marked.borrow_mut().clear();
            self.update_layout();
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(NSNotFound as NSUInteger, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let len = self.ivars().marked.borrow().encode_utf16().count();
            if len == 0 {
                NSRange::new(NSNotFound as NSUInteger, 0)
            } else {
                NSRange::new(0, len)
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            !self.ivars().marked.borrow().is_empty()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, _range: NSRange, _actual: *mut NSRange) -> NSRect {
            self.cursor_screen_rect()
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }
);

/// Abre una URL con la aplicación predeterminada de macOS.
fn open_url(url: &str) {
    if let Some(url) = objc2_foundation::NSURL::URLWithString(&NSString::from_str(url)) {
        NSWorkspace::sharedWorkspace().openURL(&url);
    }
}

fn read_pasteboard() -> Option<String> {
    // SAFETY: portapapeles general, en el hilo principal.
    unsafe { NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString) }
        .map(|s| s.to_string())
}

pub(crate) fn copy_to_pasteboard(text: &str) {
    // SAFETY: portapapeles general, en el hilo principal.
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString);
    }
}

/// Texto de un `NSString` o `NSAttributedString` recibido del sistema de entrada.
fn text_of(obj: &AnyObject) -> String {
    if let Some(s) = obj.downcast_ref::<NSString>() {
        s.to_string()
    } else if let Some(s) = obj.downcast_ref::<NSAttributedString>() {
        s.string().to_string()
    } else {
        String::new()
    }
}

impl TerminalView {
    pub fn new(
        mtm: MainThreadMarker,
        frame: NSRect,
        window: Arc<WindowShared>,
        config: Config,
        base_env: Vec<(String, String)>,
        initial_command: Option<Vec<String>>,
    ) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            window,
            waker: OnceCell::new(),
            config: RefCell::new(config),
            base_env,
            initial_command: RefCell::new(initial_command),
            workspace: RefCell::new(None),
            sessions: RefCell::new(HashMap::new()),
            next_id: Cell::new(1),
            geometry: RefCell::new(Geometry::default()),
            drag: RefCell::new(None),
            marked: RefCell::new(String::new()),
            metrics: RefCell::new(None),
            scroll_accum: Cell::new(0.0),
            drag_moved: Cell::new(false),
            hover_link: RefCell::new(None),
            last_mouse: Cell::new(None),
            last_reported: Cell::new(None),
            window_key: Cell::new(true),
            last_focus: Cell::new(None),
            spinner_phase: Cell::new(0),
            spinner_running: Cell::new(false),
            opacity: Cell::new(1.0),
            quick: Cell::new(false),
            tab_note: RefCell::new(None),
            selected_block: Cell::new(None),
            collapsed_blocks: Default::default(),
            agents: Default::default(),
            palette: Default::default(),
            settings: Default::default(),
        });
        // SAFETY: `initWithFrame:` de NSView.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// Esta vista es la del quick terminal.
    pub(crate) fn is_quick(&self) -> bool {
        self.ivars().quick.get()
    }

    /// Marca esta vista como la del quick terminal, con su opacidad de fondo.
    pub(crate) fn set_quick(&self, opacity: f32) {
        self.ivars().quick.set(true);
        self.ivars().opacity.set(opacity);
    }

    /// `(blocks…)` no: `(alto, opacidad)` del terminal rápido según la configuración.
    pub(crate) fn quick_config(&self) -> (f64, f64) {
        let c = self.ivars().config.borrow();
        (c.quick_height, c.quick_opacity)
    }

    pub(crate) fn quick_enabled(&self) -> bool {
        self.ivars().config.borrow().quick_enabled
    }

    /// Texto a la derecha de la barra de pestañas.
    pub(crate) fn set_tab_note(&self, note: Option<String>) {
        *self.ivars().tab_note.borrow_mut() = note;
        self.update_layout();
    }

    /// El terminal rápido pasa a ser una ventana normal: opaca y sin nota.
    pub(crate) fn promote_from_quick(&self) {
        self.ivars().quick.set(false);
        self.ivars().opacity.set(1.0);
        *self.ivars().tab_note.borrow_mut() = None;
        if let Some(layer) = self.layer() {
            layer.setOpaque(true);
        }
        self.update_layout();
    }

    pub(crate) fn request_frame_now(&self) {
        self.request_frame();
    }

    /// Ya no queda ningún panel: se descarta el terminal rápido (la tecla global
    /// crea otro); una ventana normal se cierra si queda otra y, si era la
    /// última, la app termina.
    fn no_panes_left(&self) {
        if self.is_quick() {
            crate::quick::dispose(self.mtm());
            return;
        }
        let app = NSApplication::sharedApplication(self.mtm());
        let mine = self.window();
        let other = app.windows().iter().any(|w| {
            w.isVisible()
                && mine.as_ref().is_none_or(|m| **m != *w)
                && crate::container::terminal_view(&w).is_some()
        });
        match (other, mine) {
            (true, Some(w)) => w.close(),
            _ => app.terminate(None),
        }
    }

    /// Arranca la primera sesión cuando el render ya existe.
    pub fn start(&self, waker: Arc<Waker>) {
        let _ = self.ivars().waker.set(waker);
        let id = self.alloc_id();
        *self.ivars().workspace.borrow_mut() = Some(Workspace::new(id));
        if !self.spawn_session(id, None) {
            self.no_panes_left();
            return;
        }
        self.update_layout();
    }

    // --- Acceso al estado ---

    fn with_workspace<R>(&self, f: impl FnOnce(&Workspace) -> R) -> R {
        f(self.ivars().workspace.borrow().as_ref().expect("workspace"))
    }

    fn with_workspace_mut<R>(&self, f: impl FnOnce(&mut Workspace) -> R) -> R {
        f(self
            .ivars()
            .workspace
            .borrow_mut()
            .as_mut()
            .expect("workspace"))
    }

    pub(crate) fn session(&self, id: PaneId) -> Option<Arc<Session>> {
        self.ivars().sessions.borrow().get(&id).cloned()
    }

    /// Bloque seleccionado (panel e identificador) y su ajuste visual.
    pub(crate) fn block_selected(&self) -> Option<(PaneId, u64)> {
        self.ivars().selected_block.get()
    }

    pub(crate) fn set_block_selected(&self, sel: Option<(PaneId, u64)>) {
        self.ivars().selected_block.set(sel);
    }

    pub(crate) fn ivars_collapsed(&self) -> &RefCell<std::collections::HashSet<(PaneId, u64)>> {
        &self.ivars().collapsed_blocks
    }

    /// `(blocks.visual, blocks.editor)`.
    pub(crate) fn block_config(&self) -> (bool, String) {
        let c = self.ivars().config.borrow();
        (c.block_visual, c.block_editor.clone())
    }

    pub(crate) fn focused_session(&self) -> Option<Arc<Session>> {
        if self.ivars().workspace.borrow().is_none() {
            return None;
        }
        self.session(self.with_workspace(Workspace::focused))
    }

    pub(crate) fn agents(&self) -> &crate::agent_controller::AgentController {
        &self.ivars().agents
    }

    /// Los atajos configurados (`[keybindings]`).
    pub(crate) fn keybindings(&self) -> std::collections::HashMap<String, String> {
        self.ivars().config.borrow().keybindings.clone()
    }

    pub(crate) fn ivars_settings(&self) -> &crate::settings_controller::SettingsController {
        &self.ivars().settings
    }

    pub(crate) fn ivars_palette(&self) -> &crate::palette_controller::PaletteController {
        &self.ivars().palette
    }

    /// Las sesiones de todos los paneles.
    pub(crate) fn all_sessions(&self) -> Vec<Arc<Session>> {
        self.ivars().sessions.borrow().values().cloned().collect()
    }

    pub(crate) fn waker_arc(&self) -> Option<Arc<Waker>> {
        self.ivars().waker.get().cloned()
    }

    pub(crate) fn focused_pane(&self) -> PaneId {
        self.with_workspace(Workspace::focused)
    }

    /// Da el foco a un panel (cambiando de pestaña si hace falta).
    pub(crate) fn focus_pane_by_id(&self, id: PaneId) -> bool {
        let ok = self.with_workspace_mut(|w| w.focus_pane(id));
        if ok {
            self.update_layout();
        }
        ok
    }

    pub(crate) fn pane_exists(&self, id: PaneId) -> bool {
        self.ivars().sessions.borrow().contains_key(&id)
    }

    pub(crate) fn user_shell(&self) -> String {
        self.ivars().config.borrow().shell()
    }

    pub(crate) fn focused_cwd(&self) -> Option<std::path::PathBuf> {
        self.focused_session().and_then(|s| s.cwd())
    }

    /// Pestaña nueva que ejecuta `command` en `cwd`. `None` si no se pudo lanzar.
    pub(crate) fn open_agent_tab(
        &self,
        command: Vec<String>,
        cwd: std::path::PathBuf,
    ) -> Option<PaneId> {
        *self.ivars().initial_command.borrow_mut() = Some(command);
        let id = self.alloc_id();
        self.with_workspace_mut(|w| w.new_tab(id));
        let ok = self.spawn_session(id, Some(cwd));
        if !ok {
            self.with_workspace_mut(|w| w.close_pane(id));
        }
        self.update_layout();
        ok.then_some(id)
    }

    /// ⌘↵ (permitir) y ⌘⌫ (denegar) con la attention bar visible.
    fn handle_agent_key(&self, event: &NSEvent) -> bool {
        let flags = event.modifierFlags();
        let only_command = flags.contains(NSEventModifierFlags::Command)
            && !flags.intersects(
                NSEventModifierFlags::Shift
                    | NSEventModifierFlags::Option
                    | NSEventModifierFlags::Control,
            );
        if !only_command || self.pending_permission().is_none() {
            return false;
        }
        match event.keyCode() {
            36 | 76 => self.allow_once(),
            51 => self.deny_with_reason(),
            _ => return false,
        }
        true
    }

    fn alloc_id(&self) -> PaneId {
        let id = self.ivars().next_id.get();
        self.ivars().next_id.set(id + 1);
        id
    }

    fn request_frame(&self) {
        if let Some(waker) = self.ivars().waker.get() {
            waker.request_frame();
        }
    }

    fn font_spec(&self) -> FontSpec {
        let c = self.ivars().config.borrow();
        FontSpec {
            family: c.font_family.clone(),
            size_pt: c.font_size,
        }
    }

    fn metrics_for(&self, scale: f64) -> Metrics {
        let spec = self.font_spec();
        if let Some((s, f, m)) = &*self.ivars().metrics.borrow()
            && *s == scale
            && *f == spec
        {
            return *m;
        }
        let m = Metrics::of(&spec.font(scale));
        *self.ivars().metrics.borrow_mut() = Some((scale, spec, m));
        m
    }

    /// Aplica una configuración nueva: lo que va en vivo se nota al momento
    /// (fuente, colores, margen, avisos); shell e historial, en pestañas nuevas.
    /// ¿Está el sistema en modo oscuro? (la apariencia de la aplicación no la
    /// tapa la de una ventana, así que sirve también con Dusk o Paper fijos).
    pub(crate) fn system_is_dark(&self) -> bool {
        NSApplication::sharedApplication(self.mtm())
            .effectiveAppearance()
            .name()
            .to_string()
            .contains("Dark")
    }

    /// Los colores de la terminal con el tema activo.
    fn effective_theme(&self) -> crate::config::Theme {
        self.ivars()
            .config
            .borrow()
            .effective_theme(crate::palette::is_paper())
    }

    /// Decide el tema (Dusk, Paper o el del sistema) y, si cambió, lo aplica a
    /// todas las ventanas. Devuelve `true` si cambió.
    pub fn apply_theme(&self) -> bool {
        let (paper, forced) = {
            let config = self.ivars().config.borrow();
            let forced = match config.appearance {
                crate::config::Appearance::System => None,
                crate::config::Appearance::Dusk => Some(false),
                crate::config::Appearance::Paper => Some(true),
            };
            (config.use_paper(self.system_is_dark()), forced)
        };
        let changed = paper != crate::palette::is_paper();
        crate::palette::set_paper(paper);
        // Las ventanas siguen al tema: oscuras con Dusk, claras con Paper, y
        // con el del sistema se deja que AppKit decida.
        let appearance: Option<Retained<NSAppearance>> = forced.and_then(|p| {
            // SAFETY: constantes de AppKit.
            NSAppearance::appearanceNamed(unsafe {
                if p {
                    NSAppearanceNameAqua
                } else {
                    NSAppearanceNameDarkAqua
                }
            })
        });
        let app = NSApplication::sharedApplication(self.mtm());
        for window in app.windows().iter() {
            window.setAppearance(appearance.as_deref());
            if changed && let Some(content) = window.contentView() {
                restyle(&content);
            }
        }
        changed
    }

    /// Cambió el tema (a mano o con el sistema): repinta lo que se fijó al
    /// crearlo.
    fn theme_changed(&self) {
        self.refresh_agent_ui();
        self.review_refresh_look();
        if let Some(page) = self.settings_current_page() {
            self.settings_show_page(page);
        }
        self.update_layout();
    }

    pub fn apply_config(&self, config: Config) {
        for s in self.ivars().sessions.borrow().values() {
            s.long_command_secs
                .store(config.long_command_seconds, SeqCst);
        }
        crate::apply_keybindings(self.mtm(), &config);
        *self.ivars().config.borrow_mut() = config;
        if self.apply_theme() {
            self.theme_changed();
        } else {
            self.update_layout();
        }
    }

    // --- Sesiones ---

    /// Lanza la shell de un panel nuevo en `cwd` con el tamaño que le toca
    /// en el layout actual. Devuelve `false` si no se pudo.
    fn spawn_session(&self, id: PaneId, cwd: Option<std::path::PathBuf>) -> bool {
        let (cols, rows) = self.compute_geometry().map_or((80, 24), |g| {
            g.panes
                .iter()
                .find(|(p, _)| *p == id)
                .map_or((80, 24), |(_, r)| pane_dims(*r, &g))
        });
        let Some(waker) = self.ivars().waker.get().cloned() else {
            return false;
        };
        let ivars = self.ivars();
        let config = ivars.config.borrow().clone();
        let mut env = ivars.base_env.clone();
        // El comando de `-e` (solo para el primer panel) o la shell.
        let (shell, args) = match ivars.initial_command.borrow_mut().take() {
            Some(mut cmd) if !cmd.is_empty() => {
                let program = cmd.remove(0);
                (program, cmd)
            }
            _ => {
                let shell = config.shell();
                if config.shell_integration {
                    let zsh = crate::shell::integration_env(&shell);
                    // Los hooks de la app para un `claude` escrito a mano
                    // (solo los usa la función de zsh).
                    if config.track_manual_claude && !zsh.is_empty() {
                        env.extend(self.manual_claude_env(id, cwd.as_deref()));
                    }
                    env.extend(zsh);
                }
                (shell, config.shell_args.clone())
            }
        };
        let opts = SpawnOptions {
            shell: &shell,
            args: &args,
            env: &env,
            cwd: cwd.as_deref(),
            history_limit: config.scrollback_lines,
            long_command_secs: config.long_command_seconds,
        };
        match Session::spawn(id, opts, cols as u16, rows as u16, waker) {
            Ok(session) => {
                ivars.sessions.borrow_mut().insert(id, session);
                true
            }
            Err(e) => {
                eprintln!("no se pudo lanzar la shell: {e}");
                false
            }
        }
    }

    fn new_tab(&self) {
        let cwd = self.focused_session().and_then(|s| s.cwd());
        let id = self.alloc_id();
        self.with_workspace_mut(|w| w.new_tab(id));
        if !self.spawn_session(id, cwd) {
            self.with_workspace_mut(|w| w.close_pane(id));
        }
        self.update_layout();
    }

    fn split(&self, axis: Axis) {
        let cwd = self.focused_session().and_then(|s| s.cwd());
        let id = self.alloc_id();
        self.with_workspace_mut(|w| {
            let target = w.focused();
            w.tab_mut().split(target, axis, id);
        });
        if !self.spawn_session(id, cwd) {
            self.with_workspace_mut(|w| w.close_pane(id));
        }
        self.update_layout();
    }

    fn move_focus(&self, dir: Direction) {
        let changed = self.with_workspace_mut(|w| {
            let next = w.tab().neighbor(dir);
            if let Some(next) = next {
                w.tab_mut().focused = next;
            }
            next.is_some()
        });
        if changed {
            self.update_layout();
        }
    }

    /// Un comando acaba de arrancar: se vuelve a mirar el primer plano un poco
    /// después, por si es un programa que no escribe nada (`ssh` conectando).
    fn schedule_foreground_rechecks(&self) {
        for delay in [0.4f64, 2.0, 5.0] {
            // SAFETY: `recheckForeground` existe en esta clase y no lleva argumentos.
            let _: () = unsafe {
                msg_send![self, performSelector: sel!(recheckForeground), withObject: None::<&AnyObject>, afterDelay: delay]
            };
        }
    }

    fn any_tab_busy(&self) -> bool {
        self.ivars()
            .sessions
            .borrow()
            .values()
            .any(|s| s.busy.load(SeqCst) && !s.exited.load(SeqCst))
    }

    /// Cambió qué pestañas tienen un programa en primer plano: se repinta la
    /// barra y, si alguna está ocupada, arranca el reloj del spinner.
    fn busy_changed(&self) {
        self.update_layout();
        if self.any_tab_busy() && !self.ivars().spinner_running.replace(true) {
            self.spin();
        }
    }

    /// Un fotograma del spinner y el siguiente 120 ms después. Sin pestañas
    /// ocupadas el reloj se detiene: en reposo no hay frames.
    fn spin(&self) {
        if !self.any_tab_busy() {
            self.ivars().spinner_running.set(false);
            return;
        }
        let phase = self.ivars().spinner_phase.get().wrapping_add(1);
        self.ivars().spinner_phase.set(phase);
        if let Some(wf) = self.ivars().window.frame.lock().unwrap().as_mut()
            && let Some(bar) = wf.tab_bar.as_mut()
        {
            bar.spinner_phase = phase;
        }
        self.request_frame();
        // SAFETY: `spinnerTick` existe en esta clase y no lleva argumentos.
        let _: () = unsafe {
            msg_send![self, performSelector: sel!(spinnerTick), withObject: None::<&AnyObject>, afterDelay: 0.12f64]
        };
    }

    /// ⌘⌃flecha: mueve el divisor más cercano del panel con el foco.
    fn resize_pane(&self, dir: Direction) {
        let moved = self.with_workspace_mut(|w| w.tab_mut().resize_focused(dir, 0.05));
        if moved {
            self.update_layout();
        }
    }

    /// Pide confirmación si alguna de las sesiones tiene un programa en marcha.
    fn confirm_close(&self, sessions: &[Arc<Session>], what: &str) -> bool {
        if !sessions.iter().any(|s| s.is_busy()) {
            return true;
        }
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(&format!("¿Cerrar {what}?")));
        alert.setInformativeText(&NSString::from_str(
            "Hay un programa en marcha. Si cierras, se terminará.",
        ));
        alert.addButtonWithTitle(&NSString::from_str("Cerrar"));
        alert.addButtonWithTitle(&NSString::from_str("Cancelar"));
        alert.runModal() == NSAlertFirstButtonReturn
    }

    /// Confirmación al salir de la app (Cmd+Q).
    pub fn confirm_quit(&self) -> bool {
        let sessions: Vec<_> = self.ivars().sessions.borrow().values().cloned().collect();
        self.confirm_close(&sessions, "todas las pestañas")
    }

    fn close_pane(&self, id: PaneId) {
        if let Some(s) = self.ivars().sessions.borrow_mut().remove(&id) {
            s.hangup();
        }
        let remaining = self.with_workspace_mut(|w| w.close_pane(id));
        if remaining {
            self.reconcile_agents();
            self.update_layout();
        } else {
            self.no_panes_left();
        }
    }

    fn close_tab(&self, index: usize) {
        let panes = self.with_workspace(|w| w.tabs.get(index).map(|t| t.panes()));
        let Some(panes) = panes else { return };
        let sessions: Vec<_> = panes.iter().filter_map(|p| self.session(*p)).collect();
        if !self.confirm_close(&sessions, "esta pestaña") {
            return;
        }
        for s in &sessions {
            self.ivars().sessions.borrow_mut().remove(&s.id);
            s.hangup();
        }
        if self.with_workspace_mut(|w| w.close_tab(index)) {
            self.reconcile_agents();
            self.update_layout();
        } else {
            self.no_panes_left();
        }
    }

    /// Una sesión cambió de título o terminó: se quitan sus paneles y se
    /// rehace el layout (títulos incluidos).
    pub fn sessions_changed(&self) {
        let exited: Vec<PaneId> = self
            .ivars()
            .sessions
            .borrow()
            .values()
            .filter(|s| s.exited.load(SeqCst))
            .map(|s| s.id)
            .collect();
        for id in exited {
            self.ivars().sessions.borrow_mut().remove(&id);
            if !self.with_workspace_mut(|w| w.close_pane(id)) {
                self.no_panes_left();
                return;
            }
        }
        self.reconcile_agents();
        self.update_layout();
    }

    // --- Layout ---

    /// Rectángulos de la barra y de los paneles para el tamaño actual.
    fn compute_geometry(&self) -> Option<Geometry> {
        let window = self.window()?;
        let workspace = self.ivars().workspace.borrow();
        let workspace = workspace.as_ref()?;
        let scale = window.backingScaleFactor();
        let size = self.bounds().size;
        let m = self.metrics_for(scale);
        let w = (size.width * scale).round().max(1.0);
        let h = (size.height * scale).round().max(1.0);
        let (cw, ch) = (m.cell_w as f64, m.cell_h as f64);

        // Barra de pestañas.
        let bar_h = (ch * 1.8).round();
        let n = workspace.tabs.len() as f64;
        let plus_w = bar_h;
        let tab_w = ((w - plus_w) / n).min(cw * 28.0).floor();
        let close_w = (cw * 2.5).round();
        let prod_hosts = self.ivars().config.borrow().production_hosts.clone();
        let sessions = self.ivars().sessions.borrow();
        let tabs = workspace
            .tabs
            .iter()
            .enumerate()
            .map(|(i, tab)| {
                let x = i as f64 * tab_w;
                let title = sessions
                    .get(&tab.focused)
                    .map(|s| s.title())
                    .unwrap_or_default();
                let busy = tab
                    .panes()
                    .iter()
                    .any(|p| sessions.get(p).is_some_and(|s| s.busy.load(SeqCst)));
                let prod = tab.panes().iter().any(|p| {
                    sessions
                        .get(p)
                        .and_then(|s| s.ssh_host())
                        .is_some_and(|h| crate::proc::host_matches(&h, &prod_hosts))
                });
                TabItem {
                    busy,
                    prod,
                    title,
                    rect: [x as f32, 0.0, tab_w as f32, bar_h as f32],
                    close: [
                        (x + tab_w - close_w) as f32,
                        0.0,
                        close_w as f32,
                        bar_h as f32,
                    ],
                    active: i == workspace.active,
                }
            })
            .collect();
        let tab_bar = TabBar {
            rect: [0.0, 0.0, w as f32, bar_h as f32],
            tabs,
            plus: [(n * tab_w) as f32, 0.0, plus_w as f32, bar_h as f32],
            spinner_phase: self.ivars().spinner_phase.get(),
            note: self.ivars().tab_note.borrow().clone(),
        };

        let area = Rect {
            x: 0.0,
            y: bar_h,
            w,
            h: (h - bar_h).max(1.0),
        };
        let (mut panes, dividers) = workspace.tab().layout(area, scale.round().max(1.0));
        // Con varios paneles (y sin zoom), cada uno lleva su cabecera de 30 pt
        // en lo alto: el panel pierde ese alto y todo lo demás (ratón,
        // tamaño de la PTY) usa el rectángulo ya recortado.
        let mut headers = Vec::new();
        if panes.len() > 1 && self.ivars().config.borrow().pane_headers {
            let header_h = (30.0 * scale).round();
            for (id, r) in &mut panes {
                if r.h > header_h * 2.0 {
                    headers.push((*id, Rect { h: header_h, ..*r }));
                    r.y += header_h;
                    r.h -= header_h;
                }
            }
        }
        Some(Geometry {
            scale,
            metrics: Some(m),
            pad: (self.ivars().config.borrow().padding * scale).round(),
            tab_bar: Some(tab_bar),
            panes,
            dividers,
            headers,
        })
    }

    /// Recalcula el layout, ajusta el layer y el tamaño de cada sesión
    /// visible, lo publica para el render y pide un frame.
    pub fn update_layout(&self) {
        let Some(g) = self.compute_geometry() else {
            return;
        };
        let size = self.bounds().size;
        let (w, h) = (
            (size.width * g.scale).round().max(1.0),
            (size.height * g.scale).round().max(1.0),
        );
        {
            let layer = self.ivars().window.layer.lock();
            layer.setContentsScale(g.scale);
            layer.setDrawableSize(CGSize::new(w, h));
        }

        let focused = self.with_workspace(Workspace::focused);
        let dim_unfocused = self.ivars().config.borrow().dim_unfocused;
        let block_visual = self.ivars().config.borrow().block_visual;
        let selected_block = self.ivars().selected_block.get();
        let collapsed = self.ivars().collapsed_blocks.borrow().clone();
        let sessions = self.ivars().sessions.borrow();
        let mut slots = Vec::new();
        for (id, r) in &g.panes {
            let Some(s) = sessions.get(id) else { continue };
            let (cols, rows) = pane_dims(*r, &g);
            s.resize(cols, rows);
            slots.push(PaneSlot {
                session: s.clone(),
                rect: [r.x as f32, r.y as f32, r.w as f32, r.h as f32],
                pad: g.pad as f32,
                focused: *id == focused,
                dim: dim_unfocused && g.panes.len() > 1 && *id != focused,
                blocks: crate::window::BlockView {
                    visual: block_visual,
                    selected: selected_block.filter(|(p, _)| p == id).map(|(_, b)| b),
                    collapsed: collapsed
                        .iter()
                        .filter(|(p, _)| p == id)
                        .map(|(_, b)| *b)
                        .collect(),
                },
                link: self
                    .ivars()
                    .hover_link
                    .borrow()
                    .filter(|(p, _)| p == id)
                    .map(|(_, r)| r),
                preedit: Some(self.ivars().marked.borrow().clone())
                    .filter(|m| *id == focused && !m.is_empty()),
            });
        }
        for s in sessions.values() {
            let visible = g.panes.iter().any(|(id, _)| *id == s.id);
            s.visible.store(visible, SeqCst);
        }
        let title = sessions.get(&focused).map(|s| s.title());
        let prod_hosts = self.ivars().config.borrow().production_hosts.clone();
        let header_prod: std::collections::HashSet<PaneId> = g
            .headers
            .iter()
            .filter(|(id, _)| {
                sessions
                    .get(id)
                    .and_then(|s| s.ssh_host())
                    .is_some_and(|h| crate::proc::host_matches(&h, &prod_hosts))
            })
            .map(|(id, _)| *id)
            .collect();
        let header_titles: std::collections::HashMap<PaneId, String> = g
            .headers
            .iter()
            .filter_map(|(id, _)| sessions.get(id).map(|s| (*id, s.title())))
            .collect();
        // Avisos de foco (`?1004`) al cambiar de panel.
        let previous = self.ivars().last_focus.replace(Some(focused));
        if previous != Some(focused) && self.ivars().window_key.get() {
            if let Some(old) = previous.and_then(|p| sessions.get(&p)) {
                old.send_focus(false);
            }
            if let Some(new) = sessions.get(&focused) {
                new.send_focus(true);
            }
        }
        drop(sessions);

        *self.ivars().window.frame.lock().unwrap() = Some(WindowFrame {
            layout: Layout {
                scale: g.scale,
                width_px: w as u32,
                height_px: h as u32,
            },
            panes: slots,
            dividers: g
                .dividers
                .iter()
                .map(|d| {
                    [
                        d.rect.x as f32,
                        d.rect.y as f32,
                        d.rect.w as f32,
                        d.rect.h as f32,
                    ]
                })
                .collect(),
            headers: g
                .headers
                .iter()
                .map(|(id, r)| crate::renderer::PaneHeader {
                    rect: [r.x as f32, r.y as f32, r.w as f32, r.h as f32],
                    title: header_titles.get(id).cloned().unwrap_or_default(),
                    focused: *id == focused,
                    prod: header_prod.contains(id),
                })
                .collect(),
            tab_bar: g.tab_bar.clone(),
            opacity: self.ivars().opacity.get(),
            theme: self.effective_theme(),
            font: self.font_spec(),
        });
        // Solo en depuración: `RT_DUMP_TABS=archivo` vuelca el estado de las pestañas.
        #[cfg(debug_assertions)]
        if let (Some(path), Some(bar)) = (std::env::var_os("RT_DUMP_TABS"), g.tab_bar.as_ref()) {
            let text: String = bar
                .tabs
                .iter()
                .map(|t| format!("{}|busy={}|prod={}\n", t.title, t.busy, t.prod))
                .collect();
            let _ = std::fs::write(path, text);
        }
        *self.ivars().geometry.borrow_mut() = g;
        if let (Some(title), Some(window)) = (title, self.window()) {
            window.setTitle(&NSString::from_str(&title));
        }
        self.request_frame();
        // El foco pudo cambiar de agente: la barra sigue al panel enfocado.
        self.refresh_agent_ui();
    }

    // --- Entrada ---

    /// Envía entrada del usuario al panel con el foco y vuelve su vista abajo.
    fn send_input(&self, bytes: &[u8]) {
        let Some(s) = self.focused_session() else {
            return;
        };
        let moved = {
            let mut term = s.term.lock().unwrap();
            let moved = term.display_offset() > 0;
            term.scroll_to_bottom();
            moved
        };
        s.write(bytes);
        if moved {
            self.request_frame();
        }
    }

    /// Posición del ratón en píxeles de la vista (origen arriba a la izquierda).
    fn mouse_px(&self, event: &NSEvent) -> (f64, f64) {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        let scale = self.ivars().geometry.borrow().scale.max(1.0);
        (p.x * scale, (self.bounds().size.height - p.y) * scale)
    }

    /// Panel bajo el punto y su rectángulo.
    fn pane_at(&self, x: f64, y: f64) -> Option<(PaneId, Rect)> {
        let g = self.ivars().geometry.borrow();
        g.panes.iter().find(|(_, r)| r.contains(x, y)).copied()
    }

    /// Fila y columna de un panel bajo el punto, sin limitar la fila (puede
    /// ser negativa o pasar del final al arrastrar fuera del panel).
    fn cell_in(&self, r: Rect, x: f64, y: f64) -> (isize, usize) {
        let g = self.ivars().geometry.borrow();
        let Some(m) = g.metrics else { return (0, 0) };
        let row = ((y - r.y - g.pad) / m.cell_h as f64).floor() as isize;
        let col = ((x - r.x - g.pad) / m.cell_w as f64).floor().max(0.0) as usize;
        // Con bloques colapsados, la fila de pantalla no es la de la vista.
        let row = match g.panes.iter().find(|(_, pr)| *pr == r).map(|(id, _)| *id) {
            Some(id) if row >= 0 => self
                .collapsed_row_of(id, row as usize)
                .map_or(row, |v| v as isize),
            _ => row,
        };
        (row, col)
    }

    /// Fila de la vista que hay en la fila de pantalla `screen_row` de un
    /// panel con bloques colapsados (`None`: no hay ninguno).
    fn collapsed_row_of(&self, pane: PaneId, screen_row: usize) -> Option<usize> {
        let ids: Vec<u64> = self
            .ivars()
            .collapsed_blocks
            .borrow()
            .iter()
            .filter(|(p, _)| *p == pane)
            .map(|(_, b)| *b)
            .collect();
        if ids.is_empty() {
            return None;
        }
        let session = self.session(pane)?;
        let term = session.term.lock().unwrap();
        let map = crate::window::block_row_map(&term, term.grid().rows(), &ids);
        (!map.is_identity()).then(|| map.view_of(screen_row))
    }

    fn handle_scroll(&self, event: &NSEvent) {
        let (x, y) = self.mouse_px(event);
        let Some(s) = self
            .pane_at(x, y)
            .and_then(|(id, _)| self.session(id))
            .or_else(|| self.focused_session())
        else {
            return;
        };
        let ch_pt = {
            let g = self.ivars().geometry.borrow();
            g.metrics
                .map_or(16.0, |m| m.cell_h as f64 / g.scale.max(1.0))
        };
        let dy = event.scrollingDeltaY();
        let lines = if event.hasPreciseScrollingDeltas() {
            let acc = self.ivars().scroll_accum.get() + dy / ch_pt;
            let whole = acc.trunc();
            self.ivars().scroll_accum.set(acc - whole);
            whole as isize
        } else {
            (dy * 3.0).round() as isize
        };
        if lines == 0 {
            return;
        }
        let mut term = s.term.lock().unwrap();
        let modes = term.modes();
        let shift = event.modifierFlags().contains(NSEventModifierFlags::Shift);
        if modes.mouse != MouseMode::Off && !shift {
            drop(term);
            let button = if lines > 0 {
                MouseButton::WheelUp
            } else {
                MouseButton::WheelDown
            };
            if let Some((id, _)) = self.pane_at(x, y) {
                for _ in 0..lines.unsigned_abs() {
                    self.report_mouse(id, event, button, MouseAction::Press);
                }
            }
        } else if modes.alt_screen && modes.alt_scroll {
            // En la pantalla alternativa (less, vim…) la rueda envía flechas.
            drop(term);
            let key = if lines > 0 {
                input::Key::Up
            } else {
                input::Key::Down
            };
            let bytes = input::encode_key(key, Mods::default(), modes.app_cursor);
            s.write(&bytes.repeat(lines.unsigned_abs()));
        } else {
            term.scroll_display(lines);
            drop(term);
            self.request_frame();
        }
    }

    fn handle_mouse_down(&self, event: &NSEvent) {
        let (x, y) = self.mouse_px(event);

        // Barra de pestañas.
        let bar = self.ivars().geometry.borrow().tab_bar.clone();
        if let Some(bar) = bar
            && y < bar.rect[3] as f64
        {
            let inside = |r: [f32; 4]| {
                x >= r[0] as f64
                    && x < (r[0] + r[2]) as f64
                    && y >= r[1] as f64
                    && y < (r[1] + r[3]) as f64
            };
            if inside(bar.plus) {
                self.new_tab();
            } else if let Some(i) = bar.tabs.iter().position(|t| inside(t.rect)) {
                if inside(bar.tabs[i].close) {
                    self.close_tab(i);
                } else {
                    self.with_workspace_mut(|w| w.select(i));
                    self.update_layout();
                }
            }
            return;
        }

        // Divisor (con un margen para que sea fácil de agarrar).
        let grab = self.ivars().geometry.borrow().scale * 4.0;
        let divider = self
            .ivars()
            .geometry
            .borrow()
            .dividers
            .iter()
            .find(|d| {
                let r = d.rect;
                match d.axis {
                    Axis::Columns => {
                        (x - (r.x + r.w / 2.0)).abs() <= grab && y >= r.y && y < r.y + r.h
                    }
                    Axis::Rows => {
                        (y - (r.y + r.h / 2.0)).abs() <= grab && x >= r.x && x < r.x + r.w
                    }
                }
            })
            .cloned();
        if let Some(d) = divider {
            *self.ivars().drag.borrow_mut() = Some(Drag::Divider(d));
            return;
        }

        // Panel: foco y selección.
        let Some((id, rect)) = self.pane_at(x, y) else {
            return;
        };
        if self.with_workspace(Workspace::focused) != id {
            self.with_workspace_mut(|w| w.tab_mut().focused = id);
            self.update_layout();
        }
        let Some(s) = self.session(id) else { return };
        let flags = event.modifierFlags();
        // Cmd+clic abre el enlace bajo el ratón.
        if flags.contains(NSEventModifierFlags::Command) {
            if let Some(url) = self.link_under(id, rect, x, y).map(|(u, _)| u) {
                open_url(&url);
            }
            return;
        }
        // Si la aplicación quiere el ratón, se lo damos (Shift lo evita para seleccionar).
        let mouse_mode = s.term.lock().unwrap().modes().mouse;
        if mouse_mode != MouseMode::Off && !flags.contains(NSEventModifierFlags::Shift) {
            self.ivars().last_reported.set(None);
            self.report_mouse(id, event, MouseButton::Left, MouseAction::Press);
            *self.ivars().drag.borrow_mut() = Some(Drag::Report(id, MouseButton::Left));
            return;
        }
        let (row, col) = self.cell_in(rect, x, y);
        let clicks = event.clickCount();
        {
            let mut term = s.term.lock().unwrap();
            let rows = term.grid().rows() as isize;
            let line = term.visible_line(row.clamp(0, rows - 1) as usize);
            let at = selection::point(line, col);
            let kind = match clicks {
                2 => SelectionKind::Word,
                n if n >= 3 => SelectionKind::Line,
                _ if flags.contains(NSEventModifierFlags::Option) => SelectionKind::Block,
                _ => SelectionKind::Simple,
            };
            let selection = match term.selection() {
                Some(mut sel) if flags.contains(NSEventModifierFlags::Shift) => {
                    sel.head = at;
                    sel
                }
                _ => Selection::new(kind, at),
            };
            term.set_selection(Some(selection));
        }
        *self.ivars().drag.borrow_mut() = Some(Drag::Select(id));
        // Doble y triple clic seleccionan sin necesidad de arrastrar.
        self.ivars()
            .drag_moved
            .set(clicks > 1 || flags.contains(NSEventModifierFlags::Shift));
        self.request_frame();
    }

    fn handle_mouse_dragged(&self, event: &NSEvent) {
        let (x, y) = self.mouse_px(event);
        let drag = self.ivars().drag.borrow();
        match drag.as_ref() {
            Some(Drag::Divider(d)) => {
                let d = d.clone();
                drop(drag);
                let pos = match d.axis {
                    Axis::Columns => x,
                    Axis::Rows => y,
                };
                self.with_workspace_mut(|w| w.tab_mut().drag_divider(&d, pos));
                self.update_layout();
            }
            Some(Drag::Select(id)) => {
                let id = *id;
                drop(drag);
                let rect = self
                    .ivars()
                    .geometry
                    .borrow()
                    .panes
                    .iter()
                    .find(|(p, _)| *p == id)
                    .map(|(_, r)| *r);
                let (Some(rect), Some(s)) = (rect, self.session(id)) else {
                    return;
                };
                let (row, col) = self.cell_in(rect, x, y);
                {
                    let mut term = s.term.lock().unwrap();
                    let rows = term.grid().rows() as isize;
                    // Arrastrar por encima o por debajo desplaza la vista.
                    if row < 0 {
                        term.scroll_display(1);
                    } else if row >= rows {
                        term.scroll_display(-1);
                    }
                    let line = term.visible_line(row.clamp(0, rows - 1) as usize);
                    if let Some(mut sel) = term.selection() {
                        sel.head = selection::point(line, col);
                        term.set_selection(Some(sel));
                    }
                }
                self.ivars().drag_moved.set(true);
                self.request_frame();
            }
            Some(Drag::Report(id, button)) => {
                let (id, button) = (*id, *button);
                drop(drag);
                let mode = self
                    .session(id)
                    .map(|s| s.term.lock().unwrap().modes().mouse);
                if matches!(mode, Some(MouseMode::Drag | MouseMode::Motion)) {
                    self.report_mouse(id, event, button, MouseAction::Motion);
                }
            }
            None => {}
        }
    }

    /// Botón derecho o central: a la aplicación si quiere el ratón.
    fn other_button(&self, event: &NSEvent, button: MouseButton, action: MouseAction) {
        let (x, y) = self.mouse_px(event);
        let Some((id, _)) = self.pane_at(x, y) else {
            return;
        };
        let wants = self
            .session(id)
            .is_some_and(|s| s.term.lock().unwrap().modes().mouse != MouseMode::Off);
        if wants && !event.modifierFlags().contains(NSEventModifierFlags::Shift) {
            self.ivars().last_reported.set(None);
            self.report_mouse(id, event, button, action);
            *self.ivars().drag.borrow_mut() = match action {
                MouseAction::Press => Some(Drag::Report(id, button)),
                _ => None,
            };
        }
    }

    /// Envía un evento de ratón a la aplicación del panel `id`.
    fn report_mouse(&self, id: PaneId, event: &NSEvent, button: MouseButton, action: MouseAction) {
        let (x, y) = self.mouse_px(event);
        let rect = self
            .ivars()
            .geometry
            .borrow()
            .panes
            .iter()
            .find(|(p, _)| *p == id)
            .map(|(_, r)| *r);
        let (Some(rect), Some(s)) = (rect, self.session(id)) else {
            return;
        };
        let (row, col) = self.cell_in(rect, x, y);
        let (modes, cols, rows) = {
            let term = s.term.lock().unwrap();
            (term.modes(), term.grid().cols(), term.grid().rows())
        };
        let (col, row) = (col.min(cols - 1), row.clamp(0, rows as isize - 1) as usize);
        if action == MouseAction::Motion {
            // Solo un evento por celda.
            if self.ivars().last_reported.get() == Some((id, col, row)) {
                return;
            }
        }
        self.ivars().last_reported.set(Some((id, col, row)));
        let flags = event.modifierFlags();
        let mods = Mods {
            shift: flags.contains(NSEventModifierFlags::Shift),
            alt: flags.contains(NSEventModifierFlags::Option),
            ctrl: flags.contains(NSEventModifierFlags::Control),
        };
        if let Some(bytes) = input::encode_mouse(button, action, col, row, mods, modes.mouse_sgr) {
            s.write(&bytes);
        }
    }

    fn handle_mouse_moved(&self, event: &NSEvent) {
        let (x, y) = self.mouse_px(event);
        self.ivars().last_mouse.set(Some((x, y)));
        let cmd = event
            .modifierFlags()
            .contains(NSEventModifierFlags::Command);
        self.update_hover(cmd);
        // `?1003`: cualquier movimiento, sin botón.
        if let Some((id, _)) = self.pane_at(x, y)
            && self
                .session(id)
                .is_some_and(|s| s.term.lock().unwrap().modes().mouse == MouseMode::Motion)
        {
            self.report_mouse(id, event, MouseButton::None, MouseAction::Motion);
        }
    }

    /// Enlace (OSC 8 o URL) bajo un punto de un panel.
    fn link_under(
        &self,
        id: PaneId,
        rect: Rect,
        x: f64,
        y: f64,
    ) -> Option<(String, SelectionRange)> {
        let s = self.session(id)?;
        let (row, col) = self.cell_in(rect, x, y);
        let term = s.term.lock().unwrap();
        if row < 0 || row as usize >= term.grid().rows() || col >= term.grid().cols() {
            return None;
        }
        let line = term.visible_line(row as usize);
        term.link_at(selection::point(line, col))
    }

    /// Con Cmd pulsado, subraya el enlace bajo el ratón y pone la mano.
    fn update_hover(&self, cmd: bool) {
        let hover = match (cmd, self.ivars().last_mouse.get()) {
            (true, Some((x, y))) => self
                .pane_at(x, y)
                .and_then(|(id, r)| self.link_under(id, r, x, y).map(|(_, range)| (id, range))),
            _ => None,
        };
        if *self.ivars().hover_link.borrow() != hover {
            if hover.is_some() {
                NSCursor::pointingHandCursor().set();
            } else {
                NSCursor::arrowCursor().set();
            }
            *self.ivars().hover_link.borrow_mut() = hover;
            self.update_layout();
        }
    }

    /// Diagnóstico para un informe de fallo (panel con el foco).
    fn diagnostics(&self) -> report::Diagnostics {
        let macos = objc2_foundation::NSProcessInfo::processInfo()
            .operatingSystemVersionString()
            .to_string();
        let size = self.bounds().size;
        let (tabs, panes) = self.with_workspace(|w| (w.tabs.len(), w.tab().panes().len()));
        let scale = self.ivars().geometry.borrow().scale;
        let spec = self.font_spec();
        let mut d = report::Diagnostics {
            macos,
            window: format!(
                "{:.0}×{:.0} pt, escala {scale}; {tabs} pestañas, {panes} paneles en la activa",
                size.width, size.height
            ),
            font: format!(
                "{} {} pt",
                spec.family.as_deref().unwrap_or("SF Mono (sistema)"),
                spec.size_pt
            ),
            unhandled: rust_terminal_core::recent_unhandled(),
            ..Default::default()
        };
        if let Some(s) = self.focused_session() {
            let term = s.term.lock().unwrap();
            let m = term.modes();
            d.pane = format!(
                "{}×{}, pantalla alternativa: {}, ratón: {:?}, teclado kitty: {}, historial: {} líneas, título: {}",
                term.grid().cols(),
                term.grid().rows(),
                if m.alt_screen { "sí" } else { "no" },
                m.mouse,
                m.kitty_flags,
                term.grid().history_len(),
                term.title().unwrap_or("-")
            );
            d.last_command = term
                .blocks()
                .iter()
                .rev()
                .find(|b| b.command.is_some())
                .map(|b| {
                    let cmd = b.command.as_deref().unwrap_or_default();
                    match (b.exit_code, b.duration()) {
                        (Some(code), Some(t)) => {
                            format!("`{cmd}` → código {code} ({:.1} s)", t.as_secs_f64())
                        }
                        (Some(code), None) => format!("`{cmd}` → código {code}"),
                        _ => format!("`{cmd}` (en marcha)"),
                    }
                });
            d.screen = (0..term.grid().rows())
                .map(|i| {
                    let line: String = term
                        .visible_row(i)
                        .iter()
                        .filter(|c| !c.flags.contains(rust_terminal_core::Flags::WIDE_SPACER))
                        .map(|c| c.ch)
                        .collect();
                    line.trim_end().to_owned()
                })
                .collect::<Vec<_>>()
                .join("\n")
                .trim_end()
                .to_owned();
        }
        d
    }

    /// `clipboard.read = "ask"`: pregunta si se permite leer el portapapeles.
    fn ask_clipboard_read(&self, who: &str) -> bool {
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str(&format!(
            "«{who}» quiere leer el portapapeles"
        )));
        alert.setInformativeText(&NSString::from_str(
            "Un programa (quizá remoto, por ssh) pide el contenido que has copiado. Permítelo solo si confías en él.",
        ));
        alert.addButtonWithTitle(&NSString::from_str("Denegar"));
        alert.addButtonWithTitle(&NSString::from_str("Permitir"));
        alert.runModal() != NSAlertFirstButtonReturn
    }

    /// Vuelve a leer la configuración (el archivo cambió). Con un error de
    /// sintaxis se conserva la anterior; con opciones no válidas, se ignoran
    /// solo esas. En ambos casos se muestra un aviso.
    pub fn reload_config(&self) {
        match Config::load() {
            Ok((config, errors)) => {
                self.apply_config(config);
                if !errors.is_empty() {
                    self.show_config_errors(&errors);
                }
            }
            Err(e) => self.show_config_errors(&[e]),
        }
    }

    /// Hoja de aviso (no bloqueante) con los errores de la configuración.
    pub fn show_config_errors(&self, errors: &[String]) {
        let Some(window) = self.window() else { return };
        if let Some(sheet) = window.attachedSheet() {
            window.endSheet(&sheet);
        }
        let alert = NSAlert::new(self.mtm());
        alert.setMessageText(&NSString::from_str("Hay errores en la configuración"));
        let list: Vec<String> = errors.iter().map(|e| format!("• {e}")).collect();
        alert.setInformativeText(&NSString::from_str(&format!(
            "{}\n\n{}\n\nSe han ignorado; el resto de la configuración está aplicada.",
            Config::path().display(),
            list.join("\n")
        )));
        alert.beginSheetModalForWindow_completionHandler(&window, None);
    }

    /// Atiende los eventos de las sesiones: portapapeles (OSC 52),
    /// notificaciones (OSC 9/777/99 y comandos largos) y campana.
    pub fn process_events(&self) {
        let sessions: Vec<_> = self.ivars().sessions.borrow().values().cloned().collect();
        let away = !self.ivars().window_key.get();
        let config = self.ivars().config.borrow().clone();
        let notify = away && config.notifications;
        for s in sessions {
            for event in s.take_events() {
                match event {
                    AppEvent::Term(TermEvent::Clipboard(text)) if config.clipboard_write => {
                        copy_to_pasteboard(&text)
                    }
                    AppEvent::Term(TermEvent::ClipboardRequest { selector }) => {
                        let allowed = match config.clipboard_read {
                            ClipboardRead::Deny => false,
                            ClipboardRead::Allow => true,
                            ClipboardRead::Ask => self.ask_clipboard_read(&s.title()),
                        };
                        if allowed {
                            let text = read_pasteboard().unwrap_or_default();
                            s.write(&Term::clipboard_response(&selector, &text));
                        }
                    }
                    AppEvent::Term(TermEvent::Notification { title, body }) if notify => {
                        notify::notify(&title.unwrap_or_else(|| s.title()), &body);
                    }
                    AppEvent::CommandFinished {
                        command,
                        exit_code,
                        seconds,
                    } if notify => {
                        let title = match exit_code {
                            Some(0) | None => "Comando terminado".to_owned(),
                            Some(code) => format!("Comando fallido (código {code})"),
                        };
                        notify::notify(&title, &format!("{command} · {seconds} s"));
                    }
                    AppEvent::Busy(running) => {
                        if running {
                            self.schedule_foreground_rechecks();
                        }
                        self.busy_changed()
                    }
                    // Otro host en primer plano: el distintivo PROD de la pestaña.
                    AppEvent::Ssh(_) => self.update_layout(),
                    AppEvent::Claude(running) if config.track_manual_claude => {
                        self.claude_foreground(s.id, running);
                    }
                    AppEvent::Bell if away && config.bell_dock_bounce => {
                        NSApplication::sharedApplication(self.mtm())
                            .requestUserAttention(NSRequestUserAttentionType::InformationalRequest);
                    }
                    _ => {}
                }
            }
        }
    }

    /// La ventana pasó a ser (o dejó de ser) la activa.
    pub fn window_focus_changed(&self, key: bool) {
        self.ivars().window_key.set(key);
        if let Some(s) = self.focused_session() {
            s.send_focus(key);
        }
    }

    /// Rectángulo del cursor del panel con el foco en coordenadas de
    /// pantalla, para situar la ventana de composición de los métodos de entrada.
    fn cursor_screen_rect(&self) -> NSRect {
        let (Some(window), Some(s)) = (self.window(), self.focused_session()) else {
            return NSRect::ZERO;
        };
        let g = self.ivars().geometry.borrow();
        let (Some(m), Some((_, r))) = (g.metrics, g.panes.iter().find(|(p, _)| *p == s.id)) else {
            return NSRect::ZERO;
        };
        let cursor = s.term.lock().unwrap().cursor();
        let scale = g.scale.max(1.0);
        let x_px = r.x + g.pad + cursor.col as f64 * m.cell_w as f64;
        let y_px = r.y + g.pad + (cursor.row + 1) as f64 * m.cell_h as f64;
        let rect = NSRect::new(
            NSPoint::new(x_px / scale, self.bounds().size.height - y_px / scale),
            NSSize::new(m.cell_w as f64 / scale, m.cell_h as f64 / scale),
        );
        let in_window = self.convertRect_toView(rect, None);
        window.convertRectToScreen(in_window)
    }

    fn handle_key(&self, event: &NSEvent, kind: KeyKind) {
        let flags = event.modifierFlags();
        let raw = flags.0;
        // Bits de dispositivo: Option izquierda (0x20) y derecha (0x40).
        // La izquierda actúa como Meta; la derecha compone (@, #…).
        let meta = raw & DEVICE_LEFT_ALT != 0 && raw & DEVICE_RIGHT_ALT == 0;
        let mods = Mods {
            shift: flags.contains(NSEventModifierFlags::Shift),
            alt: meta,
            ctrl: flags.contains(NSEventModifierFlags::Control),
        };
        let cmd = flags.contains(NSEventModifierFlags::Command);
        // Teclear quita la selección de bloque (y su barra).
        if !cmd && kind == KeyKind::Press && self.ivars().selected_block.get().is_some() {
            self.clear_block_selection();
        }
        let composing = !self.ivars().marked.borrow().is_empty();
        let Some(session) = self.focused_session() else {
            return;
        };
        let modes = session.term.lock().unwrap().modes();
        if cmd && kind != KeyKind::Release {
            // Los atajos con Cmd los resuelve el menú; el resto se ignora.
            return;
        }
        let special = input::key_from_keycode(event.keyCode());
        // Carácter de la tecla sin modificadores (en el layout actual) y con Shift.
        let base = event
            .charactersByApplyingModifiers(NSEventModifierFlags::empty())
            .and_then(|s| s.to_string().chars().next());
        let ignoring = event
            .charactersIgnoringModifiers()
            .map(|s| s.to_string())
            .unwrap_or_default();

        if modes.kitty_flags != 0 && !composing {
            let key = match (special, base) {
                (Some(k), _) => Some(KittyKey::Special(k)),
                (None, Some(c)) if !c.is_control() => {
                    Some(KittyKey::Char(c.to_lowercase().next().unwrap_or(c)))
                }
                _ => None,
            };
            if let Some(key) = key {
                let text = if mods.ctrl || meta {
                    None
                } else {
                    event.characters().map(|s| s.to_string())
                };
                let ev = KeyEvent {
                    key,
                    mods: KittyMods {
                        shift: mods.shift,
                        alt: meta,
                        ctrl: mods.ctrl,
                        super_: cmd,
                        caps_lock: flags.contains(NSEventModifierFlags::CapsLock),
                    },
                    kind,
                    shifted: ignoring.chars().next(),
                    text,
                };
                if let Some(bytes) = input::encode_kitty(&ev, modes.kitty_flags) {
                    if !bytes.is_empty() {
                        if kind == KeyKind::Release {
                            session.write(&bytes);
                        } else {
                            self.send_input(&bytes);
                        }
                    }
                    return;
                }
            }
        }
        if kind == KeyKind::Release {
            return;
        }

        if !composing {
            if let Some(key) = special {
                self.send_input(&input::encode_key(key, mods, modes.app_cursor));
                return;
            }
            let c = ignoring.chars().next();
            if let Some(c) = c
                && let Some(bytes) =
                    input::encode_modify_other_keys(c, mods, modes.modify_other_keys)
            {
                self.send_input(&bytes);
                return;
            }
            if mods.ctrl
                && let Some(byte) = c.and_then(input::ctrl_byte)
            {
                if meta {
                    self.send_input(&[0x1b, byte]);
                } else {
                    self.send_input(&[byte]);
                }
                return;
            }
            if meta && !ignoring.is_empty() {
                self.send_input(&input::encode_meta(&ignoring));
                return;
            }
        }
        // Texto normal, Option derecha que compone y teclas muertas: los
        // resuelve el sistema de entrada y llegan por `insertText:` o `setMarkedText:`.
        let events = NSArray::from_slice(&[event]);
        self.interpretKeyEvents(&events);
    }

    /// Pulsar o soltar una tecla modificadora sola (protocolo de kitty).
    fn handle_modifier_key(&self, event: &NSEvent) {
        let Some(session) = self.focused_session() else {
            return;
        };
        let modes = session.term.lock().unwrap().modes();
        if modes.kitty_flags & kitty::ALL_AS_ESCAPES == 0 {
            return;
        }
        use input::modifier_codes as mc;
        let raw = event.modifierFlags().0;
        let (code, bit) = match event.keyCode() {
            56 => (mc::LEFT_SHIFT, 0x02),
            60 => (mc::RIGHT_SHIFT, 0x04),
            59 => (mc::LEFT_CONTROL, 0x01),
            62 => (mc::RIGHT_CONTROL, 0x2000),
            58 => (mc::LEFT_ALT, DEVICE_LEFT_ALT),
            61 => (mc::RIGHT_ALT, DEVICE_RIGHT_ALT),
            55 => (mc::LEFT_SUPER, 0x08),
            54 => (mc::RIGHT_SUPER, 0x10),
            57 => (mc::CAPS_LOCK, NSEventModifierFlags::CapsLock.0),
            _ => return,
        };
        let flags = event.modifierFlags();
        let kind = if raw & bit != 0 {
            KeyKind::Press
        } else {
            KeyKind::Release
        };
        let ev = KeyEvent {
            key: KittyKey::Modifier(code),
            mods: KittyMods {
                shift: flags.contains(NSEventModifierFlags::Shift),
                alt: flags.contains(NSEventModifierFlags::Option),
                ctrl: flags.contains(NSEventModifierFlags::Control),
                super_: flags.contains(NSEventModifierFlags::Command),
                caps_lock: flags.contains(NSEventModifierFlags::CapsLock),
            },
            kind,
            shifted: None,
            text: None,
        };
        if let Some(bytes) = input::encode_kitty(&ev, modes.kitty_flags)
            && !bytes.is_empty()
        {
            session.write(&bytes);
        }
    }
}

/// Bits de dispositivo de `modifierFlags` (NX_DEVICELALTKEYMASK…).
const DEVICE_LEFT_ALT: usize = 0x20;
const DEVICE_RIGHT_ALT: usize = 0x40;

/// Columnas y filas que caben en un panel.
fn pane_dims(r: Rect, g: &Geometry) -> (usize, usize) {
    let Some(m) = g.metrics else { return (80, 24) };
    let cols = ((r.w - 2.0 * g.pad) / m.cell_w as f64).floor().max(1.0) as usize;
    let rows = ((r.h - 2.0 * g.pad) / m.cell_h as f64).floor().max(1.0) as usize;
    (cols, rows)
}

/// Repinta un árbol de vistas con el tema nuevo: las que llevan apariencia
/// propia (las de la app) la reciben de nuevo y todas se vuelven a dibujar.
fn restyle(view: &NSView) {
    if view.appearance().is_some() {
        view.setAppearance(crate::palette::ui_appearance().as_deref());
    }
    view.setNeedsDisplay(true);
    for sub in view.subviews().iter() {
        restyle(&sub);
    }
}
