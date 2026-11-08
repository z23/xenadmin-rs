#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
check_state_dir="$(mktemp -d "${TMPDIR:-/tmp}/xenadmin-rs-check.XXXXXXXX")"
trap 'rm -rf -- "$check_state_dir"' EXIT
export XENADMIN_STATE_DIR="$check_state_dir"
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
cargo test --workspace --locked
cargo test --manifest-path vendor/vnc-rs/Cargo.toml --lib --locked --offline
python3 -m unittest discover -s lab -p test_manage.py
cargo build --workspace --locked
mkdir -p artifacts
check_binary="$(cargo metadata --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"] + "/debug/xenadmin-rs")')"
"$check_binary" --snap artifacts/demo.png --tree artifacts/demo.json --scene demo --theme dark --size 1440x900 --scale 1
"$check_binary" --snap artifacts/connect.png --tree artifacts/connect.json --scene connect --theme dark --size 1440x900 --scale 1
"$check_binary" --snap artifacts/compact.png --tree artifacts/compact.json --scene demo --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/connect-insecure.png --tree artifacts/connect-insecure.json --scene connect-insecure --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/snapshots.png --tree artifacts/snapshots.json --scene snapshots --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-create.png --tree artifacts/snapshot-create.json --scene snapshot-create --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-restore.png --tree artifacts/snapshot-restore.json --scene snapshot-restore --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshots-compact.png --tree artifacts/snapshots-compact.json --scene snapshots --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/unavailable-actions.png --tree artifacts/unavailable-actions.json --scene unavailable-actions --theme dark --size 1440x1200 --scale 1 --at 1

"$check_binary" --snap artifacts/properties.png --tree artifacts/properties.json --scene properties --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/properties-running.png --tree artifacts/properties-running.json --scene properties-running --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/connections.png --tree artifacts/connections.json --scene connections --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/connection-settings.png --tree artifacts/connection-settings.json --scene connection-settings --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/vm-devices.png --tree artifacts/vm-devices.json --scene vm-devices --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/network-create.png --tree artifacts/network-create.json --scene network-create --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-administration.png --tree artifacts/pool-administration.json --scene pool-administration --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/performance.png --tree artifacts/performance.json --scene performance --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/performance-compact.png --tree artifacts/performance-compact.json --scene performance --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/vm-create.png --tree artifacts/vm-create.json --scene vm-create --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/activity.png --tree artifacts/activity.json --scene activity --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/activity-compact.png --tree artifacts/activity-compact.json --scene activity --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/network-interfaces.png --tree artifacts/network-interfaces.json --scene network-interfaces --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/storage-create.png --tree artifacts/storage-create.json --scene storage-create --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/storage-review.png --tree artifacts/storage-review.json --scene storage-review --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/vm-migration.png --tree artifacts/vm-migration.json --scene vm-migration --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/vm-migration-compact.png --tree artifacts/vm-migration-compact.json --scene vm-migration --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/host-administration.png --tree artifacts/host-administration.json --scene host-administration --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/host-administration-compact.png --tree artifacts/host-administration-compact.json --scene host-administration --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/organization.png --tree artifacts/organization.json --scene organization --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/organization-compact.png --tree artifacts/organization-compact.json --scene organization --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-policies.png --tree artifacts/pool-policies.json --scene pool-policies --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-policy-review.png --tree artifacts/pool-policy-review.json --scene pool-policy-review --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/archive-export.png --tree artifacts/archive-export.json --scene archive-export --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/archive-export-compact.png --tree artifacts/archive-export-compact.json --scene archive-export --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/appliances.png --tree artifacts/appliances.json --scene appliances --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/appliance-review.png --tree artifacts/appliance-review.json --scene appliance-review --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-security.png --tree artifacts/pool-security.json --scene pool-security --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-security-review.png --tree artifacts/pool-security-review.json --scene pool-security-review --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/disk-export.png --tree artifacts/disk-export.json --scene disk-export --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/disk-export-compact.png --tree artifacts/disk-export-compact.json --scene disk-export --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/snapshot-schedules.png --tree artifacts/snapshot-schedules.json --scene snapshot-schedules --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-schedules-compact.png --tree artifacts/snapshot-schedules-compact.json --scene snapshot-schedules --theme dark --size 513x652 --scale 1 --at 1
