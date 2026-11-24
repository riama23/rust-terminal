# Configuración

> Archivo generado a partir de `crates/app/src/config.rs`. No lo edites a mano: `RT_UPDATE_DOCS=1 cargo test -p rust-terminal config` lo regenera.

La configuración se lee de `~/.config/rust-terminal/config.toml` (o de `$XDG_CONFIG_HOME/rust-terminal/config.toml`). El archivo es opcional y todas las opciones tienen un valor por defecto.

Los cambios se aplican al guardar el archivo, sin reiniciar. Las opciones marcadas como **pestañas nuevas** solo afectan a las pestañas y paneles que se abran después. Si hay un error, se muestra un aviso y se ignora solo la opción incorrecta.

## `[appearance]`

### `theme`

Tema de la interfaz: `dusk` (oscuro), `paper` (claro) o `system` (sigue al sistema). Los colores que pongas en `[colors]` mandan sobre los del tema.

- **Tipo:** uno de: `"dusk"`, `"paper"`, `"system"`
- **Por defecto:** `"dusk"`
- **Se aplica:** en vivo

## `[font]`

### `family`

Familia de la fuente (por ejemplo "JetBrains Mono"). Vacío: la monoespaciada del sistema (SF Mono). Si no existe, se usa la del sistema.

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** en vivo

### `size`

Tamaño de la fuente en puntos.

- **Tipo:** un número (6–72)
- **Por defecto:** `13.0`
- **Se aplica:** en vivo

## `[colors]`

### `background`

Color de fondo.

- **Tipo:** un color `"#rrggbb"`
- **Por defecto:** `"#1c1c1e"`
- **Se aplica:** en vivo

### `foreground`

Color del texto.

- **Tipo:** un color `"#rrggbb"`
- **Por defecto:** `"#d8d8d8"`
- **Se aplica:** en vivo

### `cursor`

Color del cursor.

- **Tipo:** un color `"#rrggbb"`
- **Por defecto:** `"#d8d8d8"`
- **Se aplica:** en vivo

### `selection`

Fondo del texto seleccionado.

- **Tipo:** un color `"#rrggbb"`
- **Por defecto:** `"#264f78"`
- **Se aplica:** en vivo

### `palette`

Los 16 colores ANSI: negro, rojo, verde, amarillo, azul, magenta, cian, blanco y sus versiones brillantes.

- **Tipo:** una lista de 16 colores `"#rrggbb"`
- **Por defecto:** `["#000000", "#cd3131", "#0dbc79", "#e5e510", "#2472c8", "#bc3fbc", "#11a8cd", "#e5e5e5", "#666666", "#f14c4c", "#23d18b", "#f5f543", "#3b8eea", "#d670d6", "#29b8db", "#ffffff"]`
- **Se aplica:** en vivo

## `[window]`

### `dim_unfocused`

Atenúa los paneles sin foco cuando hay varios en la pestaña.

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** en vivo

### `pane_headers`

Cabecera con el título sobre cada panel cuando hay varios en la pestaña.

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** en vivo

### `padding`

Margen interior de cada panel, en puntos.

- **Tipo:** un número (0–100)
- **Por defecto:** `6.0`
- **Se aplica:** en vivo

## `[shell]`

### `program`

Ruta de la shell. Vacío: la de `$SHELL` (o `/bin/zsh`).

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** pestañas nuevas

### `args`

Argumentos de la shell. Por defecto, shell de login.

- **Tipo:** una lista de textos
- **Por defecto:** `["-l"]`
- **Se aplica:** pestañas nuevas

### `integration`

Inyecta la integración con zsh (bloques de comandos). También se desactiva con `RT_NO_SHELL_INTEGRATION=1`.

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** pestañas nuevas

## `[agents]`

### `track_manual_claude`

Muestra en la sidebar los `claude` que lanzas a mano en un panel. Con zsh, una función `claude` le pasa los hooks de la app; con otras shells solo se ve que está en marcha.

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** pestañas nuevas

## `[blocks]`

### `visual`

Tiñe los bloques fallidos y el seleccionado y muestra la barra de acciones del bloque (⌘↑ y ⌘↓ seleccionan). Con `false`, solo quedan las marcas del margen.

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** en vivo

### `editor`

Orden para «Abrir en el editor» (⌘O) con `{path}`, `{line}` y `{col}`, por ejemplo `code -g {path}:{line}:{col}`. Vacío: `open -t` (el editor de texto por defecto, sin línea).

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** en vivo

## `[scrollback]`

### `lines`

Líneas del historial por panel. Cada línea de 170 columnas ocupa unos 2,7 KB.

