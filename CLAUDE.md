# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Estado del proyecto

Workspace de Rust (edition 2024) con dos crates. Sus nombres son provisionales hasta que se decida el nombre del producto:

- `crates/core` → `rust-terminal-core`: PTY (`pty`), cuadrícula (`grid`) y estado más parser `vte` (`term`). No puede depender de nada de interfaz ni de macOS más allá de POSIX.
- `crates/app` → `rust-terminal`: el binario. En M1 es solo una prueba de humo sin interfaz; en M2 pasa a ser la app de AppKit y Metal.

El diseño de abajo está acordado con el usuario; los temas de "Abierto" no se deciden sin preguntar.

**Hoja de ruta de la v1:**
- **M1, hecho:** núcleo con PTY, `vte` y cuadrícula.
- **M2:** ventana con AppKit, Metal y Core Text; teclado, redimensionado y pantalla alternativa.
- **M3:** hilos, `CADisplayLink` y benchmarks.
- **M4:** scrollback y bloques con OSC 133 y zsh.
- **M5:** pestañas y paneles.
- **M6:** DEC 2026, Kitty keyboard, OSC 8 y 52, y notificaciones.
- **M7:** configuración TOML.

**Pendiente en el núcleo:**
- Todos los caracteres ocupan 1 celda; los anchos (CJK y emoji) se tratan en M2.
- Los CSI con intermedios, incluidos los modos privados `CSI ?`, se ignoran.
- ED 3 se ignora hasta que exista el scrollback.
- macOS no acepta `O_CLOEXEC` en `posix_openpt`, así que se pone después con `fcntl`. Con varias sesiones (M5), un `fork` concurrente podría heredar el maestro de otra; hay que revisarlo entonces.

## Qué es

Un **emulador de terminal para macOS** escrito en Rust (no una shell: por dentro se ejecuta zsh u otra shell existente). Tiene dos objetivos, con este orden de prioridad:

1. **Buen anfitrión para agentes con interfaz de terminal**, como Claude Code. Si algo choca o se complica, gana este objetivo.
2. **Controlable por agentes**: abrir sesiones, enviar teclas, leer pantalla y bloques como datos estructurados. Llega en la **v2**.

## Rendimiento

- **Métricas principales:** latencia de entrada y throughput. Objetivo: igual o mejor que **Ghostty y Alacritty**, medido en el Mac del usuario con Typometer (latencia) y `vtebench` (throughput).
- **Restricciones que no deben empeorar:** memoria con scrollback grande (el límite se fija tras la primera medición) y energía en reposo. **En reposo y sin cambios en pantalla no se renderiza ningún frame.**
- **Fuera del foco:** el tiempo de arranque.
- Los benchmarks viven en el repo y se ejecutan en local con un solo comando; no se ejecutan en CI.

## Arquitectura acordada

- **Parser:** crate `vte`. La **cuadrícula, el estado y el scrollback son propios**, porque es donde se deciden la memoria, los bloques y la futura API para agentes. No usar `alacritty_terminal`.
- **Stack nativo, solo para macOS:** AppKit mediante `objc2`, Metal sobre `CAMetalLayer`, Core Text para las fuentes. Solo hay Command Line Tools, sin Xcode completo, así que los shaders MSL se compilan en runtime con `newLibraryWithSource`, no con `xcrun metal`. El núcleo (parser y estado) se mantiene separado del renderizado.
- **Hilos:** uno de lectura y parseo de la PTY por sesión, y uno de renderizado por ventana sincronizado con `CADisplayLink`. Sin `tokio`. El lector parsea en lotes y suelta el bloqueo del estado en cada frame. Solo se pide un frame cuando algo cambia.
- **Bloques:** cada comando, su salida y su código de salida se obtienen de las marcas OSC 133. El modelo de datos se diseña pensando en exponerlo a agentes en la v2.
- **Integración con la shell:** inyección automática para zsh (mediante `ZDOTDIR`, respetando la configuración del usuario), con opción para desactivarla. Si no llegan las marcas, la terminal funciona sin bloques: **nunca se intenta adivinar los límites de un comando a partir del texto**.
- **Entorno:** `TERM=xterm-256color` y `COLORTERM=truecolor`. Las funciones modernas se detectan con consultas en tiempo de ejecución, no con terminfo.
- **Configuración:** archivo TOML en `~/.config/<nombre>/config.toml`, con recarga en caliente de lo que pueda aplicarse en vivo y buenos valores por defecto. La documentación completa de las opciones se genera a partir de la misma definición, para que no se desfase.

## Alcance de la v1

- Emulador de uso diario: pestañas, paneles divididos y bloques.
- Funciones de buen anfitrión: *synchronized output* (DEC mode 2026), Kitty keyboard protocol, notificaciones nativas, OSC 8, OSC 52 y scrollback grande.
- Batería de benchmarks.
- **Fuera de la v1:** control por agentes, IA integrada, editor de entrada propio, cuenta o sincronización en la nube, ventana de ajustes, paleta de comandos y temas avanzados.
- **Compatibilidad**, validada con programas reales: Claude Code, `vim`/`nvim`, `tmux`, `less`, `htop`, `git` (paginador y colores) y `ssh`. `vttest` sirve para depurar, no como objetivo de conformidad.
- **Distribución:** uso personal, compilada desde el código como un `.app` local sin notarizar. Solo arm64, con macOS 27 como mínimo.
- **Terminada cuando:** el usuario sustituye a Warp durante una semana, incluido el trabajo con Claude Code, sin volver a abrirlo.

## Abierto (no decidir sin preguntar)

- Nombre del producto y, con él, la ruta de configuración.
- API de control por agentes (v2): si será CLI, socket o MCP; modelo de seguridad; estabilidad de la API.
- Límite concreto de memoria del scrollback.
- Terminfo propio, integración con bash y fish, ventana de ajustes, paleta de comandos y temas avanzados.
- Licencia, código abierto y distribución pública: firma, notarización, Homebrew, actualizaciones, soporte para Intel y para versiones anteriores de macOS.

## Comandos

- Compilar: `cargo build`
- Ejecutar: `cargo run`
- Tests: `cargo test` · un crate: `cargo test -p rust-terminal-core` · un solo test: `cargo test <nombre_del_test>`. Los tests de `crates/core/tests/pty.rs` lanzan procesos reales (`/bin/sh`) en una PTY.
- Lint: `cargo clippy --all-targets`
- Formato: `cargo fmt` (comprobar sin modificar: `cargo fmt --check`)

Rust está instalado con rustup en `~/.cargo/bin`. Si `cargo` no aparece en el PATH de un shell no interactivo, ejecuta antes `. "$HOME/.cargo/env"`.
