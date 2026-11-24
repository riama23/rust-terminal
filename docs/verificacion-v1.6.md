# Verificación manual: v1.6

Entrega A (hecha): teclado configurable y ventana de ajustes. Entrega B (hecha): tema Paper y *Follow system*. Entrega C (hecha): badge PROD. Entrega D: quick terminal.

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

## Badge PROD (entrega C)

- [ ] `[ssh] production_hosts = ["prod-*", "*.prod.example.com", "db1"]` (también en Ajustes → General): patrones con `*` y `?`, sin distinguir mayúsculas.
- [ ] Una pestaña (o un panel, en su cabecera) cuyo primer plano es un `ssh` a un host que coincide lleva el distintivo naranja **PROD**. Se compara con el destino escrito (`ssh me@prod-db` → `prod-db`; entiende `-p 2222`, `-tt`, `-J salto destino`, `ssh://…` e IPv6). Si usas alias de `~/.ssh/config`, pon el alias.
- [ ] El distintivo aparece al arrancar el comando (se vuelve a mirar el primer plano a los 0,4, 2 y 5 s, porque `ssh` no escribe nada mientras conecta) y desaparece al volver al prompt.
- **Fuera de esta entrega:** el distintivo en la sidebar de shells (la sidebar solo lista agentes) y la *protección de producción* del editor de entrada (que pide confirmar comandos en hosts PROD); el roadmap marca el editor de entrada como fuera de alcance.
- Comprobado: la detección con un `ssh` de mentira (un enlace a `sleep` llamado `ssh`): `prod=true` mientras corre y con el patrón que coincide, `false` si no coincide y al terminar (volcado `RT_DUMP_TABS`, solo debug); el parser de `ssh`, los patrones y el dibujo del distintivo en pestaña y cabecera (tests). Sin comprobar a mano: verlo en la ventana y con un `ssh` real.

## Quick terminal (entrega D)

- [ ] **⌥Espacio** desde cualquier aplicación baja un panel desde el borde superior de la pantalla donde está el ratón (16 pt de margen a los lados, 410 pt de alto por defecto) con una animación de unos 0,2 s (sin ella con *Reducir movimiento*). Volver a pulsarlo lo sube y devuelve el foco a la aplicación en la que estabas.
- [ ] Fondo translúcido sobre un desenfoque (opacidad `quick_terminal.opacity`, 0,88); opaco con *Reducir transparencia*. Esquinas redondeadas. Se ve sobre cualquier espacio y sobre apps a pantalla completa.
- [ ] Es una terminal completa (pestañas, splits, bloques, paleta, PROD) con sus propias shells, que **siguen vivas** al ocultarlo. La primera vez tarda un instante en crearse.
- [ ] **Se oculta al perder el foco** (clic en otra aplicación o ventana) **salvo que esté fijado**: **⌘⌥P** (menú *Ventana*) lo fija o lo suelta y la barra de pestañas muestra «fijado». No se oculta mientras hay un diálogo de la app abierto.
- [ ] **⌘⌥W** (*Mover el terminal rápido a una ventana*) lo convierte en una ventana normal con barra de título; el siguiente ⌥Espacio crea un terminal rápido nuevo. Si sales de la última shell del terminal rápido (`exit`), se descarta y el siguiente ⌥Espacio crea otro (no cierra la app).
- [ ] Opciones (Ajustes → Terminal rápido): `quick_terminal.enabled` (con `false` no se reserva ⌥Espacio; se aplica al reiniciar), `height` y `opacity`. Los atajos ⌘⌥P y ⌘⌥W son configurables en `[keybindings]` (`quick_pin`, `quick_to_window`).
- **Fuera de esta entrega:** redimensionar arrastrando la barra inferior (el alto se fija en `quick_terminal.height`) y cambiar el atajo global (⌥Espacio es fijo; ⌥Espacio también escribe un espacio sin ruptura en algunas disposiciones de teclado, por eso se puede desactivar). La *protección de producción* sigue fuera.
- Comprobado con la app real (solo debug: `RT_OPEN_QUICK`, `RT_QUICK_STEPS`, `RT_QUICK_DUMP`): el panel aparece pegado al borde superior de la pantalla (16 pt de margen, 1648 × 410, nivel flotante) y se ve con el tema, la barra de pestañas y la shell; perder el foco lo oculta, fijado no lo oculta, y mover a ventana deja una ventana normal de 1000 × 620 y borra el terminal rápido. La geometría y las constantes de Carbon, con tests.
- **Sin comprobar a mano:** pulsar ⌥Espacio de verdad (el registro no dio error), la animación, el desenfoque sobre otras aplicaciones, los atajos ⌘⌥P/⌘⌥W con el panel sin activar la app (es un panel que no activa la aplicación; si los atajos del menú no llegaran, habría que activar la app al mostrarlo), y devolver el foco a la aplicación anterior.

