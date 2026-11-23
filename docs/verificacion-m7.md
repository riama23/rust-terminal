# Verificación manual de M7

Lista de comprobación del hito M7: configuración TOML con recarga en caliente. Los tests automáticos cubren:

- la lectura (valores válidos e inválidos, claves desconocidas, errores de sintaxis con su línea);
- que los valores por defecto salen de la tabla de opciones y que `docs/configuracion.md` está al día;
- el vigilante (escritura en el sitio y guardado por renombrado);
- el historial configurable, la fuente de respaldo y OSC 52.

Estos puntos necesitan la ventana real.

## Cómo probar

```sh
scripts/bundle.sh release
open -n --stderr /tmp/rt.log --env RT_FRAME_STATS=1 target/release/rust-terminal.app
```

El archivo es `~/.config/rust-terminal/config.toml` (todas las opciones están en [configuracion.md](configuracion.md)). Para probar sin tocar tu configuración, abre la app con `--env XDG_CONFIG_HOME=/tmp/prueba` y edita `/tmp/prueba/rust-terminal/config.toml`.

## En vivo (al guardar el archivo)

- [ ] `[font] size = 16` agranda el texto en todos los paneles; las aplicaciones (vim, Claude Code) se ajustan al nuevo número de columnas.
- [ ] `[font] family = "Menlo"` (o una fuente instalada) cambia la fuente; una familia que no existe usa SF Mono.
- [ ] `[colors]` (fondo, texto, cursor, selección y `palette`) cambia los colores al momento; `ls --color` usa la paleta nueva.
- [ ] `[window] padding = 0` quita el margen.
- [ ] Guardar con vim (`:w`), con VS Code y con `echo … >` recarga en todos los casos.
- [ ] `[notifications] enabled = false` deja de notificar; `long_command_seconds = 3` avisa con `sleep 4` en segundo plano.
- [ ] `[clipboard] write = false`: `printf '\e]52;c;%s\a' "$(printf hola | base64)"` ya no copia.
- [ ] `[bell] dock_bounce = false`: la campana en segundo plano ya no hace rebotar el Dock.

## Pestañas nuevas

- [ ] `[shell] program = "/bin/bash"`: las pestañas y paneles nuevos abren bash; las existentes siguen con zsh.
- [ ] `[shell] integration = false`: las pestañas nuevas no tienen marcas de bloques.
- [ ] `[scrollback] lines = 100`: en una pestaña nueva, `seq 1 1000` solo conserva unas 100 líneas de historial.

## Portapapeles: lectura (OSC 52)

Para probarlo: `printf '\e]52;c;?\a'; cat -v` y luego Ctrl+C.

- [ ] Con `read = "deny"` (por defecto) no llega nada.
- [ ] Con `read = "ask"` aparece un aviso; "Permitir" envía el contenido (se ve como `^[]52;c;…`) y "Denegar" no envía nada.
- [ ] Con `read = "allow"` se envía sin preguntar.

## Errores

- [ ] Un valor no válido (`size = "grande"`) muestra una hoja con el error, sin bloquear la ventana; el resto de opciones se aplica.
- [ ] Un TOML roto (`[font`) muestra el error con su línea y se conserva la configuración anterior.
- [ ] Al arreglar el archivo, se aplica sin reiniciar.
- [ ] Un error en la configuración al arrancar muestra la hoja nada más abrir la ventana.

## Rendimiento

- [ ] Con la app en reposo, `RT_FRAME_STATS` sigue en 0 frames/s y el vigilante no consume CPU (Monitor de Actividad).
