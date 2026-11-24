#!/usr/bin/env bash
# Instala o actualiza la terminal en este Mac, compilándola desde el código.
#
# La primera vez:
#   git clone git@github.com:riama23/rust-terminal.git
#   cd rust-terminal && scripts/install.sh
#
# Para actualizar: scripts/install.sh   (trae los cambios con git pull)
# Sin actualizar el código: scripts/install.sh --no-pull
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
    echo "✗ $*" >&2
    exit 1
}

# Requisitos.
[[ "$(uname -m)" == "arm64" ]] || fail "Solo funciona en Macs con Apple Silicon (arm64)."
major="$(sw_vers -productVersion | cut -d. -f1)"
(( major >= 27 )) || fail "Necesita macOS 27 o posterior (este Mac tiene $(sw_vers -productVersion))."
xcode-select -p >/dev/null 2>&1 || fail "Faltan las Command Line Tools. Instálalas con: xcode-select --install"
if ! command -v cargo >/dev/null; then
    [[ -f "$HOME/.cargo/env" ]] && . "$HOME/.cargo/env"
fi
command -v cargo >/dev/null || fail "Falta Rust. Instálalo con: curl --proto '=https' --tlsv1.2 -sSf https://sh.rustup.rs | sh"

# Código al día.
if [[ "${1:-}" != "--no-pull" ]]; then
    echo "→ Actualizando el código…"
    git pull --ff-only
fi

echo "→ Compilando (la primera vez tarda unos minutos)…"
APP="$(scripts/bundle.sh release | tail -1)"
NAME="$(basename "$APP")"

# RT_INSTALL_DIR permite instalar en otro sitio (útil para probar el script):
# se crea si hace falta y se respeta siempre, sin caer a otra carpeta.
if [[ -n "${RT_INSTALL_DIR:-}" ]]; then
    DEST="$RT_INSTALL_DIR"
    mkdir -p "$DEST" || fail "No se pudo crear $DEST."
    [[ -w "$DEST" ]] || fail "No se puede escribir en $DEST."
else
    DEST="/Applications"
    [[ -w "$DEST" ]] || DEST="$HOME/Applications"
    mkdir -p "$DEST"
fi
rm -rf "${DEST:?}/$NAME"
cp -R "$APP" "$DEST/"

echo "✓ Instalada en $DEST/$NAME (commit $(git rev-parse --short HEAD))."
if pgrep -f "$NAME/Contents/MacOS" >/dev/null; then
    echo "  La versión anterior sigue abierta: ciérrala (Cmd+Q) y vuelve a abrirla para usar la nueva."
fi
