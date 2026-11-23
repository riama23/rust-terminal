//! Microbenchmark del núcleo: throughput de `Term::advance` sin PTY ni render.
//!
//! `cargo run -p rust-terminal-core --release --bin bench-core`
//!
//! Cada caso genera ~16 MiB de entrada, la procesa varias veces en una
//! terminal de 200×50 y muestra el mejor resultado en MiB/s.

use std::fmt::Write as _;
use std::hint::black_box;
use std::time::{Duration, Instant};

use rust_terminal_core::Term;

const TARGET_BYTES: usize = 16 << 20;
const RUNS: usize = 5;

fn repeat_until_target(chunk: &str) -> Vec<u8> {
    let mut out = String::with_capacity(TARGET_BYTES + chunk.len());
    while out.len() < TARGET_BYTES {
        out.push_str(chunk);
    }
    out.into_bytes()
}

fn plain_text() -> Vec<u8> {
    let line =
        "The quick brown fox jumps over the lazy dog 0123456789 abcdefghijklmnopqrstuvwxyz\r\n";
    repeat_until_target(&line.repeat(64))
}

fn dense_sgr() -> Vec<u8> {
    let mut chunk = String::new();
    for i in 0..2000u32 {
        let _ = write!(
            chunk,
            "\x1b[38;5;{}m\x1b[48;2;{};{};{}mX",
            i % 256,
            i % 255,
            (i * 7) % 255,
            (i * 13) % 255
        );
        if i % 100 == 99 {
            chunk.push_str("\x1b[0m\r\n");
        }
    }
    repeat_until_target(&chunk)
}

fn scroll_region() -> Vec<u8> {
    let mut chunk = String::from("\x1b[5;45r\x1b[45;1H");
    for i in 0..500 {
        let _ = write!(chunk, "line {i} in a scrolling region\r\n");
    }
    chunk.push_str("\x1b[r");
    repeat_until_target(&chunk)
}

fn unicode() -> Vec<u8> {
    let line = "Acentos áéíóú ñ — CJK 日本語の文章 — emoji 🙂🚀✨ — box ┌─┬─┐│ │\r\n";
    repeat_until_target(&line.repeat(32))
}

fn cursor_motion() -> Vec<u8> {
    // Patrón de aplicaciones a pantalla completa (vim, htop): mover y repintar.
    let mut chunk = String::new();
    for row in 1..=50 {
        let _ = write!(chunk, "\x1b[{row};1H\x1b[2K\x1b[1;34m{row:>4}\x1b[0m ");
        chunk.push_str(&"x".repeat(150));
    }
    repeat_until_target(&chunk)
}

fn bench(name: &str, input: &[u8]) {
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let mut term = Term::new(200, 50);
        let start = Instant::now();
        // Lotes de 64 KiB, como el lector de la PTY.
        for chunk in input.chunks(64 * 1024) {
            term.advance(black_box(chunk));
        }
        black_box(term.cursor());
        best = best.min(start.elapsed());
    }
    let mib = input.len() as f64 / (1 << 20) as f64;
    println!(
        "{name:<14} {:>8.1} MiB/s  ({mib:.0} MiB en {:.1} ms)",
        mib / best.as_secs_f64(),
        best.as_secs_f64() * 1000.0
    );
}

/// Reflow con el historial lleno: estrechar y ensanchar la ventana.
fn bench_reflow() {
    let mut term = Term::new(200, 50);
    let line = "palabra ".repeat(20);
    let mut input = String::new();
    for i in 0..(rust_terminal_core::term::HISTORY_LIMIT + 100) {
        let _ = write!(input, "{i:>6} {line}\r\n");
    }
    term.advance(input.as_bytes());
    let history = term.grid().history_len();
    let bytes = term.grid().history_bytes();
    println!(
        "historial      {history} líneas ≈ {:.1} MiB ({} B/línea de ~170 columnas)",
        bytes as f64 / (1 << 20) as f64,
        bytes / history.max(1)
    );
    let mut best = Duration::MAX;
    for _ in 0..RUNS {
        let start = Instant::now();
        term.resize(120, 50);
        term.resize(200, 50);
        best = best.min(start.elapsed() / 2);
    }
    println!(
        "reflow         {:>8.1} ms por cambio de ancho (historial lleno)",
        best.as_secs_f64() * 1000.0
    );
}

fn main() {
    if cfg!(debug_assertions) {
        eprintln!("aviso: compilado sin optimizar; usa --release para medir");
    }
    bench("texto plano", &plain_text());
    bench("SGR denso", &dense_sgr());
    bench("scroll región", &scroll_region());
    bench("unicode", &unicode());
    bench("movimientos", &cursor_motion());
    bench_reflow();
}
