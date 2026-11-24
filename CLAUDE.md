# CLAUDE.md

This file provides guidance to Claude Code (claude.ai/code) when working with code in this repository.

## Estado del proyecto

Workspace de Rust (edition 2024) con dos crates. Sus nombres son provisionales hasta que se decida el nombre del producto:

- `crates/core` → `rust-terminal-core`: PTY (`pty`), cuadrícula con historial, reflow y filas sucias (`grid`), estado más parser `vte` (`term`), bloques OSC 133 (`block`) y selección (`selection`). No puede depender de nada de interfaz ni de macOS más allá de POSIX. Incluye el binario `bench-core`.
- `crates/app` → `rust-terminal`: la app de AppKit y Metal.
  - `main`: arranque de la app, menús, confirmación al salir y la ventana, que se crea en `applicationDidFinishLaunching:`.
  - `view`: `NSView` con `NSTextInputClient`. Se encarga de las pestañas, los paneles, el teclado, el ratón y el layout.
  - `workspace`: modelo puro de pestañas y árbol de paneles, con tests.
  - `session`: una sesión por panel (PTY, `Term` y el hilo lector).
  - `window`: el hilo de render de la ventana con `CADisplayLink`.
  - `renderer`: Metal, con un atlas de glifos y un draw instanciado de quads.
  - `font`: Core Text.
  - `input`: codificación de teclas, pura y con tests.
  - `palette`: los colores.
  - `config`: tabla única de opciones (`OPTIONS`), de la que salen los valores por defecto, la validación y `docs/configuracion.md`. **Una opción nueva se añade en `OPTIONS` y en `Config::set`.** Después se regenera la documentación con `RT_UPDATE_DOCS=1 cargo test -p rust-terminal config`; un test falla si no se ha hecho.
  - `watcher`: vigila la configuración con kqueue (sin sondeo) y avisa al hilo principal (`configChanged`).
  - `notify`: notificaciones con `UNUserNotificationCenter`. Solo funcionan desde el `.app`; con `cargo run` van a stderr.
  - `shell`: inyección para zsh. Los scripts están en `crates/app/shell/zsh/`.
  - `container`: la vista de contenido de la ventana (`ContainerView`). Reparte el ancho entre un panel izquierdo (sidebar de agentes), la terminal y un panel derecho, y deja una barra bajo la terminal (attention bar). **La `TerminalView` es una subvista: nunca se llega a ella con `window.contentView().downcast::<TerminalView>()`, sino con `container::terminal_view(&window)`.** Un `downcast` a mano falla en silencio (esto ya rompió notificaciones, recarga de configuración y la confirmación de Cmd+Q).
  - `agent/` (v1.2, ver `docs/roadmap.md`): integración con Claude Code, sin AppKit y con tests. `broker` (servidor HTTP síncrono en loopback, token por lanzamiento y clave por agente), `http`, `launch` (flags de `claude`, `--settings` por agente, status line que conserva la del usuario), `model` (estados desde los hooks), `hub` (sesiones y paneles), `rules` (*Siempre permitir*). **Fail safe: ante cualquier duda el broker contesta `{}` y gana el prompt de Claude Code; nunca permite por defecto.**
  - `agent_controller` (pegamento con la `TerminalView`), `agent_ui` (sidebar y attention bar en AppKit), `agent_sheet` (formulario ⌘N).
  - Review (v1.4): `agent/diff` (parser de diffs), `agent/git` (diff del **árbol de trabajo** contra la base común, no `base...HEAD`), `agent/review` (comentarios, mensaje y saneado: **nunca teclear en la PTY sin sanear ni con un permiso pendiente**), `review_ui` y `review_controller` (ventana de Review), y en `agent_ui` el inspector y la barra de terminado.
  - Polish de splits (v1.5, `docs/verificacion-v1.5.md`): `Tab::toggle_zoom`/`resize_focused` en `workspace`, cabeceras (`Geometry.headers`; el rectángulo del panel ya va **sin** la cabecera, todo lo demás lo usa tal cual), atenuado (`PaneFrame.dim`) y spinner de pestañas (`Session.busy` lo pone el lector; `spinnerTick` solo corre mientras haya una pestaña ocupada).
  - Bloques visuales (v1.5): `blocks` (lógica pura: duraciones, relanzar, relanzar fallidos, `archivo:línea`, editor, selección y `RowMap` del colapso), `block_controller` (⌘↑/⌘↓, acciones), `block_ui` (barra bajo la terminal, `Bottom::Block`; manda la de permisos y la de terminado) y el tinte en `Overlay.blocks`. **Colapsar:** `Overlay.row_map` (`blocks::RowMap`) salta filas al pintar y `View::cell_in` lo invierte para el ratón; la rejilla de la terminal no cambia, y cualquier coordenada nueva en filas de la vista tiene que pasar por ese mapa. **Nunca escribir en la PTY un comando de un bloque sin pasar por `blocks::rerun_bytes`.**
  - Paleta de comandos (v1.5, ⌘⇧P): `command_palette` (puro: búsqueda difusa, frecuencia, historial de zsh), `palette_ui` (diálogo AppKit que recibe las teclas él mismo, superpuesto al contenedor) y `palette_controller` (acciones del menú por `sendAction`, historial, flujos de `workflows.toml` y escritura en la PTY). **Escribir en la PTY solo con `command_palette::insertable` y en una shell libre que no sea un agente.**
  - v1.6 (`docs/verificacion-v1.6.md`): `keybindings` (puro: acciones, `[keybindings]` y `Shortcut`; `apply_keybindings` en `main.rs` los pone como equivalentes del menú al arrancar y al recargar), `config_edit` (edición de `config.toml` línea a línea, conserva comentarios) y la ventana de ajustes (`settings_ui`, `settings_controller`), generada desde `config::OPTIONS`: **una opción nueva se añade a una página de `settings_ui::PAGES` (un test lo exige)** y una acción con atajo, a `keybindings::ACTIONS` y a `OPTIONS` (otro test).
  - `agent/alerts` (política de avisos y acciones de notificación, pura), `notify` (categoría con *Permitir*, *Enviar respuesta* y *Abrir*, y su delegado), `hotkey` (⌃⌥N global con Carbon, sin Accesibilidad). El badge del Dock y el icono de la barra de menús se actualizan en `refresh_agent_ui`.
  - **`claude` lanzado a mano** (`agents.track_manual_claude`): cada panel de zsh recibe `RT_CLAUDE_SETTINGS` (los `settings` con los hooks de su clave «dormida», en `AgentController::manual_claude_env`) y `rt-integration.zsh` define una función `claude` que añade `--settings` (salvo subcomandos, `-p`, `--bg` o un `--settings` propio, y sin pisar un `claude` del usuario). El primer evento de esa clave la adopta en la sidebar (`Inner::adopt`); `SessionEnd` la devuelve a «dormida». Respaldo: el lector de la PTY avisa cuando el primer plano pasa a ser Claude (`AppEvent::Claude`, `proc.rs`) y se muestra como «En marcha» (`AgentSession::limited`, sin permisos ni Review).

