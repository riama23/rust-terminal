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

use crate::font::{Fonts, Style};
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
        }
    }

    fn clear(&mut self) {
        self.cursor_x = 0;
        self.cursor_y = 0;
        self.slots.clear();
    }

    fn get(&mut self, fonts: &Fonts, ch: char, style: Style, cells: u32) -> Option<Slot> {
        if let Some(slot) = self.slots.get(&(ch, style)) {
            return *slot;
        }
        let raster = fonts.rasterize(ch, style, cells);
        let slot = if raster.pixels.as_chunks::<4>().0.iter().all(|p| p[3] == 0) {
            None
        } else {
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
        };
        self.slots.insert((ch, style), slot);
        slot
    }
}

/// Lo que el renderizador necesita saber del layout de la vista.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Layout {
    pub scale: f64,
    pub width_px: u32,
    pub height_px: u32,
    pub pad_px: u32,
}

pub struct Renderer {
    device: Retained<ProtocolObject<dyn MTLDevice>>,
    queue: Retained<ProtocolObject<dyn MTLCommandQueue>>,
    pipeline: Retained<ProtocolObject<dyn MTLRenderPipelineState>>,
    atlas: Atlas,
    fonts: Fonts,
    scale: f64,
    /// Copia local de la pantalla.
    rows: Vec<Vec<Cell>>,
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
            fonts: Fonts::new(crate::font::FONT_SIZE_PT * scale),
            scale,
            device,
            queue,
            pipeline,
            rows: Vec::new(),
            quads: Vec::new(),
        }
    }

    /// Cambia de escala (pantalla Retina o no): nuevas fuentes y atlas vacío.
    pub fn set_scale(&mut self, scale: f64) {
        if scale != self.scale {
            self.scale = scale;
            self.fonts = Fonts::new(crate::font::FONT_SIZE_PT * scale);
            self.atlas.clear();
        }
    }

    /// Ajusta la copia local al tamaño de la cuadrícula del núcleo. Devuelve
    /// `true` si cambió, y entonces hay que copiar todas las filas.
    pub fn ensure_size(&mut self, cols: usize, rows: usize) -> bool {
        if self.rows.len() != rows || self.rows.first().is_some_and(|r| r.len() != cols) {
            self.rows = vec![vec![Cell::default(); cols]; rows];
            true
        } else {
            false
        }
    }

    /// Actualiza una fila de la copia local (se llama con el bloqueo tomado).
    pub fn update_row(&mut self, i: usize, row: &[Cell]) {
        self.rows[i].copy_from_slice(row);
    }

    /// Construye un frame y lo presenta en el siguiente drawable del layer.
    pub fn draw(&mut self, layer: &CAMetalLayer, layout: Layout, cursor: Cursor, modes: Modes) {
        self.build_quads(layout, cursor, modes);
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
        let bg = palette::to_rgba(palette::DEFAULT_BG, 1.0);
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

    fn build_quads(&mut self, layout: Layout, cursor: Cursor, modes: Modes) {
        self.quads.clear();
        let m = self.fonts.metrics;
        let (cw, ch) = (m.cell_w as f32, m.cell_h as f32);
        let pad = layout.pad_px as f32;
        let mut glyphs: Vec<Quad> = Vec::new();

        for (r, row) in self.rows.iter().enumerate() {
            let y = pad + r as f32 * ch;
            for (c, cell) in row.iter().enumerate() {
                if cell.flags.contains(Flags::WIDE_SPACER) {
                    continue;
                }
                let cells = if cell.flags.contains(Flags::WIDE) {
                    2
                } else {
                    1
                };
                let x = pad + c as f32 * cw;
                let w = cw * cells as f32;

                let mut fg = palette::rgb(cell.fg, palette::DEFAULT_FG);
                let mut bg = palette::rgb(cell.bg, palette::DEFAULT_BG);
                if cell.flags.contains(Flags::INVERSE) {
                    std::mem::swap(&mut fg, &mut bg);
                }
                let is_cursor = modes.cursor_visible && cursor.row == r && cursor.col == c;
                if is_cursor {
                    fg = bg;
                    bg = palette::CURSOR;
                }
                if bg != palette::DEFAULT_BG {
                    self.quads
                        .push(fill([x, y, w, ch], palette::to_rgba(bg, 1.0)));
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
                    if let Some(slot) = self.atlas.get(&self.fonts, cell.ch, style, cells) {
                        glyphs.push(Quad {
                            rect: [x, y, slot.w as f32, slot.h as f32],
                            uv: [slot.x as f32, slot.y as f32, slot.w as f32, slot.h as f32],
                            color: fg_rgba,
                            kind: if slot.color {
                                KIND_COLOR_GLYPH
                            } else {
                                KIND_GLYPH
                            },
                            _pad: [0; 3],
                        });
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
        self.quads.extend(glyphs);
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

    /// Pinta `term` en una textura fuera de pantalla y devuelve los píxeles BGRA.
    fn render(term: &Term, scale: f64) -> (Layout, crate::font::Metrics, Vec<u8>) {
        let device = create_device();
        let mut r = Renderer::new(device.clone(), MTLPixelFormat::BGRA8Unorm, scale);
        let m = r.fonts.metrics;
        let grid = term.grid();
        r.ensure_size(grid.cols(), grid.rows());
        for i in 0..grid.rows() {
            r.update_row(i, grid.row(i));
        }
        let pad = 4;
        let layout = Layout {
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

        r.build_quads(layout, term.cursor(), term.modes());
        let cb = r.encode(&texture, layout).unwrap();
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
        layout: Layout,
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
        let bg = bgra(palette::DEFAULT_BG);

        let glyph = cell_pixels(layout, m, &pixels, 0, 0);
        assert!(
            glyph.iter().any(|p| p[1] > 150),
            "la M no tiene píxeles claros"
        );
        assert!(glyph.contains(&bg), "la celda de la M no conserva el fondo");

        let red = bgra(palette::rgb(Color::Indexed(1), palette::DEFAULT_BG));
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
        let cursor = bgra(palette::CURSOR);
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
    fn hidden_cursor_is_not_drawn() {
        let mut term = Term::new(3, 1);
        term.advance(b"\x1b[?25l");
        let (layout, m, pixels) = render(&term, 1.0);
        let bg = bgra(palette::DEFAULT_BG);
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
        let mut term = Term::new(48, 9);
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
                "$ "
            )
            .as_bytes(),
        );
        let (layout, _m, pixels) = render(&term, 2.0);
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
}
