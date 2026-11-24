//! Terminal rápido (v1.6, pantalla 04 del spec): un panel que baja desde el
//! borde superior con ⌥Espacio desde cualquier aplicación.
//!
//! Es una segunda ventana con su propia vista, su render y sus sesiones, con
//! fondo translúcido sobre un desenfoque. Se crea la primera vez que se pide
//! y se conserva oculto (las shells siguen vivas). **Se oculta al perder el
//! foco salvo que esté fijado** (⌘⌥P). «Mover a ventana» (⌘⌥W) lo convierte en
//! una ventana normal y el siguiente ⌥Espacio crea un terminal rápido nuevo.

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, ProtocolObject};
use objc2::{MainThreadOnly, define_class, msg_send, sel};
use objc2_app_kit::{
    NSApplication, NSAutoresizingMaskOptions, NSBackingStoreType, NSColor, NSEvent, NSPanel,
    NSRunningApplication, NSScreen, NSVisualEffectBlendingMode, NSVisualEffectMaterial,
    NSVisualEffectState, NSVisualEffectView, NSWindow, NSWindowCollectionBehavior,
    NSWindowDelegate, NSWindowStyleMask, NSWorkspace,
};
use objc2_foundation::{
    MainThreadMarker, NSNotification, NSObject, NSObjectProtocol, NSPoint, NSRect, NSSize, NSString,
};
use objc2_quartz_core::{CAMetalLayer, kCAGravityTopLeft};

use crate::config::Config;
use crate::container::ContainerView;
use crate::view::TerminalView;
use crate::window::{self, WindowShared};

/// Margen lateral y tiempo de la animación (spec: 16 px y 180–220 ms).
const SIDE_INSET: f64 = 16.0;
const HIDE_DELAY: f64 = 0.22;

/// Cuánto ocupa el panel del quick terminal en una pantalla.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Frames {
    /// Dónde está visible: pegado al borde superior.
    pub shown: [f64; 4],
    /// Dónde está oculto: el mismo rectángulo justo por encima de la pantalla.
    pub hidden: [f64; 4],
}

/// Los rectángulos `[x, y, ancho, alto]` (origen abajo a la izquierda, como
/// AppKit) del panel para el área visible `screen` y el alto pedido.
pub fn frames(screen: [f64; 4], height: f64) -> Frames {
    let [sx, sy, sw, sh] = screen;
    let w = (sw - 2.0 * SIDE_INSET).max(320.0).min(sw);
    let h = height.clamp(160.0, (sh * 0.9).max(160.0));
    let x = sx + (sw - w) / 2.0;
    Frames {
        shown: [x, sy + sh - h, w, h],
        hidden: [x, sy + sh, w, h],
    }
}

struct Quick {
    window: Retained<NSWindow>,
    view: Retained<TerminalView>,
    _delegate: Retained<QuickDelegate>,
    pinned: Cell<bool>,
    shown: Cell<bool>,
    /// La aplicación que tenía el foco antes de mostrarlo (se le devuelve al ocultarlo).
    previous: RefCell<Option<Retained<NSRunningApplication>>>,
}

thread_local! {
    static QUICK: RefCell<Option<Quick>> = const { RefCell::new(None) };
}

define_class!(
    // SAFETY: NSPanel no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSPanel))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTQuickPanel"]
    struct QuickPanel;

    impl QuickPanel {
        /// Una ventana sin borde no puede ser la clave por defecto.
        #[unsafe(method(canBecomeKeyWindow))]
        fn can_become_key(&self) -> bool {
            true
        }
    }
);

define_class!(
    // SAFETY: NSObject no impone requisitos y `QuickDelegate` no implementa `Drop`.
    #[unsafe(super(NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTQuickDelegate"]
    struct QuickDelegate;

    unsafe impl NSObjectProtocol for QuickDelegate {}

    unsafe impl NSWindowDelegate for QuickDelegate {
        #[unsafe(method(windowDidBecomeKey:))]
        fn window_did_become_key(&self, _n: &NSNotification) {
            if let Some(view) = current_view() {
                view.window_focus_changed(true);
            }
        }

        /// Perder el foco lo oculta, salvo que esté fijado.
        #[unsafe(method(windowDidResignKey:))]
        fn window_did_resign_key(&self, _n: &NSNotification) {
            if let Some(view) = current_view() {
                view.window_focus_changed(false);
            }
            if let Some(mtm) = MainThreadMarker::new() {
                on_resign(mtm);
            }
        }
    }

    impl QuickDelegate {
        /// Termina la animación de ocultar: quita la ventana de la pantalla.
        #[unsafe(method(finishHide))]
        fn finish_hide(&self) {
            QUICK.with(|q| {
                if let Some(q) = q.borrow().as_ref()
                    && !q.shown.get()
                {
                    q.window.orderOut(None);
                }
            });
        }
    }
);