El diseño de abajo está acordado con el usuario; los temas de "Abierto" no se deciden sin preguntar.

**Hoja de ruta de la v1:**
- **M1, hecho:** núcleo con PTY, `vte` y cuadrícula.
- **M2+M3, fusionados, implementados y en verificación manual** (lista en `docs/verificacion-m2-m3.md`):
  - ventana con AppKit, Metal y Core Text;
  - teclado y redimensionado;
  - pantalla alternativa y modos DEC;
  - hilos con `CADisplayLink` y cero frames en reposo;
  - benchmarks propios;
  - el `.app`.
- **M4, implementado y en verificación manual** (lista en `docs/verificacion-m4.md`):
  - scrollback de 10.000 líneas y reflow;
  - bloques con OSC 133 y OSC 7, e inyección para zsh;
  - selección con el ratón, Cmd+C, Cmd+Shift+C y scroll con el trackpad.
- **M5, implementado y en verificación manual** (lista en `docs/verificacion-m5.md`): barra de pestañas propia, paneles divididos, directorio heredado por OSC 7 y confirmación al cerrar si hay un programa en marcha.
- **M6, implementado y en verificación manual** (lista en `docs/verificacion-m6.md`):
  - DEC 2026 con un límite de 150 ms, DECRQM y XTVERSION;
  - teclado kitty (todos los niveles), modifyOtherKeys y Option izquierda como Meta;
  - reporte de ratón (`?1000`, `?1002`, `?1003`, `?1006`), foco (`?1004`) y `?1007`;
  - OSC 8 y URL con Cmd+clic;
  - OSC 52 solo de escritura;
  - notificaciones (OSC 9, 777 y 99, y comandos de más de 10 s) con la ventana inactiva;
  - la campana hace rebotar el icono del Dock.
