# Verificación manual: v2 (API de control)

Referencia: `docs/api-control.md`. Experimental, protocolo `v0`.

- [ ] En una terminal de la app, `echo $RT_CONTROL_SOCKET` y `which rt` dan valores; `rt hello` contesta. Con `control.enabled = false` (y reiniciar) no hay variables.
- [ ] `rt sessions` lista las sesiones (la principal y, si está abierto, el terminal rápido) con su título, carpeta y marcas.
- [ ] `rt read <id>` devuelve lo que se ve; con `--lines 200` incluye el historial. `rt blocks <id>` y `rt output <id> <n>` dan los comandos y su salida (con la integración de zsh).
- [ ] **Escribir pide permiso:** `rt send <id> --enter "echo hola"` abre un diálogo con el texto exacto, el proceso (pid y orden) y los botones **Denegar** (por defecto), **Permitir una vez** y **Permitir en esta sesión**. Denegar devuelve `denied` y no escribe nada. «Permitir en esta sesión» no vuelve a preguntar a ese agente para ese grupo (teclear / abrir y cerrar). Tras un «no», el mismo agente no vuelve a preguntar durante 10 s.
- [ ] Tras escribir, la barra de pestañas dice «control: <agente>» unos segundos.
- [ ] `rt keys <id> ctrl+c` interrumpe un comando; `rt send` rechaza caracteres de control (ESC, Ctrl-C…).
- [ ] `rt open --cwd /tmp` abre una pestaña nueva y devuelve su id; `rt close <id>` la cierra.
- [ ] Al salir con ⌘Q desaparece la carpeta del socket (en `$TMPDIR/rust-terminal-control-*`).
- [ ] Con Claude Code dentro de un panel, pedirle que use `rt sessions` / `rt read` funciona y le sale el diálogo al escribir.

Comprobado con la app real y `rt` contra su socket (ayudas solo debug: `RT_CONTROL_DUMP` anota socket y token, y `RT_CONTROL_AUTO=allow|deny` contesta el permiso): `hello`, `sessions`, `read` con historial, `blocks`, `output`, `send` (el comando se ejecutó), `keys ctrl+c` (el `sleep` terminó con 130), rechazo de ESC en el texto, `open` y `close`, token malo (`unauthorized`), permisos `0700` y `0600`, el socket desaparece al salir, y con la respuesta «denegar» las lecturas siguen y las escrituras no ocurren. Sin comprobar a mano: el **diálogo de permiso real** (solo se probó con la respuesta automática), el aviso de la barra de pestañas y Claude Code usando `rt`.
