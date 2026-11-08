use super::*;
use std::sync::mpsc;
use xenadmin_core::{
    CloudInitConfiguration, CloudInitInputs, CloudInitMedia, ObservedVm, OperationId, PowerState,
    SrId,
};

#[derive(Default)]
pub(super) struct Panel {
    pub show: bool,
    vm: Option<VmId>,
    pool: Option<PoolId>,
    observed: Option<ObservedVm>,
    name: String,
    hostname: TextEdit,
    user_data: TextEdit,
    network: TextEdit,
    device: TextEdit,
    sr: Option<SrId>,
    pending: bool,
    receiver: Option<mpsc::Receiver<Result<CloudInitMedia, String>>>,
    media: Option<CloudInitMedia>,
    error: Option<String>,
}
fn close(s: &mut State) {
    s.cloud_init = Panel::default();
}
fn open(s: &mut State, vm: VmId) {
    let Some(i) = s.inventory.as_ref() else {
        return;
    };
    let Some(v) = i.vms.get(&vm) else { return };
    if v.is_template || v.is_control_domain {
        return;
    }
    let slot = (0_u16..=255)
        .find(|slot| {
            !i.details
                .vbds
                .values()
                .any(|v| v.vm == Some(vm) && v.userdevice == slot.to_string())
        })
        .unwrap_or(0);
    s.cloud_init = Panel {
        show: true,
        vm: Some(vm),
        pool: Some(i.pool_id),
        observed: s
            .snapshot
            .as_ref()
            .and_then(|snap| snap.observe_vm(vm).ok()),
        name: v.name.clone(),
        hostname: TextEdit::new(""),
        user_data: TextEdit::new("#cloud-config\n"),
        device: TextEdit::new(slot.to_string()),
        ..Default::default()
    };
}
pub(super) fn open_created(
    s: &mut State,
    vm: VmId,
    sr: SrId,
    hostname: String,
    user_data: String,
    network: String,
) {
    open(s, vm);
    if s.cloud_init.vm != Some(vm) {
        return;
    }
    s.cloud_init.sr = Some(sr);
    s.cloud_init.hostname = TextEdit::new(hostname);
    s.cloud_init.user_data = TextEdit::new(user_data);
    s.cloud_init.network = TextEdit::new(network);
    prepare(s);
}
pub(super) fn button(s: &State, vm: VmId, k: &Kit) -> El {
    let Some(v) = s
        .inventory
        .as_ref()
        .and_then(|i| i.vms.get(&vm))
        .filter(|v| !v.is_template && !v.is_control_domain)
    else {
        return el();
    };
    Button::new(if v.power_state == PowerState::Halted {
        "Cloud-init drive…"
    } else {
        "Cloud-init drive (shut down first)"
    })
    .variant(ButtonVariant::Outline)
    .disabled(v.power_state != PowerState::Halted)
    .on_press(move |s: &mut State| open(s, vm))
    .build(k)
    .key("open-cloud-init")
}
fn current(s: &State) -> bool {
    let p = &s.cloud_init;
    s.inventory.as_ref().is_some_and(|i| {
        Some(i.pool_id) == p.pool && p.vm.is_some_and(|vm| i.vms.contains_key(&vm))
    }) && (s.demo
        || p.observed.is_some_and(|o| {
            s.snapshot
                .as_ref()
                .and_then(|snap| p.vm.and_then(|vm| snap.observe_vm(vm).ok()))
                == Some(o)
        }))
}
fn config(p: &Panel) -> Option<CloudInitConfiguration> {
    Some(CloudInitConfiguration {
        sr: p.sr?,
        device: p.device.text.clone(),
        configuration_id: p.media.as_ref().map_or_else(
            || uuid::Uuid::from_u128(1),
            CloudInitMedia::configuration_id,
        ),
    })
}
fn can_prepare(s: &State) -> bool {
    current(s)
        && s.cloud_init
            .vm
            .zip(config(&s.cloud_init))
            .is_some_and(|(vm, c)| {
                s.inventory
                    .as_ref()
                    .is_some_and(|i| c.availability(i, vm).is_available())
            })
}
fn prepare(s: &mut State) {
    if s.cloud_init.pending || !can_prepare(s) {
        return;
    }
    let p = &mut s.cloud_init;
    let Some(vm) = p.vm else { return };
    if let Err(e) = CloudInitInputs::validate(
        p.hostname.text.trim(),
        &p.user_data.text,
        (!p.network.text.trim().is_empty()).then_some(p.network.text.as_str()),
    ) {
        p.error = Some(e.to_string());
        return;
    }
    let inputs = CloudInitInputs::new(
        p.hostname.text.trim().into(),
        std::mem::take(&mut p.user_data.text),
        (!p.network.text.trim().is_empty()).then(|| std::mem::take(&mut p.network.text)),
    );
    let inputs = match inputs {
        Ok(v) => v,
        Err(e) => {
            p.error = Some(e.to_string());
            return;
        }
    };
    let (tx, rx) = mpsc::channel();
    let wake = s.waker.clone();
    match std::thread::Builder::new()
        .name("cloud-init-media".into())
        .spawn(move || {
            let result = CloudInitMedia::prepare(vm, inputs).map_err(|e| e.to_string());
            let _ = tx.send(result);
            if let Some(w) = wake {
                w();
            }
        }) {
        Ok(_) => {
            p.pending = true;
            p.receiver = Some(rx);
            p.error = None;
            p.user_data = TextEdit::default();
            p.network = TextEdit::default();
        }
        Err(_) => p.error = Some("Cannot start private media preparation".into()),
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let Some(rx) = &s.cloud_init.receiver else {
        return false;
    };
    let result = match rx.try_recv() {
        Ok(v) => v,
        Err(mpsc::TryRecvError::Empty) => return false,
        Err(_) => Err("Private media preparation stopped".into()),
    };
    s.cloud_init.receiver = None;
    s.cloud_init.pending = false;
    match result {
        Ok(media) => s.cloud_init.media = Some(media),
        Err(e) => s.cloud_init.error = Some(e),
    };
    true
}
fn apply(s: &mut State) {
    if s.demo || !s.can_mutate() || !can_prepare(s) {
        return;
    }
    let Some(observed) = s.cloud_init.observed else {
        return;
    };
    let Some(config) = config(&s.cloud_init) else {
        return;
    };
    let Some(media) = s.cloud_init.media.take() else {
        return;
    };
    let result = s
        .connection
        .as_ref()
        .ok_or(xenadmin_core::Error::Closed)
        .and_then(|c| {
            c.prepare_cloud_init(
                OperationId::new(),
                observed,
                config.sr,
                config.device,
                media,
            )
        });
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            close(s);
            activity::open(s);
        }
        Err(e) => s.cloud_init.error = Some(e.to_string()),
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let p = &s.cloud_init;
    let mut body = col()
        .w((s.window_width - 36.0).clamp(270.0, 760.0))
        .max_h((s.window_height - 50.0).clamp(260.0, 900.0))
        .padding(22.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .child(
            row()
                .gap(8.0)
                .flex_wrap()
                .child(heading(
                    if p.media.is_some() {
                        "Review cloud-init drive"
                    } else {
                        "Prepare cloud-init drive"
                    },
                    k,
                ))
                .child(
                    Button::new("Close")
                        .on_press(close)
                        .build(k)
                        .key("cloud-init-close"),
                ),
        )
        .child(heading(&p.name, k))
        .child(
            caption(
                format!(
                    "VM {} · pool {}",
                    p.vm.map(|i| i.to_string()).unwrap_or_default(),
                    p.pool.map(|i| i.to_string()).unwrap_or_default()
                ),
                k,
            )
            .wrap_anywhere(),
        );
    if !current(s) {
        body = body.child(
            caption(
                "Connection or pool changed. Close this review and reopen it from the intended VM.",
                k,
            )
            .wrap(),
        );
    }
    if let Some((vm, config)) = p.vm.zip(config(p))
        && let Some(i) = &s.inventory
    {
        let available = config.availability(i, vm);
        if !available.is_available() {
            body = body.child(caption(available.explanation(), k).wrap());
        }
    }
    if !s.demo && !s.can_mutate() {
        body=body.child(caption("A current connection with write access is required to upload and attach this drive.",k).wrap());
    }
    if let Some(e) = &p.error {
        body = body.child(text(e.clone()).color(k.c.text_base).wrap());
    }
    if let Some(media) = &p.media {
        let sr =
            p.sr.and_then(|id| {
                s.inventory
                    .as_ref()?
                    .storage
                    .iter()
                    .find(|sr| sr.uuid == id.uuid())
            })
            .map(|r| format!("{} ({})", r.name, r.uuid))
            .unwrap_or_else(|| "Unavailable storage".into());
        body=body.child(caption(format!("Hostname: {}\nStorage: {}\nDisk slot: {}\nMedia: 8 MiB FAT16 · CIDATA\nConfiguration ID: {}",p.hostname.text,sr,p.device.text,media.configuration_id()),k).wrap_anywhere())
            .child(caption("Create a new data disk, upload the prepared media, then attach it read-only. The drive contains your user-data and stays attached. Review its removal separately after provisioning.",k).wrap())
            .child(caption("The VM stays halted. Its guest must already support cloud-init with the NoCloud datasource. Preparing this drive does not prove the guest accepted or ran the configuration. A new instance ID may cause an existing cloud-init guest to run first-boot configuration again. External seed media can affect which configuration the guest selects.",k).wrap())
            .child(caption("Other changes in this pool wait while this operation is active or unresolved. Interrupted uploads or attachments are never retried automatically. Retained disk and attachment UUIDs appear in Activity; uncertain operations may retain a private local artifact.",k).wrap())
            .child(row().gap(8.0).flex_wrap().child(Button::new("Discard and edit").on_press(|s:&mut State|{s.cloud_init.media=None;s.cloud_init.user_data=TextEdit::new("#cloud-config\n");}).build(k).key("cloud-init-edit"))
                .child(Button::new("Upload and attach drive").disabled(s.demo||!s.can_mutate()||!can_prepare(s)).on_press(apply).build(k).key("cloud-init-apply")));
    } else if p.pending {
        body = body.child(caption("Preparing private local media…", k).wrap());
    } else {
        body=body.child(caption("For a halted VM with cloud-init and the NoCloud datasource already installed. Enter UTF-8 user-data; guest-side configuration validation happens in the guest.",k).wrap())
            .child(caption("Guest hostname",k))
            .child(TextInput::new(&p.hostname).label("Guest hostname").on_edit(|s:&mut State|&mut s.cloud_init.hostname).build(k).key("cloud-init-hostname"))
            .child(caption("Unused disk slot (0–255)",k))
            .child(TextInput::new(&p.device).label("Unused disk slot (0–255)").on_edit(|s:&mut State|&mut s.cloud_init.device).build(k).key("cloud-init-device"))
            .child(heading("Choose storage for the new drive",k));
        let mut choices = col().gap(6.0).max_h(150.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
        if let Some(i) = &s.inventory {
            for sr in i
                .details
                .storage
                .values()
                .filter(|sr| sr.content_type == "user")
            {
                let id = sr.id;
                choices = choices.child(
                    Button::new(format!(
                        "{}{}",
                        if p.sr == Some(id) { "✓ " } else { "" },
                        i.storage
                            .iter()
                            .find(|s| s.uuid == sr.id.uuid())
                            .map(|s| s.name.as_str())
                            .unwrap_or("Unnamed storage")
                    ))
                    .on_press(move |s: &mut State| s.cloud_init.sr = Some(id))
                    .build(k)
                    .key(format!("cloud-init-sr-{id}")),
                );
            }
        }
        body=body.child(choices.key("cloud-init-storage"))
            .child(caption("User-data (up to 2 MiB)",k))
            .child(rune_kit::TextArea::new(&p.user_data).label("User-data (up to 2 MiB)").on_edit(|s:&mut State|&mut s.cloud_init.user_data).build(k).h(150.0).key("cloud-init-user-data"))
            .child(caption("Optional network-config (up to 1 MiB)",k))
            .child(rune_kit::TextArea::new(&p.network).label("Optional network-config (up to 1 MiB)").on_edit(|s:&mut State|&mut s.cloud_init.network).build(k).h(100.0).key("cloud-init-network"))
            .child(caption("Inputs stay in private local media and are never included in operation history or exports. Only the reviewed VM receives the new drive.",k).wrap())
            .child(Button::new("Prepare and review").disabled(!can_prepare(s)).on_press(prepare).build(k).key("cloud-init-prepare"));
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Cloud-init drive")
        .modal(false, close)
        .key("cloud-init-dialog")
        .child(body)
}
pub(super) fn scene(s: &mut State) {
    demo(s);
    let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
    for (index, sr) in inventory.details.storage.keys().copied().enumerate() {
        let id = xenadmin_core::PbdId::from(uuid::Uuid::from_u128(900 + index as u128));
        inventory.details.pbds.insert(
            id,
            xenadmin_core::StorageAttachment {
                id,
                location: None,
                requires_credentials: false,
                multipathed: Some(false),
                paths: vec![],
                host: inventory.hosts.keys().next().copied(),
                sr: Some(sr),
                currently_attached: Some(true),
            },
        );
    }
    let Some(vm) = s.inventory.as_ref().and_then(|i| {
        i.guests()
            .find(|v| v.power_state == PowerState::Halted)
            .map(|v| v.id)
    }) else {
        return;
    };
    open(s, vm);
    s.cloud_init.hostname = TextEdit::new("fixture-guest");
    s.cloud_init.sr = s.inventory.as_ref().and_then(|i| {
        i.details
            .storage
            .values()
            .find(|sr| sr.content_type == "user")
            .map(|sr| sr.id)
    });
}

#[cfg(test)]
#[path = "cloud_init_view_tests.rs"]
mod tests;

pub(super) fn review_scene(s: &mut State) {
    scene(s);
    let p = &mut s.cloud_init;
    if let Some(vm) = p.vm {
        // Synthetic render setup only; the interactive path prepares on a background thread.
        p.media = CloudInitMedia::prepare(
            vm,
            CloudInitInputs::new(
                "fixture-guest".into(),
                "#cloud-config\nwrite_files: []\n".into(),
                None,
            )
            .unwrap(),
        )
        .ok();
        p.user_data = TextEdit::default();
    }
}
