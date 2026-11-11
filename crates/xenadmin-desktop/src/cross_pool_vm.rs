use super::*;
use xenadmin_core::{
    CrossPoolMode, CrossPoolVmOperation, CrossPoolVmSelection, DiskPlacement, NetworkId,
    NicPlacement, ObservedResource, ObservedVm, OperationControl, OperationId, OperationState,
    ProfileId, ResourceAction, ResourceOperationRecord, ResourceTarget, SrId,
};

#[derive(Clone)]
struct Peer {
    profile: Option<ProfileId>,
    observed: Option<ObservedResource>,
    inventory: Arc<Inventory>,
}
impl Peer {
    fn connection(&self, s: &State) -> Option<Connection> {
        let connection = profiles_view::inspection_connections(s)
            .find(|(id, _)| Some(*id) == self.profile)?
            .1;
        if connection.is_read_only()
            || self.observed.is_none()
            || connection
                .latest()
                .observe_resource(ResourceTarget::Pool(self.inventory.pool_id))
                .ok()
                != self.observed
        {
            return None;
        }
        Some(connection)
    }
    fn current(&self, s: &State) -> Option<Arc<Inventory>> {
        if s.demo {
            return Some(self.inventory.clone());
        }
        self.connection(s)?.latest().inventory().cloned()
    }
}
pub(super) struct Editor {
    source: Peer,
    observed: Option<ObservedVm>,
    vm: Vm,
    peers: Vec<Peer>,
    destination: Option<usize>,
    host: Option<HostId>,
    mode: CrossPoolMode,
    disks: Vec<DiskPlacement>,
    networks: Vec<NicPlacement>,
    transfer: Option<(NetworkId, String)>,
    transport_reviewed: bool,
    stage: u8,
    review: Option<CrossPoolVmOperation>,
    error: Option<String>,
}
#[derive(Default)]
pub(super) struct Panel {
    pub show: bool,
    editor: Option<Editor>,
    acknowledge: Option<OperationId>,
    notice: Option<String>,
}
fn close(s: &mut State) {
    s.cross_pool = Panel::default();
}
pub(super) fn history(s: &mut State) {
    s.cross_pool = Panel {
        show: true,
        ..Default::default()
    };
}
fn peers(s: &State) -> Vec<Peer> {
    let mut pools = std::collections::BTreeSet::new();
    profiles_view::inspection_connections(s)
        .filter_map(|(profile, c)| {
            if c.is_read_only() {
                return None;
            }
            let snapshot = c.latest();
            let inventory = snapshot.inventory()?.clone();
            let observed = snapshot
                .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                .ok()?;
            pools.insert(inventory.pool_id).then_some(Peer {
                profile: Some(profile),
                observed: Some(observed),
                inventory,
            })
        })
        .take(128)
        .collect()
}
fn editor(source: Peer, observed: Option<ObservedVm>, vm: Vm, peers: Vec<Peer>) -> Editor {
    let disks = source
        .inventory
        .details
        .vbds
        .values()
        .filter(|b| b.vm == Some(vm.id) && b.kind == "Disk")
        .filter_map(|b| source.inventory.details.vdis.get(&b.vdi?))
        .filter_map(|d| {
            Some(DiskPlacement {
                vdi: d.id,
                expected_sr: d.sr?,
                destination_sr: d.sr?,
            })
        })
        .collect();
    let networks = source
        .inventory
        .details
        .vifs
        .values()
        .filter(|n| n.vm == Some(vm.id))
        .filter_map(|n| {
            Some(NicPlacement {
                vif: n.id,
                expected_network: n.network?,
                destination_network: n.network?,
            })
        })
        .collect();
    let mode = if vm.power_state == PowerState::Running {
        CrossPoolMode::MoveRunning
    } else {
        CrossPoolMode::MoveHalted
    };
    Editor {
        source,
        observed,
        vm,
        peers,
        destination: None,
        host: None,
        mode,
        disks,
        networks,
        transfer: None,
        transport_reviewed: false,
        stage: 0,
        review: None,
        error: None,
    }
}
pub(super) fn open(s: &mut State, id: VmId) {
    if !s.can_mutate() {
        return;
    }
    let Some(i) = s.inventory.as_ref() else {
        return;
    };
    let Some(vm) = i
        .vms
        .get(&id)
        .filter(|v| {
            !v.is_template
                && !v.is_control_domain
                && matches!(v.power_state, PowerState::Running | PowerState::Halted)
        })
        .cloned()
    else {
        return;
    };
    let observed = s.snapshot.as_ref().and_then(|p| p.observe_vm(id).ok());
    let Some((profile, connection)) = profiles_view::inspection_connections(s)
        .find(|(_, c)| c.latest().observe_vm(id).ok() == observed && observed.is_some())
    else {
        return;
    };
    let snapshot = connection.latest();
    let source = Peer {
        profile: Some(profile),
        observed: snapshot
            .observe_resource(ResourceTarget::Pool(i.pool_id))
            .ok(),
        inventory: i.clone(),
    };
    let available = peers(s)
        .into_iter()
        .filter(|p| p.inventory.pool_id != i.pool_id)
        .collect();
    s.cross_pool = Panel {
        show: true,
        editor: Some(editor(source, observed, vm, available)),
        ..Default::default()
    };
}
pub(super) fn button(s: &State, id: VmId, k: &Kit) -> El {
    if s.inventory
        .as_ref()
        .and_then(|i| i.vms.get(&id))
        .is_none_or(|v| v.is_template || v.is_control_domain)
    {
        return el();
    }
    Button::new("Move / copy to another pool…")
        .disabled(!s.can_mutate())
        .on_press(move |s: &mut State| open(s, id))
        .build(k)
        .key("cross-pool-open")
}
fn choose(s: &mut State, index: usize) {
    let Some(e) = s.cross_pool.editor.as_mut() else {
        return;
    };
    if index >= e.peers.len() {
        return;
    }
    e.destination = Some(index);
    e.host = None;
    e.transfer = None;
    e.transport_reviewed = false;
    e.review = None;
    e.error = None;
    // Destination UUIDs are deliberately invalid until explicitly selected for this pool.
    for d in &mut e.disks {
        d.destination_sr = SrId::from(uuid::Uuid::nil());
    }
    for n in &mut e.networks {
        n.destination_network = NetworkId::from(uuid::Uuid::nil());
    }
}
fn destination(e: &Editor) -> Option<&Peer> {
    e.peers.get(e.destination?)
}
fn current(s: &State, e: &Editor) -> bool {
    let source = e.source.current(s);
    source
        .as_ref()
        .is_some_and(|i| i.vms.contains_key(&e.vm.id))
        && s.inventory
            .as_ref()
            .is_some_and(|i| i.pool_id == e.source.inventory.pool_id)
        && (s.demo
            || e.observed.is_some_and(|o| {
                s.snapshot.as_ref().and_then(|p| p.observe_vm(e.vm.id).ok()) == Some(o)
            }))
        && destination(e).is_none_or(|p| p.current(s).is_some())
}
fn operation(e: &Editor) -> Result<CrossPoolVmOperation, String> {
    let d = destination(e).ok_or("Choose a connected destination pool")?;
    let (network, address) = e
        .transfer
        .clone()
        .ok_or("Choose an attached destination transfer address")?;
    if e.disks.iter().any(|v| v.destination_sr.uuid().is_nil())
        || e.networks
            .iter()
            .any(|v| v.destination_network.uuid().is_nil())
    {
        return Err("Map every disk and network interface explicitly".into());
    }
    Ok(CrossPoolVmOperation {
        vm: e.vm.id,
        destination_pool: d.inventory.pool_id,
        destination_host: e.host.ok_or("Choose a destination host")?,
        expected_host: e.vm.host,
        expected_affinity: e
            .source
            .inventory
            .details
            .vms
            .get(&e.vm.id)
            .and_then(|v| v.affinity),
        transfer_network: network,
        transfer_address: address,
        mode: e.mode,
        disks: e.disks.clone(),
        networks: e.networks.clone(),
        host_transport_reviewed: e.transport_reviewed,
    })
}
fn availability(s: &State, e: &Editor) -> xenadmin_core::ActionAvailability {
    let Some((source, dest)) = e
        .source
        .current(s)
        .zip(destination(e).and_then(|p| p.current(s)))
    else {
        return xenadmin_core::ActionAvailability::Stale;
    };
    e.review
        .as_ref()
        .map_or(xenadmin_core::ActionAvailability::Unknown, |o| {
            o.availability(&source, &dest)
        })
}
fn next(s: &mut State) {
    let Some(e) = s.cross_pool.editor.as_mut() else {
        return;
    };
    e.error = None;
    if e.stage == 0 {
        if e.destination.is_some() && e.host.is_some() {
            e.stage = 1;
        } else {
            e.error = Some("Choose a destination pool and host".into());
        }
    } else {
        // Transport acknowledgement belongs to the final review, after exact mapping.
        match operation(e) {
            Ok(o) => {
                e.review = Some(o);
                e.stage = 2;
            }
            Err(error) => e.error = Some(error),
        }
    }
}
fn connection_for_pool(s: &State, pool: PoolId) -> Option<Connection> {
    profiles_view::inspection_connections(s)
        .map(|(_, c)| c)
        .chain(s.connection.iter().cloned())
        .find(|c| {
            !c.is_read_only()
                && c.latest()
                    .observe_resource(ResourceTarget::Pool(pool))
                    .is_ok()
        })
}
fn apply(s: &mut State) {
    let result = (|| {
        let e = s
            .cross_pool
            .editor
            .as_ref()
            .ok_or("Transfer review is closed")?;
        if s.demo || !s.can_mutate() || !current(s, e) || !availability(s, e).is_available() {
            return Err("The reviewed source or destination is no longer available".to_string());
        }
        let d = destination(e).ok_or("Destination is unavailable")?;
        let source = e.source.connection(s).ok_or("Source disconnected")?;
        let dest = d.connection(s).ok_or("Destination disconnected")?;
        let operation = e.review.clone().ok_or("Review the transfer first")?;
        source
            .transfer_vm_to_pool(
                OperationId::new(),
                CrossPoolVmSelection {
                    source: e.observed.ok_or("Source is stale")?,
                    destination: d.observed.ok_or("Destination is stale")?,
                    operation,
                },
                &dest,
            )
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            if let Some(e) = &s.cross_pool.editor
                && e.mode.live()
                && s.console_ui
                    .target()
                    .is_some_and(|t| t.pool == e.source.inventory.pool_id && t.vm == e.vm.id)
            {
                console_view::release(s);
                s.console = None;
                s.console_snapshot = None;
                s.console_ui = Default::default();
            }
            s.resource_operations.push(op);
            history(s);
        }
        Err(error) => s.cross_pool.notice = Some(error),
    }
}
pub(super) fn can_recheck(s: &State, id: OperationId) -> bool {
    s.resource_rows.iter().find(|r|r.id==id).is_some_and(|r|matches!(&r.action,ResourceAction::CrossPoolVm(o) if connection_for_pool(s,r.pool).is_some()&&connection_for_pool(s,o.destination_pool).is_some()))
}
pub(super) fn recheck(s: &mut State, id: OperationId) -> Result<(), xenadmin_core::Error> {
    let r = s
        .resource_rows
        .iter()
        .find(|r| r.id == id)
        .ok_or(xenadmin_core::Error::OperationConflict)?;
    let ResourceAction::CrossPoolVm(o) = &r.action else {
        return Err(xenadmin_core::Error::InvalidMigration);
    };
    let source = connection_for_pool(s, r.pool).ok_or(xenadmin_core::Error::Closed)?;
    let dest = connection_for_pool(s, o.destination_pool).ok_or(xenadmin_core::Error::Closed)?;
    let handle = source.recheck_cross_pool_vm(id, &dest)?;
    if let Some(w) = &s.waker {
        handle.notify_on_change(w.clone());
    }
    s.resource_operations.push(handle);
    Ok(())
}
fn control(s: &mut State, id: OperationId, control: OperationControl) {
    let result = if matches!(control, OperationControl::Recheck) {
        recheck(s, id)
    } else {
        profiles_view::resource_control_connection(s, id, control)
            .ok_or(xenadmin_core::Error::Closed)
            .and_then(|c| c.control_resource(id, control))
    };
    s.cross_pool.notice = Some(
        result
            .map(|()| {
                match control {
                    OperationControl::Recheck => {
                        "Checking existing evidence; the transfer is never repeated."
                    }
                    OperationControl::RequestCancellation => {
                        "Cancellation requested; the hosts may still be transferring data."
                    }
                    OperationControl::AcknowledgeUnknown => {
                        "Acknowledgement requested; the unresolved transfer remains in history."
                    }
                }
                .into()
            })
            .unwrap_or_else(|e| e.to_string()),
    );
    s.cross_pool.acknowledge = None;
}
fn destination_guest(
    inventory: &Inventory,
    record: &ResourceOperationRecord,
) -> Option<(VmId, bool)> {
    let ResourceAction::CrossPoolVm(o) = &record.action else {
        return None;
    };
    if record.state != OperationState::Succeeded
        || record.transfer_created.vms.len() != 1
        || inventory.pool_id != o.destination_pool
    {
        return None;
    }
    let vm = record.transfer_created.vms[0];
    let guest = inventory.vms.get(&vm)?;
    if guest.is_template
        || guest.is_control_domain
        || (o.mode.live()
            && (guest.host != Some(o.destination_host) || guest.power_state != PowerState::Running))
    {
        return None;
    }
    Some((vm, o.mode.live()))
}
fn destination_target(s: &State, r: &ResourceOperationRecord) -> Option<(ProfileId, VmId, bool)> {
    profiles_view::inspection_connections(s).find_map(|(profile, c)| {
        if c.is_read_only() {
            return None;
        }
        let snapshot = c.latest();
        let (vm, console) = destination_guest(snapshot.inventory()?, r)?;
        snapshot.observe_vm(vm).ok().map(|_| (profile, vm, console))
    })
}

