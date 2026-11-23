//! Contenedor de la ventana: reparte el área entre la terminal y paneles
//! laterales (sidebar de sesiones a la izquierda, inspector a la derecha).
//!
//! La [`TerminalView`](crate::view::TerminalView) es una subvista más: sigue
//! siendo quien aloja la `CAMetalLayer` y calcula su propio layout a partir de
//! `bounds()`, de modo que el render no sabe que hay un contenedor. Sin paneles
//! laterales la terminal ocupa todo el contenedor, igual que antes.

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadOnly, Message, define_class, msg_send};
use objc2_app_kit::{NSColor, NSRectFill, NSResponder, NSView, NSWindow};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

use crate::view::TerminalView;

/// Ancho mínimo que se deja a la terminal al repartir.
pub const MIN_TERMINAL_WIDTH: f64 = 320.0;

/// Reparte `total` puntos de ancho. Devuelve `(izquierdo, x_terminal,
/// ancho_terminal, derecho)`. Si los paneles no caben, se encoge primero el
/// derecho y después el izquierdo, para que la terminal conserve su mínimo.
pub fn split_widths(total: f64, left: f64, right: f64, min_terminal: f64) -> (f64, f64, f64, f64) {
    let total = total.max(0.0);
    let mut left = left.max(0.0);
    let mut right = right.max(0.0);
    let room = (total - min_terminal).max(0.0);
    if left + right > room {
        right = right.min((room - left).max(0.0));
        left = left.min(room);
    }
    (left, left, total - left - right, right)
}

type Panel = Option<(Retained<NSView>, f64)>;

pub struct ContainerIvars {
    terminal: RefCell<Option<Retained<NSView>>>,
    left: RefCell<Panel>,
    right: RefCell<Panel>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y `ContainerView`
    // no implementa `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTContainerView"]
    #[ivars = ContainerIvars]
    pub struct ContainerView;

    unsafe impl NSObjectProtocol for ContainerView {}

    impl ContainerView {
        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: llamada a la implementación de NSView.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.relayout();
        }
    }
);

impl ContainerView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ContainerIvars {
            terminal: RefCell::new(None),
            left: RefCell::new(None),
            right: RefCell::new(None),
        });
        // SAFETY: inicializador designado de NSView.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    /// La vista que ocupa el centro (la terminal).
    pub fn set_terminal(&self, view: &NSView) {
        self.addSubview(view);
        *self.ivars().terminal.borrow_mut() = Some(view.retain());
        self.relayout();
    }

    /// La terminal que ocupa el centro.
    pub fn terminal(&self) -> Option<Retained<TerminalView>> {
        let view = self.ivars().terminal.borrow().clone()?;
        view.downcast::<TerminalView>().ok()
    }

    /// Panel izquierdo con su ancho en puntos; `None` lo quita.
    pub fn set_left(&self, panel: Option<(&NSView, f64)>) {
        self.replace(&self.ivars().left, panel);
    }

    /// Panel derecho con su ancho en puntos; `None` lo quita. Reservado para
    /// el inspector (v1.4).
    #[allow(dead_code)]
    pub fn set_right(&self, panel: Option<(&NSView, f64)>) {
        self.replace(&self.ivars().right, panel);
    }

    fn replace(&self, slot: &RefCell<Panel>, panel: Option<(&NSView, f64)>) {
        if let Some((old, _)) = slot.borrow_mut().take() {
            old.removeFromSuperview();
        }
        if let Some((view, width)) = panel {
            self.addSubview(view);
            *slot.borrow_mut() = Some((view.retain(), width));
        }
        self.relayout();
    }

    /// Coloca terminal y paneles según el tamaño actual.
    pub fn relayout(&self) {
        let ivars = self.ivars();
        let bounds = self.bounds();
        let (w, h) = (bounds.size.width, bounds.size.height);
        let want = |p: &Panel| p.as_ref().map_or(0.0, |(_, width)| *width);
        let (left_w, term_x, term_w, right_w) = split_widths(
            w,
            want(&ivars.left.borrow()),
            want(&ivars.right.borrow()),
            MIN_TERMINAL_WIDTH,
        );
        if let Some((view, _)) = ivars.left.borrow().as_ref() {
            view.setFrame(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(left_w, h)));
        }
        if let Some(term) = ivars.terminal.borrow().as_ref() {
            term.setFrame(NSRect::new(
                NSPoint::new(term_x, 0.0),
                NSSize::new(term_w, h),
            ));
        }
        if let Some((view, _)) = ivars.right.borrow().as_ref() {
            view.setFrame(NSRect::new(
                NSPoint::new(w - right_w, 0.0),
                NSSize::new(right_w, h),
            ));
        }
    }
}

