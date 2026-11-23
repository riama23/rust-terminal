# Verificación manual de M2+M3

Lista de comprobación del hito M2+M3 (ventana, hilos, entrada y mediciones). Los tests automáticos no cubren estos puntos porque necesitan la ventana real y el teclado.

## Cómo probar

1. Generar y abrir la app con logs:
   ```sh
   scripts/bundle.sh release
   open -n --stderr /tmp/rt.log --env RT_FRAME_STATS=1 --env RT_LOG_UNHANDLED=1 target/release/rust-terminal.app
   ```
2. Si queda detrás de otra ventana, ir a ella con Cmd+Tab. macOS no pinta una ventana tapada.
3. Al terminar, revisar `/tmp/rt.log`. Debe mostrar `[frames/s] 0` en reposo y las secuencias no soportadas que hayan llegado.

**Estado (2026-11-23):** se ha probado parte de la lista y no ha fallado nada. Marca cada punto al confirmarlo.

## Teclado y acentos

- [ ] `echo $LANG` muestra un locale `.UTF-8`.
- [ ] `´` seguido de `e` escribe `é`; `ñ` sale tal cual; Option+2 escribe `@`; Option+3 escribe `#`.
- [ ] `sleep 10` y Ctrl+C lo interrumpe; Ctrl+R busca en el historial; las flechas recorren el historial.

## Programas a pantalla completa

- [ ] `vim`: las flechas y Esc funcionan, y al salir con `:q` vuelve la pantalla anterior.
- [ ] `nvim` y `tmux` arrancan sin quedarse esperando unos segundos.
- [ ] `less` sobre un archivo, `htop` (sin ratón) y `git log` con colores.

## Claude Code y pegado

- [ ] `claude` arranca, se dibuja bien y acepta texto.
- [ ] Cmd+V pega texto de varias líneas sin ejecutarlo de golpe (bracketed paste).

## Redimensionado, emoji y rendimiento

- [ ] Al cambiar el tamaño de la ventana, `stty size` lo refleja y el contenido no se estira de forma rara.
- [ ] `echo 🙂🚀 日本語` ocupa 2 celdas por carácter.
- [ ] `cat` de un archivo grande no congela la ventana, por ejemplo `head -c 5000000 target/release/rust-terminal`.
- [ ] `RT_FRAME_STATS` marca 0 frames/s en reposo.
- [ ] `scripts/bench.sh vtebench` dentro de la terminal produce un `.dat` en `target/bench/`.

## Limitaciones conocidas (no son fallos)

- El texto en composición no se dibuja: tras `´` no aparece nada hasta la segunda tecla.
- No hay scrollback, selección ni Cmd+C; llegan en M4.
- El ratón no se reporta a las aplicaciones; llega en M6.
