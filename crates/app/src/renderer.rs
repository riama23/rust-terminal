//! Renderizado con Metal: atlas de glifos y un único draw instanciado de quads.
//!
//! Vive en el hilo de render. Guarda su propia copia de la cuadrícula y solo
//! copia del núcleo las filas sucias, para que el bloqueo del estado dure poco.

use std::collections::HashMap;
use std::ptr::NonNull;

use objc2::rc::Retained;
use objc2::runtime::ProtocolObject;
use objc2_foundation::{NSString, NSUInteger};
use objc2_metal::{
    MTLBlendFactor, MTLClearColor, MTLCommandBuffer, MTLCommandEncoder, MTLCommandQueue,
    MTLCreateSystemDefaultDevice, MTLDevice, MTLDrawable, MTLLibrary, MTLLoadAction, MTLOrigin,
    MTLPixelFormat, MTLPrimitiveType, MTLRegion, MTLRenderCommandEncoder, MTLRenderPassDescriptor,
    MTLRenderPipelineDescriptor, MTLRenderPipelineState, MTLResourceOptions, MTLSize,
    MTLStoreAction, MTLTexture, MTLTextureDescriptor, MTLTextureUsage,
};
use objc2_quartz_core::{CAMetalDrawable, CAMetalLayer};
use rust_terminal_core::{Cell, Cursor, Flags, Modes};

use crate::config::Theme;
use crate::font::{FontSpec, Fonts, Style};
use crate::palette::{self, Rgba};

const SHADER: &str = r#"
#include <metal_stdlib>
using namespace metal;

struct Quad {
    float4 rect;   // x, y, ancho, alto en píxeles (origen arriba a la izquierda)
    float4 uv;     // x, y, ancho, alto en píxeles del atlas
    float4 color;
    uint kind;     // 0 = relleno, 1 = glifo teñido, 2 = glifo de color
    uint pad0;
    uint pad1;
    uint pad2;
};

struct VOut {
    float4 position [[position]];
    float2 uv;
    float4 color;
    uint kind [[flat]];
};

vertex VOut vs(uint vid [[vertex_id]],
               uint iid [[instance_id]],
               const device Quad* quads [[buffer(0)]],
               constant float2& viewport [[buffer(1)]]) {
    Quad q = quads[iid];
    float2 corner = float2(vid & 1, vid >> 1);
    float2 px = q.rect.xy + corner * q.rect.zw;
    VOut out;
    out.position = float4(px.x / viewport.x * 2.0 - 1.0, 1.0 - px.y / viewport.y * 2.0, 0.0, 1.0);
    out.uv = q.uv.xy + corner * q.uv.zw;
    out.color = q.color;
    out.kind = q.kind;
    return out;
}

fragment float4 fs(VOut in [[stage_in]], texture2d<float> atlas [[texture(0)]]) {
    constexpr sampler s(coord::pixel, filter::nearest);
    if (in.kind == 0) {
        return float4(in.color.rgb * in.color.a, in.color.a);
    }
    float4 t = atlas.sample(s, in.uv);
    if (in.kind == 1) {
        float a = t.a * in.color.a;
        return float4(in.color.rgb * a, a);
    }
    return t * in.color.a;
}
"#;

#[repr(C)]
#[derive(Clone, Copy)]
struct Quad {
    rect: [f32; 4],
    uv: [f32; 4],
    color: Rgba,
    kind: u32,
    _pad: [u32; 3],
}

const KIND_FILL: u32 = 0;
const KIND_GLYPH: u32 = 1;
const KIND_COLOR_GLYPH: u32 = 2;

const ATLAS_SIZE: u32 = 2048;

#[derive(Clone, Copy)]
struct Slot {
    x: u32,
    y: u32,
    w: u32,
    h: u32,
    color: bool,
}

/// Atlas por estantes: todos los glifos miden una o dos celdas, así que basta
/// con ir llenando filas de altura `cell_h`.
struct Atlas {
    texture: Retained<ProtocolObject<dyn MTLTexture>>,
    cursor_x: u32,
    cursor_y: u32,
    slots: HashMap<(char, Style), Option<Slot>>,
    cluster_slots: HashMap<(String, Style), Option<Slot>>,
}

impl Atlas {
    fn new(device: &ProtocolObject<dyn MTLDevice>) -> Atlas {
        // SAFETY: descriptor 2D estándar.
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                ATLAS_SIZE as NSUInteger,
                ATLAS_SIZE as NSUInteger,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::ShaderRead);
        let texture = device
            .newTextureWithDescriptor(&desc)
            .expect("textura del atlas");
        Atlas {
            texture,
            cursor_x: 0,
            cursor_y: 0,
            slots: HashMap::new(),
            cluster_slots: HashMap::new(),
        }
    }

    fn clear(&mut self) {
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.slots.clear();
        self.cluster_slots.clear();
    }

    fn get(&mut self, fonts: &Fonts, ch: char, style: Style, cells: u32) -> Option<Slot> {
        if let Some(slot) = self.slots.get(&(ch, style)) {
            return *slot;
        }
        let slot = self.upload(fonts.rasterize(ch, style, cells));
        self.slots.insert((ch, style), slot);
        slot
    }

    /// Como [`Atlas::get`], para un grupo de caracteres (base + combinantes).
    fn get_text(&mut self, fonts: &Fonts, text: &str, style: Style, cells: u32) -> Option<Slot> {
        let key = (text.to_owned(), style);
        if let Some(slot) = self.cluster_slots.get(&key) {
            return *slot;
        }
        let slot = self.upload(fonts.rasterize_text(text, style, cells));
        self.cluster_slots.insert(key, slot);
        slot
    }

    fn upload(&mut self, raster: crate::font::Raster) -> Option<Slot> {
        if raster.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            return None;
        }
        if self.cursor_x + raster.width > ATLAS_SIZE {
            self.cursor_x = 0;
            self.cursor_y += raster.height;
        }
        if self.cursor_y + raster.height > ATLAS_SIZE {
            // Atlas lleno: se vacía y se vuelve a llenar con lo que se use.
            self.clear();
        }
        let (x, y) = (self.cursor_x, self.cursor_y);
        self.cursor_x += raster.width;
        let region = MTLRegion {
            origin: MTLOrigin {
                x: x as NSUInteger,
                y: y as NSUInteger,
                z: 0,
            },
            size: MTLSize {
                width: raster.width as NSUInteger,
                height: raster.height as NSUInteger,
                depth: 1,
            },
        };
        // SAFETY: el buffer tiene `width * height * 4` bytes y la región cabe en la textura.
        unsafe {
            self.texture
                .replaceRegion_mipmapLevel_withBytes_bytesPerRow(
                    region,
                    0,
                    NonNull::new_unchecked(raster.pixels.as_ptr() as *mut _),
                    (raster.width * 4) as NSUInteger,
                );
        }
        Some(Slot {
            x,
            y,
            w: raster.width,
            h: raster.height,
            color: raster.color,
        })
    }
}