- **Tipo:** un entero (0–1000000)
- **Por defecto:** `10000`
- **Se aplica:** pestañas nuevas

## `[clipboard]`

### `write`

Permite que los programas copien al portapapeles con OSC 52 (vim, tmux, ssh…).

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** en vivo

### `read`

Si los programas pueden leer el portapapeles con OSC 52: `deny` (nunca), `ask` (preguntar cada vez) o `allow` (siempre). Leerlo permite a un programa remoto ver lo que has copiado.

- **Tipo:** uno de: `"deny"`, `"ask"`, `"allow"`
- **Por defecto:** `"deny"`
- **Se aplica:** en vivo

## `[notifications]`

### `enabled`

Notificaciones del sistema con la ventana en segundo plano (las de los programas y las de comandos largos).

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** en vivo

### `long_command_seconds`

Avisa cuando termina un comando que tardó al menos estos segundos (requiere la integración con zsh). 0: nunca.

- **Tipo:** un entero (0–86400)
- **Por defecto:** `10`
- **Se aplica:** en vivo

## `[bell]`

### `dock_bounce`

Con la ventana en segundo plano, la campana (BEL) hace rebotar el icono del Dock.

- **Tipo:** `true` o `false`
- **Por defecto:** `true`
- **Se aplica:** en vivo

## `[keybindings]`

### `command_palette`

Paleta de comandos. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+p"`
- **Se aplica:** en vivo

### `new_agent`

Nuevo agente. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+n"`
- **Se aplica:** en vivo

### `review_changes`

Revisar cambios del agente. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+r"`
- **Se aplica:** en vivo

### `new_tab`

Nueva pestaña. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+t"`
- **Se aplica:** en vivo

### `split_vertical`

Dividir en vertical. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+d"`
- **Se aplica:** en vivo

### `split_horizontal`

Dividir en horizontal. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+d"`
- **Se aplica:** en vivo

### `close_pane`

Cerrar el panel. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+w"`
- **Se aplica:** en vivo

### `close_tab`

Cerrar la pestaña. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+w"`
- **Se aplica:** en vivo

### `copy_last_output`

Copiar la salida del último comando. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+c"`
- **Se aplica:** en vivo

### `previous_block`

Bloque anterior. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+up"`
- **Se aplica:** en vivo

### `next_block`

Bloque siguiente. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+down"`
- **Se aplica:** en vivo

### `block_rerun`

Relanzar el bloque seleccionado. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** en vivo

### `block_rerun_failed`

Relanzar solo los tests fallidos del bloque. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** en vivo

### `block_copy`

Copiar la salida del bloque seleccionado. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** en vivo

### `block_stop`

Detener el bloque en ejecución. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** en vivo

### `block_collapse`

Colapsar o expandir la salida del bloque. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `""`
- **Se aplica:** en vivo

### `block_open`

Abrir el error del bloque en el editor. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+o"`
- **Se aplica:** en vivo

### `previous_tab`

Pestaña anterior. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+["`
- **Se aplica:** en vivo

### `next_tab`

Pestaña siguiente. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+]"`
- **Se aplica:** en vivo

### `focus_left`

Panel de la izquierda. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+alt+left"`
- **Se aplica:** en vivo

### `focus_right`

Panel de la derecha. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+alt+right"`
- **Se aplica:** en vivo

### `focus_up`

Panel de arriba. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+alt+up"`
- **Se aplica:** en vivo

### `focus_down`

Panel de abajo. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+alt+down"`
- **Se aplica:** en vivo

### `toggle_zoom`

Ampliar o restaurar el panel. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+return"`
- **Se aplica:** en vivo

### `resize_left`

Mover el divisor a la izquierda. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+ctrl+left"`
- **Se aplica:** en vivo

### `resize_right`

Mover el divisor a la derecha. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+ctrl+right"`
- **Se aplica:** en vivo

### `resize_up`

Mover el divisor hacia arriba. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+ctrl+up"`
- **Se aplica:** en vivo

### `resize_down`

Mover el divisor hacia abajo. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+ctrl+down"`
- **Se aplica:** en vivo

### `open_settings`

Ajustes. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+,"`
- **Se aplica:** en vivo

### `report_issue`

Informar de un fallo. Atajo como `cmd+shift+d` (⌘, ⇧, ⌃ como `ctrl`, ⌥ como `alt`; teclas `left`, `return`, `tab`…); vacío, sin atajo.

- **Tipo:** un texto
- **Por defecto:** `"cmd+shift+i"`
- **Se aplica:** en vivo

## Ejemplo

```toml
[font]
family = "JetBrains Mono"
size = 14

[colors]
background = "#101014"

[notifications]
long_command_seconds = 30
```
