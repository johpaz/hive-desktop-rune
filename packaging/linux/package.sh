#!/usr/bin/env bash
# Empaqueta la app para Linux: un .deb y un tar.gz portátil. Uso: packaging/linux/package.sh [carpeta-salida]
# Necesita: cargo, dpkg-deb y el sidecar hive-gateway-rust (HIVE_GATEWAY_BIN o el del SDK Rust vecino).
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${1:-dist}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
ARCH_DEB="$(dpkg --print-architecture)"
if [ -z "${HIVE_GATEWAY_BIN:-}" ]; then
  cargo build --manifest-path ../hiveSDK-Rust/Cargo.toml --release -p hive-gateway-rust
fi
GATEWAY="${HIVE_GATEWAY_BIN:-../hiveSDK-Rust/target/release/hive-gateway-rust}"
[ -f "$GATEWAY" ] || { echo "No encuentro el sidecar: $GATEWAY (define HIVE_GATEWAY_BIN)" >&2; exit 1; }

cargo build --release --bin hive-desktop-rune
BIN=target/release/hive-desktop-rune
rm -rf "$OUT/stage" && mkdir -p "$OUT"

# El sidecar debe estar junto al ejecutable (gateway.rs lo busca ahí).
LIB="$OUT/stage/usr/lib/hive-agents"
mkdir -p "$LIB" "$OUT/stage/usr/bin" "$OUT/stage/usr/share/applications" "$OUT/stage/usr/share/icons/hicolor/128x128/apps" "$OUT/stage/DEBIAN"
install -m755 "$BIN" "$LIB/hive-desktop-rune"
install -m755 "$GATEWAY" "$LIB/hive-gateway-rust"
ln -s ../lib/hive-agents/hive-desktop-rune "$OUT/stage/usr/bin/hive-agents"
install -m644 packaging/linux/hive-agents.desktop "$OUT/stage/usr/share/applications/hive-agents.desktop"
install -m644 assets/logo.png "$OUT/stage/usr/share/icons/hicolor/128x128/apps/hive-agents.png"
cat > "$OUT/stage/DEBIAN/control" <<CTL
Package: hive-agents-rune
Version: $VERSION
Architecture: $ARCH_DEB
Maintainer: Hive <noreply@hive.local>
Depends: libcrypt1, libargon2-1, libasound2, libdbus-1-3, libwayland-client0, libxkbcommon0, libvulkan1
Description: Hive Agents (cliente nativo en Rust con Rune)
CTL
dpkg-deb --root-owner-group --build "$OUT/stage" "$OUT/hive-agents-rune_${VERSION}_${ARCH_DEB}.deb" >/dev/null

# Portátil: carpeta autocontenida.
P="hive-agents-rune-$VERSION-linux-$ARCH_DEB"
rm -rf "$OUT/$P" && mkdir -p "$OUT/$P"
cp "$LIB/hive-desktop-rune" "$LIB/hive-gateway-rust" "$OUT/$P/"
cp assets/logo.png "$OUT/$P/hive-agents.png"
cp packaging/linux/hive-agents.desktop "$OUT/$P/hive-agents.desktop.in"
cp packaging/linux/install-user.sh "$OUT/$P/install.sh"
tar -C "$OUT" -czf "$OUT/$P.tar.gz" "$P" && rm -rf "$OUT/$P"
echo "Listo en $OUT:"; ls -1 "$OUT" | grep -v '^stage$'
