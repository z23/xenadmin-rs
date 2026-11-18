//! Tests of the General, Memory, Storage, Networking and NICs tab bodies.
//!
//! The Rune tests of the modules:
//! - `inventory_information.rs`
//!   `information_keeps_its_object_and_pool_and_refreshes_only_the_same_pool`:
//!   ported as `the_general_tab_of_a_vm_and_of_the_pool_list_the_rune_fields`
//!   (the field lists) and `the_general_tab_is_live` (what replaces the
//!   capture). Retired with the dialog: the panel keeping its target while
//!   the table selection moves, "Refresh from client" refusing another pool
//!   and picking up a new revision of the same one, the "Pool information"
//!   button and Close. A tab follows the selection and is always current.
//! - `inventory_information.rs`
//!   `captured_host_hardware_and_pool_entry_point_are_reachable`: ported as
//!   `the_general_tab_of_a_server_lists_its_hardware`; the `pool-information`
//!   entry point is retired, the pool's General tab replaces it.
//! - `resource_details.rs`
//!   `resource_details_show_relationships_and_unknown_attachment_state`:
//!   ported as `resource_tabs_show_relationships_and_unknown_attachment_state`.

use std::sync::Arc;

use xenadmin_core::{ClassAvailability, Inventory, PbdId, PoolId, SrId, StorageAttachment};

use super::*;
use crate::Harness;

fn inventory(h: &Harness) -> Inventory {
    h.model.inventory.as_deref().cloned().expect("inventory")
}

fn pool(h: &Harness) -> PoolId {
    h.model.inventory.as_ref().expect("inventory").pool_id
}

/// The value of the first property with this label in what is shown.
fn value(h: &Harness, label: &str) -> Option<String> {
    h.nodes()
        .into_iter()
        .find(|node| node.role == "property" && node.label.as_deref() == Some(label))
        .and_then(|node| node.value)
}

/// The cells of one table row, as (column title, text).
fn cells(h: &Harness, table: &str, row: &str) -> Vec<(String, String)> {
    let key = format!("{table}:{row}");
    h.nodes()
        .into_iter()
        .skip_while(|node| node.key.as_deref() != Some(key.as_str()))
        .skip(1)
        .take_while(|node| node.role == "cell")
        .map(|node| {
            (
                node.label.unwrap_or_default(),
                node.text.unwrap_or_default(),
            )
        })
        .collect()
}

fn cell(h: &Harness, table: &str, row: &str, column: &str) -> String {
    cells(h, table, row)
        .into_iter()
        .find(|(title, _)| title == column)
        .map(|(_, text)| text)
        .unwrap_or_else(|| panic!("no cell {column} in row {row} of {table}"))
}

#[test]
fn the_general_tab_of_a_vm_and_of_the_pool_list_the_rune_fields() {
    let mut h = Harness::scene("general-information");
    assert!(h.surfaces().is_empty(), "information is a tab, not a dialog");
    assert_eq!(h.model.active_tab(), TabId::General);
    h.assert_unique_keys();
    for label in [
        "VM UUID",
        "Description",
        "Power state",
        "Resident host",
        "vCPUs · startup / maximum",
        "Static memory · minimum / maximum",
        "Dynamic memory · minimum / maximum",
        "Home host",
        "Firmware / Secure Boot",
        "Boot policy / order",
        "Autostart",
        "HA restart priority",
        "Tags",
        "Actual memory",
        "Current virtual CPUs",
        "Last boot (host UTC)",
        "Installed (host UTC)",
        "Metrics updated (host UTC)",
        "Runtime domain type",
        "Runtime flags",
        "Elapsed since boot at capture",
    ] {
        assert!(value(&h, label).is_some(), "{label} is missing");
    }
    assert_eq!(value(&h, "Actual memory").as_deref(), Some("3.0 GiB"));
    assert_eq!(value(&h, "Current virtual CPUs").as_deref(), Some("2"));
    assert_eq!(value(&h, "Runtime flags").as_deref(), Some("running"));
    // The boot time of the scene is in the past of any clock that runs this.
    let elapsed = value(&h, "Elapsed since boot at capture").expect("elapsed");
    assert!(elapsed.ends_with('m') && elapsed.contains("d "), "{elapsed}");
    assert!(h.has_text("It is not guest-measured uptime."));
    assert_eq!(
        h.node("general-freshness").value.as_deref(),
        Some("Synthetic inventory")
    );

    // The pool: the Rune "Pool information" is the pool's own General tab.
    h.select(Target::one(ObjectRef::pool(pool(&h))));
    h.open_tab(TabId::General);
    h.assert_unique_keys();
    for label in [
        "Pool UUID",
        "Coordinator",
        "Readable inventory",
        "Host-to-host TLS verification",
        "HA enabled",
        "Failures to tolerate / current plan",
        "Overcommitted / allow overcommit",
        "Member",
    ] {
        assert!(value(&h, label).is_some(), "{label} is missing");
    }
    assert_eq!(
        value(&h, "Readable inventory").as_deref(),
        Some(
            "3 hosts · 48 guests · 0 templates · 3 snapshots · 1 storage repositories · 1 networks"
        )
    );
    assert!(h.has_text("A missing or unavailable object does not establish"));
}

