//! Generador del icono de la app (solo en tests). No forma parte del binario:
//! `scripts/make-icon.sh` lo ejecuta y produce `crates/app/assets/AppIcon.icns`.

#[cfg(test)]
mod tests {
    use std::ptr::NonNull;

    use objc2_core_foundation::{CFRetained, CGFloat, CGPoint};
    use objc2_core_graphics::{
        CGBitmapContextCreate, CGColorSpace, CGContext, CGImageAlphaInfo, CGImageByteOrderInfo,
    };
    use objc2_core_text::CTFont;

    const SIZE: usize = 1024;

    fn crc32(data: &[u8]) -> u32 {
        let mut crc = !0u32;
        for &b in data {
            crc ^= b as u32;
            for _ in 0..8 {
                crc = if crc & 1 != 0 {
                    (crc >> 1) ^ 0xEDB8_8320
                } else {
                    crc >> 1
                };
            }
        }
        !crc
    }

    /// PNG RGBA sin comprimir (deflate con bloques "stored"), sin dependencias.
    fn encode_png(bgra_premultiplied: &[u8], w: usize, h: usize) -> Vec<u8> {
        let mut raw = Vec::with_capacity((w * 4 + 1) * h);
        for row in bgra_premultiplied.chunks(w * 4) {
            raw.push(0); // sin filtro
            for px in row.chunks(4) {
                let a = px[3] as u32;
                let un =
                    |c: u8| ((c as u32 * 255 + a / 2).checked_div(a).unwrap_or(0)).min(255) as u8;
                raw.extend_from_slice(&[un(px[2]), un(px[1]), un(px[0]), px[3]]);
            }
        }
        let mut z = vec![0x78, 0x01];
        for (i, block) in raw.chunks(65_535).enumerate() {
            let last = (i + 1) * 65_535 >= raw.len();
            z.push(last as u8);
            let len = block.len() as u16;
            z.extend_from_slice(&len.to_le_bytes());
            z.extend_from_slice(&(!len).to_le_bytes());
            z.extend_from_slice(block);
        }
        let (mut a, mut b) = (1u32, 0u32);
        for &x in &raw {
            a = (a + x as u32) % 65_521;
            b = (b + a) % 65_521;
        }
        z.extend_from_slice(&((b << 16) | a).to_be_bytes());

        let mut png = b"\x89PNG\r\n\x1a\n".to_vec();
        let mut chunk = |kind: &[u8], data: &[u8]| {
            png.extend_from_slice(&(data.len() as u32).to_be_bytes());
            let mut c = kind.to_vec();
            c.extend_from_slice(data);
            png.extend_from_slice(&c);
            png.extend_from_slice(&crc32(&c).to_be_bytes());
        };
        let mut ihdr = Vec::new();
        ihdr.extend_from_slice(&(w as u32).to_be_bytes());
        ihdr.extend_from_slice(&(h as u32).to_be_bytes());
        ihdr.extend_from_slice(&[8, 6, 0, 0, 0]);
        chunk(b"IHDR", &ihdr);
        chunk(b"IDAT", &z);
        chunk(b"IEND", &[]);
        png
    }

    fn rounded_rect(ctx: &CGContext, x: CGFloat, y: CGFloat, w: CGFloat, h: CGFloat, r: CGFloat) {
        let c = Some(ctx);
        CGContext::begin_path(c);
        CGContext::move_to_point(c, x + r, y);
        CGContext::add_arc_to_point(c, x + w, y, x + w, y + h, r);
        CGContext::add_arc_to_point(c, x + w, y + h, x, y + h, r);
        CGContext::add_arc_to_point(c, x, y + h, x, y, r);
        CGContext::add_arc_to_point(c, x, y, x + w, y, r);
        CGContext::close_path(c);
    }

    fn draw_char(ctx: &CGContext, font: &CFRetained<CTFont>, ch: char, x: CGFloat, y: CGFloat) {
        let mut utf16 = [0u16; 2];
        ch.encode_utf16(&mut utf16);
        let mut glyph = [0u16; 2];
        // SAFETY: buffers de dos elementos y fuente válida.
        unsafe {
            font.glyphs_for_characters(
                NonNull::new_unchecked(utf16.as_mut_ptr()),
                NonNull::new_unchecked(glyph.as_mut_ptr()),
                1,
            );
            let mut pos = [CGPoint::new(x, y)];
            font.draw_glyphs(
                NonNull::new_unchecked(glyph.as_mut_ptr()),
                NonNull::new_unchecked(pos.as_mut_ptr()),
                1,
                ctx,
            );
        }
    }

    /// `RT_ICON_OUT=/tmp/icon.png cargo test -p rust-terminal icon -- --ignored`
    #[test]
    #[ignore]
    fn render_app_icon() {
        let Some(out) = std::env::var_os("RT_ICON_OUT") else {
            return;
        };
        let stride = SIZE * 4;
        let mut pixels = vec![0u8; stride * SIZE];
        let space = CGColorSpace::new_device_rgb().unwrap();
        let info = CGImageByteOrderInfo::Order32Little.0 | CGImageAlphaInfo::PremultipliedFirst.0;
        // SAFETY: el buffer vive más que el contexto y mide lo declarado.
        let ctx = unsafe {
            CGBitmapContextCreate(
                pixels.as_mut_ptr().cast(),
                SIZE,
                SIZE,
                8,
                stride,
                Some(&space),
                info,
            )
        }
        .unwrap();
        let c = Some(&*ctx);

        // Rejilla de iconos de macOS: cuadrado de 824 px centrado con esquinas de ~185 px.
        let (m, s) = (100.0, 824.0);
        CGContext::set_rgb_fill_color(c, 0.07, 0.07, 0.08, 1.0);
        rounded_rect(&ctx, m, m, s, s, 185.0);
        CGContext::fill_path(c);
        // Borde fino.
        CGContext::set_rgb_stroke_color(c, 0.30, 0.30, 0.34, 1.0);
        CGContext::set_line_width(c, 6.0);
        rounded_rect(&ctx, m + 3.0, m + 3.0, s - 6.0, s - 6.0, 182.0);
        CGContext::stroke_path(c);

        // El prompt: ">" azul y "_" claro, en SF Mono negrita.
        // SAFETY: NSFont es intercambiable con CTFont.
        let font: CFRetained<CTFont> = unsafe {
            let ns = objc2_app_kit::NSFont::monospacedSystemFontOfSize_weight(
                400.0,
                objc2_app_kit::NSFontWeightBold,
            );
            let ct: &CTFont = ns.as_ref();
            CFRetained::retain(NonNull::from(ct))
        };
        CGContext::set_rgb_fill_color(c, 0.23, 0.56, 0.92, 1.0);
        draw_char(&ctx, &font, '>', 230.0, 330.0);
        CGContext::set_rgb_fill_color(c, 0.85, 0.85, 0.87, 1.0);
        draw_char(&ctx, &font, '_', 490.0, 330.0);
        drop(ctx);

        let png = encode_png(&pixels, SIZE, SIZE);
        std::fs::write(out, png).unwrap();
    }
}
