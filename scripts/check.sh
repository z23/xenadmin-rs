#!/usr/bin/env bash
set -euo pipefail
cd "$(dirname "$0")/.."
check_state_dir="$(mktemp -d "${TMPDIR:-/tmp}/xenadmin-rs-check.XXXXXXXX")"
trap 'rm -rf -- "$check_state_dir"' EXIT
export XENADMIN_STATE_DIR="$check_state_dir"
# Nothing here opens a window: the tests run on the Kit's test platform and the scenes are
# drawn off screen. Without a display a step that tried to would fail instead.
unset DISPLAY WAYLAND_DISPLAY
cargo fmt --all -- --check
cargo clippy --workspace --all-targets --locked -- -D warnings
# The model must stay free of any toolkit.
if cargo tree -p xenadmin-ui -e normal --locked | grep -E '(^|[^A-Za-z0-9_-])(rune|rune-kit|gpui)([^A-Za-z0-9_-]|$)'; then
    echo "xenadmin-ui depends on a UI toolkit" >&2
    exit 1
fi
# Gate G5 of docs/GPUI-MIGRATION.md: no crate of the Rune toolkit is in the workspace's
# dependency tree, by any edge.
if cargo tree --workspace --locked | grep -iE '(^|[^A-Za-z0-9_])rune([^A-Za-z0-9_]|$)'; then
    echo "the workspace depends on a rune crate" >&2
    exit 1
fi
# Debug builds report, when asked to, every record a view was given that carries a key the
# view's class lacks: a record read as another class, or a fixture with a field the API
# does not have. Only what the tests mean may be reported.
XENADMIN_XAPI_VIEW_AUDIT="$check_state_dir/view-audit.tsv" cargo test --workspace --locked
# The front end's renderer tests and its scene tool need the Kit's test harness, which is
# behind the crate feature scenes: the workspace steps above build without it.
cargo clippy -p xenadmin-desktop --all-targets --features scenes --locked -- -D warnings
cargo test -p xenadmin-desktop --features scenes --locked
# The front end never blocks on a future: the core's runtime is the only one, and the
# interface reads the latest state when it is woken.
if grep -rn 'block_on' crates/xenadmin-desktop/src; then
    echo "crates/xenadmin-desktop/src names block_on" >&2
    exit 1
fi
# The scene tool renders scenes without a display, each as the whole window: the model's
# registered scenes, the renderer's own fixtures and the shell's (gallery-*). A PNG needs a
# graphics adapter (a software one will do); the tool fails when a frame lacks an element its
# description has. The files are the CI artifact ui.
mkdir -p artifacts/ui
cargo build -p xenadmin-desktop --features scenes --locked --bin xenadmin-scenes
scenes() {
    cargo run -p xenadmin-desktop --features scenes --locked --quiet --bin xenadmin-scenes -- "$@"
}
scenes --list > artifacts/ui/scenes.txt
# The command registry, one line per command (id, label, menu path, toolbar slot, context
# kinds, local row, shortcut, area): what the Commands table of the migration ledger is made
# from (scripts/gpui_commands_table.py).
scenes --commands > artifacts/ui/commands.txt
# Every scene the tool lists is rendered once: an area's scenes are in the list from the moment
# the area registers them.
while IFS= read -r scene; do
    scenes "$scene" --snap "artifacts/ui/${scene}.png" --tree "artifacts/ui/${scene}.json" --theme dark --size 1440x900 --scale 1
done < artifacts/ui/scenes.txt
scenes demo --snap artifacts/ui/demo-light.png --tree artifacts/ui/demo-light.json --theme light --size 1440x900 --scale 1
# The artifact of console-window is detached console window 1, not the main window: at the
# size it opens with and at the smallest it can be given.
scenes console-window --snap artifacts/ui/console-window-default.png --tree artifacts/ui/console-window-default.json --theme dark --size 1100x800 --scale 1
scenes console-window --snap artifacts/ui/console-window-compact.png --tree artifacts/ui/console-window-compact.json --theme dark --size 480x640 --scale 1
# 800x600 is the smallest size the main window can be given.
scenes demo --snap artifacts/ui/demo-compact.png --tree artifacts/ui/demo-compact.json --theme dark --size 800x600 --scale 1
scenes gallery-shell --snap artifacts/ui/gallery-shell-light-compact.png --tree artifacts/ui/gallery-shell-light-compact.json --theme light --size 800x600 --scale 1
scenes gallery-dialog --snap artifacts/ui/gallery-dialog-light-compact.png --tree artifacts/ui/gallery-dialog-light-compact.json --theme light --size 800x600 --scale 1
scenes gallery-tab --snap artifacts/ui/gallery-tab-light-2x.png --theme light --size 800x600 --scale 2
# What is below the fold, scrolled into a frame of the real size: the last chart of the
# performance tab at the smallest window, and the confirmation at the end of a long dialog.
scenes performance --reveal metrics-chart-3-plot --snap artifacts/ui/performance-last-chart-compact.png --tree artifacts/ui/performance-last-chart-compact.json --theme light --size 800x600 --scale 1
scenes pool-join --reveal join-confirmation --snap artifacts/ui/pool-join-confirmation.png --tree artifacts/ui/pool-join-confirmation.json --theme dark --size 1440x900 --scale 1
# The client binary itself draws a scene to files with the same flags, without opening a
# window, when it is built with the feature scenes. The packaged binary is built without it.
cargo run -p xenadmin-desktop --features scenes --locked --quiet --bin xenadmin-rs -- --scene demo --snap artifacts/ui/client-demo.png --tree artifacts/ui/client-demo.json --theme dark --size 1440x900 --scale 1
python3 scripts/view_audit.py "$check_state_dir/view-audit.tsv"
cargo test --manifest-path vendor/vnc-rs/Cargo.toml --lib --locked --offline
python3 -m unittest discover -s lab -p 'test_*.py'
python3 -m unittest discover -s packaging -p 'test_*.py'
# The build the packages are staged from reports both executables: the client and the probe.
cargo build --workspace --locked --message-format=json-render-diagnostics > "$check_state_dir/build.jsonl"
for check_executable in xenadmin-rs host-probe; do
    test -x "$(python3 packaging/artifacts.py "$check_state_dir/build.jsonl" --print-path "$check_executable")"
done
