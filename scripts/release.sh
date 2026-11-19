#!/usr/bin/env bash
# Hace todo el lanzamiento: commitea lo pendiente, sube la versión, crea el tag y empuja (lo que dispara
# .github/workflows/release.yml y publica el instalador en el release de GitHub de este repo).
#
# Uso:  scripts/release.sh X.Y.Z [-y] [-m "mensaje del commit"]
#   -y   no pide confirmación antes de empujar
#   -m   mensaje para los cambios pendientes (por defecto: "release vX.Y.Z")
set -euo pipefail
cd "$(dirname "$0")/.."

v="" ; yes=0 ; msg=""
while [ $# -gt 0 ]; do
  case "$1" in
    -y) yes=1 ;;
    -m) shift; msg="${1:?falta el mensaje tras -m}" ;;
    -*) echo "opción desconocida: $1" >&2; exit 1 ;;
    *)  v="${1#v}" ;;
  esac
  shift
done
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "uso: scripts/release.sh X.Y.Z [-y] [-m mensaje]" >&2; exit 1; }
tag="v$v"
branch="$(git rev-parse --abbrev-ref HEAD)"
msg="${msg:-release $tag}"

git rev-parse -q --verify "refs/tags/$tag" >/dev/null && { echo "el tag $tag ya existe en local" >&2; exit 1; }
git ls-remote --exit-code --tags origin "$tag" >/dev/null 2>&1 && { echo "el tag $tag ya existe en origin" >&2; exit 1; }

echo "Rama: $branch · versión: $v"
git status --short
if [ "$yes" -ne 1 ]; then
  read -r -p "Esto commitea lo pendiente, crea $tag y lo empuja (publica el instalador). ¿Seguir? [s/N] " ok
  [[ "$ok" =~ ^[sSyY]$ ]] || { echo "Cancelado."; exit 1; }
fi

# 1. Versión (única fuente: Cargo.toml) y Cargo.lock alineado.
sed -i "0,/^version = \".*\"/s//version = \"$v\"/" Cargo.toml
cargo update --offline -w >/dev/null 2>&1 || cargo update -w >/dev/null
grep -q "^version = \"$v\"" Cargo.toml

# 2. Commit de todo lo pendiente (si hay algo).
git add -A
git diff --cached --quiet || git commit -m "$msg"

# 3. Tag anotado sobre ese commit y push de rama y tag.
git tag -a "$tag" -m "release $tag"
git push origin "$branch" "$tag"
echo "Publicado $tag. Sigue el workflow en la pestaña Actions del repo."
