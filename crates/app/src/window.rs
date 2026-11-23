//! Render de una ventana: un hilo propio con su `NSRunLoop` y un
//! `CADisplayLink`.
//!
//! El hilo principal publica un [`WindowFrame`]: qué paneles hay, dónde van,
//! los divisores y la barra de pestañas. En cada tick, el render copia las
//! filas sucias de cada panel (bloqueo breve de su `Term`) y pinta. Si no hay
//! cambios, pausa el display link: en reposo no hay frames.

use std::collections::HashMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use objc2::rc::Retained;
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::{
    MainThreadMarker, NSDefaultRunLoopMode, NSObject, NSObjectNSThreadPerformAdditions, NSPort,
    NSRunLoop, NSThread,
};
use objc2_quartz_core::{CADisplayLink, CAMetalLayer};
use rust_terminal_core::{Cursor, Modes, SelectionRange, Term};

use crate::config::Theme;
use crate::font::FontSpec;
use crate::renderer::{Frame, Layout, Overlay, PaneFrame, Renderer, TabBar};
use crate::session::Session;

/// Un panel visible: su sesión, su rectángulo en píxeles y si tiene el foco.
#[derive(Clone)]
pub struct PaneSlot {
    pub session: Arc<Session>,
    pub rect: [f32; 4],
    pub pad: f32,
    pub focused: bool,
    /// Enlace bajo el ratón con Cmd (se subraya).
    pub link: Option<SelectionRange>,
}

/// Lo que publica el hilo principal para el render.
#[derive(Clone)]
pub struct WindowFrame {
    pub layout: Layout,
    pub panes: Vec<PaneSlot>,
    pub dividers: Vec<[f32; 4]>,
    pub tab_bar: Option<TabBar>,
    pub theme: Theme,
    pub font: FontSpec,
}

/// `CAMetalLayer` compartido entre el hilo principal (tamaño y escala) y el
/// de render (drawables). Ambos lo usan con el mutex tomado, como en el
/// ejemplo de Apple de render en un hilo propio.
pub struct SharedLayer(Mutex<Retained<CAMetalLayer>>);

// SAFETY: el layer solo se usa a través del mutex, nunca desde dos hilos a la vez.
unsafe impl Send for SharedLayer {}
unsafe impl Sync for SharedLayer {}

impl SharedLayer {
    pub fn lock(&self) -> std::sync::MutexGuard<'_, Retained<CAMetalLayer>> {
        self.0.lock().unwrap()
    }
}

/// Estado de la ventana compartido entre el hilo principal y el de render.
pub struct WindowShared {
    pub layer: SharedLayer,
    pub frame: Mutex<Option<WindowFrame>>,
    /// Hay algo nuevo que pintar.
    dirty: AtomicBool,
    /// El display link está activo (o va a reactivarse).
    running: AtomicBool,
    frames: AtomicU64,
}

impl WindowShared {
    pub fn new(layer: Retained<CAMetalLayer>) -> Arc<WindowShared> {
        Arc::new(WindowShared {
            layer: SharedLayer(Mutex::new(layer)),
            frame: Mutex::new(None),
            dirty: AtomicBool::new(true),
            running: AtomicBool::new(true),
            frames: AtomicU64::new(0),
        })
    }
}

/// Selección y marcas de bloque en coordenadas del panel.
fn overlay(term: &Term, rows: usize, cols: usize, link: Option<SelectionRange>) -> Overlay {
    let mut overlay = Overlay::default();
    if let Some(range) = link {
        for row in 0..rows {
            if let Some((a, b)) = range.cols_on(term.visible_line(row), cols) {
                overlay.links.push((row, a, b));
            }
        }
    }
    if let Some(range) = term.selection_range() {
        for row in 0..rows {
            if let Some((a, b)) = range.cols_on(term.visible_line(row), cols) {
                overlay.selection.push((row, a, b));
            }
        }
    }
    let (top, bottom) = (term.visible_line(0), term.visible_line(rows - 1));
    for block in term.blocks() {
        let line = block.prompt.line;
        if (top..=bottom).contains(&line) {
            let failed = block.exit_code.is_some_and(|c| c != 0);
            overlay.marks.push(((line - top) as usize, failed));
        }
    }
    overlay
}

struct SendLink(Retained<CADisplayLink>);
// SAFETY: el display link solo se toca desde el hilo de render tras `attachLink:`.
unsafe impl Send for SendLink {}

pub struct TargetIvars {
    window: Arc<WindowShared>,
    renderer: Mutex<Option<Renderer>>,
    link: Mutex<Option<SendLink>>,
    /// Último cursor y modos de cada panel, para repetirlos durante DEC 2026.
    held: Mutex<HashMap<u64, (Cursor, Modes)>>,
}

