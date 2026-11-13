use crate::{
    CrossPoolVmOperation, Error, PoolId, TransferCreated, VmId, migration_backend as m, snapshots,
    xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::{BTreeMap, BTreeSet};
use uuid::Uuid;
use xenadmin_xapi::enumerations::{
    on_boot, primary_address_type, vbd_mode, vbd_type, vdi_type, vm_operations, vm_power_state,
};
use xenadmin_xapi::records::{PifRecord, PoolRecord, SrRecord, VbdRecord, VdiRecord, VmRecord};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Shape {
    name: String,
    description: String,
    cpus: [u64; 2],
    memory: [u64; 4],
    firmware: String,
    boot_order: String,
    secureboot: String,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    pub source_pool: PoolId,
    pub operation: CrossPoolVmOperation,
    pub graph: m::Graph,
    pub shape: Shape,
    pub baseline: BTreeMap<String, BTreeSet<Uuid>>,
    pub task_uuid: Option<Uuid>,
    pub source_tasks: Option<BTreeSet<Uuid>>,
    pub expires_at_ms: u64,
    pub completed: bool,
    pub ended: bool,
    pub destination_vm: Option<VmId>,
}
impl Plan {
    fn validates(
        &self,
        record: &crate::ResourceOperationRecord,
        operation: &CrossPoolVmOperation,
    ) -> bool {
        let unique = |ids: Vec<Uuid>| {
            ids.len() <= 256
                && ids.iter().all(|id| !id.is_nil())
                && ids.iter().collect::<BTreeSet<_>>().len() == ids.len()
        };
        let valid_set =
            |ids: &BTreeSet<Uuid>| ids.len() <= 10_000 && ids.iter().all(|id| !id.is_nil());
        self.source_pool == record.pool
            && &self.operation == operation
            && operation.validate(record.pool).is_ok()
            && self.expires_at_ms > 0
            && self
                .baseline
                .keys()
                .map(String::as_str)
                .collect::<BTreeSet<_>>()
                == BTreeSet::from(["VM", "VDI", "VBD", "VIF"])
            && self.baseline.values().all(valid_set)
            && collision_check(self).is_ok()
            && self.source_tasks.as_ref().is_none_or(valid_set)
            && unique(self.graph.disks.iter().map(|b| b.id.uuid()).collect())
            && unique(self.graph.nics.iter().map(|n| n.id.uuid()).collect())
            && self.graph.nics.len() == operation.networks.len()
            && self.graph.disks.iter().filter(|b| b.disk.is_some()).count() == operation.disks.len()
            && self
                .graph
                .disks
                .iter()
                .map(|b| &b.device)
                .collect::<BTreeSet<_>>()
                .len()
                == self.graph.disks.len()
            && self
                .graph
                .nics
                .iter()
                .map(|n| &n.device)
                .collect::<BTreeSet<_>>()
                .len()
                == self.graph.nics.len()
            && self.graph.disks.iter().all(|b| {
                b.device.len() <= 256
                    && match (&b.kind[..], &b.disk) {
                        (vbd_type::CD, None) => true,
                        (vbd_type::DISK, Some(d)) => {
                            b.owner
                                && b.mode == vbd_mode::RW
                                && d.kind == vdi_type::USER
                                && !d.id.uuid().is_nil()
                                && !d.sr.uuid().is_nil()
                        }
                        _ => false,
                    }
            })
            && self
                .graph
                .nics
                .iter()
                .all(|n| !n.network.uuid().is_nil() && n.device.len() <= 256 && n.mac.len() <= 256)
            && self
                .graph
                .disks
                .iter()
                .filter_map(|b| b.disk.as_ref())
                .map(|d| (d.id, d.sr))
                .collect::<BTreeMap<_, _>>()
                == operation
                    .disks
                    .iter()
                    .map(|d| (d.vdi, d.expected_sr))
                    .collect()
            && self
                .graph
                .nics
                .iter()
                .map(|n| (n.id, n.network))
                .collect::<BTreeMap<_, _>>()
                == operation
                    .networks
                    .iter()
                    .map(|n| (n.vif, n.expected_network))
                    .collect()
            && self.shape.name.len() <= 1024
            && self.shape.description.len() <= 16384
            && [
                &self.shape.firmware,
                &self.shape.boot_order,
                &self.shape.secureboot,
            ]
            .iter()
            .all(|s| s.len() <= 256)
            && record.task.as_ref().is_none_or(|r| {
                self.source_tasks.is_some() && snapshots::reference(&json!(r)).is_ok()
            })
            && self.task_uuid.is_none_or(|id| {
                !id.is_nil()
                    && record.task.is_some()
                    && self
                        .source_tasks
                        .as_ref()
                        .is_some_and(|ids| !ids.contains(&id))
            })
            && (!self.ended || self.task_uuid.is_some())
            && (!self.completed || (self.ended && record.result_reference.is_some()))
            && record
                .result_reference
                .as_ref()
                .is_none_or(|r| self.completed && snapshots::reference(&json!(r)).is_ok())
            && self.destination_vm.is_none_or(|id| {
                self.completed
                    && self.fresh("VM", id.uuid()).is_ok()
                    && (if operation.mode.copy() {
                        id != operation.vm
                    } else {
                        id == operation.vm
                    })
            })
            && record.transfer_created.vms == self.destination_vm.into_iter().collect::<Vec<_>>()
            && record.transfer_created.networks.is_empty()
            && (record.state != crate::OperationState::Succeeded
                || (self.completed
                    && self.destination_vm.is_some()
                    && record.transfer_created.disks.len() == operation.disks.len()
                    && record.transfer_created.vbds.len() == self.graph.disks.len()
                    && record.transfer_created.vifs.len() == self.graph.nics.len()
                    && record
                        .steps
                        .iter()
                        .all(|s| s.state == crate::StepState::Verified)))
            && [
                (
                    &record
                        .transfer_created
                        .disks
                        .iter()
                        .map(|id| id.uuid())
                        .collect::<Vec<_>>(),
                    "VDI",
                ),
                (
                    &record
                        .transfer_created
                        .vbds
                        .iter()
                        .map(|id| id.uuid())
                        .collect::<Vec<_>>(),
                    "VBD",
                ),
                (
                    &record
                        .transfer_created
                        .vifs
                        .iter()
                        .map(|id| id.uuid())
                        .collect::<Vec<_>>(),
                    "VIF",
                ),
            ]
            .iter()
            .all(|(ids, class)| {
                unique((*ids).clone())
                    && (ids.is_empty() || self.destination_vm.is_some())
                    && ids.iter().all(|id| self.fresh(class, *id).is_ok())
            })
    }
    pub fn accepts(&self, next: &Self) -> bool {
        self.source_pool == next.source_pool
            && self.operation == next.operation
            && self.graph == next.graph
            && self.shape == next.shape
            && self.baseline == next.baseline
            && self.expires_at_ms == next.expires_at_ms
            && self
                .source_tasks
                .as_ref()
                .is_none_or(|v| next.source_tasks.as_ref() == Some(v))
            && (!self.completed || next.completed)
            && (!self.ended || next.ended)
            && self.task_uuid.is_none_or(|id| next.task_uuid == Some(id))
            && self
                .destination_vm
                .is_none_or(|id| next.destination_vm == Some(id))
    }
    pub fn merge(&mut self, evidence: &Self) -> Result<(), Error> {
        let mut merged = self.clone();
        merged.task_uuid = merged.task_uuid.or(evidence.task_uuid);
        merged.destination_vm = merged.destination_vm.or(evidence.destination_vm);
        merged.completed |= evidence.completed;
        merged.ended |= evidence.ended;
        require(
            self.accepts(&merged) && evidence.accepts(&merged),
            "Cross-pool completion evidence conflicts",
        )?;
        *self = merged;
        Ok(())
    }
    pub fn fresh(&self, class: &str, uuid: Uuid) -> Result<(), Error> {
        require(
            !uuid.is_nil()
                && self
                    .baseline
                    .get(class)
                    .is_some_and(|ids| !ids.contains(&uuid)),
            "The destination result was already present before this transfer",
        )
    }
}
pub(crate) fn validate_record(record: &crate::ResourceOperationRecord) -> Result<(), Error> {
    let crate::ResourceAction::CrossPoolVm(operation) = &record.action else {
        return if record.cross_pool_plan.is_none() {
            Ok(())
        } else {
            Err(Error::Journal)
        };
    };
    let valid = record.target == crate::ResourceTarget::Pool(record.pool)
        && operation.validate(record.pool).is_ok()
        && record.created.is_none()
        && record.transfer.is_none()
        && match &record.cross_pool_plan {
            Some(plan) => plan.validates(record, operation),
            None => {
                matches!(
                    record.state,
                    crate::OperationState::Preparing
                        | crate::OperationState::Rejected
                        | crate::OperationState::Cancelled
                        | crate::OperationState::OutcomeUnknown
                        | crate::OperationState::AcknowledgedUnknown
                ) && record.steps.is_empty()
                    && record.task.is_none()
                    && record.result_reference.is_none()
                    && record.transfer_created == TransferCreated::default()
            }
        };
    if valid { Ok(()) } else { Err(Error::Journal) }
}
fn require(value: bool, message: &'static str) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(Error::Protocol(message))
    }
}
/// Something the operator can put right before a transfer. It is said as it is: the host
/// answered correctly, so this is not an invalid response.
fn unmet(value: bool, message: &'static str) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(Error::MigrationPrerequisite(message))
    }
}
/// Something that no longer is as it was reviewed: the review is stale, the answer is not
/// wrong, and the operator is asked to review again.
fn unchanged(value: bool) -> Result<(), Error> {
    if value {
        Ok(())
    } else {
        Err(Error::MigrationConflict)
    }
}
fn shape(record: &Value) -> Result<Shape, Error> {
    let text = |key: &str, limit: usize| {
        record[key]
            .as_str()
            .filter(|v| v.len() <= limit && !v.contains('\0'))
            .map(str::to_owned)
            .ok_or(Error::MigrationConflict)
    };
    let parameter = |map: &str, key: &str, default: &str| {
        record[map]
            .as_object()
            .ok_or(Error::MigrationConflict)?
            .get(key)
            .map_or(Ok(default.into()), |v| {
                v.as_str()
                    .filter(|v| v.len() <= 256)
                    .map(str::to_owned)
                    .ok_or(Error::MigrationConflict)
            })
    };
    unmet(
        VmRecord::new(record).hvm_boot_policy() == "BIOS order",
        "This cross-pool workflow supports ordinary HVM guests",
    )?;
    Ok(Shape {
        name: text("name_label", 1024)?,
        description: text("name_description", 16384)?,
        cpus: [
            m::number(record, "VCPUs_max")?,
            m::number(record, "VCPUs_at_startup")?,
        ],
        memory: [
            m::number(record, "memory_static_min")?,
            m::number(record, "memory_static_max")?,
            m::number(record, "memory_dynamic_min")?,
            m::number(record, "memory_dynamic_max")?,
        ],
        firmware: parameter("HVM_boot_params", "firmware", "bios")?,
        boot_order: parameter("HVM_boot_params", "order", "")?,
        secureboot: parameter("platform", "secureboot", "false")?,
    })
}
pub(crate) async fn pool(session: &Session, expected: PoolId) -> Result<Value, Error> {
    let rows = crate::security_discovery::records(session, "pool", 1).await?;
    let row = rows.values().next().ok_or(Error::MigrationConflict)?;
    unchanged(rows.len() == 1 && PoolRecord::new(row).uuid() == expected.to_string())?;
    m::idle(row)?;
    Ok(row.clone())
}
async fn source(
    session: &Session,
    operation: &CrossPoolVmOperation,
) -> Result<(String, Value, m::Graph, Shape), Error> {
    let (reference, record) = m::fresh(session, "VM", operation.vm.uuid()).await?;
    unmet(
        VmRecord::new(&record).is_a_template() == false
            && VmRecord::new(&record).is_a_snapshot() == false
            && VmRecord::new(&record).is_control_domain() == false,
        "Choose an ordinary VM; templates, snapshots and control domains cannot use this transfer",
    )?;
    // A guest that is no longer as it was reviewed is a stale review, and is said to be:
    // the host answered correctly, so this is not an invalid response.
    if VmRecord::new(&record).power_state()
        != if operation.mode.live() {
            vm_power_state::RUNNING
        } else {
            vm_power_state::HALTED
        }
    {
        return Err(Error::MigrationConflict);
    }
    m::idle(&record)?;
    unmet(
        crate::xapi::advertises(&record, vm_operations::MIGRATE_SEND),
        "The host does not currently allow cross-pool migration",
    )?;
    if m::nullable_host(session, VmRecord::new(&record).resident_on()).await?
        != operation.expected_host
        || m::nullable_host(session, VmRecord::new(&record).affinity()).await?
            != operation.expected_affinity
    {
        return Err(Error::MigrationConflict);
    }
    for field in ["snapshots", "VGPUs", "attached_PCIs", "VUSBs"] {
        unmet(
            m::refs(&record, field)?.is_empty(),
            "Snapshot chains or specialized guest devices require another workflow",
        )?;
    }
    if record.get("VTPMs").is_some() {
        unmet(
            m::refs(&record, "VTPMs")?.is_empty(),
            "vTPM guests are outside this transfer workflow",
        )?;
    }
    for field in [
        "suspend_VDI",
        "appliance",
        "protection_policy",
        "snapshot_schedule",
    ] {
        unmet(
            record[field] == "OpaqueRef:NULL",
            "Suspension, vApp or snapshot schedule membership requires a separate transfer",
        )?;
    }
    unmet(
        VmRecord::new(&record).ha_always_run() == false
            && VmRecord::new(&record).ha_restart_priority() == "",
        "Disable VM HA protection separately before transfer",
    )?;
    let graph = m::graph(session, &reference, &record).await?;
    require(
        graph.disks.iter().all(|b| !b.id.uuid().is_nil())
            && graph.nics.iter().all(|n| !n.id.uuid().is_nil()),
        "The source reports invalid device identities",
    )?;
    let disks: BTreeMap<_, _> = graph
        .disks
        .iter()
        .filter_map(|a| a.disk.as_ref().map(|d| (d.id, d.sr)))
        .collect();
    unchanged(
        disks
            == operation
                .disks
                .iter()
                .map(|d| (d.vdi, d.expected_sr))
                .collect()
            && graph
                .nics
                .iter()
                .map(|n| (n.id, n.network))
                .collect::<BTreeMap<_, _>>()
                == operation
                    .networks
                    .iter()
                    .map(|n| (n.vif, n.expected_network))
                    .collect(),
    )?;
    require(
        graph
            .disks
            .iter()
            .map(|a| &a.device)
            .collect::<BTreeSet<_>>()
            .len()
            == graph.disks.len()
            && graph
                .nics
                .iter()
                .map(|a| &a.device)
                .collect::<BTreeSet<_>>()
                .len()
                == graph.nics.len(),
        "Guest device slots are ambiguous",
    )?;
    for attachment in &graph.disks {
        if attachment.kind == vbd_type::CD {
            unmet(
                attachment.disk.is_none(),
                "Eject installation media separately before transfer",
            )?;
            continue;
        }
        unmet(
            attachment.owner && attachment.mode == vbd_mode::RW,
            "Only owned writable data disks can be transferred",
        )?;
        let disk = attachment.disk.as_ref().ok_or(Error::MigrationConflict)?;
        let (_, record) = m::fresh(session, "VDI", disk.id.uuid()).await?;
        unmet(
            disk.kind == vdi_type::USER
                && VdiRecord::new(&record).read_only() == false
                && VdiRecord::new(&record).sharable() == false
                && VdiRecord::new(&record).is_a_snapshot() == false
                && VdiRecord::new(&record).cbt_enabled() == false
                && VdiRecord::new(&record).on_boot() == on_boot::PERSIST,
            "Shared, snapshot, CBT or reset-on-boot disks cannot use this transfer",
        )?;
        let consumers = m::refs(&record, "VBDs")?;
        unmet(consumers.len() == 1, "A data disk has other consumers")?;
        let block = m::read(session, "VBD", &consumers[0]).await?;
        unchanged(
            m::id::<crate::VbdId>(&block)? == attachment.id
                && VbdRecord::new(&block).vm() == reference,
        )?;
    }
    let shape = shape(&record)?;
    Ok((reference, record, graph, shape))
}
async fn baseline(session: &Session) -> Result<BTreeMap<String, BTreeSet<Uuid>>, Error> {
    let mut result = BTreeMap::new();
    for class in ["VM", "VDI", "VBD", "VIF"] {
        result.insert(
            class.into(),
            crate::device_lifecycle::baseline(session, class)
                .await?
                .into_iter()
                .collect(),
        );
    }
    Ok(result)
}
pub(crate) struct Prepared {
    pub vm: String,
    pub host: String,
    pub network: String,
    pub coordinator_address: String,
    pub disks: Value,
    pub nics: Value,
}
/// Each destination repository must report at least the free space asked of it.
async fn capacity(
    destination: &Session,
    required: &BTreeMap<crate::SrId, u64>,
    refusal: &'static str,
) -> Result<(), Error> {
    let whole = |field: xenadmin_xapi::WholeField| {
        field
            .number()
            .and_then(|n| u64::try_from(n).ok())
            .ok_or(Error::MigrationConflict)
    };
    for (sr, size) in required {
        let (_, record) = m::fresh(destination, "SR", sr.uuid()).await?;
        let view = SrRecord::new(&record);
        unmet(
            whole(view.physical_size())?.saturating_sub(whole(view.physical_utilisation())?)
                >= *size,
            refusal,
        )?;
    }
    Ok(())
}
/// The checks a batch of moves makes before its first transfer: the single transfer's own
/// preparation for every guest, which reads both pools and prepares no receiver, and then
/// each destination repository's free space against the disks of all the guests mapped to
/// it, where a single transfer asks only for its own.
pub(crate) async fn preflight_selection(
    source_session: &Session,
    destination: &Session,
    source_pool: PoolId,
    operations: &[&CrossPoolVmOperation],
) -> Result<(), Error> {
    let mut required = BTreeMap::<crate::SrId, u64>::new();
    for operation in operations {
        let (plan, _) = prepare(source_session, destination, source_pool, operation).await?;
        for mapping in &operation.disks {
            let disk = plan
                .graph
                .disks
                .iter()
                .filter_map(|a| a.disk.as_ref())
                .find(|d| d.id == mapping.vdi)
                .ok_or(Error::MigrationConflict)?;
            let size = required.entry(mapping.destination_sr).or_default();
            *size = size
                .checked_add(disk.size)
                .ok_or(Error::MigrationConflict)?;
        }
    }
    capacity(
        destination,
        &required,
        "Destination storage does not report enough space for the disks of the whole selection",
    )
    .await
}
async fn destinations(
    source_session: &Session,
    destination: &Session,
    plan: &Plan,
) -> Result<Prepared, Error> {
    let operation = &plan.operation;
    let pool = pool(destination, operation.destination_pool).await?;
    let coordinator = m::read(
        destination,
        "host",
        &snapshots::reference(PoolRecord::new(&pool).master())?,
    )
    .await?;
    let coordinator_address = m::text(&coordinator, "address")?;
    unmet(
        crate::cross_pool::usable_address(&coordinator_address),
        "Destination coordinator address is unavailable",
    )?;
    let host = m::host(destination, operation.destination_host).await?;
    let network = m::network_visible(destination, operation.transfer_network, &host, false).await?;
    let network_record = m::read(destination, "network", &network).await?;
    let mut transfer_address_matches = false;
    for pif in m::refs(&network_record, "PIFs")? {
        let pif = m::read(destination, "PIF", &pif).await?;
        let address_matches =
            if PifRecord::new(&pif).primary_address_type() == primary_address_type::IPV6 {
                PifRecord::new(&pif).ipv6().members().is_some_and(|ips| {
                    ips.iter()
                        .filter_map(Value::as_str)
                        .any(|ip| ip.split('/').next() == Some(operation.transfer_address.as_str()))
                })
            } else {
                PifRecord::new(&pif).ip() == operation.transfer_address
            };
        transfer_address_matches |= PifRecord::new(&pif).host() == host
            && PifRecord::new(&pif).currently_attached() == true
            && address_matches;
    }
    unchanged(transfer_address_matches)?;
    let mut disks = serde_json::Map::new();
    let mut required = BTreeMap::<crate::SrId, u64>::new();
    for mapping in &operation.disks {
        let disk = plan
            .graph
            .disks
            .iter()
            .filter_map(|a| a.disk.as_ref())
            .find(|d| d.id == mapping.vdi)
            .ok_or(Error::MigrationConflict)?;
        let size = required.entry(mapping.destination_sr).or_default();
        *size = size
            .checked_add(disk.size)
            .ok_or(Error::MigrationConflict)?;
        let (reference, _) = m::fresh(source_session, "VDI", mapping.vdi.uuid()).await?;
        let sr = m::sr_visible(destination, mapping.destination_sr, &host, true).await?;
        disks.insert(reference, json!(sr));
    }
    capacity(
        destination,
        &required,
        "Destination storage does not report enough space for the reviewed disks",
    )
    .await?;
    let mut nics = serde_json::Map::new();
    for mapping in &operation.networks {
        let (reference, _) = m::fresh(source_session, "VIF", mapping.vif.uuid()).await?;
        let network =
            m::network_visible(destination, mapping.destination_network, &host, false).await?;
        nics.insert(reference, json!(network));
    }
    let (vm, _, graph, shape) = source(source_session, operation).await?;
    unchanged(graph == plan.graph && shape == plan.shape)?;
    Ok(Prepared {
        vm,
        host,
        network,
        coordinator_address,
        disks: Value::Object(disks),
        nics: Value::Object(nics),
    })
}
pub(crate) async fn prepare(
    source_session: &Session,
    destination: &Session,
    source_pool: PoolId,
    operation: &CrossPoolVmOperation,
) -> Result<(Plan, Prepared), Error> {
    operation.validate(source_pool)?;
    pool(source_session, source_pool).await?;
    let (_, _, graph, shape) = source(source_session, operation).await?;
    let mut plan = Plan {
        source_pool,
        operation: operation.clone(),
        graph,
        shape,
        baseline: BTreeMap::new(),
        task_uuid: None,
        source_tasks: None,
        expires_at_ms: now().saturating_add(2 * 60 * 60 * 1000),
        completed: false,
        ended: false,
        destination_vm: None,
    };
    let prepared = destinations(source_session, destination, &plan).await?;
    plan.baseline = baseline(destination).await?;
    collision_check(&plan)?;
    Ok((plan, prepared))
}
pub(crate) fn now() -> u64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .unwrap_or_default()
        .as_millis() as u64
}
fn collision_check(plan: &Plan) -> Result<(), Error> {
    if !plan.operation.mode.copy() {
        plan.fresh("VM", plan.operation.vm.uuid())?;
        for block in &plan.graph.disks {
            plan.fresh("VBD", block.id.uuid())?;
        }
        for nic in &plan.graph.nics {
            plan.fresh("VIF", nic.id.uuid())?;
        }
    }
    Ok(())
}
pub(crate) async fn revalidate(
    source: &Session,
    destination: &Session,
    plan: &Plan,
) -> Result<Prepared, Error> {
    pool(source, plan.source_pool).await?;
    let current = baseline(destination).await?;
    // Only arrivals since the review can be mistaken for this transfer's result; records
    // the destination released meanwhile (a storage scan after an earlier move) are not
    // a reason to send the operator back to the review.
    let arrivals = current.iter().any(|(class, ids)| {
        ids.iter().any(|id| {
            !plan
                .baseline
                .get(class)
                .is_some_and(|known| known.contains(id))
        })
    });
    unchanged(!arrivals)?;
    // Fresh collision checks immediately before dispatch also cover arrivals since review.
    let mut latest = plan.clone();
    latest.baseline = current;
    collision_check(&latest)?;
    destinations(source, destination, plan).await
}