#[test]
fn the_general_tab_is_live() {
    // What replaces the Rune capture and "Refresh from client": the tab
    // shows the inventory the client has now, for the object selected now.
    let mut h = Harness::scene("general-information");
    let first = h.model.scope().single().cloned().expect("a VM");
    let mut changed = inventory(&h);
    let vm = first.vm_id().expect("vm id");
    changed.vms.get_mut(&vm).expect("vm").description = "Changed on the host".into();
    changed.revision = 42;
    h.model.inventory = Some(Arc::new(changed));
    assert_eq!(
        value(&h, "Description").as_deref(),
        Some("Changed on the host")
    );
    // Another object: the tab follows the selection.
    let other = inventory(&h)
        .guests()
        .find(|v| v.id != vm)
        .map(|v| v.id)
        .expect("another VM");
    h.select(Target::one(ObjectRef::vm(pool(&h), other)));
    assert_eq!(
        value(&h, "VM UUID").as_deref(),
        Some(other.to_string().as_str())
    );
    // A VM that left the inventory says so instead of showing another one.
    let mut gone = inventory(&h);
    gone.vms.remove(&other);
    h.model.inventory = Some(Arc::new(gone));
    assert_eq!(
        h.node("general-missing").text.as_deref(),
        Some("This VM is no longer present in the inventory")
    );
}

#[test]
fn a_stale_inventory_is_named_and_unknown_is_never_no() {
    let mut h = Harness::scene("general-information");
    // Not synthetic and no connection: the inventory is stale.
    h.model.demo = false;
    assert_eq!(h.node("general-freshness").value.as_deref(), Some("Stale"));
    // Unknown booleans.
    let vm = h.model.scope().single().and_then(ObjectRef::vm_id).unwrap();
    let mut changed = inventory(&h);
    let details = changed.details.vms.get_mut(&vm).expect("details");
    details.secure_boot = None;
    details.firmware = None;
    details.auto_power_on = None;
    details.affinity_known = false;
    changed.details.classes.remove("vm_metrics");
    h.model.inventory = Some(Arc::new(changed));
    assert_eq!(
        value(&h, "Firmware / Secure Boot").as_deref(),
        Some("Not reported / Unknown")
    );
    assert_eq!(value(&h, "Autostart").as_deref(), Some("Unknown"));
    assert_eq!(value(&h, "Home host").as_deref(), Some("Unknown"));
    assert_eq!(
        value(&h, "VM_metrics").as_deref(),
        Some("Availability unknown")
    );
    // No detail record at all.
    let mut bare = inventory(&h);
    bare.details.vms.remove(&vm);
    h.model.inventory = Some(Arc::new(bare));
    assert_eq!(
        h.node("general-unavailable").text.as_deref(),
        Some("VM properties are unavailable")
    );
}

