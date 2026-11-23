//! Fuentes con Core Text: métricas de celda y rasterizado de glifos a BGRA.
//!
//! Todas las medidas de este módulo están en píxeles físicos: las fuentes se
//! crean ya escaladas (`tamaño en puntos × backingScaleFactor`).

use std::ptr::NonNull;

use objc2_core_foundation::{CFRange, CFRetained, CFString, CGFloat, CGPoint, CGSize};
use objc2_core_graphics::{
    CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
};
use objc2_core_text::{CTFont, CTFontOrientation, CTFontSymbolicTraits};

/// Tamaño de la fuente en puntos (fijo hasta M7).
pub const FONT_SIZE_PT: CGFloat = 13.0;

/// Estilo tipográfico de una celda.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Style {
    pub bold: bool,
    pub italic: bool,
}

/// Métricas de la celda, en píxeles enteros.
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct Metrics {
    pub cell_w: u32,
    pub cell_h: u32,
    /// Distancia desde la parte superior de la celda hasta la línea base.
    pub baseline: u32,
    /// Grosor de subrayado y tachado.
    pub stroke: u32,
}

impl Metrics {
    pub fn of(font: &CTFont) -> Metrics {
        // SAFETY: llamadas de solo lectura sobre una fuente válida.
        let (ascent, descent, leading, advance) = unsafe {
            let mut glyph = [0u16; 1];
            let mut chars = [u16::from(b'M')];
            font.glyphs_for_characters(
                NonNull::new_unchecked(chars.as_mut_ptr()),
                NonNull::new_unchecked(glyph.as_mut_ptr()),
                1,
            );
            let advance = font.advances_for_glyphs(
                CTFontOrientation::Horizontal,
                NonNull::new_unchecked(glyph.as_mut_ptr()),
                std::ptr::null_mut(),
                1,
            );
            (font.ascent(), font.descent(), font.leading(), advance)
        };
        let cell_h = (ascent + descent + leading).ceil();
        Metrics {
            cell_w: advance.round().max(1.0) as u32,
            cell_h: cell_h.max(1.0) as u32,
            baseline: (ascent + leading / 2.0).round() as u32,
            stroke: (cell_h / 16.0).round().max(1.0) as u32,
        }
    }
}

/// Las cuatro variantes de la fuente principal a un tamaño en píxeles.
pub struct Fonts {
    regular: CFRetained<CTFont>,
    bold: CFRetained<CTFont>,
    italic: CFRetained<CTFont>,
    bold_italic: CFRetained<CTFont>,
    pub metrics: Metrics,
}

// SAFETY: CTFont es inmutable y Core Text permite usarlo desde cualquier hilo.
unsafe impl Send for Fonts {}

/// Un glifo rasterizado, listo para subir al atlas.
pub struct Raster {
    pub width: u32,
    pub height: u32,
    /// BGRA premultiplicado, `width * 4` bytes por fila.
    pub pixels: Vec<u8>,
    /// Glifo con color propio (emoji): no se tiñe con el color del texto.
    pub color: bool,
}

/// Crea la fuente monoespaciada del sistema (SF Mono) a `size` píxeles.
pub fn system_monospace(size: CGFloat) -> CFRetained<CTFont> {
    // `NSFont` es intercambiable con `CTFont` (toll-free bridged).
    // SAFETY: NSFont es seguro para hilos; la conversión no copia.
    let ns = unsafe {
        objc2_app_kit::NSFont::monospacedSystemFontOfSize_weight(
            size,
            objc2_app_kit::NSFontWeightRegular,
        )
    };
    let ct: &CTFont = ns.as_ref();
    // SAFETY: `ct` es una referencia válida; se toma una referencia propia.
    unsafe { CFRetained::retain(NonNull::from(ct)) }
}

fn with_traits(base: &CFRetained<CTFont>, traits: CTFontSymbolicTraits) -> CFRetained<CTFont> {
    // SAFETY: `base` es válida; si no hay variante con esos rasgos se usa la base.
    unsafe {
        base.copy_with_symbolic_traits(0.0, std::ptr::null(), traits, traits)
            .unwrap_or_else(|| base.clone())
    }
}

impl Fonts {
    pub fn new(size_px: CGFloat) -> Fonts {
        let regular = system_monospace(size_px);
        let bold = with_traits(&regular, CTFontSymbolicTraits::TraitBold);
        let italic = with_traits(&regular, CTFontSymbolicTraits::TraitItalic);
        let bold_italic = with_traits(
            &regular,
            CTFontSymbolicTraits::TraitBold | CTFontSymbolicTraits::TraitItalic,
        );
        let metrics = Metrics::of(&regular);
        Fonts {
            regular,
            bold,
            italic,
            bold_italic,
            metrics,
        }
    }