/// This map contains host-created credentials and must never reach a journal or log.
pub(crate) fn receiver(value: Value, prepared: &Prepared, address: &str) -> Result<Value, Error> {
    let fields = value.as_object().ok_or(Error::MigrationConflict)?;
    require(
        fields.len() == 5 && fields.get("host") == Some(&json!(&prepared.host)),
        "Unexpected migration receiver identity or fields",
    )?;
    let token = fields
        .get("session_id")
        .and_then(Value::as_str)
        .filter(|s| {
            s.starts_with("OpaqueRef:")
                && *s != "OpaqueRef:NULL"
                && s.len() <= 512
                && s.bytes()
                    .all(|b| b.is_ascii_graphic() && !b";&?#".contains(&b))
        })
        .ok_or(Error::MigrationConflict)?;
    for (key, host, path) in [
        ("SM", address, "/services/SM"),
        ("xenops", address, "/services/xenops"),
        ("master", prepared.coordinator_address.as_str(), "/"),
    ] {
        let raw = fields
            .get(key)
            .and_then(Value::as_str)
            .filter(|s| s.len() <= 4096)
            .ok_or(Error::MigrationConflict)?;
        let url = url::Url::parse(raw).map_err(|_| Error::MigrationConflict)?;
        require(
            matches!(url.scheme(), "http" | "https")
                && url.username().is_empty()
                && url.password().is_none()
                && url.fragment().is_none()
                && url.host_str().is_some_and(|s| {
                    s.trim_matches(['[', ']']).parse::<std::net::IpAddr>().ok() == host.parse().ok()
                })
                && url.port().is_none()
                && url.path() == path,
            "Migration receiver endpoint differs from the reviewed destination",
        )?;
        let query: Vec<_> = url.query_pairs().collect();
        require(
            if key == "master" {
                query.is_empty()
            } else {
                query.len() == 1 && query[0].0 == "session_id" && query[0].1 == token
            },
            "Migration receiver credential fields are inconsistent",
        )?;
    }
    Ok(value)
}
pub(crate) fn arguments(plan: &Plan, prepared: &Prepared, receiver: Value) -> Vec<Value> {
    vec![
        json!(&prepared.vm),
        receiver,
        json!(plan.operation.mode.live()),
        prepared.disks.clone(),
        prepared.nics.clone(),
        if plan.operation.mode.copy() {
            json!({"copy":"true"})
        } else {
            json!({})
        },
    ]
}

