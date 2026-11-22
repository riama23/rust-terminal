# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Estado del proyecto

Crate binario `rust-terminal` (Rust edition 2024). Todavía solo contiene el `src/main.rs` por defecto y no tiene dependencias. El diseño de abajo está acordado con el usuario; los temas de "Abierto" no se deciden sin preguntar.

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
- **Stack nativo, solo para macOS:** AppKit mediante `objc2`, Metal sobre `CAMetalLayer`, Core Text para las fuentes. El núcleo (parser y estado) se mantiene separado del renderizado.
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
- Tests: `cargo test` · un solo test: `cargo test <nombre_del_test>`
- Lint: `cargo clippy --all-targets`
- Formato: `cargo fmt` (comprobar sin modificar: `cargo fmt --check`)

Rust está instalado con rustup en `~/.cargo/bin`. Si `cargo` no aparece en el PATH de un shell no interactivo, ejecuta antes `. "$HOME/.cargo/env"`.
