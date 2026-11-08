use super::*;
use std::time::{SystemTime, UNIX_EPOCH};
use xenadmin_core::{ClassAvailability, VmDetails};

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Target {
    Vm(VmId),
    Host(HostId),
    Pool,
}
pub(super) struct Panel {
    target: Target,
    inventory: Arc<Inventory>,
    observed_at: SystemTime,
    current: bool,
    synthetic: bool,
    error: Option<String>,
}
fn capture(s: &State, target: Target) -> Option<Panel> {
    Some(Panel {
        target,
        inventory: s.inventory.clone()?,
        observed_at: s
            .snapshot
            .as_ref()
            .and_then(|v| v.updated_at())
            .unwrap_or_else(SystemTime::now),
        current: s.snapshot.as_ref().is_some_and(|v| v.is_current()),
        synthetic: s.demo,
        error: None,
    })
}
pub(super) fn open(s: &mut State) {
    let Some(row) = s.selected() else { return };
    let target = if let Some(vm) = row.vm {
        Target::Vm(vm)
    } else if s.page == Page::Hosts || console_view::host(s).is_some() {
        let Ok(id) = HostId::parse(&row.id) else {
            return;
        };
        Target::Host(id)
    } else {
        return;
    };
    console_view::release(s);
    s.information = capture(s, target);
}
pub(super) fn open_pool(s: &mut State) {
    console_view::release(s);
    s.information = capture(s, Target::Pool);
}
fn refresh(s: &mut State) {
    let Some(panel) = &s.information else { return };
    if !s
        .inventory
        .as_ref()
        .is_some_and(|i| i.pool_id == panel.inventory.pool_id)
    {
        s.information.as_mut().unwrap().error =
            Some("Return to the same pool before refreshing this information".into());
        return;
    }
    s.information = capture(s, panel.target);
}
fn field(label: impl Into<String>, value: impl Into<String>, k: &Kit) -> El {
    col().gap(4.0).child(caption(label, k)).child(
        text(value.into())
            .font(k.ty.text_small)
            .color(k.c.text_base)
            .wrap_anywhere()
            .selectable(true),
    )
}
fn known(v: Option<bool>) -> &'static str {
    match v {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}
