# Roadmap de versiones

Este documento encaja el spec de producto (`docs/macOS Terminal — UI UX Spec for Engineering.md`) con el estado real del código y con las restricciones de `CLAUDE.md`. El spec queda sin tocar como referencia. Cada versión es usable por sí sola y tiene un criterio de salida verificable.

## Resumen

| Versión | Tema | Estado |
| --- | --- | --- |
| v1.0 | Cierre de la v1: semana de prueba, latencia, throughput | En curso |
| v1.1 | Spike de hooks + ventana como contenedor de vistas | Hecho (rama `v1.1-contenedor`), verificación manual pendiente |
| v1.2 | Broker, lanzar agente, sidebar, aprobar y denegar | Hecho (misma rama), verificación manual pendiente (`docs/verificacion-v1.2.md`) |
| v1.3 | Notificaciones accionables, Dock badge, menu bar extra | Hecho (rama `v1.1-contenedor`), verificación manual pendiente (`docs/verificacion-v1.3.md`) |
| v1.4 | Review de diff, comentarios en lote, inspector | Hecho salvo descartar worktree y abrir PR (rama `v1.1-contenedor`), verificación manual pendiente (`docs/verificacion-v1.4.md`) |
| v1.5 | Bloques visuales, paleta de comandos, polish de splits | Pendiente |
| v1.6 | Ajustes, tema Paper, quick terminal, keybindings | Pendiente |
| v2 | API de control por agentes | Sin diseñar |

El eje del roadmap es **actuar sobre el agente** desde la terminal (aprobar, denegar, revisar), no ser una terminal más. Los bloques visuales y la paleta van al final.

---

## v1.0 · Cierre de la v1

**Qué incluye**
- Semana de prueba sustituyendo a Warp (`docs/semana-de-prueba.md`), incluido el trabajo con Claude Code.
- Latencia medida con Typometer (`scripts/latency.sh`).
- Throughput frente a Ghostty y Alacritty (`docs/rendimiento.md`).

**Criterio de salida:** sustituir a Warp durante una semana sin volver a abrirlo.

**Dependencias:** ninguna. Es la base sobre la que se construye todo lo demás.

**Alcance:** sin cambios respecto al `CLAUDE.md`.

---

## v1.1 · Spike de hooks y ventana como contenedor

Dos trabajos independientes que se hacen en la misma versión.

### Hito 0: spike de verificación

Sin UI. Un binario o script de prueba con un Claude Code real. Resultados escritos en `docs/spike-hooks.md`.

**Estado (2026-11-23, Claude Code 2.1.330): parcialmente hecho.** Resuelto: (1) una respuesta HTTP `{}` sin `decision` muestra el prompt propio de Claude Code, así que el *fail safe* se sostiene; (2) los `hooks` se fusionan con los del usuario; (3) `PermissionRequest` no trae `tool_use_id` y el matching por hash de `tool_input` **no sirve** porque otros hooks reescriben la orden; (4) el `message` del deny llega a Claude, pero como resultado de herramienta. Pendiente: la fusión de `statusLine` con una existente, los tipos de `Notification` distintos de permiso, el timeout y las peticiones simultáneas. Además, `SessionStart` no llega por HTTP, solo por hook `command`. Detalle en `docs/spike-hooks.md`.

Lista original de comprobaciones:

1. **Respuesta exit 0 sin `decision`** en `PermissionRequest`. La documentación oficial solo lo garantiza para exit 2. Si falla, el *fail safe* del spec cambia y se vuelve a hablar antes de construir la aprobación.
2. **Fusión con `--settings`:** cómo se combinan las claves `hooks` y `statusLine` con las del usuario. Lo más probable es que `statusLine` se reemplace entero, pero no está verificado.
3. **Campos reales** de `PermissionRequest`. La documentación no incluye `tool_use_id`, que el spec usaba para casar peticiones. Se confirma el conjunto real.
4. **Formato del deny:** el campo es `message`; se confirma que Claude Code lo muestra al modelo.

