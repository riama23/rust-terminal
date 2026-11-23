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

### Throughput, 2026-11-23

`vtebench` con 3 s por benchmark, 100×30, fuente de 13 pt, en el Mac del usuario. Media por muestra en ms: **menos es mejor**. Ghostty y Alacritty (0.17, compilado con `cargo install`) son de la primera comparación; rust-terminal aparece antes y después de las optimizaciones del mismo día (`0460fb9`, `e07039b`).

| Benchmark | Alacritty | Ghostty | rust-terminal (antes) | rust-terminal (ahora) |
|---|---:|---:|---:|---:|
| dense_cells | 7,76 | 12,02 | 8,55 | **7,86** |
| medium_cells | 9,89 | 6,62 | 10,02 | 10,01 |
| scrolling | 51,70 | 29,46 | 41,95 | 33,68 |
| scrolling_bottom_region | 11,62 | 20,13 | 32,67 | 22,79 |
| scrolling_bottom_small_region | 12,42 | 20,12 | 32,88 | 23,69 |
| scrolling_fullscreen | 49,94 | 31,60 | 52,55 | 35,62 |
| scrolling_top_region | 37,67 | 19,50 | 31,20 | 23,00 |
| scrolling_top_small_region | 11,55 | 20,27 | 33,17 | 23,44 |
| sync_medium_cells | 12,96 | 6,10 | 10,35 | 10,08 |
| unicode | 7,28 | 4,62 | 8,28 | 8,30 |

**Lectura:**
- Por delante o a la par de Alacritty en `dense_cells`, `medium_cells`, `scrolling`, `scrolling_fullscreen`, `scrolling_top_region` y `sync_medium_cells`.
- Por detrás de Ghostty en casi todo, salvo `dense_cells`. La mayor distancia está en `unicode`, `medium_cells` y `sync_medium_cells`.
- Las regiones pequeñas (un salto de línea cada 2 bytes) siguen siendo el punto más débil frente a Alacritty: unos 23 ms frente a 12.

**Pendiente de investigar:**
- En `y\r\n` en región, el núcleo solo (`bench-core`) ya va a 91 MiB/s, unos 31 ns por línea. El resto del tiempo se va en el lector de la PTY, el bloqueo de `Term` y el render.
- Con muchas celdas cambiadas (`medium_cells`, `unicode`), el render reconstruye todos los quads y un buffer de Metal en cada frame.

### Latencia

_Pendiente: requiere la medición manual con Typometer (`scripts/latency.sh`)._
