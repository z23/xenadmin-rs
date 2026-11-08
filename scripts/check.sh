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
python3 -m unittest discover -s packaging -p 'test_*.py'
cargo build --workspace --locked --message-format=json-render-diagnostics > "$check_state_dir/build.jsonl"
mkdir -p artifacts
check_binary="$(python3 packaging/artifacts.py "$check_state_dir/build.jsonl" --print-path xenadmin-rs)"
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
"$check_binary" --snap artifacts/connection-trust.png --tree artifacts/connection-trust.json --scene connection-trust --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/connection-trust-compact.png --tree artifacts/connection-trust-compact.json --scene connection-trust --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/certificate-review.png --tree artifacts/certificate-review.json --scene certificate-review --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/certificate-review-compact.png --tree artifacts/certificate-review-compact.json --scene certificate-review --theme dark --size 513x652 --scale 1 --at 1
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
"$check_binary" --snap artifacts/host-power-settings.png --tree artifacts/host-power-settings.json --scene host-power-settings --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/host-power-settings-compact.png --tree artifacts/host-power-settings-compact.json --scene host-power-settings --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/organization.png --tree artifacts/organization.json --scene organization --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/organization-compact.png --tree artifacts/organization-compact.json --scene organization --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-policies.png --tree artifacts/pool-policies.json --scene pool-policies --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-policy-review.png --tree artifacts/pool-policy-review.json --scene pool-policy-review --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/cloud-init-review.png --tree artifacts/cloud-init-review.json --scene cloud-init-review --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/cloud-init-review-compact.png --tree artifacts/cloud-init-review-compact.json --scene cloud-init-review --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/cloud-init.png --tree artifacts/cloud-init.json --scene cloud-init --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/cloud-init-compact.png --tree artifacts/cloud-init-compact.json --scene cloud-init --theme dark --size 513x652 --scale 1 --at 1
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
"$check_binary" --snap artifacts/device-properties.png --tree artifacts/device-properties.json --scene device-properties --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/device-properties-compact.png --tree artifacts/device-properties-compact.json --scene device-properties --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/home-host.png --tree artifacts/home-host.json --scene home-host --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/root-password.png --tree artifacts/root-password.json --scene root-password --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/root-password-compact.png --tree artifacts/root-password-compact.json --scene root-password --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-join.png --tree artifacts/pool-join.json --scene pool-join --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/pool-join-compact.png --tree artifacts/pool-join-compact.json --scene pool-join --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/snapshot-restore-compact.png --tree artifacts/snapshot-restore-compact.json --scene snapshot-restore --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-edit.png --tree artifacts/snapshot-edit.json --scene snapshot-edit --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-edit-compact.png --tree artifacts/snapshot-edit-compact.json --scene snapshot-edit --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/ovf-import.png --tree artifacts/ovf-import.json --scene ovf-import --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/ovf-import-compact.png --tree artifacts/ovf-import-compact.json --scene ovf-import --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/ovf-export.png --tree artifacts/ovf-export.json --scene ovf-export --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/ovf-export-compact.png --tree artifacts/ovf-export-compact.json --scene ovf-export --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/search-pools.png --tree artifacts/search-pools.json --scene search-pools --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/search-pools-compact.png --tree artifacts/search-pools-compact.json --scene search-pools --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/storage-discovery.png --tree artifacts/storage-discovery.json --scene storage-discovery --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/storage-discovery-compact.png --tree artifacts/storage-discovery-compact.json --scene storage-discovery --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/firmware-create.png --tree artifacts/firmware-create.json --scene firmware-create --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/firmware-create-compact.png --tree artifacts/firmware-create-compact.json --scene firmware-create --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/secure-boot.png --tree artifacts/secure-boot.json --scene secure-boot --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/secure-boot-compact.png --tree artifacts/secure-boot-compact.json --scene secure-boot --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/batch-power.png --tree artifacts/batch-power.json --scene batch-power --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/batch-power-compact.png --tree artifacts/batch-power-compact.json --scene batch-power --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/cross-pool.png --tree artifacts/cross-pool.json --scene cross-pool --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/cross-pool-compact.png --tree artifacts/cross-pool-compact.json --scene cross-pool --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/storage-auth-review.png --tree artifacts/storage-auth-review.json --scene storage-auth-review --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/storage-auth-review-compact.png --tree artifacts/storage-auth-review-compact.json --scene storage-auth-review --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/console-recovery.png --tree artifacts/console-recovery.json --scene console-recovery --theme dark --size 1440x900 --scale 1 --at 1

"$check_binary" --snap artifacts/console-recovery-compact.png --tree artifacts/console-recovery-compact.json --scene console-recovery --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/host-console.png --tree artifacts/host-console.json --scene host-console --theme dark --size 1440x900 --scale 1 --at 1

"$check_binary" --snap artifacts/host-console-compact.png --tree artifacts/host-console-compact.json --scene host-console --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/options.png --tree artifacts/options.json --scene options --theme dark --size 1440x900 --scale 1 --at 1

"$check_binary" --snap artifacts/options-compact.png --tree artifacts/options-compact.json --scene options --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/help.png --tree artifacts/help.json --scene help --theme dark --size 1440x900 --scale 1 --at 1

"$check_binary" --snap artifacts/help-compact.png --tree artifacts/help-compact.json --scene help --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/console-details-compact.png --tree artifacts/console-details-compact.json --scene console-details --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/network-ipv6.png --tree artifacts/network-ipv6.json --scene network-ipv6 --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/network-ipv6-compact.png --tree artifacts/network-ipv6-compact.json --scene network-ipv6 --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/metric-presets.png --tree artifacts/metric-presets.json --scene metric-presets --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/metric-presets-compact.png --tree artifacts/metric-presets-compact.json --scene metric-presets --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/general-information.png --tree artifacts/general-information.json --scene general-information --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/general-information-compact.png --tree artifacts/general-information-compact.json --scene general-information --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/network-topology.png --tree artifacts/network-topology.json --scene network-topology --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/network-topology-compact.png --tree artifacts/network-topology-compact.json --scene network-topology --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/snapshot-ancestry.png --tree artifacts/snapshot-ancestry.json --scene snapshot-ancestry --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-ancestry-compact.png --tree artifacts/snapshot-ancestry-compact.json --scene snapshot-ancestry --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-information.png --tree artifacts/snapshot-information.json --scene snapshot-information --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/snapshot-information-compact.png --tree artifacts/snapshot-information-compact.json --scene snapshot-information --theme dark --size 513x652 --scale 1 --at 1

"$check_binary" --snap artifacts/guest-tools.png --tree artifacts/guest-tools.json --scene guest-tools --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/guest-tools-compact.png --tree artifacts/guest-tools-compact.json --scene guest-tools --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/guest-tools-review.png --tree artifacts/guest-tools-review.json --scene guest-tools-review --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/guest-tools-review-compact.png --tree artifacts/guest-tools-review-compact.json --scene guest-tools-review --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/management-network.png --tree artifacts/management-network.json --scene management-network --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/management-network-compact.png --tree artifacts/management-network-compact.json --scene management-network --theme dark --size 513x652 --scale 1 --at 1
"$check_binary" --snap artifacts/host-support.png --tree artifacts/host-support.json --scene host-support --theme dark --size 1440x900 --scale 1 --at 1
"$check_binary" --snap artifacts/host-support-compact.png --tree artifacts/host-support-compact.json --scene host-support --theme dark --size 513x652 --scale 1 --at 1