define_class!(
    // SAFETY: NSObject no impone requisitos a sus subclases y `RenderTarget`
    // no implementa `Drop`.
    #[unsafe(super(NSObject))]
    #[name = "RTRenderTarget"]
    #[ivars = TargetIvars]
    pub struct RenderTarget;

    impl RenderTarget {
        /// Callback del `CADisplayLink` (hilo de render).
        #[unsafe(method(tick:))]
        fn tick(&self, link: &CADisplayLink) {
            objc2::rc::autoreleasepool(|_| self.on_tick(link));
        }

        /// Añade el display link al run loop del hilo actual (el de render).
        #[unsafe(method(attachLink:))]
        fn attach_link(&self, link: &CADisplayLink) {
            // SAFETY: run loop del hilo actual y modo por defecto.
            unsafe { link.addToRunLoop_forMode(&NSRunLoop::currentRunLoop(), NSDefaultRunLoopMode) };
            *self.ivars().link.lock().unwrap() = Some(SendLink(link.retain()));
        }

        /// Reanuda el display link (hilo de render).
        #[unsafe(method(wake))]
        fn wake(&self) {
            if let Some(link) = &*self.ivars().link.lock().unwrap() {
                link.0.setPaused(false);
            }
        }

        /// Hay eventos de sesiones: portapapeles, notificaciones… (hilo principal).
        #[unsafe(method(sessionEvents))]
        fn session_events(&self) {
            let mtm = MainThreadMarker::new().expect("hilo principal");
            for window in NSApplication::sharedApplication(mtm).windows().iter() {
                if let Some(view) = window
                    .contentView()
                    .and_then(|v| v.downcast::<crate::view::TerminalView>().ok())
                {
                    view.process_events();
                }
            }
        }

        /// Cambió el archivo de configuración (hilo principal).
        #[unsafe(method(configChanged))]
        fn config_changed(&self) {
            let mtm = MainThreadMarker::new().expect("hilo principal");
            for window in NSApplication::sharedApplication(mtm).windows().iter() {
                if let Some(view) = window
                    .contentView()
                    .and_then(|v| v.downcast::<crate::view::TerminalView>().ok())
                {
                    view.reload_config();
                }
            }
        }

        /// Cambió el título de una sesión o terminó alguna (hilo principal).
        #[unsafe(method(sessionsChanged))]
        fn sessions_changed(&self) {
            let mtm = MainThreadMarker::new().expect("hilo principal");
            for window in NSApplication::sharedApplication(mtm).windows().iter() {
                if let Some(view) = window
                    .contentView()
                    .and_then(|v| v.downcast::<crate::view::TerminalView>().ok())
                {
                    view.sessions_changed();
                }
            }
        }
    }
);

// SAFETY: los ivars son `Send + Sync` (Arc y Mutex con contenido `Send`).
unsafe impl Send for RenderTarget {}
unsafe impl Sync for RenderTarget {}

impl RenderTarget {
    fn new(window: Arc<WindowShared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(TargetIvars {
            window,
            renderer: Mutex::new(None),
            link: Mutex::new(None),
            held: Mutex::new(HashMap::new()),
        });
        // SAFETY: `init` de NSObject.
        unsafe { msg_send![super(this), init] }
    }

