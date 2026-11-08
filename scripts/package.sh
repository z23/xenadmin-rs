#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
if [[ $# -gt 1 || ( $# -eq 1 && "$1" != "--appimage" ) ]]; then
    echo "Usage: $0 [--appimage]" >&2
    exit 2
fi
with_appimage="${1-}"
version="$(python3 -c 'import tomllib; print(tomllib.load(open("Cargo.toml", "rb"))["workspace"]["package"]["version"])')"
package_name="xenadmin-rs-$version-linux-$(uname -m)"
if [[ -e "artifacts/$package_name.tar.gz" || -e "artifacts/$package_name.tar.gz.sha256" ]]; then
    echo "Refusing to replace an existing versioned package: $package_name" >&2
    exit 1
fi
if [[ -n "$with_appimage" && ( -e "artifacts/$package_name.AppImage" || -e "artifacts/$package_name.AppImage.sha256" ) ]]; then
    echo "Refusing to replace an existing AppImage: $package_name" >&2
    exit 1
fi
stage_dir="$(mktemp -d "${TMPDIR:-/tmp}/xenadmin-rs-package.XXXXXXXX")"
trap 'rm -rf -- "$stage_dir"' EXIT
cargo build --release --locked --workspace --message-format=json-render-diagnostics > "$stage_dir/build.jsonl"
target_dir="$(cargo metadata --locked --format-version 1 --no-deps | python3 -c 'import json,sys; print(json.load(sys.stdin)["target_directory"])')"
mkdir -p "$stage_dir/$package_name/bin" "$stage_dir/$package_name/share/applications" "$stage_dir/$package_name/share/icons/hicolor/scalable/apps" artifacts
python3 packaging/artifacts.py "$stage_dir/build.jsonl" "$stage_dir/$package_name/bin"
install -m755 packaging/install.py "$stage_dir/$package_name/"
install -m644 packaging/xenadmin-rs.desktop "$stage_dir/$package_name/share/applications/"
install -m644 packaging/xenadmin-rs.svg "$stage_dir/$package_name/share/icons/hicolor/scalable/apps/"
install -m644 LICENSE "$stage_dir/$package_name/"
install -m644 docs/RUNNING.md "$stage_dir/$package_name/README.md"
install -m644 docs/CONNECTIONS.md "$stage_dir/$package_name/CONNECTIONS.md"
install -m644 docs/INSTALLATION.md "$stage_dir/$package_name/INSTALLATION.md"
python3 scripts/collect-notices.py "$stage_dir/$package_name/third-party"
python3 packaging/manifest.py "$stage_dir/$package_name" "$version"
tar -czf "$stage_dir/package.tar.gz" -C "$stage_dir" "$package_name"
# A hard-link publish fails if another packager already produced this version.
# Stage the final bytes beside the output so the link remains on one filesystem.
output_tmp="$(mktemp "artifacts/.package.XXXXXXXX")"
trap 'rm -rf -- "$stage_dir"; rm -f -- "$output_tmp"' EXIT
cp "$stage_dir/package.tar.gz" "$output_tmp"
ln "$output_tmp" "artifacts/$package_name.tar.gz"
python3 - "$output_tmp" "artifacts/$package_name.tar.gz.sha256" "$package_name.tar.gz" <<'PY'
import hashlib, pathlib, sys
with open(sys.argv[1], "rb") as stream:
    checksum = hashlib.file_digest(stream, "sha256").hexdigest()
with open(sys.argv[2], "x") as stream:
    stream.write(f"{checksum}  {sys.argv[3]}\n")
print(checksum, sys.argv[3])
PY

if [[ -n "$with_appimage" ]]; then
    python3 packaging/appimage.py "$stage_dir/$package_name" "artifacts/$package_name.AppImage" --cache "$target_dir/package-tools"
fi
