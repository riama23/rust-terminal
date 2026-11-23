//! Aplicación de macOS: ventana con AppKit, render con Metal y la shell del
//! usuario en una PTY.

mod font;
mod input;
mod palette;
mod renderer;
mod session;
mod shell;
mod view;

use std::cell::OnceCell;
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate, NSBackingStoreType,
    NSMenu, NSMenuItem, NSScreen, NSWindow, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSLocale, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString, ns_string,
};
use objc2_metal::MTLPixelFormat;
use objc2_quartz_core::{CAMetalLayer, kCAGravityTopLeft};
use rust_terminal_core::{Pty, Term};

use crate::font::Metrics;
use crate::renderer::Layout;
use crate::session::Shared;
use crate::view::{PAD_PT, TerminalView};

const INITIAL_COLS: usize = 100;
const INITIAL_ROWS: usize = 30;

#[derive(Default)]
struct DelegateIvars {
    window: OnceCell<Retained<NSWindow>>,
}

define_class!(
    // SAFETY: NSObject no impone requisitos y `AppDelegate` no implementa `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTAppDelegate"]
    #[ivars = DelegateIvars]
    struct AppDelegate;

    unsafe impl NSObjectProtocol for AppDelegate {}

    unsafe impl NSApplicationDelegate for AppDelegate {
        /// La ventana y la sesión se crean aquí: el display link de la vista
        /// necesita que la ventana ya esté en una pantalla.
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let window = open_window(self.mtm());
            let _ = self.ivars().window.set(window);
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            true
        }
    }
);

impl AppDelegate {
    fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DelegateIvars::default());
        // SAFETY: `init` de NSObject.
        unsafe { msg_send![super(this), init] }
    }
}

fn menu_item(
    mtm: MainThreadMarker,
    title: &str,
    action: objc2::runtime::Sel,
    key: &str,
) -> Retained<NSMenuItem> {
    // SAFETY: título, acción y atajo válidos.
    unsafe {
        NSMenuItem::initWithTitle_action_keyEquivalent(
            NSMenuItem::alloc(mtm),
            &NSString::from_str(title),
            Some(action),
            &NSString::from_str(key),
        )
    }
}

fn build_menu(app: &NSApplication, mtm: MainThreadMarker) {
    let bar = NSMenu::new(mtm);

    let app_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    app_menu.addItem(&menu_item(mtm, "Salir", sel!(terminate:), "q"));
    app_item.setSubmenu(Some(&app_menu));
    bar.addItem(&app_item);

    let edit_item = NSMenuItem::new(mtm);
    let edit_menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), ns_string!("Edición"));
    edit_menu.addItem(&menu_item(mtm, "Pegar", sel!(paste:), "v"));
    edit_item.setSubmenu(Some(&edit_menu));
    bar.addItem(&edit_item);

    app.setMainMenu(Some(&bar));
}

fn main() {
    let mtm = MainThreadMarker::new().expect("debe ejecutarse en el hilo principal");

    // Variables que heredamos si se lanza desde otra terminal y que
    // confundirían a la shell sobre dónde se ejecuta.
    // SAFETY: todavía no hay más hilos.
    unsafe {
        for var in ["TERM_PROGRAM", "TERM_PROGRAM_VERSION", "TERM_SESSION_ID"] {
            std::env::remove_var(var);
        }
    }

    // Abierta desde el Finder o el Dock, la app arranca en `/`: la shell
    // debe empezar en el directorio personal.
    if std::env::current_dir().is_ok_and(|d| d == std::path::Path::new("/"))
        && let Some(home) = std::env::var_os("HOME")
    {
        let _ = std::env::set_current_dir(home);
    }

    let app = NSApplication::sharedApplication(mtm);
    app.setActivationPolicy(NSApplicationActivationPolicy::Regular);
    build_menu(&app, mtm);
    let delegate = AppDelegate::new(mtm);
    app.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));

    #[allow(deprecated)]
    app.activateIgnoringOtherApps(true);
    app.run();
}

