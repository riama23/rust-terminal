#!/bin/sh
# Captura solo la ventana de rust-terminal (no el resto de la pantalla).
# Uso: scripts/window-shot.sh [salida.png]   (por defecto /tmp/rt-window.png)
# Requiere el permiso de Grabación de pantalla para la terminal que lo ejecuta.
# Primero captura por identificador de ventana; si macOS no puede, captura el
# rectángulo que ocupa esa ventana.
out="${1:-/tmp/rt-window.png}"
info=$(osascript -l JavaScript -e '
ObjC.import("CoreGraphics");
const l = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1, 0)));
const w = l.filter(x => x.kCGWindowOwnerName === "rust-terminal" && x.kCGWindowLayer === 0);
w.length ? [w[0].kCGWindowNumber, w[0].kCGWindowBounds.X, w[0].kCGWindowBounds.Y, w[0].kCGWindowBounds.Width, w[0].kCGWindowBounds.Height].join(" ") : "";
' 2>/dev/null)
[ -n "$info" ] || { echo "no hay ventana de rust-terminal" >&2; exit 1; }
set -- $info
id=$1; x=$2; y=$3; w=$4; h=$5
if screencapture -x -o -l "$id" "$out" 2>/dev/null; then echo "$out"; exit 0; fi
screencapture -x -R "$x,$y,$w,$h" "$out" && echo "$out"