pub(crate) async fn identify(
    destination: &Session,
    plan: &Plan,
    reference: &str,
) -> Result<VmId, Error> {
    pool(destination, plan.operation.destination_pool).await?;
    let record = m::read(destination, "VM", reference).await?;
    let id: VmId = m::id(&record)?;
    plan.fresh("VM", id.uuid())?;
    require(
        VmRecord::new(&record).is_control_domain() == false
            && VmRecord::new(&record).is_a_snapshot() == false
            && VmRecord::new(&record).is_a_template() == false,
        "Migration task returned an unexpected guest class",
    )?;
    require(
        if plan.operation.mode.copy() {
            id != plan.operation.vm
        } else {
            id == plan.operation.vm
        },
        "Migration result identity does not match the reviewed move or copy",
    )?;
    Ok(id)
}
pub(crate) async fn verify_destination(
    destination: &Session,
    plan: &Plan,
    reference: &str,
) -> Result<TransferCreated, Error> {
    require(
        plan.completed,
        "Migration completion receipt is missing; no transfer will be replayed",
    )?;
    let vm = identify(destination, plan, reference).await?;
    let record = m::read(destination, "VM", reference).await?;
    m::idle(&record)?;
    require(
        shape(&record)? == plan.shape
            && VmRecord::new(&record).power_state()
                == if plan.operation.mode.live() {
                    vm_power_state::RUNNING
                } else {
                    vm_power_state::HALTED
                },
        "The destination guest configuration or state is not yet verified",
    )?;
    require(
        m::nullable_host(destination, VmRecord::new(&record).resident_on()).await?
            == if plan.operation.mode.live() {
                Some(plan.operation.destination_host)
            } else {
                None
            },
        "Destination resident host does not match the reviewed placement",
    )?;
    let graph = m::graph(destination, reference, &record).await?;
    require(
        graph.disks.len() == plan.graph.disks.len() && graph.nics.len() == plan.graph.nics.len(),
        "Destination device membership differs from the reviewed source",
    )?;
    let mut created = TransferCreated {
        vms: vec![vm],
        ..Default::default()
    };
    for old in &plan.graph.disks {
        let new = graph
            .disks
            .iter()
            .find(|v| v.device == old.device)
            .ok_or(Error::MigrationConflict)?;
        plan.fresh("VBD", new.id.uuid())?;
        require(
            if plan.operation.mode.copy() {
                new.id != old.id
            } else {
                new.id == old.id
            },
            "Destination disk attachment identity is unexpected",
        )?;
        require(
            new.mode == old.mode
                && new.kind == old.kind
                && new.owner == old.owner
                && new.bootable == old.bootable,
            "Destination disk attachment settings differ",
        )?;
        match (&old.disk, &new.disk) {
            (None, None) => {}
            (Some(old), Some(new)) => {
                let mapping = plan
                    .operation
                    .disks
                    .iter()
                    .find(|d| d.vdi == old.id)
                    .ok_or(Error::MigrationConflict)?;
                plan.fresh("VDI", new.id.uuid())?;
                require(
                    new.size == old.size
                        && new.kind == old.kind
                        && new.sr == mapping.destination_sr,
                    "Destination disk contents or placement cannot be identified",
                )?;
                let (_, disk) = m::fresh(destination, "VDI", new.id.uuid()).await?;
                let consumers = m::refs(&disk, "VBDs")?;
                require(
                    consumers.len() == 1
                        && VdiRecord::new(&disk).read_only() == false
                        && VdiRecord::new(&disk).sharable() == false,
                    "Destination disk has unexpected consumers or access",
                )?;
                let block = m::read(destination, "VBD", &consumers[0]).await?;
                require(
                    VbdRecord::new(&block).vm() == reference,
                    "Destination disk belongs to another guest",
                )?;
                created.disks.push(new.id);
            }
            _ => return Err(Error::MigrationConflict),
        }
        created.vbds.push(new.id);
    }
    for old in &plan.graph.nics {
        let new = graph
            .nics
            .iter()
            .find(|v| v.device == old.device)
            .ok_or(Error::MigrationConflict)?;
        plan.fresh("VIF", new.id.uuid())?;
        let mapping = plan
            .operation
            .networks
            .iter()
            .find(|n| n.vif == old.id)
            .ok_or(Error::MigrationConflict)?;
        require(
            new.network == mapping.destination_network
                && if plan.operation.mode.copy() {
                    new.id != old.id && new.mac != old.mac
                } else {
                    new.id == old.id && new.mac == old.mac
                },
            "Destination NIC identity, MAC or network differs from the reviewed transfer",
        )?;
        created.vifs.push(new.id);
    }
    Ok(created)
}

