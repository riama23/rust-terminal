//! Vistas AppKit del agente: sidebar de sesiones y attention bar.
//!
//! Dibujan con los tokens `palette::dusk`. **Cada vista rellena solo sus
//! propios límites**: desde macOS 14 una vista no recorta su dibujo y, si se
//! sale, tapa a las vecinas (la terminal incluida).

use std::cell::RefCell;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearance, NSAppearanceCustomization, NSAppearanceNameDarkAqua, NSBezierPath, NSButton,
    NSColor, NSCompositingOperation, NSEvent, NSFont, NSFontAttributeName,
    NSForegroundColorAttributeName, NSLineBreakMode, NSMutableParagraphStyle,
    NSParagraphStyleAttributeName, NSRectFill, NSRectFillUsingOperation, NSResponder,
    NSStringDrawing, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use crate::palette::dusk;

pub const SIDEBAR_WIDTH: f64 = 264.0;
pub const ATTENTION_BAR_HEIGHT: f64 = 88.0;

pub fn ns_color(c: [u8; 3], alpha: f64) -> Retained<NSColor> {
    NSColor::colorWithSRGBRed_green_blue_alpha(
        c[0] as f64 / 255.0,
        c[1] as f64 / 255.0,
        c[2] as f64 / 255.0,
        alpha,
    )
}

/// Dibuja una línea de texto recortada con «…» si no cabe.
pub fn draw_text(text: &str, rect: NSRect, size: f64, weight: f64, color: [u8; 3]) {
    let font = NSFont::systemFontOfSize_weight(size, weight);
    let style = NSMutableParagraphStyle::new();
    style.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    let fg = ns_color(color, 1.0);
    // SAFETY: constantes de AppKit.
    let keys = unsafe {
        [
            NSFontAttributeName,
            NSForegroundColorAttributeName,
            NSParagraphStyleAttributeName,
        ]
    };
    let values: [&AnyObject; 3] = [&font, &fg, &style];
    let attrs = NSDictionary::from_slices(&keys, &values);
    // SAFETY: se llama desde `drawRect:` con un contexto gráfico activo.
    unsafe { NSString::from_str(text).drawInRect_withAttributes(rect, Some(&attrs)) };
}

fn fill_rounded(rect: NSRect, radius: f64, color: [u8; 3], alpha: f64) {
    ns_color(color, alpha).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
}

fn stroke_rounded(rect: NSRect, radius: f64, color: [u8; 3], alpha: f64, width: f64) {
    ns_color(color, alpha).setStroke();
    let path = NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius);
    path.setLineWidth(width);
    path.stroke();
}

/// Vuelca una vista a un PNG sin pasar por la pantalla (sirve con el monitor
/// apagado). Solo para verificar la interfaz: `RT_SNAPSHOT_DIR`.
pub fn snapshot_png(view: &NSView, path: &str) -> bool {
    use objc2_app_kit::{NSBitmapImageFileType, NSBitmapImageRep};
    let bounds = view.bounds();
    let Some(rep) = view.bitmapImageRepForCachingDisplayInRect(bounds) else {
        return false;
    };
    view.cacheDisplayInRect_toBitmapImageRep(bounds, &rep);
    let props = NSDictionary::new();
    // SAFETY: propiedades vacías, válidas para PNG.
    let data =
        unsafe { rep.representationUsingType_properties(NSBitmapImageFileType::PNG, &props) };
    let _: &NSBitmapImageRep = &rep;
    data.is_some_and(|d| d.writeToFile_atomically(&NSString::from_str(path), true))
}

// --- Sidebar ---

/// Una fila de la sidebar, ya resuelta (sin lógica de estados).
#[derive(Debug, Clone, PartialEq)]
pub struct SidebarRow {
    pub key: String,
    pub title: String,
    pub icon: &'static str,
    pub detail: String,
    pub color: [u8; 3],
    /// Anillo ámbar: la sesión necesita al usuario.
    pub ring: bool,
    pub selected: bool,
}

const HEADER_H: f64 = 40.0;
const ROW_H: f64 = 56.0;
const ROW_GAP: f64 = 4.0;