**Regla:** si (1) falla, no se construye la aprobación hasta revisar el diseño.

### Ventana como contenedor de vistas

Hoy hay una sola `NSWindow` (`crates/app/src/main.rs`, `open_window`) con una `CAMetalLayer` y una sola `TerminalView` (`crates/app/src/view.rs`). Todo el chrome, incluida la barra de pestañas (`TabBar` en `renderer.rs`), se dibuja en Metal.

El cambio es estructural:
- La `TerminalView` pasa a ser **subvista** de un contenedor AppKit.
- Se deja sitio para sidebar, inspector y attention bar como `NSView` nativas.
- **Las pestañas actuales no se migran.** Siguen en Metal.

**Criterio de salida:** la terminal funciona exactamente igual que antes; ninguna regresión en `docs/verificacion-m2-m3.md` a `m7.md`. Además, se puede añadir una `NSView` lateral de prueba sin tocar el render.

**Dependencias:** la v1.0 cerrada. Bloquea todo lo siguiente con UI nueva.

**Riesgo:** toca la jerarquía de vistas, que hasta ahora es única. Conviene tener el hito aislado en su propia rama.

---

## v1.2 · Broker, lanzar agente y aprobación

Es el núcleo de la tesis del producto.

**Qué incluye**

- **Broker:** hilo síncrono dentro de la app, con `std::net`, escuchando en `127.0.0.1` con puerto aleatorio y token por lanzamiento. Sin `tokio`. Los eventos llegan al hilo principal por el mismo mecanismo que `sessionEvents`.
- **Rechazo** de peticiones con `session_id` desconocido o token incorrecto.
- **Lanzar agente (⌘N simple):** sheet con tarea, aislamiento (`-w <name>` o carpeta actual), modo de permisos (`plan`, `default`, `acceptEdits`) y modelo. Muestra en solo lectura el comando exacto que se ejecutará (principio "no magic"). El agente se abre en un panel propio, siempre en Classic.
- **Sidebar de sesiones:** solo sesiones lanzadas por la app. Orden: *Necesita tu atención* primero, luego *Trabajando*, *Error*, *Plan listo*, *Terminado*.
- **Attention bar:** muestra el comando exacto pendiente y ofrece *Permitir una vez* (⌘↵) y *Denegar…* (⌘⌫). *Siempre permitir* escribe en `.claude/settings.local.json` con la regla visible antes de guardar.
- **Denegar con motivo:** el `message` del deny llega a Claude, pero como resultado de herramienta y no como mensaje del usuario (spike). **Decidido (2026-11-23):** solo `message`, sin teclear nada en la sesión; si Claude no lo sigue al pie de la letra, el usuario le escribe directamente.
- **Política de matching:** sin `tool_use_id`, y sin poder casar por hash de `tool_input` (otros hooks lo reescriben), se casa por `session_id` y orden de llegada. Siempre se muestra el `tool_input` del `PermissionRequest`, que es lo que se aprueba. Si hay dos peticiones simultáneas de la misma sesión, o ninguna encaja con una sesión conocida, no se devuelve decisión y gana el prompt de Claude Code. **Nunca se permite a ciegas.**
- **Inicio de sesión:** `SessionStart` no llega por HTTP, así que se usa un hook `command` que avise al broker.
- **Tokens de color** del spec en `palette.rs`, solo Dusk. La paleta ANSI no cambia.
- **Wrapper de status line:** ejecuta el comando del usuario, devuelve su salida sin cambios y reenvía el JSON de stdin al broker.
- **Versión mínima de Claude Code:** la probada en el spike. Se lee del campo `version` de la status line. Por debajo, el agente funciona como panel normal, sin sidebar ni attention bar.

**Criterio de salida:** aprobar y denegar un permiso real de Claude Code sin salir del teclado. Verificar que una petición ambigua cae al prompt de Claude Code y que nunca se permite automáticamente.

**Dependencias:** v1.1 (spike aprobado y ventana como contenedor).

