#!/usr/bin/env bash
# Compara el throughput (vtebench) de esta terminal con Ghostty y Alacritty.
#
#   scripts/compare.sh            # 5 s por benchmark y terminal (~4 minutos)
#   RT_BENCH_SECS=3 scripts/compare.sh
#
# Abre una ventana de cada terminal, una tras otra, con el mismo tamaño
# (100×30) y la misma fuente (13 pt). No toques el teclado mientras corre y
# deja las ventanas a la vista: una ventana tapada puede dejar de pintar.
# Necesita /Applications/Ghostty.app y `alacritty` (cargo install alacritty).
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

run() { echo "$VTEBENCH -b $BENCHMARKS --max-secs $SECS --silent --dat $OUT/$1.dat"; }

echo "→ rust-terminal…"
open -nW "$APP" --args -e /bin/sh -c "$(run rust-terminal)"

if [[ -d /Applications/Ghostty.app ]]; then
    echo "→ Ghostty…"
    open -nW -a Ghostty --args --quit-after-last-window-closed=true --wait-after-command=false \
        --window-width=100 --window-height=30 --font-size=13 -e /bin/sh -c "$(run ghostty)"
fi

if [[ -x "$ALACRITTY" ]]; then
    echo "→ Alacritty…"
    "$ALACRITTY" -o window.dimensions.columns=100 -o window.dimensions.lines=30 -o font.size=13 \
        -e /bin/sh -c "$(run alacritty)"
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
echo "Resultados: $OUT"
