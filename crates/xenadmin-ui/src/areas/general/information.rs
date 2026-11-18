//! The General and Memory tabs of a VM, a template, a server and a pool.
//!
//! This is the body of the Rune "General information" dialog
//! (`inventory_information.rs`) with the fields of the details pane
//! (`details` of `main.rs`) and the scalar fields `resource_details.rs`
//! appended for a server. Labels and value texts are the Rune ones.
//!
//! The Rune dialog showed a captured inventory. The tab is live: it reads
//! the inventory of the object's pool on every build. What is kept of the
//! capture is in [`View`]: whether the inventory is synthetic, current or
//! stale, and the observation time elapsed times are computed against.

use std::collections::BTreeMap;
use std::time::{SystemTime, UNIX_EPOCH};

use xenadmin_core::{ClassAvailability, HostId, Inventory, PoolId, PowerState, VmDetails, VmId};

use crate::prelude::*;

/// What a tab body is built from: the inventory of the object's pool as the
/// client has it now, and how fresh it is.
pub(super) struct View<'a> {
    pub inventory: &'a Inventory,
    /// When the inventory was last updated; now, when there is no connection
    /// to say (a synthetic inventory).
    pub observed_at: SystemTime,
    pub current: bool,
    pub synthetic: bool,
}

impl<'a> View<'a> {
    pub fn of(m: &Model, pool: PoolId, inventory: &'a Inventory) -> Self {
        let snapshot = m.snapshot_for(pool);
        Self {
            inventory,
            observed_at: snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.updated_at())
                .unwrap_or_else(SystemTime::now),
            current: snapshot.as_ref().is_some_and(|snapshot| snapshot.is_current()),
            synthetic: snapshot.is_none()
                && m.inventory
                    .as_ref()
                    .is_some_and(|scope| scope.pool_id == pool)
                && m.demo,
        }
    }

    /// The freshness line the Rune dialog had in its first caption.
    pub fn freshness(&self) -> Property {
        if self.synthetic {
            property("Inventory", "Synthetic inventory")
        } else if self.current {
            property("Inventory", "Current")
        } else {
            property("Inventory", "Stale").tone(Tone::Warning)
        }
        .key("general-freshness")
    }
}

// -- Value texts: unknown is never rendered as No ------------------------------

pub(super) fn known(value: Option<bool>) -> &'static str {
    match value {
        Some(true) => "Yes",
        Some(false) => "No",
        None => "Unknown",
    }
}

fn supplied(value: Option<&str>) -> &str {
    value.filter(|v| !v.is_empty()).unwrap_or("Not reported")
}

