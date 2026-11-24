//! Vistas de la pantalla de Review (A3): lista de archivos y diff.
//!
//! Dibujadas a mano con los tokens `palette::dusk` (como la sidebar), para
//! poder volcarlas a PNG sin pantalla. **Cada vista rellena solo sus límites.**

use std::cell::{Cell, RefCell};

use objc2::rc::Retained;
use objc2::runtime::NSObjectProtocol;
use objc2::runtime::{AnyObject, Sel};
use objc2::{AnyThread, DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSAppearanceCustomization, NSButton, NSCompositingOperation, NSEvent, NSFont, NSLineBreakMode,
    NSRectFill, NSRectFillUsingOperation, NSResponder, NSScrollView, NSTextField, NSTrackingArea,
    NSTrackingAreaOptions, NSView,
};
use objc2_foundation::{MainThreadMarker, NSPoint, NSRect, NSSize, NSString};

use crate::agent::diff::{FileStatus, LineKind};
use crate::agent::review::{ROW_H, Row, Side, row_at};
use crate::agent_ui::{draw_text, draw_text_ex, fill_rounded, ns_color, stroke_rounded};
use crate::palette::tok;

// --- Lista de archivos ---

#[derive(Debug, Clone, PartialEq)]
pub struct FileRow {
    pub path: String,
    pub status: FileStatus,
    pub comments: usize,
    pub selected: bool,
}

const LIST_HEADER_H: f64 = 36.0;
const LIST_ROW_H: f64 = 30.0;

pub fn file_at(y: f64, count: usize) -> Option<usize> {
    if y < LIST_HEADER_H {
        return None;
    }
    let i = ((y - LIST_HEADER_H) / LIST_ROW_H) as usize;
    (i < count).then_some(i)
}

fn status_color(s: FileStatus) -> [u8; 3] {
    match s {
        FileStatus::Added => tok::ok(),
        FileStatus::Modified => tok::warn(),
        FileStatus::Deleted => tok::error(),
        FileStatus::Renamed => tok::accent(),
    }
}

type IndexHandler = Box<dyn Fn(usize)>;

pub struct ListIvars {
    rows: RefCell<Vec<FileRow>>,
    on_select: RefCell<Option<IndexHandler>>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTFileListView"]
    #[ivars = ListIvars]
    pub struct FileListView;

    unsafe impl NSObjectProtocol for FileListView {}

    impl FileListView {
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
            NSRectFill(NSRect::new(NSPoint::new(b.size.width - 1.0, 0.0), NSSize::new(1.0, b.size.height)));
            draw_text(
                "ARCHIVOS",
                NSRect::new(NSPoint::new(16.0, 12.0), NSSize::new(b.size.width - 32.0, 16.0)),
                11.0,
                0.6,
                tok::muted(),
            );
            for (i, row) in self.ivars().rows.borrow().iter().enumerate() {
                let y = LIST_HEADER_H + i as f64 * LIST_ROW_H;
                if row.selected {
                    let r = NSRect::new(NSPoint::new(6.0, y), NSSize::new(b.size.width - 13.0, LIST_ROW_H - 2.0));
                    fill_rounded(r, 6.0, tok::accent(), 0.07);
                    stroke_rounded(r, 6.0, tok::accent(), 0.5, 1.0);
                }
                // Etiqueta A/M/D/R: letra y color (nunca solo color).
                draw_text_ex(
                    row.status.tag(),
                    NSRect::new(NSPoint::new(14.0, y + 6.0), NSSize::new(16.0, 18.0)),
                    12.0,
                    0.6,
                    status_color(row.status),
                    true,
                    false,
                );
                let pill_w = if row.comments > 0 { 30.0 } else { 0.0 };
                draw_text(
                    &crate::agent_ui::shorten_head(&row.path, 30),
                    NSRect::new(
                        NSPoint::new(36.0, y + 6.0),
                        NSSize::new(b.size.width - 36.0 - 14.0 - pill_w, 18.0),
                    ),
                    12.0,
                    0.0,
                    tok::text(),
                );
                if row.comments > 0 {
                    let pill = NSRect::new(NSPoint::new(b.size.width - 40.0, y + 6.0), NSSize::new(26.0, 18.0));
                    fill_rounded(pill, 9.0, tok::attention(), 0.9);
                    draw_text_ex(
                        &row.comments.to_string(),
                        NSRect::new(
                            NSPoint::new(pill.origin.x, pill.origin.y + 1.0),
                            NSSize::new(pill.size.width - 9.0, pill.size.height),
                        ),
                        11.0,
                        0.7,
                        tok::bg(),
                        false,
                        true,
                    );
                }
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let count = self.ivars().rows.borrow().len();
            if let (Some(i), Some(f)) = (file_at(p.y, count), self.ivars().on_select.borrow().as_ref()) {
                f(i);
            }
        }
    }
);

