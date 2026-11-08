#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
inspection_state_dir="${XENADMIN_STATE_DIR:-${XDG_STATE_HOME:-$HOME/.local/state}/xenadmin-rs}"
exec ./target/debug/host-probe --live "$inspection_state_dir/dev-inspect.sock" "$@"
