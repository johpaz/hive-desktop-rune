#!/usr/bin/env bash
# Empaqueta la app para Windows (se ejecuta con Git Bash en el runner): instalador .exe (NSIS) y .zip portátil.
# Uso: packaging/windows/package.sh <carpeta-salida>
# Necesita: HIVE_GATEWAY_BIN (hive-gateway-rust.exe), cargo, makensis y python (para generar el .ico).
set -euo pipefail
cd "$(dirname "$0")/../.."
OUT="${1:-dist}"
VERSION="$(sed -n 's/^version = "\(.*\)"/\1/p' Cargo.toml | head -1)"
[ -f "${HIVE_GATEWAY_BIN:-}" ] || { echo "Define HIVE_GATEWAY_BIN con hive-gateway-rust.exe" >&2; exit 1; }

cargo build --locked --release --bin hive-desktop-rune
STAGE="$OUT/stage"; rm -rf "$STAGE" && mkdir -p "$STAGE"
cp target/release/hive-desktop-rune.exe "$STAGE/"
cp "$HIVE_GATEWAY_BIN" "$STAGE/hive-gateway-rust.exe"
# .ico a partir del PNG (un .ico admite PNG embebido).
python - "$STAGE/hive-agents.ico" <<'PY'
import struct, sys
png = open('assets/logo.png', 'rb').read()
w, h = struct.unpack('>II', png[16:24])
w, h = (0 if w >= 256 else w), (0 if h >= 256 else h)
open(sys.argv[1], 'wb').write(struct.pack('<HHH', 0, 1, 1) + struct.pack('<BBBBHHII', w, h, 0, 0, 1, 32, len(png), 22) + png)
PY

abs() { cygpath -w "$(realpath "$1")" 2>/dev/null || realpath "$1"; }
# -D y no /D: Git Bash convertiría un argumento que empieza por «/» en una ruta de Windows.
makensis -V2 "-DVERSION=$VERSION" "-DSTAGE=$(abs "$STAGE")" "-DOUT=$(abs "$OUT")\\hive-agents-rune_${VERSION}_windows-x64-setup.exe" "$(abs packaging/windows/installer.nsi)"

P="hive-agents-rune-$VERSION-windows-x64"
mkdir -p "$OUT/$P" && cp "$STAGE/hive-desktop-rune.exe" "$STAGE/hive-gateway-rust.exe" "$STAGE/hive-agents.ico" "$OUT/$P/"
(cd "$OUT" && 7z a -tzip "$P.zip" "$P" >/dev/null) && rm -rf "$OUT/$P" "$STAGE"
echo "Listo en $OUT:"; ls -1 "$OUT"
