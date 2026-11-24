# Verificación manual: v1.5 (polish de splits)

Hecho en esta entrega: atenuado, cabecera por panel, zoom, resize por teclado y spinner de pestañas. Bloques visuales y paleta de comandos siguen pendientes.

- [ ] Con 2 o más paneles en la pestaña, cada uno lleva una **cabecera de 30 pt** con su título; el panel con el foco tiene la línea de acento. Con un solo panel (o con zoom) no hay cabecera. Se apaga con `window.pane_headers = false`.
- [ ] Los paneles **sin foco se ven al 55 %**; el cursor de esos paneles es hueco. Se apaga con `window.dim_unfocused = false`. Con un solo panel no se atenúa.
- [ ] **⌘⇧↵** amplía el panel con el foco a toda la pestaña y vuelve a pulsarlo lo restaura. Dividir o cerrar el panel ampliado lo restaura. Con un solo panel no hace nada. Menú *Ventana → Ampliar o restaurar el panel*.
- [ ] **⌘⌃←↑↓→** mueven el divisor más cercano del panel con el foco en ese eje (5 % por pulsación, entre 10 % y 90 %). Si no hay divisor en ese eje, no pasa nada.
- [ ] Una pestaña con un programa en primer plano (`sleep 10`, `vim`, `claude`) muestra un **spinner** braille a la izquierda del título. Desaparece al volver al prompt. Con la ventana en reposo y sin programas corriendo no se pide ningún frame.
- Sin comprobar con las manos: arrastrar divisores con las cabeceras puestas y el aspecto con la escala 1x.

**Coste del spinner:** mientras haya una pestaña ocupada se repinta unas 8 veces por segundo (un `claude` abierto cuenta como ocupado). Es la excepción a «cero frames en reposo»; si molesta, hay que añadir una opción.