/// Lo que se dibuja encima de las celdas, calculado con el bloqueo tomado.
#[derive(Debug, Clone, Default)]
pub struct Overlay {
    /// Columnas seleccionadas `[inicio, fin]` por fila de la vista.
    pub selection: Vec<(usize, usize, usize)>,
    /// Marcas de bloque: fila de la vista del prompt y si el comando falló.
    pub marks: Vec<(usize, bool)>,
    /// Enlace bajo el ratón (Cmd): columnas `[inicio, fin]` por fila, se subrayan.
    pub links: Vec<(usize, usize, usize)>,
    /// Bloques con fondo propio (fallidos o seleccionados), en filas de la vista.
    pub blocks: Vec<BlockSpan>,
    /// Filas ocultas por bloques colapsados (vacío: ninguna).
    pub row_map: crate::blocks::RowMap,
}

/// Las filas de la vista que ocupa un bloque y cómo se tiñen.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct BlockSpan {
    pub first: usize,
    pub last: usize,
    pub failed: bool,
    pub selected: bool,
}

/// Tamaño del drawable y escala.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub scale: f64,
    pub width_px: u32,
    pub height_px: u32,
}

/// Un panel a pintar: su rectángulo en píxeles (x, y, ancho, alto; incluye
/// el margen interior `pad`) y su estado copiado con el bloqueo tomado.
#[derive(Debug, Clone)]
pub struct PaneFrame {
    pub id: u64,
    pub rect: [f32; 4],
    pub pad: f32,
    pub cursor: Cursor,
    pub modes: Modes,
    pub overlay: Overlay,
    /// Panel con el foco del teclado (cursor macizo; los demás, hueco).
    pub focused: bool,
    /// Texto en composición, que se dibuja en el cursor.
    pub preedit: Option<String>,
    /// Panel sin foco entre varios: se pinta atenuado.
    pub dim: bool,
}

/// Cabecera de un panel (solo con varios paneles en la pestaña).
#[derive(Debug, Clone, PartialEq)]
pub struct PaneHeader {
    pub rect: [f32; 4],
    pub title: String,
    pub focused: bool,
}

/// Opacidad de un panel atenuado (spec: 55 %).
pub const DIM_OPACITY: f32 = 0.55;

/// Una pestaña de la barra.
#[derive(Debug, Clone, PartialEq)]
pub struct TabItem {
    pub title: String,
    pub rect: [f32; 4],
    pub close: [f32; 4],
    pub active: bool,
    /// Un programa corre en la pestaña: lleva spinner.
    pub busy: bool,
}

/// La barra de pestañas, ya maquetada en píxeles.
#[derive(Debug, Clone, PartialEq, Default)]
pub struct TabBar {
    pub rect: [f32; 4],
    pub tabs: Vec<TabItem>,
    pub plus: [f32; 4],
    /// Fotograma del spinner (lo avanza la app mientras alguna pestaña está ocupada).
    pub spinner_phase: usize,
}

/// Todo lo que hay que pintar en un frame.
#[derive(Debug, Clone)]
pub struct Frame {
    pub layout: Layout,
    pub panes: Vec<PaneFrame>,
    pub dividers: Vec<[f32; 4]>,
    pub headers: Vec<PaneHeader>,
    pub tab_bar: Option<TabBar>,
}

pub struct Renderer {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    atlas: Atlas,
    fonts: Fonts,
    font: FontSpec,
    scale: f64,
    theme: Theme,
    /// Copia local de la pantalla de cada panel.
    panes: HashMap<u64, Vec<Vec<Cell>>>,
    /// Texto de los grupos de caracteres que aparecen en cada panel.
    clusters: HashMap<u64, HashMap<char, String>>,
    /// Códigos de grupo vistos al copiar filas, pendientes de traducir.
    pending_clusters: Vec<(u64, char)>,
    quads: Vec<Quad>,
}

// SAFETY: el renderizador solo se usa desde el hilo de render. Apple permite
// pedir drawables a un CAMetalLayer y codificar comandos desde otro hilo.
unsafe impl Send for Renderer {}

pub fn create_device() -> Retained<ProtocolObject<dyn MTLDevice>> {
    MTLCreateSystemDefaultDevice().expect("no hay GPU compatible con Metal")
}

impl Renderer {
    pub fn new(
        device: Retained<ProtocolObject<dyn MTLDevice>>,
        pixel_format: MTLPixelFormat,
        font: &FontSpec,
        theme: Theme,
        scale: f64,
    ) -> Renderer {
        let queue = device.newCommandQueue().expect("cola de comandos");
        let library = device
            .newLibraryWithSource_options_error(&NSString::from_str(SHADER), None)
            .unwrap_or_else(|e| panic!("error compilando shaders: {e}"));
        let vs = library
            .newFunctionWithName(&NSString::from_str("vs"))
            .expect("función vs");
        let fs = library
            .newFunctionWithName(&NSString::from_str("fs"))
            .expect("función fs");

        let desc = MTLRenderPipelineDescriptor::new();
        desc.setVertexFunction(Some(&vs));
        desc.setFragmentFunction(Some(&fs));
        // SAFETY: el índice 0 siempre existe.
        let attachment = unsafe { desc.colorAttachments().objectAtIndexedSubscript(0) };
        attachment.setPixelFormat(pixel_format);
        attachment.setBlendingEnabled(true);
        attachment.setSourceRGBBlendFactor(MTLBlendFactor::One);
        attachment.setSourceAlphaBlendFactor(MTLBlendFactor::One);
        attachment.setDestinationRGBBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        attachment.setDestinationAlphaBlendFactor(MTLBlendFactor::OneMinusSourceAlpha);
        let pipeline = device
            .newRenderPipelineStateWithDescriptor_error(&desc)
            .unwrap_or_else(|e| panic!("error creando el pipeline: {e}"));

        Renderer {
            atlas: Atlas::new(&device),
            fonts: Fonts::new(font, scale),
            font: font.clone(),
            scale,
            theme,
            device,
            queue,
            pipeline,
            panes: HashMap::new(),
            clusters: HashMap::new(),
            pending_clusters: Vec::new(),
            quads: Vec::new(),
        }
    }

