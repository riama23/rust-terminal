# rust-terminal

> El nombre es provisional, hasta que se decida el del producto.

Emulador de terminal para macOS escrito en Rust, pensado para trabajar con agentes de terminal como Claude Code. No es una shell: dentro se ejecuta tu shell de siempre (zsh por defecto).

- **Rápido y nativo:** AppKit, Metal y Core Text, sin capas intermedias.
- **Sin gasto en reposo:** si no cambia nada en pantalla, no se dibuja ningún frame.
- **Buen anfitrión de aplicaciones de terminal:**
  - colores de 24 bits, pantalla alternativa (vim, htop, less…) y caracteres anchos (CJK y emoji);
  - pegado seguro (bracketed paste), salida sincronizada y protocolo de teclado de kitty (por ejemplo, Shift+Enter en Claude Code);
  - ratón en las aplicaciones (htop, vim, tmux), enlaces y portapapeles por OSC 52;
  - notificaciones del sistema.
- **Pestañas y paneles divididos**, con una barra de pestañas propia.
- **Bloques de comandos:** con zsh, la terminal sabe dónde empieza y acaba cada comando y con qué código terminó. Puedes saltar entre comandos y copiar la salida del último.

> **Estado:** en desarrollo, todavía no sirve para el uso diario. Mira [Limitaciones actuales](#limitaciones-actuales).

## Requisitos

- Un Mac con Apple Silicon (arm64) y macOS 27 o posterior.
- Rust, instalado con [rustup](https://rustup.rs).
- Command Line Tools de Xcode (`xcode-select --install`). No hace falta Xcode completo.

## Instalación

Clona el repositorio y genera la aplicación:

```sh
scripts/bundle.sh release
```

Se crea `target/release/RustTerminal.app`. Puedes abrirla con doble clic o con:

```sh
open target/release/RustTerminal.app
```

Si quieres tenerla en el Launchpad y en Spotlight, cópiala a `/Applications`. La app solo lleva firma local (ad hoc) y no está notarizada, así que es para uso personal en tu propio Mac.

Para desarrollar también puedes lanzarla directamente con `cargo run`.

## Uso

La terminal abre tu shell (`$SHELL`, o `/bin/zsh` si no está definida) como shell de login, en tu directorio personal.

| Acción | Atajo |
|---|---|
| Nueva pestaña | Cmd+T |
| Dividir en vertical / en horizontal | Cmd+D / Cmd+Shift+D |
| Mover el foco entre paneles | Cmd+Option+flechas |
| Cerrar el panel / la pestaña | Cmd+W / Cmd+Shift+W |
| Ir a la pestaña 1…9 | Cmd+1…9 |
| Pestaña anterior / siguiente | Cmd+Shift+[ / Cmd+Shift+] |
| Abrir un enlace | Cmd+clic |
| Copiar la selección | Cmd+C |
| Copiar la salida del último comando | Cmd+Shift+C |
| Pegar | Cmd+V |
| Ir al comando anterior / siguiente | Cmd+↑ / Cmd+↓ |
| Salir | Cmd+Q |

- **Pestañas y paneles:** las pestañas y los paneles nuevos se abren en la carpeta del panel activo. Un clic en un panel le da el foco y los divisores se pueden arrastrar. Si al cerrar hay un programa en marcha (por ejemplo, vim o Claude Code), la terminal pide confirmación.
- **Historial:** desplázate con el trackpad o la rueda (hasta 10.000 líneas). Al escribir, la vista vuelve abajo. En programas a pantalla completa como `less` o `vim`, la rueda desplaza el documento.
- **Selección:** arrastra para seleccionar, haz doble clic para seleccionar una palabra y triple clic para una línea. Shift+clic amplía la selección y Option+arrastrar selecciona un rectángulo.
- **Bloques (solo zsh):** cada comando lleva una marca en el margen izquierdo, que sale roja si falló. Para eso la terminal añade una pequeña integración a zsh sin tocar tus archivos de configuración. Puedes desactivarla abriendo la app con la variable `RT_NO_SHELL_INTEGRATION=1`.
- **Teclado:** la **Option izquierda** funciona como Meta (Option+b/f para moverse por palabras en zsh, Option+Enter…). La **Option derecha** escribe caracteres como en el resto de macOS; en el teclado español, por ejemplo, Option+2 da `@`. Las teclas muertas (`´` y luego `e` → `é`) funcionan.
- **Ratón en las aplicaciones:** si un programa usa el ratón (htop, vim con `mouse=a`, tmux), los clics le llegan a él. Para seleccionar texto de todas formas, mantén Shift.
- **Enlaces:** mantén Cmd para ver subrayados los enlaces y las URL; Cmd+clic los abre.
- **Portapapeles:** los programas pueden copiar al portapapeles (OSC 52, útil por ssh o en tmux), pero no pueden leerlo.
- **Notificaciones:** con la ventana en segundo plano, los programas pueden mostrar notificaciones (Claude Code, por ejemplo). También avisa cuando termina un comando de más de 10 segundos. Funcionan al abrir la app como `.app`; la primera vez, macOS pide permiso.
- **Configuración:** la fuente es SF Mono de 13 pt y los colores son fijos. Ambas cosas se podrán configurar más adelante.

## Limitaciones actuales

- Solo hay una ventana; las pestañas no se pueden reordenar arrastrándolas.
- Los bloques solo funcionan con zsh.
- Mientras se escribe una tecla muerta no se ve nada hasta pulsar la segunda tecla.
- No hay archivo de configuración.

## Solución de problemas

Para obtener información de diagnóstico, abre la app con estas variables:

```sh
open -n --stderr /tmp/rt.log --env RT_LOG_UNHANDLED=1 --env RT_FRAME_STATS=1 target/release/RustTerminal.app
```

- `RT_LOG_UNHANDLED=1` apunta las secuencias de escape que la terminal todavía no entiende. Si un programa se ve mal, adjunta ese log al avisar.
- `RT_FRAME_STATS=1` muestra cada segundo los frames dibujados.

## Rendimiento

```sh
scripts/bench.sh core       # velocidad del intérprete de la terminal
scripts/bench.sh vtebench   # ejecútalo dentro de la terminal que quieras medir
```

Los resultados se guardan en `target/bench/`.