fn number(value: Option<u64>) -> String {
    value
        .map(|v| v.to_string())
        .unwrap_or_else(|| "Unknown".into())
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

/// One property per entry, or one that says nothing was reported.
fn map(out: &mut Vec<Property>, label: &str, values: &BTreeMap<String, String>) {
    if values.is_empty() {
        out.push(property(label, "Not reported"));
        return;
    }
    for (name, value) in values {
        out.push(property(format!("{label} · {name}"), value));
    }
}

/// A property that says why an inventory class is not available; nothing
/// when it is.
fn class(out: &mut Vec<Property>, i: &Inventory, name: &str) {
    let message = match i.details.classes.get(&name.to_ascii_lowercase()) {
        Some(ClassAvailability::Available) => return,
        Some(ClassAvailability::PermissionDenied) => "Permission denied",
        Some(ClassAvailability::Unsupported) => "Unsupported by this host",
        Some(ClassAvailability::InvalidData) => "Invalid data received",
        _ => "Availability unknown",
    };
    out.push(property(name, message).tone(Tone::Warning));
}

fn section(key: &str, title: &str, items: Vec<Property>) -> Option<Group> {
    (!items.is_empty()).then(|| group(title, vec![properties(items)]).key(key))
}

// -- VM and template -------------------------------------------------------------

pub(super) fn vm_general(view: &View, id: VmId) -> Vec<Block> {
    let i = view.inventory;
    let Some(vm) = i.vms.get(&id) else {
        return blocks![
            caption("This VM is no longer present in the inventory").key("general-missing")
        ];
    };
    let row = crate::rows::vm_row(i, vm);
    let summary = vec![
        property("Name", &vm.name),
        property("VM UUID", id.to_string()).copyable(),
        property("Description", supplied(Some(&vm.description))),
        property("Power state", vm.power_state.to_string()),
        property("Resident host", host(i, vm.host)),
        property("Virtual CPUs", row.cpu),
        property("Static memory maximum", row.memory),
        view.freshness(),
    ];
    let mut out = blocks![section("general-summary", "General", summary)];
    let Some(d) = i.details.vms.get(&id) else {
        out.push(
            caption("VM properties are unavailable")
                .key("general-unavailable")
                .into(),
        );
        return out;
    };
    out.extend(blocks![section(
        "general-configured",
        "Configured resources and boot",
        vm_properties(d, i)
    )]);
    let mut runtime = Vec::new();
    class(&mut runtime, i, "VM_metrics");
    let mut runtime_notes: Vec<Text> = Vec::new();
    if let Some(metrics) = d.metrics.and_then(|id| i.details.vm_metrics.get(&id)) {
        runtime.extend([
            property("Actual memory", storage_bytes(metrics.memory_actual)),
            property("Current virtual CPUs", number(metrics.vcpus_number)),
            property(
                "Last boot (host UTC)",
                supplied(metrics.start_time.as_deref()),
            ),
            property(
                "Installed (host UTC)",
                supplied(metrics.install_time.as_deref()),
            ),
            property(
                "Metrics updated (host UTC)",
                supplied(metrics.last_updated.as_deref()),
            ),
            property(
                "Runtime domain type",
                supplied(metrics.current_domain_type.as_deref()),
            ),
            property(
                "Runtime flags",
                supplied(Some(&metrics.state.join(", "))),
            ),
        ]);
        if matches!(vm.power_state, PowerState::Running | PowerState::Paused) {
            runtime.push(property(
                "Elapsed since boot at capture",
                metrics
                    .elapsed_since_boot(view.observed_at)
                    .map(elapsed)
                    .unwrap_or_else(|| {
                        "Unknown; boot time is missing, invalid or ahead of this computer's clock"
                            .into()
                    }),
            ));
            runtime_notes.push(caption(
                "Elapsed time uses this computer's clock and includes paused time. It is not \
                 guest-measured uptime.",
            ));
        }
    } else {
        runtime_notes.push(caption("No VM runtime metrics record is available"));
    }
    let mut runtime_blocks = Vec::new();
    if !runtime.is_empty() {
        runtime_blocks.push(properties(runtime));
    }
    runtime_blocks.extend(runtime_notes.into_iter().map(Block::from));
    out.push(
        group("Reported runtime", runtime_blocks)
            .key("general-runtime")
            .into(),
    );
    let mut guest = Vec::new();
    class(&mut guest, i, "VM_guest_metrics");
    let mut guest_notes: Vec<Text> = Vec::new();
    if let Some(metrics) = d
        .guest_metrics
        .and_then(|id| i.details.guest_metrics.get(&id))
    {
        guest.extend([
            property("Guest agent live", known(metrics.live)),
            property(
                "Guest data updated (host UTC)",
                supplied(metrics.last_updated.as_deref()),
            ),
        ]);
        if metrics.live != Some(true) {
            guest_notes.push(caption(
                "The guest agent is not reporting live data. Its addresses and OS details may \
                 be stale.",
            ));
        }
        map(&mut guest, "Operating system", &metrics.os_version);
        map(&mut guest, "Tools", &metrics.tools_version);
        map(&mut guest, "Address", &metrics.networks);
    }
    if !guest.is_empty() {
        let mut blocks = vec![properties(guest)];
        blocks.extend(guest_notes.into_iter().map(Block::from));
        out.push(
            group("Guest-reported information", blocks)
                .key("general-guest")
                .into(),
        );
    }
    out
}

fn vm_properties(d: &VmDetails, i: &Inventory) -> Vec<Property> {
    vec![
        property(
            "vCPUs · startup / maximum",
            format!("{} / {}", number(d.vcpus_at_startup), number(d.vcpus_max)),
        ),
        property(
            "Static memory · minimum / maximum",
            format!(
                "{} / {}",
                storage_bytes(d.memory_static_min),
                storage_bytes(d.memory_static_max)
            ),
        ),
        property(
            "Dynamic memory · minimum / maximum",
            format!(
                "{} / {}",
                storage_bytes(d.memory_dynamic_min),
                storage_bytes(d.memory_dynamic_max)
            ),
        ),
        property(
            "Home host",
            if d.affinity_known {
                host(i, d.affinity)
            } else {
                "Unknown".into()
            },
        ),
        property(
            "Firmware / Secure Boot",
            format!(
                "{} / {}",
                supplied(d.firmware.as_deref()),
                known(d.secure_boot)
            ),
        ),
        property(
            "Boot policy / order",
            format!(
                "{} / {}",
                supplied(d.boot_policy.as_deref()),
                supplied(d.boot_order.as_deref())
            ),
        ),
        property("Autostart", known(d.auto_power_on)),
        property(
            "HA restart priority",
            supplied(d.ha_restart_priority.as_deref()),
        ),
        property("Tags", supplied(Some(&d.tags.join(", ")))),
    ]
}

pub(super) fn vm_memory(view: &View, id: VmId) -> Vec<Block> {
    let i = view.inventory;
    let Some(vm) = i.vms.get(&id) else {
        return blocks![
            caption("This VM is no longer present in the inventory").key("general-missing")
        ];
    };
    let mut items = vec![property(
        "Static memory maximum",
        memory_label(vm.memory_bytes),
    )];
    let mut notes: Vec<Text> = Vec::new();
    match i.details.vms.get(&id) {
        Some(d) => {
            items.extend([
                property(
                    "Static memory · minimum / maximum",
                    format!(
                        "{} / {}",
                        storage_bytes(d.memory_static_min),
                        storage_bytes(d.memory_static_max)
                    ),
                ),
                property(
                    "Dynamic memory · minimum / maximum",
                    format!(
                        "{} / {}",
                        storage_bytes(d.memory_dynamic_min),
                        storage_bytes(d.memory_dynamic_max)
                    ),
                ),
            ]);
            class(&mut items, i, "VM_metrics");
            match d.metrics.and_then(|id| i.details.vm_metrics.get(&id)) {
                Some(metrics) => items.extend([
                    property("Actual memory", storage_bytes(metrics.memory_actual)),
                    property(
                        "Metrics updated (host UTC)",
                        supplied(metrics.last_updated.as_deref()),
                    ),
                ]),
                None => notes.push(caption("No VM runtime metrics record is available")),
            }
        }
        None => notes.push(caption("VM properties are unavailable")),
    }
    items.push(view.freshness());
    blocks![section("memory-vm", "Memory", items), notes]
}

// -- Server --------------------------------------------------------------------

pub(super) fn host_general(view: &View, id: HostId) -> Vec<Block> {
    let i = view.inventory;
    let Some(h) = i.hosts.get(&id) else {
        return blocks![
            caption("This host is no longer present in the inventory").key("general-missing")
        ];
    };
    let mut summary = vec![
        property("Name", &h.name),
        property("Host UUID", id.to_string()).copyable(),
        property("Management address", &h.address).copyable(),
        property("Scheduling enabled", known(Some(h.enabled))),
        property(
            "Pool coordinator",
            known(i.coordinator.map(|coordinator| coordinator == id)),
        ),
        property("Version", format!("XCP-ng {}", h.version)),
    ];
    let Some(d) = i.details.hosts.get(&id) else {
        summary.push(view.freshness());
        return blocks![
            section("general-summary", "General", summary),
            caption("Host properties are unavailable").key("general-unavailable"),
        ];
    };
    summary.extend([
        property("Description", supplied(Some(&d.description))),
        property("Hostname", supplied(Some(&d.hostname))),
        property("Maintenance", known(d.maintenance_mode)),
        property("Multipathing", known(d.multipathing)),
        property(
            "Remote logging",
            if d.syslog_known {
                d.syslog_destination.as_deref().unwrap_or("Local only")
            } else {
                "Unknown"
            },
        ),
        property(
            "Power-on mode",
            d.power_on_mode
                .as_deref()
                .map(|v| if v.is_empty() { "Disabled" } else { v })
                .unwrap_or("Unknown"),
        ),
        property(
            "Capabilities",
            supplied_or(&d.capabilities.join(", "), "Not supplied"),
        ),
        property("Tags", supplied_or(&d.tags.join(", "), "Not supplied")),
        property(
            "Authentication type",
            supplied(d.external_auth_type.as_deref()),
        ),
        property(
            "Authentication service",
            supplied(d.external_auth_service_name.as_deref()),
        ),
    ]);
    summary.push(view.freshness());
    let mut times = Vec::new();
    for (label, epoch) in [
        ("Host boot", &d.boot_time),
        ("Toolstack start", &d.agent_start_time),
    ] {
        times.push(property(
            format!("{label} · UTC epoch seconds"),
            supplied(epoch.as_deref()),
        ));
        let age = epoch
            .as_deref()
            .and_then(|v| v.parse::<u64>().ok())
            .filter(|v| *v > 0)
            .and_then(|v| {
                view.observed_at
                    .duration_since(UNIX_EPOCH)
                    .ok()?
                    .as_secs()
                    .checked_sub(v)
            });
        times.push(property(
            format!("Elapsed since {label} at capture"),
            age.map(elapsed).unwrap_or_else(|| "Unknown".into()),
        ));
    }
    let mut metrics = Vec::new();
    class(&mut metrics, i, "host_metrics");
    if let Some(m) = d.metrics.and_then(|id| i.details.host_metrics.get(&id)) {
        metrics.extend([
            property("Host live", known(m.live)),
            property(
                "Memory · total / free",
                format!(
                    "{} / {}",
                    storage_bytes(m.memory_total),
                    storage_bytes(m.memory_free)
                ),
            ),
            property(
                "Metrics updated (host UTC)",
                supplied(m.last_updated.as_deref()),
            ),
        ]);
    }
    let mut hardware = Vec::new();
    map(&mut hardware, "Hardware", &d.bios_strings);
    map(&mut hardware, "Chipset", &d.chipset_info);
    map(&mut hardware, "CPU", &d.cpu_info);
    let mut software = Vec::new();
    map(&mut software, "Software", &d.software_version);
    blocks![
        section("general-summary", "General", summary),
        section("general-times", "Start times", times),
        section("general-metrics", "Reported state", metrics),
        section("general-hardware", "Hardware", hardware),
        section("general-software", "Software", software),
        caption(
            "Network link details are on the NICs tab and storage attachments on the Storage \
             tab."
        ),
    ]
}

fn supplied_or<'a>(value: &'a str, empty: &'a str) -> &'a str {
    if value.is_empty() { empty } else { value }
}

pub(super) fn host_memory(view: &View, id: HostId) -> Vec<Block> {
    let i = view.inventory;
    let Some(h) = i.hosts.get(&id) else {
        return blocks![
            caption("This host is no longer present in the inventory").key("general-missing")
        ];
    };
    let mut items = vec![property("Server", &h.name)];
    host_memory_properties(i, id, &mut items);
    items.push(view.freshness());
    let guests: Vec<Row> = by_name(i.guests().filter(|vm| vm.host == Some(id)).collect())
        .into_iter()
        .map(|vm| memory_row(i, vm))
        .collect();
    blocks![
        section("memory-host", "Memory", items),
        group(
            "VMs on this server",
            vec![
                table("memory-vms", memory_columns(), guests)
                    .empty("No VM is running on this server.")
                    .into()
            ]
        )
        .key("memory-vms-group"),
    ]
}

fn host_memory_properties(i: &Inventory, id: HostId, items: &mut Vec<Property>) {
    class(items, i, "host_metrics");
    let metrics = i
        .details
        .hosts
        .get(&id)
        .and_then(|d| d.metrics)
        .and_then(|id| i.details.host_metrics.get(&id));
    match metrics {
        Some(m) => items.extend([
            property(
                "Memory · total / free",
                format!(
                    "{} / {}",
                    storage_bytes(m.memory_total),
                    storage_bytes(m.memory_free)
                ),
            ),
            property("Host live", known(m.live)),
            property(
                "Metrics updated (host UTC)",
                supplied(m.last_updated.as_deref()),
            ),
        ]),
        None => items.push(property("Memory · total / free", "Unknown / Unknown")),
    }
}

fn by_name(mut vms: Vec<&xenadmin_core::Vm>) -> Vec<&xenadmin_core::Vm> {
    vms.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)));
    vms
}