/// Fila bajo `y` (coordenadas de la vista, de arriba abajo).
pub fn row_at(y: f64, count: usize) -> Option<usize> {
    if y < HEADER_H {
        return None;
    }
    let i = ((y - HEADER_H) / (ROW_H + ROW_GAP)) as usize;
    let within = (y - HEADER_H) - i as f64 * (ROW_H + ROW_GAP) < ROW_H;
    (i < count && within).then_some(i)
}

type SelectHandler = Box<dyn Fn(&str)>;

pub struct SidebarIvars {
    rows: RefCell<Vec<SidebarRow>>,
    on_select: RefCell<Option<SelectHandler>>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTSidebarView"]
    #[ivars = SidebarIvars]
    pub struct SidebarView;

    unsafe impl NSObjectProtocol for SidebarView {}

    impl SidebarView {
        /// Origen arriba a la izquierda: el layout se escribe de arriba abajo.
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(dusk::SIDEBAR, 1.0).setFill();
            NSRectFill(b);
            // Borde derecho.
            ns_color(dusk::LINE, 1.0).setFill();
            NSRectFill(NSRect::new(
                NSPoint::new(b.size.width - 1.0, 0.0),
                NSSize::new(1.0, b.size.height),
            ));
            draw_text(
                "AGENTES",
                NSRect::new(NSPoint::new(16.0, 14.0), NSSize::new(b.size.width - 32.0, 16.0)),
                11.0,
                0.6,
                dusk::MUTED,
            );
            for (i, row) in self.ivars().rows.borrow().iter().enumerate() {
                let y = HEADER_H + i as f64 * (ROW_H + ROW_GAP);
                let r = NSRect::new(
                    NSPoint::new(8.0, y),
                    NSSize::new(b.size.width - 17.0, ROW_H),
                );
                if row.selected {
                    fill_rounded(r, 10.0, dusk::ACCENT, 0.07);
                    stroke_rounded(r, 10.0, dusk::ACCENT, 0.5, 1.0);
                }
                if row.ring {
                    stroke_rounded(r, 10.0, dusk::ATTENTION, 0.9, 1.5);
                }
                // Icono de estado (siempre con texto al lado: nunca solo color).
                draw_text(
                    row.icon,
                    NSRect::new(NSPoint::new(r.origin.x + 12.0, y + 16.0), NSSize::new(20.0, 22.0)),
                    15.0,
                    0.5,
                    row.color,
                );
                let text_x = r.origin.x + 38.0;
                let text_w = r.size.width - 38.0 - 10.0;
                draw_text(
                    &row.title,
                    NSRect::new(NSPoint::new(text_x, y + 10.0), NSSize::new(text_w, 18.0)),
                    13.0,
                    0.4,
                    dusk::TEXT,
                );
                draw_text(
                    &row.detail,
                    NSRect::new(NSPoint::new(text_x, y + 31.0), NSSize::new(text_w, 16.0)),
                    11.0,
                    0.0,
                    if row.ring { dusk::ATTENTION } else { dusk::MUTED },
                );
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let key = {
                let rows = self.ivars().rows.borrow();
                row_at(p.y, rows.len()).map(|i| rows[i].key.clone())
            };
            if let (Some(key), Some(f)) = (key, self.ivars().on_select.borrow().as_ref()) {
                f(&key);
            }
        }

        /// Un clic sobre una ventana inactiva también selecciona la fila.
        #[unsafe(method(acceptsFirstMouse:))]
        fn accepts_first_mouse(&self, _event: Option<&NSEvent>) -> bool {
            true
        }
    }
);

impl SidebarView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(SidebarIvars {
            rows: RefCell::new(Vec::new()),
            on_select: RefCell::new(None),
        });
        // SAFETY: inicializador designado de NSView.
        unsafe { msg_send![super(this), init] }
    }

    pub fn set_on_select(&self, f: impl Fn(&str) + 'static) {
        *self.ivars().on_select.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_rows(&self, rows: Vec<SidebarRow>) {
        if *self.ivars().rows.borrow() != rows {
            *self.ivars().rows.borrow_mut() = rows;
            self.setNeedsDisplay(true);
        }
    }
}

// --- Attention bar ---

pub struct BarIvars {
    label: Retained<NSTextField>,
    allow: Retained<NSButton>,
    always: Retained<NSButton>,
    deny: Retained<NSButton>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTAttentionBar"]
    #[ivars = BarIvars]
    pub struct AttentionBar;

    unsafe impl NSObjectProtocol for AttentionBar {}

