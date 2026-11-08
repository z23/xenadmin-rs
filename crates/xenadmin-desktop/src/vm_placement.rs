use super::*;
use xenadmin_core::{ObservedVm, PowerAction};
pub(super) struct Placement {
    target: ObservedVm,
    pool: PoolId,
    vm: VmId,
    name: String,
    action: PowerAction,
    host: Option<(HostId, String)>,
}
pub(super) fn open(s: &mut State) {
    let result = (|| {
        let id = s
            .selected()
            .and_then(|r| r.vm)
            .ok_or(xenadmin_core::Error::Stale)?;
        let snapshot = s.snapshot.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let inventory = snapshot.inventory().ok_or(xenadmin_core::Error::Stale)?;
        let vm = inventory.vms.get(&id).ok_or(xenadmin_core::Error::Stale)?;
        let action = match vm.power_state {
            PowerState::Halted => PowerAction::Start,
            PowerState::Suspended => PowerAction::Resume,
            _ => return Err(xenadmin_core::Error::UnavailableAction),
        };
        s.placement = Some(Placement {
            target: snapshot.observe_vm(id)?,
            pool: inventory.pool_id,
            vm: id,
            name: vm.name.clone(),
            action,
            host: None,
        });
        Ok::<(), xenadmin_core::Error>(())
    })();
    s.error = result.err().map(|e| e.to_string());
}
fn requirements(placement: &Placement) -> Option<xenadmin_core::PermissionRequirements> {
    let (host, _) = placement.host.as_ref()?;
    Some(
        xenadmin_core::OperationAction::Placement(xenadmin_core::PlacementOperation {
            placement: placement.action,
            host: *host,
        })
        .permission_requirements(),
    )
}
fn submit(s: &mut State) {
    let Some(placement) = &s.placement else {
        return;
    };
    let Some((host, _)) = &placement.host else {
        return;
    };
    let command = xenadmin_core::VmCommand::PowerOnHost {
        vm: placement.target,
        action: placement.action,
        host: *host,
    };
    let Some(required) = requirements(placement) else {
        return;
    };
    if !session_permissions_view::check(s, &required) {
        return;
    }
    s.submit_command(command);
    if s.error.is_none() {
        s.placement = None;
    }
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(placement) = &s.placement else {
        return el();
    };
    let mut body = col()
        .w((s.window_width - 40.0).clamp(280.0, 520.0))
        .max_h(600.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        })
        .padding(24.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading(
            format!("{} on a selected host", placement.action.label()),
            k,
        ))
        .child(
            caption(format!("{}\nVM UUID: {}", placement.name, placement.vm), k).wrap_anywhere(),
        );
    if let Some((host, name)) = &placement.host {
        body=body.child(heading(format!("Destination: {name}"),k)).child(caption(format!("Host UUID: {host}"),k).wrap_anywhere())
            .child(caption("The host must be enabled and compatible with this VM's storage and network requirements. These prerequisites are checked again before submission.",k).wrap());
    } else if let Some(inventory) = s.inventory.as_ref().filter(|i| i.pool_id == placement.pool) {
        for host in inventory.hosts.values() {
            let id = host.id;
            let name = host.name.clone();
            body = body.child(
                Button::new(&host.name)
                    .variant(ButtonVariant::Outline)
                    .disabled(!host.enabled)
                    .on_press(move |s: &mut State| {
                        if let Some(p) = &mut s.placement {
                            p.host = Some((id, name.clone()));
                        }
                    })
                    .build(k),
            );
        }
    }
    if let Some(error) = &s.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .role("alert")
                .color(k.c.danger.icon),
        );
    }
    let permission = requirements(placement).map(|r| session_permissions_view::assess(s, &r));
    if let Some(permission) = &permission {
        body = body.child(session_permissions_view::review_notice(permission, k));
    }
    let mut controls = row().gap(8.0).justify_end().child(
        Button::new("Cancel")
            .variant(ButtonVariant::Ghost)
            .on_press(|s: &mut State| s.placement = None)
            .build(k),
    );
    if placement.host.is_some() {
        controls = controls
            .child(
                Button::new("Choose another host")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        if let Some(p) = &mut s.placement {
                            p.host = None
                        }
                    })
                    .build(k),
            )
            .child(
                Button::new(placement.action.label())
                    .variant(ButtonVariant::Outline)
                    .disabled(
                        !s.can_mutate()
                            || permission
                                .as_ref()
                                .is_some_and(|p| p.decision().is_denied())
                            || s.snapshot
                                .as_ref()
                                .and_then(|s| s.observe_vm(placement.vm).ok())
                                != Some(placement.target),
                    )
                    .on_press(submit)
                    .build(k),
            );
    }
    el().role("dialog")
        .label("Choose a destination host")
        .w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .modal(false, |s: &mut State| s.placement = None)
        .child(body.child(controls))
}
