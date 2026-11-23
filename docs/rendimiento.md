# Rendimiento

Objetivo de la v1 (`CLAUDE.md`): latencia de entrada y throughput **iguales o mejores que Ghostty y Alacritty**, medidos en el Mac del usuario. Restricciones: memoria con historial grande y cero frames en reposo.

## Throughput (vtebench)

```sh
scripts/compare.sh                 # 5 s por benchmark y terminal (~4 min)
RT_BENCH_SECS=3 scripts/compare.sh # más rápido, menos preciso
```

- Abre una ventana de cada terminal, una tras otra, con 100×30 y fuente de 13 pt, y ejecuta `vtebench` (fijado a un commit) dentro de cada una.
- Resultado en `target/bench/compare-<fecha>/`: un `.dat` por terminal y `resumen.txt` con la media de cada benchmark en milisegundos (**menos es mejor**).
- No toques el teclado mientras corre y deja las ventanas a la vista: una ventana tapada puede dejar de pintar y falsear la comparación.
- Requisitos: `/Applications/Ghostty.app` y `alacritty`. El cask de Alacritty está desactivado en Homebrew (no pasa Gatekeeper), así que se instala con `cargo install --locked alacritty`.

Para medir solo el intérprete del núcleo, sin ventana: `scripts/bench.sh core`.

## Latencia de teclado (Typometer, manual)

```sh
scripts/latency.sh
```

Typometer (Java; `brew install openjdk`) escribe caracteres en una ventana y mide cuánto tardan en aparecer en pantalla.

1. La primera vez, macOS pide permisos de **Accesibilidad** y **Grabación de pantalla** para Java: concédelos en Ajustes del Sistema y vuelve a abrirlo.
2. Abre la terminal que vas a medir con un editor sencillo a pantalla completa (por ejemplo `vim -u NONE`) y deja la ventana a la vista.
3. En Typometer: *Benchmark* → coloca el cursor en el editor cuando lo pida → espera a que termine (unas 200 pulsaciones).
4. Anota la latencia mínima, media y máxima. Repite con Ghostty y Alacritty en las mismas condiciones (mismo tamaño de ventana, misma fuente y la misma pantalla).

## Resultados

_Pendientes de la primera comparación completa._
