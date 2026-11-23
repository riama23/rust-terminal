#!/usr/bin/env bash
# Regenera crates/app/assets/AppIcon.icns a partir del dibujo de src/icon.rs.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || . "$HOME/.cargo/env"

WORK="$(mktemp -d)"
trap 'rm -rf "$WORK"' EXIT
RT_ICON_OUT="$WORK/icon-1024.png" cargo test -q -p rust-terminal render_app_icon -- --ignored >/dev/null

SET="$WORK/AppIcon.iconset"
mkdir -p "$SET"
for size in 16 32 128 256 512; do
    sips -z $size $size "$WORK/icon-1024.png" --out "$SET/icon_${size}x${size}.png" >/dev/null
    double=$((size * 2))
    sips -z $double $double "$WORK/icon-1024.png" --out "$SET/icon_${size}x${size}@2x.png" >/dev/null
done
mkdir -p crates/app/assets
iconutil -c icns "$SET" -o crates/app/assets/AppIcon.icns
cp "$WORK/icon-1024.png" crates/app/assets/AppIcon-1024.png
echo "crates/app/assets/AppIcon.icns"
