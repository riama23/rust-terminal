//! La vista de la terminal: entrada de teclado (con `NSTextInputClient` para
//! teclas muertas y Option), pegado, ratón (selección y scroll), copiar,
//! navegación por bloques y cambios de tamaño o de escala.

use std::cell::{Cell, OnceCell, RefCell};
use std::sync::Arc;

use objc2::rc::Retained;
use objc2::runtime::{AnyObject, NSObjectProtocol, Sel};
use objc2::{DefinedClass, MainThreadOnly, define_class, msg_send};
use objc2_app_kit::{
    NSEvent, NSEventModifierFlags, NSPasteboard, NSPasteboardTypeString, NSResponder,
    NSTextInputClient, NSView,
};
use objc2_core_foundation::CGSize;
use objc2_foundation::{
    MainThreadMarker, NSArray, NSAttributedString, NSAttributedStringKey, NSNotFound, NSPoint,
    NSRange, NSRect, NSSize, NSString, NSUInteger,
};

use rust_terminal_core::selection;
use rust_terminal_core::{Selection, SelectionKind};

use crate::font::{self, Metrics};
use crate::input::{self, Mods};
use crate::renderer::Layout;
use crate::session::{Shared, Waker};

/// Margen interior alrededor de la cuadrícula, en puntos.
pub const PAD_PT: f64 = 6.0;

pub struct ViewIvars {
    shared: Arc<Shared>,
    waker: OnceCell<Arc<Waker>>,
    /// Texto en composición (tecla muerta o método de entrada).
    marked: RefCell<String>,
    /// Métricas para la escala actual, para no recalcularlas en cada resize.
    metrics: Cell<Option<(f64, Metrics)>>,
    /// Desplazamiento del trackpad acumulado que aún no llega a una línea.
    scroll_accum: Cell<f64>,
    /// El ratón se movió desde `mouseDown` (si no, un clic simple deselecciona).
    drag_moved: Cell<bool>,
}