    /// Aplica la fuente, la escala (Retina o no) y el tema. Si cambia la
    /// fuente o la escala, se recrean las fuentes y se vacía el atlas.
    pub fn set_appearance(&mut self, font: &FontSpec, scale: f64, theme: Theme) {
        if scale != self.scale || *font != self.font {
            self.scale = scale;
            self.font = font.clone();
            self.fonts = Fonts::new(font, scale);
            self.atlas.clear();
        }
        self.theme = theme;
    }

    /// Ajusta la copia local de un panel al tamaño de su cuadrícula. Devuelve
    /// `true` si es nueva o cambió, y entonces hay que copiar todas las filas.
    pub fn ensure_pane(&mut self, id: u64, cols: usize, rows: usize) -> bool {
        let copy = self.panes.entry(id).or_default();
        if copy.len() != rows || copy.first().is_some_and(|r| r.len() != cols) || copy.is_empty() {
            *copy = vec![vec![Cell::default(); cols]; rows];
            true
        } else {
            false
        }
    }

    /// Actualiza una fila de la copia local de un panel (con el bloqueo
    /// tomado). Las filas del historial pueden ser más cortas: el resto queda
    /// en blanco.
    pub fn update_row(&mut self, id: u64, i: usize, row: &[Cell]) {
        let Some(dst) = self.panes.get_mut(&id).and_then(|p| p.get_mut(i)) else {
            return;
        };
        let n = row.len().min(dst.len());
        dst[..n].copy_from_slice(&row[..n]);
        dst[n..].fill(Cell::default());
        for c in &row[..n] {
            if is_cluster(c.ch) {
                self.pending_clusters.push((id, c.ch));
            }
        }
    }

    /// Códigos de grupo vistos desde la última llamada; hay que darles su
    /// texto con [`Renderer::set_cluster`] (con la cuadrícula del panel).
    pub fn take_pending_clusters(&mut self) -> Vec<(u64, char)> {
        std::mem::take(&mut self.pending_clusters)
    }

    pub fn set_cluster(&mut self, id: u64, ch: char, text: String) {
        self.clusters.entry(id).or_default().insert(ch, text);
    }

    /// Olvida las copias de los paneles que ya no se pintan.
    pub fn retain_panes(&mut self, keep: impl Fn(u64) -> bool) {
        self.panes.retain(|id, _| keep(*id));
        self.clusters.retain(|id, _| keep(*id));
    }

    /// Construye un frame y lo presenta en el siguiente drawable del layer.
    pub fn draw(&mut self, layer: &CAMetalLayer, frame: &Frame) {
        let layout = frame.layout;
        self.build_quads(frame);
        let Some(drawable) = layer.nextDrawable() else {
            return;
        };
        let Some(command_buffer) = self.encode(&drawable.texture(), layout) else {
            return;
        };
        let drawable: &ProtocolObject<dyn MTLDrawable> = ProtocolObject::from_ref(&*drawable);
        command_buffer.presentDrawable(drawable);
        command_buffer.commit();
    }

    /// Codifica los quads del frame sobre `texture` y devuelve el command
    /// buffer sin enviar.
    fn encode(
        &mut self,
        texture: &ProtocolObject<dyn MTLTexture>,
        layout: Layout,
    ) -> Option<Retained<ProtocolObject<dyn MTLCommandBuffer>>> {
        let command_buffer = self.queue.commandBuffer()?;
        let pass = MTLRenderPassDescriptor::new();
        // SAFETY: el índice 0 siempre existe.
        let color = unsafe { pass.colorAttachments().objectAtIndexedSubscript(0) };
        let bg = palette::to_rgba(self.theme.background, 1.0);
        color.setTexture(Some(texture));
        color.setLoadAction(MTLLoadAction::Clear);
        color.setStoreAction(MTLStoreAction::Store);
        color.setClearColor(MTLClearColor {
            red: bg[0] as f64,
            green: bg[1] as f64,
            blue: bg[2] as f64,
            alpha: 1.0,
        });
        let encoder = command_buffer.renderCommandEncoderWithDescriptor(&pass)?;

        if !self.quads.is_empty() {
            let bytes = std::mem::size_of_val(self.quads.as_slice());
            // SAFETY: el puntero y la longitud describen `self.quads`, que Metal copia.
            let buffer = unsafe {
                self.device.newBufferWithBytes_length_options(
                    NonNull::new_unchecked(self.quads.as_ptr() as *mut _),
                    bytes as NSUInteger,
                    MTLResourceOptions::StorageModeShared,
                )
            }
            .expect("buffer de quads");
            let viewport = [layout.width_px as f32, layout.height_px as f32];
            encoder.setRenderPipelineState(&self.pipeline);
            // SAFETY: los índices y tamaños coinciden con los del shader.
            unsafe {
                encoder.setVertexBuffer_offset_atIndex(Some(&buffer), 0, 0);
                encoder.setVertexBytes_length_atIndex(
                    NonNull::new_unchecked(viewport.as_ptr() as *mut _),
                    std::mem::size_of_val(&viewport) as NSUInteger,
                    1,
                );
                encoder.setFragmentTexture_atIndex(Some(&self.atlas.texture), 0);
                encoder.drawPrimitives_vertexStart_vertexCount_instanceCount(
                    MTLPrimitiveType::TriangleStrip,
                    0,
                    4,
                    self.quads.len() as NSUInteger,
                );
            }
        }
        encoder.endEncoding();
        Some(command_buffer)
    }

    fn build_quads(&mut self, frame: &Frame) {
        self.quads.clear();
        let mut glyphs: Vec<Quad> = Vec::new();
        for pane in &frame.panes {
            self.build_pane(pane, &mut glyphs);
        }
        for d in &frame.dividers {
            self.quads
                .push(fill(*d, palette::to_rgba(palette::DIVIDER, 1.0)));
        }
        for h in &frame.headers {
            self.build_pane_header(h, &mut glyphs);
        }
        if let Some(bar) = &frame.tab_bar {
            self.build_tab_bar(bar, &mut glyphs);
        }
        self.quads.extend(glyphs);
        // Atenuado: el fondo por encima del panel (texto incluido) deja ver
        // el contenido al 55 %.
        for pane in frame.panes.iter().filter(|p| p.dim) {
            self.quads.push(fill(
                pane.rect,
                palette::to_rgba(self.theme.background, 1.0 - DIM_OPACITY),
            ));
        }
    }

