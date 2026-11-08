use super::*;
use xenadmin_core::{
    CloudInitInputs, OperationAction, OperationId, OperationRecord, OperationState,
};
use zeroize::Zeroize;

pub(super) struct Draft {
    pub hostname: TextEdit,
    pub user_data: TextEdit,
    pub network: TextEdit,
}
impl Default for Draft {
    fn default() -> Self {
        Self {
            hostname: TextEdit::default(),
            user_data: TextEdit::new("#cloud-config\n"),
            network: TextEdit::default(),
        }
    }
}
impl Drop for Draft {
    fn drop(&mut self) {
        self.user_data.text.zeroize();
        self.network.text.zeroize();
    }
}
impl Draft {
    pub fn validate(&self) -> Result<(), String> {
        CloudInitInputs::validate(
            self.hostname.text.trim(),
            &self.user_data.text,
            (!self.network.text.trim().is_empty()).then_some(self.network.text.as_str()),
        )
        .map_err(|e| e.to_string())
    }
}
pub(super) fn last_step(form: &Editor) -> u8 {
    if form.cloud { 3 } else { 2 }
}
pub(super) fn inputs(form: &Editor, k: &Kit) -> El {
    col().w_full().gap(8.0)
        .child(caption("The template must already support cloud-init with the NoCloud datasource. This configuration stays in this wizard until the new VM is verified. It is not saved with the connection profile or creation history.",k).wrap())
        .child(caption("Guest hostname",k))
        .child(TextInput::new(&form.cloud_input.hostname).label("Guest hostname").on_edit(|s:&mut State|&mut editor_mut(s).unwrap().cloud_input.hostname).build(k).w_full().key("creation-cloud-hostname"))
        .child(caption("User-data (up to 2 MiB)",k))
        .child(rune_kit::TextArea::new(&form.cloud_input.user_data).label("User-data (up to 2 MiB)").on_edit(|s:&mut State|&mut editor_mut(s).unwrap().cloud_input.user_data).build(k).w_full().h(150.0).key("creation-cloud-user-data"))
        .child(caption("Optional network-config (up to 1 MiB)",k))
        .child(rune_kit::TextArea::new(&form.cloud_input.network).label("Optional network-config (up to 1 MiB)").on_edit(|s:&mut State|&mut editor_mut(s).unwrap().cloud_input.network).build(k).w_full().h(100.0).key("creation-cloud-network"))
        .child(caption("The new VM stays halted. After successful creation, review the exact new VM, storage and unused disk slot before uploading its configuration drive. Closing the wizard discards these unsaved inputs without cancelling host work.",k).wrap())
}
pub(crate) struct Pending {
    pub(super) id: OperationId,
    pub(super) pool: PoolId,
    pub(super) source: VmId,
    pub(super) target: Option<ObservedVm>,
    pub(super) operation: VmLifecycleOperation,
    pub(super) sr: SrId,
    pub(super) draft: Draft,
}
fn record(s: &State, p: &Pending) -> Option<OperationRecord> {
    s.operations
        .iter()
        .find(|o| o.latest().id == p.id)
        .map(|o| (*o.latest()).clone())
        .or_else(|| s.operation_rows.iter().find(|r| r.id == p.id).cloned())
}
fn completed_vm(p: &Pending, r: &OperationRecord) -> Result<VmId, String> {
    if r.id != p.id
        || r.pool != p.pool
        || r.vm != p.source
        || r.action != OperationAction::Lifecycle(p.operation.clone())
    {
        return Err("Creation identity does not match this wizard. Inspect VM changes.".into());
    }
    if r.state != OperationState::Succeeded {
        return Err(format!(
            "Creation is {:?}. Configuration has not been uploaded. Resolve the creation operation in VM changes before configuring the retained VM separately.",
            r.state
        ));
    }
    r.created_vm
        .filter(|vm| *vm != p.source)
        .ok_or_else(|| "The successful creation has no verified new VM identity.".into())
}
fn ready(s: &State, p: &Pending) -> Result<VmId, String> {
    let r = record(s, p).ok_or("Waiting for the creation operation to appear.")?;
    let vm = completed_vm(p, &r)?;
    let inventory = s
        .inventory
        .as_ref()
        .filter(|i| i.pool_id == p.pool)
        .ok_or("Select the original pool to inspect the created VM.")?;
    if !s.demo
        && (p.target.is_none()
            || s.snapshot
                .as_ref()
                .and_then(|snap| snap.observe_vm(p.source).ok())
                != p.target)
    {
        return Err("The connection generation changed. Open cloud-init from the verified created VM to review it again; this wizard will not continue across reconnects.".into());
    }
    let guest=inventory.vms.get(&vm).filter(|v|!v.is_template&&!v.is_control_domain&&v.power_state==PowerState::Halted)
        .ok_or("Waiting for the exact new halted VM in current inventory. A running, removed or converted VM cannot continue here.")?;
    if inventory
        .details
        .vms
        .get(&guest.id)
        .is_none_or(|v| !v.current_operations.is_empty())
    {
        return Err("Waiting for current VM details with no active operations.".into());
    }
    if inventory
        .details
        .storage
        .get(&p.sr)
        .is_none_or(|sr| sr.content_type != "user")
    {
        return Err("The selected configuration-drive storage is no longer available.".into());
    }
    Ok(vm)
}
fn continue_to_drive(s: &mut State) {
    let Some(Panel::CloudCreation(p)) = &s.vm_lifecycle else {
        return;
    };
    if s.demo || !s.can_mutate() {
        return;
    }
    let Ok(vm) = ready(s, p) else { return };
    if p.draft.validate().is_err() {
        return;
    }
    let Some(Panel::CloudCreation(mut p)) = s.vm_lifecycle.take() else {
        return;
    };
    cloud_init_view::open_created(
        s,
        vm,
        p.sr,
        std::mem::take(&mut p.draft.hostname.text),
        std::mem::take(&mut p.draft.user_data.text),
        std::mem::take(&mut p.draft.network.text),
    );
}
pub(super) fn pending_view(s: &State, p: &Pending, k: &Kit) -> El {
    let mut body = col().w_full().gap(10.0).child(
        caption(
            format!(
                "Creation operation: {}\nSource template: {}\nPool: {}",
                p.id, p.source, p.pool
            ),
            k,
        )
        .wrap_anywhere(),
    );
    if let Some(r) = record(s, p) {
        body = body.child(caption(format!("{:?} · {}", r.state, r.detail), k).wrap());
        if let Some(vm) = r.created_vm {
            body = body.child(caption(format!("Created VM: {vm}"), k).wrap_anywhere());
        }
        for step in r.steps {
            body = body.child(caption(format!("{} · {:?}", step.label, step.state), k).wrap());
        }
    }
    let readiness = ready(s, p);
    body=body.child(caption(match &readiness {Ok(_)=>"The new VM is verified and halted. Review its configuration drive before uploading.".into(),Err(e)=>e.clone()},k).wrap())
        .child(caption("Configuration is still private and has not been uploaded. Closing discards this draft; it does not cancel creation or delete the retained VM. VM changes keeps the creation outcome, and cloud-init can be opened from the VM later.",k).wrap());
    let card = col()
        .w((s.window_width - 40.0).clamp(280.0, 700.0))
        .h((s.window_height - 60.0).clamp(280.0, 800.0))
        .padding(22.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading("Create VM · configuration drive", k))
        .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(
                    Button::new("Review configuration drive")
                        .disabled(s.demo || !s.can_mutate() || readiness.is_err())
                        .on_press(continue_to_drive)
                        .build(k)
                        .key("creation-cloud-continue"),
                )
                .child(
                    Button::new("Close and discard draft")
                        .on_press(close)
                        .build(k)
                        .key("creation-cloud-close"),
                ),
        );
    overlay(card, "Create VM · configuration drive")
}