#[test]
fn the_general_tab_of_a_server_lists_its_hardware() {
    // Rune: captured_host_hardware_and_pool_entry_point_are_reachable
    let mut h = Harness::scene("demo");
    let mut changed = inventory(&h);
    let host = *changed.hosts.keys().next().expect("a host");
    changed
        .details
        .hosts
        .entry(host)
        .or_default()
        .bios_strings
        .insert("system-manufacturer".into(), "Synthetic Systems".into());
    h.model.inventory = Some(Arc::new(changed));
    h.select(Target::one(ObjectRef::host(pool(&h), host)));
    h.open_tab(TabId::General);
    h.assert_unique_keys();
    assert_eq!(
        value(&h, "Hardware · system-manufacturer").as_deref(),
        Some("Synthetic Systems")
    );
    for label in [
        "Host UUID",
        "Management address",
        "Scheduling enabled",
        "Pool coordinator",
        "Hostname",
        "Maintenance",
        "Multipathing",
        "Remote logging",
        "Power-on mode",
        "Capabilities",
        "Authentication type",
        "Host boot · UTC epoch seconds",
        "Elapsed since Host boot at capture",
        "Toolstack start · UTC epoch seconds",
        "Elapsed since Toolstack start at capture",
    ] {
        assert!(value(&h, label).is_some(), "{label} is missing");
    }
    assert!(h.has_text("NICs tab"));
    // The pool is selectable and has a General tab of its own.
    h.select(Target::one(ObjectRef::pool(pool(&h))));
    h.open_tab(TabId::General);
    assert!(value(&h, "Pool UUID").is_some());
}

#[test]
fn resource_tabs_show_relationships_and_unknown_attachment_state() {
    // Rune: resource_details_show_relationships_and_unknown_attachment_state
    let mut h = Harness::scene("demo");
    let mut changed = inventory(&h);
    let attachment = PbdId::from(uuid::Uuid::from_u128(77));
    let host = *changed.hosts.keys().next().expect("a host");
    let repository = changed.storage[0].uuid;
    let network = changed.networks[0].uuid;
    changed.details.pbds.insert(
        attachment,
        StorageAttachment {
            location: None,
            requires_credentials: false,
            multipathed: None,
            paths: Vec::new(),
            id: attachment,
            host: Some(host),
            sr: Some(SrId::from(repository)),
            currently_attached: None,
        },
    );
    changed
        .details
        .classes
        .insert("pbd".into(), ClassAvailability::Available);
    h.model.inventory = Some(Arc::new(changed));
    let pool = pool(&h);
    let no_power = |h: &Harness| {
        assert!(
            h.keys().iter().all(|key| !key.starts_with("power-")),
            "resource tabs do not offer VM power controls"
        );
    };

    // Server: General has the scalar fields, Storage the attachments.
    h.select(Target::one(ObjectRef::host(pool, host)));
    h.open_tab(TabId::General);
    assert!(value(&h, "Management address").is_some());
    no_power(&h);
    h.open_tab(TabId::Storage);
    h.assert_unique_keys();
    assert_eq!(
        cell(&h, "storage-pbds", &attachment.to_string(), "Attached"),
        "Unknown",
        "an unknown attachment state is Unknown, never No"
    );
    assert_eq!(
        cell(
            &h,
            "storage-pbds",
            &attachment.to_string(),
            "Storage repository"
        ),
        "Demo storage"
    );
    no_power(&h);
    h.open_tab(TabId::Nics);
    h.assert_unique_keys();
    assert!(h.find("nics-interfaces").is_some());
    h.open_tab(TabId::Networking);
    h.assert_unique_keys();
    assert!(h.find("networking-uplinks").is_some());

    // Storage repository: its disks and the same attachment from its side.
    h.select(Target::one(ObjectRef::storage(pool, repository)));
    h.open_tab(TabId::General);
    assert!(value(&h, "Capacity").is_some());
    h.open_tab(TabId::Storage);
    h.assert_unique_keys();
    assert!(h.has_text("Demo system disk"));
    assert_eq!(
        cell(&h, "storage-pbds", &attachment.to_string(), "Attached"),
        "Unknown"
    );
    assert_eq!(
        cell(&h, "storage-pbds", &attachment.to_string(), "Server"),
        "xcp-lab-01"
    );
    no_power(&h);

    // Network: uplinks and VM interfaces.
    h.select(Target::one(ObjectRef::network(pool, network)));
    h.open_tab(TabId::General);
    assert!(value(&h, "Bridge").is_some());
    h.open_tab(TabId::Networking);
    h.assert_unique_keys();
    assert!(h.has_text("VM interfaces"));
    assert!(h.has_text("Host uplinks"));
    no_power(&h);
}