    impl AttentionBar {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(dusk::BG, 1.0).setFill();
            NSRectFill(b);
            // attention al 8 % y borde superior al 45 %. `NSRectFill` copia en
            // vez de mezclar: con colores translúcidos hay que usar SourceOver.
            ns_color(dusk::ATTENTION, 0.08).setFill();
            NSRectFillUsingOperation(b, NSCompositingOperation::SourceOver);
            ns_color(dusk::ATTENTION, 0.45).setFill();
            NSRectFillUsingOperation(
                NSRect::new(
                    NSPoint::new(0.0, b.size.height - 1.0),
                    NSSize::new(b.size.width, 1.0),
                ),
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

impl AttentionBar {
    /// `target` recibe las tres acciones (sin argumento más que el remitente).
    pub fn new(
        mtm: MainThreadMarker,
        target: &AnyObject,
        allow: Sel,
        always: Sel,
        deny: Sel,
    ) -> Retained<Self> {
        let button = |title: &str, action: Sel| {
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
        label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        let ivars = BarIvars {
            label,
            allow: button("Permitir una vez  \u{2318}\u{21a9}", allow),
            always: button("Siempre permitir", always),
            deny: button("Denegar\u{2026}  \u{2318}\u{232b}", deny),
        };
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: inicializador designado de NSView.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        // La interfaz es oscura (Dusk): los botones también.
        // SAFETY: constante de AppKit.
        let dark = NSAppearance::appearanceNamed(unsafe { NSAppearanceNameDarkAqua });
        this.setAppearance(dark.as_deref());
        let iv = this.ivars();
        this.addSubview(&iv.label);
        this.addSubview(&iv.allow);
        this.addSubview(&iv.always);
        this.addSubview(&iv.deny);
        this
    }

    /// Qué se pide. `can_always`: hay una regla exacta que ofrecer.
    pub fn set_request(&self, tool: &str, detail: &str, can_always: bool) {
        let first_line = detail.lines().next().unwrap_or("");
        let text = if first_line.is_empty() {
            tool.to_owned()
        } else {
            format!("{tool}   {first_line}")
        };
        self.ivars()
            .label
            .setStringValue(&NSString::from_str(&text));
        self.ivars().always.setEnabled(can_always);
    }

    /// Dos filas: arriba lo que se aprueba (a todo el ancho, para que una orden
    /// larga se lea) y debajo los botones, alineados a la derecha.
    fn relayout(&self) {
        let b = self.bounds();
        let iv = self.ivars();
        let (button_h, gap, margin) = (28.0, 8.0, 16.0);
        let widths = [190.0, 150.0, 150.0];
        let mut x = b.size.width - margin;
        for (button, w) in [&iv.deny, &iv.always, &iv.allow]
            .into_iter()
            .zip(widths.iter().rev())
        {
            x -= w;
            button.setFrame(NSRect::new(
                NSPoint::new(x.max(margin), 14.0),
                NSSize::new(*w, button_h),
            ));
            x -= gap;
        }
        iv.label.setFrame(NSRect::new(
            NSPoint::new(margin, 14.0 + button_h + 12.0),
            NSSize::new((b.size.width - 2.0 * margin).max(0.0), 20.0),
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_cabecera_no_es_una_fila() {
        assert_eq!(row_at(0.0, 3), None);
        assert_eq!(row_at(HEADER_H - 0.1, 3), None);
    }

    #[test]
    fn localiza_cada_fila_y_el_hueco_entre_ellas() {
        assert_eq!(row_at(HEADER_H, 3), Some(0));
        assert_eq!(row_at(HEADER_H + ROW_H - 0.1, 3), Some(0));
        assert_eq!(
            row_at(HEADER_H + ROW_H + 1.0, 3),
            None,
            "el hueco no es de nadie"
        );
        assert_eq!(row_at(HEADER_H + ROW_H + ROW_GAP, 3), Some(1));
        assert_eq!(row_at(HEADER_H + 2.0 * (ROW_H + ROW_GAP) + 5.0, 3), Some(2));
    }

    #[test]
    fn por_debajo_de_la_ultima_no_hay_nada() {
        assert_eq!(row_at(HEADER_H + 3.0 * (ROW_H + ROW_GAP), 3), None);
        assert_eq!(row_at(10_000.0, 3), None);
        assert_eq!(row_at(HEADER_H, 0), None);
    }
}