fn memory_columns() -> Vec<Column> {
    vec![
        column("name", "VM"),
        column("state", "Power state"),
        column("static", "Static maximum").right(),
        column("dynamic", "Dynamic minimum / maximum").right(),
        column("actual", "Actual memory").right(),
    ]
}

fn memory_row(i: &Inventory, vm: &xenadmin_core::Vm) -> Row {
    let d = i.details.vms.get(&vm.id);
    let dynamic = d
        .map(|d| {
            format!(
                "{} / {}",
                storage_bytes(d.memory_dynamic_min),
                storage_bytes(d.memory_dynamic_max)
            )
        })
        .unwrap_or_else(|| "Unknown".into());
    let actual = d
        .and_then(|d| d.metrics)
        .and_then(|id| i.details.vm_metrics.get(&id))
        .map(|metrics| storage_bytes(metrics.memory_actual))
        .unwrap_or_else(|| "Unknown".into());
    row(
        vm.id,
        [
            vm.name.clone(),
            vm.power_state.to_string(),
            memory_label(vm.memory_bytes),
            dynamic,
            actual,
        ],
    )
}

// -- Pool ----------------------------------------------------------------------

pub(super) fn pool_general(view: &View) -> Vec<Block> {
    let i = view.inventory;
    let mut summary = vec![
        property("Name", &i.pool_name),
        property("Pool UUID", i.pool_id.to_string()).copyable(),
        property("Coordinator", host(i, i.coordinator)),
        property(
            "Readable inventory",
            format!(
                "{} hosts · {} guests · {} templates · {} snapshots · {} storage repositories · \
                 {} networks",
                i.hosts.len(),
                i.guests().count(),
                i.vms.values().filter(|v| v.is_template).count(),
                i.snapshots.len(),
                i.storage.len(),
                i.networks.len()
            ),
        ),
    ];
    let mut availability = Vec::new();
    if let Some(d) = &i.details.pool {
        let a = &d.administration;
        summary.extend([
            property("Description", supplied(Some(&d.description))),
            property(
                "Default storage",
                if d.default_sr_known {
                    sr(i, d.default_sr)
                } else {
                    "Unknown".into()
                },
            ),
            property("Autostart", known(d.auto_poweron)),
            property("IGMP snooping", known(d.igmp_snooping_enabled)),
            property("Migration compression", known(a.migration_compression)),
            property("HTTPS only", known(a.https_only)),
            property(
                "Host-to-host TLS verification",
                known(a.tls_verification_enabled),
            ),
            property(
                "Pool secret rotation pending",
                known(a.secret_rotation_pending),
            ),
        ]);
        availability.extend([
            property("HA enabled", known(d.ha_enabled)),
            property(
                "Failures to tolerate / current plan",
                format!(
                    "{} / {}",
                    number(a.ha_failures_to_tolerate),
                    number(a.ha_plan_exists_for)
                ),
            ),
            property(
                "Overcommitted / allow overcommit",
                format!(
                    "{} / {}",
                    known(a.ha_overcommitted),
                    known(a.ha_allow_overcommit)
                ),
            ),
        ]);
        for id in &a.heartbeat_srs {
            availability.push(property("Heartbeat storage", sr(i, Some(*id))));
        }
        if !a.heartbeat_srs_known {
            availability.push(property("Heartbeat storage", "Unknown"));
        }
    }
    summary.push(view.freshness());
    let members: Vec<Property> = i
        .hosts
        .values()
        .map(|h| property("Member", format!("{} · {} · {}", h.name, h.id, h.address)))
        .collect();
    blocks![
        section("general-summary", "General", summary),
        section("general-ha", "High availability", availability),
        section("general-members", "Members", members),
        caption(
            "Counts cover this account's readable inventory. A missing or unavailable object \
             does not establish that it does not exist."
        ),
    ]
}

