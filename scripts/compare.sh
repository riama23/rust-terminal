#!/usr/bin/env bash
# Compara el throughput (vtebench) de esta terminal con Ghostty, Alacritty y Warp.
#
#   scripts/compare.sh            # 5 s por benchmark y terminal (~4 minutos)
#   RT_BENCH_SECS=3 scripts/compare.sh
#   RT_COMPARE="rust-terminal ghostty" scripts/compare.sh   # solo esas
#
# Abre una ventana de cada terminal, una tras otra, con el mismo tamaño
# (100×30) y la misma fuente (13 pt). No toques el teclado mientras corre y
# deja las ventanas a la vista: una ventana tapada puede dejar de pintar.
# Necesita /Applications/Ghostty.app, `alacritty` (cargo install alacritty) y,
# para Warp, /Applications/Warp.app.
#
# WARP no se puede lanzar con tamaño ni fuente por línea de comandos: se abre
# una ventana nueva con una *launch configuration* temporal
# (~/.warp/launch_configurations/rt-bench.yaml, que se borra al terminar) y
# mide con el tamaño y la fuente que tenga Warp. Para que sea comparable,
# ponle antes 100×30 y 13 pt en Ajustes → Apariencia. El tamaño real de cada
# terminal queda en `<terminal>.size` y en el resumen. La ventana de Warp se
# cierra sola (`exit`); tus otras ventanas y sesiones no se tocan.
# Resultados en target/bench/compare-<fecha>/ (milisegundos: menos es mejor).
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || . "$HOME/.cargo/env"

SECS="${RT_BENCH_SECS:-5}"
STAMP="$(date +%Y%m%d-%H%M%S)"
OUT="$PWD/target/bench/compare-$STAMP"
mkdir -p "$OUT"

VTEBENCH="$(scripts/bench.sh build-vtebench | tail -1)"
BENCHMARKS="$PWD/target/vtebench/benchmarks"
APP="$PWD/$(scripts/bundle.sh release | tail -1)"
ALACRITTY="$(command -v alacritty || echo "$HOME/.cargo/bin/alacritty")"

# Anota el tamaño real (filas columnas) y lanza vtebench.
run() { echo "stty size > $OUT/$1.size; $VTEBENCH -b $BENCHMARKS --max-secs $SECS --silent --dat $OUT/$1.dat"; }
WANT="${RT_COMPARE:-rust-terminal ghostty alacritty warp}"
want() { [[ " $WANT " == *" $1 "* ]]; }

if want rust-terminal; then
    echo "→ rust-terminal…"
    open -nW "$APP" --args -e /bin/sh -c "$(run rust-terminal)"
fi

if want ghostty && [[ -d /Applications/Ghostty.app ]]; then
    echo "→ Ghostty…"
    open -nW -a Ghostty --args --quit-after-last-window-closed=true --wait-after-command=false \
        --window-width=100 --window-height=30 --font-size=13 -e /bin/sh -c "$(run ghostty)"
fi

if want alacritty && [[ -x "$ALACRITTY" ]]; then
    echo "→ Alacritty…"
    "$ALACRITTY" -o window.dimensions.columns=100 -o window.dimensions.lines=30 -o font.size=13 \
        -e /bin/sh -c "$(run alacritty)"
fi

if want warp && [[ -d /Applications/Warp.app ]]; then
    echo "→ Warp… (ventana nueva con el tamaño y la fuente de Warp; se cierra sola)"
    CFG_DIR="$HOME/.warp/launch_configurations"
    CFG="$CFG_DIR/rt-bench.yaml"
    if [[ -e "$CFG" ]]; then
        echo "  ya existe $CFG: bórralo o renómbralo y vuelve a ejecutar." >&2
    else
        mkdir -p "$CFG_DIR"
        trap 'rm -f "$CFG"' EXIT
        # Sin comillas simples dentro del comando: va en un escalar YAML entre comillas simples.
        cat > "$CFG" <<YAML
---
name: rt-bench
windows:
  - tabs:
      - title: rt-bench
        layout:
          cwd: "$PWD"
          commands:
            - exec: '$(run warp); touch $OUT/warp.done; exit'
YAML
        open "warp://launch/rt-bench"
        # vtebench dura unos 10 benchmarks × SECS; se espera con margen.
        for _ in $(seq 1 $(( (SECS * 12 + 90) / 2 ))); do
            [[ -e "$OUT/warp.done" ]] && break
            sleep 2
        done
        if [[ -e "$OUT/warp.done" ]]; then
            rm -f "$OUT/warp.done"
        else
            echo "  Warp no terminó a tiempo: revisa su ventana (¿pidió algo al arrancar?)." >&2
        fi
        rm -f "$CFG"
        trap - EXIT
    fi
fi

# Resumen: media de las muestras de cada benchmark (ms; menos es mejor).
summary="$OUT/resumen.txt"
awk '
    FNR == 1 { file = FILENAME; sub(/.*\//, "", file); sub(/\.dat$/, "", file); terms[++nt] = file;
               for (i = 1; i <= NF; i++) { name[file, i] = $i; if (!(i in col)) { col[i] = $i; nc = (i > nc ? i : nc) } } next }
    { for (i = 1; i <= NF; i++) if ($i != "_") { sum[file, i] += $i; n[file, i]++ } }
    END {
        printf "%-32s", "benchmark (ms, menos es mejor)"; for (t = 1; t <= nt; t++) printf "%16s", terms[t]; printf "\n";
        for (i = 1; i <= nc; i++) {
            printf "%-32s", col[i];
            for (t = 1; t <= nt; t++) { k = terms[t] SUBSEP i; printf "%16s", (n[k] ? sprintf("%.2f", sum[k] / n[k]) : "-") }
            printf "\n";
        }
    }' "$OUT"/*.dat | tee "$summary"
# Tamaño real de cada terminal (para comprobar que son comparables).
echo
echo "Tamaño (filas columnas):"
for f in "$OUT"/*.size; do
    [[ -e "$f" ]] && printf "  %-16s %s\n" "$(basename "$f" .size)" "$(cat "$f")"
done | tee -a "$summary"
# Warp abre con el tamaño que tenga configurado: si no son 100×30, las cifras
# de Warp no se pueden comparar con las demás (vtebench pinta más o menos celdas).
if [[ -e "$OUT/warp.size" && "$(cat "$OUT/warp.size")" != "30 100" ]]; then
    echo
    echo "AVISO: Warp midió con $(cat "$OUT/warp.size") (filas columnas), no con 30 100: sus cifras NO son comparables con las demás." | tee -a "$summary"
    echo "       Ponle 100×30 y 13 pt en Ajustes → Apariencia (ventanas nuevas con tamaño personalizado) y repite: RT_COMPARE=warp scripts/compare.sh" | tee -a "$summary"
fi
echo "Resultados: $OUT"
