#!/usr/bin/env bash
# Sube la versión, hace commit y crea el tag anotado. No empuja nada: el push (y con él la publicación) es manual.
# Uso: scripts/release.sh 0.1.0
set -euo pipefail
cd "$(dirname "$0")/.."
v="${1:?uso: scripts/release.sh X.Y.Z}"
[[ "$v" =~ ^[0-9]+\.[0-9]+\.[0-9]+$ ]] || { echo "versión inválida: $v" >&2; exit 1; }
[ -z "$(git status --porcelain)" ] || { echo "Hay cambios sin commitear; commitea o guarda antes." >&2; exit 1; }
git rev-parse -q --verify "refs/tags/v$v" >/dev/null && { echo "el tag v$v ya existe" >&2; exit 1; }
sed -i "0,/^version = \".*\"/s//version = \"$v\"/" Cargo.toml
cargo update --offline -w >/dev/null 2>&1 || cargo update -w >/dev/null
grep -q "^version = \"$v\"" Cargo.toml
git add Cargo.toml Cargo.lock
git commit -m "release v$v"
git tag -a "v$v" -m "release v$v"
echo "Listo. Publica con:  git push origin master v$v"
