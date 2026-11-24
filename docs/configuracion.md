# Configuración

> Archivo generado a partir de `crates/app/src/config.rs`. No lo edites a mano: `RT_UPDATE_DOCS=1 cargo test -p rust-terminal config` lo regenera.

La configuración se lee de `~/.config/rust-terminal/config.toml` (o de `$XDG_CONFIG_HOME/rust-terminal/config.toml`). El archivo es opcional y todas las opciones tienen un valor por defecto.

Los cambios se aplican al guardar el archivo, sin reiniciar. Las opciones marcadas como **pestañas nuevas** solo afectan a las pestañas y paneles que se abran después. Si hay un error, se muestra un aviso y se ignora solo la opción incorrecta.

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