impl FileListView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ListIvars {
            rows: RefCell::new(Vec::new()),
            on_select: RefCell::new(None),
        });
        // SAFETY: inicializador designado de NSView.
        unsafe { msg_send![super(this), init] }
    }

    pub fn set_on_select(&self, f: impl Fn(usize) + 'static) {
        *self.ivars().on_select.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_rows(&self, rows: Vec<FileRow>) {
        if *self.ivars().rows.borrow() != rows {
            *self.ivars().rows.borrow_mut() = rows;
            self.setNeedsDisplay(true);
        }
    }
}

// --- Diff ---

type SideHandler = Box<dyn Fn(Side, u32)>;

pub struct DiffIvars {
    rows: RefCell<Vec<Row>>,
    hover: Cell<Option<usize>>,
    on_add: RefCell<Option<SideHandler>>,
    on_delete: RefCell<Option<IndexHandler>>,
}

/// Columnas de la zona de números (puntos).
const X_PLUS: f64 = 6.0;
const X_OLD: f64 = 30.0;
const X_NEW: f64 = 74.0;
const X_SIGN: f64 = 122.0;
const X_TEXT: f64 = 138.0;
const NUM_W: f64 = 38.0;

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTDiffView"]
    #[ivars = DiffIvars]
    pub struct DiffView;

    unsafe impl NSObjectProtocol for DiffView {}

    impl DiffView {
        #[unsafe(method(isFlipped))]
        fn is_flipped(&self) -> bool {
            true
        }

        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(tok::bg(), 1.0).setFill();
            NSRectFill(b);
            let visible = self.visibleRect();
            let rows = self.ivars().rows.borrow();
            let first = (visible.origin.y / ROW_H).floor().max(0.0) as usize;
            let last = (((visible.origin.y + visible.size.height) / ROW_H).ceil() as usize).min(rows.len());
            let hover = self.ivars().hover.get();
            for i in first..last {
                let y = i as f64 * ROW_H;
                let line = NSRect::new(NSPoint::new(0.0, y), NSSize::new(b.size.width, ROW_H));
                draw_row(&rows[i], line, b.size.width, hover == Some(i));
            }
            if rows.is_empty() {
                draw_text(
                    "Sin cambios que revisar.",
                    NSRect::new(NSPoint::new(24.0, 24.0), NSSize::new(b.size.width - 48.0, 18.0)),
                    13.0,
                    0.0,
                    tok::muted(),
                );
            }
        }

        #[unsafe(method(mouseMoved:))]
        fn mouse_moved(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let count = self.ivars().rows.borrow().len();
            let now = row_at(p.y, count);
            if self.ivars().hover.replace(now) != now {
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseExited:))]
        fn mouse_exited(&self, _event: &NSEvent) {
            if self.ivars().hover.replace(None).is_some() {
                self.setNeedsDisplay(true);
            }
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            let p = self.convertPoint_fromView(event.locationInWindow(), None);
            let (action, width) = {
                let rows = self.ivars().rows.borrow();
                let Some(row) = row_at(p.y, rows.len()).map(|i| rows[i].clone()) else {
                    return;
                };
                (row, self.bounds().size.width)
            };
            match action {
                Row::Comment { index, .. } if p.x > width - 44.0 => {
                    if let Some(f) = self.ivars().on_delete.borrow().as_ref() {
                        f(index);
                    }
                }
                row => {
                    if p.x < X_OLD
                        && let Some((side, n)) = row.commentable()
                        && let Some(f) = self.ivars().on_add.borrow().as_ref()
                    {
                        f(side, n);
                    }
                }
            }
        }
    }
);