#[test]
fn an_unreadable_inventory_class_is_named_above_its_list() {
    let mut h = Harness::scene("demo");
    let mut changed = inventory(&h);
    let host = *changed.hosts.keys().next().expect("a host");
    changed.details.classes.remove("pbd");
    changed
        .details
        .classes
        .insert("pif".into(), ClassAvailability::PermissionDenied);
    h.model.inventory = Some(Arc::new(changed));
    h.select(Target::one(ObjectRef::host(pool(&h), host)));
    h.open_tab(TabId::Storage);
    assert_eq!(
        h.node("class-notice-pbd").text.as_deref(),
        Some("PBD: Availability of this inventory class is unknown")
    );
    h.open_tab(TabId::Nics);
    assert_eq!(
        h.node("class-notice-pif").text.as_deref(),
        Some("PIF: Your account cannot read this inventory class")
    );
}

#[test]
fn memory_tabs_show_what_the_inventory_reports_and_say_unknown_otherwise() {
    let mut h = Harness::scene("general-information");
    let vm = h.model.scope().single().and_then(ObjectRef::vm_id).unwrap();
    let pool = pool(&h);
    h.open_tab(TabId::Memory);
    h.assert_unique_keys();
    assert_eq!(value(&h, "Actual memory").as_deref(), Some("3.0 GiB"));
    assert!(value(&h, "Dynamic memory · minimum / maximum").is_some());
    assert!(value(&h, "Static memory · minimum / maximum").is_some());

    // The server that VM is on lists it with its memory.
    let host = inventory(&h).vms[&vm].host.expect("resident host");
    h.select(Target::one(ObjectRef::host(pool, host)));
    h.open_tab(TabId::Memory);
    h.assert_unique_keys();
    assert_eq!(
        cell(&h, "memory-vms", &vm.to_string(), "Actual memory"),
        "3.0 GiB"
    );
    assert!(value(&h, "Memory · total / free").is_some());
    // A guest without a metrics record is Unknown, not zero.
    let other = inventory(&h)
        .guests()
        .find(|v| v.host == Some(host) && v.id != vm)
        .map(|v| v.id)
        .expect("another guest on the host");
    let mut changed = inventory(&h);
    if let Some(details) = changed.details.vms.get_mut(&other) {
        details.metrics = None;
    }
    h.model.inventory = Some(Arc::new(changed));
    assert_eq!(
        cell(&h, "memory-vms", &other.to_string(), "Actual memory"),
        "Unknown"
    );

    // The pool lists every server.
    h.select(Target::one(ObjectRef::pool(pool)));
    h.open_tab(TabId::Memory);
    h.assert_unique_keys();
    let mut without = inventory(&h);
    let hosts: Vec<_> = without.hosts.keys().copied().collect();
    assert_eq!(hosts.len(), 3);
    for id in &hosts {
        assert!(!cells(&h, "memory-hosts", &id.to_string()).is_empty());
    }
    if let Some(details) = without.details.hosts.get_mut(&hosts[0]) {
        details.metrics = None;
    }
    h.model.inventory = Some(Arc::new(without));
    assert_eq!(
        cell(&h, "memory-hosts", &hosts[0].to_string(), "Total"),
        "Unknown"
    );
    assert_eq!(
        cell(&h, "memory-hosts", &hosts[0].to_string(), "Host live"),
        "Unknown"
    );
}

#[test]
fn every_tab_the_area_provides_has_a_body_with_unique_keys() {
    let mut h = Harness::scene("demo");
    let i = inventory(&h);
    let pool = i.pool_id;
    let guest = i.guests().next().expect("guest").id;
    let disk = *i.details.vdis.keys().next().expect("a disk");
    let objects = [
        ObjectRef::pool(pool),
        ObjectRef::host(pool, *i.hosts.keys().next().unwrap()),
        ObjectRef::vm(pool, guest),
        ObjectRef::storage(pool, i.storage[0].uuid),
        ObjectRef::network(pool, i.networks[0].uuid),
        ObjectRef::new(Some(pool), ObjectKind::VirtualDisk, disk),
    ];
    for object in objects {
        h.select(Target::one(object.clone()));
        for tab in h.model.tabs().to_vec() {
            if !provides(tab, object.kind) {
                continue;
            }
            h.open_tab(tab);
            let body = h.model.tab_body(tab);
            assert!(body.is_some(), "{:?} of {:?} has no body", tab, object.kind);
            h.assert_unique_keys();
            assert!(
                h.find("general-missing").is_none(),
                "{:?} of {:?} did not find its object",
                tab,
                object.kind
            );
        }
    }
    // The virtual disk names its repository and its attachments.
    h.select(Target::one(ObjectRef::new(
        Some(pool),
        ObjectKind::VirtualDisk,
        disk,
    )));
    h.open_tab(TabId::General);
    assert_eq!(
        value(&h, "Storage repository").as_deref(),
        Some("Demo storage")
    );
    // A VM's Storage and Networking list its devices.
    let with_disk = i
        .details
        .vbds
        .values()
        .find_map(|vbd| vbd.vm)
        .expect("a VM with a disk");
    h.select(Target::one(ObjectRef::vm(pool, with_disk)));
    h.open_tab(TabId::Storage);
    assert!(h.find("storage-vbds-empty").is_none());
}

