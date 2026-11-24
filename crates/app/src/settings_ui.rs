//! Vistas de la ventana de ajustes (⌘,): barra de páginas a la izquierda y,
//! a la derecha, una fila por opción de `config::OPTIONS` con su control.
//! Dibujada con los tokens `dusk`. **Cada vista rellena solo sus límites.**

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSButton, NSControl,
    NSControlStateValueOn, NSEvent, NSFont, NSPopUpButton, NSRectFill, NSResponder, NSScrollView,
    NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::agent_ui::{draw_text, fill_rounded, ns_color};
use crate::palette::dusk;

pub const WINDOW_W: f64 = 960.0;
pub const WINDOW_H: f64 = 680.0;
pub const SIDEBAR_W: f64 = 220.0;
const HEADER_H: f64 = 96.0;
const PAGE_ROW_H: f64 = 34.0;
const PAGES_TOP: f64 = 70.0;

/// Las páginas de la barra y las secciones de `config.toml` que muestran.
pub const PAGES: &[(&str, &[&str])] = &[
    (
        "General",
        &["shell", "scrollback", "clipboard", "notifications", "bell"],
    ),
    ("Apariencia", &["font", "colors", "window"]),
    ("Teclado", &["keybindings"]),
    ("Bloques y agentes", &["blocks", "agents"]),
];

/// El control de una fila.
#[derive(Debug, Clone, PartialEq)]
pub enum Control {
    Switch(bool),
    Text(String),
    Choice(Vec<String>, usize),
    ReadOnly(String),
}

#[derive(Debug, Clone, PartialEq)]
pub struct RowSpec {
    /// Posición en `config::OPTIONS` (va en el `tag` del control).
    pub index: usize,
    pub label: String,
    /// `sección.clave`, tal como está en `config.toml`.
    pub path: String,
    pub control: Control,
    /// La opción está puesta en el archivo (se puede restablecer).
    pub overridden: bool,
}

/// Selectores de los controles.
#[derive(Clone, Copy)]
pub struct SettingsActions {
    pub toggle: Sel,
    pub commit: Sel,
    pub reset: Sel,
    pub open_file: Sel,
}

type PageHandler = Box<dyn Fn(usize)>;

pub struct RootIvars {
    selected: Cell<usize>,
    on_page: RefCell<Option<PageHandler>>,
    title: Retained<NSTextField>,
    note: Retained<NSTextField>,
    status: Retained<NSTextField>,
    open_file: Retained<NSButton>,
    scroll: Retained<NSScrollView>,
    doc: Retained<DocView>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTSettingsDoc"]
    pub struct DocView;

    unsafe impl NSObjectProtocol for DocView {}