    /// Texto de una sola línea a partir de `(x, y)` (arriba a la izquierda),
    /// recortado con "…" si no cabe en `max_w`.
    fn push_text(
        &mut self,
        text: &str,
        x: f32,
        y: f32,
        max_w: f32,
        color: Rgba,
        out: &mut Vec<Quad>,
    ) {
        let m = self.fonts.metrics;
        let cw = m.cell_w as f32;
        let fit = (max_w / cw).floor().max(0.0) as usize;
        let chars: Vec<char> = text.chars().collect();
        let shown: Vec<char> = if chars.len() > fit && fit > 0 {
            chars[..fit - 1]
                .iter()
                .copied()
                .chain(std::iter::once('…'))
                .collect()
        } else {
            chars.into_iter().take(fit).collect()
        };
        let style = Style {
            bold: false,
            italic: false,
        };
        for (i, ch) in shown.into_iter().enumerate() {
            if ch == ' ' {
                continue;
            }
            if let Some(slot) = self.atlas.get(&self.fonts, ch, style, 1) {
                out.push(glyph_quad(x + i as f32 * cw, y, slot, color));
            }
        }
    }

    fn build_pane_header(&mut self, header: &PaneHeader, glyphs: &mut Vec<Quad>) {
        let m = self.fonts.metrics;
        let (cw, ch) = (m.cell_w as f32, m.cell_h as f32);
        let [x, y, w, h] = header.rect;
        self.quads
            .push(fill(header.rect, palette::to_rgba(palette::TAB_BAR, 1.0)));
        // Línea inferior, y el acento en el panel con el foco.
        self.quads.push(fill(
            [x, y + h - 1.0, w, 1.0],
            palette::to_rgba(palette::DIVIDER, 1.0),
        ));
        let fg = if header.focused {
            let accent_h = (ch / 12.0).round().max(2.0);
            self.quads.push(fill(
                [x, y, w, accent_h],
                palette::to_rgba(palette::ACCENT, 1.0),
            ));
            self.theme.foreground
        } else {
            palette::TAB_INACTIVE_FG
        };
        self.push_text(
            &header.title,
            x + cw,
            y + ((h - ch) / 2.0).round(),
            w - cw * 2.0,
            palette::to_rgba(fg, 1.0),
            glyphs,
        );
    }

    fn build_tab_bar(&mut self, bar: &TabBar, glyphs: &mut Vec<Quad>) {
        let m = self.fonts.metrics;
        let ch = m.cell_h as f32;
        let cw = m.cell_w as f32;
        self.quads
            .push(fill(bar.rect, palette::to_rgba(palette::TAB_BAR, 1.0)));
        for tab in &bar.tabs {
            let [x, y, w, h] = tab.rect;
            let (bg, fg) = if tab.active {
                (self.theme.background, self.theme.foreground)
            } else {
                (palette::TAB_BAR, palette::TAB_INACTIVE_FG)
            };
            self.quads.push(fill(tab.rect, palette::to_rgba(bg, 1.0)));
            if tab.active {
                let accent_h = (ch / 12.0).round().max(2.0);
                self.quads.push(fill(
                    [x, y, w, accent_h],
                    palette::to_rgba(palette::ACCENT, 1.0),
                ));
            }
            // Separador a la derecha de cada pestaña.
            self.quads.push(fill(
                [x + w - 1.0, y + h * 0.2, 1.0, h * 0.6],
                palette::to_rgba(palette::DIVIDER, 1.0),
            ));
            let text_y = y + ((h - ch) / 2.0).round();
            let mut text_x = x + cw;
            if tab.busy {
                const SPINNER: [&str; 10] = ["⠋", "⠙", "⠹", "⠸", "⠼", "⠴", "⠦", "⠧", "⠇", "⠏"];
                self.push_text(
                    SPINNER[bar.spinner_phase % SPINNER.len()],
                    text_x,
                    text_y,
                    cw * 2.0,
                    palette::to_rgba(palette::ACCENT, 1.0),
                    glyphs,
                );
                text_x += cw * 2.0;
            }
            let max_w = tab.close[0] - text_x - cw * 0.5;
            self.push_text(
                &tab.title,
                text_x,
                text_y,
                max_w,
                palette::to_rgba(fg, 1.0),
                glyphs,
            );
            let [cx, cy, cwid, chei] = tab.close;
            self.push_text(
                "×",
                cx + ((cwid - cw) / 2.0).round(),
                cy + ((chei - ch) / 2.0).round(),
                cw,
                palette::to_rgba(palette::TAB_INACTIVE_FG, 1.0),
                glyphs,
            );
        }
        let [px, py, pw, ph] = bar.plus;
        self.push_text(
            "+",
            px + ((pw - cw) / 2.0).round(),
            py + ((ph - ch) / 2.0).round(),
            cw,
            palette::to_rgba(palette::TAB_INACTIVE_FG, 1.0),
            glyphs,
        );
    }