/// What a move left behind on the source pool once its VM and attachments are gone.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum SourceDisposition {
    Clean,
    /// Source disk records XAPI has not yet garbage-collected; their count.
    DisksPending(usize),
}

pub(crate) async fn verify_source(
    source_session: &Session,
    plan: &Plan,
) -> Result<SourceDisposition, Error> {
    pool(source_session, plan.source_pool).await?;
    if plan.operation.mode.copy() {
        let (_, _, current, shape) = source(source_session, &plan.operation).await?;
        require(
            current == plan.graph && shape == plan.shape,
            "Source guest changed during copy; destination exists but source preservation is unverified",
        )?;
        return Ok(SourceDisposition::Clean);
    }
    require(
        snapshots::optional_record(source_session, "VM", &plan.operation.vm.to_string())
            .await?
            .is_none(),
        "Destination exists but source VM removal is not yet verified",
    )?;
    for (class, id) in plan
        .graph
        .disks
        .iter()
        .map(|b| ("VBD", b.id.uuid()))
        .chain(plan.graph.nics.iter().map(|n| ("VIF", n.id.uuid())))
    {
        require(
            snapshots::optional_record(source_session, class, &id.to_string())
                .await?
                .is_none(),
            "Destination exists but source device cleanup is not yet verified",
        )?;
    }
    // Source disks are released by the storage manager's own scan after the move, which
    // can lag the migration task by a while; they are reported rather than treated as doubt.
    let mut pending = 0;
    for disk in plan.graph.disks.iter().filter_map(|b| b.disk.as_ref()) {
        if snapshots::optional_record(source_session, "VDI", &disk.id.to_string())
            .await?
            .is_some()
        {
            pending += 1;
        }
    }
    Ok(if pending == 0 {
        SourceDisposition::Clean
    } else {
        SourceDisposition::DisksPending(pending)
    })
}

