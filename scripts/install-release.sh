#!/usr/bin/env bash
# Instala una versión publicada: clona su tag y la compila con scripts/install.sh.
#
# Este archivo es la plantilla de install-vX.Y.0.sh, el script que el workflow
# de release adjunta a cada release (con la versión ya fijada en TAG).
#
# Uso (el repositorio es privado: hace falta acceso por SSH):
#   gh release download vX.Y.0 -R riama23/rust-terminal -p 'install-*.sh'
#   bash install-vX.Y.0.sh
set -euo pipefail

TAG="${1:-@TAG@}"
REPO="git@github.com:riama23/rust-terminal.git"

fail() {
    echo "✗ $*" >&2
    exit 1
}

[[ "$TAG" =~ ^v[0-9]+\.[0-9]+\.0$ ]] || fail "Versión no válida: $TAG. Usa el script install-vX.Y.0.sh del release, o pasa el tag: $0 vX.Y.0"
command -v git >/dev/null || fail "Falta git."

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT

echo "→ Clonando ${TAG}…"
git clone --quiet --depth 1 --branch "$TAG" "$REPO" "$WORK/rust-terminal" ||
    fail "No se pudo clonar $TAG. ¿Tienes acceso SSH al repositorio y existe el tag?"

# El tag es una versión fija: no hay nada que actualizar con git pull.
"$WORK/rust-terminal/scripts/install.sh" --no-pull
