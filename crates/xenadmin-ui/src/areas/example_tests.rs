//! The worked example of `notes/briefs/MODEL-API.md`: a complete small area.
//!
//! It is compiled only for tests and is not a feature of the client. It
//! shows every part an area has: the state struct, a command with its
//! eligibility, a dialog surface with an input, a checkbox, a review and
//! footer buttons, scenes, and harness tests. A real area lives in
//! `areas/<id>/mod.rs`, and its `State` is `m.areas.<id>`; this one is
//! `m.areas.example`.

use xenadmin_core::{PoolId, VmId};

use crate::prelude::*;

// -- State ------------------------------------------------------------------

/// The state of the area. `Default` is "nothing open".
#[derive(Default)]
pub struct State {
    /// The open editor. `Some` means the dialog is open: opening a surface
    /// is a state change, not a message to the view.
    pub editor: Option<Editor>,
    /// What the last confirmed rename handed over. A real area hands the
    /// reviewed change to `xenadmin-core` here and keeps the operation.
    pub submitted: Option<Rename>,
}

/// The draft of one rename. Drafts hold `String`, `bool` and enums.
pub struct Editor {
    pub pool: PoolId,
    pub vm: VmId,
    /// The name observed when the dialog opened. The review is stale when
    /// the inventory no longer shows it.
    pub observed: String,
    pub name: String,
    pub keep_description: bool,
    /// False while editing, true on the review step.
    pub reviewing: bool,
    pub error: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rename {
    pub vm: VmId,
    pub name: String,
    pub keep_description: bool,
}

// -- The spec: what the kernel sees of the area -------------------------------

pub fn spec() -> AreaSpec {
    AreaSpec {
        commands,
        scenes,
        surfaces,
        on_scope_change,
        ..AreaSpec::new("example")
    }
}

// -- Commands -----------------------------------------------------------------

pub const RENAME: CommandId = CommandId("example.rename");

fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(RENAME.0, "Rename", "example")
            .menu(MenuPath::new(Menu::Vm, 8, 10))
            .context(&[ObjectKind::Vm])
            .shortcut("f2")
            .eligibility(can_rename)
            .invoke(open),
    ]
}

fn vm_name(m: &Model, vm: VmId) -> Option<String> {
    let inventory = m.inventory.as_ref()?;
    inventory.vms.get(&vm).map(|vm| vm.name.clone())
}

/// Hidden unless the target is one VM; disabled, with the reason, when that
/// VM cannot be renamed now.
fn can_rename(m: &Model, target: &Target) -> Eligibility {
    let Some(vm) = target.single_of(ObjectKind::Vm).and_then(ObjectRef::vm_id) else {
        return Eligibility::Hidden;
    };
    Eligibility::enabled_if(
        vm_name(m, vm).is_some(),
        "The VM is no longer in the inventory.",
    )
}

/// Runs only when `can_rename` returned `Enabled` for the same target.
fn open(m: &mut Model, target: &Target) {
    let Some(object) = target.single_of(ObjectKind::Vm) else {
        return;
    };
    let (Some(pool), Some(vm)) = (object.pool, object.vm_id()) else {
        return;
    };
    let Some(name) = vm_name(m, vm) else {
        return;
    };
    m.areas.example.editor = Some(Editor {
        pool,
        vm,
        observed: name.clone(),
        name,
        keep_description: true,
        reviewing: false,
        error: None,
    });
}

// -- Logic: validation, staleness, review, submission -------------------------

fn validate(editor: &Editor) -> Result<String, String> {
    let name = editor.name.trim();
    if name.is_empty() {
        return Err("Enter a name.".into());
    }
    if name == editor.observed {
        return Err("The name is unchanged.".into());
    }
    Ok(name.to_owned())
}

/// The reason the review can no longer be confirmed, when there is one.
fn stale(m: &Model, editor: &Editor) -> Option<String> {
    (vm_name(m, editor.vm).as_deref() != Some(editor.observed.as_str()))
        .then(|| "The VM changed after this dialog opened. Close it and start again.".to_owned())
}

fn review_lines(editor: &Editor, name: &str) -> Vec<String> {
    let mut lines = vec![format!("Rename \"{}\" to \"{}\".", editor.observed, name)];
    lines.push(if editor.keep_description {
        "The description is kept.".into()
    } else {
        "The description is cleared.".into()
    });
    lines
}

fn to_review(m: &mut Model) {
    let Some(editor) = &mut m.areas.example.editor else {
        return;
    };
    match validate(editor) {
        Ok(_) => {
            editor.error = None;
            editor.reviewing = true;
        }
        Err(error) => editor.error = Some(error),
    }
}

