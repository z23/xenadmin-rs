//! The detail lines of an operation record beyond what the list searches:
//! the local error and, for a resource operation, the evidence the Rune pool
//! history (`resource_admin::history`) printed under each record: reviewed
//! consumers of a management bond, IPv6 management stages, recreated VLANs,
//! created and removed network objects, and the results of a batch.
//!
//! The texts are word for word. What the row's own details already say (the
//! record's detail, its created resource and its steps) is not repeated.

use xenadmin_core::{
    ManagementNetworkOperation, OperationRecord, ResourceAction, ResourceOperationRecord,
    VmWorkflow,
};

pub fn vm(record: &OperationRecord) -> Vec<String> {
    record.local_error.iter().cloned().collect()
}

pub fn resource(record: &ResourceOperationRecord) -> Vec<String> {
    let mut lines: Vec<String> = record.local_error.iter().cloned().collect();
    if let ResourceAction::ManagementNetwork(op) = &record.action
        && let ManagementNetworkOperation::Bond {
            consumers: Some(consumers),
            ..
        } = op.as_ref()
    {
        for v in consumers.interfaces.values() {
            lines.push(format!(
                "Reviewed NIC {} · VM {} · {} · {} → {}",
                v.interface,
                v.vm,
                v.change.label(),
                v.network,
                v.destination
            ));
        }
    }
    if let Some(e) = &record.management_ipv6_bond {
        lines.push(format!(
            "IPv6 management: {}/5 stages verified · staged reservation {} · temporary recovery {} · target {}",
            e.completed_stages,
            if e.retains_reservation {
                "held"
            } else {
                "not held by this row"
            },
            e.temporary_endpoint,
            e.target_endpoint
        ));
        if let Some(m) = e.created_bond {
            lines.push(format!("Created bond {} · master {}", m.bond, m.master));
        }
        for m in &e.moved_vlans {
            lines.push(format!(
                "Recreated VLAN {} → {} · PIF {} → {}",
                m.old_vlan, m.new_vlan, m.old_interface, m.new_interface
            ));
        }
    }
    if let Some(evidence) = &record.management_network {
        for moved in &evidence.moved_vlans {
            lines.push(format!(
                "Recreated VLAN {} → {} · interface {} → {}",
                moved.old_vlan, moved.new_vlan, moved.old_interface, moved.new_interface
            ));
        }
        if let Some(created) = evidence.created_bond {
            lines.push(format!(
                "Created bond {} · master interface {}",
                created.bond, created.master
            ));
        }
        lines.push(format!(
            "Management configuration observed: {} · recovery endpoint verified: {}",
            if evidence.configuration_observed {
                "yes"
            } else {
                "not yet"
            },
            if evidence.recovery_endpoint_verified {
                "yes"
            } else {
                "not yet"
            }
        ));
        if let Some(endpoint) = &evidence.recovery_endpoint {
            lines.push(format!("Reviewed recovery endpoint: {endpoint}"));
        }
    }
    if let Some(e) = &record.network_fabric_created {
        lines.push(format!(
            "Created network object {} · interface {} · host {} · network {}",
            e.object.uuid(),
            e.interface,
            e.host,
            e.network
        ));
    }
    for (label, evidence) in [
        ("Created", &record.topology_changes.created_fabric),
        ("Removed", &record.topology_changes.removed_fabric),
    ] {
        for e in evidence {
            lines.push(format!(
                "{label} network object {} · interface {} · host {}",
                e.object.uuid(),
                e.interface,
                e.host
            ));
        }
    }
    for (label, ids) in [
        (
            "Created VLAN",
            record
                .topology_changes
                .created_vlans
                .iter()
                .map(|id| id.to_string())
                .collect::<Vec<_>>(),
        ),
        (
            "Created interface",
            record
                .topology_changes
                .created_pifs
                .iter()
                .map(|id| id.to_string())
                .collect(),
        ),
        (
            "Removed VLAN",
            record
                .topology_changes
                .removed_vlans
                .iter()
                .map(|id| id.to_string())
                .collect(),
        ),
        (
            "Removed interface",
            record
                .topology_changes
                .removed_pifs
                .iter()
                .map(|id| id.to_string())
                .collect(),
        ),
    ] {
        for id in ids {
            lines.push(format!("{label}: {id}"));
        }
    }
    for (index, result) in record.workflow_batch.iter().enumerate() {
        // Several snapshots of one VM share its UUID, so each line names its snapshot.
        let snapshot = match &record.action {
            ResourceAction::VmWorkflowBatch(batch) => {
                match batch.items.get(index).map(|item| &item.workflow) {
                    Some(VmWorkflow::ExportSnapshot { snapshot, .. }) => {
                        format!("Snapshot {snapshot} of ")
                    }
                    _ => String::new(),
                }
            }
            _ => String::new(),
        };
        lines.push(format!(
            "{snapshot}VM {} · child {} · {}",
            result.vm,
            result.child,
            result
                .state
                .as_ref()
                .map_or_else(|| "Not started".into(), |s| format!("{s:?}")),
        ));
        if !result.detail.is_empty() {
            lines.push(result.detail.clone());
        }
    }
    lines
}
