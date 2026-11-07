#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
check_state_dir="$(mktemp -d "${TMPDIR:-/tmp}/xenadmin-rs-check.XXXXXXXX")"
trap 'rm -rf -- "$check_state_dir"' EXIT
export XENADMIN_STATE_DIR="$check_state_dir"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo build --workspace --locked
mkdir -p artifacts
target/debug/xenadmin-rs --snap artifacts/demo.png --tree artifacts/demo.json --scene demo --theme dark --size 1440x900 --scale 1
target/debug/xenadmin-rs --snap artifacts/connect.png --tree artifacts/connect.json --scene connect --theme dark --size 1440x900 --scale 1
target/debug/xenadmin-rs --snap artifacts/compact.png --tree artifacts/compact.json --scene demo --theme dark --size 513x652 --scale 1 --at 1