/// The handler checks again what the button showed: a handler can run after
/// the state moved on.
fn confirm(m: &mut Model) {
    let Some(editor) = &m.areas.example.editor else {
        return;
    };
    let refusal = if m.demo {
        Some("Demo mode cannot change infrastructure.".to_owned())
    } else {
        stale(m, editor)
    };
    let checked = validate(editor);
    let (vm, keep_description) = (editor.vm, editor.keep_description);
    match (refusal, checked) {
        (Some(reason), _) | (None, Err(reason)) => {
            if let Some(editor) = &mut m.areas.example.editor {
                editor.error = Some(reason);
            }
        }
        (None, Ok(name)) => {
            m.areas.example.submitted = Some(Rename {
                vm,
                name,
                keep_description,
            });
            m.areas.example.editor = None;
        }
    }
}

fn close(m: &mut Model) {
    m.areas.example.editor = None;
}

/// The dialog belongs to the VM it was opened for.
fn on_scope_change(m: &mut Model) {
    close(m);
}

// -- The surface: a description, rebuilt on every render ----------------------

fn surfaces(m: &Model) -> Vec<Surface> {
    let Some(editor) = &m.areas.example.editor else {
        return Vec::new();
    };
    let body = if editor.reviewing {
        let name = validate(editor).unwrap_or_default();
        blocks![
            review("example-review", review_lines(editor, &name))
                .affected([
                    property("VM", &editor.observed),
                    property("UUID", editor.vm.to_string()).copyable(),
                ])
                .stale(stale(m, editor)),
            editor
                .error
                .clone()
                .map(|error| alert(error).key("example-error")),
        ]
    } else {
        blocks![
            input("example-name", "Name", &editor.name, |m, text| {
                if let Some(editor) = &mut m.areas.example.editor {
                    editor.name = text.to_owned();
                    editor.error = None;
                }
            })
            .help("Shown in the tree and in every list.")
            .on_submit(to_review),
            checkbox(
                "example-keep-description",
                "Keep the description",
                editor.keep_description,
                |m| {
                    if let Some(editor) = &mut m.areas.example.editor {
                        editor.keep_description = !editor.keep_description;
                    }
                }
            ),
            editor
                .error
                .clone()
                .map(|error| alert(error).key("example-error")),
        ]
    };
    let cancel = button("example-cancel", "Cancel", close);
    let footer = if editor.reviewing {
        vec![
            cancel,
            button("example-back", "Back", |m| {
                if let Some(editor) = &mut m.areas.example.editor {
                    editor.reviewing = false;
                }
            }),
            button("example-confirm", "Rename", confirm)
                .primary()
                .disabled_opt(stale(m, editor))
                .disabled_if(m.demo, "Demo mode cannot change infrastructure."),
        ]
    } else {
        vec![
            cancel,
            button("example-to-review", "Review", to_review).primary(),
        ]
    };
    vec![
        Surface::dialog("example-rename", "Rename VM")
            .subtitle(format!("{} ({})", editor.observed, editor.vm))
            // AREAS.md, decision 1: exactly when the Rune code shows the notice.
            .experimental(true)
            .body(body)
            .footer(footer)
            .on_close(close),
    ]
}

// -- Scenes -------------------------------------------------------------------

fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("example-rename", "example", scene_rename),
        SceneSpec::new("example-rename-review", "example", scene_review),
    ]
}

/// A scene sets the model up as a user would have: the demo pool, a VM
/// selected, the command invoked.
fn scene_rename(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(inventory) = m.inventory.clone() else {
        return;
    };
    let Some(vm) = inventory.vms.keys().next().copied() else {
        return;
    };
    m.select(Target::one(ObjectRef::vm(inventory.pool_id, vm)));
    let target = m.target();
    m.invoke(RENAME, &target);
}

fn scene_review(m: &mut Model) {
    scene_rename(m);
    if let Some(editor) = &mut m.areas.example.editor {
        editor.name = "Xen Orchestra (old)".into();
    }
    to_review(m);
}

// -- Tests: the harness drives the model through what it describes ------------

#[cfg(test)]
mod tests {
    use super::*;
    use crate::Harness;
    use std::sync::Arc;

