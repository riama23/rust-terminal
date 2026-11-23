# Verificación manual de la v1.2

Lista de comprobación de la v1.2: agentes de Claude Code con sidebar, attention bar y formulario ⌘N. Los tests automáticos cubren el broker (HTTP, token, plazos, peticiones simultáneas), el lanzamiento (flags, `--settings`, status line), el modelo de estados, las reglas de *Siempre permitir* y el reparto del contenedor. Dos tests de extremo a extremo con el `claude` real están ignorados por defecto: `cargo test -p rust-terminal e2e -- --ignored --test-threads=1`.

**Criterio de salida:** aprobar y denegar un permiso real de Claude Code sin salir del teclado, y que nunca se permita nada por defecto.

## Cómo probar

```sh
scripts/bundle.sh release
open -n --stderr /tmp/rt.log target/release/rust-terminal.app
# Cmd+N abre el formulario de nuevo agente
```

Ayudas de verificación (todas de depuración, ver `CLAUDE.md`): `RT_AGENT_TEST`, `RT_SNAPSHOT_DIR`, `RT_DUMP_PANE`, `RT_OPEN_SHEET`, `RT_SHEET_PREFILL`, `RT_AUTO_DECIDE` (solo compilación debug).

## Ya comprobado (sin la ventana real)

- La sidebar se dibuja con anillo ámbar, icono «!» y «Necesita tu atención · Bash» cuando hay un permiso pendiente, y vuelve a «Trabajando» si Claude cierra la conexión (volcado a PNG).
- La attention bar muestra la orden completa y los tres botones sobre fondo oscuro (volcado a PNG).
- El formulario compone el comando (`-w`, `--permission-mode`, `--model`, tarea tras `--`) y elige «Carpeta actual» fuera de un repositorio.
- Con el broker en vivo: un `session_id` ajeno da 403 y solo `SessionStart` lo cambia; *Permitir* devuelve `allow` y *Denegar* devuelve `deny` con el mensaje.
- En compilación release, `RT_AUTO_DECIDE` no hace nada.

## Pendiente de comprobar a mano

- [ ] **Claude real dentro de la app.** Con tu pasarela en la red local, macOS puede pedir el permiso de «Red local» a `rust-terminal.app` la primera vez; sin él, Claude se queda en «No internet route». Concédelo y repite.
- [ ] Cmd+N abre el formulario; «Lanzar» se desactiva con la tarea vacía y se activa al escribir.
- [ ] Se abre una pestaña con Claude Code y aparece la sidebar a la izquierda con una fila.
- [ ] Un permiso real muestra la attention bar **bajo** la terminal y la terminal se acorta (la PTY recibe el nuevo tamaño).
- [ ] **Cmd+Intro** permite una vez y **Cmd+Retroceso** abre «Denegar…». Con la barra oculta, ambos atajos llegan a la terminal como siempre.
- [ ] «Permitir una vez», «Siempre permitir» y «Denegar…» funcionan con el ratón. «Siempre permitir» enseña la regla y el archivo antes de escribir, y está desactivado para órdenes con `*` o varias líneas.
- [ ] Contestar el permiso en la propia sesión de Claude Code retira la barra.
- [ ] Un clic en una fila de la sidebar lleva el foco a esa pestaña y panel.
- [ ] Con dos agentes, el que necesita atención sube arriba.
- [ ] Cerrar la pestaña de un agente quita su fila; sin agentes, la sidebar desaparece.
- [ ] Cmd+Q con un agente en marcha pide confirmación y, al salir, no queda `rust-terminal-agents-<pid>` en `$TMPDIR`.
- [ ] En reposo, con agentes abiertos, `RT_FRAME_STATS` marca 0 frames/s.
- [ ] VoiceOver lee cada fila de la sidebar y los botones de la barra.

## Puntos abiertos

- **Dónde escribe *Siempre permitir*:** en el `.claude/settings.local.json` del proyecto desde el que se lanzó el agente. Con `-w`, Claude trabaja en un worktree; falta comprobar si lee ese archivo desde allí o necesita la regla en el propio worktree.
- **Sin Dock badge, notificaciones accionables ni menu bar extra:** son la v1.3.
- **Tema:** solo Dusk; la sidebar y la barra no siguen el modo claro del sistema.