- **M7, implementado y en verificación manual** (lista en `docs/verificacion-m7.md`): configuración TOML con recarga en caliente. Cubre fuente, colores, margen, shell, historial, portapapeles, notificaciones y campana. El teclado todavía no es configurable.
- **Pendiente para cerrar la v1:**
  - el criterio de terminada: sustituir a Warp durante una semana (`docs/semana-de-prueba.md`);
  - medir la latencia con Typometer (`scripts/latency.sh`);
  - cerrar la distancia con Ghostty en throughput (`docs/rendimiento.md`).
- **Después de la v1:** el roadmap de versiones v1.1–v2 (agentes, broker, review, bloques visuales, ajustes) está en `docs/roadmap.md`, derivado del spec de producto. Ese alcance no forma parte de la v1; las decisiones ya cerradas están en su anexo. **Rama `v1.1-contenedor`:** v1.1 (contenedor, `docs/verificacion-v1.1.md`), v1.2 (agentes, `docs/verificacion-v1.2.md`) y v1.3 (notificaciones, Dock y barra de menús, `docs/verificacion-v1.3.md`) y v1.4 (Review e inspector, `docs/verificacion-v1.4.md`) implementadas y en verificación manual; sin mergear a `main`.
- **Regla de AppKit (macOS 14+):** una vista no recorta su dibujo a sus límites. Las vistas propias rellenan `self.bounds()`, no el `rect` de `drawRect:`, y `NSRectFill` copia en vez de mezclar (usar `NSRectFillUsingOperation(…, SourceOver)` con colores translúcidos).
- **Pospuesto:** comparación con Ghostty y Alacritty. Hay que instalarlos, y Java para Typometer.

**Modelo de hilos (`session.rs` y `window.rs`):**
- **Lector de la PTY (uno por panel):** hace `advance` por lotes con el `Mutex<Term>` tomado y escribe las respuestas (DA/DSR). Solo pide un frame si el panel está visible.
- **Layout:** el hilo principal calcula los rectángulos de la barra, los paneles y los divisores, y publica un `WindowFrame`. El render pinta lo que hay en él.
- **Render (uno por ventana):** tiene su propio `NSRunLoop` y un `CADisplayLink`, creado por la vista y enganchado con `performSelector:onThread:`.
  - En cada tick copia las filas sucias y pinta.
  - Si no hay cambios, pausa el display link.
  - El protocolo de `dirty`/`running` en `on_tick` y `Waker::request_frame` evita perder avisos; si se cambia, hay que conservar esa propiedad.
- **macOS no dispara el display link si la ventana está tapada:** una ventana lanzada desde otra app que queda detrás no pinta hasta que se ve.

**Historial y líneas absolutas (`grid.rs`):**
- Las líneas tienen números absolutos (`Line`), que no cambian con el scroll. La selección y los bloques guardan `Point`s absolutos.
- Al descartar líneas antiguas, `first_line` avanza. Los bloques terminados que quedan fuera se descartan, y la selección se anula.
- **Reflow:** reconstruye las líneas lógicas y recoloca los puntos. Por eso, cualquier posición nueva que haya que conservar al redimensionar debe pasar por `Grid::resize_reflow`.
- Si se redimensiona con el cursor en el prompt (entre las marcas A y C), el prompt se borra y lo redibuja la shell.
- **Memoria medida:** 25,6 MiB para 10.000 líneas de unas 170 columnas (16 B por celda). El límite definitivo sigue abierto.

**Integración con zsh (`crates/app/shell/zsh/`):**
- Se inyecta con `ZDOTDIR` y restaura el del usuario.
- Los hooks se colocan en el primer prompt, detrás de p10k: A en `precmd`, B en `zle-line-init` y C, con `cmdline_url`, en `preexec`.
- D va dentro de `PROMPT_EOL_MARK`, porque zsh imprime la marca de `PROMPT_SP` antes que los `precmd`.
- Los tests de `shell.rs` usan una zsh real con un `HOME` temporal; no deben cargar la configuración del usuario.

**Pendiente en el núcleo:**
- Los caracteres de ancho 0 (combinantes, ZWJ, selectores) se unen a la celda anterior como grupo, en una tabla por cuadrícula con códigos de uso privado. Las columnas siguen a `wcwidth`: una secuencia ZWJ se ve como varios emoji.
- Solo existe G0, con ASCII o gráficos de línea DEC. SO/SI no tienen efecto.
- ED 3 se ignora hasta que exista el scrollback.
- macOS no acepta `O_CLOEXEC` en `posix_openpt`, así que se pone después con `fcntl`. Para evitar que otro `fork` herede el maestro en ese hueco, **todas las sesiones se lanzan desde el hilo principal**; hay que mantenerlo así.
- **Throughput de `bench-core`:** 91–179 MiB/s según el caso. La comparación con Ghostty y Alacritty está en `docs/rendimiento.md`.
  - Las filas de pantalla van en `Box`, para que desplazar una región mueva punteros.
  - Cada fila guarda `used`, una cota de la parte con contenido. Invariante: más allá de `used`, todo es `Cell::default()`. Lo comprueba un test.

