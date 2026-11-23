//! Hilos de una sesión: lector de la PTY y render con `CADisplayLink`.
//!
//! - El **lector** parsea lotes de la PTY con el bloqueo de `Term` tomado y
//!   lo suelta en cada lote; después pide un frame.
//! - El **render** tiene su propio `NSRunLoop`, al que se añade el
//!   `CADisplayLink`. En cada tick copia las filas sucias (bloqueo breve) y
//!   pinta. Si no hay cambios, pausa el display link: en reposo no hay frames.

use std::io::{Read, Write};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering::SeqCst};
use std::sync::{Arc, Mutex, mpsc};
use std::time::Duration;

use objc2::rc::Retained;
use objc2::{AnyThread, DefinedClass, Message, define_class, msg_send, sel};
use objc2_app_kit::NSApplication;
use objc2_foundation::{
    MainThreadMarker, NSDefaultRunLoopMode, NSObject, NSObjectNSThreadPerformAdditions, NSPort,
    NSRunLoop, NSString, NSThread,
};
use objc2_quartz_core::{CADisplayLink, CAMetalLayer};
use rust_terminal_core::{PtyReader, PtyWriter, Term};

use crate::renderer::{Layout, Overlay, Renderer};

/// Selección y marcas de bloque en coordenadas de la vista.
fn overlay(term: &Term, rows: usize, cols: usize) -> Overlay {
    let mut overlay = Overlay::default();
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

/// Estado de una sesión compartido entre hilos.
pub struct Shared {
    pub term: Mutex<Term>,
    pub writer: Mutex<PtyWriter>,
    pub layout: Mutex<Layout>,
    pub layer: SharedLayer,
    /// Hay algo nuevo que pintar.
    dirty: AtomicBool,
    /// El display link está activo (o va a reactivarse).
    running: AtomicBool,
    frames: AtomicU64,
}

impl Shared {
    pub fn new(
        term: Term,
        writer: PtyWriter,
        layout: Layout,
        layer: Retained<CAMetalLayer>,
    ) -> Self {
        Shared {
            term: Mutex::new(term),
            writer: Mutex::new(writer),
            layout: Mutex::new(layout),
            layer: SharedLayer(Mutex::new(layer)),
            dirty: AtomicBool::new(true),
            running: AtomicBool::new(true),
            frames: AtomicU64::new(0),
        }
    }

    /// Escribe en la PTY (teclas, pegado o respuestas a consultas).
    pub fn write(&self, bytes: &[u8]) {
        if let Ok(mut w) = self.writer.lock() {
            let _ = w.write_all(bytes);
        }
    }
}

struct SendLink(Retained<CADisplayLink>);
// SAFETY: el display link solo se toca desde el hilo de render tras `attachLink:`.
unsafe impl Send for SendLink {}

pub struct TargetIvars {
    shared: Arc<Shared>,
    renderer: Mutex<Option<Renderer>>,
    link: Mutex<Option<SendLink>>,
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

        /// La shell terminó (hilo principal).
        #[unsafe(method(sessionEnded))]
        fn session_ended(&self) {
            let mtm = MainThreadMarker::new().expect("hilo principal");
            NSApplication::sharedApplication(mtm).terminate(None);
        }

        /// Cambió el título por OSC 0/2 (hilo principal).
        #[unsafe(method(titleChanged))]
        fn title_changed(&self) {
            let mtm = MainThreadMarker::new().expect("hilo principal");
            let title = self.ivars().shared.term.lock().unwrap().title().map(str::to_owned);
            if let (Some(title), Some(window)) =
                (title, NSApplication::sharedApplication(mtm).windows().firstObject())
            {
                window.setTitle(&NSString::from_str(&title));
            }
        }
    }
);

// SAFETY: los ivars son `Send + Sync` (Arc y Mutex con contenido `Send`).
unsafe impl Send for RenderTarget {}
unsafe impl Sync for RenderTarget {}

