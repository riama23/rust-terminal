# Verificación manual: v1.5 (polish de splits)

Hecho: atenuado, cabecera por panel, zoom, resize por teclado, spinner de pestañas y la primera capa de bloques visuales (tinte y barra de acciones). Pendiente: cabecera inline por bloque, colapsar, «relanzar solo fallidos», vuelta automática a Classic y la paleta de comandos.

- [ ] Con 2 o más paneles en la pestaña, cada uno lleva una **cabecera de 30 pt** con su título; el panel con el foco tiene la línea de acento. Con un solo panel (o con zoom) no hay cabecera. Se apaga con `window.pane_headers = false`.
- [ ] Los paneles **sin foco se ven al 55 %**; el cursor de esos paneles es hueco. Se apaga con `window.dim_unfocused = false`. Con un solo panel no se atenúa.
- [ ] **⌘⇧↵** amplía el panel con el foco a toda la pestaña y vuelve a pulsarlo lo restaura. Dividir o cerrar el panel ampliado lo restaura. Con un solo panel no hace nada. Menú *Ventana → Ampliar o restaurar el panel*.
- [ ] **⌘⌃←↑↓→** mueven el divisor más cercano del panel con el foco en ese eje (5 % por pulsación, entre 10 % y 90 %). Si no hay divisor en ese eje, no pasa nada.
- [ ] Una pestaña con un programa en primer plano (`sleep 10`, `vim`, `claude`) muestra un **spinner** braille a la izquierda del título. Desaparece al volver al prompt. Con la ventana en reposo y sin programas corriendo no se pide ningún frame.
- Sin comprobar con las manos: arrastrar divisores con las cabeceras puestas y el aspecto con la escala 1x.

**Coste del spinner:** mientras haya una pestaña ocupada se repinta unas 8 veces por segundo (un `claude` abierto cuenta como ocupado). Es la excepción a «cero frames en reposo»; si molesta, hay que añadir una opción.

## Bloques visuales (primera capa)

No hay cabecera dentro de la rejilla (haría falta reescribir scroll, selección y ratón): el bloque seleccionado se enseña con un tinte y una **barra de acciones** bajo la terminal.

- [ ] **⌘↑** selecciona el último comando ejecutado; ⌘↑ y ⌘↓ se mueven entre bloques y hacen scroll hasta ellos. Pasar del último con ⌘↓ quita la selección. Teclear también la quita.
- [ ] Un bloque **fallido** tiene el fondo tintado de rojo (más fuerte si está seleccionado); uno seleccionado correcto, de azul, con una barra de acento a la izquierda. No hay tinte con la pantalla alternativa (vim, htop).
- [ ] La barra muestra `✓ correcto · 1,2 s · comando` o `✕ código N · …`, con **Relanzar**, **Copiar salida** y, si la salida de un bloque fallido contiene `archivo:línea[:columna]` de un archivo que existe, **Abrir en el editor ⌘O**. Con el bloque en ejecución, solo **Detener** (Ctrl-C).
- [ ] **Relanzar** escribe el comando en la shell solo si está libre, y no acepta comandos con caracteres de control. Los mismos comandos están en el menú *Ver*.
- [ ] `blocks.editor` define el editor (`code -g {path}:{line}:{col}`, `zed {path}:{line}`…). Vacío: `open -t`. La orden se parte en palabras y no pasa por una shell.
- [ ] `blocks.visual = false` quita tinte y barra (quedan las marcas del margen).
- La barra no sustituye a la de permisos ni a la de terminado de un agente: ellas mandan.
- Sin comprobar a mano: los clics en los botones de la barra (por teclado y menú sí).

