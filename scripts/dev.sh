#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
export XENADMIN_DEV_INSPECT=1
export RUST_BACKTRACE=1
exec cargo run --locked --bin xenadmin-rs -- "$@"
