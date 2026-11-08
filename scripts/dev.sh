#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --workspace --locked
export XENADMIN_DEV_INSPECT=1
export RUST_BACKTRACE=1
exec ./target/debug/xenadmin-rs "$@"
