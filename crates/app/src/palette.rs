//! Colores: los del tema vienen de la configuración (`[colors]`); aquí está
//! la paleta extendida de xterm (16–255) y los colores fijos de la interfaz.

use rust_terminal_core::Color;

use crate::config::Theme;

/// RGBA en `[0, 1]`.
pub type Rgba = [f32; 4];

use std::sync::atomic::{AtomicBool, Ordering};

use objc2::rc::Retained;
use objc2_app_kit::{NSAppearance, NSAppearanceNameAqua, NSAppearanceNameDarkAqua};

/// ¿Está activo el tema claro (Paper)? Lo cambia la vista al aplicar la
/// configuración; lo leen las vistas AppKit y el hilo de render.
static PAPER: AtomicBool = AtomicBool::new(false);

pub fn set_paper(on: bool) {
    PAPER.store(on, Ordering::Relaxed);
}

pub fn is_paper() -> bool {
    PAPER.load(Ordering::Relaxed)
}

/// La apariencia de AppKit que va con el tema (para los controles nativos).
pub fn ui_appearance() -> Option<Retained<NSAppearance>> {
    // SAFETY: constantes de AppKit.
    NSAppearance::appearanceNamed(unsafe {
        if is_paper() {
            NSAppearanceNameAqua
        } else {
            NSAppearanceNameDarkAqua
        }
    })
}

/// Tokens de color del spec de producto, uno por tema. Los nombres son los
/// del spec. Los de Paper `sidebar`, `attention` y `prod` son propuestas del
/// spec (las pantallas de agentes se dibujaron solo en Dusk).
#[allow(dead_code)] // los tokens que aún no usa ninguna vista llegan con las pantallas del spec
pub struct Tokens {
    pub bg: [u8; 3],
    pub chrome: [u8; 3],
    pub sidebar: [u8; 3],
    pub surface: [u8; 3],
    pub popover: [u8; 3],
    pub line: [u8; 3],
    pub line_strong: [u8; 3],
    pub text: [u8; 3],
    pub muted: [u8; 3],
    pub ghost: [u8; 3],
    pub accent: [u8; 3],
    pub ok: [u8; 3],
    pub error: [u8; 3],
    pub attention: [u8; 3],
    pub warn: [u8; 3],
    pub prod: [u8; 3],
}

pub const DUSK: Tokens = Tokens {
    bg: [0x13, 0x14, 0x17],
    chrome: [0x1b, 0x1c, 0x20],
    sidebar: [0x17, 0x18, 0x1c],
    surface: [0x1c, 0x1e, 0x23],
    popover: [0x20, 0x22, 0x28],
    line: [0x2b, 0x2e, 0x35],
    line_strong: [0x3a, 0x3e, 0x47],
    text: [0xe8, 0xe8, 0xea],
    muted: [0xa0, 0xa4, 0xad],
    ghost: [0x6f, 0x74, 0x80],
    accent: [0x7a, 0xa2, 0xff],
    ok: [0x5f, 0xd3, 0x8d],
    error: [0xff, 0x7b, 0x72],
    attention: [0xf0, 0xb4, 0x5a],
    warn: [0xe6, 0xc0, 0x7b],
    prod: [0xff, 0xb2, 0x7a],
};

pub const PAPER_TOKENS: Tokens = Tokens {
    bg: [0xfb, 0xfb, 0xfa],
    chrome: [0xef, 0xef, 0xed],
    sidebar: [0xf4, 0xf4, 0xf2],
    surface: [0xff, 0xff, 0xff],
    popover: [0xff, 0xff, 0xff],
    line: [0xe1, 0xe2, 0xe5],
    line_strong: [0xcf, 0xd2, 0xd8],
    text: [0x1c, 0x1d, 0x21],
    muted: [0x55, 0x5a, 0x63],
    ghost: [0x9a, 0x9e, 0xa6],
    accent: [0x2f, 0x5f, 0xd0],
    // `ok` y `attention` son más oscuros que los del spec (#1F8A4C, #A86400):
    // con ellos el texto no llega a 4,5:1 sobre el fondo y la barra lateral.
    ok: [0x1b, 0x7f, 0x45],
    error: [0xc8, 0x37, 0x2d],
    attention: [0x9a, 0x5a, 0x00],
    warn: [0x8a, 0x61, 0x00],
    prod: [0xa1, 0x4a, 0x00],
};

