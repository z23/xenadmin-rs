#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
cargo build --release --locked --workspace
version="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')"
package_name="xenadmin-rs-$version-linux-$(uname -m)"
stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/xenadmin-rs-package.XXXXXXXX")"
trap 'rm -rf -- "$stage_dir"' EXIT
mkdir -p "$stage_dir/$package_name/bin" "$stage_dir/$package_name/share/applications" "$stage_dir/$package_name/share/icons/hicolor/scalable/apps" artifacts
install -m755 target/release/xenadmin-rs target/release/host-probe "$stage_dir/$package_name/bin/"
install -m644 packaging/xenadmin-rs.desktop "$stage_dir/$package_name/share/applications/"
install -m644 packaging/xenadmin-rs.svg "$stage_dir/$package_name/share/icons/hicolor/scalable/apps/"
install -m644 LICENSE "$stage_dir/$package_name/"
install -m644 docs/RUNNING.md "$stage_dir/$package_name/README.md"
python3 scripts/collect-notices.py "$stage_dir/$package_name/third-party"
tar -czf "artifacts/$package_name.tar.gz" -C "$stage_dir" "$package_name"
sha256sum "artifacts/$package_name.tar.gz" | tee "artifacts/$package_name.tar.gz.sha256"
