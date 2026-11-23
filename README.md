# rust-terminal

> El nombre es provisional, hasta que se decida el del producto.

Emulador de terminal para macOS escrito en Rust, pensado para trabajar con agentes de terminal como Claude Code. No es una shell: dentro se ejecuta tu shell de siempre (zsh por defecto).

- **Rápido y nativo:** AppKit, Metal y Core Text, sin capas intermedias.
- **Sin gasto en reposo:** si no cambia nada en pantalla, no se dibuja ningún frame.
- **Buen anfitrión de aplicaciones de terminal:** colores de 24 bits, pantalla alternativa (vim, htop, less…), caracteres anchos (CJK y emoji) y pegado seguro (bracketed paste).

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
| Pegar | Cmd+V |
| Salir | Cmd+Q |

- **Teclado:** Option escribe caracteres, como en el resto de macOS. En el teclado español, por ejemplo, Option+2 da `@`. Las teclas muertas (`´` y luego `e` → `é`) funcionan.
- **Configuración:** la fuente es SF Mono de 13 pt y los colores son fijos. Ambas cosas se podrán configurar más adelante.

## Limitaciones actuales

- No hay scrollback, selección con el ratón ni copiar con Cmd+C.
- No hay pestañas ni paneles divididos.
- Las aplicaciones no reciben eventos de ratón (clics ni rueda).
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