/// Los tokens del tema activo.
pub fn tokens() -> &'static Tokens {
    if is_paper() { &PAPER_TOKENS } else { &DUSK }
}

/// Los tokens por nombre, como funciones: `tok::bg()`, `tok::text()`…
#[allow(dead_code)]
pub mod tok {
    macro_rules! token_fns {
        ($($name:ident),*) => {
            $(pub fn $name() -> [u8; 3] { super::tokens().$name })*
        };
    }
    token_fns!(
        bg,
        chrome,
        sidebar,
        surface,
        popover,
        line,
        line_strong,
        text,
        muted,
        ghost,
        accent,
        ok,
        error,
        attention,
        warn,
        prod
    );
}

/// Marca de bloque en el margen: comando terminado bien (o aún en curso).
pub fn mark_ok() -> [u8; 3] {
    if is_paper() {
        [0xa8, 0xad, 0xb5]
    } else {
        [0x5a, 0x5a, 0x60]
    }
}
/// Marca de bloque en el margen: comando con código de salida distinto de 0.
pub fn mark_failed() -> [u8; 3] {
    if is_paper() {
        PAPER_TOKENS.error
    } else {
        [0xe0, 0x4f, 0x4f]
    }
}
/// Divisores entre paneles y separadores de pestañas.
pub fn divider() -> [u8; 3] {
    if is_paper() {
        PAPER_TOKENS.line_strong
    } else {
        [0x3a, 0x3a, 0x40]
    }
}
/// Fondo de la barra de pestañas y de las pestañas inactivas.
pub fn tab_bar() -> [u8; 3] {
    if is_paper() {
        PAPER_TOKENS.chrome
    } else {
        [0x12, 0x12, 0x14]
    }
}
pub fn tab_inactive_fg() -> [u8; 3] {
    if is_paper() {
        PAPER_TOKENS.muted
    } else {
        [0x90, 0x90, 0x98]
    }
}
/// Línea de la pestaña activa y subrayado de enlaces.
pub fn accent() -> [u8; 3] {
    if is_paper() {
        PAPER_TOKENS.accent
    } else {
        [0x3b, 0x8e, 0xea]
    }
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
mod token_tests {
    use super::{DUSK, PAPER_TOKENS, Tokens};

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

    fn check(name: &str, t: &Tokens) {
        for (what, fg, bg) in [
            ("text/bg", t.text, t.bg),
            ("text/sidebar", t.text, t.sidebar),
            ("muted/sidebar", t.muted, t.sidebar),
            ("muted/bg", t.muted, t.bg),
            ("attention/sidebar", t.attention, t.sidebar),
            ("accent/sidebar", t.accent, t.sidebar),
            ("ok/sidebar", t.ok, t.sidebar),
            ("error/sidebar", t.error, t.sidebar),
            ("accent/popover", t.accent, t.popover),
            ("text/popover", t.text, t.popover),
        ] {
            let c = contrast(fg, bg);
            assert!(c >= 4.5, "{name} {what}: {c:.2}:1");
        }
    }

    /// El spec exige 4,5:1 para el texto sobre su fondo, en los dos temas.
    #[test]
    fn el_texto_cumple_el_contraste_del_spec_en_dusk_y_en_paper() {
        check("dusk", &DUSK);
        check("paper", &PAPER_TOKENS);
    }

    #[test]
    fn paper_es_claro_y_dusk_es_oscuro() {
        assert!(lum(PAPER_TOKENS.bg) > 0.9);
        assert!(lum(DUSK.bg) < 0.05);
    }
}
