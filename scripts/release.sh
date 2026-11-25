#!/usr/bin/env bash
# Crea el tag de una versión minor o major (vX.Y.0). El tag dispara el workflow
# .github/workflows/release.yml, que compila y publica el release en GitHub.
#
# Uso: scripts/release.sh X.Y.0 [--push]
#   Sin --push solo crea el tag en local y explica cómo subirlo.
#   Con --push lo sube a origin (sin preguntar: pensado para automatizarlo).
#
# Antes: subir `version` en [workspace.package] de Cargo.toml, regenerar
# Cargo.lock (cargo check) y fusionar ese cambio en main.
set -euo pipefail
cd "$(dirname "$0")/.."

fail() {
    echo "✗ $*" >&2
    exit 1
}

VERSION="${1:-}"
[[ "$VERSION" =~ ^[0-9]+\.[0-9]+\.0$ ]] || fail "Uso: scripts/release.sh X.Y.0 [--push] (solo minor o major; los patch no llevan tag)."
TAG="v$VERSION"
PUSH="${2:-}"
[[ -z "$PUSH" || "$PUSH" == "--push" ]] || fail "Opción desconocida: $PUSH"

[[ "$(git rev-parse --abbrev-ref HEAD)" == "main" ]] || fail "Hay que estar en main."
[[ -z "$(git status --porcelain --untracked-files=no)" ]] || fail "El árbol de trabajo tiene cambios sin confirmar."
git fetch --quiet origin main
[[ "$(git rev-parse HEAD)" == "$(git rev-parse origin/main)" ]] || fail "main no coincide con origin/main (haz pull o push antes)."

CARGO_VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[[ "$CARGO_VERSION" == "$VERSION" ]] || fail "Cargo.toml dice $CARGO_VERSION y el tag sería $VERSION. Sube la versión primero."

git rev-parse -q --verify "refs/tags/$TAG" >/dev/null && fail "El tag $TAG ya existe."

git tag -a "$TAG" -m "Release $TAG"
echo "✓ Tag $TAG creado en $(git rev-parse --short HEAD)."

if [[ "$PUSH" == "--push" ]]; then
    git push origin "$TAG"
    echo "✓ Subido. El workflow Release publicará el artefacto en GitHub."
else
    echo "  Para publicarlo: git push origin $TAG"
    echo "  Para deshacerlo: git tag -d $TAG"
fi