pub(super) fn pool_memory(view: &View) -> Vec<Block> {
    let i = view.inventory;
    let mut hosts: Vec<_> = i.hosts.values().collect();
    hosts.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.id.cmp(&b.id)));
    let rows: Vec<Row> = hosts
        .into_iter()
        .map(|h| {
            let metrics = i
                .details
                .hosts
                .get(&h.id)
                .and_then(|d| d.metrics)
                .and_then(|id| i.details.host_metrics.get(&id));
            row(
                h.id,
                [
                    h.name.clone(),
                    metrics
                        .map(|m| storage_bytes(m.memory_total))
                        .unwrap_or_else(|| "Unknown".into()),
                    metrics
                        .map(|m| storage_bytes(m.memory_free))
                        .unwrap_or_else(|| "Unknown".into()),
                    known(metrics.and_then(|m| m.live)).to_owned(),
                    metrics
                        .and_then(|m| m.last_updated.clone())
                        .unwrap_or_else(|| "Not reported".into()),
                ],
            )
        })
        .collect();
    let mut notes = Vec::new();
    class(&mut notes, i, "host_metrics");
    notes.push(view.freshness());
    blocks![
        group(
            "Server memory",
            vec![
                table(
                    "memory-hosts",
                    [
                        column("name", "Server"),
                        column("total", "Total").right(),
                        column("free", "Free").right(),
                        column("live", "Host live"),
                        column("updated", "Metrics updated (host UTC)"),
                    ],
                    rows
                )
                .empty("This pool has no server in the readable inventory.")
                .into()
            ]
        )
        .key("memory-hosts-group"),
        properties(notes),
    ]
}