/// La `TerminalView` de una ventana. Es el único sitio que sabe cómo llegar a
/// ella desde la vista de contenido: **no hay que hacer `downcast` de
/// `contentView()` a mano**, porque la vista de contenido es el contenedor.
pub fn terminal_view(window: &NSWindow) -> Option<Retained<TerminalView>> {
    let content = window.contentView()?;
    match content.downcast::<ContainerView>() {
        Ok(container) => container.terminal(),
        Err(content) => content.downcast::<TerminalView>().ok(),
    }
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTDebugPanel"]
    pub struct DebugPanel;

    unsafe impl NSObjectProtocol for DebugPanel {}

    impl DebugPanel {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, rect: NSRect) {
            NSColor::colorWithSRGBRed_green_blue_alpha(0.09, 0.09, 0.11, 1.0).setFill();
            NSRectFill(rect);
        }
    }
);

impl DebugPanel {
    /// Panel liso para comprobar el reparto del contenedor
    /// (`RT_DEBUG_SIDEBAR=1`). Lo sustituyen los paneles reales de la v1.2.
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(());
        // SAFETY: inicializador designado de NSView.
        unsafe { msg_send![super(this), init] }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sin_paneles_la_terminal_ocupa_todo() {
        assert_eq!(
            split_widths(1000.0, 0.0, 0.0, 320.0),
            (0.0, 0.0, 1000.0, 0.0)
        );
    }

    #[test]
    fn reparte_con_ambos_paneles() {
        assert_eq!(
            split_widths(1200.0, 260.0, 300.0, 320.0),
            (260.0, 260.0, 640.0, 300.0)
        );
    }

    #[test]
    fn si_no_caben_se_encoge_primero_el_derecho() {
        // Quedan 1000 - 320 = 680 para paneles: 260 a la izquierda y 420 de 700 a la derecha.
        let (l, x, t, r) = split_widths(1000.0, 260.0, 700.0, 320.0);
        assert_eq!((l, x, t, r), (260.0, 260.0, 320.0, 420.0));
    }

    #[test]
    fn si_aun_asi_no_caben_se_encoge_el_izquierdo() {
        let (l, _, t, r) = split_widths(500.0, 400.0, 300.0, 320.0);
        assert_eq!((l, t, r), (180.0, 320.0, 0.0));
    }

    #[test]
    fn ventana_mas_estrecha_que_el_minimo_no_da_anchos_negativos() {
        let (l, x, t, r) = split_widths(200.0, 260.0, 300.0, 320.0);
        assert_eq!((l, x, t, r), (0.0, 0.0, 200.0, 0.0));
    }

    #[test]
    fn la_suma_de_anchos_es_siempre_el_total() {
        for total in [0.0, 100.0, 320.0, 700.0, 1600.0] {
            for (l, r) in [
                (0.0, 0.0),
                (260.0, 0.0),
                (0.0, 300.0),
                (260.0, 300.0),
                (900.0, 900.0),
            ] {
                let (lw, _, tw, rw) = split_widths(total, l, r, 320.0);
                assert!(
                    (lw + tw + rw - total).abs() < 1e-9,
                    "total={total} l={l} r={r}"
                );
                assert!(lw >= 0.0 && tw >= 0.0 && rw >= 0.0);
            }
        }
    }
}
