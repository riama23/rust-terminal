#!/usr/bin/env bash
# Benchmarks locales (no se ejecutan en CI).
#
#   scripts/bench.sh core       microbenchmark del núcleo (no necesita ventana)
#   scripts/bench.sh vtebench   vtebench; ejecútalo DENTRO de la terminal que
#                               quieras medir (la nuestra, Ghostty, Alacritty…)
#
# Los resultados se guardan en target/bench/.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || . "$HOME/.cargo/env"

VTEBENCH_REPO="https://github.com/alacritty/vtebench"
VTEBENCH_COMMIT="ead80032e57dee2e75f0b51f2ea67528647d9944"
OUT="target/bench"
STAMP="$(date +%Y%m%d-%H%M%S)"
mkdir -p "$OUT"

case "${1:-}" in
core)
    cargo run -q -p rust-terminal-core --release --bin bench-core | tee "$OUT/core-$STAMP.txt"
    ;;
build-vtebench)
    # Solo clona (fijado al commit) y compila vtebench; imprime su ruta.
    DIR="target/vtebench"
    if [[ ! -d "$DIR/.git" ]]; then
        git clone --quiet "$VTEBENCH_REPO" "$DIR"
    fi
    git -C "$DIR" fetch --quiet origin "$VTEBENCH_COMMIT" 2>/dev/null || true
    git -C "$DIR" checkout --quiet "$VTEBENCH_COMMIT"
    cargo build -q --release --manifest-path "$DIR/Cargo.toml"
    echo "$PWD/$DIR/target/release/vtebench"
    ;;
vtebench)
    DIR="target/vtebench"
    "$0" build-vtebench >/dev/null
    "$DIR/target/release/vtebench" -b "$DIR/benchmarks" --dat "$OUT/vtebench-$STAMP.dat"
    echo "Resultados: $OUT/vtebench-$STAMP.dat"
    ;;
*)
    sed -n '2,8p' "$0"
    exit 1
    ;;
esac