define_class!(
    // SAFETY: NSView no impone requisitos a sus subclases y `TerminalView`
    // no implementa `Drop`.
    #[unsafe(super(NSView, NSResponder, objc2_foundation::NSObject))]
    #[thread_kind = MainThreadOnly]
    #[name = "RTTerminalView"]
    #[ivars = ViewIvars]
    pub struct TerminalView;

    unsafe impl NSObjectProtocol for TerminalView {}

    impl TerminalView {
        #[unsafe(method(acceptsFirstResponder))]
        fn accepts_first_responder(&self) -> bool {
            true
        }

        #[unsafe(method(keyDown:))]
        fn key_down(&self, event: &NSEvent) {
            self.handle_key(event);
        }

        #[unsafe(method(setFrameSize:))]
        fn set_frame_size(&self, size: NSSize) {
            // SAFETY: llamada a la implementación de NSView.
            let _: () = unsafe { msg_send![super(self), setFrameSize: size] };
            self.update_layout();
        }

        #[unsafe(method(viewDidChangeBackingProperties))]
        fn did_change_backing(&self) {
            self.update_layout();
        }

        #[unsafe(method(paste:))]
        fn paste(&self, _sender: Option<&AnyObject>) {
            // SAFETY: lectura del portapapeles general en el hilo principal.
            let text = unsafe {
                NSPasteboard::generalPasteboard().stringForType(NSPasteboardTypeString)
            };
            if let Some(text) = text {
                let bracketed = self.ivars().shared.term.lock().unwrap().modes().bracketed_paste;
                self.send_input(&input::encode_paste(&text.to_string(), bracketed));
            }
        }

        #[unsafe(method(copy:))]
        fn copy(&self, _sender: Option<&AnyObject>) {
            let text = self.ivars().shared.term.lock().unwrap().selection_text();
            if let Some(text) = text.filter(|t| !t.is_empty()) {
                copy_to_pasteboard(&text);
            }
        }

        /// Copia la salida del último comando terminado (bloques OSC 133).
        #[unsafe(method(copyLastOutput:))]
        fn copy_last_output(&self, _sender: Option<&AnyObject>) {
            let text = {
                let term = self.ivars().shared.term.lock().unwrap();
                term.last_finished_block().and_then(|b| term.block_output(b))
            };
            if let Some(text) = text {
                copy_to_pasteboard(&text);
            }
        }

        #[unsafe(method(previousBlock:))]
        fn previous_block(&self, _sender: Option<&AnyObject>) {
            {
                let mut term = self.ivars().shared.term.lock().unwrap();
                let top = term.visible_line(0);
                let target = term.blocks().iter().rev().map(|b| b.prompt.line).find(|&l| l < top);
                if let Some(line) = target {
                    term.scroll_to_line(line);
                }
            }
            self.request_frame();
        }

        #[unsafe(method(nextBlock:))]
        fn next_block(&self, _sender: Option<&AnyObject>) {
            {
                let mut term = self.ivars().shared.term.lock().unwrap();
                let top = term.visible_line(0);
                let target = term.blocks().iter().map(|b| b.prompt.line).find(|&l| l > top);
                match target {
                    Some(line) => term.scroll_to_line(line),
                    None => term.scroll_to_bottom(),
                }
            }
            self.request_frame();
        }

        #[unsafe(method(scrollWheel:))]
        fn scroll_wheel(&self, event: &NSEvent) {
            self.handle_scroll(event);
        }

        #[unsafe(method(mouseDown:))]
        fn mouse_down(&self, event: &NSEvent) {
            self.handle_mouse_down(event);
        }

        #[unsafe(method(mouseDragged:))]
        fn mouse_dragged(&self, event: &NSEvent) {
            self.handle_mouse_dragged(event);
        }

        #[unsafe(method(mouseUp:))]
        fn mouse_up(&self, event: &NSEvent) {
            if !self.ivars().drag_moved.get() {
                self.ivars().shared.term.lock().unwrap().set_selection(None);
                self.request_frame();
            }
        }
    }

    unsafe impl NSTextInputClient for TerminalView {
        #[unsafe(method(insertText:replacementRange:))]
        fn insert_text(&self, string: &AnyObject, _range: NSRange) {
            self.ivars().marked.borrow_mut().clear();
            let text = text_of(string);
            if !text.is_empty() {
                self.send_input(text.as_bytes());
            }
        }

        #[unsafe(method(doCommandBySelector:))]
        fn do_command_by_selector(&self, _selector: Sel) {
            // Las teclas con comando (Enter, flechas…) se codifican antes, en `handle_key`.
        }

        #[unsafe(method(setMarkedText:selectedRange:replacementRange:))]
        fn set_marked_text(&self, string: &AnyObject, _selected: NSRange, _replacement: NSRange) {
            *self.ivars().marked.borrow_mut() = text_of(string);
        }

        #[unsafe(method(unmarkText))]
        fn unmark_text(&self) {
            self.ivars().marked.borrow_mut().clear();
        }

        #[unsafe(method(selectedRange))]
        fn selected_range(&self) -> NSRange {
            NSRange::new(NSNotFound as NSUInteger, 0)
        }

        #[unsafe(method(markedRange))]
        fn marked_range(&self) -> NSRange {
            let len = self.ivars().marked.borrow().encode_utf16().count();
            if len == 0 {
                NSRange::new(NSNotFound as NSUInteger, 0)
            } else {
                NSRange::new(0, len)
            }
        }

        #[unsafe(method(hasMarkedText))]
        fn has_marked_text(&self) -> bool {
            !self.ivars().marked.borrow().is_empty()
        }

        #[unsafe(method_id(attributedSubstringForProposedRange:actualRange:))]
        fn attributed_substring(
            &self,
            _range: NSRange,
            _actual: *mut NSRange,
        ) -> Option<Retained<NSAttributedString>> {
            None
        }

        #[unsafe(method_id(validAttributesForMarkedText))]
        fn valid_attributes(&self) -> Retained<NSArray<NSAttributedStringKey>> {
            NSArray::new()
        }

        #[unsafe(method(firstRectForCharacterRange:actualRange:))]
        fn first_rect(&self, _range: NSRange, _actual: *mut NSRange) -> NSRect {
            self.cursor_screen_rect()
        }

        #[unsafe(method(characterIndexForPoint:))]
        fn character_index(&self, _point: NSPoint) -> NSUInteger {
            NSNotFound as NSUInteger
        }
    }
);

fn copy_to_pasteboard(text: &str) {
    // SAFETY: portapapeles general, en el hilo principal.
    unsafe {
        let pb = NSPasteboard::generalPasteboard();
        pb.clearContents();
        pb.setString_forType(&NSString::from_str(text), NSPasteboardTypeString);
    }
}

/// Texto de un `NSString` o `NSAttributedString` recibido del sistema de entrada.
fn text_of(obj: &AnyObject) -> String {
    if let Some(s) = obj.downcast_ref::<NSString>() {
        s.to_string()
    } else if let Some(s) = obj.downcast_ref::<NSAttributedString>() {
        s.string().to_string()
    } else {
        String::new()
    }
}

