# Verificación manual: v1.6

Entrega A: teclado configurable y ventana de ajustes. Pendiente en la v1.6: tema Paper y *Follow system* (B), badge PROD (C) y quick terminal (D).

## Teclado configurable

- [ ] `[keybindings]` en `config.toml` reasigna cualquier acción del menú: `new_tab = "cmd+shift+t"`. Con `""` se quita el atajo. Todas están en `docs/configuracion.md`.
- [ ] Formato: `cmd`, `shift`, `ctrl`, `alt` y una tecla (`d`, `[`, `,`, `left`, `up`, `return`, `tab`, `space`, `f5`…). Una tecla normal sin ⌘, ⌃ ni ⌥ se rechaza (impediría escribir).
- [ ] Al guardar el archivo, los atajos del menú cambian sin reiniciar. Un atajo mal escrito o repetido avisa en la hoja de errores de configuración y se ignora (o, si está repetido, gana el primero del menú).
- [ ] La paleta de comandos muestra los atajos configurados.
- Comprobado: el volcado del menú con `RT_DUMP_MENU` (`newTab` con ⇧⌘, `closePane` sin atajo). Sin comprobar a mano: pulsar los atajos nuevos.

## Ajustes (⌘,)

- [ ] **⌘,** (menú de la app → *Ajustes…*) abre una ventana de tamaño fijo de 960 × 680 con páginas: General, Apariencia, Teclado y Bloques y agentes. Cada fila es una opción de `config.toml` con su descripción y su `sección.clave`.
- [ ] Casillas para las booleanas, campos de texto para texto, números, colores y listas (separadas por comas), y menús desplegables para las de elegir. La paleta de 16 colores solo se edita en el archivo.
- [ ] Cambiar una opción **escribe su línea en `config.toml`** conservando comentarios y el resto del archivo; la configuración se recarga sola. Un valor no válido (fuera de rango, un atajo mal escrito…) no se guarda y se explica en rojo.
- [ ] «Restablecer» (en las opciones puestas en el archivo) quita la línea. «Abrir config.toml» lo abre en el editor de texto.
- [ ] Un valor que no cambia no se escribe (al cambiar de página no se toca el archivo).
- Comprobado: pulsar una casilla (`RT_OPEN_SETTINGS` y `RT_SETTINGS_CLICK`, solo debug) escribió `[blocks] visual = false` sin tocar el resto, y capturas de la ventana. Sin comprobar a mano: escribir en los campos de texto, los menús desplegables y «Restablecer».