    #[test]
    fn the_command_follows_the_selection() {
        let mut h = Harness::scene("demo");
        // The demo scene selects the pool, not a VM: the command is hidden, so
        // the VM menu omits it.
        assert_eq!(h.eligibility("example.rename"), Eligibility::Hidden);
        assert!(h.menu_item("example.rename").is_none());
        let inventory = h.model.inventory.clone().expect("inventory");
        let vm = *inventory.vms.keys().next().expect("a VM");
        let target = Target::one(ObjectRef::vm(inventory.pool_id, vm));
        h.select(target.clone());
        let item = h.menu_item("example.rename").expect("menu item");
        assert_eq!((item.label.as_str(), item.disabled), ("Rename", None));
        assert!(h.context_menu(&target).iter().any(|item| item.id == RENAME));
        // A VM the inventory does not have: shown, disabled, with the reason.
        let gone = VmId::parse("30000000-0000-0000-0000-00000000ffff").expect("id");
        h.select(Target::one(ObjectRef::vm(inventory.pool_id, gone)));
        assert_eq!(
            h.try_invoke("example.rename"),
            Err("The VM is no longer in the inventory.".into())
        );
        assert!(h.surfaces().is_empty());
    }

    #[test]
    fn the_scene_opens_the_dialog_with_its_keys_and_the_experimental_notice() {
        let h = Harness::scene("example-rename");
        let surface = h.surface();
        assert_eq!(surface.id, "example-rename");
        assert!(surface.experimental);
        h.assert_unique_keys();
        assert_eq!(
            h.keys(),
            [
                "example-rename",
                "experimental-notice",
                "example-name",
                "example-keep-description",
                "example-cancel",
                "example-to-review",
            ]
        );
        assert_eq!(
            h.node("example-name").value.as_deref(),
            Some("Xen Orchestra")
        );
        assert_eq!(h.node("example-keep-description").checked, Some(true));
        assert!(h.has_text("Shown in the tree and in every list."));
    }

    #[test]
    fn validation_keeps_the_dialog_on_the_form() {
        let mut h = Harness::scene("example-rename");
        h.click("example-to-review");
        assert_eq!(
            h.node("example-error").text.as_deref(),
            Some("The name is unchanged.")
        );
        h.type_text("example-name", "   ");
        assert!(
            h.find("example-error").is_none(),
            "an edit clears the error"
        );
        h.submit("example-name");
        assert_eq!(
            h.node("example-error").text.as_deref(),
            Some("Enter a name.")
        );
        assert!(h.find("example-review").is_none());
    }

    #[test]
    fn demo_mode_shows_the_review_and_cannot_submit() {
        let mut h = Harness::scene("example-rename-review");
        assert!(h.has_text("Rename \"Xen Orchestra\" to \"Xen Orchestra (old)\"."));
        assert!(h.has_text("The description is kept."));
        assert_eq!(
            h.try_click("example-confirm"),
            Err("Demo mode cannot change infrastructure.".into())
        );
        assert!(h.model.areas.example.submitted.is_none());
        h.click("example-back");
        assert!(h.find("example-name").is_some());
    }

    #[test]
    fn a_reviewed_rename_is_submitted_once_and_closes_the_dialog() {
        let mut h = Harness::scene("example-rename");
        h.model.demo = false;
        h.type_text("example-name", " web-01 ");
        h.toggle("example-keep-description");
        h.click("example-to-review");
        assert!(h.has_text("The description is cleared."));
        h.click("example-confirm");
        let submitted = h.model.areas.example.submitted.clone().expect("submitted");
        assert_eq!(submitted.name, "web-01");
        assert!(!submitted.keep_description);
        assert!(h.surfaces().is_empty());
    }

    #[test]
    fn a_stale_review_cannot_be_confirmed() {
        let mut h = Harness::scene("example-rename-review");
        h.model.demo = false;
        // The VM is renamed on the server while the review is open.
        let mut inventory = h
            .model
            .inventory
            .clone()
            .expect("inventory")
            .as_ref()
            .clone();
        let vm = h.model.areas.example.editor.as_ref().expect("editor").vm;
        inventory.vms.get_mut(&vm).expect("vm").name = "renamed elsewhere".into();
        h.model.inventory = Some(Arc::new(inventory));
        let reason = "The VM changed after this dialog opened. Close it and start again.";
        assert_eq!(h.node("example-review-stale").text.as_deref(), Some(reason));
        assert_eq!(h.try_click("example-confirm"), Err(reason.into()));
        assert!(h.model.areas.example.submitted.is_none());
    }

    #[test]
    fn escape_and_a_scope_change_close_the_dialog() {
        let mut h = Harness::scene("example-rename");
        h.dismiss();
        assert!(h.surfaces().is_empty());
        let mut h = Harness::scene("example-rename");
        h.select(Target::none());
        assert!(h.surfaces().is_empty());
    }
}
