//! Vista de la paleta de comandos (⌘⇧P, pantalla 03 del spec): un diálogo
//! centrado de 640 pt sobre un velo, dibujado a mano con los tokens `dusk`.
//!
//! Recibe las teclas ella misma (sin `NSTextField`): ↑↓ mueven, ↵ ejecuta,
//! ⌘↵ inserta, ⇥ cambia de ámbito y Esc cierra. **Rellena solo sus límites.**

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSCompositingOperation,
    NSEvent, NSEventModifierFlags, NSRectFillUsingOperation, NSResponder, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize};

use crate::agent_ui::{draw_text, draw_text_ex, fill_rounded, ns_color, stroke_rounded};
use crate::command_palette::Scope;
use crate::palette::dusk;

pub const DIALOG_W: f64 = 640.0;
const SEARCH_H: f64 = 56.0;
const ROW_H: f64 = 48.0;
const FOOTER_H: f64 = 34.0;
/// Filas visibles como máximo.
pub const MAX_ROWS: usize = 8;

#[derive(Debug, Clone, PartialEq)]
pub struct PaletteRow {
    pub tag: &'static str,
    pub label: String,
    pub detail: String,
}

type Activate = Box<dyn Fn(usize, bool)>;
type Change = Box<dyn Fn(&str, Scope)>;
type Close = Box<dyn Fn()>;

pub struct PaletteIvars {
    query: RefCell<String>,
    scope: Cell<Scope>,
    rows: RefCell<Vec<PaletteRow>>,
    selected: Cell<usize>,
    on_activate: RefCell<Option<Activate>>,
    on_change: RefCell<Option<Change>>,
    on_close: RefCell<Option<Close>>,
}

