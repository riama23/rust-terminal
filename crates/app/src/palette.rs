//! Colores: los del tema vienen de la configuración (`[colors]`); aquí está
//! la paleta extendida de xterm (16–255) y los colores fijos de la interfaz.

use rust_terminal_core::Color;

use crate::config::Theme;

/// RGBA en `[0, 1]`.
pub type Rgba = [f32; 4];

/// Marca de bloque en el margen: comando terminado bien (o aún en curso).
pub const MARK_OK: [u8; 3] = [0x5a, 0x5a, 0x60];
/// Marca de bloque en el margen: comando con código de salida distinto de 0.
pub const MARK_FAILED: [u8; 3] = [0xe0, 0x4f, 0x4f];
/// Divisores entre paneles y separadores de pestañas.
pub const DIVIDER: [u8; 3] = [0x3a, 0x3a, 0x40];
/// Fondo de la barra de pestañas y de las pestañas inactivas.
pub const TAB_BAR: [u8; 3] = [0x12, 0x12, 0x14];
pub const TAB_INACTIVE_FG: [u8; 3] = [0x90, 0x90, 0x98];
/// Línea de la pestaña activa y subrayado de enlaces.
pub const ACCENT: [u8; 3] = [0x3b, 0x8e, 0xea];

/// Color de la paleta de 256: los 16 del tema, cubo 6×6×6 y 24 grises.
pub fn indexed(i: u8, theme: &Theme) -> [u8; 3] {
    match i {
        0..=15 => theme.palette[i as usize],
        16..=231 => {
            let i = i - 16;
            let level = |v: u8| if v == 0 { 0 } else { 55 + v * 40 };
            [level(i / 36), level((i / 6) % 6), level(i % 6)]
        }
        232..=255 => {
            let v = 8 + (i - 232) * 10;
            [v, v, v]
        }
    }
}

pub fn rgb(color: Color, default: [u8; 3], theme: &Theme) -> [u8; 3] {
    match color {
        Color::Default => default,
        Color::Indexed(i) => indexed(i, theme),
        Color::Rgb(r, g, b) => [r, g, b],
    }
}

pub fn to_rgba(c: [u8; 3], alpha: f32) -> Rgba {
    [
        c[0] as f32 / 255.0,
        c[1] as f32 / 255.0,
        c[2] as f32 / 255.0,
        alpha,
    ]
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::config::Config;

    #[test]
    fn cube_and_grayscale() {
        let t = Config::default().theme;
        assert_eq!(indexed(1, &t), t.palette[1]);
        assert_eq!(indexed(16, &t), [0, 0, 0]);
        assert_eq!(indexed(231, &t), [255, 255, 255]);
        assert_eq!(indexed(196, &t), [255, 0, 0]);
        assert_eq!(indexed(232, &t), [8, 8, 8]);
        assert_eq!(indexed(255, &t), [238, 238, 238]);
    }
}
