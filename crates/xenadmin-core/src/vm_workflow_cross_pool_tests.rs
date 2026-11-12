//! A batch of cross-pool moves in the journal: what a valid one is, what it reserves, which
//! children it admits, and who may fence or release it.
use super::*;
use crate::{CrossPoolMode, CrossPoolVmOperation, DiskPlacement, NicPlacement};

fn away() -> PoolId {
    Uuid::from_u128(101).into()
}
fn operation(n: u128, live: bool) -> CrossPoolVmOperation {
    CrossPoolVmOperation {
        vm: vm(n),
        destination_pool: away(),
        destination_host: Uuid::from_u128(102).into(),
        expected_host: live.then(|| Uuid::from_u128(10).into()),
        expected_affinity: None,
        transfer_network: Uuid::from_u128(103).into(),
        transfer_address: "192.0.2.20".into(),
        mode: if live {
            CrossPoolMode::MoveRunning
        } else {
            CrossPoolMode::MoveHalted
        },
        disks: vec![DiskPlacement {
            vdi: Uuid::from_u128(800 + n).into(),
            expected_sr: Uuid::from_u128(4).into(),
            destination_sr: Uuid::from_u128(104).into(),
        }],
        networks: vec![NicPlacement {
            vif: Uuid::from_u128(700 + n).into(),
            expected_network: Uuid::from_u128(5).into(),
            destination_network: Uuid::from_u128(105).into(),
        }],
        host_transport_reviewed: true,
    }
}
/// A halted guest and a running one, to the same pool, host and migration address.
fn batch() -> VmWorkflowBatch {
    VmWorkflowBatch {
        items: vec![
            BatchVmWorkflow::move_to_pool(operation(2, false)),
            BatchVmWorkflow::move_to_pool(operation(3, true)),
        ],
    }
}
fn observed(pool: PoolId) -> crate::ObservedResource {
    crate::ObservedResource {
        pool,
        generation: 4,
        target: ResourceTarget::Pool(pool),
    }
}
fn parent_moves() -> crate::resources::ResourceRequest {
    crate::resources::channel(
        OperationId::new(),
        observed(pool()),
        ResourceAction::VmWorkflowBatch(batch()),
        tokio::runtime::Handle::current(),
    )
    .1
}
fn child_move(
    p: &crate::ResourceOperationRecord,
    index: usize,
) -> crate::resources::ResourceRequest {
    let plan = p.workflow_batch_plan.as_ref().unwrap();
    let operation = plan.selection.items[index]
        .workflow
        .cross_pool()
        .unwrap()
        .clone();
    let mut request = crate::resources::channel(
        plan.children[index],
        observed(p.pool),
        ResourceAction::CrossPoolVm(operation),
        tokio::runtime::Handle::current(),
    )
    .1;
    request.record.batch_parent = Some(p.id);
    request
}
fn rename(pool: PoolId) -> crate::resources::ResourceRequest {
    crate::resources::channel(
        OperationId::new(),
        observed(pool),
        ResourceAction::RenameMetadata {
            expected_name: "old".into(),
            expected_description: String::new(),
            name: "new".into(),
            description: String::new(),
        },
        tokio::runtime::Handle::current(),
    )
    .1
}
fn start(pool: PoolId) -> MutationRequest {
    channel(
        OperationId::new(),
        pool,
        vm(90),
        4,
        OperationAction::Power(PowerAction::Start),
        tokio::runtime::Handle::current(),
    )
    .1
}
fn item(batch: &mut VmWorkflowBatch, index: usize) -> &mut CrossPoolVmOperation {
    let VmWorkflow::MoveToPool(operation) = &mut batch.items[index].workflow else {
        panic!("a cross-pool item")
    };
    operation
}

