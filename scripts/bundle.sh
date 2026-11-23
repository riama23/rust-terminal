#!/usr/bin/env bash
# Genera un .app local (sin notarizar, firma ad hoc) a partir del binario.
# Uso: scripts/bundle.sh [debug|release]   (por defecto: release)
# El nombre sale de PRODUCT_NAME (crates/core/src/lib.rs), provisional hasta decidirlo.
set -euo pipefail
cd "$(dirname "$0")/.."
command -v cargo >/dev/null || . "$HOME/.cargo/env"

PROFILE="${1:-release}"
NAME="$(sed -n 's/^pub const PRODUCT_NAME: \&str = "\(.*\)";/\1/p' crates/core/src/lib.rs)"
BUNDLE_ID="local.$NAME"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' crates/app/Cargo.toml | head -1)"

if [[ "$PROFILE" == "release" ]]; then
    cargo build --release -p rust-terminal
else
    cargo build -p rust-terminal
fi

APP="target/$PROFILE/$NAME.app"
rm -rf "$APP"
mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
cp "target/$PROFILE/rust-terminal" "$APP/Contents/MacOS/rust-terminal"
cp crates/app/assets/AppIcon.icns "$APP/Contents/Resources/AppIcon.icns"

cat > "$APP/Contents/Info.plist" <<PLIST
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0">
<dict>
    <key>CFBundleExecutable</key><string>rust-terminal</string>
    <key>CFBundleIdentifier</key><string>$BUNDLE_ID</string>
    <key>CFBundleName</key><string>$NAME</string>
    <key>CFBundleIconFile</key><string>AppIcon</string>
    <key>CFBundlePackageType</key><string>APPL</string>
    <key>CFBundleShortVersionString</key><string>$VERSION</string>
    <key>CFBundleVersion</key><string>$VERSION</string>
    <key>LSMinimumSystemVersion</key><string>27.0</string>
    <key>LSArchitecturePriority</key><array><string>arm64</string></array>
    <key>NSHighResolutionCapable</key><true/>
    <key>NSSupportsAutomaticGraphicsSwitching</key><true/>
</dict>
</plist>
PLIST

codesign --force --sign - "$APP" >/dev/null
echo "$APP"
