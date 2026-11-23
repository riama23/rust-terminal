# Verificación manual de M4

Lista de comprobación del hito M4: historial, reflow, bloques, integración con zsh y selección. Los tests automáticos cubren el núcleo, la integración con una zsh real y el render. Estos puntos necesitan la ventana real, el trackpad y tu configuración de zsh (oh-my-zsh y Powerlevel10k).

## Cómo probar

```sh
scripts/bundle.sh release
open -n --stderr /tmp/rt.log --env RT_FRAME_STATS=1 --env RT_LOG_UNHANDLED=1 target/release/rust-terminal.app
```

Si la ventana queda detrás de otra, ve a ella con Cmd+Tab.

## Historial y reflow

- [ ] `seq 1 20000`: con el trackpad se sube y se baja por el historial con suavidad; las primeras líneas (por debajo de unas 10.000) ya no están.
- [ ] Mientras se mira el historial, `while true; do date; sleep 0.2; done` no mueve la vista; al escribir algo, la vista vuelve abajo.
- [ ] Una línea larga (`printf 'x%.0s' {1..300}; echo`) se reajusta al estrechar y ensanchar la ventana, sin perder ni duplicar caracteres.
- [ ] Redimensionar con el cursor en el prompt (estrechando y ensanchando) deja un solo prompt, sin restos del prompt derecho de p10k.
- [ ] Redimensionar con **Claude Code** activo no deja frames duplicados en el historial.
- [ ] `clear` y luego `printf '\e[3J'` vacían también el historial.

## Bloques

- [ ] Cada prompt tiene una marca gris en el margen izquierdo; tras `false`, la de ese comando sale roja.
- [ ] Cmd+↑ y Cmd+↓ saltan al prompt anterior o siguiente.
- [ ] Cmd+Shift+C tras `ls -la` copia solo la salida de `ls` (sin el prompt ni el comando).
- [ ] Tras `printf foo` (sin salto de línea), Cmd+Shift+C copia `foo` y la marca `%` de zsh sigue apareciendo como siempre.
- [ ] Pulsar Enter con la línea vacía no crea bloques extra.
- [ ] El arranque no muestra avisos de p10k sobre salida durante la inicialización (*instant prompt*).
- [ ] Con `RT_NO_SHELL_INTEGRATION=1` (`open … --env RT_NO_SHELL_INTEGRATION=1`) no aparecen marcas y la shell funciona igual.

## Selección y copiar

- [ ] Arrastrar selecciona; un clic simple deselecciona.
- [ ] Doble clic selecciona una palabra o una ruta completa (`/usr/local/bin`); triple clic, la línea entera, aunque esté partida en varias filas.
- [ ] Shift+clic amplía la selección; Option+arrastrar selecciona un rectángulo.
- [ ] Arrastrar por encima o por debajo de la ventana desplaza la vista y sigue seleccionando.
- [ ] Cmd+C copia; al pegar en otra app, las líneas partidas por el ancho salen unidas y sin espacios finales.
- [ ] La selección funciona también sobre el historial y sobre caracteres anchos (日本語, emoji).

## Pantalla alternativa

- [ ] En `less` o `vim`, la rueda del trackpad desplaza el documento (envía flechas) en lugar del historial.

## Rendimiento

- [ ] `RT_FRAME_STATS` sigue en 0 frames/s en reposo, también después de desplazarse por el historial.
- [ ] `seq 1 1000000` termina en unos pocos segundos y la ventana sigue respondiendo.

## Limitaciones conocidas (no son fallos)

- La rueda en la pantalla alternativa envía flechas; las aplicaciones no reciben eventos de ratón hasta M6.
- Un comando que lea `$PROMPT_EOL_MARK` mientras se ejecuta ve el valor modificado por la integración.
- La integración solo existe para zsh.