#[test]
fn a_batch_of_cross_pool_moves_is_reviewed_moves_to_one_destination() {
    let target = ResourceTarget::Pool(pool());
    let valid = batch();
    valid.validate(target).unwrap();
    assert_eq!(valid.cross_pool_destination(), Some(away()));
    assert!(!valid.is_export() && !valid.is_snapshot_export());
    assert_eq!(valid.noun(), "VM");
    // The source pool is asked to send each guest and the destination to receive it.
    assert_eq!(
        valid
            .permission_requirements()
            .methods()
            .collect::<Vec<_>>(),
        ["VM.migrate_send"]
    );
    assert_eq!(
        valid
            .peer_permission_requirements()
            .unwrap()
            .methods()
            .collect::<Vec<_>>(),
        ["host.migrate_receive"]
    );
    assert!(selection().peer_permission_requirements().is_none());
    let action = ResourceAction::VmWorkflowBatch(valid.clone());
    assert_eq!(action.label(), "Move selected VMs to another pool");
    assert!(action.spans_pools() && !action.read_only_export());
    assert_eq!(crate::cross_pool::destination(&action), Some(away()));
    assert!(!ResourceAction::VmWorkflowBatch(selection()).spans_pools());
    assert_eq!(
        valid.items[0].step_label(),
        format!("Move VM to another pool {}", vm(2))
    );
    // A batch of one is a batch.
    VmWorkflowBatch {
        items: vec![BatchVmWorkflow::move_to_pool(operation(2, false))],
    }
    .validate(target)
    .unwrap();

    let refused = |label: &str, edit: &dyn Fn(&mut VmWorkflowBatch)| {
        let mut changed = batch();
        edit(&mut changed);
        assert!(changed.validate(target).is_err(), "{label}");
    };
    refused("no transport acknowledgement", &|b| {
        for index in 0..2 {
            item(b, index).host_transport_reviewed = false;
        }
    });
    refused("one guest without the acknowledgement", &|b| {
        item(b, 1).host_transport_reviewed = false
    });
    refused("a second destination pool", &|b| {
        item(b, 1).destination_pool = Uuid::from_u128(201).into()
    });
    refused("a second destination host", &|b| {
        item(b, 1).destination_host = Uuid::from_u128(202).into()
    });
    refused("a second migration network", &|b| {
        item(b, 1).transfer_network = Uuid::from_u128(203).into()
    });
    refused("a second migration address", &|b| {
        item(b, 1).transfer_address = "192.0.2.21".into()
    });
    refused("the source pool as destination", &|b| {
        for index in 0..2 {
            item(b, index).destination_pool = pool();
        }
    });
    refused("a copy", &|b| item(b, 0).mode = CrossPoolMode::CopyHalted);
    refused("a guest reviewed in another state than its move", &|b| {
        b.items[0].expected_state = PowerState::Running
    });
    refused("a suspended guest", &|b| {
        b.items[0].expected_state = PowerState::Suspended
    });
    refused("a paused guest", &|b| {
        b.items[1].expected_state = PowerState::Paused
    });
    refused("an item for another guest than its move", &|b| {
        b.items[0].vm = vm(9)
    });
    refused("a guest named twice", &|b| {
        b.items[1] = BatchVmWorkflow::move_to_pool(operation(2, false))
    });
    refused("a disk named twice", &|b| {
        item(b, 1).disks[0].vdi = Uuid::from_u128(802).into()
    });
    refused("a NIC named twice", &|b| {
        item(b, 1).networks[0].vif = Uuid::from_u128(702).into()
    });
    refused("an unmapped disk", &|b| {
        item(b, 0).disks[0].destination_sr = Uuid::nil().into()
    });
    refused("a move within the pool among them", &|b| {
        b.items[1].workflow = VmWorkflow::Move(crate::VmMigration {
            destination: Uuid::from_u128(11).into(),
            expected_host: Some(Uuid::from_u128(10).into()),
            expected_affinity: None,
            live: true,
            disks: vec![],
            networks: vec![],
            transfer_network: None,
        })
    });
    refused("no guest", &|b| b.items.clear());
    refused("more than 32 guests", &|b| {
        b.items = (0..33)
            .map(|n| BatchVmWorkflow::move_to_pool(operation(1000 + n, false)))
            .collect()
    });
    assert!(
        valid
            .validate(ResourceTarget::Host(Uuid::from_u128(2).into()))
            .is_err()
    );
    // 32 guests are the most a batch holds.
    VmWorkflowBatch {
        items: (0..32)
            .map(|n| BatchVmWorkflow::move_to_pool(operation(1000 + n, false)))
            .collect(),
    }
    .validate(target)
    .unwrap();
}