**Riesgos**
- Sin `tool_use_id`, el emparejamiento es más débil. Se mitiga con la política conservadora y con el spike.
- `--settings` podría pisar la configuración del usuario. Se mitiga con el wrapper de status line.
- Cualquier proceso local podría enviar eventos falsos al broker. Se mitiga con el token y el rechazo de sesiones desconocidas.

---

## v1.3 · Notificaciones accionables y menu bar extra

**Qué incluye**

- **Notificaciones con acciones** (`notify.rs` hoy solo envía título y cuerpo). Acciones: *Permitir una vez*, *Enviar respuesta* (deny con texto) y *Abrir*. Las notificaciones de finalización llevan *Revisar*.
- **Dock badge** con el número de sesiones que necesitan atención. No cuenta salida sin leer.
- **Menu bar extra** (`NSStatusItem`): lista de agentes con estado y contador de los que necesitan atención. *Nuevo agente…* con ⌃⌥N global.
- Las notificaciones de *Necesita tu atención* y de finalización solo se disparan cuando la app no está en primer plano, y *Terminado* y *Error* avisan una vez por turno.

**Criterio de salida:** responder a un permiso desde la notificación con la app en segundo plano, y ver el contador correcto en el Dock y en la barra de menús.

**Dependencias:** v1.2 (broker y estados de sesión).

**Riesgo:** `agent_needs_input` y `agent_completed` solo se emiten con la vista de agentes abierta. Si no están disponibles, la notificación se basa en `permission_prompt` y `Stop`.

---

## v1.4 · Review

**Qué incluye**

- **Review (⌘⇧R):** diff de la worktree del agente contra su rama base (`git diff <base>...HEAD`). Vista unificada con números de línea, alternable a dividida.
- **Lista de archivos** con etiqueta A/M y contador de comentarios.
- **Comentarios en lote:** *Añadir a la revisión* (⌘↵) encola; *Enviar N comentarios a Claude* entrega todos a la sesión de origen en un solo mensaje, con referencias `archivo:línea`.
- **Inspector** (columna derecha): *Plan* (lista de tareas), *Changes* (archivos con +/−), *Context* (barra, ámbar por encima del 70 %) y *Worktree*.
- **Done bar:** aparece cuando una sesión termina un turno con cambios, con resumen, *Revisar cambios* y *Abrir PR*.
- **Pie de sesión:** modelo, contexto %, coste y pista de arrastre.
- **Descartar worktree** y **Confirmar y abrir PR** en la cabecera de Review.

**Criterio de salida:** revisar los cambios de un agente, dejar comentarios en varias líneas y devolverlos a la sesión en un único envío.

**Dependencias:** v1.2 (sesiones) y v1.1 (contenedor de vistas, para la columna del inspector).

**Riesgo:** con muchos agentes en paralelo, la cola de revisión crece. Se mostrará el número de sesiones pendientes de revisar y un aviso a partir de un límite blando.

---

## v1.5 · Bloques visuales, paleta y polish de splits

**Qué incluye**

- **Bloques visuales** sobre el modelo OSC 133 que ya existe (`crates/core/src/block.rs`):
  - Cabecera con icono de estado, metadatos (`cwd · rama · hora`) y comando.
  - Duración o código de salida, y acciones *Detener* (solo en ejecución), *Copiar salida*, *Relanzar* y *Colapsar*.
  - Estados: ejecutando, correcto, fallido (fondo tintado y barra de acciones), seleccionado y colapsado.
  - Acciones de fallo: *Abrir en editor* (⌘O) desde el primer `ruta:línea:columna`, y *Relanzar solo fallidos* (⌘⇧R) si el runner lo permite.
  - Pantalla alternativa: sin bloques mientras una app ocupa la pantalla (vim, htop).
