# Verificación manual: teclas de macOS

Compila con `scripts/bundle.sh` y abre la app. Todo en zsh salvo que se diga otra cosa.

## Option izquierda (Meta)
- [ ] ⌥← y ⌥→ saltan de palabra, sin escribir `D` ni `C`.
- [ ] ⌥⌫ borra la palabra anterior.
- [ ] ⌥Supr (⌥Fn⌫) borra la palabra siguiente.
- [ ] ⌃← y ⌃→ también saltan de palabra.

## Option derecha (compone)
- [ ] ⌥derecha + `2` escribe `@` en un teclado español; no actúa como Meta.
- [ ] `keyboard.option_as_meta = "both"`: la derecha pasa a ser Meta (recarga en caliente).
- [ ] `keyboard.option_as_meta = "none"`: ninguna Option es Meta.

## Command (edición de línea)
- [ ] ⌘← va al inicio de la línea y ⌘→ al final.
- [ ] ⌘⌫ borra hasta el inicio y ⌘Supr hasta el final.
- [ ] `keyboard.command_line_editing = false` los desactiva.
- [ ] Con un permiso de agente pendiente, ⌘⌫ sigue denegando (no borra línea).
- [ ] ⌘↑ y ⌘↓ siguen navegando entre bloques.

## Selección con Shift (línea de comandos de zsh)
Solo en pestañas nuevas (`keyboard.shift_select = true`, por defecto).
- [ ] ⇧← y ⇧→ seleccionan carácter a carácter (se ve resaltado).
- [ ] ⌥⇧← y ⌥⇧→ seleccionan por palabras.
- [ ] ⌘⇧← y ⌘⇧→ seleccionan hasta el inicio o el final de la línea; ⇧Inicio y ⇧Fin, igual.
- [ ] Teclear con una selección la reemplaza; ⌫ y Supr la borran.
- [ ] Mover el cursor sin Shift, Esc o Intro la quitan.
- [ ] Con p10k, autosuggestions o syntax-highlighting (si los usas): teclear sigue funcionando y los colores no se rompen.
- [ ] Un enlace propio de ⇧← en tu `.zshrc` se respeta.
- [ ] En vim o tmux, ⇧flecha llega a la aplicación como antes.
- [ ] `keyboard.shift_select = false` lo desactiva (en pestañas nuevas).

## Cmd+K
- [ ] Aparece en Edición → «Limpiar pantalla» con ⌘K.
- [ ] Borra el historial y la shell redibuja el prompt arriba.
- [ ] En vim o htop (pantalla alternativa) no envía nada a la aplicación.
- [ ] Se puede reasignar con `[keybindings] clear_screen`.

## Sin regresiones
- [ ] vim, tmux y Claude Code: las flechas y ⌥← siguen funcionando (en pantalla alternativa va la secuencia xterm).
- [ ] ⌘C, ⌘V, ⌘T, ⌘D, ⌘W y ⌘, siguen funcionando.
- [ ] Con kitty keyboard activo (p. ej. Claude Code con kitty), nada cambia.