#[cfg(test)]
mod tests {
    use super::*;
    fn prepared() -> Prepared {
        Prepared {
            vm: "OpaqueRef:vm".into(),
            host: "OpaqueRef:host".into(),
            network: "OpaqueRef:network".into(),
            coordinator_address: "192.0.2.20".into(),
            disks: json!({}),
            nics: json!({}),
        }
    }
    fn advertised(scheme: &str) -> Value {
        json!({"host":"OpaqueRef:host","SM":format!("{scheme}://192.0.2.30/services/SM?session_id=OpaqueRef:receiver"),"xenops":format!("{scheme}://192.0.2.30/services/xenops?session_id=OpaqueRef:receiver"),"master":format!("{scheme}://192.0.2.20/"),"session_id":"OpaqueRef:receiver"})
    }
    #[test]
    fn receiver_preserves_host_transport_policy_and_rejects_endpoint_or_credential_changes() {
        for scheme in ["http", "https"] {
            let value = advertised(scheme);
            assert_eq!(
                receiver(value.clone(), &prepared(), "192.0.2.30").unwrap(),
                value
            );
        }
        for (key, value) in [
            (
                "SM",
                json!("https://198.51.100.99/services/SM?session_id=OpaqueRef:receiver"),
            ),
            (
                "SM",
                json!("https://192.0.2.30:8443/services/SM?session_id=OpaqueRef:receiver"),
            ),
            (
                "SM",
                json!("https://user@192.0.2.30/services/SM?session_id=OpaqueRef:receiver"),
            ),
            (
                "SM",
                json!("https://192.0.2.30/services/SM?session_id=OpaqueRef:other"),
            ),
            (
                "SM",
                json!("https://192.0.2.30/services/SM?session_id=OpaqueRef:receiver&x=1"),
            ),
            (
                "SM",
                json!("https://192.0.2.30/services/SM?session_id=OpaqueRef:receiver#fragment"),
            ),
            (
                "master",
                json!("http://192.0.2.20/?session_id=OpaqueRef:receiver"),
            ),
            ("master", json!("http://192.0.2.30/")),
            ("host", json!("OpaqueRef:other")),
            ("session_id", json!("OpaqueRef:NULL")),
            ("session_id", json!("OpaqueRef:a&b")),
            ("extra", json!("unexpected")),
        ] {
            let mut record = advertised("http");
            record[key] = value;
            assert!(
                receiver(record, &prepared(), "192.0.2.30").is_err(),
                "{key}"
            );
        }
        for address in [
            "0.0.0.0",
            "127.0.0.1",
            "255.255.255.255",
            "224.0.0.1",
            "169.254.0.1",
            "::1",
            "::",
            "fe80::1",
            "ff02::1",
            "example.test",
        ] {
            assert!(!crate::cross_pool::usable_address(address), "{address}");
        }
    }
}
