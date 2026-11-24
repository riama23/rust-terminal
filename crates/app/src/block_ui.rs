//! Barra de acciones del bloque seleccionado (v1.5), bajo la terminal.
//! Rellena solo sus propios límites (ver la regla de AppKit en `agent_ui`).

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSButton,
    NSCompositingOperation, NSFont, NSLineBreakMode, NSRectFill, NSRectFillUsingOperation,
    NSResponder, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::agent_ui::ns_color;
use crate::palette::dusk;

pub const BLOCK_BAR_HEIGHT: f64 = 44.0;

/// Qué enseña la barra.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct BlockBarInfo {
    pub label: String,
    pub failed: bool,
    pub running: bool,
    pub can_rerun: bool,
    pub can_open: bool,
}

/// Selectores que reciben los botones.
#[derive(Clone, Copy)]
pub struct BlockActions {
    pub stop: Sel,
    pub rerun: Sel,
    pub copy: Sel,
    pub open: Sel,
}

pub struct BlockBarIvars {
    label: Retained<NSTextField>,
    stop: Retained<NSButton>,
    rerun: Retained<NSButton>,
    copy: Retained<NSButton>,
    open: Retained<NSButton>,
    failed: std::cell::Cell<bool>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTBlockBar"]
    #[ivars = BlockBarIvars]
    pub struct BlockBar;

    unsafe impl NSObjectProtocol for BlockBar {}

    impl BlockBar {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(dusk::BG, 1.0).setFill();
            NSRectFill(b);
            let tint = if self.ivars().failed.get() {
                dusk::ERROR
            } else {
                dusk::ACCENT
            };
            ns_color(tint, 0.08).setFill();
            NSRectFillUsingOperation(b, NSCompositingOperation::SourceOver);
            ns_color(tint, 0.45).setFill();
            NSRectFillUsingOperation(
                NSRect::new(NSPoint::new(0.0, b.size.height - 1.0), NSSize::new(b.size.width, 1.0)),
                NSCompositingOperation::SourceOver,
            );
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: llamada a la implementación de NSView.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.relayout();
        }
    }
);

impl BlockBar {
    pub fn new(mtm: MainThreadMarker, target: &AnyObject, actions: BlockActions) -> Retained<Self> {
        let make = |title: &str, action: Sel| {
            // SAFETY: `target` implementa `action`.
            unsafe {
                NSButton::buttonWithTitle_target_action(
                    &NSString::from_str(title),
                    Some(target),
                    Some(action),
                    mtm,
                )
            }
        };
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        label.setTextColor(Some(&ns_color(dusk::TEXT, 1.0)));
        label.setFont(Some(&NSFont::monospacedSystemFontOfSize_weight(12.0, 0.0)));
        let this = Self::alloc(mtm).set_ivars(BlockBarIvars {
            label,
            stop: make("Detener", actions.stop),
            rerun: make("Relanzar", actions.rerun),
            copy: make("Copiar salida", actions.copy),
            open: make("Abrir en el editor  \u{2318}O", actions.open),
            failed: std::cell::Cell::new(false),
        });
        // SAFETY: inicializador designado de NSView.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        // SAFETY: constante de AppKit.
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
        this.setAppearance(dark.as_deref());
        let iv = this.ivars();
        for v in [
            &*iv.label as &NSView,
            &iv.stop,
            &iv.rerun,
            &iv.copy,
            &iv.open,
        ] {
            this.addSubview(v);
        }
        this
    }

    pub fn set_info(&self, info: &BlockBarInfo) {
        let iv = self.ivars();
        iv.label.setStringValue(&NSString::from_str(&info.label));
        iv.failed.set(info.failed);
        iv.stop.setHidden(!info.running);
        iv.rerun.setHidden(info.running);
        iv.rerun.setEnabled(info.can_rerun);
        iv.copy.setHidden(info.running);
        iv.open.setHidden(!info.can_open);
        self.setNeedsDisplay(true);
        self.relayout();
    }

    fn relayout(&self) {
        let b = self.bounds();
        let iv = self.ivars();
        let (h, margin, gap) = (28.0, 12.0, 8.0);
        let y = (b.size.height - h) / 2.0;
        let mut right = b.size.width - margin;
        for (button, w) in [
            (&iv.open, 190.0),
            (&iv.copy, 120.0),
            (&iv.rerun, 90.0),
            (&iv.stop, 90.0),
        ] {
            if button.isHidden() {
                continue;
            }
            right -= w;
            button.setFrame(NSRect::new(NSPoint::new(right, y), NSSize::new(w, h)));
            right -= gap;
        }
        iv.label.setFrame(NSRect::new(
            NSPoint::new(margin, (b.size.height - 18.0) / 2.0),
            NSSize::new((right - margin).max(0.0), 18.0),
        ));
    }
}