#[tokio::test]
async fn a_batch_of_cross_pool_moves_reserves_both_pools_and_admits_only_its_exact_children() {
    let dir = tempfile::tempdir().unwrap();
    let mut j = Journal::open(dir.path()).unwrap();
    let mut p = parent_moves();
    assert!(j.resource_admit(&mut p.record).unwrap());
    let third: PoolId = Uuid::from_u128(301).into();
    for reserved in [pool(), away()] {
        assert!(j.resource_pool_busy(reserved).unwrap());
        assert!(matches!(
            j.resource_admit(&mut rename(reserved).record),
            Err(Error::ConflictingPoolOperation)
        ));
        assert!(matches!(
            j.admit(&mut start(reserved).record),
            Err(Error::ConflictingPoolOperation)
        ));
    }
    assert!(!j.resource_pool_busy(third).unwrap());
    // The batch reads back as it was stored, for either pool and for no other.
    let stored = j.resource_load(p.record.id).unwrap().unwrap();
    assert_eq!(stored.action, ResourceAction::VmWorkflowBatch(batch()));
    assert_eq!(stored.peer_pool(), Some(away()));
    for (history, listed) in [(pool(), true), (away(), true), (third, false)] {
        assert_eq!(
            j.resource_history(history)
                .unwrap()
                .iter()
                .any(|r| r.id == p.record.id),
            listed
        );
    }

    // A child is admitted only with the batch's permit and only as the reviewed move.
    let mut first = child_move(&p.record, 0);
    assert!(j.resource_admit(&mut first.record).is_err());
    let permit = j.workflow_permit(p.record.id, 0).unwrap();
    let mut remapped = child_move(&p.record, 0);
    let ResourceAction::CrossPoolVm(changed) = &mut remapped.record.action else {
        panic!("a cross-pool move")
    };
    changed.disks[0].destination_sr = Uuid::from_u128(999).into();
    assert!(matches!(
        j.resource_admit_permitted(&mut remapped.record, Some(&permit)),
        Err(Error::OperationConflict)
    ));
    let mut other_guest = child_move(&p.record, 1);
    other_guest.record.id = first.record.id;
    assert!(
        j.resource_admit_permitted(&mut other_guest.record, Some(&permit))
            .is_err()
    );
    let second_permit = j.workflow_permit(p.record.id, 1).unwrap();
    let mut second = child_move(&p.record, 1);
    assert!(
        j.resource_admit_permitted(&mut second.record, Some(&second_permit))
            .is_err(),
        "the second guest waits for the first"
    );
    assert!(
        j.resource_admit_permitted(&mut first.record, Some(&permit))
            .unwrap()
    );
    assert_eq!(
        j.resource_load(p.record.id)
            .unwrap()
            .unwrap()
            .workflow_batch_plan
            .unwrap()
            .admitted,
        1
    );
    // The child reserves the same two pools and excludes nothing but its parent.
    assert_eq!(first.record.pool_scopes(), vec![pool(), away()]);
    assert!(
        j.resource_admit_permitted(&mut second.record, Some(&second_permit))
            .is_err(),
        "the first guest has not succeeded"
    );
    assert!(j.resource_load(second.record.id).unwrap().is_none());
    // Neither the child nor the batch takes an ordinary control or an ordinary recheck.
    assert!(
        j.resource_control(pool(), first.record.id, OperationControl::Recheck)
            .is_err()
    );
    assert!(
        j.resource_control(
            pool(),
            first.record.id,
            OperationControl::AcknowledgeUnknown
        )
        .is_err()
    );

    // A stored child whose mapping differs from the reviewed one fails the batch closed.
    let payload: String =
        j.0.query_row(
            "SELECT payload FROM resource_operations WHERE id=?1",
            [first.record.id.to_string()],
            |r| r.get(0),
        )
        .unwrap();
    j.0.execute(
        "UPDATE resource_operations SET payload=json_set(payload,'$.action.CrossPoolVm.disks[0].destination_sr',?2) WHERE id=?1",
        rusqlite::params![first.record.id.to_string(), Uuid::from_u128(999).to_string()],
    )
    .unwrap();
    assert!(j.resource_load(p.record.id).is_err());
    assert!(j.resource_load(first.record.id).is_err());
    j.0.execute(
        "UPDATE resource_operations SET payload=?2 WHERE id=?1",
        rusqlite::params![first.record.id.to_string(), payload],
    )
    .unwrap();
    j.resource_load(p.record.id).unwrap().unwrap();

    // Recovery by the destination pool's connection fences the batch and its child.
    assert!(
        j.workflow_interrupt(third, p.record.id, false).is_err(),
        "a pool the batch does not touch cannot fence it"
    );
    let recovered = j.resource_recover(away()).unwrap();
    let fenced = recovered.iter().find(|r| r.id == p.record.id).unwrap();
    assert_eq!(fenced.state, OperationState::OutcomeUnknown);
    assert!(fenced.interrupted);
    assert!(fenced.detail.contains("Reconnect both pools and recheck"));
    let child = j.resource_load(first.record.id).unwrap().unwrap();
    assert_eq!(child.state, OperationState::OutcomeUnknown);
    assert!(child.interrupted);
    assert!(j.workflow_child_stopped(first.record.id).unwrap());
    assert!(matches!(
        j.resource_dispatch(&mut first.record),
        Err(Error::JournalChanged)
    ));
    // No child is admitted after the fence, and both pools stay reserved.
    assert!(
        j.resource_admit_permitted(&mut second.record, Some(&second_permit))
            .is_err()
    );
    for reserved in [pool(), away()] {
        assert!(j.resource_pool_busy(reserved).unwrap());
    }
    assert!(
        j.resource_control(third, p.record.id, OperationControl::AcknowledgeUnknown)
            .is_err()
    );
    // The acknowledgement, given on either pool, releases both and the child with them.
    let acknowledged = j
        .resource_control(away(), p.record.id, OperationControl::AcknowledgeUnknown)
        .unwrap();
    assert_eq!(acknowledged.state, OperationState::AcknowledgedUnknown);
    assert!(acknowledged.detail.contains("both pools are released"));
    let child = j.resource_load(first.record.id).unwrap().unwrap();
    assert_eq!(child.state, OperationState::AcknowledgedUnknown);
    assert!(child.detail.contains("was not verified"));
    assert_eq!(acknowledged.workflow_batch[1].state, None);
    for released in [pool(), away()] {
        assert!(!j.resource_pool_busy(released).unwrap());
        assert!(j.resource_admit(&mut rename(released).record).unwrap());
    }
}

