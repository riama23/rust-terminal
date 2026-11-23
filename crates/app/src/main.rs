//! Aplicación de macOS: una ventana con pestañas y paneles, render con Metal
//! y la shell del usuario en una PTY por panel.

mod config;
mod font;
mod input;
mod notify;
mod palette;
mod renderer;
mod session;
mod shell;
mod view;
mod watcher;
mod window;
mod workspace;

use std::cell::OnceCell;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSApplicationActivationPolicy, NSApplicationDelegate,
    NSApplicationTerminateReply, NSBackingStoreType, NSEventModifierFlags, NSMenu, NSMenuItem,
    NSScreen, NSWindow, NSWindowDelegate, NSWindowStyleMask,
};
use objc2_foundation::{
    MainThreadMarker, NSLocale, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect,
    NSSize, NSString,
};
use objc2_metal::MTLPixelFormat;
use objc2_quartz_core::{CAMetalLayer, kCAGravityTopLeft};

use crate::config::Config;
use crate::font::Metrics;
use crate::view::TerminalView;
use crate::window::WindowShared;

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

    unsafe impl NSWindowDelegate for AppDelegate {
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _notification: &NSNotification) {
            if let Some(view) = self.view() {
                view.window_focus_changed(true);
            }
        }

        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _notification: &NSNotification) {
            if let Some(view) = self.view() {
                view.window_focus_changed(false);
            }
        }
    }

    unsafe impl NSApplicationDelegate for AppDelegate {
        /// La ventana y la sesión se crean aquí: el display link de la vista
        /// necesita que la ventana ya esté en una pantalla.
        #[unsafe(method(applicationDidFinishLaunching:))]
        fn did_finish_launching(&self, _notification: &NSNotification) {
            let window = open_window(self.mtm());
            window.setDelegate(Some(ProtocolObject::from_ref(self)));
            let _ = self.ivars().window.set(window);
        }

        #[unsafe(method(applicationShouldTerminateAfterLastWindowClosed:))]
        fn should_terminate_after_last_window_closed(&self, _app: &NSApplication) -> bool {
            true
        }

        /// Cmd+Q: confirma si hay programas en marcha.
        #[unsafe(method(applicationShouldTerminate:))]
        fn should_terminate(&self, _app: &NSApplication) -> NSApplicationTerminateReply {
            let ok = self
                .ivars()
                .window
                .get()
                .and_then(|w| w.contentView())
                .and_then(|v| v.downcast::<TerminalView>().ok())
                .is_none_or(|v| v.confirm_quit());
            if ok {
                NSApplicationTerminateReply::TerminateNow
            } else {
                NSApplicationTerminateReply::TerminateCancel
            }
        }
    }
);

impl AppDelegate {
    fn view(&self) -> Option<Retained<TerminalView>> {
        self.ivars()
            .window
            .get()
            .and_then(|w| w.contentView())
            .and_then(|v| v.downcast::<TerminalView>().ok())
    }
}

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

fn submenu(bar: &NSMenu, mtm: MainThreadMarker, title: &str, items: Vec<Retained<NSMenuItem>>) {
    let item = NSMenuItem::new(mtm);
    let menu = NSMenu::initWithTitle(NSMenu::alloc(mtm), &NSString::from_str(title));
    for i in items {
        menu.addItem(&i);
    }
    item.setSubmenu(Some(&menu));
    bar.addItem(&item);
}