- **Modo Classic siempre disponible** por ajuste, por perfil y por host. Vuelta automática a Classic si faltan las marcas tres comandos seguidos.
- **Paleta de comandos (⌘⇧P):** acciones, historial y flujos, con ranking por frecuencia. ↵ ejecuta; ⌘↵ inserta sin ejecutar.
- **Polish de splits:** atenuado de paneles no enfocados (55 %, ajuste *Atenuar splits sin foco*), cabecera de 30 px por panel, zoom (⌘⇧↵) y resize por teclado.
- **Spinner en las pestañas** mientras corre un proceso, según lo que dibuja el spec en la pantalla 01.

**Decisión pendiente:** *Relanzar solo fallidos* (⌘⇧R) se fija aquí. Como ⌘⇧R ya es *Revisar* desde la v1.4, se decide si el rerun sigue en ese atajo o se asigna otro. La decisión se toma con el teclado configurable de la v1.6 en mente.

**Criterio de salida:** un bloque fallido se puede abrir en el editor y relanzar, y la paleta ejecuta un comando del historial. Con los bloques desactivados, la terminal se comporta como en la v1.0.

**Dependencias:** v1.0 (OSC 133 ya existente). Puede empezar en paralelo a v1.4, pero se entrega después.

**Riesgo:** es la capa más arriesgada y la menos ligada a la tesis de agentes. Por eso va después de la revisión.

---

## v1.6 · Ajustes, tema claro, quick terminal y teclado

**Qué incluye**

- **Ventana de ajustes** (⌘,), generada desde la tabla `OPTIONS` de `config.rs`. Cada opción visible es una línea de `config.toml`, y la ventana lo indica. Tamaño fijo de 960 × 680 según el spec.
- **Tema Paper y Follow system.** Los tokens de Paper ya se definieron en v1.2; aquí se activan. Los valores marcados como propuesta en el spec (`sidebar`, `attention`, `prod` en Paper) se validan.
- **Quick terminal (⌥Space):** panel desde el borde superior, con fondo translúcido y desenfoque. Se oculta al perder el foco salvo que esté fijado. *Mover a ventana nueva* lo convierte en ventana normal.
- **Badge PROD** para hosts SSH marcados como producción, desde una opción de `config.toml`. Aparece en la pestaña y en la sidebar de shells.
- **Keybindings configurables** en `config.toml`. Resuelve la superposición de atajos de la v1.5.

**Criterio de salida:** cualquier opción de la ventana de ajustes aparece también en `config.toml`, y viceversa. El quick terminal aparece y se oculta con ⌥Space sin robar foco de forma inesperada.

**Dependencias:** v1.5 (paleta y bloques ya cerrados, para no rehacer ajustes). Puede empezar antes en lo que no depende de UI.

**Riesgo:** ⌥Space es global. Requiere permiso de accesibilidad o un mecanismo equivalente, que hay que comprobar antes de comprometer la versión.

---

## v2 · API de control por agentes

Permite a un agente abrir sesiones, enviar teclas, leer la pantalla y leer bloques como datos estructurados.

**Abierto:** CLI, socket o MCP; modelo de seguridad; estabilidad de la API. No se diseña ahora.

**Separación:** el broker de v1.2 (Claude Code hacia la app) y esta API (agentes hacia la app) son canales distintos. El modelo de sesiones de la v1.2 se diseña para reutilizarse aquí.

---

## Fuera de alcance de este roadmap

- Nombre del producto (`PRODUCT_NAME`). Bloquea la distribución y la ruta de configuración definitiva.
- Licencia, código abierto, firma, notarización, Homebrew, actualizaciones, soporte para Intel y versiones anteriores de macOS.
- Límite de memoria del scrollback.
- Sesiones lanzadas fuera de la app (`claude agents --json`, `claude attach`) y reanudación tras reiniciar (`claude --resume`).
- Editor de entrada propio, con chips, sugerencias en línea y completado.
- Terminfo propio, integración con bash y fish.

---

## Cobertura del spec

Cada pantalla del spec aparece en una versión o queda fuera. Al verificarlo se encontraron piezas sin asignar, que ya se han incluido: atenuado y cabeceras de splits, zoom, spinner de pestañas y badge PROD.

