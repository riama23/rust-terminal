# Semana de prueba

El criterio para dar la v1 por terminada es usar la terminal **una semana en lugar de Warp**, incluido el trabajo con Claude Code, sin volver a abrir Warp. Esta guía explica cómo prepararla y qué vigilar.

## Preparación en cada Mac

1. Requisitos: Apple Silicon, macOS 27, las Command Line Tools (`xcode-select --install`) y Rust ([rustup.rs](https://rustup.rs)).
2. Instalar:
   ```sh
   git clone git@github.com:riama23/rust-terminal.git
   cd rust-terminal && scripts/install.sh
   ```
3. Abrirla desde `/Applications` (o Spotlight: "rust-terminal") y fijarla en el Dock.
4. Opcional: copiar tu `~/.config/rust-terminal/config.toml` entre Macs.
5. Para actualizar cuando haya arreglos: `scripts/install.sh` en la carpeta del repositorio. Después, Cmd+Q y volver a abrir.

Para probar también los agentes de Claude Code (sidebar, permisos, revisión de cambios), sigue `docs/probar-agentes.md`.

## Cuando algo falle

Pulsa **Cmd+Shift+I** (menú Ayuda → Informar de un fallo…) **en el panel donde ha fallado**. Se abre la página de issue nueva de GitHub con un diagnóstico ya rellenado:

- versión y commit, macOS, ventana, fuente;
- estado del panel y último comando con su código de salida;
- las últimas secuencias de escape que la terminal no entendió (la pista más útil para fallos de dibujo);
- el texto visible del panel.

Escribe qué pasó y **revisa el diagnóstico antes de enviarlo**: el texto del panel puede contener rutas, tokens o salida de comandos que no quieras guardar en la issue. Bórralo si hace falta.

Si la terminal se cuelga o se cierra y no puedes usar el atajo, abre una issue a mano en <https://github.com/riama23/rust-terminal/issues/new> e indica qué estabas haciendo.

## Qué vigilar

- **Claude Code**: que se dibuje bien al redimensionar, el pegado de textos largos, Shift+Enter, las notificaciones al terminar y copiar con OSC 52.
- **Editores** (vim/nvim): ratón, colores, eventos de foco, cambiar de pestaña y volver.
- **Teclado español**: acentos con tecla muerta, ñ, @ # [ ] { } con la Option derecha, atajos con la Option izquierda.
- **Varias pestañas y paneles** durante horas: memoria (Monitor de Actividad) y que en reposo no consuma CPU.
- **ssh y tmux**: colores, ratón y portapapeles.
- **Cosas que echas de menos de Warp**: apúntalas también como issue (son candidatas para después de la v1).

Al final de la semana, la lista de issues abiertas dice qué falta para cerrar la v1.