fn draw_row(row: &Row, line: NSRect, width: f64, hovered: bool) {
    let y = line.origin.y;
    let text_rect = |x: f64| {
        NSRect::new(
            NSPoint::new(x, y + 2.0),
            NSSize::new((width - x - 8.0).max(0.0), ROW_H - 2.0),
        )
    };
    match row {
        Row::Hunk(header) => {
            ns_color(tok::surface(), 1.0).setFill();
            NSRectFill(line);
            draw_text_ex(
                header,
                text_rect(X_OLD),
                11.0,
                0.0,
                tok::muted(),
                true,
                false,
            );
        }
        Row::Line {
            kind,
            old,
            new,
            text,
        } => {
            let (tint, sign_color) = match kind {
                LineKind::Add => (Some(tok::ok()), tok::ok()),
                LineKind::Del => (Some(tok::error()), tok::error()),
                LineKind::Context => (None, tok::muted()),
            };
            if let Some(c) = tint {
                ns_color(c, 0.12).setFill();
                NSRectFillUsingOperation(line, NSCompositingOperation::SourceOver);
            }
            let num = |n: &Option<u32>| n.map(|n| n.to_string()).unwrap_or_default();
            let num_rect =
                |x: f64| NSRect::new(NSPoint::new(x, y + 2.0), NSSize::new(NUM_W, ROW_H - 2.0));
            draw_text_ex(
                &num(old),
                num_rect(X_OLD),
                11.0,
                0.0,
                tok::ghost(),
                true,
                true,
            );
            draw_text_ex(
                &num(new),
                num_rect(X_NEW),
                11.0,
                0.0,
                tok::ghost(),
                true,
                true,
            );
            let sign = match kind {
                LineKind::Add => "+",
                LineKind::Del => "−",
                LineKind::Context => " ",
            };
            draw_text_ex(sign, text_rect(X_SIGN), 12.0, 0.6, sign_color, true, false);
            draw_text_ex(text, text_rect(X_TEXT), 12.0, 0.0, tok::text(), true, false);
            if hovered {
                let plus = NSRect::new(
                    NSPoint::new(X_PLUS, y + 2.0),
                    NSSize::new(18.0, ROW_H - 4.0),
                );
                fill_rounded(plus, 4.0, tok::accent(), 0.9);
                draw_text_ex(
                    "+",
                    NSRect::new(NSPoint::new(plus.origin.x, plus.origin.y), plus.size),
                    12.0,
                    0.8,
                    tok::bg(),
                    true,
                    true,
                );
            }
        }
        Row::Comment { text, first, .. } => {
            // `NSRectFill` copia en vez de mezclar: con colores translúcidos, SourceOver.
            ns_color(tok::attention(), 0.10).setFill();
            NSRectFillUsingOperation(line, NSCompositingOperation::SourceOver);
            ns_color(tok::attention(), 0.9).setFill();
            NSRectFillUsingOperation(
                NSRect::new(NSPoint::new(X_OLD, y), NSSize::new(3.0, ROW_H)),
                NSCompositingOperation::SourceOver,
            );
            draw_text(text, text_rect(X_NEW), 12.0, 0.0, tok::text());
            if *first {
                draw_text_ex(
                    "×",
                    NSRect::new(
                        NSPoint::new(width - 34.0, y + 1.0),
                        NSSize::new(20.0, ROW_H),
                    ),
                    14.0,
                    0.4,
                    tok::muted(),
                    false,
                    true,
                );
            }
        }
    }
}

impl DiffView {
    pub fn new(mtm: MainThreadMarker) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(DiffIvars {
            rows: RefCell::new(Vec::new()),
            hover: Cell::new(None),
            on_add: RefCell::new(None),
            on_delete: RefCell::new(None),
        });
        // SAFETY: inicializador designado de NSView.
        let view: Retained<Self> = unsafe { msg_send![super(this), init] };
        // SAFETY: el propietario es esta vista, que vive más que el área.
        let area = unsafe {
            NSTrackingArea::initWithRect_options_owner_userInfo(
                NSTrackingArea::alloc(),
                NSRect::ZERO,
                NSTrackingAreaOptions::MouseMoved
                    | NSTrackingAreaOptions::MouseEnteredAndExited
                    | NSTrackingAreaOptions::ActiveInKeyWindow
                    | NSTrackingAreaOptions::InVisibleRect,
                Some(&view),
                None,
            )
        };
        view.addTrackingArea(&area);
        view
    }

    pub fn set_on_add(&self, f: impl Fn(Side, u32) + 'static) {
        *self.ivars().on_add.borrow_mut() = Some(Box::new(f));
    }

    pub fn set_on_delete(&self, f: impl Fn(usize) + 'static) {
        *self.ivars().on_delete.borrow_mut() = Some(Box::new(f));
    }

    /// Cambia las filas y ajusta el alto del documento.
    pub fn set_rows(&self, rows: Vec<Row>, width: f64) {
        let height = (rows.len() as f64 * ROW_H).max(60.0);
        *self.ivars().rows.borrow_mut() = rows;
        self.ivars().hover.set(None);
        self.setFrameSize(NSSize::new(width, height));
        self.setNeedsDisplay(true);
    }
}