/// El rectángulo del diálogo dentro de una vista de `bounds`, con `rows` filas.
fn dialog_rect(bounds: NSRect, rows: usize) -> NSRect {
    let w = DIALOG_W.min((bounds.size.width - 32.0).max(280.0));
    let h = SEARCH_H + rows.max(1) as f64 * ROW_H + FOOTER_H;
    let top = (bounds.size.height * 0.14).max(40.0);
    NSRect::new(
        NSPoint::new((bounds.size.width - w) / 2.0, top),
        NSSize::new(w, h),
    )
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTPaletteView"]
    #[ivars = PaletteIvars]
    pub struct PaletteView;

    unsafe impl NSObjectProtocol for PaletteView {}

    impl PaletteView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            // Velo (55 %).
            ns_color(dusk::BG, 0.55).setFill();
            NSRectFillUsingOperation(b, NSCompositingOperation::SourceOver);

            let iv = self.ivars();
            let rows = iv.rows.borrow();
            let d = dialog_rect(b, rows.len());
            fill_rounded(d, 12.0, dusk::POPOVER, 1.0);
            stroke_rounded(d, 12.0, dusk::LINE_STRONG, 1.0, 1.0);

            // Consulta y ámbito.
            let query = iv.query.borrow();
            let (text, color) = if query.is_empty() {
                ("Buscar acciones y comandos…".to_owned(), dusk::GHOST)
            } else {
                (format!("{query}\u{258F}"), dusk::TEXT)
            };
            draw_text(
                &text,
                NSRect::new(
                    NSPoint::new(d.origin.x + 20.0, d.origin.y + 18.0),
                    NSSize::new(d.size.width - 220.0, 22.0),
                ),
                16.0,
                0.0,
                color,
            );
            let mut x = d.origin.x + d.size.width - 16.0;
            for scope in [Scope::History, Scope::Actions, Scope::All] {
                let w = 12.0 + 7.0 * scope.label().chars().count() as f64;
                x -= w;
                let chip = NSRect::new(NSPoint::new(x, d.origin.y + 17.0), NSSize::new(w, 22.0));
                if scope == iv.scope.get() {
                    fill_rounded(chip, 11.0, dusk::ACCENT, 0.18);
                    stroke_rounded(chip, 11.0, dusk::ACCENT, 0.6, 1.0);
                }
                draw_text_ex(
                    scope.label(),
                    NSRect::new(
                        NSPoint::new(chip.origin.x + 8.0, chip.origin.y + 4.0),
                        NSSize::new(chip.size.width, 16.0),
                    ),
                    11.0,
                    0.4,
                    if scope == iv.scope.get() { dusk::ACCENT } else { dusk::MUTED },
                    false,
                    false,
                );
                x -= 6.0;
            }
            ns_color(dusk::LINE, 1.0).setFill();
            NSRectFillUsingOperation(
                NSRect::new(
                    NSPoint::new(d.origin.x, d.origin.y + SEARCH_H),
                    NSSize::new(d.size.width, 1.0),
                ),
                NSCompositingOperation::SourceOver,
            );

            // Filas.
            if rows.is_empty() {
                draw_text(
                    "Nada que mostrar",
                    NSRect::new(
                        NSPoint::new(d.origin.x + 20.0, d.origin.y + SEARCH_H + 14.0),
                        NSSize::new(d.size.width - 40.0, 20.0),
                    ),
                    13.0,
                    0.0,
                    dusk::MUTED,
                );
            }
            for (i, row) in rows.iter().enumerate() {
                let y = d.origin.y + SEARCH_H + 1.0 + i as f64 * ROW_H;
                if i == iv.selected.get() {
                    let r = NSRect::new(
                        NSPoint::new(d.origin.x + 8.0, y + 2.0),
                        NSSize::new(d.size.width - 16.0, ROW_H - 4.0),
                    );
                    fill_rounded(r, 8.0, dusk::ACCENT, 0.12);
                    stroke_rounded(r, 8.0, dusk::ACCENT, 0.45, 1.0);
                }
                let tag_color = if row.tag == "ACT" { dusk::ACCENT } else { dusk::ATTENTION };
                draw_text_ex(
                    row.tag,
                    NSRect::new(
                        NSPoint::new(d.origin.x + 20.0, y + 16.0),
                        NSSize::new(36.0, 16.0),
                    ),
                    11.0,
                    0.7,
                    tag_color,
                    true,
                    false,
                );
                draw_text(
                    &row.label,
                    NSRect::new(
                        NSPoint::new(d.origin.x + 62.0, y + 6.0),
                        NSSize::new(d.size.width - 82.0, 18.0),
                    ),
                    13.0,
                    0.3,
                    dusk::TEXT,
                );
                draw_text(
                    &row.detail,
                    NSRect::new(
                        NSPoint::new(d.origin.x + 62.0, y + 25.0),
                        NSSize::new(d.size.width - 82.0, 16.0),
                    ),
                    11.0,
                    0.0,
                    dusk::MUTED,
                );
            }

            // Pie con las teclas.
            let fy = d.origin.y + d.size.height - FOOTER_H;
            ns_color(dusk::LINE, 1.0).setFill();
            NSRectFillUsingOperation(
                NSRect::new(NSPoint::new(d.origin.x, fy), NSSize::new(d.size.width, 1.0)),
                NSCompositingOperation::SourceOver,
            );
            draw_text(
                "\u{21a9} ejecutar   \u{2318}\u{21a9} insertar sin ejecutar   \u{21e5} ámbito   esc cerrar",
                NSRect::new(
                    NSPoint::new(d.origin.x + 20.0, fy + 9.0),
                    NSSize::new(d.size.width - 40.0, 16.0),
                ),
                11.0,
                0.0,
                dusk::MUTED,
            );
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            let flags = event.modifierFlags();
            let cmd = flags.contains(NSEventModifierFlags::Command);
            match event.keyCode() {
                53 => self.close(),
                36 | 76 => self.activate(cmd),
                126 => self.move_selection(-1),
                125 => self.move_selection(1),
                48 => {
                    let next = self.ivars().scope.get().next();
                    self.ivars().scope.set(next);
                    self.changed();
                }
                51 => {
                    // Retroceso; con Opción borra la última palabra.
                    let mut q = self.ivars().query.borrow_mut();
                    if flags.contains(NSEventModifierFlags::Option) {
                        let trimmed = q.trim_end().len();
                        q.truncate(trimmed);
                        while q.chars().last().is_some_and(|c| !c.is_whitespace()) {
                            q.pop();
                        }
                    } else {
                        q.pop();
                    }
                    drop(q);
                    self.changed();
                }
                _ => {
                    if flags.contains(NSEventModifierFlags::Control) || cmd {
                        return;
                    }
                    let Some(chars) = event.characters() else {
                        return;
                    };
                    let typed: String = chars
                        .to_string()
                        .chars()
                        .filter(|c| !c.is_control())
                        .collect();
                    if !typed.is_empty() {
                        self.ivars().query.borrow_mut().push_str(&typed);
                        self.changed();
                    }
                }
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let rows = self.ivars().rows.borrow().len();
            let d = dialog_rect(self.bounds(), rows);
            let inside = p.x >= d.origin.x
                && p.x <= d.origin.x + d.size.width
                && p.y >= d.origin.y
                && p.y <= d.origin.y + d.size.height;
            if !inside {
                self.close();
                return;
            }
            let y = p.y - (d.origin.y + SEARCH_H + 1.0);
            if y >= 0.0 {
                let i = (y / ROW_H) as usize;
                if i < rows {
                    self.ivars().selected.set(i);
                    self.activate(event.modifierFlags().contains(NSEventModifierFlags::Command));
                }
            }
        }
    }
);

