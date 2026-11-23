#!/bin/sh
# Captura solo la ventana de rust-terminal (no el resto de la pantalla).
# Uso: scripts/window-shot.sh [salida.png]   (por defecto /tmp/rt-window.png)
# Requiere el permiso de Grabación de pantalla para la terminal que lo ejecuta.
out="${1:-/tmp/rt-window.png}"
id=$(osascript -l JavaScript -e '
ObjC.import("CoreGraphics");
const l = ObjC.deepUnwrap(ObjC.castRefToObject($.CGWindowListCopyWindowInfo(1, 0)));
const w = l.filter(x => x.kCGWindowOwnerName === "rust-terminal" && x.kCGWindowLayer === 0);
w.length ? String(w[0].kCGWindowNumber) : "";
' 2>/dev/null)
[ -n "$id" ] || { echo "no hay ventana de rust-terminal" >&2; exit 1; }
screencapture -x -o -l "$id" "$out" && echo "$out"