**Eventos del núcleo a la app:**
- El núcleo deja en `Term::take_events` el portapapeles (OSC 52) y las notificaciones. Nunca actúa por su cuenta.
- La sesión los recoge en el lector, añade la campana y los comandos largos, y avisa al hilo principal (`sessionEvents`).
- La vista decide qué hacer según si la ventana está activa.

**Pendiente en la app:**
- **Escrituras a la PTY:** van por una cola a un hilo escritor por sesión (trozos pequeños tras `poll`). Así un pegado enorme no bloquea ni la interfaz ni el lector. El tamaño y las consultas usan otro manejador.
- Si `LANG` no está definida (app abierta desde el Finder), se le da a la shell un locale `<idioma del sistema>.UTF-8` o `en_US.UTF-8`.

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

- **Nombre del producto.** Mientras tanto se usa `rust-terminal`, centralizado en `PRODUCT_NAME` (`crates/core/src/lib.rs`). De esa constante salen la ruta de configuración, el `.app` (`scripts/bundle.sh` la lee), el identificador del bundle, el título por defecto y XTVERSION.
  - Al decidirlo se renombra todo eso, pero no los crates.
  - Hay que migrar la configuración desde `~/.config/rust-terminal/`.
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
- **Instalar o actualizar en un Mac:** `scripts/install.sh`. Hace `git pull`, compila el `.app` e instala en `/Applications`.
- **Icono:** `scripts/make-icon.sh` lo regenera desde `crates/app/src/icon.rs` (dibujo con Core Graphics en un test ignorado) a `crates/app/assets/AppIcon.icns`.
- **Repositorio:** `github.com/riama23/rust-terminal` (privado). Los fallos se informan con Cmd+Shift+I (`report.rs`); la URL de las issues está en `report::NEW_ISSUE_URL`. Guía de la semana de prueba: `docs/semana-de-prueba.md`.
- **App empaquetada:** `scripts/bundle.sh [debug|release]` genera `target/<perfil>/rust-terminal.app`, con firma ad hoc y nombre provisional.
  - Para probarla con logs: `open -n --stderr /tmp/rt.log --env RT_FRAME_STATS=1 target/debug/rust-terminal.app`.
- **Benchmarks:**
  - `scripts/compare.sh` compara con Ghostty y Alacritty usando vtebench. `RT_COMPARE` elige qué terminales medir.
  - `scripts/latency.sh` abre Typometer para medir la latencia a mano.
  - `scripts/bench.sh core [filtro]` es el microbenchmark del núcleo.
  - `scripts/bench.sh vtebench` hay que ejecutarlo dentro de la terminal que se quiere medir. Usa `vtebench` fijado a un commit.
  - Los resultados se guardan en `target/bench/`.
- **Render a imagen** para revisarlo a ojo: `RT_RENDER_DUMP=/tmp/render.bmp cargo test -p rust-terminal dump -- --ignored`.
- **Variables de depuración:**
  - `RT_LOG_UNHANDLED=1`: muestra en stderr las secuencias que el núcleo ignora.
  - `RT_FRAME_STATS=1`: muestra los frames por segundo.
  - `RT_NO_SHELL_INTEGRATION=1`: desactiva la inyección para zsh.
  - Verificación de la interfaz de agentes (no hace falta ver la pantalla): `RT_SNAPSHOT_DIR=/ruta` vuelca la sidebar, la barra y el formulario a PNG; `RT_DUMP_PANE=/archivo` vuelca el texto del panel del agente; `RT_AGENT_TEST="tarea"` (con `RT_AGENT_DIR`) lanza un agente al arrancar; `RT_OPEN_SHEET=1` y `RT_SHEET_PREFILL` abren el formulario; `RT_DEBUG_SIDEBAR=1` pone un panel lateral de prueba; `RT_AUTO_DECIDE=allow|deny` contesta solo los permisos y **solo existe en compilaciones debug**.
  - `scripts/window-shot.sh` captura solo la ventana de la app (necesita el permiso de Grabación de pantalla). Una ventana lanzada desde otra app no pinta Metal hasta ponerla al frente.

Rust está instalado con rustup en `~/.cargo/bin`. Si `cargo` no aparece en el PATH de un shell no interactivo, ejecuta antes `. "$HOME/.cargo/env"`.