    fn font(&self, style: Style) -> &CFRetained<CTFont> {
        match (style.bold, style.italic) {
            (false, false) => &self.regular,
            (true, false) => &self.bold,
            (false, true) => &self.italic,
            (true, true) => &self.bold_italic,
        }
    }

    /// Busca el glifo de `ch`: primero en la fuente del estilo y, si no lo
    /// tiene, en la que Core Text elija como sustituta.
    fn lookup(&self, ch: char, style: Style) -> (CFRetained<CTFont>, u16) {
        let mut utf16 = [0u16; 2];
        let units = ch.encode_utf16(&mut utf16).len();
        let primary = self.font(style);
        if let Some(glyph) = glyph_in(primary, &mut utf16, units) {
            return (primary.clone(), glyph);
        }
        let s = CFString::from_str(ch.encode_utf8(&mut [0; 4]));
        // SAFETY: rango dentro de la cadena recién creada.
        let fallback = unsafe { primary.for_string(&s, CFRange::new(0, units as isize)) };
        match glyph_in(&fallback, &mut utf16, units) {
            Some(glyph) => (fallback, glyph),
            None => (primary.clone(), 0),
        }
    }

    /// Rasteriza `ch` en un recuadro de `cells` celdas de ancho.
    pub fn rasterize(&self, ch: char, style: Style, cells: u32) -> Raster {
        let (font, glyph) = self.lookup(ch, style);
        let m = self.metrics;
        let width = m.cell_w * cells;
        let height = m.cell_h;
        // SAFETY: lectura de rasgos de una fuente válida.
        let color =
            unsafe { font.symbolic_traits() }.contains(CTFontSymbolicTraits::TraitColorGlyphs);

        let stride = width as usize * 4;
        let mut pixels = vec![0u8; stride * height as usize];
        let space = CGColorSpace::new_device_rgb().expect("espacio de color RGB");
        let info = CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0;
        // SAFETY: el buffer vive más que el contexto y tiene el tamaño declarado.
        let ctx = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                width as usize,
                height as usize,
                8,
                stride,
                Some(&space),
                info,
            )
        }
        .expect("contexto de bitmap");
        CGContext::set_allows_font_smoothing(Some(&ctx), false);
        CGContext::set_should_smooth_fonts(Some(&ctx), false);
        CGContext::set_rgb_fill_color(Some(&ctx), 1.0, 1.0, 1.0, 1.0);

        // Los emoji se escalan para que quepan en el recuadro.
        let mut glyphs = [glyph];
        let origin_y = (m.cell_h - m.baseline) as CGFloat;
        let mut positions = [CGPoint::new(0.0, origin_y)];
        if color {
            let mut advance = CGSize::new(0.0, 0.0);
            // SAFETY: un glifo y un avance.
            unsafe {
                font.advances_for_glyphs(
                    CTFontOrientation::Horizontal,
                    NonNull::new_unchecked(glyphs.as_mut_ptr()),
                    &mut advance,
                    1,
                );
            }
            let scale = if advance.width > width as CGFloat {
                width as CGFloat / advance.width
            } else {
                1.0
            };
            CGContext::scale_ctm(Some(&ctx), scale, scale);
            positions[0].y /= scale;
        }
        // SAFETY: arrays de un elemento y contexto válido.
        unsafe {
            font.draw_glyphs(
                NonNull::new_unchecked(glyphs.as_mut_ptr()),
                NonNull::new_unchecked(positions.as_mut_ptr()),
                1,
                &ctx,
            );
        }
        drop(ctx);
        Raster {
            width,
            height,
            pixels,
            color,
        }
    }
}

fn glyph_in(font: &CTFont, utf16: &mut [u16; 2], units: usize) -> Option<u16> {
    let mut glyphs = [0u16; 2];
    // SAFETY: `utf16` y `glyphs` tienen al menos `units` elementos.
    let found = unsafe {
        font.glyphs_for_characters(
            NonNull::new_unchecked(utf16.as_mut_ptr()),
            NonNull::new_unchecked(glyphs.as_mut_ptr()),
            units as isize,
        )
    };
    (found && glyphs[0] != 0).then_some(glyphs[0])
}
