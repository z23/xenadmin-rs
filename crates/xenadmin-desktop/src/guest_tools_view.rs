use super::*;
use xenadmin_core::{DeviceOperation, ObservedVm, VbdId, VdiId};

pub(super) struct Panel {
    inventory: Arc<Inventory>,
    vm: VmId,
    observed: Option<ObservedVm>,
    cd: Option<VbdId>,
    image: Option<VdiId>,
    review: Option<DeviceOperation>,
    submitted: Option<xenadmin_core::OperationId>,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let Some(inventory) = s.inventory.clone() else {
        return;
    };
    let vm = match &s.devices {
        Some(view) if view.pool == inventory.pool_id => view.vm,
        Some(_) => return,
        None => match s.selected().and_then(|r| r.vm) {
            Some(vm) => vm,
            None => return,
        },
    };
    if !inventory.vms.contains_key(&vm) {
        return;
    }
    let cd = inventory
        .details
        .vm_disks
        .get(&vm)
        .into_iter()
        .flatten()
        .find(|id| {
            inventory
                .details
                .vbds
                .get(id)
                .is_some_and(|v| v.kind == "CD")
        })
        .copied();
    console_view::release(s);
    s.guest_tools = Some(Panel {
        inventory,
        vm,
        observed: s.snapshot.as_ref().and_then(|v| v.observe_vm(vm).ok()),
        cd,
        image: None,
        review: None,
        submitted: None,
        error: None,
    });
}
fn capture(p: &Panel) -> Result<DeviceOperation, String> {
    let cd = p.cd.ok_or("Select a CD drive")?;
    let image = p.image.ok_or("Select guest tools media")?;
    let drive = p
        .inventory
        .details
        .vbds
        .get(&cd)
        .ok_or("CD drive unavailable")?;
    let previous = match drive.empty {
        Some(true) if drive.vdi.is_none() => None,
        Some(false) => Some(drive.vdi.ok_or("Mounted image identity is unavailable")?),
        _ => return Err("Mounted image state is unknown".into()),
    };
    let sr = p
        .inventory
        .details
        .vdis
        .get(&image)
        .and_then(|d| d.sr)
        .ok_or("Image storage repository is unavailable")?;
    Ok(DeviceOperation::MountGuestTools {
        vbd: cd,
        image,
        sr,
        previous,
    })
}
fn review(s: &mut State) {
    let Some(p) = &mut s.guest_tools else { return };
    match capture(p) {
        Ok(op) => {
            p.review = Some(op);
            p.error = None;
        }
        Err(e) => p.error = Some(e),
    }
}
fn apply(s: &mut State) {
    let Some(p) = &s.guest_tools else { return };
    if p.submitted.is_some() {
        return;
    }
    let Some(op) = p.review.clone() else { return };
    let Some(observed) = p.observed else { return };
    if s.demo
        || !s.can_mutate()
        || s.snapshot.as_ref().and_then(|v| v.observe_vm(p.vm).ok()) != Some(observed)
        || s.inventory.as_ref().is_none_or(|i| {
            i.pool_id != p.inventory.pool_id || !op.availability(i, p.vm).is_available()
        })
    {
        return;
    }
    if !session_permissions_view::check(s, &op.permission_requirements()) {
        return;
    }
    s.operations
        .retain(|operation| !operation.latest().state.terminal());
    if s.operations.len() >= 64 {
        s.guest_tools.as_mut().unwrap().error = Some(xenadmin_core::Error::Busy.to_string());
        return;
    }
    let id = xenadmin_core::OperationId::new();
    let result = s.connection.as_ref().unwrap().submit(
        id,
        xenadmin_core::VmCommand::Device(xenadmin_core::DeviceCommand {
            vm: observed,
            operation: op,
        }),
    );
    match result {
        Ok(operation) => {
            if let Some(wake) = &s.waker {
                operation.notify_on_change(wake.clone());
            }
            s.operations.push(operation);
            s.guest_tools.as_mut().unwrap().submitted = Some(id);
        }
        Err(e) => s.guest_tools.as_mut().unwrap().error = Some(e.to_string()),
    }
}
fn field(label: impl Into<String>, value: impl Into<String>, k: &Kit) -> El {
    col().gap(3.0).child(caption(label, k)).child(
        text(value.into())
            .font(k.ty.text_small)
            .color(k.c.text_base)
            .wrap_anywhere(),
    )
}
fn image_name(i: &Inventory, id: VdiId) -> String {
    i.details
        .vdis
        .get(&id)
        .map(|d| format!("{} ({id})", d.name))
        .unwrap_or_else(|| id.to_string())
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.guest_tools else { return el() };
    let i = &p.inventory;
    let mut body=col().gap(12.0).child(field("VM",format!("{} ({})",i.vms.get(&p.vm).map(|v|v.name.as_str()).unwrap_or("Unavailable"),p.vm),k))
        .child(caption("Mount the host's guest tools ISO, then run the appropriate installer inside the guest. Existing guest tools and drivers should be checked before changing versions.",k).wrap());
    if let Some(error) = &p.error {
        body = body.child(caption(error, k).wrap().role("alert"));
    }
    if let Some(id) = p.submitted {
        let record = s
            .operations
            .iter()
            .find(|o| o.latest().id == id)
            .map(|o| (*o.latest()).clone())
            .or_else(|| s.operation_rows.iter().find(|r| r.id == id).cloned());
        if let Some(record) = record {
            body = body.child(field(
                "Operation",
                format!("{:?}: {}", record.state, record.detail),
                k,
            ));
            for step in record.steps {
                body = body.child(caption(format!("{} · {:?}", step.label, step.state), k).wrap());
            }
            if record.state == xenadmin_core::OperationState::Succeeded {
                body=body.child(caption("Tools media is mounted. Open the console to run the Windows installer or the Linux installer from the mounted media. Guest tools status will update when its agent reports to the host.",k).wrap());
            }
        } else {
            body = body.child(caption(
                "Operation admitted; open Activity to inspect its progress.",
                k,
            ));
        }
    } else if let Some(op) = &p.review {
        let DeviceOperation::MountGuestTools {
            vbd,
            image,
            sr,
            previous,
        } = op
        else {
            return el();
        };
        body = body
            .child(field("CD drive", vbd.to_string(), k))
            .child(field("Tools ISO", image_name(i, *image), k))
            .child(field("Storage repository", sr.to_string(), k));
        body=body.child(caption(if let Some(old)=previous {format!("Eject {} first, then mount the selected tools ISO. If mounting fails, the drive can remain empty; inspect the operation before retrying.",image_name(i,*old))}else{"Mount the selected tools ISO in the reviewed empty CD drive.".into()},k).wrap());
        let available = s
            .inventory
            .as_ref()
            .filter(|live| live.pool_id == i.pool_id)
            .map(|i| op.availability(i, p.vm));
        if let Some(status) = available.filter(|a| !a.is_available()) {
            body = body.child(caption(status.explanation(), k).wrap());
        }
    } else {
        body = body.child(heading("CD drive", k));
        let cds: Vec<_> = i
            .details
            .vm_disks
            .get(&p.vm)
            .into_iter()
            .flatten()
            .filter_map(|id| i.details.vbds.get(id))
            .filter(|v| v.kind == "CD")
            .collect();
        if cds.is_empty() {
            body=body.child(caption("No CD drive is available. Shut down the VM and add an empty CD drive from Disks and network interfaces.",k).wrap());
        }
        let mut choices = col().gap(4.0).h(95.0).min_h(95.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
        for cd in cds.iter().take(128) {
            let id = cd.id;
            let contents = match (cd.empty, cd.vdi) {
                (Some(true), None) => "Empty".into(),
                (Some(false), Some(id)) => image_name(i, id),
                _ => "Media state unavailable".into(),
            };
            choices = choices.child(
                col()
                    .w_full()
                    .gap(4.0)
                    .child(field(format!("Drive {}", cd.userdevice), contents, k))
                    .child(
                        Button::new(if p.cd == Some(id) {
                            "✓ Selected drive"
                        } else {
                            "Select drive"
                        })
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            if let Some(p) = &mut s.guest_tools {
                                p.cd = Some(id);
                            }
                        })
                        .build(k)
                        .key(format!("tools-cd-{id}")),
                    ),
            );
        }
        body = body.child(choices).child(heading("Guest tools ISO", k));
        if i.details.tools_media.is_empty() {
            body=body.child(caption("No guest tools ISO is advertised or recognized in this inventory. Check the host's tools repository or add the appropriate tools ISO to an ISO library.",k).wrap());
        }
        let mut choices = col()
            .gap(4.0)
            .h(140.0)
            .min_h(140.0)
            .scroll(rune::ScrollSpec {
                stick_bottom: false,
            });
        for (id, source) in i.details.tools_media.iter().take(256) {
            let id = *id;
            choices = choices.child(
                col()
                    .w_full()
                    .gap(4.0)
                    .child(field(source.label(), image_name(i, id), k))
                    .child(
                        Button::new(if p.image == Some(id) {
                            "✓ Selected tools ISO"
                        } else {
                            "Select tools ISO"
                        })
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            if let Some(p) = &mut s.guest_tools {
                                p.image = Some(id);
                            }
                        })
                        .build(k)
                        .key(format!("tools-image-{id}")),
                    ),
            );
        }
        body = body.child(choices);
        if cds.len() > 128 || i.details.tools_media.len() > 256 {
            body=body.child(caption("This pool exceeds the media chooser's display limit; reduce the advertised tools media before continuing.",k).wrap());
        }
    }
    let permission = p
        .review
        .as_ref()
        .map(|op| session_permissions_view::assess(s, &op.permission_requirements()));
    let mut footer = col().gap(5.0);
    if let Some(permission) = &permission {
        footer = footer.child(session_permissions_view::review_notice(permission, k));
    }
    let mut controls = row().gap(6.0).flex_wrap();
    if p.submitted.is_none() {
        if let Some(op) = &p.review {
            let allowed = !s.demo
                && s.can_mutate()
                && p.observed.is_some()
                && s.snapshot.as_ref().and_then(|v| v.observe_vm(p.vm).ok()) == p.observed
                && s.inventory.as_ref().is_some_and(|i| {
                    i.pool_id == p.inventory.pool_id && op.availability(i, p.vm).is_available()
                })
                && !permission
                    .as_ref()
                    .is_some_and(|p| p.decision().is_denied());
            controls = controls
                .child(
                    Button::new("Mount tools media")
                        .disabled(!allowed)
                        .on_press(apply)
                        .build(k)
                        .key("tools-confirm"),
                )
                .child(
                    Button::new("Back")
                        .on_press(|s: &mut State| {
                            if let Some(p) = &mut s.guest_tools {
                                p.review = None
                            }
                        })
                        .build(k)
                        .key("tools-back"),
                );
        } else {
            controls = controls.child(
                Button::new("Review mounting")
                    .disabled(p.cd.is_none() || p.image.is_none())
                    .on_press(review)
                    .build(k)
                    .key("tools-review"),
            );
        }
    }
    controls = controls
        .child(
            Button::new("Installation guide")
                .variant(ButtonVariant::Outline)
                .on_press_tx(|s: &mut State, tx: &mut Tx| {
                    if let Err(e) = tx.open_url("https://docs.xcp-ng.org/vms/")
                        && let Some(p) = &mut s.guest_tools
                    {
                        p.error = Some(e.to_string());
                    }
                })
                .build(k),
        )
        .child(
            Button::new("Close")
                .on_press(|s: &mut State| s.guest_tools = None)
                .build(k)
                .key("tools-close"),
        );
    let card = col()
        .w((s.window_width - 32.0).clamp(280.0, 780.0))
        .h((s.window_height - 48.0).clamp(280.0, 800.0))
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading("Guest tools media", k))
        .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(footer.child(controls));
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Guest tools media")
        .modal(false, |s: &mut State| s.guest_tools = None)
        .child(card)
        .key("tools-dialog")
}