    fn build_pane(&mut self, pane: &PaneFrame, glyphs: &mut Vec<Quad>) {
        let Some(rows) = self.panes.get(&pane.id) else {
            return;
        };
        let rows = rows.clone();
        let m = self.fonts.metrics;
        let (cw, ch) = (m.cell_w as f32, m.cell_h as f32);
        let pad = pane.pad;
        let (ox, oy) = (pane.rect[0] + pad, pane.rect[1] + pad);
        let (cursor, modes, overlay) = (pane.cursor, pane.modes, &pane.overlay);

        // Marcas de bloque en el margen izquierdo.
        let mark_w = (pad * 0.3).round().max(2.0);
        let map = &overlay.row_map;
        for &(r, failed) in overlay.marks.iter() {
            let Some(sr) = map.screen_of(r) else { continue };
            let color = if failed {
                palette::MARK_FAILED
            } else {
                palette::MARK_OK
            };
            let y = oy + sr as f32 * ch;
            self.quads.push(fill(
                [pane.rect[0] + (pad - mark_w) / 3.0, y, mark_w, ch],
                palette::to_rgba(color, 1.0),
            ));
        }

        // Subrayado del enlace bajo el ratón.
        let stroke = m.stroke as f32;
        for &(r, a, b) in &overlay.links {
            let Some(sr) = map.screen_of(r) else { continue };
            let y = oy + sr as f32 * ch + (m.baseline as f32 + stroke).min(ch - stroke);
            glyphs.push(fill(
                [ox + a as f32 * cw, y, (b + 1 - a) as f32 * cw, stroke],
                palette::to_rgba(palette::ACCENT, 1.0),
            ));
        }

        for (r, row) in rows.iter().enumerate() {
            // Fila oculta por un bloque colapsado: no se pinta y las de debajo suben.
            let Some(sr) = map.screen_of(r) else {
                continue;
            };
            let y = oy + sr as f32 * ch;
            // La primera línea de una salida colapsada enseña un resumen.
            if let Some(&(_, hidden)) = map.summaries.iter().find(|(vr, _)| *vr == r) {
                self.quads.push(fill(
                    [pane.rect[0], y, pane.rect[2], ch],
                    palette::to_rgba(palette::TAB_BAR, 1.0),
                ));
                self.push_text(
                    &format!("▸ salida colapsada · {hidden} líneas ocultas"),
                    ox,
                    y,
                    pane.rect[2] - 2.0 * pad,
                    palette::to_rgba(palette::TAB_INACTIVE_FG, 1.0),
                    glyphs,
                );
                continue;
            }
            let selected = overlay
                .selection
                .iter()
                .find(|(sr, _, _)| *sr == r)
                .map(|&(_, a, b)| (a, b));
            for (c, cell) in row.iter().enumerate() {
                if cell.flags.contains(Flags::WIDE_SPACER) {
                    continue;
                }
                let cells = if cell.flags.contains(Flags::WIDE) {
                    2
                } else {
                    1
                };
                let x = ox + c as f32 * cw;
                let w = cw * cells as f32;

                let theme = self.theme;
                let mut fg = palette::rgb(cell.fg, theme.foreground, &theme);
                let mut bg = palette::rgb(cell.bg, theme.background, &theme);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let is_cursor = modes.cursor_visible && cursor.row == r && cursor.col == c;
                if is_cursor && pane.focused {
                    fg = bg;
                    bg = theme.cursor;
                } else if selected.is_some_and(|(a, b)| c >= a && c <= b) {
                    bg = theme.selection;
                }
                if bg != theme.background {
                    self.quads
                        .push(fill([x, y, w, ch], palette::to_rgba(bg, 1.0)));
                }
                if is_cursor && !pane.focused {
                    // Cursor hueco en los paneles sin foco.
                    let t = m.stroke.max(1) as f32;
                    let c = palette::to_rgba(theme.cursor, 1.0);
                    for rect in [
                        [x, y, w, t],
                        [x, y + ch - t, w, t],
                        [x, y, t, ch],
                        [x + w - t, y, t, ch],
                    ] {
                        glyphs.push(fill(rect, c));
                    }
                }

                let alpha = if cell.flags.contains(Flags::DIM) {
                    0.6
                } else {
                    1.0
                };
                let fg_rgba = palette::to_rgba(fg, alpha);
                if cell.ch != ' ' {
                    let style = Style {
                        bold: cell.flags.contains(Flags::BOLD),
                        italic: cell.flags.contains(Flags::ITALIC),
                    };
                    let slot = match self.clusters.get(&pane.id).and_then(|m| m.get(&cell.ch)) {
                        Some(text) if is_cluster(cell.ch) => {
                            let text = text.clone();
                            self.atlas.get_text(&self.fonts, &text, style, cells)
                        }
                        _ => self.atlas.get(&self.fonts, cell.ch, style, cells),
                    };
                    if let Some(slot) = slot {
                        glyphs.push(glyph_quad(x, y, slot, fg_rgba));
                    }
                }
                let stroke = m.stroke as f32;
                if cell.flags.contains(Flags::UNDERLINE) {
                    let uy = y + m.baseline as f32 + stroke;
                    glyphs.push(fill([x, uy.min(y + ch - stroke), w, stroke], fg_rgba));
                }
                if cell.flags.contains(Flags::STRIKEOUT) {
                    let sy = y + (m.baseline as f32 * 0.65).round();
                    glyphs.push(fill([x, sy, w, stroke], fg_rgba));
                }
            }
        }
        // Bloques: fondo tintado (rojo si falló, acento si está seleccionado)
        // sobre las celdas y bajo el texto.
        for span in &overlay.blocks {
            let (Some(first), Some(last)) = (
                map.screen_at_or_after(span.first),
                map.screen_at_or_before(span.last),
            ) else {
                continue;
            };
            if first > last {
                continue;
            }
            let y = oy + first as f32 * ch;
            let h = (last + 1 - first) as f32 * ch;
            let rect = [pane.rect[0], y, pane.rect[2], h];
            if span.failed {
                self.quads.push(fill(
                    rect,
                    palette::to_rgba(
                        palette::MARK_FAILED,
                        if span.selected { 0.16 } else { 0.09 },
                    ),
                ));
            } else if span.selected {
                self.quads
                    .push(fill(rect, palette::to_rgba(palette::ACCENT, 0.12)));
            }
            if span.selected {
                let bar = (pad * 0.5).round().max(3.0);
                self.quads.push(fill(
                    [pane.rect[0], y, bar, h],
                    palette::to_rgba(palette::ACCENT, 1.0),
                ));
            }
        }
        if let Some(text) = &pane.preedit {
            self.draw_preedit(text, pane, glyphs);
        }
    }

    /// Texto en composición: celdas con fondo propio y subrayado, desde el
    /// cursor, encima de lo que haya (la aplicación aún no lo ha recibido).
    fn draw_preedit(&mut self, text: &str, pane: &PaneFrame, glyphs: &mut Vec<Quad>) {
        let m = self.fonts.metrics;
        let (cw, ch) = (m.cell_w as f32, m.cell_h as f32);
        let (ox, oy) = (pane.rect[0] + pane.pad, pane.rect[1] + pane.pad);
        let cursor_row = pane
            .overlay
            .row_map
            .screen_of(pane.cursor.row)
            .unwrap_or(pane.cursor.row);
        let y = oy + cursor_row as f32 * ch;
        let mut x = ox + pane.cursor.col as f32 * cw;
        let max_x = pane.rect[0] + pane.rect[2] - pane.pad;
        let theme = self.theme;
        let style = Style {
            bold: false,
            italic: false,
        };
        let start = x;
        for c in text.chars() {
            let cells = rust_terminal_core::char_width(c).max(1) as u32;
            let w = cw * cells as f32;
            if x + w > max_x {
                break;
            }
            self.quads
                .push(fill([x, y, w, ch], palette::to_rgba(theme.selection, 1.0)));
            if let Some(slot) = self.atlas.get(&self.fonts, c, style, cells) {
                glyphs.push(glyph_quad(
                    x,
                    y,
                    slot,
                    palette::to_rgba(theme.foreground, 1.0),
                ));
            }
            x += w;
        }
        let stroke = m.stroke as f32;
        glyphs.push(fill(
            [start, y + ch - stroke, x - start, stroke],
            palette::to_rgba(palette::ACCENT, 1.0),
        ));
    }
}

/// Carácter de celda que representa un grupo (ver `Grid::cluster`).
fn is_cluster(ch: char) -> bool {
    ch as u32 >= 0x10_0000
}

