# Probar los agentes de Claude Code en otro Mac

Guía para instalar y probar la parte de agentes (v1.1–v1.4): sidebar de sesiones, aprobar o denegar permisos sin salir del teclado, notificaciones, revisión de cambios y comentarios. El resto de la terminal se prueba con `docs/semana-de-prueba.md`.

## Requisitos

Los de la terminal (Apple Silicon, macOS 27, Command Line Tools y Rust; ver `docs/semana-de-prueba.md`) y además:

- **Claude Code instalado y con sesión iniciada.** Comprueba que en una terminal normal `claude --version` funciona.
- **`git`** (viene con las Command Line Tools). La revisión de cambios lo usa.
- Una **shell zsh o bash**. Con otra (fish, nu…), los agentes se lanzan igualmente con `/bin/zsh`.

## Instalar

```sh
git clone git@github.com:riama23/rust-terminal.git
cd rust-terminal && scripts/install.sh
```

Para actualizar: `scripts/install.sh` en la misma carpeta, y Cmd+Q y volver a abrir.

## Permisos de macOS

Se piden la primera vez que hacen falta. Si dices que no por error, se reactivan en Ajustes del Sistema.

| Permiso | Para qué | Si falta |
| --- | --- | --- |
| **Notificaciones** | Avisos de permisos pendientes y de turnos terminados, con sus botones | La app escribe «notificaciones no permitidas» y no avisa. Actívalo en Ajustes › Notificaciones |
| **Red local** | Solo si Claude Code usa una pasarela en tu red local (`ANTHROPIC_BASE_URL` apuntando a una IP privada) | Claude se queda en «No internet route». Actívalo en Ajustes › Privacidad › Red local |

La app **no necesita** Accesibilidad ni Grabación de pantalla. El atajo global ⌃⌥N usa una API que no pide permisos.

## Probar

1. **Cmd+N** abre «Nuevo agente». Escribe una tarea, elige el modo y pulsa *Lanzar*. Se abre una pestaña con Claude Code y aparece la sidebar a la izquierda. El recuadro «Ejecuta exactamente» muestra la línea de `claude` que se lanzará.
2. Pide algo que necesite permiso (por ejemplo, crear un archivo). Debajo de la terminal aparece la **barra de atención** con la orden exacta:
   - **⌘↵** permite una vez, **⌘⌫** abre «Denegar…» (con un motivo opcional) y *Siempre permitir* enseña la regla antes de escribirla.
   - Si no se contesta o hay cualquier duda, **gana el prompt de Claude Code**: la app nunca permite por su cuenta.
3. Pasa a otra aplicación y vuelve a pedir un permiso: sale una **notificación** con *Permitir una vez*, *Enviar respuesta* y *Abrir*. Mientras haya sesiones que te esperan, el **Dock** muestra su número y la **barra de menús** lista los agentes.
4. Cuando el agente termina un turno aparece la **barra de terminado**. **⌘⇧R** (o su botón) abre la **revisión**: pasa el ratón por una línea, pulsa «+», escribe el comentario y, al final, *Enviar N comentarios a Claude*. Solo se envían con el agente terminado.
5. **⌃⌥N** desde cualquier aplicación abre «Nuevo agente».

La lista completa de comprobaciones está en `docs/verificacion-v1.2.md`, `v1.3` y `v1.4`. Lo que sale mal, como issue: **Cmd+Shift+I** en el panel afectado.

## Qué sigue sin hacerse

Descartar el worktree y abrir un PR desde la revisión, la vista dividida del diff, y que el aviso de «terminado» incluya archivos y tests. Lo que no se ha podido comprobar sin las manos (clics, atajos, notificaciones con acciones y el atajo global) está marcado como pendiente en esas listas.
