# API de control por agentes (v2, experimental)

Permite que un agente que corre **dentro de una terminal de la app** (por ejemplo Claude Code) liste las sesiones, lea pantalla y bloques y, con permiso del usuario, escriba en ellas. Es un canal distinto del broker de hooks de la v1.2 (Claude Code hacia la app): este va del agente hacia la app.

**Estado:** protocolo `v0`, **experimental**. Puede cambiar sin aviso hasta que se use de verdad. Desactivable con `control.enabled = false` (se aplica al reiniciar).

## Cómo se usa

Cada shell que lanza la app recibe:

| Variable | Qué es |
| --- | --- |
| `RT_CONTROL_SOCKET` | Ruta del socket Unix |
| `RT_CONTROL_TOKEN` | Token de este arranque de la app |
| `RT_CTL` | Ruta del comando `rt` (que además queda en el `PATH` de esas shells) |

```sh
rt sessions                       # id, ventana, pestaña, título, cwd, [foco, agente, ocupada]
rt read 1-1 --lines 200           # la pantalla (o las últimas N líneas, con historial)
rt blocks 1-1 --limit 10          # comandos con su código de salida
rt output 1-1 7                   # la salida del bloque 7
rt send 1-1 --enter "cargo test"  # teclea texto (pide permiso)
rt keys 1-1 ctrl+c                # pulsa teclas (pide permiso)
rt open --cwd ~/proyecto          # abre una sesión (pide permiso)
rt close 1-3                      # cierra una sesión (pide permiso)
```

`--json` imprime el resultado en JSON. Códigos de salida: 0 bien, 1 error de la app, 2 uso, 3 sin conexión. El nombre del agente sale de `RT_CLIENT` o, si no, del proceso padre.

Los ids de sesión tienen la forma `<ventana>-<panel>` (`1-1`): la ventana principal y el terminal rápido tienen ventanas distintas.

## Seguridad

- **Socket privado:** en una carpeta `0700` con el socket `0600`; la app comprueba además que el otro extremo es el mismo usuario. La carpeta se borra al salir (y las de ejecuciones que murieron, al arrancar).
- **Token por arranque:** 128 bits del sistema, distinto en cada lanzamiento, que solo reciben las shells de la app. Se comprueba en tiempo constante y con 5 fallos se cierra la conexión.
- **Leer funciona con el token. Escribir pide permiso.** Teclear (`input.*`) y abrir o cerrar sesiones (`sessions.open`, `sessions.close`) abren un diálogo con la acción exacta (el texto, las teclas), el proceso que se conectó (pid y orden) y tres botones: **Denegar** (por defecto), **Permitir una vez** y **Permitir en esta sesión** (por agente y por grupo, hasta que cierres la app). Tras un «no», el mismo agente no vuelve a preguntar durante 10 s.
- **Aviso visible:** tras una acción permitida, la barra de pestañas muestra «control: <agente>» unos segundos.
- **Límites:** 1 MiB por petición, 64 KiB de texto, 256 teclas, 16 conexiones, 2 MiB por lectura. El texto de `send` solo admite letras, tabulador y saltos de línea; cualquier otro carácter de control (Ctrl-C, secuencias de escape) se rechaza y hay que usar `keys`.
- **El nombre del agente lo declara él mismo** y no es de fiar: el diálogo enseña también el pid y la orden del proceso.

**Lo que esto NO protege:** cualquier proceso que corra en una shell de la app tiene el token y puede **leer la pantalla y los bloques de todas las sesiones** (por ejemplo, una pantalla donde se vio un secreto) sin permiso. Es la decisión de diseño (la lectura va solo con token). Si no lo quieres, `control.enabled = false`.

## Protocolo

Una petición JSON por línea y una respuesta JSON por línea, por el socket:

```json
→ {"v":0,"id":1,"token":"…","client":"claude","method":"sessions.list","params":{}}
← {"v":0,"id":1,"ok":true,"result":{"sessions":[…]}}
← {"v":0,"id":1,"ok":false,"error":{"code":"denied","message":"…"}}
```

Códigos de error: `bad_request`, `unauthorized`, `unknown_method`, `not_found`, `denied`, `too_large`, `timeout`, `busy`.

| Método | Parámetros | Resultado | Permiso |
| --- | --- | --- | --- |
| `hello` | — | `protocol`, `experimental`, `app`, `pid` | — |
| `sessions.list` | — | `sessions`: `id`, `window` (`main`/`quick`), `tab`, `active`, `focused`, `title`, `cwd`, `busy`, `ssh_host`, `agent`, `cols`, `rows` | — |
| `screen.read` | `session`, `lines?` | `text`, `cols`, `rows`, `cursor`, `alt_screen`, `truncated` | — |
| `blocks.list` | `session`, `limit?` (20) | `blocks`: `id`, `command`, `cwd`, `running`, `exit_code`, `duration_ms` | — |
| `blocks.output` | `session`, `block` | `text`, `truncated` | — |
| `input.send_text` | `session`, `text`, `enter?` | `sent` | Teclear |
| `input.send_keys` | `session`, `keys` (`ctrl+c`, `enter`, `up`, `alt+x`, `f5`…) | `sent` | Teclear |
| `sessions.open` | `cwd?` | `session` | Sesiones |
| `sessions.close` | `session` | `closed` | Sesiones |

`blocks.*` necesita la integración de zsh (marcas OSC 133); sin ella no hay bloques. Un agente (pane de Claude Code) aparece con `agent: true`, pero se trata como cualquier otra sesión.

## Fuera de la v0

Lanzar agentes de Claude Code, aprobar permisos del broker y enviar revisiones por la API; un servidor MCP (si se hace, será un adaptador sobre este socket); suscribirse a cambios (eventos); cambiar el socket de sitio o usar TCP.