#[test]
fn a_saved_connection_that_shows_no_pool_has_a_general_tab() {
    let mut h = Harness::scene("demo");
    let copy = inventory(&h);
    let alias = h
        .model
        .add_synthetic_pool("Alias of the demonstration pool", copy);
    // The pool is shown through the scope session; the alias is a
    // connection node.
    let node = ObjectRef::new(None, ObjectKind::Connection, alias);
    let body = tab(&h.model, TabId::General, &Target::one(node.clone())).expect("a body");
    let nodes = crate::surface::flatten_blocks("tab", &body.blocks());
    let value = |label: &str| {
        nodes
            .iter()
            .find(|node| node.label.as_deref() == Some(label))
            .and_then(|node| node.value.clone())
    };
    assert_eq!(
        value("Name").as_deref(),
        Some("Alias of the demonstration pool")
    );
    assert_eq!(value("Status").as_deref(), Some("Demo"));
    assert!(tab(&h.model, TabId::Memory, &Target::one(node)).is_none());
}

// Rune: the "Operations" section of the details pane (`details` of
// `main.rs`), which had no test of its own: the records, and under each
// what Rune showed there (the controls are the power and events areas').
#[test]
fn the_general_tab_of_a_vm_lists_its_recent_operations_with_their_rune_controls() {
    use xenadmin_core::{OperationRecord, OperationState};
    let mut h = Harness::scene("demo");
    let pool = pool(&h);
    let vm = xenadmin_core::VmId::from(
        uuid::Uuid::parse_str("30000000-0000-0000-0000-000000000001").unwrap(),
    );
    let record = |index: u128, pool: String, vm: &str, state: OperationState| -> OperationRecord {
        serde_json::from_value(serde_json::json!({
            "id": uuid::Uuid::from_u128(0x8100_0000_0000_0000_0000_0000_0000_0000 + index),
            "pool": pool,
            "vm": vm,
            "action": "Start",
            "state": state,
            "progress": null,
            "detail": format!("Synthetic operation {index}"),
            "created_at_ms": index as u64,
        }))
        .unwrap()
    };
    let this = "30000000-0000-0000-0000-000000000001";
    let mut rows: Vec<OperationRecord> = (1..=12)
        .map(|index| record(index, pool.to_string(), this, OperationState::Succeeded))
        .collect();
    rows.push(record(20, pool.to_string(), this, OperationState::OutcomeUnknown));
    // An operation in progress that can be asked to cancel.
    let mut running = serde_json::to_value(record(
        19,
        pool.to_string(),
        this,
        OperationState::Tracking,
    ))
    .unwrap();
    running["can_cancel"] = true.into();
    rows.push(serde_json::from_value(running).unwrap());
    rows.push(record(
        21,
        pool.to_string(),
        "30000000-0000-0000-0000-000000000002",
        OperationState::Failed,
    ));
    rows.push(record(
        22,
        "10000000-0000-0000-0000-000000000063".into(),
        this,
        OperationState::Failed,
    ));
    h.model.operation_rows = rows;
    h.select(Target::one(ObjectRef::vm(pool, vm)));
    h.open_tab(TabId::General);
    let shown: Vec<_> = h
        .nodes()
        .into_iter()
        .filter(|node| {
            node.key
                .as_deref()
                .is_some_and(|key| key.starts_with("vm-operation-"))
        })
        .collect();
    // Ten at most; the unresolved one first, then the newest; never the
    // record of another VM or of the same id in another pool.
    assert_eq!(shown.len(), 10);
    assert_eq!(shown[0].label.as_deref(), Some("Start • OutcomeUnknown"));
    assert_eq!(shown[0].value.as_deref(), Some("unknown"));
    assert_eq!(shown[1].label.as_deref(), Some("Start • Tracking"));
    assert_eq!(shown[2].label.as_deref(), Some("Start • Succeeded"));
    assert!(h.has_text("Synthetic operation 12"));
    assert!(!h.has_text("Synthetic operation 3"));
    assert!(!h.has_text("Synthetic operation 21"));
    assert!(!h.has_text("Synthetic operation 22"));
    assert_eq!(
        h.node("general-operations-note").text.as_deref(),
        Some("Demo mode cannot change infrastructure.")
    );
    // Under each record what Rune showed under it, and nothing twice.
    let under = |h: &Harness, index: u128| -> Vec<(String, String)> {
        let id = uuid::Uuid::from_u128(0x8100_0000_0000_0000_0000_0000_0000_0000 + index);
        h.nodes()
            .into_iter()
            .filter(|node| node.role == "button" && node.path.contains(&format!("vm-operation-{id}")))
            .map(|node| {
                let key = node.key.unwrap_or_default();
                (
                    key.strip_suffix(&id.to_string()).unwrap_or(&key).to_owned(),
                    node.label.unwrap_or_default(),
                )
            })
            .collect()
    };
    let pair = |key: &str, label: &str| (key.to_owned(), label.to_owned());
    // An unknown outcome: its explanation (events), then the review of the
    // power area. A fixture record cannot be rechecked.
    assert_eq!(
        under(&h, 20),
        [
            pair("vm-failure-help-", "Explain outcome…"),
            pair("operation-unknown-", "Review unknown outcome"),
        ]
    );
    // In progress and cancellable: the request and its reviewed form.
    assert_eq!(
        under(&h, 19),
        [
            pair("operation-cancel-", "Request cancellation"),
            pair("recovery-cancel-", "Review cancellation request…"),
        ]
    );
    // A finished record has nothing to press.
    assert!(under(&h, 12).is_empty());
    h.assert_unique_keys();
    // The buttons do what they did in Rune. The explanation opens over the
    // tab; the review is the one acknowledgement dialog, in its Rune style.
    let unknown = uuid::Uuid::from_u128(0x8100_0000_0000_0000_0000_0000_0000_0014);
    h.click(&format!("vm-failure-help-{unknown}"));
    assert_eq!(h.surface().id, "failure-help-dialog");
    h.dismiss();
    let review = format!("operation-unknown-{unknown}");
    assert_eq!(h.node(&review).tone, Some(Tone::Warning));
    h.click(&review);
    assert_eq!(h.surface().id, "unknown-outcome");
    assert_eq!(h.surface().title, "Acknowledge unknown outcome");
    h.dismiss();
    assert!(h.surfaces().is_empty());
    // The reviewed cancellation is the power area's recovery review.
    let running = uuid::Uuid::from_u128(0x8100_0000_0000_0000_0000_0000_0000_0013);
    h.click(&format!("recovery-cancel-{running}"));
    assert_eq!(h.surface().id, "vm-recovery");
    h.dismiss();
    // Asking directly needs a connection: the refusal is on the error line.
    h.model.error = None;
    h.click(&format!("operation-cancel-{running}"));
    assert_eq!(h.model.error, Some(xenadmin_core::Error::Closed.to_string()));
    // Above the operations: why each power action is or is not available,
    // the power area's section.
    assert!(h.find("power-availability").is_some());
    assert_eq!(
        h.node("power-reason-Start").value.as_deref(),
        Some("Demo mode cannot change infrastructure.")
    );
    let keys = h.keys();
    let position = |key: &str| keys.iter().position(|other| other == key).unwrap();
    assert!(position("general-summary") < position("power-availability"));
    assert!(position("power-availability") < position("general-operations"));
    // A template has no Operations section, as in Rune.
    let template = inventory(&h)
        .vms
        .values()
        .find(|vm| vm.is_template && !vm.is_control_domain)
        .map(|vm| vm.id);
    if let Some(template) = template {
        h.select(Target::one(ObjectRef::template(pool, template)));
        h.open_tab(TabId::General);
        assert!(h.find("general-operations").is_none());
    }
}