fn current_view() -> Option<Retained<TerminalView>> {
    QUICK.with(|q| q.borrow().as_ref().map(|q| q.view.clone()))
}

fn reduce_motion() -> bool {
    NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceMotion()
}

/// El área visible de la pantalla donde está el ratón (si no, la principal).
fn target_screen(mtm: MainThreadMarker) -> [f64; 4] {
    let mouse = NSEvent::mouseLocation();
    let screens = NSScreen::screens(mtm);
    let screen = screens
        .iter()
        .find(|s| {
            let f = s.frame();
            mouse.x >= f.origin.x
                && mouse.x <= f.origin.x + f.size.width
                && mouse.y >= f.origin.y
                && mouse.y <= f.origin.y + f.size.height
        })
        .or_else(|| NSScreen::mainScreen(mtm));
    screen.map_or([0.0, 0.0, 1440.0, 900.0], |s| {
        let f = s.visibleFrame();
        [f.origin.x, f.origin.y, f.size.width, f.size.height]
    })
}

fn rect(r: [f64; 4]) -> NSRect {
    NSRect::new(NSPoint::new(r[0], r[1]), NSSize::new(r[2], r[3]))
}

/// Crea el panel, su vista y su render.
fn create(mtm: MainThreadMarker) -> Option<Quick> {
    let config = match Config::load() {
        Ok((c, _)) => c,
        Err(_) => Config::default(),
    };
    let f = frames(target_screen(mtm), config.quick_height);

    let device = crate::renderer::create_device();
    let layer = CAMetalLayer::new();
    layer.setDevice(Some(&device));
    layer.setPixelFormat(objc2_metal::MTLPixelFormat::BGRA8Unorm);
    layer.setFramebufferOnly(true);
    // Fondo translúcido: el layer no es opaco y el render limpia con alfa.
    layer.setOpaque(false);
    // SAFETY: constante de Core Animation.
    layer.setContentsGravity(unsafe { kCAGravityTopLeft });
    let shared = WindowShared::new(layer.clone());

    // SAFETY: la ventana no se libera al cerrarse (`setReleasedWhenClosed(false)`).
    let panel: Retained<QuickPanel> = unsafe {
        let this = QuickPanel::alloc(mtm).set_ivars(());
        msg_send![
            super(this),
            initWithContentRect: rect(f.hidden),
            styleMask: NSWindowStyleMask::Borderless | NSWindowStyleMask::NonactivatingPanel,
            backing: NSBackingStoreType::Buffered,
            defer: false
        ]
    };
    let window: &NSWindow = &panel;
    // SAFETY: ventana creada fuera de un window controller.
    unsafe { window.setReleasedWhenClosed(false) };
    window.setOpaque(false);
    window.setBackgroundColor(Some(&NSColor::clearColor()));
    window.setHasShadow(true);
    // Por encima del resto y en todos los espacios (también sobre apps a pantalla completa).
    window.setLevel(3);
    window.setCollectionBehavior(
        NSWindowCollectionBehavior::CanJoinAllSpaces
            | NSWindowCollectionBehavior::FullScreenAuxiliary
            | NSWindowCollectionBehavior::IgnoresCycle,
    );
    panel.setFloatingPanel(true);
    panel.setHidesOnDeactivate(false);
    window.setAcceptsMouseMovedEvents(true);

    // Desenfoque detrás del fondo translúcido.
    let content = rect([0.0, 0.0, f.hidden[2], f.hidden[3]]);
    let effect = NSVisualEffectView::initWithFrame(NSVisualEffectView::alloc(mtm), content);
    effect.setMaterial(NSVisualEffectMaterial::Sidebar);
    effect.setBlendingMode(NSVisualEffectBlendingMode::BehindWindow);
    effect.setState(NSVisualEffectState::Active);
    effect.setWantsLayer(true);
    if let Some(l) = effect.layer() {
        l.setCornerRadius(12.0);
        l.setMasksToBounds(true);
    }
    window.setContentView(Some(&effect));

    let view = TerminalView::new(
        mtm,
        content,
        shared.clone(),
        config.clone(),
        crate::utf8_locale_env(),
        None,
    );
    // «Reducir transparencia» del sistema: fondo opaco (spec, Accesibilidad).
    let opacity = if NSWorkspace::sharedWorkspace().accessibilityDisplayShouldReduceTransparency() {
        1.0
    } else {
        config.quick_opacity as f32
    };
    view.set_quick(opacity);
    view.setLayer(Some(&layer));
    view.setWantsLayer(true);
    let container = ContainerView::new(mtm, content);
    container.set_terminal(&view);
    container.setAutoresizingMask(
        NSAutoresizingMaskOptions::ViewWidthSizable | NSAutoresizingMaskOptions::ViewHeightSizable,
    );
    effect.addSubview(&container);

    let delegate: Retained<QuickDelegate> = {
        let this = QuickDelegate::alloc(mtm).set_ivars(());
        // SAFETY: `init` de NSObject.
        unsafe { msg_send![super(this), init] }
    };
    window.setDelegate(Some(ProtocolObject::from_ref(&*delegate)));
    window.orderFrontRegardless();
    window.makeFirstResponder(Some(&view));

    let waker = window::start_render(shared, |target| {
        // SAFETY: `tick:` existe en RenderTarget y recibe el display link.
        unsafe { view.displayLinkWithTarget_selector(target, sel!(tick:)) }
    });
    view.start(waker);
    view.apply_theme();
    Some(Quick {
        window: window.retain_window(),
        view,
        _delegate: delegate,
        pinned: Cell::new(false),
        shown: Cell::new(false),
        previous: RefCell::new(None),
    })
}