fn supplied(v: Option<&str>) -> &str {
    v.filter(|v| !v.is_empty()).unwrap_or("Not reported")
}
fn number(v: Option<u64>) -> String {
    v.map(|v| v.to_string()).unwrap_or_else(|| "Unknown".into())
}
fn elapsed(seconds: u64) -> String {
    format!(
        "{}d {}h {}m",
        seconds / 86400,
        seconds % 86400 / 3600,
        seconds % 3600 / 60
    )
}
fn host(i: &Inventory, id: Option<HostId>) -> String {
    id.map(|id| {
        i.hosts
            .get(&id)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| format!("Unresolved host {id}"))
    })
    .unwrap_or_else(|| "None / not reported".into())
}
fn sr(i: &Inventory, id: Option<xenadmin_core::SrId>) -> String {
    id.map(|id| {
        i.storage
            .iter()
            .find(|v| v.uuid == id.uuid())
            .map(|v| v.name.clone())
            .unwrap_or_else(|| format!("Unresolved storage {id}"))
    })
    .unwrap_or_else(|| "None / not reported".into())
}
fn map(
    mut body: El,
    label: &str,
    values: &std::collections::BTreeMap<String, String>,
    k: &Kit,
) -> El {
    if values.is_empty() {
        return body.child(field(label, "Not reported", k));
    }
    for (name, value) in values {
        body = body.child(field(format!("{label} · {name}"), value, k));
    }
    body
}
fn class(body: El, i: &Inventory, name: &str, k: &Kit) -> El {
    let message = match i.details.classes.get(&name.to_ascii_lowercase()) {
        Some(ClassAvailability::Available) => return body,
        Some(ClassAvailability::PermissionDenied) => "Permission denied",
        Some(ClassAvailability::Unsupported) => "Unsupported by this host",
        Some(ClassAvailability::InvalidData) => "Invalid data received",
        _ => "Availability unknown",
    };
    body.child(field(name, message, k))
}
fn vm_body(p: &Panel, id: VmId, mut b: El, k: &Kit) -> El {
    let i = &p.inventory;
    let Some(vm) = i.vms.get(&id) else {
        return b
            .child(caption("This VM is no longer present in the captured inventory", k).wrap());
    };
    b = b
        .child(heading(&vm.name, k))
        .child(field("VM UUID", id.to_string(), k))
        .child(field("Description", supplied(Some(&vm.description)), k))
        .child(field("Power state", vm.power_state.to_string(), k))
        .child(field("Resident host", host(i, vm.host), k));
    let Some(d) = i.details.vms.get(&id) else {
        return b.child(caption("VM properties are unavailable", k));
    };
    b = vm_properties(b, d, i, k);
    b = class(b, i, "VM_metrics", k);
    if let Some(m) = d.metrics.and_then(|id| i.details.vm_metrics.get(&id)) {
        b = b
            .child(heading("Reported runtime", k))
            .child(field("Actual memory", storage_bytes(m.memory_actual), k))
            .child(field("Current virtual CPUs", number(m.vcpus_number), k))
            .child(field(
                "Last boot (host UTC)",
                supplied(m.start_time.as_deref()),
                k,
            ))
            .child(field(
                "Installed (host UTC)",
                supplied(m.install_time.as_deref()),
                k,
            ))
            .child(field(
                "Metrics updated (host UTC)",
                supplied(m.last_updated.as_deref()),
                k,
            ))
            .child(field(
                "Runtime domain type",
                supplied(m.current_domain_type.as_deref()),
                k,
            ))
            .child(field(
                "Runtime flags",
                supplied(Some(&m.state.join(", "))),
                k,
            ));
        if matches!(vm.power_state, PowerState::Running | PowerState::Paused) {
            b=b.child(field("Elapsed since boot at capture",m.elapsed_since_boot(p.observed_at).map(elapsed).unwrap_or_else(||"Unknown; boot time is missing, invalid or ahead of this computer's clock".into()),k))
                .child(caption("Elapsed time uses this computer's clock and includes paused time. It is not guest-measured uptime.",k).wrap());
        }
    } else {
        b = b.child(caption("No VM runtime metrics record is available", k).wrap());
    }
    b = class(b, i, "VM_guest_metrics", k);
    if let Some(m) = d
        .guest_metrics
        .and_then(|id| i.details.guest_metrics.get(&id))
    {
        b = b
            .child(heading("Guest-reported information", k))
            .child(field("Guest agent live", known(m.live), k))
            .child(field(
                "Guest data updated (host UTC)",
                supplied(m.last_updated.as_deref()),
                k,
            ));
        if m.live != Some(true) {
            b=b.child(caption("The guest agent is not reporting live data. Its addresses and OS details may be stale.",k).wrap());
        }
        b = map(b, "Operating system", &m.os_version, k);
        b = map(b, "Tools", &m.tools_version, k);
        b = map(b, "Address", &m.networks, k);
    }
    b
}
fn vm_properties(mut b: El, d: &VmDetails, i: &Inventory, k: &Kit) -> El {
    b = b
        .child(heading("Configured resources and boot", k))
        .child(field(
            "vCPUs · startup / maximum",
            format!("{} / {}", number(d.vcpus_at_startup), number(d.vcpus_max)),
            k,
        ))
        .child(field(
            "Static memory · minimum / maximum",
            format!(
                "{} / {}",
                storage_bytes(d.memory_static_min),
                storage_bytes(d.memory_static_max)
            ),
            k,
        ))
        .child(field(
            "Dynamic memory · minimum / maximum",
            format!(
                "{} / {}",
                storage_bytes(d.memory_dynamic_min),
                storage_bytes(d.memory_dynamic_max)
            ),
            k,
        ))
        .child(field(
            "Home host",
            if d.affinity_known {
                host(i, d.affinity)
            } else {
                "Unknown".into()
            },
            k,
        ))
        .child(field(
            "Firmware / Secure Boot",
            format!(
                "{} / {}",
                supplied(d.firmware.as_deref()),
                known(d.secure_boot)
            ),
            k,
        ))
        .child(field(
            "Boot policy / order",
            format!(
                "{} / {}",
                supplied(d.boot_policy.as_deref()),
                supplied(d.boot_order.as_deref())
            ),
            k,
        ))
        .child(field("Autostart", known(d.auto_power_on), k))
        .child(field(
            "HA restart priority",
            supplied(d.ha_restart_priority.as_deref()),
            k,
        ))
        .child(field("Tags", supplied(Some(&d.tags.join(", "))), k));
    b
}
fn host_body(p: &Panel, id: HostId, mut b: El, k: &Kit) -> El {
    let i = &p.inventory;
    let Some(h) = i.hosts.get(&id) else {
        return b.child(
            caption(
                "This host is no longer present in the captured inventory",
                k,
            )
            .wrap(),
        );
    };
    b = b
        .child(heading(&h.name, k))
        .child(field("Host UUID", id.to_string(), k))
        .child(field("Management address", &h.address, k))
        .child(field("Scheduling enabled", known(Some(h.enabled)), k));
    let Some(d) = i.details.hosts.get(&id) else {
        return b.child(caption("Host properties are unavailable", k));
    };
    b = b
        .child(field("Description", supplied(Some(&d.description)), k))
        .child(field("Hostname", supplied(Some(&d.hostname)), k))
        .child(field("Maintenance", known(d.maintenance_mode), k))
        .child(field(
            "Authentication type",
            supplied(d.external_auth_type.as_deref()),
            k,
        ))
        .child(field(
            "Authentication service",
            supplied(d.external_auth_service_name.as_deref()),
            k,
        ));
    for (label, epoch) in [
        ("Host boot", &d.boot_time),
        ("Toolstack start", &d.agent_start_time),
    ] {
        b = b.child(field(
            format!("{label} · UTC epoch seconds"),
            supplied(epoch.as_deref()),
            k,
        ));
        let age = epoch
            .as_deref()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .and_then(|v| {
                p.observed_at
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_secs()
                    .checked_sub(v)
            });
        b = b.child(field(
            format!("Elapsed since {label} at capture"),
            age.map(elapsed).unwrap_or_else(|| "Unknown".into()),
            k,
        ));
    }
    b = class(b, i, "host_metrics", k);
    if let Some(m) = d.metrics.and_then(|id| i.details.host_metrics.get(&id)) {
        b = b
            .child(field("Host live", known(m.live), k))
            .child(field(
                "Memory · total / free",
                format!(
                    "{} / {}",
                    storage_bytes(m.memory_total),
                    storage_bytes(m.memory_free)
                ),
                k,
            ))
            .child(field(
                "Metrics updated (host UTC)",
                supplied(m.last_updated.as_deref()),
                k,
            ));
    }
    b = map(b, "Hardware", &d.bios_strings, k);
    b = map(b, "Chipset", &d.chipset_info, k);
    b = map(b, "CPU", &d.cpu_info, k);
    b = map(b, "Software", &d.software_version, k);
    b.child(
        caption(
            "Network link and storage attachment details remain in the host's Details panel.",
            k,
        )
        .wrap(),
    )
}
fn pool_body(p: &Panel, mut b: El, k: &Kit) -> El {
    let i = &p.inventory;
    b=b.child(heading(&i.pool_name,k)).child(field("Pool UUID",i.pool_id.to_string(),k)).child(field("Coordinator",host(i,i.coordinator),k))
        .child(field("Readable inventory",format!("{} hosts · {} guests · {} templates · {} snapshots · {} storage repositories · {} networks",i.hosts.len(),i.guests().count(),i.vms.values().filter(|v|v.is_template).count(),i.snapshots.len(),i.storage.len(),i.networks.len()),k));
    if let Some(d) = &i.details.pool {
        let a = &d.administration;
        b = b
            .child(field("Description", supplied(Some(&d.description)), k))
            .child(field(
                "Default storage",
                if d.default_sr_known {
                    sr(i, d.default_sr)
                } else {
                    "Unknown".into()
                },
                k,
            ))
            .child(field("Autostart", known(d.auto_poweron), k))
            .child(field("IGMP snooping", known(d.igmp_snooping_enabled), k))
            .child(field(
                "Migration compression",
                known(a.migration_compression),
                k,
            ))
            .child(field("HTTPS only", known(a.https_only), k))
            .child(field(
                "Host-to-host TLS verification",
                known(a.tls_verification_enabled),
                k,
            ))
            .child(field(
                "Pool secret rotation pending",
                known(a.secret_rotation_pending),
                k,
            ))
            .child(heading("High availability", k))
            .child(field("HA enabled", known(d.ha_enabled), k))
            .child(field(
                "Failures to tolerate / current plan",
                format!(
                    "{} / {}",
                    number(a.ha_failures_to_tolerate),
                    number(a.ha_plan_exists_for)
                ),
                k,
            ))
            .child(field(
                "Overcommitted / allow overcommit",
                format!(
                    "{} / {}",
                    known(a.ha_overcommitted),
                    known(a.ha_allow_overcommit)
                ),
                k,
            ));
        for id in &a.heartbeat_srs {
            b = b.child(field("Heartbeat storage", sr(i, Some(*id)), k));
        }
        if !a.heartbeat_srs_known {
            b = b.child(field("Heartbeat storage", "Unknown", k));
        }
    }
    for h in i.hosts.values() {
        b = b.child(field(
            "Member",
            format!("{} · {} · {}", h.name, h.id, h.address),
            k,
        ));
    }
    b.child(caption("Counts cover this account's readable inventory. A missing or unavailable object does not establish that it does not exist.",k).wrap())
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.information else {
        return el();
    };
    let mut body=col().gap(12.0).child(caption(format!("Captured pool {} · revision {} · {}",p.inventory.pool_id,p.inventory.revision,if p.synthetic{"Synthetic inventory"}else if p.current{"Current at capture"}else{"Stale at capture"}),k).wrap_anywhere())
        .child(caption("This view is a captured observation. Refresh from client updates it from the same pool; it does not contact the host separately.",k).wrap());
    if let Some(e) = &p.error {
        body = body.child(caption(e, k).wrap());
    }
    body = match p.target {
        Target::Vm(id) => vm_body(p, id, body, k),
        Target::Host(id) => host_body(p, id, body, k),
        Target::Pool => pool_body(p, body, k),
    };
    let card = col()
        .w((s.window_width - 32.0).clamp(280.0, 760.0))
        .h((s.window_height - 48.0).clamp(280.0, 760.0))
        .min_h(0.0)
        .padding(20.0)
        .gap(12.0)
        .radius(10.0)
        .fill(k.c.bg_base)
        .child(heading("General information", k))
        .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
            stick_bottom: false,
        }))
        .child(
            row()
                .gap(6.0)
                .flex_wrap()
                .child(
                    Button::new("Refresh from client")
                        .on_press(refresh)
                        .build(k)
                        .key("information-refresh"),
                )
                .child(
                    Button::new("Pool information")
                        .on_press(|s: &mut State| {
                            if let Some(p) = &mut s.information {
                                p.target = Target::Pool
                            }
                        })
                        .build(k)
                        .key("information-pool"),
                )
                .child(
                    Button::new("Close")
                        .on_press(|s: &mut State| s.information = None)
                        .build(k)
                        .key("information-close"),
                ),
        );
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("General information")
        .modal(false, |s: &mut State| s.information = None)
        .child(card)
        .key("information-dialog")
}

