#!/usr/bin/env bash
# Instalación de usuario, sin administrador: copia la app a ~/.local y crea el lanzador del menú.
# Uso: ./install.sh            (instala o actualiza)
#      ./install.sh --uninstall (quita lo instalado; no toca ~/.hive)
set -euo pipefail
HERE="$(cd "$(dirname "$0")" && pwd)"
PREFIX="${HIVE_PREFIX:-$HOME/.local}"
APP="$PREFIX/opt/hive-agents-rune"
BIN="$PREFIX/bin/hive-agents"
DESKTOP="$PREFIX/share/applications/hive-agents.desktop"
ICON="$PREFIX/share/icons/hicolor/128x128/apps/hive-agents.png"

if [ "${1:-}" = "--uninstall" ]; then
  rm -rf "$APP" "$BIN" "$DESKTOP" "$ICON"
  command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
  echo "Hive Agents (Rune) desinstalado. Tus datos en ~/.hive no se tocaron."
  exit 0
fi

mkdir -p "$APP" "$PREFIX/bin" "$PREFIX/share/applications" "$(dirname "$ICON")"
# El sidecar debe quedar junto al ejecutable (gateway.rs lo busca ahí).
install -m755 "$HERE/hive-desktop-rune" "$APP/hive-desktop-rune"
install -m755 "$HERE/hive-gateway-rust" "$APP/hive-gateway-rust"
install -m644 "$HERE/hive-agents.png" "$ICON"
ln -sf "$APP/hive-desktop-rune" "$BIN"
# El lanzador lleva la ruta absoluta: el menú no siempre tiene ~/.local/bin en el PATH.
sed "s|^Exec=.*|Exec=$APP/hive-desktop-rune|" "$HERE/hive-agents.desktop.in" > "$DESKTOP"
sed -i "s|^Icon=.*|Icon=$ICON|" "$DESKTOP"
command -v update-desktop-database >/dev/null && update-desktop-database "$PREFIX/share/applications" 2>/dev/null || true
echo "Instalado en $APP"
echo "  - Menú de aplicaciones: «Hive Agents»"
echo "  - Terminal: $BIN  (si no lo encuentra, añade $PREFIX/bin al PATH)"
