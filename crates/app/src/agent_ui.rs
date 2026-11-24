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
    NSAppearanceCustomization, NSBezierPath, NSButton, NSColor, NSCompositingOperation, NSEvent,
    NSFont, NSFontAttributeName, NSForegroundColorAttributeName, NSLineBreakMode,
    NSMutableParagraphStyle, NSParagraphStyleAttributeName, NSRectFill, NSRectFillUsingOperation,
    NSResponder, NSStringDrawing, NSTextField, NSView,
};
use objc2_foundation::{MainThreadMarker, NSDictionary, NSPoint, NSRect, NSSize, NSString};

use crate::palette::tok;

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
    draw_text_ex(text, rect, size, weight, color, false, false);
}

/// Como [`draw_text`], con fuente monoespaciada y alineación a la derecha opcionales.
pub fn draw_text_ex(
    text: &str,
    rect: NSRect,
    size: f64,
    weight: f64,
    color: [u8; 3],
    mono: bool,
    right: bool,
) {
    let font = if mono {
        NSFont::monospacedSystemFontOfSize_weight(size, weight)
    } else {
        NSFont::systemFontOfSize_weight(size, weight)
    };
    let style = NSMutableParagraphStyle::new();
    style.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    if right {
        style.setAlignment(objc2_app_kit::NSTextAlignment::Right);
    }
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

pub fn fill_rounded(rect: NSRect, radius: f64, color: [u8; 3], alpha: f64) {
    ns_color(color, alpha).setFill();
    NSBezierPath::bezierPathWithRoundedRect_xRadius_yRadius(rect, radius, radius).fill();
}

pub fn stroke_rounded(rect: NSRect, radius: f64, color: [u8; 3], alpha: f64, width: f64) {
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
            ns_color(tok::sidebar(), 1.0).setFill();
            NSRectFill(b);
            // Borde derecho.
            ns_color(tok::line(), 1.0).setFill();
            NSRectFill(NSRect::new(
                NSPoint::new(b.size.width - 1.0, 0.0),
                NSSize::new(1.0, b.size.height),
            ));
            draw_text(
                "AGENTES",
                NSRect::new(NSPoint::new(16.0, 14.0), NSSize::new(b.size.width - 32.0, 16.0)),
                11.0,
                0.6,
                tok::muted(),
            );
            for (i, row) in self.ivars().rows.borrow().iter().enumerate() {
                let y = HEADER_H + i as f64 * (ROW_H + ROW_GAP);
                let r = NSRect::new(
                    NSPoint::new(8.0, y),
                    NSSize::new(b.size.width - 17.0, ROW_H),
                );
                if row.selected {
                    fill_rounded(r, 10.0, tok::accent(), 0.07);
                    stroke_rounded(r, 10.0, tok::accent(), 0.5, 1.0);
                }
                if row.ring {
                    stroke_rounded(r, 10.0, tok::attention(), 0.9, 1.5);
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
                    tok::text(),
                );
                draw_text(
                    &row.detail,
                    NSRect::new(NSPoint::new(text_x, y + 31.0), NSSize::new(text_w, 16.0)),
                    11.0,
                    0.0,
                    if row.ring { tok::attention() } else { tok::muted() },
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
            ns_color(tok::bg(), 1.0).setFill();
            NSRectFill(b);
            // attention al 8 % y borde superior al 45 %. `NSRectFill` copia en
            // vez de mezclar: con colores translúcidos hay que usar SourceOver.
            ns_color(tok::attention(), 0.08).setFill();
            NSRectFillUsingOperation(b, NSCompositingOperation::SourceOver);
            ns_color(tok::attention(), 0.45).setFill();
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
        label.setTextColor(Some(&ns_color(tok::text(), 1.0)));
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
        this.setAppearance(crate::palette::ui_appearance().as_deref());
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
        self.ivars()
            .label
            .setTextColor(Some(&ns_color(tok::text(), 1.0)));
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

// --- Barra de terminado ---

pub struct DoneIvars {
    label: Retained<NSTextField>,
    review: Retained<NSButton>,
    pr: Retained<NSButton>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTDoneBar"]
    #[ivars = DoneIvars]
    pub struct DoneBar;

    unsafe impl NSObjectProtocol for DoneBar {}

    impl DoneBar {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(tok::bg(), 1.0).setFill();
            NSRectFill(b);
            ns_color(tok::ok(), 0.08).setFill();
            NSRectFillUsingOperation(b, NSCompositingOperation::SourceOver);
            ns_color(tok::ok(), 0.45).setFill();
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

pub const DONE_BAR_HEIGHT: f64 = 52.0;

impl DoneBar {
    pub fn new(mtm: MainThreadMarker, target: &AnyObject, review: Sel, pr: Sel) -> Retained<Self> {
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
        let button = make("Revisar cambios  \u{2318}\u{21e7}R", review);
        let pr_button = make("Abrir PR\u{2026}", pr);
        let label = NSTextField::labelWithString(&NSString::from_str(""), mtm);
        label.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
        label.setTextColor(Some(&ns_color(tok::text(), 1.0)));
        label.setFont(Some(&NSFont::systemFontOfSize(13.0)));
        let this = Self::alloc(mtm).set_ivars(DoneIvars {
            label,
            review: button,
            pr: pr_button,
        });
        // SAFETY: inicializador designado de NSView.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setAppearance(crate::palette::ui_appearance().as_deref());
        this.addSubview(&this.ivars().label);
        this.addSubview(&this.ivars().review);
        this.addSubview(&this.ivars().pr);
        this
    }

    pub fn set_summary(&self, text: &str) {
        self.ivars().label.setStringValue(&NSString::from_str(text));
        self.ivars()
            .label
            .setTextColor(Some(&ns_color(tok::text(), 1.0)));
        self.relayout();
    }

    fn relayout(&self) {
        let b = self.bounds();
        let iv = self.ivars();
        let (h, margin, w, pr_w, gap) = (28.0, 16.0, 190.0, 110.0, 8.0);
        let y = (b.size.height - h) / 2.0;
        iv.review.setFrame(NSRect::new(
            NSPoint::new((b.size.width - margin - w).max(margin), y),
            NSSize::new(w, h),
        ));
        iv.pr.setFrame(NSRect::new(
            NSPoint::new((b.size.width - margin - w - gap - pr_w).max(margin), y),
            NSSize::new(pr_w, h),
        ));
        iv.label.setFrame(NSRect::new(
            NSPoint::new(margin, (b.size.height - 20.0) / 2.0),
            NSSize::new(
                (b.size.width - margin * 3.0 - w - gap - pr_w).max(0.0),
                20.0,
            ),
        ));
    }
}

// --- Inspector ---

pub const INSPECTOR_WIDTH: f64 = 296.0;
/// Ancho mínimo de la columna central (la terminal) para que el inspector quepa
/// (spec: «Claude Code pane … min 560 px»).
pub const MIN_CENTER_FOR_INSPECTOR: f64 = 560.0;

/// ¿Cabe el inspector junto a la sidebar sin dejar la terminal por debajo de su mínimo?
pub fn inspector_fits(window_width: f64, sidebar_shown: bool) -> bool {
    let sidebar = if sidebar_shown { SIDEBAR_WIDTH } else { 0.0 };
    window_width - sidebar - INSPECTOR_WIDTH >= MIN_CENTER_FOR_INSPECTOR
}

/// Recorta por el principio, conservando el final (el nombre del archivo):
/// `crates/app/src/review_ui.rs` → `…/src/review_ui.rs`.
pub fn shorten_head(text: &str, max: usize) -> String {
    let n = text.chars().count();
    if n <= max || max < 2 {
        return text.to_owned();
    }
    let tail: String = text.chars().skip(n - (max - 1)).collect();
    format!("…{tail}")
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct ChangeRow {
    pub tag: &'static str,
    pub path: String,
    pub added: usize,
    pub removed: usize,
}

#[derive(Debug, Clone, PartialEq, Default)]
pub struct InspectorData {
    pub todos: Vec<crate::agent::model::Todo>,
    /// `None`: aún no se ha calculado.
    pub changes: Option<Vec<ChangeRow>>,
    pub context_percent: Option<f64>,
    pub worktree: String,
    pub branch: Option<String>,
}

/// Por encima de este porcentaje el contexto se pinta en ámbar y se avisa.
pub const CONTEXT_WARN: f64 = 70.0;

pub struct InspectorIvars {
    data: RefCell<InspectorData>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTInspectorView"]
    #[ivars = InspectorIvars]
    pub struct InspectorView;

    unsafe impl NSObjectProtocol for InspectorView {}

    impl InspectorView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(tok::sidebar(), 1.0).setFill();
            NSRectFill(b);
            ns_color(tok::line(), 1.0).setFill();
            NSRectFill(NSRect::new(NSPoint::new(0.0, 0.0), NSSize::new(1.0, b.size.height)));
            let d = self.ivars().data.borrow();
            let w = b.size.width - 32.0;
            let mut y = 16.0;
            let rect = |y: f64, h: f64| NSRect::new(NSPoint::new(16.0, y), NSSize::new(w, h));

            let heading = |text: &str, y: &mut f64| {
                draw_text(text, rect(*y, 16.0), 11.0, 0.6, tok::muted());
                *y += 22.0;
            };

            heading("PLAN", &mut y);
            if d.todos.is_empty() {
                draw_text("Sin plan todavía.", rect(y, 18.0), 12.0, 0.0, tok::muted());
                y += 24.0;
            }
            for todo in d.todos.iter().take(12) {
                let color = match todo.status {
                    crate::agent::model::TodoStatus::Done => tok::ok(),
                    crate::agent::model::TodoStatus::InProgress => tok::accent(),
                    crate::agent::model::TodoStatus::Pending => tok::muted(),
                };
                draw_text(todo.status.icon(), NSRect::new(NSPoint::new(16.0, y), NSSize::new(16.0, 18.0)), 12.0, 0.5, color);
                draw_text(
                    &todo.content,
                    NSRect::new(NSPoint::new(36.0, y), NSSize::new(w - 20.0, 18.0)),
                    12.0,
                    0.0,
                    tok::text(),
                );
                y += 22.0;
            }
            y += 14.0;

            heading("CAMBIOS", &mut y);
            match &d.changes {
                None => {
                    draw_text("Calculando…", rect(y, 18.0), 12.0, 0.0, tok::muted());
                    y += 24.0;
                }
                Some(rows) if rows.is_empty() => {
                    draw_text("Sin cambios.", rect(y, 18.0), 12.0, 0.0, tok::muted());
                    y += 24.0;
                }
                Some(rows) => {
                    for row in rows.iter().take(10) {
                        let color = match row.tag {
                            "A" => tok::ok(),
                            "D" => tok::error(),
                            "R" => tok::accent(),
                            _ => tok::warn(),
                        };
                        draw_text_ex(row.tag, NSRect::new(NSPoint::new(16.0, y), NSSize::new(14.0, 18.0)), 12.0, 0.6, color, true, false);
                        draw_text(
                            &shorten_head(&row.path, 22),
                            NSRect::new(NSPoint::new(34.0, y), NSSize::new(w - 34.0 - 78.0, 18.0)),
                            12.0,
                            0.0,
                            tok::text(),
                        );
                        draw_text_ex(
                            &format!("+{} −{}", row.added, row.removed),
                            NSRect::new(NSPoint::new(16.0 + w - 76.0, y), NSSize::new(76.0, 18.0)),
                            11.0,
                            0.0,
                            tok::muted(),
                            true,
                            true,
                        );
                        y += 22.0;
                    }
                    if rows.len() > 10 {
                        draw_text(&format!("… y {} más", rows.len() - 10), rect(y, 16.0), 11.0, 0.0, tok::muted());
                        y += 22.0;
                    }
                }
            }
            y += 14.0;

            heading("CONTEXTO", &mut y);
            match d.context_percent {
                None => {
                    draw_text("Sin datos todavía.", rect(y, 18.0), 12.0, 0.0, tok::muted());
                    y += 24.0;
                }
                Some(p) => {
                    let warn = p > CONTEXT_WARN;
                    let color = if warn { tok::attention() } else { tok::accent() };
                    let track = NSRect::new(NSPoint::new(16.0, y + 2.0), NSSize::new(w, 6.0));
                    fill_rounded(track, 3.0, tok::line_strong(), 1.0);
                    let filled = NSRect::new(track.origin, NSSize::new(w * (p.clamp(0.0, 100.0) / 100.0), 6.0));
                    fill_rounded(filled, 3.0, color, 1.0);
                    y += 16.0;
                    draw_text(&format!("{p:.0} % usado"), rect(y, 16.0), 12.0, 0.0, if warn { tok::attention() } else { tok::text() });
                    y += 20.0;
                    if warn {
                        draw_text("Conviene compactar la conversación.", rect(y, 16.0), 11.0, 0.0, tok::attention());
                        y += 20.0;
                    }
                }
            }
            y += 14.0;

            heading("WORKTREE", &mut y);
            draw_text(&d.worktree, rect(y, 16.0), 11.0, 0.0, tok::muted());
            y += 20.0;
            if let Some(branch) = &d.branch {
                draw_text(&format!("Rama: {branch}"), rect(y, 16.0), 12.0, 0.0, tok::text());
            }
        }
    }
);

impl InspectorView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(InspectorIvars {
            data: RefCell::new(InspectorData::default()),
        });
        // SAFETY: inicializador designado de NSView.
        unsafe { msg_send![super(this), init] }
    }

    pub fn set_data(&self, data: InspectorData) {
        if *self.ivars().data.borrow() != data {
            *self.ivars().data.borrow_mut() = data;
            self.setNeedsDisplay(true);
        }
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

    #[test]
    fn recortar_por_el_principio_conserva_el_nombre() {
        assert_eq!(shorten_head("README.md", 22), "README.md");
        assert_eq!(
            shorten_head("crates/app/src/review_ui.rs", 22),
            "…/app/src/review_ui.rs"
        );
        assert_eq!(
            shorten_head("crates/app/src/review_ui.rs", 22)
                .chars()
                .count(),
            22
        );
        assert_eq!(shorten_head("ñandú/ÁÉÍ/archivo.txt", 12), "…archivo.txt");
        assert_eq!(
            shorten_head("abc", 1),
            "abc",
            "un máximo absurdo no recorta"
        );
    }

    #[test]
    fn el_inspector_solo_cabe_si_la_terminal_conserva_su_minimo() {
        // Ventana de 812 pt: ni con sidebar ni sin ella.
        assert!(!inspector_fits(812.0, true));
        assert!(!inspector_fits(812.0, false));
        // Sin sidebar: 296 + 560.
        assert!(inspector_fits(856.0, false));
        assert!(!inspector_fits(855.0, false));
        // Con sidebar: 264 + 296 + 560.
        assert!(inspector_fits(1120.0, true));
        assert!(!inspector_fits(1119.0, true));
    }
}
