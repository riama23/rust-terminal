#!/usr/bin/env bash
# Abre Typometer para medir la latencia de teclado (manual).
#
# Typometer escribe caracteres en la ventana elegida y mide cuánto tardan en
# aparecer en pantalla. La primera vez, macOS pide permisos de Accesibilidad
# y Grabación de pantalla para Java: concédelos y vuelve a ejecutar.
# Procedimiento: docs/rendimiento.md.
set -euo pipefail
cd "$(dirname "$0")/.."

JAVA="/opt/homebrew/opt/openjdk/bin/java"
[[ -x "$JAVA" ]] || { echo "Falta Java: brew install openjdk" >&2; exit 1; }

DIR="target/typometer"
JAR="$DIR/typometer-1.0.1/typometer-1.0.1.jar"
if [[ ! -f "$JAR" ]]; then
    mkdir -p "$DIR"
    gh release download v1.0.1 -R pavelfatin/typometer -p "typometer-1.0.1-bin.zip" -D "$DIR" --clobber
    echo "8afc176ef7b5b94021640be3089c7f7adbe94f0bdc5b15b84308e765fcb68d35  $DIR/typometer-1.0.1-bin.zip" | shasum -a 256 -c -
    (cd "$DIR" && unzip -oq typometer-1.0.1-bin.zip)
fi
exec "$JAVA" -jar "$JAR"