    fn on_tick(&self, link: &CADisplayLink) {
        let window = &self.ivars().window;
        if !window.dirty.swap(false, SeqCst) {
            // Sin cambios: se anuncia la pausa y se vuelve a mirar, para no
            // perder un aviso que llegue justo ahora (ver `Waker::request_frame`).
            window.running.store(false, SeqCst);
            if !window.dirty.swap(false, SeqCst) {
                link.setPaused(true);
                return;
            }
            window.running.store(true, SeqCst);
        }

        let Some(wf) = window.frame.lock().unwrap().clone() else {
            return;
        };
        let mut guard = self.ivars().renderer.lock().unwrap();
        let renderer = guard.get_or_insert_with(|| {
            let layer = window.layer.lock();
            let device = layer.device().expect("el layer no tiene device");
            Renderer::new(
                device,
                layer.pixelFormat(),
                &wf.font,
                wf.theme,
                wf.layout.scale,
            )
        });
        renderer.set_appearance(&wf.font, wf.layout.scale, wf.theme);

        let mut panes = Vec::with_capacity(wf.panes.len());
        let mut held = self.ivars().held.lock().unwrap();
        let mut holding = false;
        for slot in &wf.panes {
            let id = slot.session.id;
            let mut term = slot.session.term.lock().unwrap();
            let (cols, rows) = (term.grid().cols(), term.grid().rows());
            // DEC 2026: la aplicación está a mitad de una actualización. Se
            // repite el último frame del panel hasta que termine (o venza el límite).
            if term.synchronized()
                && let Some(&(cursor, modes)) = held.get(&id)
                && !renderer.ensure_pane(id, cols, rows)
            {
                holding = true;
                panes.push(PaneFrame {
                    id,
                    rect: slot.rect,
                    pad: slot.pad,
                    cursor,
                    modes,
                    overlay: overlay(&term, rows, cols, slot.link),
                    focused: slot.focused,
                });
                continue;
            }
            if renderer.ensure_pane(id, cols, rows) {
                for i in 0..rows {
                    renderer.update_row(id, i, term.visible_row(i));
                }
                term.drain_damage(|_, _| {});
            } else {
                term.drain_damage(|i, row| renderer.update_row(id, i, row));
            }
            for (pane, ch) in renderer.take_pending_clusters() {
                renderer.set_cluster(pane, ch, term.grid().char_text(ch).into_owned());
            }
            // Mirando el historial, el cursor queda desplazado (o fuera de la vista).
            let mut cursor = term.cursor();
            let mut modes = term.modes();
            cursor.row += term.display_offset();
            if cursor.row >= rows {
                modes.cursor_visible = false;
            }
            held.insert(id, (cursor, modes));
            panes.push(PaneFrame {
                id,
                rect: slot.rect,
                pad: slot.pad,
                cursor,
                modes,
                overlay: overlay(&term, rows, cols, slot.link),
                focused: slot.focused,
            });
        }
        held.retain(|id, _| wf.panes.iter().any(|p| p.session.id == *id));
        drop(held);
        if holding {
            // Seguir con ticks para pintar en cuanto termine (o venza) DEC 2026.
            window.dirty.store(true, SeqCst);
        }
        renderer.retain_panes(|id| wf.panes.iter().any(|p| p.session.id == id));
        let frame = Frame {
            layout: wf.layout,
            panes,
            dividers: wf.dividers.clone(),
            tab_bar: wf.tab_bar.clone(),
        };
        renderer.draw(&window.layer.lock(), &frame);
        window.frames.fetch_add(1, SeqCst);
    }
}

/// Lo que necesita cualquier hilo para pedir un frame o avisar al principal.
pub struct Waker {
    window: Arc<WindowShared>,
    target: Retained<RenderTarget>,
    thread: Retained<NSThread>,
}

impl Waker {
    /// Marca que hay cambios y, si el display link está pausado, lo reanuda.
    pub fn request_frame(&self) {
        self.window.dirty.store(true, SeqCst);
        if !self.window.running.swap(true, SeqCst) {
            // SAFETY: `wake` existe en RenderTarget y no recibe argumentos.
            unsafe {
                self.target
                    .performSelector_onThread_withObject_waitUntilDone(
                        sel!(wake),
                        &self.thread,
                        None,
                        false,
                    );
            }
        }
    }

    /// Ejecuta un método sin argumentos de `RenderTarget` en el hilo principal.
    pub fn on_main_thread(&self, selector: objc2::runtime::Sel) {
        // SAFETY: el selector existe en RenderTarget y no recibe argumentos.
        unsafe {
            self.target
                .performSelectorOnMainThread_withObject_waitUntilDone(selector, None, false);
        }
    }
}

/// Arranca el hilo de render y engancha a él un display link creado por la
/// vista (que debe estar ya en una ventana).
pub fn start_render(
    window: Arc<WindowShared>,
    create_link: impl FnOnce(&RenderTarget) -> Retained<CADisplayLink>,
) -> Arc<Waker> {
    let (tx, rx) = mpsc::channel();
    std::thread::Builder::new()
        .name("render".into())
        .spawn(move || {
            let run_loop = NSRunLoop::currentRunLoop();
            // Un puerto mantiene vivo el run loop mientras el display link está pausado.
            // SAFETY: puerto nuevo y modo por defecto.
            unsafe { run_loop.addPort_forMode(&NSPort::port(), NSDefaultRunLoopMode) };
            tx.send(NSThread::currentThread()).unwrap();
            loop {
                run_loop.run();
            }
        })
        .expect("hilo de render");
    let thread = rx.recv().expect("hilo de render arrancado");

    let target = RenderTarget::new(window.clone());
    let link = create_link(&target);
    // SAFETY: `attachLink:` recibe un CADisplayLink.
    unsafe {
        target.performSelector_onThread_withObject_waitUntilDone(
            sel!(attachLink:),
            &thread,
            Some(&link),
            false,
        );
    }
    if std::env::var_os("RT_FRAME_STATS").is_some() {
        let window = window.clone();
        std::thread::spawn(move || {
            loop {
                std::thread::sleep(Duration::from_secs(1));
                eprintln!("[frames/s] {}", window.frames.swap(0, SeqCst));
            }
        });
    }
    Arc::new(Waker {
        window,
        target,
        thread,
    })
}