    impl DocView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            ns_color(dusk::BG, 1.0).setFill();
            NSRectFill(self.bounds());
        }
    }
);

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTSettingsRoot"]
    #[ivars = RootIvars]
    pub struct SettingsRoot;

    unsafe impl NSObjectProtocol for SettingsRoot {}

    impl SettingsRoot {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(dusk::BG, 1.0).setFill();
            NSRectFill(b);
            // Barra de páginas.
            ns_color(dusk::SIDEBAR, 1.0).setFill();
            NSRectFill(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(SIDEBAR_W, b.size.height)));
            ns_color(dusk::LINE, 1.0).setFill();
            NSRectFill(NSRect::new(NSPoint::new(SIDEBAR_W - 1.0, 0.0), NSSize::new(1.0, b.size.height)));
            draw_text(
                "AJUSTES",
                NSRect::new(NSPoint::new(20.0, 34.0), NSSize::new(SIDEBAR_W - 40.0, 16.0)),
                11.0,
                0.6,
                dusk::MUTED,
            );
            for (i, (name, _)) in PAGES.iter().enumerate() {
                let y = PAGES_TOP + i as f64 * PAGE_ROW_H;
                if i == self.ivars().selected.get() {
                    let r = NSRect::new(
                        NSPoint::new(10.0, y),
                        NSSize::new(SIDEBAR_W - 20.0, PAGE_ROW_H - 4.0),
                    );
                    fill_rounded(r, 7.0, dusk::ACCENT, 0.16);
                }
                draw_text(
                    name,
                    NSRect::new(NSPoint::new(24.0, y + 7.0), NSSize::new(SIDEBAR_W - 48.0, 18.0)),
                    13.0,
                    if i == self.ivars().selected.get() { 0.5 } else { 0.0 },
                    if i == self.ivars().selected.get() { dusk::TEXT } else { dusk::MUTED },
                );
            }
            // Cabecera de la página.
            ns_color(dusk::LINE, 1.0).setFill();
            NSRectFill(NSRect::new(
                NSPoint::new(SIDEBAR_W, HEADER_H),
                NSSize::new(b.size.width - SIDEBAR_W, 1.0),
            ));
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            if p.x >= SIDEBAR_W || p.y < PAGES_TOP {
                return;
            }
            let i = ((p.y - PAGES_TOP) / PAGE_ROW_H) as usize;
            if i < PAGES.len() {
                self.ivars().selected.set(i);
                self.setNeedsDisplay(true);
                if let Some(f) = self.ivars().on_page.borrow().as_ref() {
                    f(i);
                }
            }
        }
    }
);

fn label(
    mtm: MainThreadMarker,
    text: &str,
    size: f64,
    color: [u8; 3],
    mono: bool,
) -> Retained<NSTextField> {
    let l = NSTextField::wrappingLabelWithString(&NSString::from_str(text), mtm);
    let font = if mono {
        NSFont::monospacedSystemFontOfSize_weight(size, 0.0)
    } else {
        NSFont::systemFontOfSize(size)
    };
    l.setFont(Some(&font));
    l.setTextColor(Some(&ns_color(color, 1.0)));
    l
}

