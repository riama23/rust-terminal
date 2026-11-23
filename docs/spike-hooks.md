# Spike de hooks (v1.1, hito 0)

Verificación empírica de los supuestos del spec con un Claude Code real. Versión probada: **2.1.330**. Fecha: 2026-11-23.

Herramientas, en `scripts/spike-hooks/`:
- `server.py`: servidor de captura en `127.0.0.1` con token en la ruta. Registra cada petición en un JSONL, responde a `PermissionRequest` según el modo (`silent`, `allow`, `deny`) y genera el `settings.json` para `claude --settings`.
- `drive.py`: lanza `claude` interactivo en un pseudo-terminal y vuelca la pantalla. Solo observa; no envía Intro, así que un prompt de permiso queda sin contestar.

Dos formas de probar: `claude -p` (no interactivo; sirve para ver payloads y el efecto de allow/deny) y `drive.py` (sesión interactiva; necesaria para lo que depende de la pantalla).

## Resultados

| # | Pregunta | Resultado | Cómo se comprobó |
| --- | --- | --- | --- |
| 1 | Respuesta HTTP 200 con `{}` (sin `decision`): ¿aparece el prompt propio? | **Sí.** Se muestra "Do you want to proceed? 1. Yes … 4. No" y la orden no se ejecuta. El *fail safe* del spec se sostiene. | `drive.py`, modo `silent`, sesión interactiva |
| 2 | `allow` desde el hook | **Funciona.** La orden se ejecuta sin prompt. | `claude -p`, modo `allow`: el archivo se creó |
| 3 | `deny` con `message` | **El texto llega a Claude** como motivo literal del rechazo y la orden no se ejecuta. Ver matiz abajo. | `claude -p`, modo `deny` |
| 4 | Campos de `PermissionRequest` | `cwd`, `effort`, `hook_event_name`, `permission_mode`, `permission_suggestions`, `prompt_id`, `session_id`, `tool_input`, `tool_name`, `transcript_path`. **No hay `tool_use_id`** (confirmado). | Payload capturado |
| 5 | ¿Se fusionan los `hooks` de `--settings` con los del usuario? | **Sí, se fusionan** en esta versión. El hook `PreToolUse` de `rtk` (usuario) siguió reescribiendo órdenes junto al inyectado. | `git status --short` llegó a `PermissionRequest` como `rtk git status --short` |
| 6 | `statusLine` con `--settings` | **No probado contra una existente:** el usuario no tiene `statusLine` en `~/.claude/settings.json`. La inyectada sí se ejecutó y recibió el JSON. | `drive.py` |
| 7 | `SessionStart` | **No llega por HTTP**; **sí llega como hook `command`**. `SessionStart` tampoco se dispara en `claude -p`. | Ambas formas |
| 8 | `Notification` | Llega con un prompt de permiso pendiente (`message`: "Claude needs your permission…"). No se observaron los otros tipos (`idle_prompt`, `agent_*`). | `drive.py` |
| 9 | Campos de la status line | Recibidos: `context_window`, `cost`, `cwd`, `effort`, `model`, `session_id`, `transcript_path`, `version`, `workspace`, entre otros. **No** llegaron `rate_limits` ni `worktree` (facturación por API y sesión sin worktree). | `statusline.jsonl` |

## Hallazgos que cambian el diseño

### 1. El matching por hash de `tool_input` no sirve

`PreToolUse` sí trae `tool_use_id` y llega justo antes que `PermissionRequest`. La idea natural es casarlos por `session_id` + hash de `tool_name` y `tool_input`. **No funciona** cuando otro hook reescribe la orden: `PreToolUse` vio `git status --short` y `PermissionRequest` trajo `rtk git status --short`.

Dos consecuencias:
- Lo que se aprueba es el `tool_input` del `PermissionRequest`. Ese es el que hay que mostrar al usuario, nunca el del `PreToolUse`.
- La respuesta HTTP del hook es síncrona y pertenece a esa petición, así que para responder no hace falta `tool_use_id`. El riesgo real es solo mostrar la sesión equivocada. Se mitiga casando por `session_id` y orden de llegada, y si hay más de una petición pendiente en la misma sesión, no se decide (decisión 9 del roadmap, ajustada).

### 2. El `message` del deny se presenta como resultado de herramienta

Claude recibió el texto, lo citó literalmente cuando se le pidió, y **no obedeció** la instrucción que contenía ("responde con la palabra BANANA"): lo trató como salida de herramienta, no como un mensaje del usuario. Para *Denegar y responder*, esto implica que el texto funciona como motivo del rechazo, pero no garantiza que Claude lo siga como si fuera una orden del usuario. **Hay que decidir** si basta como motivo, o si el texto debe además teclearse en la sesión como mensaje de usuario (la alternativa del spec). La decisión 10 del roadmap queda abierta hasta que se pruebe con un texto redactado como respuesta real y no como instrucción.

### 3. `SessionStart` solo por `command`

El broker no puede enterarse del inicio de una sesión por un hook HTTP. Opciones: un hook `command` que haga el `curl` al broker, o deducirlo del primer evento o de la status line. El resto de eventos probados sí funcionan por HTTP.

### 4. Órdenes de solo lectura no generan `PermissionRequest`

`echo` se ejecuta sin pedir permiso y no dispara el hook. La attention bar solo aparece cuando Claude Code de verdad pide permiso, lo cual es correcto, pero conviene no contar con `PermissionRequest` como señal de actividad.

## Qué queda sin verificar

- **Fusión de `statusLine`** con una del usuario ya configurada (probar con otra cuenta o un proyecto con `statusLine`). Si se reemplaza entera, el wrapper de paso del spec es necesario.
- **Tipos de `Notification`** distintos de permiso: `idle_prompt`, `agent_needs_input`, `agent_completed`.
- **Timeout**: qué pasa si el hook no responde en 600 s.
- **Deny con texto redactado como respuesta** (ver hallazgo 2) y `interrupt`.
- **Varias peticiones simultáneas de la misma sesión** (herramientas en paralelo).
- **Versiones anteriores y posteriores** a 2.1.330, para fijar el mínimo.

## Cómo repetirlo

```sh
scripts/spike-hooks/server.py --mode silent --dir /tmp/spike   # en otra terminal
scripts/spike-hooks/drive.py /tmp/spike/settings.json "Usa Bash para ejecutar exactamente: touch /tmp/x" 60
```

Los eventos quedan en `/tmp/spike/events.jsonl` y la status line en `/tmp/spike/statusline.jsonl`. Para una prueba manual, lanza `claude --settings /tmp/spike/settings.json` tú mismo.
