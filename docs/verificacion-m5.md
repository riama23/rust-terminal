# Verificación manual de M5

Lista de comprobación del hito M5: pestañas con barra propia y paneles divididos. Los tests automáticos cubren el modelo (divisiones, cierre, foco por dirección, arrastre del divisor y pestañas), la PTY (directorio de arranque y SIGHUP) y el render. Estos puntos necesitan la ventana real.

## Cómo probar

```sh
scripts/bundle.sh release
open -n --stderr /tmp/rt.log --env RT_FRAME_STATS=1 --env RT_LOG_UNHANDLED=1 target/release/rust-terminal.app
```

## Pestañas

- [ ] Cmd+T abre una pestaña nueva detrás de la activa, en el mismo directorio que el panel activo.
- [ ] El "+" de la barra también abre una pestaña; un clic en una pestaña la activa y su "×" la cierra.
- [ ] Cmd+1…9 va a cada pestaña; Cmd+Shift+[ y Cmd+Shift+] pasan a la anterior o la siguiente.
- [ ] El título de cada pestaña es el de la aplicación (OSC 0/2), o si no la carpeta actual; los largos se recortan con "…".
- [ ] La ventana lleva el título de la pestaña activa.
- [ ] Con muchas pestañas (10 o más), todas siguen siendo accesibles.

## Paneles

- [ ] Cmd+D divide en vertical (panel a la derecha) y Cmd+Shift+D en horizontal (panel debajo), en el mismo directorio.
- [ ] Cmd+Option+flechas mueve el foco; un clic en un panel también.
- [ ] El panel con foco tiene el cursor macizo; los demás, hueco.
- [ ] El divisor se puede arrastrar; las aplicaciones de cada panel se ajustan al nuevo tamaño (`stty size`, vim).
- [ ] Divisiones anidadas (por ejemplo, tres o cuatro paneles) funcionan y se cierran sin dejar huecos.
- [ ] La selección, el scroll del trackpad y Cmd+Shift+C actúan sobre el panel correspondiente.

## Cerrar y salir

- [ ] Cmd+W en un panel con solo el prompt lo cierra sin preguntar; con `vim` o Claude Code abiertos, pide confirmación.
- [ ] Cmd+Shift+W y la "×" de una pestaña con un programa en marcha piden confirmación; "Cancelar" no cierra nada.
- [ ] `exit` en una shell cierra su panel; si era el último, la pestaña; si era la última pestaña, la app.
- [ ] Cmd+Q con un programa en marcha en cualquier pestaña pide confirmación.
- [ ] Al cerrar un panel, su shell y sus procesos terminan (no quedan procesos huérfanos en `ps`).

## Rendimiento

- [ ] Con varias pestañas abiertas y en reposo, `RT_FRAME_STATS` marca 0 frames/s.
- [ ] Una salida continua en una pestaña oculta (`while true; do date; sleep 0.1; done`) no genera frames.
- [ ] Claude Code en un panel y vim en otro, a la vez, sin artefactos.

## Fuera de M5

- Varias ventanas (Cmd+N), reordenar pestañas arrastrándolas y pestañas en la barra de título.
