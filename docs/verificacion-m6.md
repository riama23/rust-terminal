# Verificación manual de M6

Lista de comprobación del hito M6: funciones de buen anfitrión. Los tests automáticos cubren:

- **Núcleo:** modos, DECRQM, XTVERSION, pilas de kitty, OSC 8/52/9/777/99 y URL.
- **Codificación de teclado y ratón:** casos de las especificaciones de kitty y xterm.
- **Render:** el subrayado de enlaces.

Estos puntos necesitan la ventana real y las aplicaciones.

## Cómo probar

```sh
scripts/bundle.sh release
open -n --stderr /tmp/rt.log --env RT_FRAME_STATS=1 --env RT_LOG_UNHANDLED=1 target/release/RustTerminal.app
```

Las notificaciones solo funcionan desde el `.app`. La primera vez, macOS pide permiso.

## Teclado

- [ ] En Claude Code, Shift+Enter inserta un salto de línea en lugar de enviar (teclado kitty).
- [ ] La Option izquierda funciona como Meta: Option+b y Option+f mueven por palabras en zsh, y Option+Enter funciona en Claude Code.
- [ ] La Option derecha sigue escribiendo @ (Option+2), # (Option+3), [ ] { }.
- [ ] En nvim, Ctrl+i y Tab se distinguen si la configuración lo usa; Ctrl+flechas funcionan.
- [ ] `kitten show-key -m kitty`, si está instalado, muestra los eventos de pulsar y soltar.

## Ratón

- [ ] htop: un clic en una columna cambia el orden y la rueda desplaza la lista.
- [ ] vim con `:set mouse=a`: un clic mueve el cursor, arrastrar selecciona en modo visual y la rueda desplaza.
- [ ] tmux con `set -g mouse on`: un clic cambia de panel de tmux y arrastrar el borde lo redimensiona.
- [ ] Con una aplicación que use el ratón, Shift+arrastrar selecciona texto de la terminal.
- [ ] En `less` (sin ratón) la rueda sigue desplazando el documento.

## Enlaces

- [ ] Con Cmd pulsado, al pasar sobre una URL (`echo https://example.com`) se subraya y aparece la mano; Cmd+clic la abre en el navegador.
- [ ] Enlaces OSC 8: `printf '\e]8;;https://example.com\e\\enlace\e]8;;\e\\\n'` muestra "enlace" y Cmd+clic lo abre.
- [ ] `ls --hyperlink=auto` (GNU coreutils), si está disponible: Cmd+clic abre el archivo.

## Portapapeles (OSC 52)

- [ ] `printf '\e]52;c;%s\a' "$(printf hola | base64)"` copia "hola" al portapapeles.
- [ ] Copiar en vim/nvim con un plugin OSC 52, o en tmux con `set -g set-clipboard on`, llega al portapapeles de macOS.
- [ ] `printf '\e]52;c;?\a'` no responde nada (la lectura está denegada).

## Notificaciones y campana (con la ventana en segundo plano)

- [ ] `sleep 3; printf '\e]9;Hola\a'` y cambiar a otra app: aparece la notificación.
- [ ] `sleep 3; printf '\e]777;notify;Título;Cuerpo\a'`: notificación con título.
- [ ] `sleep 12` y cambiar a otra app: al terminar aparece "Comando terminado · sleep 12 · 12 s"; con `sleep 12; false`, "Comando fallido".
- [ ] Claude Code avisa al terminar una tarea con la ventana en segundo plano (según su configuración de notificaciones).
- [ ] `sleep 3; printf '\a'` en segundo plano: el icono del Dock rebota una vez.
- [ ] Con la ventana activa no aparecen notificaciones.

## Foco y salida sincronizada

- [ ] vim/nvim con `autoread` recarga un archivo cambiado fuera al volver a la ventana (eventos de foco).
- [ ] Claude Code y nvim se redibujan sin parpadeo ni frames a medias (DEC 2026).
- [ ] `scripts/bench.sh vtebench` dentro de la terminal incluye `sync_medium_cells` sin errores.
- [ ] `RT_FRAME_STATS` sigue en 0 frames/s en reposo.
