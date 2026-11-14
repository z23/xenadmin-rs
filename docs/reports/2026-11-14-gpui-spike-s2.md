# GPUI spike S2: headless verification

Result: **not passed**. Two element-tree dumps of one scene matched. PNG bytes were not produced. The named renderer function is not in the pinned platform crate.

## What the pin exports

`gpui-pre-linux` 0.3.12's crate root exports `current_platform` and `linux_platform`. It does not export `current_headless_renderer`. The Kit's own rendering test names that function. `HeadlessAppContext::capture_screenshot` then returns `render_to_image not available: no HeadlessRenderer configured` when the factory returns `None`.

`WgpuHeadlessRenderer` exists on `gpui-pre-wgpu` behind `test-support`. The spike does not take a direct dependency on `gpui-pre` or `gpui-pre-wgpu`. Route B is the plan's only exception, and it was not selected.

## What ran

`cargo run -p xenadmin-gpui --example s2_scene --features scenes` on Ubuntu 24.04.5 with `VK_DRIVER_FILES` pointed at Mesa lavapipe (`lvp_icd.json`) and `XDG_RUNTIME_DIR=/tmp/xdg-run`.

```text
element_trees_match=true
elements=2
png=unavailable
render_to_image not available: no HeadlessRenderer configured
```

The scene is a full-size div `spike-scene` and a button `spike-button`. The dump is id path, role, label, value, focus, checked, disabled, and bounds. Two dumps were equal. Render time was not separated from process startup. The example is not wired into `scripts/check.sh` or CI: a CI step that expects a PNG would fail for the missing renderer.

`#[gpui_kit::test]` in `tests/shell.rs` finds elements by id, clicks, and presses keys. That is the interaction half of the spike, recorded with S4. It does not capture pixels.

## What this does not establish

Equal element trees are not byte-identical PNGs. Headless dumps do not establish Wayland, the system clipboard, physical keys, portal dialogs, or Orca. Lavapipe was installed and was not used, because no headless renderer was installed in the context.