fn build_menu(app: &NSApplication, mtm: MainThreadMarker) {
    let bar = NSMenu::new(mtm);

    let app_item = NSMenuItem::new(mtm);
    let app_menu = NSMenu::new(mtm);
    app_menu.addItem(&menu_item(mtm, "Salir", sel!(terminate:), "q"));
    app_item.setSubmenu(Some(&app_menu));
    bar.addItem(&app_item);

    // Una mayúscula en el atajo implica Shift (p. ej. "D" = Cmd+Shift+D).
    submenu(
        &bar,
        mtm,
        "Shell",
        vec![
            menu_item(mtm, "Nueva pestaña", sel!(newTab:), "t"),
            menu_item(mtm, "Dividir en vertical", sel!(splitVertical:), "d"),
            menu_item(mtm, "Dividir en horizontal", sel!(splitHorizontal:), "D"),
            menu_item(mtm, "Cerrar", sel!(closePane:), "w"),
            menu_item(mtm, "Cerrar pestaña", sel!(closeTab:), "W"),
        ],
    );

    submenu(
        &bar,
        mtm,
        "Edición",
        vec![
            menu_item(mtm, "Copiar", sel!(copy:), "c"),
            menu_item(
                mtm,
                "Copiar salida del último comando",
                sel!(copyLastOutput:),
                "C",
            ),
            menu_item(mtm, "Pegar", sel!(paste:), "v"),
        ],
    );

    // U+F700…U+F703: flechas (NSUpArrowFunctionKey…).
    submenu(
        &bar,
        mtm,
        "Ver",
        vec![
            menu_item(mtm, "Bloque anterior", sel!(previousBlock:), "\u{F700}"),
            menu_item(mtm, "Bloque siguiente", sel!(nextBlock:), "\u{F701}"),
        ],
    );

    let mut window_items = vec![
        menu_item(mtm, "Pestaña anterior", sel!(previousTab:), "{"),
        menu_item(mtm, "Pestaña siguiente", sel!(nextTab:), "}"),
    ];
    for (title, action, key) in [
        ("Panel de la izquierda", sel!(focusLeft:), "\u{F702}"),
        ("Panel de la derecha", sel!(focusRight:), "\u{F703}"),
        ("Panel de arriba", sel!(focusUp:), "\u{F700}"),
        ("Panel de abajo", sel!(focusDown:), "\u{F701}"),
    ] {
        let item = menu_item(mtm, title, action, key);
        item.setKeyEquivalentModifierMask(
            NSEventModifierFlags::Command | NSEventModifierFlags::Option,
        );
        window_items.push(item);
    }
    for n in 1..=9 {
        let item = menu_item(
            mtm,
            &format!("Pestaña {n}"),
            sel!(selectTab:),
            &n.to_string(),
        );
        item.setTag(n);
        window_items.push(item);
    }
    submenu(&bar, mtm, "Ventana", window_items);

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
fn utf8_locale_env() -> Vec<(String, String)> {
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
    vec![("LANG".to_owned(), lang)]
}

/// Crea la ventana con su vista, arranca el render y la primera sesión.
fn open_window(mtm: MainThreadMarker) -> Retained<NSWindow> {
    // Tamaño inicial a partir de las métricas de la pantalla principal
    // (contando la barra de pestañas).
    let scale = NSScreen::mainScreen(mtm).map_or(2.0, |s| s.backingScaleFactor());
    let (config, errors) = match Config::load() {
        Ok(loaded) => loaded,
        Err(e) => (Config::default(), vec![e]),
    };
    let spec = font::FontSpec {
        family: config.font_family.clone(),
        size_pt: config.font_size,
    };
    let metrics = Metrics::of(&spec.font(scale));
    let bar_pt = (metrics.cell_h as f64 * 1.8).round() / scale;
    let content = NSSize::new(
        INITIAL_COLS as f64 * metrics.cell_w as f64 / scale + 2.0 * config.padding,
        INITIAL_ROWS as f64 * metrics.cell_h as f64 / scale + 2.0 * config.padding + bar_pt,
    );

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
    let shared = WindowShared::new(layer.clone());

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
    window.setTitle(&NSString::from_str(rust_terminal_core::PRODUCT_NAME));

    let view = TerminalView::new(
        mtm,
        NSRect::new(NSPoint::new(0.0, 0.0), content),
        shared.clone(),
        config,
        utf8_locale_env(),
    );
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    window.setContentView(Some(&view));
    window.center();
    window.makeKeyAndOrderFront(None);
    window.makeFirstResponder(Some(&view));
    // Movimiento del ratón sin botón: modo `?1003` y Cmd sobre enlaces.
    window.setAcceptsMouseMovedEvents(true);

    let waker = window::start_render(shared, |target| {
        // SAFETY: `tick:` existe en RenderTarget y recibe el display link.
        unsafe { view.displayLinkWithTarget_selector(target, sel!(tick:)) }
    });
    view.start(waker.clone());
    if !errors.is_empty() {
        view.show_config_errors(&errors);
    }
    // Recarga en caliente: el vigilante avisa al hilo principal.
    watcher::watch(Config::path(), move || {
        waker.on_main_thread(sel!(configChanged))
    });
    window
}