fn glyph_quad(x: f32, y: f32, slot: Slot, color: Rgba) -> Quad {
    Quad {
        rect: [x, y, slot.w as f32, slot.h as f32],
        uv: [slot.x as f32, slot.y as f32, slot.w as f32, slot.h as f32],
        color,
        kind: if slot.color {
            KIND_COLOR_GLYPH
        } else {
            KIND_GLYPH
        },
        _pad: [0; 3],
    }
}

fn fill(rect: [f32; 4], color: Rgba) -> Quad {
    Quad {
        rect,
        uv: [0.0; 4],
        color,
        kind: KIND_FILL,
        _pad: [0; 3],
    }
}

#[cfg(test)]
mod tests {
    use objc2_metal::{MTLStorageMode, MTLTextureUsage};
    use rust_terminal_core::{Color, Term};

    use super::*;

    fn theme() -> Theme {
        crate::config::Config::default().theme
    }

    fn test_font() -> FontSpec {
        FontSpec {
            family: None,
            size_pt: 13.0,
        }
    }

    /// Layout de los tests: un solo panel que ocupa toda la textura.
    #[derive(Clone, Copy)]
    struct TestLayout {
        #[allow(dead_code)]
        scale: f64,
        width_px: u32,
        height_px: u32,
        pad_px: u32,
    }

    /// Pinta `term` en una textura fuera de pantalla y devuelve los píxeles BGRA.
    fn render(term: &Term, scale: f64) -> (TestLayout, crate::font::Metrics, Vec<u8>) {
        render_with(term, scale, &Overlay::default())
    }

    fn render_with(
        term: &Term,
        scale: f64,
        overlay: &Overlay,
    ) -> (TestLayout, crate::font::Metrics, Vec<u8>) {
        let device = create_device();
        let mut r = Renderer::new(
            device.clone(),
            MTLPixelFormat::BGRA8Unorm,
            &test_font(),
            theme(),
            scale,
        );
        let m = r.fonts.metrics;
        let grid = term.grid();
        r.ensure_pane(0, grid.cols(), grid.rows());
        for i in 0..grid.rows() {
            r.update_row(0, i, grid.row(i));
        }
        for (pane, ch) in r.take_pending_clusters() {
            r.set_cluster(pane, ch, grid.char_text(ch).into_owned());
        }
        let pad = 4;
        let layout = TestLayout {
            scale,
            width_px: 2 * pad + m.cell_w * grid.cols() as u32,
            height_px: 2 * pad + m.cell_h * grid.rows() as u32,
            pad_px: pad,
        };
        // SAFETY: descriptor 2D estándar.
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                layout.width_px as NSUInteger,
                layout.height_px as NSUInteger,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Shared);
        let texture = device.newTextureWithDescriptor(&desc).unwrap();

        let frame = Frame {
            layout: Layout {
                scale,
                width_px: layout.width_px,
                height_px: layout.height_px,
            },
            panes: vec![PaneFrame {
                id: 0,
                rect: [0.0, 0.0, layout.width_px as f32, layout.height_px as f32],
                pad: pad as f32,
                cursor: term.cursor(),
                modes: term.modes(),
                overlay: overlay.clone(),
                focused: true,
                preedit: None,
                dim: false,
            }],
            dividers: Vec::new(),
            headers: Vec::new(),
            tab_bar: None,
        };
        r.build_quads(&frame);
        let cb = r.encode(&texture, frame.layout).unwrap();
        cb.commit();
        cb.waitUntilCompleted();