impl TerminalView {
    pub fn new(mtm: MainThreadMarker, frame: NSRect, shared: Arc<Shared>) -> Retained<Self> {
        let this = Self::alloc(mtm).set_ivars(ViewIvars {
            shared,
            waker: OnceCell::new(),
            marked: RefCell::new(String::new()),
            metrics: Cell::new(None),
            scroll_accum: Cell::new(0.0),
            drag_moved: Cell::new(false),
        });
        // SAFETY: `initWithFrame:` de NSView.
        unsafe { msg_send![super(this), initWithFrame: frame] }
    }

    pub fn set_waker(&self, waker: Arc<Waker>) {
        let _ = self.ivars().waker.set(waker);
    }

    fn request_frame(&self) {
        if let Some(waker) = self.ivars().waker.get() {
            waker.request_frame();
        }
    }

    fn metrics_for(&self, scale: f64) -> Metrics {
        match self.ivars().metrics.get() {
            Some((s, m)) if s == scale => m,
            _ => {
                let m = Metrics::of(&font::system_monospace(font::FONT_SIZE_PT * scale));
                self.ivars().metrics.set(Some((scale, m)));
                m
            }
        }
    }

    /// Recalcula tamaño en píxeles, columnas y filas; ajusta el layer, el
    /// núcleo y la PTY, y pide un frame.
    pub fn update_layout(&self) {
        let Some(window) = self.window() else {
            return;
        };
        let scale = window.backingScaleFactor();
        let size = self.bounds().size;
        let m = self.metrics_for(scale);
        let width_px = (size.width * scale).round().max(1.0) as u32;
        let height_px = (size.height * scale).round().max(1.0) as u32;
        let pad_px = (PAD_PT * scale).round() as u32;
        let cols = (width_px.saturating_sub(2 * pad_px) / m.cell_w).max(1) as usize;
        let rows = (height_px.saturating_sub(2 * pad_px) / m.cell_h).max(1) as usize;

        let shared = &self.ivars().shared;
        {
            let layer = shared.layer.lock();
            layer.setContentsScale(scale);
            layer.setDrawableSize(CGSize::new(width_px as f64, height_px as f64));
        }
        *shared.layout.lock().unwrap() = Layout {
            scale,
            width_px,
            height_px,
            pad_px,
        };
        {
            let mut term = shared.term.lock().unwrap();
            if term.grid().cols() != cols || term.grid().rows() != rows {
                term.resize(cols, rows);
                let _ = shared
                    .writer
                    .lock()
                    .unwrap()
                    .resize(cols as u16, rows as u16);
            }
        }
        self.request_frame();
    }

    /// Rectángulo del cursor en coordenadas de pantalla, para situar la
    /// ventana de composición de los métodos de entrada.
    fn cursor_screen_rect(&self) -> NSRect {
        let Some(window) = self.window() else {
            return NSRect::ZERO;
        };
        let scale = window.backingScaleFactor();
        let m = self.metrics_for(scale);
        let cursor = self.ivars().shared.term.lock().unwrap().cursor();
        let (cw, ch) = (m.cell_w as f64 / scale, m.cell_h as f64 / scale);
        let height = self.bounds().size.height;
        let rect = NSRect::new(
            NSPoint::new(
                PAD_PT + cursor.col as f64 * cw,
                height - PAD_PT - (cursor.row + 1) as f64 * ch,
            ),
            NSSize::new(cw, ch),
        );
        let in_window = self.convertRect_toView(rect, None);
        window.convertRectToScreen(in_window)
    }

    /// Envía entrada del usuario a la PTY y vuelve la vista abajo del todo.
    fn send_input(&self, bytes: &[u8]) {
        let shared = &self.ivars().shared;
        let moved = {
            let mut term = shared.term.lock().unwrap();
            let moved = term.display_offset() > 0;
            term.scroll_to_bottom();
            moved
        };
        shared.write(bytes);
        if moved {
            self.request_frame();
        }
    }

    /// Tamaño de la celda en puntos.
    fn cell_size_pt(&self) -> (f64, f64) {
        let scale = self.window().map_or(2.0, |w| w.backingScaleFactor());
        let m = self.metrics_for(scale);
        (m.cell_w as f64 / scale, m.cell_h as f64 / scale)
    }

    /// Fila y columna de la vista bajo el ratón, sin limitar (la fila puede
    /// ser negativa o pasar del final al arrastrar fuera de la ventana).
    fn cell_at(&self, event: &NSEvent) -> (isize, usize) {
        let p = self.convertPoint_fromView(event.locationInWindow(), None);
        let (cw, ch) = self.cell_size_pt();
        let y = self.bounds().size.height - p.y - PAD_PT;
        let row = (y / ch).floor() as isize;
        let col = ((p.x - PAD_PT) / cw).floor().max(0.0) as usize;
        (row, col)
    }