pub(super) fn scene(s: &mut State) {
    demo(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let vm = i
        .guests()
        .find(|v| v.power_state == PowerState::Running)
        .unwrap()
        .id;
    let id = xenadmin_core::VmMetricsId::from(uuid::Uuid::from_u128(0x900));
    i.details.vms.get_mut(&vm).unwrap().metrics = Some(id);
    i.details.vm_metrics.insert(
        id,
        xenadmin_core::VmMetrics {
            id,
            memory_actual: Some(3 << 30),
            vcpus_number: Some(2),
            start_time: Some("2026-11-01T00:00:00Z".into()),
            install_time: Some("2026-01-01T00:00:00Z".into()),
            last_updated: Some("2026-11-08T00:00:00Z".into()),
            state: vec!["running".into()],
            current_domain_type: Some("hvm".into()),
        },
    );
    i.details
        .classes
        .insert("vm_metrics".into(), ClassAvailability::Available);
    s.table.select(vm.to_string());
    open(s);
}
#[cfg(test)]
mod tests {
    use super::*;
    fn click(r: &mut rune::Runtime<State>, key: &str) {
        let (x, y) = r.find(key).unwrap();
        r.pointer_down(x, y);
        r.pointer_up(x, y);
        for _ in 0..30 {
            r.frame(0.016);
        }
    }
    #[test]
    fn information_keeps_its_object_and_pool_and_refreshes_only_the_same_pool() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = init();
            scene(&mut s);
            let target = s.information.as_ref().unwrap().target;
            let pool = s.inventory.as_ref().unwrap().pool_id;
            let other = s
                .inventory
                .as_ref()
                .unwrap()
                .guests()
                .find(|v| Target::Vm(v.id) != target)
                .unwrap()
                .id;
            s.table.select(other.to_string());
            let mut r = rune::Runtime::new(
                s,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                r.frame(0.016);
            }
            assert_eq!(r.state.information.as_ref().unwrap().target, target);
            let a = r.accessibility_json().to_string();
            assert!(a.contains("Current virtual CPUs"));
            assert!(a.contains("Actual memory"));
            assert!(a.contains("Elapsed since boot at capture"));
            r.update(|s, _| {
                Arc::make_mut(s.inventory.as_mut().unwrap()).pool_id =
                    PoolId::from(uuid::Uuid::from_u128(999))
            });
            click(&mut r, "information-refresh");
            assert_eq!(
                r.state.information.as_ref().unwrap().inventory.pool_id,
                pool
            );
            assert!(r.state.information.as_ref().unwrap().error.is_some());
            r.update(|s, _| {
                let i = Arc::make_mut(s.inventory.as_mut().unwrap());
                i.pool_id = pool;
                i.revision = 42;
            });
            click(&mut r, "information-refresh");
            assert_eq!(r.state.information.as_ref().unwrap().inventory.revision, 42);
            click(&mut r, "information-pool");
            assert_eq!(r.state.information.as_ref().unwrap().target, Target::Pool);
            assert!(
                r.accessibility_json()
                    .to_string()
                    .contains("Host-to-host TLS verification")
            );
            click(&mut r, "information-close");
            assert!(r.state.information.is_none());
        }
    }
    #[test]
    fn captured_host_hardware_and_pool_entry_point_are_reachable() {
        let mut s = init();
        demo(&mut s);
        s.page = Page::Hosts;
        s.rebuild();
        let host = *s.inventory.as_ref().unwrap().hosts.keys().next().unwrap();
        Arc::make_mut(s.inventory.as_mut().unwrap())
            .details
            .hosts
            .entry(host)
            .or_default()
            .bios_strings
            .insert("system-manufacturer".into(), "Synthetic Systems".into());
        s.table.select(host.to_string());
        open(&mut s);
        let mut r = rune::Runtime::new(
            s,
            crate::view,
            (513.0, 652.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        r.frame(0.0);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("Synthetic Systems")
        );
        r.update(|s, _| {
            s.information = None;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        });
        for _ in 0..30 {
            r.frame(0.016);
        }
        click(&mut r, "pool-information");
        assert!(matches!(
            r.state.information.as_ref().unwrap().target,
            Target::Pool
        ));
    }
}