fn show_destination(s: &mut State, id: OperationId) {
    let target = s
        .resource_rows
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| destination_target(s, r));
    let Some((profile, vm, console)) = target else {
        s.cross_pool.notice =
            Some("Wait for current destination inventory to confirm this VM and host.".into());
        return;
    };
    let pool = s
        .resource_rows
        .iter()
        .find(|r| r.id == id)
        .and_then(|r| r.peer_pool())
        .unwrap();
    if !profiles_view::select_transfer_destination(s, profile, pool, vm) {
        return;
    }
    close(s);
    if console {
        s.open_console();
    }
}
#[path = "cross_pool_vm_view.rs"]
mod ui;
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    ui::dialog(s, k)
}

pub(super) fn scene(s: &mut State) {
    vm_migration::scene(s);
    s.vm_migration = None;
    let id = VmId::parse("30000000-0000-0000-0000-000000000000").unwrap();
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let details = i.details.vms.get_mut(&id).unwrap();
    details.ha_restart_priority = Some(String::new());
    details.snapshot_policy = None;
    details.appliance = None;
    details.appliance_known = true;
    details.affinity_known = true;
    let vm = i.vms[&id].clone();
    for disk in i.details.vdis.values_mut() {
        disk.sharable = Some(false);
        disk.read_only = Some(false);
        disk.is_snapshot = Some(false);
        disk.cbt_enabled = Some(false);
        disk.managed = Some(true);
        disk.missing = Some(false);
    }
    for sr in i.details.storage.values_mut() {
        sr.content_type = "user".into();
        sr.allowed_operations = Some(vec!["vdi_create".into()]);
    }
    let source = Peer {
        profile: None,
        observed: None,
        inventory: s.inventory.as_ref().unwrap().clone(),
    };
    let mut destination = source.inventory.as_ref().clone();
    destination.pool_id = PoolId::from(uuid::Uuid::from_u128(85001));
    destination.pool_name = "Destination lab pool".into();
    destination.vms.clear();
    destination.details.vms.clear();
    destination.details.vdis.clear();
    destination.details.vbds.clear();
    destination.details.vifs.clear();
    destination.details.vdi_attachments.clear();
    let dest = Peer {
        profile: None,
        observed: None,
        inventory: Arc::new(destination),
    };
    let mut e = editor(source, None, vm, vec![dest]);
    e.destination = Some(0);
    let dest = &e.peers[0].inventory;
    e.host = dest.hosts.keys().next().copied();
    let pif = dest
        .details
        .pifs
        .values()
        .find(|p| p.host == e.host)
        .unwrap();
    e.transfer = Some((pif.network.unwrap(), pif.ip.clone()));
    e.transport_reviewed = true;
    e.stage = 2;
    e.review = Some(operation(&e).unwrap());
    s.cross_pool = Panel {
        show: true,
        editor: Some(e),
        ..Default::default()
    };
}
#[cfg(test)]
#[path = "cross_pool_vm_tests.rs"]
mod tests;