/// `&NSWindow` → `Retained<NSWindow>` (el panel es una `NSWindow`).
trait RetainWindow {
    fn retain_window(&self) -> Retained<NSWindow>;
}

impl RetainWindow for NSWindow {
    fn retain_window(&self) -> Retained<NSWindow> {
        objc2::rc::Retained::from(self)
    }
}

/// ⌥Espacio: lo muestra si está oculto (o sin foco) y lo oculta si está a la vista.
pub fn toggle(mtm: MainThreadMarker) {
    let visible_and_key = QUICK.with(|q| {
        q.borrow()
            .as_ref()
            .is_some_and(|q| q.shown.get() && q.window.isKeyWindow())
    });
    if visible_and_key {
        hide(true);
    } else {
        show(mtm);
    }
}

fn show(mtm: MainThreadMarker) {
    if QUICK.with(|q| q.borrow().is_none()) {
        let Some(quick) = create(mtm) else { return };
        QUICK.with(|q| *q.borrow_mut() = Some(quick));
    }
    QUICK.with(|q| {
        let slot = q.borrow();
        let Some(quick) = slot.as_ref() else { return };
        // Quién tenía el foco, para devolvérselo al ocultarlo.
        let front = NSWorkspace::sharedWorkspace().frontmostApplication();
        let ours = std::process::id() as i32;
        *quick.previous.borrow_mut() = front.filter(|a| a.processIdentifier() != ours);

        let config = quick.view.quick_config();
        let f = frames(target_screen(mtm), config.0);
        let window = &quick.window;
        if !quick.shown.get() {
            window.setFrame_display(rect(f.hidden), false);
        }
        quick.shown.set(true);
        window.orderFrontRegardless();
        window.makeKeyAndOrderFront(None);
        window.makeFirstResponder(Some(&*quick.view));
        window.setFrame_display_animate(rect(f.shown), true, !reduce_motion());
        quick
            .view
            .set_tab_note(quick.pinned.get().then(|| "fijado".to_owned()));
    });
}

/// Lo oculta (con la animación de subida). Con `restore`, devuelve el foco a
/// la aplicación anterior (al pulsar ⌥Espacio, no al perder el foco).
fn hide(restore: bool) {
    let Some(mtm) = MainThreadMarker::new() else {
        return;
    };
    let previous = QUICK.with(|q| {
        let slot = q.borrow();
        let quick = slot.as_ref()?;
        if !quick.shown.get() {
            return None;
        }
        quick.shown.set(false);
        let f = frames(target_screen(mtm), quick.view.quick_config().0);
        if reduce_motion() {
            quick.window.orderOut(None);
        } else {
            quick
                .window
                .setFrame_display_animate(rect(f.hidden), true, true);
            // SAFETY: `finishHide` existe en el delegado y no lleva argumentos.
            let _: () = unsafe {
                msg_send![&*quick._delegate, performSelector: sel!(finishHide), withObject: None::<&AnyObject>, afterDelay: HIDE_DELAY]
            };
        }
        quick.previous.borrow_mut().take()
    });
    if restore && let Some(app) = previous {
        // SAFETY: devolver el foco a una aplicación en marcha.
        #[allow(deprecated)]
        let _ = app.activateWithOptions(
            objc2_app_kit::NSApplicationActivationOptions::ActivateIgnoringOtherApps,
        );
    }
}

/// El panel perdió el foco: se oculta salvo que esté fijado o haya un diálogo de la app.
fn on_resign(mtm: MainThreadMarker) {
    let hide_it = QUICK.with(|q| {
        q.borrow()
            .as_ref()
            .is_some_and(|q| q.shown.get() && !q.pinned.get())
    });
    if hide_it
        && NSApplication::sharedApplication(mtm)
            .modalWindow()
            .is_none()
    {
        hide(false);
    }
}