impl RenderTarget {
    pub fn new(shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc().set_ivars(TargetIvars {
            shared,
            renderer: Mutex::new(None),
            link: Mutex::new(None),
        });
        // SAFETY: `init` de NSObject.
        unsafe { msg_send![super(this), init] }
    }

    fn on_tick(&self, link: &CADisplayLink) {
        let shared = &self.ivars().shared;
        if !shared.dirty.swap(false, SeqCst) {
            // Sin cambios: se anuncia la pausa y se vuelve a mirar, para no
            // perder un aviso que llegue justo ahora (ver `Waker::request_frame`).
            shared.running.store(false, SeqCst);
            if !shared.dirty.swap(false, SeqCst) {
                link.setPaused(true);
                return;
            }
            shared.running.store(true, SeqCst);
        }

        let layout = *shared.layout.lock().unwrap();
        let mut guard = self.ivars().renderer.lock().unwrap();
        let renderer = guard.get_or_insert_with(|| {
            let layer = shared.layer.lock();
            let device = layer.device().expect("el layer no tiene device");
            Renderer::new(device, layer.pixelFormat(), layout.scale)
        });
        renderer.set_scale(layout.scale);

        let (cursor, modes, overlay) = {
            let mut term = shared.term.lock().unwrap();
            let grid = term.grid();
            let (cols, rows) = (grid.cols(), grid.rows());
            if renderer.ensure_size(cols, rows) {
                for i in 0..rows {
                    renderer.update_row(i, term.visible_row(i));
                }
                term.drain_damage(|_, _| {});
            } else {
                term.drain_damage(|i, row| renderer.update_row(i, row));
            }
            // Mirando el historial, el cursor queda desplazado (o fuera de la vista).
            let mut cursor = term.cursor();
            let mut modes = term.modes();
            cursor.row += term.display_offset();
            if cursor.row >= rows {
                modes.cursor_visible = false;
            }
            (cursor, modes, overlay(&term, rows, cols))
        };
        renderer.draw(&shared.layer.lock(), layout, cursor, modes, &overlay);
        shared.frames.fetch_add(1, SeqCst);
    }
}

/// Lo que necesita cualquier hilo para pedir un frame.
pub struct Waker {
    shared: Arc<Shared>,
    target: Retained<RenderTarget>,
    thread: Retained<NSThread>,
}

impl Waker {
    /// Marca que hay cambios y, si el display link está pausado, lo reanuda.
    pub fn request_frame(&self) {
        self.shared.dirty.store(true, SeqCst);
        if !self.shared.running.swap(true, SeqCst) {
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

    fn on_main_thread(&self, selector: objc2::runtime::Sel) {
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
    shared: Arc<Shared>,
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

    let target = RenderTarget::new(shared.clone());
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
        spawn_frame_stats(shared.clone());
    }
    Arc::new(Waker {
        shared,
        target,
        thread,
    })
}

/// Imprime en stderr los frames pintados en cada segundo.
fn spawn_frame_stats(shared: Arc<Shared>) {
    std::thread::spawn(move || {
        loop {
            std::thread::sleep(Duration::from_secs(1));
            eprintln!("[frames/s] {}", shared.frames.swap(0, SeqCst));
        }
    });
}

/// Arranca el hilo lector de la PTY.
pub fn start_reader(mut reader: PtyReader, waker: Arc<Waker>) {
    std::thread::Builder::new()
        .name("pty-reader".into())
        .spawn(move || {
            let mut buf = vec![0u8; 64 * 1024];
            loop {
                let n = match reader.read(&mut buf) {
                    Ok(0) | Err(_) => break,
                    Ok(n) => n,
                };
                let (responses, title_changed) = {
                    let mut term = waker.shared.term.lock().unwrap();
                    term.advance(&buf[..n]);
                    (term.take_responses(), term.take_title_changed())
                };
                if !responses.is_empty() {
                    waker.shared.write(&responses);
                }
                if title_changed {
                    waker.on_main_thread(sel!(titleChanged));
                }
                waker.request_frame();
            }
            waker.on_main_thread(sel!(sessionEnded));
        })
        .expect("hilo lector");
}
