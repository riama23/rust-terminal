# Verificación manual: v1.6

Entrega A (hecha): teclado configurable y ventana de ajustes. Entrega B: tema Paper y *Follow system*. Pendiente: badge PROD (C) y quick terminal (D).

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

## Tema Paper y Follow system (entrega B)

- [ ] `[appearance] theme = "dusk" | "paper" | "system"` (también en Ajustes → Apariencia). **Dusk** es el oscuro de siempre, **Paper** el claro y **system** sigue al sistema.
- [ ] Con Paper cambian la terminal (fondo `#FBFBFA`, texto `#1C1D21`, cursor azul, selección azul claro, paleta ANSI propia), la barra de pestañas, los divisores y todas las vistas de la app (sidebar, barras de permiso, terminado y bloque, inspector, Review, paleta, ajustes), y la ventana pasa a apariencia clara (barra de título y controles nativos).
- [ ] **Se cambia en caliente**: editar `config.toml` (o la casilla de Ajustes) cambia el tema sin reiniciar. Con `system`, alternar entre claro y oscuro en Ajustes del Sistema lo cambia solo.
- [ ] Los colores que pongas en `[colors]` mandan sobre los del tema (`colors.background` fijo se queda aunque cambies de tema).
- **Desviaciones del spec (contraste):** el spec pide 4,5:1 para el texto y sus valores `ok` (`#1F8A4C`) y `attention` (`#A86400`) de Paper no lo cumplen sobre su fondo (4,2:1 y 4,5:1 sobre la barra lateral). Se usan `#1B7F45` y `#9A5A00`. La paleta ANSI de Paper (que el spec deja sin definir) se deriva de los colores de estado y **todos sus colores cumplen 4,5:1** sobre el fondo, incluido el «blanco» (7 y 15), que es un gris oscuro porque un blanco no se leería. Lo comprueban tests.
- Comprobado: las vistas de AppKit (ajustes y paleta) en Paper con capturas, y el cambio en caliente Dusk → Paper (el fondo de la ventana de ajustes pasó de `#131417` a `#FBFBFA` al editar el archivo). Sin comprobar a mano: la terminal con Paper a la vista (usa los mismos colores del tema que Dusk, sin código propio), el modo `system` al alternar el sistema y las pantallas de agentes en Paper.