/// ⌘⌥P: fija el terminal rápido (no se oculta al perder el foco) o lo suelta.
pub fn toggle_pin() {
    QUICK.with(|q| {
        if let Some(quick) = q.borrow().as_ref() {
            let pinned = !quick.pinned.get();
            quick.pinned.set(pinned);
            quick.view.set_tab_note(pinned.then(|| "fijado".to_owned()));
        }
    });
}

/// ⌘⌥W: lo convierte en una ventana normal. El siguiente ⌥Espacio crea otro.
pub fn promote(mtm: MainThreadMarker) {
    let Some(quick) = QUICK.with(|q| q.borrow_mut().take()) else {
        return;
    };
    let window = &quick.window;
    quick.view.promote_from_quick();
    // Sin delegado: ya no se oculta al perder el foco.
    window.setDelegate(None);
    window.setLevel(0);
    window.setCollectionBehavior(NSWindowCollectionBehavior::Default);
    window.setStyleMask(
        NSWindowStyleMask::Titled
            | NSWindowStyleMask::Closable
            | NSWindowStyleMask::Miniaturizable
            | NSWindowStyleMask::Resizable,
    );
    window.setOpaque(true);
    window.setBackgroundColor(None);
    window.setTitle(&NSString::from_str(rust_terminal_core::PRODUCT_NAME));
    // La terminal pasa a ser el contenido de la ventana (sin el desenfoque).
    // SAFETY: `superview` solo lee el árbol de vistas, desde el hilo principal.
    if let Some(container) = unsafe { quick.view.superview() } {
        window.setContentView(Some(&container));
    }
    window.setFrame_display(rect([200.0, 200.0, 1000.0, 620.0]), true);
    window.center();
    window.makeKeyAndOrderFront(None);
    quick.view.request_frame_now();
    let _ = mtm;
}

/// La última shell del terminal rápido terminó: se descarta (⌥Espacio crea otro).
pub fn dispose(_mtm: MainThreadMarker) {
    if let Some(quick) = QUICK.with(|q| q.borrow_mut().take()) {
        quick.window.orderOut(None);
        quick.window.close();
    }
}

/// Solo en depuración (`RT_QUICK_STEPS=pin,resign,toggle,promote`): ejecuta
/// esos pasos en orden y anota el estado tras cada uno en `RT_QUICK_DUMP`.
#[cfg(debug_assertions)]
pub fn debug_steps(mtm: MainThreadMarker, steps: &str, dump: Option<std::ffi::OsString>) {
    let mut out = String::new();
    for step in steps.split(',').map(str::trim).filter(|s| !s.is_empty()) {
        match step {
            "pin" => toggle_pin(),
            "resign" => on_resign(mtm),
            "toggle" => toggle(mtm),
            "show" => show(mtm),
            "promote" => promote(mtm),
            _ => {}
        }
        let state = QUICK.with(|q| {
            q.borrow()
                .as_ref()
                .map_or("sin terminal rápido".to_owned(), |q| {
                    format!(
                        "shown={} pinned={} visible={}",
                        q.shown.get(),
                        q.pinned.get(),
                        q.window.isVisible()
                    )
                })
        });
        out.push_str(&format!("{step}: {state}\n"));
    }
    if let Some(path) = dump {
        let _ = std::fs::write(path, out);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_panel_hugs_the_top_edge_and_hides_just_above_it() {
        // Pantalla de 1440×875 con la barra de menús ya descontada.
        let f = frames([0.0, 0.0, 1440.0, 875.0], 410.0);
        assert_eq!(f.shown, [16.0, 465.0, 1408.0, 410.0]);
        // Oculto: el mismo rectángulo, pero empezando donde acaba la pantalla.
        assert_eq!(f.hidden, [16.0, 875.0, 1408.0, 410.0]);
    }

    #[test]
    fn the_height_is_limited_and_other_screens_are_respected() {
        // Nunca más del 90 % de la pantalla ni menos de 160.
        assert_eq!(frames([0.0, 0.0, 1000.0, 500.0], 1600.0).shown[3], 450.0);
        assert_eq!(frames([0.0, 0.0, 1000.0, 500.0], 10.0).shown[3], 160.0);
        // Una pantalla a la derecha (x positivo) y más abajo (y negativo).
        let f = frames([1440.0, -200.0, 1920.0, 1080.0], 400.0);
        assert_eq!(f.shown[0], 1440.0 + 16.0);
        assert_eq!(f.shown[1], -200.0 + 1080.0 - 400.0);
        assert_eq!(f.hidden[1], -200.0 + 1080.0);
        // Una pantalla muy estrecha no da un ancho negativo.
        assert!(frames([0.0, 0.0, 200.0, 500.0], 300.0).shown[2] <= 200.0);
    }
}