    fn handle_scroll(&self, event: &NSEvent) {
        let (_, ch) = self.cell_size_pt();
        let dy = event.scrollingDeltaY();
        let lines = if event.hasPreciseScrollingDeltas() {
            let acc = self.ivars().scroll_accum.get() + dy / ch;
            let whole = acc.trunc();
            self.ivars().scroll_accum.set(acc - whole);
            whole as isize
        } else {
            (dy * 3.0).round() as isize
        };
        if lines == 0 {
            return;
        }
        let shared = &self.ivars().shared;
        let mut term = shared.term.lock().unwrap();
        let modes = term.modes();
        if modes.alt_screen {
            // En la pantalla alternativa (less, vim…) la rueda envía flechas.
            drop(term);
            let key = if lines > 0 {
                input::Key::Up
            } else {
                input::Key::Down
            };
            let bytes = input::encode_key(key, Mods::default(), modes.app_cursor);
            shared.write(&bytes.repeat(lines.unsigned_abs()));
        } else {
            term.scroll_display(lines);
            drop(term);
            self.request_frame();
        }
    }

    fn handle_mouse_down(&self, event: &NSEvent) {
        let flags = event.modifierFlags();
        let (row, col) = self.cell_at(event);
        let clicks = event.clickCount();
        {
            let mut term = self.ivars().shared.term.lock().unwrap();
            let rows = term.grid().rows() as isize;
            let line = term.visible_line(row.clamp(0, rows - 1) as usize);
            let at = selection::point(line, col);
            let kind = match clicks {
                2 => SelectionKind::Word,
                n if n >= 3 => SelectionKind::Line,
                _ if flags.contains(NSEventModifierFlags::Option) => SelectionKind::Block,
                _ => SelectionKind::Simple,
            };
            let selection = match term.selection() {
                Some(mut sel) if flags.contains(NSEventModifierFlags::Shift) => {
                    sel.head = at;
                    sel
                }
                _ => Selection::new(kind, at),
            };
            term.set_selection(Some(selection));
        }
        // Doble y triple clic seleccionan sin necesidad de arrastrar.
        self.ivars()
            .drag_moved
            .set(clicks > 1 || flags.contains(NSEventModifierFlags::Shift));
        self.request_frame();
    }

    fn handle_mouse_dragged(&self, event: &NSEvent) {
        let (row, col) = self.cell_at(event);
        {
            let mut term = self.ivars().shared.term.lock().unwrap();
            let rows = term.grid().rows() as isize;
            // Arrastrar por encima o por debajo desplaza la vista.
            if row < 0 {
                term.scroll_display(1);
            } else if row >= rows {
                term.scroll_display(-1);
            }
            let line = term.visible_line(row.clamp(0, rows - 1) as usize);
            if let Some(mut sel) = term.selection() {
                sel.head = selection::point(line, col);
                term.set_selection(Some(sel));
            }
        }
        self.ivars().drag_moved.set(true);
        self.request_frame();
    }

    fn handle_key(&self, event: &NSEvent) {
        let flags = event.modifierFlags();
        let mods = Mods {
            shift: flags.contains(NSEventModifierFlags::Shift),
            alt: flags.contains(NSEventModifierFlags::Option),
            ctrl: flags.contains(NSEventModifierFlags::Control),
        };
        let cmd = flags.contains(NSEventModifierFlags::Command);
        let shared = &self.ivars().shared;
        let composing = !self.ivars().marked.borrow().is_empty();

        if !composing && !cmd {
            if let Some(key) = input::key_from_keycode(event.keyCode()) {
                let app_cursor = shared.term.lock().unwrap().modes().app_cursor;
                self.send_input(&input::encode_key(key, mods, app_cursor));
                return;
            }
            if mods.ctrl {
                let byte = event
                    .charactersIgnoringModifiers()
                    .and_then(|s| s.to_string().chars().next())
                    .and_then(input::ctrl_byte);
                if let Some(byte) = byte {
                    self.send_input(&[byte]);
                    return;
                }
            }
        }
        if cmd {
            // Los atajos con Cmd los resuelve el menú; el resto se ignora.
            return;
        }
        // Texto normal, Option que compone y teclas muertas: los resuelve el
        // sistema de entrada y llegan por `insertText:` o `setMarkedText:`.
        let events = NSArray::from_slice(&[event]);
        self.interpretKeyEvents(&events);
    }
}
