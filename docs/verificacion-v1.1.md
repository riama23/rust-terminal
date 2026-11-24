# Verificación manual de la v1.1

Lista de comprobación de la v1.1: la ventana pasa a tener un contenedor (`ContainerView`, `crates/app/src/container.rs`) y la `TerminalView` es una subvista. Los tests automáticos cubren el reparto de anchos (`split_widths`). Estos puntos necesitan la ventana real.

**Criterio de salida:** la terminal se comporta exactamente igual que antes, y se puede añadir una vista lateral sin tocar el render.

## Cómo probar

```sh
scripts/bundle.sh release
open -n --stderr /tmp/rt.log --env RT_FRAME_STATS=1 target/release/rust-terminal.app
# con un panel lateral de prueba de 240 pt a la izquierda:
open -n --stderr /tmp/rt.log --env RT_DEBUG_SIDEBAR=1 target/release/rust-terminal.app
```

## Sin panel lateral (comportamiento previo)

- [ ] Repetir las listas `verificacion-m2-m3.md` a `verificacion-m7.md`: ninguna regresión.
- [ ] El teclado, las teclas muertas y la composición (acentos, ñ) funcionan; el texto compuesto aparece en el cursor.
- [ ] El ratón: clic en pestañas y paneles, arrastre de divisores, selección, scroll del trackpad y Cmd+clic en enlaces.
- [ ] Redimensionar la ventana en vivo no estira ni parpadea el contenido.
- [ ] Pantalla completa y volver: el tamaño de la cuadrícula (`stty size`) es correcto.
- [ ] Mover la ventana entre una pantalla Retina y otra con distinta escala redibuja nítido.
- [ ] En reposo, `RT_FRAME_STATS` marca 0 frames/s.

## Con `RT_DEBUG_SIDEBAR=1`

- [ ] Aparece una franja oscura de 240 pt a la izquierda y la terminal ocupa el resto.
- [ ] La barra de pestañas y los paneles se dibujan solo en el área de la terminal.
- [ ] El ratón sobre la terminal da la celda correcta (selecciona el texto bajo el puntero, no desplazado 240 pt).
- [ ] Cmd+clic en un enlace abre el enlace bajo el puntero.
- [ ] Al estrechar la ventana, la franja se encoge antes de que la terminal baje de 320 pt.
- [ ] El IME y las teclas muertas muestran el texto compuesto en el sitio correcto.
- [ ] Las notificaciones y el rebote del Dock siguen funcionando.

## Regresión encontrada y corregida

El refactor dejó varios `downcast` de `contentView()` a `TerminalView` apuntando a una vista que ya no era esa. Dejaron de funcionar, sin error, los avisos de sesión (notificaciones, OSC 52, campana), la recarga de configuración, los cambios de título y **la confirmación de Cmd+Q con un programa en marcha**. Ahora todos pasan por `container::terminal_view`, y no debe volver a hacerse un `downcast` a mano.

Comprobado con `osascript` (`tell application "rust-terminal" to quit`): con la shell inactiva la app cierra; con un programa en marcha (`-e zsh -ic "sleep 300; true"`) sigue viva, es decir, pide confirmación. No se contrastó contra el binario anterior al arreglo.

## Estado

El reparto calculado se comprobó con una traza temporal (ya retirada): sin panel, la terminal ocupa `(0,0,812,505)`; con el panel de 240 pt, queda en `(240,0,572,505)`. **Comprobado a ojo** (`scripts/window-shot.sh`, captura solo la ventana): sin panel, la terminal se pinta con su barra de pestañas y el prompt; con `RT_DEBUG_SIDEBAR=1`, el panel ocupa 240 pt a la izquierda y la terminal el resto. **Sigue pendiente la comprobación del ratón** (celda bajo el puntero con el panel puesto) y el resto de la lista.

**Regla para toda vista propia:** desde macOS 14 una vista no recorta su dibujo a sus límites. Hay que rellenar `self.bounds()`, no el `rect` que llega a `drawRect:`; si no, el relleno invade a las vistas vecinas. Una ventana lanzada desde otra app no pinta Metal hasta ponerla al frente (`osascript -e 'tell application "rust-terminal" to activate'`).