/// Si no hay `LANG` (lo normal al abrir la app desde el Finder o el Dock),
/// se le da a la shell un locale UTF-8 derivado del idioma del sistema. Sin
/// él, zsh, vim o tmux trabajan en locale C y no muestran bien `ñ` ni `é`.
fn utf8_locale_env() -> Vec<(&'static str, String)> {
    if std::env::var_os("LANG").is_some() {
        return Vec::new();
    }
    let id = NSLocale::currentLocale().localeIdentifier().to_string();
    // "es_ES@rg=eszzzz" → "es_ES"
    let id = id.split('@').next().unwrap_or("").to_owned();
    let candidate = format!("{id}.UTF-8");
    let lang = if std::path::Path::new("/usr/share/locale")
        .join(&candidate)
        .exists()
    {
        candidate
    } else {
        "en_US.UTF-8".to_owned()
    };
    vec![("LANG", lang)]
}

/// Crea la ventana con su vista y arranca la sesión (shell, lector y render).
fn open_window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    // Tamaño inicial de la ventana a partir de las métricas de la pantalla principal.
    let scale = NSScreen::mainScreen(mtm).map_or(2.0, |s| s.backingScaleFactor());
    let metrics = Metrics::of(&font::system_monospace(font::FONT_SIZE_PT * scale));
    let content = NSSize::new(
        INITIAL_COLS as f64 * metrics.cell_w as f64 / scale + 2.0 * PAD_PT,
        INITIAL_ROWS as f64 * metrics.cell_h as f64 / scale + 2.0 * PAD_PT,
    );

    let shell = std::env::var("SHELL").unwrap_or_else(|_| "/bin/zsh".to_owned());
    let mut env: Vec<(String, String)> = utf8_locale_env()
        .into_iter()
        .map(|(k, v)| (k.to_owned(), v))
        .collect();
    env.extend(shell::integration_env(&shell));
    let pty = Pty::spawn(
        &shell,
        &["-l"],
        env,
        INITIAL_COLS as u16,
        INITIAL_ROWS as u16,
    )
    .expect("no se pudo lanzar la shell");
    let (reader, writer, _child) = pty.split().expect("no se pudo separar la PTY");

    let device = renderer::create_device();
    let layer = CAMetalLayer::new();
    layer.setDevice(Some(&device));
    layer.setPixelFormat(MTLPixelFormat::BGRA8Unorm);
    layer.setFramebufferOnly(true);
    layer.setOpaque(true);
    // Durante el redimensionado en vivo, el último frame queda anclado arriba
    // a la izquierda en lugar de estirarse.
    // SAFETY: constante de Core Animation.
    layer.setContentsGravity(unsafe { kCAGravityTopLeft });

    let shared = Arc::new(Shared::new(
        Term::new(INITIAL_COLS, INITIAL_ROWS),
        writer,
        Layout {
            scale,
            width_px: 1,
            height_px: 1,
            pad_px: 0,
        },
        layer.clone(),
    ));

    // SAFETY: la ventana no se libera al cerrarse (`setReleasedWhenClosed(false)`).
    let window = unsafe {
        NSWindow::initWithContentRect_styleMask_backing_defer(
            NSWindow::alloc(mtm),
            NSRect::new(NSPoint::new(0.0, 0.0), content),
            NSWindowStyleMask::Titled
                | NSWindowStyleMask::Closable
                | NSWindowStyleMask::Miniaturizable
                | NSWindowStyleMask::Resizable,
            NSBackingStoreType::Buffered,
            false,
        )
    };
    // SAFETY: ventana creada fuera de un window controller.
    unsafe { window.setReleasedWhenClosed(false) };
    let shell_name = shell.rsplit('/').next().unwrap_or(&shell).to_owned();
    window.setTitle(&NSString::from_str(&shell_name));

    let view = TerminalView::new(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), content),
        shared.clone(),
    );
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    window.setContentView(Some(&view));
    window.center();
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&view));

    let waker = session::start_render(shared, |target| {
        // SAFETY: `tick:` existe en RenderTarget y recibe el display link.
        unsafe { view.displayLinkWithTarget_selector(target, sel!(tick:)) }
    });
    view.set_waker(waker.clone());
    view.update_layout();
    session::start_reader(reader, waker);

    window
}