// --- Ventana de Review ---

pub const HEADER_H: f64 = 68.0;
pub const LIST_W: f64 = 264.0;

/// Los selectores que reciben los botones de la cabecera.
#[derive(Clone, Copy)]
pub struct ReviewActions {
    pub refresh: Sel,
    pub send: Sel,
    pub pr: Sel,
    pub discard: Sel,
}

pub struct RootIvars {
    title: Retained<NSTextField>,
    subtitle: Retained<NSTextField>,
    refresh: Retained<NSButton>,
    send: Retained<NSButton>,
    pr: Retained<NSButton>,
    discard: Retained<NSButton>,
    pub list: Retained<FileListView>,
    pub scroll: Retained<NSScrollView>,
    pub diff: Retained<DiffView>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y no tiene `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTReviewRoot"]
    #[ivars = RootIvars]
    pub struct ReviewRoot;

    unsafe impl NSObjectProtocol for ReviewRoot {}

    impl ReviewRoot {
        #[unsafe(method(drawRect:))]
        fn draw_rect(&self, _rect: NSRect) {
            let b = self.bounds();
            ns_color(tok::bg(), 1.0).setFill();
            NSRectFill(b);
            ns_color(tok::chrome(), 1.0).setFill();
            NSRectFill(NSRect::new(
                NSPoint::new(0.0, b.size.height - HEADER_H),
                NSSize::new(b.size.width, HEADER_H),
            ));
            ns_color(tok::line(), 1.0).setFill();
            NSRectFill(NSRect::new(
                NSPoint::new(0.0, b.size.height - HEADER_H),
                NSSize::new(b.size.width, 1.0),
            ));
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: llamada a la implementación de NSView.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.relayout();
        }
    }
);

fn small_label(mtm: MainThreadMarker, size: f64, color: [u8; 3]) -> Retained<NSTextField> {
    let l = NSTextField::labelWithString(&NSString::from_str(""), mtm);
    l.setFont(Some(&NSFont::systemFontOfSize(size)));
    l.setTextColor(Some(&ns_color(color, 1.0)));
    l.setLineBreakMode(NSLineBreakMode::ByTruncatingTail);
    l
}

impl ReviewRoot {
    /// `target` recibe las acciones de los botones.
    pub fn new(
        mtm: MainThreadMarker,
        target: &AnyObject,
        actions: ReviewActions,
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
        let diff = DiffView::new(mtm);
        let scroll = NSScrollView::initWithFrame(NSScrollView::alloc(mtm), NSRect::ZERO);
        scroll.setHasVerticalScroller(true);
        scroll.setDrawsBackground(false);
        scroll.setDocumentView(Some(&diff));
        let ivars = RootIvars {
            title: small_label(mtm, 15.0, tok::text()),
            subtitle: small_label(mtm, 12.0, tok::muted()),
            refresh: button("Actualizar", actions.refresh),
            send: button("Enviar comentarios a Claude", actions.send),
            pr: button("Abrir PR\u{2026}", actions.pr),
            discard: button("Descartar worktree\u{2026}", actions.discard),
            list: FileListView::new(mtm),
            scroll,
            diff,
        };
        ivars
            .title
            .setFont(Some(&NSFont::systemFontOfSize_weight(15.0, 0.6)));
        let this = Self::alloc(mtm).set_ivars(ivars);
        // SAFETY: inicializador designado de NSView.
        let this: Retained<Self> = unsafe { msg_send![super(this), initWithFrame: NSRect::ZERO] };
        this.setAppearance(crate::palette::ui_appearance().as_deref());
        let iv = this.ivars();
        for v in [
            &*iv.title as &NSView,
            &iv.subtitle,
            &iv.refresh,
            &iv.send,
            &iv.pr,
            &iv.discard,
            &*iv.list,
            &*iv.scroll,
        ] {
            this.addSubview(v);
        }
        this
    }

    pub fn ivars_pub(&self) -> &RootIvars {
        self.ivars()
    }

    pub fn set_header(&self, title: &str, subtitle: &str, send_title: &str, send_enabled: bool) {
        let iv = self.ivars();
        iv.title.setStringValue(&NSString::from_str(title));
        iv.subtitle.setStringValue(&NSString::from_str(subtitle));
        iv.title.setTextColor(Some(&ns_color(tok::text(), 1.0)));
        iv.subtitle.setTextColor(Some(&ns_color(tok::muted(), 1.0)));
        iv.send.setTitle(&NSString::from_str(send_title));
        iv.send.setEnabled(send_enabled);
        self.relayout();
    }

    /// Ancho disponible para el diff.
    pub fn diff_width(&self) -> f64 {
        (self.bounds().size.width - LIST_W).max(200.0)
    }

    fn relayout(&self) {
        let b = self.bounds();
        let iv = self.ivars();
        let (w, h) = (b.size.width, b.size.height);
        let body_h = (h - HEADER_H).max(0.0);
        iv.list.setFrame(NSRect::new(
            NSPoint::new(0.0, 0.0),
            NSSize::new(LIST_W, body_h),
        ));
        iv.scroll.setFrame(NSRect::new(
            NSPoint::new(LIST_W, 0.0),
            NSSize::new((w - LIST_W).max(0.0), body_h),
        ));
        let (btn_h, margin, gap) = (28.0, 16.0, 8.0);
        let send_w = 240.0;
        let refresh_w = 100.0;
        let y = h - HEADER_H + (HEADER_H - btn_h) / 2.0;
        iv.send.setFrame(NSRect::new(
            NSPoint::new(w - margin - send_w, y),
            NSSize::new(send_w, btn_h),
        ));
        iv.refresh.setFrame(NSRect::new(
            NSPoint::new(w - margin - send_w - gap - refresh_w, y),
            NSSize::new(refresh_w, btn_h),
        ));
        let (pr_w, discard_w) = (110.0, 170.0);
        let mut x = w - margin - send_w - gap - refresh_w;
        iv.pr.setFrame(NSRect::new(
            NSPoint::new(x - gap - pr_w, y),
            NSSize::new(pr_w, btn_h),
        ));
        x -= gap + pr_w;
        iv.discard.setFrame(NSRect::new(
            NSPoint::new(x - gap - discard_w, y),
            NSSize::new(discard_w, btn_h),
        ));
        x -= gap + discard_w;
        let text_w = (x - margin - 16.0).max(0.0);
        iv.title.setFrame(NSRect::new(
            NSPoint::new(margin, h - 30.0),
            NSSize::new(text_w, 20.0),
        ));
        iv.subtitle.setFrame(NSRect::new(
            NSPoint::new(margin, h - 52.0),
            NSSize::new(text_w, 18.0),
        ));
        // El documento del diff sigue el ancho disponible.
        let doc_w = self.diff_width();
        let rows_h = iv.diff.frame().size.height;
        iv.diff.setFrameSize(NSSize::new(doc_w, rows_h));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn la_cabecera_de_la_lista_no_es_un_archivo() {
        assert_eq!(file_at(0.0, 3), None);
        assert_eq!(file_at(LIST_HEADER_H, 3), Some(0));
        assert_eq!(file_at(LIST_HEADER_H + LIST_ROW_H * 2.0 + 1.0, 3), Some(2));
        assert_eq!(file_at(LIST_HEADER_H + LIST_ROW_H * 3.0, 3), None);
        assert_eq!(file_at(100.0, 0), None);
    }

    /// Las columnas de números no se solapan (comprobado al compilar).
    const _: () = {
        assert!(X_PLUS + 18.0 <= X_OLD);
        assert!(X_OLD + NUM_W <= X_NEW);
        assert!(X_NEW + NUM_W <= X_SIGN);
        assert!(X_SIGN + 12.0 <= X_TEXT);
    };
}