impl PaletteView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(PaletteIvars {
            query: RefCell::new(String::new()),
            scope: Cell::new(Scope::All),
            rows: RefCell::new(Vec::new()),
            selected: Cell::new(0),
            on_activate: RefCell::new(None),
            on_change: RefCell::new(None),
            on_close: RefCell::new(None),
        });
        // SAFETY: inicializador designado de NSView.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: frame] };
        // SAFETY: constante de AppKit.
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
        this.setAppearance(dark.as_deref());
        this
    }

    pub fn set_handlers(
        &self,
        on_activate: impl Fn(usize, bool) + 'static,
        on_change: impl Fn(&str, Scope) + 'static,
        on_close: impl Fn() + 'static,
    ) {
        let iv = self.ivars();
        *iv.on_activate.borrow_mut() = Some(Box::new(on_activate));
        *iv.on_change.borrow_mut() = Some(Box::new(on_change));
        *iv.on_close.borrow_mut() = Some(Box::new(on_close));
    }

    pub fn set_query(&self, query: &str) {
        *self.ivars().query.borrow_mut() = query.to_owned();
    }

    pub fn set_rows(&self, rows: Vec<PaletteRow>) {
        let iv = self.ivars();
        let n = rows.len();
        *iv.rows.borrow_mut() = rows;
        iv.selected.set(iv.selected.get().min(n.saturating_sub(1)));
        self.setNeedsDisplay(true);
    }

    pub fn query(&self) -> String {
        self.ivars().query.borrow().clone()
    }

    pub fn scope(&self) -> Scope {
        self.ivars().scope.get()
    }

    fn changed(&self) {
        self.ivars().selected.set(0);
        let (q, s) = (self.query(), self.scope());
        if let Some(f) = self.ivars().on_change.borrow().as_ref() {
            f(&q, s);
        }
        self.setNeedsDisplay(true);
    }

    fn move_selection(&self, delta: isize) {
        let iv = self.ivars();
        let n = iv.rows.borrow().len();
        if n == 0 {
            return;
        }
        let next = (iv.selected.get() as isize + delta).rem_euclid(n as isize) as usize;
        iv.selected.set(next);
        self.setNeedsDisplay(true);
    }

    fn activate(&self, insert: bool) {
        let iv = self.ivars();
        if iv.rows.borrow().is_empty() {
            return;
        }
        let i = iv.selected.get();
        if let Some(f) = iv.on_activate.borrow().as_ref() {
            f(i, insert);
        }
    }

    fn close(&self) {
        if let Some(f) = self.ivars().on_close.borrow().as_ref() {
            f();
        }
    }
}
