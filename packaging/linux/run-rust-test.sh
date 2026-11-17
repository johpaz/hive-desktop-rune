#!/usr/bin/env bash
# Build both binaries and test with a new HiveDB copy. The original Hive must be stopped.
# Usage: packaging/linux/run-rust-test.sh [Rune arguments, e.g. --check or --chat "hola"]
# HIVE_HOME selects the SOURCE profile; the launched app always uses the isolated copy.
set -euo pipefail
ROOT="$(cd "$(dirname "$0")/../.." && pwd)"
SOURCE="${HIVE_HOME:-$HOME/.hive}"
SDK="$ROOT/../hiveSDK-Rust"
cargo build --locked --manifest-path "$SDK/Cargo.toml" -p hive-gateway-rust
cargo build --locked --manifest-path "$ROOT/Cargo.toml" --bin hive-desktop-rune
TEST_PARENT="$(mktemp -d "${TMPDIR:-/tmp}/hive-rune-rust.XXXXXX")"
TEST_PROFILE="$TEST_PARENT/profile"
GATEWAY="$SDK/target/debug/hive-gateway-rust"
env -u HIVE_DB_PATH "$GATEWAY" --prepare-test-home "$SOURCE" "$TEST_PROFILE"
env -u HIVE_DB_PATH HIVE_HOME="$TEST_PROFILE" "$GATEWAY" --check-config
echo "Perfil de prueba: $TEST_PROFILE"
exec env -u HIVE_DB_PATH HIVE_HOME="$TEST_PROFILE" HIVE_GATEWAY_BIN="$GATEWAY" "$ROOT/target/debug/hive-desktop-rune" "$@"