        let stride = layout.width_px as usize * 4;
        let mut pixels = vec![0u8; stride * layout.height_px as usize];
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: layout.width_px as NSUInteger,
                height: layout.height_px as NSUInteger,
                depth: 1,
            },
        };
        // SAFETY: el buffer tiene el tamaño de la región completa.
        unsafe {
            texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                NonNull::new_unchecked(pixels.as_mut_ptr().cast()),
                stride as NSUInteger,
                region,
                0,
            );
        }
        (layout, m, pixels)
    }

    /// Píxeles (BGRA) de la celda `(row, col)`.
    fn cell_pixels(
        layout: TestLayout,
        m: crate::font::Metrics,
        pixels: &[u8],
        row: u32,
        col: u32,
    ) -> Vec<[u8; 4]> {
        let stride = layout.width_px as usize * 4;
        let mut out = Vec::new();
        for y in 0..m.cell_h {
            for x in 0..m.cell_w {
                let px = (layout.pad_px + col * m.cell_w + x) as usize;
                let py = (layout.pad_px + row * m.cell_h + y) as usize;
                let i = py * stride + px * 4;
                out.push([pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]);
            }
        }
        out
    }

    fn bgra(rgb: [u8; 3]) -> [u8; 4] {
        [rgb[2], rgb[1], rgb[0], 255]
    }

    #[test]
    fn renders_glyphs_backgrounds_and_cursor() {
        // Fila 0: "M", un espacio con fondo rojo y "日" (ancho). Cursor en (1, 0).
        let mut term = Term::new(5, 2);
        term.advance("M\x1b[41m \x1b[0m日\r\n".as_bytes());
        let (layout, m, pixels) = render(&term, 2.0);
        let bg = bgra(theme().background);

        let glyph = cell_pixels(layout, m, &pixels, 0, 0);
        assert!(
            glyph.iter().any(|p| p[1] > 150),
            "la M no tiene píxeles claros"
        );
        assert!(glyph.contains(&bg), "la celda de la M no conserva el fondo");

        let red = bgra(palette::rgb(
            Color::Indexed(1),
            theme().background,
            &theme(),
        ));
        assert!(
            cell_pixels(layout, m, &pixels, 0, 1)
                .iter()
                .all(|p| *p == red),
            "el fondo rojo no ocupa toda la celda"
        );

        // El carácter ancho pinta en sus dos celdas.
        assert!(
            cell_pixels(layout, m, &pixels, 0, 2)
                .iter()
                .any(|p| p[1] > 150)
        );
        assert!(
            cell_pixels(layout, m, &pixels, 0, 3)
                .iter()
                .any(|p| p[1] > 150)
        );

        // El cursor es un bloque del color del cursor.
        let cursor = bgra(theme().cursor);
        assert!(
            cell_pixels(layout, m, &pixels, 1, 0)
                .iter()
                .all(|p| *p == cursor)
        );
        // Y una celda vacía queda con el fondo por defecto.
        assert!(
            cell_pixels(layout, m, &pixels, 1, 2)
                .iter()
                .all(|p| *p == bg)
        );
    }

    #[test]
    fn failed_and_selected_blocks_are_tinted_under_the_text() {
        let mut term = Term::new(4, 3);
        term.advance(b"\x1b[?25l");
        let span = |first, failed, selected| BlockSpan {
            first,
            last: first,
            failed,
            selected,
        };
        let overlay = Overlay {
            blocks: vec![span(0, true, false), span(1, false, true)],
            ..Overlay::default()
        };
        let (layout, m, pixels) = render_with(&term, 2.0, &overlay);
        let bg = bgra(theme().background);
        // Fila 0 (fallida) y fila 1 (seleccionada) cambian de color; la 2 no.
        let center = |row: u32| cell_pixels(layout, m, &pixels, row, 3)[0];
        assert_ne!(center(0), bg, "el bloque fallido se tiñe");
        assert_ne!(center(1), bg, "el seleccionado se tiñe");
        assert_ne!(center(0), center(1), "con colores distintos");
        assert_eq!(center(2), bg, "el resto no cambia");
    }

    #[test]
    fn collapsed_rows_are_skipped_and_the_rest_moves_up() {
        // Cuatro líneas; la salida de la 1 a la 2 está colapsada.
        let mut term = Term::new(4, 4);
        term.advance(b"\x1b[?25lAAAA\r\nBBBB\r\nCCCC\r\nDDDD");
        let map = crate::blocks::row_map(&[crate::blocks::Collapse { first: 1, last: 2 }], 0, 4);
        let overlay = Overlay {
            row_map: map,
            ..Overlay::default()
        };
        let (layout, m, pixels) = render_with(&term, 2.0, &overlay);
        let bg = bgra(theme().background);
        let has_text = |row: u32| {
            cell_pixels(layout, m, &pixels, row, 0)
                .iter()
                .any(|p| *p != bg && *p != bgra(palette::TAB_BAR))
        };
        assert!(has_text(0), "AAAA se queda");
        // Fila 1: el resumen (fondo propio), no BBBB.
        assert_eq!(
            cell_pixels(layout, m, &pixels, 1, 0)[0],
            bgra(palette::TAB_BAR)
        );
        // DDDD (fila 3 de la vista) sube a la fila 2 de pantalla; la 3 queda vacía.
        assert!(has_text(2), "DDDD sube");
        assert!(!has_text(3), "el hueco queda vacío");
    }

    #[test]
    fn selection_and_block_marks() {
        let mut term = Term::new(4, 2);
        term.advance(b"\x1b[?25l");
        let overlay = Overlay {
            selection: vec![(0, 1, 2)],
            marks: vec![(1, true)],
            links: vec![(1, 2, 3)],
            blocks: Vec::new(),
            row_map: Default::default(),
        };
        let (layout, m, pixels) = render_with(&term, 2.0, &overlay);
        let sel = bgra(theme().selection);
        let bg = bgra(theme().background);
        assert!(
            cell_pixels(layout, m, &pixels, 0, 1)
                .iter()
                .all(|p| *p == sel)
        );
        assert!(
            cell_pixels(layout, m, &pixels, 0, 3)
                .iter()
                .all(|p| *p == bg)
        );
        // La marca roja está en el margen izquierdo de la fila 1.
        let stride = layout.width_px as usize * 4;
        let y = (layout.pad_px + m.cell_h + m.cell_h / 2) as usize;
        let red = bgra(palette::MARK_FAILED);
        let margin: Vec<[u8; 4]> = (0..layout.pad_px as usize)
            .map(|x| {
                let i = y * stride + x * 4;
                [pixels[i], pixels[i + 1], pixels[i + 2], pixels[i + 3]]
            })
            .collect();
        assert!(margin.contains(&red), "no hay marca roja en el margen");
        // El enlace de la fila 1 está subrayado con el color de acento.
        let accent = bgra(palette::ACCENT);
        assert!(cell_pixels(layout, m, &pixels, 1, 2).contains(&accent));
        assert!(!cell_pixels(layout, m, &pixels, 1, 1).contains(&accent));
    }

    #[test]
    fn combining_marks_are_drawn_on_their_base() {
        let plain = {
            let mut t = Term::new(2, 1);
            t.advance(b"\x1b[?25le");
            let (layout, m, pixels) = render(&t, 2.0);
            cell_pixels(layout, m, &pixels, 0, 0)
        };
        let accented = {
            let mut t = Term::new(2, 1);
            t.advance("\x1b[?25le\u{301}".as_bytes());
            let (layout, m, pixels) = render(&t, 2.0);
            cell_pixels(layout, m, &pixels, 0, 0)
        };
        let lit = |px: &[[u8; 4]]| px.iter().filter(|p| p[1] > 150).count();
        // Misma celda, con más píxeles encendidos: el acento está encima de la "e"
        // (y no un recuadro de glifo inexistente).
        assert!(
            lit(&accented) > lit(&plain),
            "{} vs {}",
            lit(&accented),
            lit(&plain)
        );
        assert!(lit(&accented) < lit(&plain) * 2);
    }

    #[test]
    fn hidden_cursor_is_not_drawn() {
        let mut term = Term::new(3, 1);
        term.advance(b"\x1b[?25l");
        let (layout, m, pixels) = render(&term, 1.0);
        let bg = bgra(theme().background);
        assert!(
            cell_pixels(layout, m, &pixels, 0, 0)
                .iter()
                .all(|p| *p == bg)
        );
    }

    /// Vuelca una pantalla de muestra a un BMP para revisarla a ojo:
    /// `RT_RENDER_DUMP=/tmp/render.bmp cargo test -p rust-terminal dump -- --ignored`
    #[test]
    #[ignore]
    fn dump_sample_screen() {
        let Some(path) = std::env::var_os("RT_RENDER_DUMP") else {
            return;
        };
        let mut term = Term::new(48, 10);
        term.advance(
            concat!(
                "\x1b[1;32m~/rust-terminal\x1b[0m \x1b[34mmain\x1b[0m $ ls -la\r\n",
                "\x1b[1mnegrita\x1b[0m \x1b[3mcursiva\x1b[0m \x1b[4msubrayado\x1b[0m ",
                "\x1b[9mtachado\x1b[0m \x1b[2mtenue\x1b[0m\r\n",
                "\x1b[7m inverso \x1b[0m \x1b[38;2;255;128;0mtruecolor\x1b[0m ",
                "\x1b[38;5;201m256\x1b[0m \x1b[44;97m fondo \x1b[0m\r\n",
                "Acentos: áéíóú ñ ü ç ¿¡ €  CJK: 日本語  Emoji: 🙂🚀\r\n",
                "\x1b(0lqqqqwqqqqk\x1b(B ┌──┬──┐ ╭─╮ █▓▒░ ⠿\r\n",
                "\x1b(0x    x    x\x1b(B │  │  │ ╰─╯ → ✓ ✗ λ\r\n",
                "\x1b(0mqqqqvqqqqj\x1b(B └──┴──┘\r\n",
                "Combinantes: e\u{301} n\u{303} a\u{308} o\u{302} ❤\u{fe0f} 👨\u{200d}💻\r\n",
                "$ "
            )
            .as_bytes(),
        );
        let overlay = Overlay {
            selection: vec![(1, 0, 6), (2, 10, 20)],
            marks: vec![(0, false), (8, true)],
            links: Vec::new(),
            blocks: Vec::new(),
            row_map: Default::default(),
        };
        let (layout, _m, pixels) = render_with(&term, 2.0, &overlay);
        let (w, h) = (layout.width_px as usize, layout.height_px as usize);
        let mut bmp = Vec::with_capacity(54 + w * h * 4);
        let size = (54 + w * h * 4) as u32;
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&size.to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(w as i32).to_le_bytes());
        bmp.extend_from_slice(&(-(h as i32)).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(&pixels);
        std::fs::write(path, bmp).unwrap();
    }

    /// Barra de pestañas y dos paneles, a un BMP para revisarlo a ojo:
    /// `RT_RENDER_DUMP=/tmp/tabs.bmp cargo test -p rust-terminal dump_tabs -- --ignored`
    #[test]
    #[ignore]
    fn dump_tabs_and_panes() {
        let Some(path) = std::env::var_os("RT_RENDER_DUMP") else {
            return;
        };
        let scale = 2.0;
        let device = create_device();
        let mut r = Renderer::new(
            device.clone(),
            MTLPixelFormat::BGRA8Unorm,
            &test_font(),
            theme(),
            scale,
        );
        let m = r.fonts.metrics;
        let (cw, ch) = (m.cell_w as f32, m.cell_h as f32);
        let pad = 12.0;
        let bar_h = (ch * 1.8).round();
        let pane_w = 2.0 * pad + 30.0 * cw;
        let pane_h = 2.0 * pad + 6.0 * ch;
        let (w, h) = (2.0 * pane_w + 2.0, bar_h + pane_h);

        let mut left = Term::new(30, 6);
        left.advance(
            "\x1b[32m~/proyecto\x1b[0m $ cargo test\r\n   Compiling ok\r\ntest result: ok\r\n$ "
                .as_bytes(),
        );
        let mut right = Term::new(30, 6);
        right.advance("$ vim src/main.rs\r\n\x1b[1mfn\x1b[0m main() {}\r\n$ ".as_bytes());
        let mut panes = Vec::new();
        for (id, term, x, focused) in [(1u64, &left, 0.0, true), (2, &right, pane_w + 2.0, false)] {
            r.ensure_pane(id, 30, 6);
            for i in 0..6 {
                r.update_row(id, i, term.grid().row(i));
            }
            panes.push(PaneFrame {
                id,
                rect: [x, bar_h, pane_w, pane_h],
                pad,
                cursor: term.cursor(),
                modes: term.modes(),
                overlay: Overlay::default(),
                focused,
                preedit: (id == 1).then(|| "´".to_owned()),
                dim: !focused,
            });
        }
        let tab_w = (cw * 28.0).floor();
        let close_w = (cw * 2.5).round();
        let tab = |i: usize, title: &str, active: bool| {
            let x = i as f32 * tab_w;
            TabItem {
                title: title.to_owned(),
                rect: [x, 0.0, tab_w, bar_h],
                close: [x + tab_w - close_w, 0.0, close_w, bar_h],
                active,
                busy: false,
            }
        };
        let frame = Frame {
            layout: Layout {
                scale,
                width_px: w as u32,
                height_px: h as u32,
            },
            panes,
            dividers: vec![[pane_w, bar_h, 2.0, pane_h]],
            headers: Vec::new(),
            tab_bar: Some(TabBar {
                rect: [0.0, 0.0, w, bar_h],
                tabs: vec![
                    tab(0, "proyecto", true),
                    tab(1, "un título muy largo que no cabe en la pestaña", false),
                ],
                plus: [2.0 * tab_w, 0.0, bar_h, bar_h],
                spinner_phase: 0,
            }),
        };
        // SAFETY: descriptor 2D estándar.
        let desc = unsafe {
            MTLTextureDescriptor::texture2DDescriptorWithPixelFormat_width_height_mipmapped(
                MTLPixelFormat::BGRA8Unorm,
                w as NSUInteger,
                h as NSUInteger,
                false,
            )
        };
        desc.setUsage(MTLTextureUsage::RenderTarget | MTLTextureUsage::ShaderRead);
        desc.setStorageMode(MTLStorageMode::Shared);
        let texture = device.newTextureWithDescriptor(&desc).unwrap();
        r.build_quads(&frame);
        let cb = r.encode(&texture, frame.layout).unwrap();
        cb.commit();
        cb.waitUntilCompleted();
        let (w, h) = (w as usize, h as usize);
        let mut pixels = vec![0u8; w * h * 4];
        let region = MTLRegion {
            origin: MTLOrigin { x: 0, y: 0, z: 0 },
            size: MTLSize {
                width: w as NSUInteger,
                height: h as NSUInteger,
                depth: 1,
            },
        };
        // SAFETY: el buffer tiene el tamaño de la región completa.
        unsafe {
            texture.getBytes_bytesPerRow_fromRegion_mipmapLevel(
                NonNull::new_unchecked(pixels.as_mut_ptr().cast()),
                (w * 4) as NSUInteger,
                region,
                0,
            );
        }
        write_bmp(&path, w, h, &pixels);
    }

    fn write_bmp(path: &std::ffi::OsStr, w: usize, h: usize, pixels: &[u8]) {
        let mut bmp = Vec::with_capacity(54 + w * h * 4);
        bmp.extend_from_slice(b"BM");
        bmp.extend_from_slice(&((54 + w * h * 4) as u32).to_le_bytes());
        bmp.extend_from_slice(&[0; 4]);
        bmp.extend_from_slice(&54u32.to_le_bytes());
        bmp.extend_from_slice(&40u32.to_le_bytes());
        bmp.extend_from_slice(&(w as i32).to_le_bytes());
        bmp.extend_from_slice(&(-(h as i32)).to_le_bytes());
        bmp.extend_from_slice(&1u16.to_le_bytes());
        bmp.extend_from_slice(&32u16.to_le_bytes());
        bmp.extend_from_slice(&[0; 24]);
        bmp.extend_from_slice(pixels);
        std::fs::write(path, bmp).unwrap();
    }
}
