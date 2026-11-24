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

/// Tokens de color del spec de producto, tema Dusk. Los nombres son los del
/// spec. Los usan las vistas AppKit (sidebar, attention bar…); los colores de
/// arriba son los de la cuadrícula y las pestañas dibujadas con Metal.
#[allow(dead_code)] // los tokens que aún no usa ninguna vista llegan con las pantallas del spec
pub mod dusk {
    pub const BG: [u8; 3] = [0x13, 0x14, 0x17];
    pub const CHROME: [u8; 3] = [0x1b, 0x1c, 0x20];
    pub const SIDEBAR: [u8; 3] = [0x17, 0x18, 0x1c];
    pub const SURFACE: [u8; 3] = [0x1c, 0x1e, 0x23];
    pub const POPOVER: [u8; 3] = [0x20, 0x22, 0x28];
    pub const LINE: [u8; 3] = [0x2b, 0x2e, 0x35];
    pub const LINE_STRONG: [u8; 3] = [0x3a, 0x3e, 0x47];
    pub const TEXT: [u8; 3] = [0xe8, 0xe8, 0xea];
    pub const MUTED: [u8; 3] = [0xa0, 0xa4, 0xad];
    pub const GHOST: [u8; 3] = [0x6f, 0x74, 0x80];
    pub const ACCENT: [u8; 3] = [0x7a, 0xa2, 0xff];
    pub const OK: [u8; 3] = [0x5f, 0xd3, 0x8d];
    pub const ERROR: [u8; 3] = [0xff, 0x7b, 0x72];
    pub const ATTENTION: [u8; 3] = [0xf0, 0xb4, 0x5a];
    pub const WARN: [u8; 3] = [0xe6, 0xc0, 0x7b];
    pub const PROD: [u8; 3] = [0xff, 0xb2, 0x7a];
}

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

#[cfg(test)]
mod dusk_tests {
    use super::dusk;

    /// Luminancia relativa (WCAG).
    fn lum(c: [u8; 3]) -> f64 {
        let f = |v: u8| {
            let v = v as f64 / 255.0;
            if v <= 0.03928 {
                v / 12.92
            } else {
                ((v + 0.055) / 1.055).powf(2.4)
            }
        };
        0.2126 * f(c[0]) + 0.7152 * f(c[1]) + 0.0722 * f(c[2])
    }

    fn contrast(a: [u8; 3], b: [u8; 3]) -> f64 {
        let (x, y) = (lum(a), lum(b));
        (x.max(y) + 0.05) / (x.min(y) + 0.05)
    }

    /// El spec exige 4,5:1 para el texto sobre su fondo.
    #[test]
    fn el_texto_de_dusk_cumple_el_contraste_del_spec() {
        for (name, fg, bg) in [
            ("text/bg", dusk::TEXT, dusk::BG),
            ("text/sidebar", dusk::TEXT, dusk::SIDEBAR),
            ("muted/sidebar", dusk::MUTED, dusk::SIDEBAR),
            ("muted/bg", dusk::MUTED, dusk::BG),
            ("attention/sidebar", dusk::ATTENTION, dusk::SIDEBAR),
            ("accent/sidebar", dusk::ACCENT, dusk::SIDEBAR),
            ("ok/sidebar", dusk::OK, dusk::SIDEBAR),
            ("error/sidebar", dusk::ERROR, dusk::SIDEBAR),
        ] {
            let c = contrast(fg, bg);
            assert!(c >= 4.5, "{name}: {c:.2}:1");
        }
    }
}
