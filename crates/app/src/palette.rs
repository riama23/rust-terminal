//! Colores: paleta xterm de 256 entradas y colores por defecto (fijos hasta M7).

use rust_terminal_core::Color;

/// RGBA en `[0, 1]`.
pub type Rgba = [f32; 4];

pub const DEFAULT_FG: [u8; 3] = [0xd8, 0xd8, 0xd8];
pub const DEFAULT_BG: [u8; 3] = [0x1c, 0x1c, 0x1e];
pub const CURSOR: [u8; 3] = [0xd8, 0xd8, 0xd8];

/// Los 16 colores ANSI (esquema de xterm, algo suavizado).
const ANSI: [[u8; 3]; 16] = [
    [0x00, 0x00, 0x00],
    [0xcd, 0x31, 0x31],
    [0x0d, 0xbc, 0x79],
    [0xe5, 0xe5, 0x10],
    [0x24, 0x72, 0xc8],
    [0xbc, 0x3f, 0xbc],
    [0x11, 0xa8, 0xcd],
    [0xe5, 0xe5, 0xe5],
    [0x66, 0x66, 0x66],
    [0xf1, 0x4c, 0x4c],
    [0x23, 0xd1, 0x8b],
    [0xf5, 0xf5, 0x43],
    [0x3b, 0x8e, 0xea],
    [0xd6, 0x70, 0xd6],
    [0x29, 0xb8, 0xdb],
    [0xff, 0xff, 0xff],
];

/// Color de la paleta de 256: 16 ANSI, cubo 6×6×6 y 24 grises.
pub fn indexed(i: u8) -> [u8; 3] {
    match i {
        0..=15 => ANSI[i as usize],
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

pub fn rgb(color: Color, default: [u8; 3]) -> [u8; 3] {
    match color {
        Color::Default => default,
        Color::Indexed(i) => indexed(i),
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

    #[test]
    fn cube_and_grayscale() {
        assert_eq!(indexed(16), [0, 0, 0]);
        assert_eq!(indexed(231), [255, 255, 255]);
        assert_eq!(indexed(196), [255, 0, 0]);
        assert_eq!(indexed(232), [8, 8, 8]);
        assert_eq!(indexed(255), [238, 238, 238]);
    }
}