pub(super) fn scene(s: &mut State) {
    super::scene(s);
    let form = editor_mut(s).unwrap();
    form.cloud = true;
    form.step = 3;
    form.cloud_input.hostname = TextEdit::new("new-application");
    form.cloud_input.user_data = TextEdit::new("#cloud-config\npackages: [curl]\n");
}
pub(super) fn pending_scene(s: &mut State) {
    scene(s);
    let form = editor_mut(s).unwrap();
    let operation = creation::operation(form).unwrap();
    let p = Pending {
        id: OperationId::new(),
        pool: form.pool,
        source: form.source.id,
        target: None,
        operation,
        sr: form.sr.unwrap(),
        draft: std::mem::take(&mut form.cloud_input),
    };
    let vm = VmId::from(uuid::Uuid::from_u128(0x30000000000000000000000000000099));
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let mut guest = i.vms[&p.source].clone();
    guest.id = vm;
    guest.is_template = false;
    guest.name = "New application VM".into();
    i.vms.insert(vm, guest);
    let details = i.details.vms[&p.source].clone();
    i.details.vms.insert(vm, details);
    let record:OperationRecord=serde_json::from_value(serde_json::json!({"id":p.id,"pool":p.pool,"vm":p.source,"action":OperationAction::Lifecycle(p.operation.clone()),"state":"Succeeded","progress":null,"detail":"Synthetic creation completed; configuration not uploaded","created_at_ms":1,"created_vm":vm})).unwrap();
    s.operation_rows.push(record);
    s.vm_lifecycle = Some(Panel::CloudCreation(Box::new(p)));
}
#[cfg(test)]
mod tests {
    use super::*;
    fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
        s.window_width = size.0;
        s.window_height = size.1;
        let mut r = rune::Runtime::new(
            s,
            crate::view,
            size,
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        settle(&mut r);
        r
    }
    fn settle(r: &mut rune::Runtime<State>) {
        for _ in 0..30 {
            r.frame(0.016);
        }
    }
    fn click(r: &mut rune::Runtime<State>, key: &str) {
        r.update(|_, tx| tx.scroll_into_view(key));
        settle(r);
        let (x, y) = r.find(key).unwrap();
        assert!(x > 0.0 && x < r.state.window_width && y > 0.0 && y < r.state.window_height);
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        settle(r);
    }
    fn pending(s: &State) -> &Pending {
        let Some(Panel::CloudCreation(p)) = &s.vm_lifecycle else {
            panic!("pending wizard missing")
        };
        p
    }
    #[test]
    fn optional_cloud_step_is_reachable_and_disabling_it_discards_the_draft() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = init();
            super::super::scene(&mut s);
            let mut r = runtime(s, size);
            click(&mut r, "creation-cloud-enable");
            assert!(editor(&r.state).unwrap().cloud);
            click(&mut r, "review-vm-lifecycle");
            assert_eq!(editor(&r.state).unwrap().step, 3);
            r.update(|s, _| {
                editor_mut(s).unwrap().cloud_input.user_data =
                    TextEdit::new("#cloud-config\nprivate-fixture-token: true\n")
            });
            click(&mut r, "back-vm-lifecycle");
            click(&mut r, "creation-cloud-enable");
            let form = editor(&r.state).unwrap();
            assert!(!form.cloud);
            assert!(
                !form
                    .cloud_input
                    .user_data
                    .text
                    .contains("private-fixture-token")
            );
            click(&mut r, "review-vm-lifecycle");
            assert!(editor(&r.state).unwrap().review.is_some());
        }
    }
    #[test]
    fn creation_cloud_inputs_are_validated_and_review_omits_the_private_payload() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = init();
            scene(&mut s);
            editor_mut(&mut s).unwrap().cloud_input.hostname = TextEdit::new("bad/hostname");
            let mut r = runtime(s, size);
            click(&mut r, "review-vm-lifecycle");
            assert!(editor(&r.state).unwrap().review.is_none());
            assert!(editor(&r.state).unwrap().error.is_some());
            r.update(|s, _| {
                let f = editor_mut(s).unwrap();
                f.cloud_input.hostname = TextEdit::new("fixture");
                f.cloud_input.user_data =
                    TextEdit::new("#cloud-config\nprivate-fixture-token: true\n");
            });
            settle(&mut r);
            click(&mut r, "review-vm-lifecycle");
            let f = editor(&r.state).unwrap();
            assert!(f.review.is_some());
            let text = r.accessibility_json().to_string();
            assert!(!text.contains("private-fixture-token"));
            assert!(text.contains("separately reviewed"));
            let payload = serde_json::to_string(&f.review.as_ref().unwrap().operation).unwrap();
            assert!(!payload.contains("private-fixture-token"));
            click(&mut r, "apply-vm-lifecycle");
            assert!(r.state.operations.is_empty());
        }
    }
    #[test]
    fn creation_cloud_pending_requires_exact_success_and_never_uses_the_selection() {
        let mut s = init();
        pending_scene(&mut s);
        let p = pending(&s);
        let r = record(&s, p).unwrap();
        let vm = completed_vm(p, &r).unwrap();
        for state in [
            OperationState::OutcomeUnknown,
            OperationState::AcknowledgedUnknown,
            OperationState::PartiallyCompleted,
            OperationState::Failed,
            OperationState::Cancelled,
        ] {
            let mut bad = r.clone();
            bad.state = state;
            assert!(completed_vm(p, &bad).is_err());
        }
        for mutation in [0, 1, 2, 3, 4, 5] {
            let mut bad = r.clone();
            match mutation {
                0 => bad.id = OperationId::new(),
                1 => bad.pool = uuid::Uuid::from_u128(3).into(),
                2 => bad.vm = vm,
                3 => {
                    bad.action = OperationAction::Lifecycle(VmLifecycleOperation::ConvertToTemplate)
                }
                4 => bad.created_vm = None,
                _ => bad.created_vm = Some(p.source),
            };
            assert!(completed_vm(p, &bad).is_err());
        }
        assert_eq!(ready(&s, pending(&s)).unwrap(), vm);
        s.table.select(pending(&s).source.to_string());
        assert_eq!(ready(&s, pending(&s)).unwrap(), vm);
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .vms
            .get_mut(&vm)
            .unwrap()
            .power_state = PowerState::Running;
        assert!(ready(&s, pending(&s)).is_err());
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .vms
            .get_mut(&vm)
            .unwrap()
            .power_state = PowerState::Halted;
        s.demo = false;
        assert!(ready(&s, pending(&s)).is_err());
    }
    #[test]
    fn creation_cloud_pending_controls_are_visible_and_synthetic_continue_never_prepares_media() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = init();
            pending_scene(&mut s);
            let mut r = runtime(s, size);
            click(&mut r, "creation-cloud-continue");
            assert!(!r.state.cloud_init.show);
            assert!(r.state.operations.is_empty());
            let text = r.accessibility_json().to_string();
            assert!(text.contains("Created VM:"));
            assert!(text.contains("Closing discards"));
            click(&mut r, "creation-cloud-close");
            assert!(r.state.vm_lifecycle.is_none());
            assert!(!r.state.operation_rows.is_empty());
        }
    }
}