impl SettingsRoot {
    pub fn new(
        mtm: MainThreadMarker,
        target: &AnyObject,
        actions: SettingsActions,
    ) -> Retained<Self> {
        let open_file = unsafe {
            // SAFETY: `target` implementa `actions.open_file`.
            NSButton::buttonWithTitle_target_action(
                &NSString::from_str("Abrir config.toml"),
                Some(target),
                Some(actions.open_file),
                mtm,
            )
        };
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        let doc: Retained<DocView> = {
            let d = DocView::alloc(mtm).set_ivars(());
            // SAFETY: inicializador designado de NSView.
            unsafe { msg_send![super(d), init] }
        };
        scroll.setDocumentView(Some(&doc));
        let title = label(mtm, "", 20.0, dusk::TEXT, false);
        title.setFont(Some(&NSFont::systemFontOfSize_weight(20.0, 0.6)));
        let ivars = RootIvars {
            selected: Cell::new(0),
            on_page: RefCell::new(None),
            title,
            note: label(mtm, "", 12.0, dusk::MUTED, false),
            status: label(mtm, "", 12.0, dusk::OK, false),
            open_file,
            scroll,
            doc,
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: inicializador designado de NSView.
        let this: Retained<Self> = unsafe {
            msg_send![super(this), initWithFrame: NSRect::new(NSPoint::ZERO, NSSize::new(WINDOW_W, WINDOW_H))]
        };
        // SAFETY: constante de AppKit.
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
        this.setAppearance(dark.as_deref());
        let iv = this.ivars();
        iv.title.setFrame(NSRect::new(
            NSPoint::new(SIDEBAR_W + 24.0, 22.0),
            NSSize::new(380.0, 28.0),
        ));
        iv.note.setFrame(NSRect::new(
            NSPoint::new(SIDEBAR_W + 24.0, 54.0),
            NSSize::new(400.0, 32.0),
        ));
        iv.status.setFrame(NSRect::new(
            NSPoint::new(SIDEBAR_W + 430.0, 58.0),
            NSSize::new(WINDOW_W - SIDEBAR_W - 454.0, 32.0),
        ));
        iv.open_file.setFrame(NSRect::new(
            NSPoint::new(WINDOW_W - 24.0 - 150.0, 20.0),
            NSSize::new(150.0, 28.0),
        ));
        iv.scroll.setFrame(NSRect::new(
            NSPoint::new(SIDEBAR_W, HEADER_H + 1.0),
            NSSize::new(WINDOW_W - SIDEBAR_W, WINDOW_H - HEADER_H - 1.0),
        ));
        for v in [
            &*iv.title as &NSView,
            &iv.note,
            &iv.status,
            &iv.open_file,
            &iv.scroll,
        ] {
            this.addSubview(v);
        }
        this
    }

    pub fn set_on_page(&self, f: impl Fn(usize) + 'static) {
        *self.ivars().on_page.borrow_mut() = Some(Box::new(f));
    }

    pub fn select_page(&self, i: usize) {
        self.ivars().selected.set(i);
        self.setNeedsDisplay(true);
    }

    pub fn set_header(&self, title: &str, note: &str) {
        self.ivars()
            .title
            .setStringValue(&NSString::from_str(title));
        self.ivars().note.setStringValue(&NSString::from_str(note));
    }

    /// Mensaje de estado junto a la cabecera (verde, o rojo si es un error).
    pub fn set_status(&self, text: &str, error: bool) {
        let iv = self.ivars();
        iv.status.setStringValue(&NSString::from_str(text));
        iv.status.setTextColor(Some(&ns_color(
            if error { dusk::ERROR } else { dusk::OK },
            1.0,
        )));
    }

    /// Reconstruye las filas de la página.
    pub fn show_rows(
        &self,
        mtm: MainThreadMarker,
        rows: &[RowSpec],
        target: &AnyObject,
        actions: SettingsActions,
    ) {
        let iv = self.ivars();
        for v in iv.doc.subviews().iter() {
            v.removeFromSuperview();
        }
        let width = WINDOW_W - SIDEBAR_W;
        let (margin, label_w, control_x, control_w) = (24.0, 380.0, 420.0, 200.0);
        let mut y = 8.0;
        for row in rows {
            let lines = (row.label.chars().count() as f64 / 58.0).ceil().max(1.0);
            let label_h = lines * 17.0;
            let row_h = (label_h + 4.0 + 15.0 + 22.0).max(64.0);

            let l = label(mtm, &row.label, 13.0, dusk::TEXT, false);
            l.setFrame(NSRect::new(
                NSPoint::new(margin, y + 10.0),
                NSSize::new(label_w, label_h),
            ));
            let path = label(mtm, &row.path, 11.0, dusk::GHOST, true);
            path.setFrame(NSRect::new(
                NSPoint::new(margin, y + 10.0 + label_h + 3.0),
                NSSize::new(label_w, 15.0),
            ));
            iv.doc.addSubview(&l);
            iv.doc.addSubview(&path);

            let frame = NSRect::new(
                NSPoint::new(control_x, y + 12.0),
                NSSize::new(control_w, 26.0),
            );
            let tag = row.index as isize;
            match &row.control {
                Control::Switch(on) => {
                    // SAFETY: `target` implementa `actions.toggle`.
                    let b = unsafe {
                        NSButton::checkboxWithTitle_target_action(
                            &NSString::from_str(if *on { "Activado" } else { "Desactivado" }),
                            Some(target),
                            Some(actions.toggle),
                            mtm,
                        )
                    };
                    if *on {
                        b.setState(NSControlStateValueOn);
                    }
                    b.setTag(tag);
                    b.setFrame(frame);
                    iv.doc.addSubview(&b);
                }
                Control::Text(value) => {
                    let f = NSTextField::textFieldWithString(&NSString::from_str(value), mtm);
                    f.setFrame(frame);
                    f.setTag(tag);
                    // SAFETY: `target` implementa `actions.commit`.
                    unsafe {
                        f.setTarget(Some(target));
                        f.setAction(Some(actions.commit));
                    }
                    iv.doc.addSubview(&f);
                }
                Control::Choice(options, selected) => {
                    let p = NSPopUpButton::initWithFrame_pullsDown(
                        NSPopUpButton::alloc(mtm),
                        frame,
                        false,
                    );
                    for o in options {
                        p.addItemWithTitle(&NSString::from_str(o));
                    }
                    p.selectItemAtIndex(*selected as isize);
                    p.setTag(tag);
                    // SAFETY: `target` implementa `actions.commit`.
                    unsafe {
                        p.setTarget(Some(target));
                        p.setAction(Some(actions.commit));
                    }
                    iv.doc.addSubview(&p);
                }
                Control::ReadOnly(text) => {
                    let l = label(mtm, text, 12.0, dusk::MUTED, false);
                    l.setFrame(NSRect::new(
                        NSPoint::new(control_x, y + 14.0),
                        NSSize::new(control_w + 90.0, 34.0),
                    ));
                    iv.doc.addSubview(&l);
                }
            }
            if row.overridden {
                // SAFETY: `target` implementa `actions.reset`.
                let r = unsafe {
                    NSButton::buttonWithTitle_target_action(
                        &NSString::from_str("Restablecer"),
                        Some(target),
                        Some(actions.reset),
                        mtm,
                    )
                };
                r.setTag(tag);
                r.setFrame(NSRect::new(
                    NSPoint::new(control_x + control_w + 10.0, y + 11.0),
                    NSSize::new(100.0, 28.0),
                ));
                iv.doc.addSubview(&r);
            }
            y += row_h;
        }
        let total = (y + 16.0).max(WINDOW_H - HEADER_H - 1.0);
        iv.doc
            .setFrame(NSRect::new(NSPoint::ZERO, NSSize::new(width, total)));
        iv.doc.setNeedsDisplay(true);
        // Arriba del todo al cambiar de página.
        iv.doc.scrollPoint(NSPoint::ZERO);
    }
}

impl SettingsRoot {
    /// Solo en depuración: pulsa el control con ese `tag` (como un clic).
    #[cfg(debug_assertions)]
    pub fn debug_click(&self, tag: isize) -> bool {
        for v in self.ivars().doc.subviews().iter() {
            // SAFETY: `isKindOfClass:` y `tag` existen en los controles.
            let is_button: bool =
                unsafe { msg_send![&*v, isKindOfClass: <NSButton as objc2::ClassType>::class()] };
            if is_button {
                let b: &NSButton = unsafe { &*(&*v as *const NSView as *const NSButton) };
                if b.tag() == tag && b.title().to_string() != "Restablecer" {
                    // SAFETY: pulsar un botón de la propia ventana.
                    unsafe { b.performClick(None) };
                    return true;
                }
            }
        }
        false
    }
}

/// Un control cualquiera → su `tag` (el índice de la opción).
pub fn tag_of(sender: &AnyObject) -> isize {
    // SAFETY: todos los controles de la ventana son `NSControl`.
    let c: &NSControl = unsafe { &*(sender as *const AnyObject as *const NSControl) };
    c.tag()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn every_config_section_is_on_a_page() {
        for o in crate::config::OPTIONS {
            assert!(
                PAGES.iter().any(|(_, secs)| secs.contains(&o.section)),
                "la sección `{}` no está en ninguna página de ajustes",
                o.section
            );
        }
        // Y ninguna página nombra una sección que no existe.
        for (_, secs) in PAGES {
            for s in *secs {
                assert!(
                    crate::config::OPTIONS.iter().any(|o| o.section == *s),
                    "{s}"
                );
            }
        }
    }
}