pub(super) fn scene(s: &mut State) {
    demo(s);
    let vm = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let sr = uuid::Uuid::from_u128(0x76000000000000000000000000000001).into();
    let image = uuid::Uuid::from_u128(0x76000000000000000000000000000002).into();
    let pbd = uuid::Uuid::from_u128(0x76000000000000000000000000000003).into();
    let mut storage = i.details.storage.values().next().unwrap().clone();
    storage.id = sr;
    storage.content_type = "iso".into();
    i.details.storage.insert(sr, storage);
    let mut disk = i.details.vdis.values().next().unwrap().clone();
    disk.id = image;
    disk.sr = Some(sr);
    disk.name = "guest-tools.iso".into();
    disk.read_only = Some(true);
    disk.missing = Some(false);
    i.details.vdis.insert(image, disk);
    i.details
        .tools_media
        .insert(image, xenadmin_core::GuestToolsMediaSource::HostFlag);
    i.details.pbds.insert(
        pbd,
        xenadmin_core::StorageAttachment {
            id: pbd,
            location: None,
            requires_credentials: false,
            multipathed: None,
            paths: vec![],
            host: i.hosts.keys().next().copied(),
            sr: Some(sr),
            currently_attached: Some(true),
        },
    );
    i.details
        .classes
        .insert("pbd".into(), xenadmin_core::ClassAvailability::Available);
    s.table.select(vm.to_string());
    open(s);
}
pub(super) fn review_scene(s: &mut State) {
    scene(s);
    let p = s.guest_tools.as_mut().unwrap();
    p.image = p.inventory.details.tools_media.keys().next().copied();
    review(s);
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
        let (x, y) = r.find(key).unwrap_or_else(|| panic!("Missing {key}"));
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        settle(r);
    }
    #[test]
    fn tools_review_captures_explicit_media_and_identity_and_fixed_footer_at_both_sizes() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = init();
            scene(&mut s);
            assert!(s.guest_tools.as_ref().unwrap().image.is_none());
            let image = *s
                .inventory
                .as_ref()
                .unwrap()
                .details
                .tools_media
                .keys()
                .next()
                .unwrap();
            let mut r = runtime(s, size);
            click(&mut r, &format!("tools-image-{image}"));
            click(&mut r, "tools-review");
            let p = r.state.guest_tools.as_ref().unwrap();
            let original = p.vm;
            let review = p.review.clone().unwrap();
            assert!(
                matches!(review,DeviceOperation::MountGuestTools{image: id,previous:None,..}if id==image)
            );
            r.update(|s, _| {
                s.table.select("30000000-0000-0000-0000-000000000001");
            });
            settle(&mut r);
            assert_eq!(r.state.guest_tools.as_ref().unwrap().vm, original);
            let (_, y) = r.find("tools-confirm").unwrap();
            assert!(y > 0.0 && y < size.1);
            click(&mut r, "tools-confirm");
            assert!(r.state.operations.is_empty());
            assert_eq!(
                r.state.guest_tools.as_ref().unwrap().review.as_ref(),
                Some(&review)
            );
            click(&mut r, "tools-close");
            assert!(r.state.guest_tools.is_none());
        }
    }
    #[test]
    fn tools_review_explains_existing_media_and_missing_devices_without_issuing_commands() {
        let mut s = init();
        scene(&mut s);
        let p = s.guest_tools.as_mut().unwrap();
        let old = *p
            .inventory
            .details
            .vdis
            .keys()
            .find(|id| !p.inventory.details.tools_media.contains_key(id))
            .unwrap();
        let i = Arc::make_mut(&mut p.inventory);
        let cd = i.details.vbds.get_mut(&p.cd.unwrap()).unwrap();
        cd.empty = Some(false);
        cd.vdi = Some(old);
        p.image = p.inventory.details.tools_media.keys().next().copied();
        review(&mut s);
        let mut r = runtime(s, (513.0, 652.0));
        let tree = r.accessibility_json().to_string();
        assert!(tree.contains("Eject"));
        assert!(tree.contains("remain empty"));
        click(&mut r, "tools-confirm");
        assert!(r.state.operations.is_empty());
        let mut s = init();
        scene(&mut s);
        let p = s.guest_tools.as_mut().unwrap();
        let i = Arc::make_mut(&mut p.inventory);
        i.details.vm_disks.clear();
        i.details.tools_media.clear();
        p.cd = None;
        let r = runtime(s, (513.0, 652.0));
        let tree = r.accessibility_json().to_string();
        assert!(tree.contains("No CD drive"));
        assert!(tree.contains("No guest tools ISO"));
    }
    #[test]
    fn tools_panel_refuses_device_view_from_another_pool() {
        let mut s = init();
        scene(&mut s);
        s.guest_tools = None;
        vm_devices::open(&mut s);
        s.devices.as_mut().unwrap().pool = uuid::Uuid::from_u128(999).into();
        open(&mut s);
        assert!(s.guest_tools.is_none());
    }
}