| Spec | Qué es | Versión |
| --- | --- | --- |
| 01 Ventana principal | Pestañas, bloques, editor de entrada | Pestañas ya existen; bloques v1.5; editor de entrada fuera |
| 02 Splits | Foco, atenuado, cabeceras, zoom | Foco ya existe; atenuado, cabeceras y zoom v1.5 |
| 03 Paleta | ⌘⇧P | v1.5 |
| 04 Quick terminal | ⌥Space | v1.6 |
| 05 Ajustes | Ventana desde `OPTIONS` | v1.6 |
| 06 Estados de bloque | Ejecutando, fallido, seleccionado, colapsado | v1.5 |
| A1 Home de agentes | Sidebar, panel, inspector | Sidebar v1.2; inspector v1.4 |
| A2 Notificaciones y menu bar | Acciones, Dock, menu bar | v1.3 |
| A3 Review | Diff y comentarios en lote | v1.4 |
| A4 Nuevo agente | Sheet con comando exacto | v1.2 (versión simple) |

Atajos: ⌘⇧R queda para *Revisar* (v1.4). *Relanzar solo fallidos* se decide en v1.5. Los demás atajos del spec se asignan con su pantalla.

---

## Anexo: decisiones cerradas

Respuestas a las tres rondas de preguntas, todas aceptadas.

**Alcance y tesis**
1. La v1 se cierra sin cambios de alcance. El spec alimenta v1.x y v2.
2. El eje del roadmap es actuar sobre el agente. Los bloques visuales y la paleta van al final.
3. La v1 incluye la ventana de ajustes y la paleta solo como fuera de alcance; ambas pasan a v1.5 y v1.6.

**Tecnología**
4. UI nueva en AppKit con `objc2`. Sin SwiftUI ni web views. Metal solo para la cuadrícula.
5. Las pestañas actuales en Metal no se migran.
6. Broker: hilo síncrono dentro de la app, con `std::net`, solo loopback, token por lanzamiento. Sin `tokio`.
7. El broker y la futura API de control son canales separados.
8. Se mantiene el TOML y la tabla `OPTIONS` como fuente de verdad. La ventana de ajustes se genera desde ella.

**Agentes**
9. Matching sin `tool_use_id`: `session_id` y orden de llegada (el hash de `tool_input` se descartó tras el spike). Ante ambigüedad, no se decide y gana el prompt de Claude Code.
10. Denegar con motivo: campo `message` y nada más. Tras el spike se reabrió (llega a Claude como resultado de herramienta, no como mensaje del usuario) y se confirmó así.
11. Un agente es un panel con `claude --settings …`, en pestaña propia y siempre en Classic.
12. Sesiones externas y reanudación tras reinicio, diferidas.
13. Rama: se acepta `worktree-<name>` de Claude Code, y el spec se corrige en consecuencia.
14. *Siempre permitir* escribe en `.claude/settings.local.json`.
15. Layout por defecto Classic hasta los bloques visuales.
16. Versión mínima de Claude Code: la probada en el spike. Por debajo, el agente funciona como panel normal.

**Atajos y temas**
17. ⌘⇧R es *Revisar*. *Relanzar solo fallidos* se decide en v1.5.
18. Tokens del spec en `palette.rs` desde v1.2. Solo Dusk hasta v1.6. La paleta ANSI de 16 colores no se define.

**Entregable**
19. Un solo documento de roadmap, con una sección por versión. El spec queda como referencia sin tocar.
20. Pendiente abierto: nombre del producto.

## Correcciones al spec

- `PermissionRequest` no incluye `tool_use_id`. El spec usa ese campo para casar peticiones.
- El campo de denegación es `message`, no un campo de motivo distinto.
- `agent_needs_input` y `agent_completed` solo se emiten con la vista de agentes abierta.
- `rate_limits` solo aparece en suscripciones Pro, Max y Ultra, y solo tras la primera respuesta.
- `--settings` y la fusión de `hooks` y `statusLine` no están verificados; se prueban en v1.1.
