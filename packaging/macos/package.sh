#!/usr/bin/env bash
# Empaqueta la app para macOS: Hive Agents.app y un .dmg. Uso: packaging/macos/package.sh <carpeta-salida>
# Necesita: HIVE_GATEWAY_BIN (sidecar hive-gateway-rust de macOS), cargo, hdiutil, sips e iconutil (vienen con macOS).
# Sin firma ni notarización: macOS pedirá abrirla con clic derecho > Abrir la primera vez (ver README).
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${1:-dist}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH="$(uname -m)"
[ -f "${HIVE_GATEWAY_BIN:-}" ] || { echo "Define HIVE_GATEWAY_BIN con el sidecar de macOS" >&2; exit 1; }

cargo build --locked --release --bin hive-desktop-rune
APP="$OUT/stage/Hive Agents.app"
rm -rf "$OUT/stage" && mkdir -p "$APP/Contents/MacOS" "$APP/Contents/Resources"
install -m755 target/release/hive-desktop-rune "$APP/Contents/MacOS/hive-desktop-rune"
install -m755 "$HIVE_GATEWAY_BIN" "$APP/Contents/MacOS/hive-gateway-rust"

# Icono: PNG -> .icns
ICONSET="$OUT/stage/icon.iconset"; mkdir -p "$ICONSET"
for s in 16 32 64 128 256; do
  sips -z $s $s assets/logo.png --out "$ICONSET/icon_${s}x${s}.png" >/dev/null
  sips -z $((s*2)) $((s*2)) assets/logo.png --out "$ICONSET/icon_${s}x${s}@2x.png" >/dev/null
done
iconutil -c icns "$ICONSET" -o "$APP/Contents/Resources/hive-agents.icns"
rm -rf "$ICONSET"

cat > "$APP/Contents/Info.plist" <<PL
<?xml version="1.0" encoding="UTF-8"?>
<!DOCTYPE plist PUBLIC "-//Apple//DTD PLIST 1.0//EN" "http://www.apple.com/DTDs/PropertyList-1.0.dtd">
<plist version="1.0"><dict>
  <key>CFBundleName</key><string>Hive Agents</string>
  <key>CFBundleDisplayName</key><string>Hive Agents</string>
  <key>CFBundleIdentifier</key><string>app.hive.agents.rune</string>
  <key>CFBundleExecutable</key><string>hive-desktop-rune</string>
  <key>CFBundleIconFile</key><string>hive-agents</string>
  <key>CFBundleVersion</key><string>$VERSION</string>
  <key>CFBundleShortVersionString</key><string>$VERSION</string>
  <key>CFBundlePackageType</key><string>APPL</string>
  <key>LSMinimumSystemVersion</key><string>12.0</string>
  <key>NSHighResolutionCapable</key><true/>
  <key>NSMicrophoneUsageDescription</key><string>Hive usa el micrófono para la voz y las reuniones.</string>
  <key>NSCameraUsageDescription</key><string>Hive usa la cámara en las llamadas de voz.</string>
</dict></plist>
PL
# Firma ad-hoc: sin ella los binarios arm64 no arrancan en Apple Silicon.
codesign --force --deep --sign - "$APP"

DMG="$OUT/hive-agents-rune_${VERSION}_macos-${ARCH}.dmg"
ln -s /Applications "$OUT/stage/Applications"
hdiutil create -volname "Hive Agents" -srcfolder "$OUT/stage" -ov -format UDZO "$DMG" >/dev/null
rm -rf "$OUT/stage"
echo "Listo: $DMG"