#[tokio::test]
async fn an_unknown_move_keeps_what_was_found_about_it_when_its_batch_is_fenced_again() {
    let dir = tempfile::tempdir().unwrap();
    let j = Journal::open(dir.path()).unwrap();
    let mut p = parent_moves();
    j.resource_admit(&mut p.record).unwrap();
    let mut first = child_move(&p.record, 0);
    let permit = j.workflow_permit(p.record.id, 0).unwrap();
    j.resource_admit_permitted(&mut first.record, Some(&permit))
        .unwrap();
    // The first fence finds a move in flight and says what a recheck needs.
    j.workflow_interrupt(pool(), p.record.id, false).unwrap();
    let mut child = j.resource_load(first.record.id).unwrap().unwrap();
    assert!(child.detail.contains("with both pools connected"));
    // A recheck that could not settle it leaves its finding on the move.
    child.detail = "The existing host task is still pending".into();
    j.resource_save(&mut child).unwrap();
    let again = j.workflow_interrupt(away(), p.record.id, false).unwrap();
    assert_eq!(again.state, OperationState::OutcomeUnknown);
    assert_eq!(
        again.workflow_batch[0].detail,
        "The existing host task is still pending"
    );
    let child = j.resource_load(first.record.id).unwrap().unwrap();
    assert_eq!(child.state, OperationState::OutcomeUnknown);
    assert_eq!(child.detail, "The existing host task is still pending");
}
