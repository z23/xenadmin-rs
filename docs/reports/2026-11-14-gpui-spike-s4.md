# GPUI spike S4: shell primitives

Result: **not passed**. The headless tests cover menus, tabs, a dialog, a wizard, an alert, tree and table keys, and the test platform's file prompts. A native Wayland session was not available, so the pass line is not met.

## What ran

`cargo test -p xenadmin-gpui --test shell` on Ubuntu 24.04.5. Five tests, all passing after the wizard read the current step on each dialog rebuild:

- `menus_follow_selection`. Stub rows Import, Export, and Toolbar. With nothing selected, Import is disabled and Toolbar is unchecked. After the Select VM button, Import is enabled and Toolbar is checked. `cx.set_menus` stores that on the platform. `AppMenuBar` does not read that store. It reads `GlobalState::app_menus`, and nothing in `gpui-component` 0.9.1 copies one into the other. The test copies `get_menus()` into `GlobalState` and calls `AppMenuBar::reload`.
- `tabs_dialog_wizard_and_alert`. A tab id changes when a tab is selected. An empty rename fails validation and a non-empty value clears it. Escape was pressed on the dialog. The wizard's Next button reaches a review element. The alert's OK control was found by its accessible label and clicked; the confirm flag was set.
- `keyboard_reaches_tree_table_and_a_button`. Down changes the tree's selected index and the table's selected row. Tab and Enter are then dispatched. Enter's effect is not asserted.
- `file_prompts_use_the_test_platform`. `prompt_for_new_path` and `prompt_for_paths` complete through `simulate_new_path_selection` and `simulate_path_prompt_response`.
- `light_and_dark_themes_and_rem_size`. `Theme::change` switches `is_dark`. `set_rem_size` and `simulate_window_scale_factor_change(1.25)` return. The test does not assert that bounds changed.

`AppMenuBar` on Linux is a drawn menu. `cx.set_menus` on Linux only stores `OwnedMenu` values. It does not open a system menu bar.

## What this does not establish

No native Wayland session was started. `WAYLAND_DISPLAY` was unset. Portal file prompts with no portal running were not tried. Scale at 100% and 125% was not shown to change layout. Clipboard, physical keys, and Orca were not exercised. Headless tests do not establish those.
