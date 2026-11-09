use crate::{
    CreatedDevice, DeviceOperation, Error, GuestFirmware, OperationId, SnapshotId, SrId, VbdId,
    VdiId, VifId, VmCopyMode, VmId, VmLifecycleOperation, device_lifecycle as devices, snapshots,
    xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use std::collections::BTreeMap;
use uuid::Uuid;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Change {
    Name(String),
    Description(String),
    Template(bool),
    Memory(u64),
    VcpuMax(u64),
    VcpuStartup(u64),
    Firmware(GuestFirmware),
    SecureBoot(bool),
    RemoveProvisionDisks,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Step {
    CopyBios {
        vm: VmId,
        host: crate::HostId,
    },
    Copy {
        mode: VmCopyMode,
        name: String,
    },
    Set {
        vm: VmId,
        change: Change,
    },
    Provision {
        vm: VmId,
    },
    /// Asks the host to recompute the VM's allowed operations after a template became a VM:
    /// XCP-ng 8.3 keeps the template's list until asked, which withholds Start.
    RefreshAllowedOperations {
        vm: VmId,
    },
    ConfigureDevices {
        vm: VmId,
    },
    CheckHosts {
        vm: VmId,
    },
    Device {
        vm: VmId,
        group: usize,
        index: usize,
    },
    InstallIso {
        vm: VmId,
        image: VdiId,
    },
    EjectIso {
        vm: VmId,
        vbd: VbdId,
        image: VdiId,
    },
    ClaimDisk {
        vm: VmId,
        vbd: VbdId,
        disk: VdiId,
    },
    DestroySnapshot {
        snapshot: SnapshotId,
        disks: Vec<VdiId>,
    },
    DestroyVm,
    DestroyDisk {
        disk: VdiId,
        snapshot: bool,
    },
}
impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Copy {
                mode: VmCopyMode::Fast,
                ..
            } => "Clone source",
            Self::Copy { .. } => "Copy source disks and VM",
            Self::Set { change, .. } => match change {
                Change::Name(_) => "Set name",
                Change::Description(_) => "Set description",
                Change::Template(true) => "Convert to template",
                Change::Template(false) => "Convert to VM",
                Change::Memory(_) => "Set memory",
                Change::VcpuMax(_) => "Set vCPU maximum",
                Change::VcpuStartup(_) => "Set startup vCPUs",
                Change::Firmware(_) => "Set guest firmware",
                Change::SecureBoot(_) => "Set Secure Boot policy",
                Change::RemoveProvisionDisks => "Use explicitly reviewed disks",
            },
            Self::CopyBios { .. } => "Copy host BIOS identity and set home host",
            Self::Provision { .. } => "Provision VM",
            Self::RefreshAllowedOperations { .. } => "Refresh allowed operations",
            Self::ConfigureDevices { .. } => "Prepare VM devices",
            Self::CheckHosts { .. } => "Check compatible hosts",
            Self::Device { .. } => "Configure device",
            Self::InstallIso { .. } => "Insert installation ISO",
            Self::EjectIso { .. } => "Eject inherited ISO",
            Self::ClaimDisk { .. } => "Record ownership of newly copied disk",
            Self::DestroySnapshot { .. } => "Delete selected snapshot record",
            Self::DestroyVm => "Delete VM or template record",
            Self::DestroyDisk { .. } => "Delete selected disk data",
        }
    }
    pub fn asynchronous(&self) -> bool {
        matches!(self, Self::Copy { .. } | Self::Provision { .. })
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Evidence {
    pub task: Option<String>,
    pub result: Option<String>,
    pub completed: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ExpectedDisk {
    pub size: u64,
    pub sr: SrId,
    pub mode: String,
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Graph {
    pub disks: BTreeMap<String, ExpectedDisk>,
    pub cds: BTreeMap<String, Option<VdiId>>,
    pub nics: BTreeMap<String, crate::NetworkId>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Plan {
    #[serde(default)]
    pub bios_identity: Option<serde_json::Map<String, Value>>,
    pub source_graph: Graph,
    pub metadata: Option<(String, String)>,
    pub source: VmId,
    pub operation: VmLifecycleOperation,
    pub steps: Vec<Step>,
    pub evidence: Vec<Evidence>,
    pub created: Option<VmId>,
    pub baseline: BTreeMap<String, Vec<Uuid>>,
    pub created_devices: Vec<CreatedDevice>,
    pub groups: Vec<devices::Plan>,
    pub owned_disks: Vec<VdiId>,
    pub workflow_planned: bool,
}
impl Plan {
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.source == old.source
            && self.operation == old.operation
            && (old.bios_identity.is_none() || self.bios_identity == old.bios_identity)
            && (old.baseline.is_empty()
                || self.baseline == old.baseline && self.source_graph == old.source_graph)
            && self.steps.starts_with(&old.steps)
            && self.evidence.len() == self.steps.len()
            && self.evidence.iter().zip(&old.evidence).all(|(new, old)| {
                (old.task.is_none() || old.task == new.task)
                    && (old.result.is_none() || old.result == new.result)
                    && (!old.completed || new.completed)
            })
            && (old.created.is_none() || self.created == old.created)
            && self.created_devices.starts_with(&old.created_devices)
            && (!old.workflow_planned || self.workflow_planned)
    }
    fn push(&mut self, step: Step) {
        self.steps.push(step);
        self.evidence.push(Evidence::default());
    }
}
fn number(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
fn id<T: From<Uuid>>(r: &Value) -> Result<T, Error> {
    r["uuid"]
        .as_str()
        .and_then(|v| v.parse::<Uuid>().ok())
        .map(T::from)
        .ok_or(Error::VmLifecycleConflict)
}
fn reference(v: &Value) -> Result<String, Error> {
    snapshots::reference(v)
}
fn refs(r: &Value, key: &str) -> Result<Vec<String>, Error> {
    r[key]
        .as_array()
        .filter(|v| v.len() <= 256)
        .ok_or(Error::VmLifecycleConflict)?
        .iter()
        .map(reference)
        .collect()
}
fn idle(r: &Value) -> Result<(), Error> {
    if r["current_operations"]
        .as_object()
        .is_none_or(|m| !m.is_empty())
    {
        Err(Error::Busy)
    } else {
        Ok(())
    }
}
fn permits(r: &Value, cap: &str) -> Result<(), Error> {
    idle(r)?;
    if crate::xapi::advertises(r, cap) {
        Ok(())
    } else {
        Err(Error::UnavailableAction)
    }
}
/// XCP-ng 8.3.0 never advertises `changing_memory_limits` for a halted guest; it advertises
/// the static and dynamic range operations that the combined limits setter changes.
fn permits_memory_limits(r: &Value) -> Result<(), Error> {
    idle(r)?;
    if crate::xapi::advertises(r, "changing_memory_limits")
        || (crate::xapi::advertises(r, "changing_static_range")
            && crate::xapi::advertises(r, "changing_dynamic_range"))
    {
        Ok(())
    } else {
        Err(Error::UnavailableAction)
    }
}
/// XCP-ng 8.3.0 advertises no vCPU operation for a halted guest although its setters succeed.
/// A halted, idle guest reaches the server, whose explicit refusal stays authoritative; a
/// running guest still needs the advertised capability.
fn permits_vcpu_change(r: &Value) -> Result<(), Error> {
    idle(r)?;
    if crate::xapi::advertises(r, "changing_VCPUs") || r["power_state"] == "Halted" {
        Ok(())
    } else {
        Err(Error::UnavailableAction)
    }
}
#[cfg(test)]
pub(crate) fn permits_memory_limits_for_test(r: &Value) -> Result<(), Error> {
    permits_memory_limits(r)
}
#[cfg(test)]
pub(crate) fn permits_vcpu_change_for_test(r: &Value) -> Result<(), Error> {
    permits_vcpu_change(r)
}
async fn fresh(session: &Session, class: &str, id: Uuid) -> Result<(String, Value), Error> {
    snapshots::optional_record(session, class, &id.to_string())
        .await?
        .ok_or(Error::VmLifecycleConflict)
}
fn ordinary(r: &Value) -> bool {
    r["is_a_snapshot"] == false
        && r["is_control_domain"] == false
        && r["is_a_template"].is_boolean()
}
async fn source(session: &Session, plan: &Plan) -> Result<(String, Value), Error> {
    let (reference, r) = fresh(session, "VM", plan.source.uuid()).await?;
    if !ordinary(&r)
        || r["is_a_template"] != plan.operation.template()
        || r["power_state"] != "Halted"
    {
        return Err(Error::VmLifecycleConflict);
    }
    idle(&r)?;
    if matches!(
        plan.operation,
        VmLifecycleOperation::EditTemplate { .. } | VmLifecycleOperation::Delete { .. }
    ) && (r["is_a_template"] == true && !crate::guest_boot::custom_template(&r)
        || r["other_config"]["default_template"] == "true"
        || r["is_default_template"] == true)
    {
        return Err(Error::UnavailableAction);
    }
    if let Some((name, description)) = &plan.metadata
        && (r["name_label"] != *name || r["name_description"] != *description)
    {
        return Err(Error::VmLifecycleConflict);
    }
    Ok((reference, r))
}
async fn target(session: &Session, plan: &Plan, vm: VmId) -> Result<(String, Value), Error> {
    if vm == plan.source {
        return source(session, plan).await;
    }
    if plan.created != Some(vm)
        || plan
            .baseline
            .get("VM")
            .is_none_or(|ids| ids.contains(&vm.uuid()))
    {
        return Err(Error::VmLifecycleConflict);
    }
    let (reference, r) = fresh(session, "VM", vm.uuid()).await?;
    if !ordinary(&r) || r["power_state"] != "Halted" {
        return Err(Error::VmLifecycleConflict);
    }
    idle(&r)?;
    Ok((reference, r))
}
async fn sr(session: &Session, sr: SrId) -> Result<String, Error> {
    let (reference, r) = fresh(session, "SR", sr.uuid()).await?;
    if r["content_type"] != "user" {
        return Err(Error::UnavailableAction);
    }
    permits(&r, "vdi_create")?;
    Ok(reference)
}
/// An installation image is a read-only, present disk of type `iso` or any read-only disk
/// in an ISO library: XCP-ng 8.3.0 records the images of its ISO repositories (including
/// the host's tools image) with type `user`, so the repository's content type decides.
async fn require_iso_image(session: &Session, image: &Value) -> Result<(), Error> {
    if image["read_only"] != true || image["missing"] != false {
        return Err(Error::UnavailableAction);
    }
    if image["type"] == "iso" {
        return Ok(());
    }
    let sr = session
        .call("SR.get_record", vec![json!(reference(&image["SR"])?)])
        .await?;
    if sr["content_type"] == "iso" {
        Ok(())
    } else {
        Err(Error::UnavailableAction)
    }
}
async fn validate_create_choices(
    session: &Session,
    operation: &VmLifecycleOperation,
) -> Result<(), Error> {
    if let VmLifecycleOperation::Create {
        sr: chosen_sr,
        network,
        iso,
        firmware,
        ..
    } = operation
    {
        sr(session, *chosen_sr).await?;
        let (_, network) = fresh(session, "network", network.uuid()).await?;
        idle(&network)?;
        if let Some(image) = iso {
            let (_, image) = fresh(session, "VDI", image.uuid()).await?;
            require_iso_image(session, &image).await?;
        }
        let _ = firmware;
    }
    Ok(())
}
pub(crate) async fn plan(
    session: &Session,
    source_id: VmId,
    operation: &VmLifecycleOperation,
) -> Result<Plan, Error> {
    operation.validate()?;
    let mut plan = Plan {
        bios_identity: None,
        source_graph: Graph::default(),
        metadata: None,
        source: source_id,
        operation: operation.clone(),
        steps: vec![],
        evidence: vec![],
        created: None,
        baseline: BTreeMap::new(),
        created_devices: vec![],
        groups: vec![],
        owned_disks: vec![],
        workflow_planned: !matches!(
            operation,
            VmLifecycleOperation::Create { .. } | VmLifecycleOperation::Clone { .. }
        ),
    };
    let (source_reference, record) = source(session, &plan).await?;
    if matches!(
        operation,
        VmLifecycleOperation::Create { .. } | VmLifecycleOperation::Clone { .. }
    ) {
        validate_source_devices(session, &record).await?;
    }
    if matches!(operation, VmLifecycleOperation::EditTemplate { .. }) {
        plan.metadata = Some((
            record["name_label"]
                .as_str()
                .ok_or(Error::VmLifecycleConflict)?
                .into(),
            record["name_description"]
                .as_str()
                .ok_or(Error::VmLifecycleConflict)?
                .into(),
        ));
    }
    match operation {
        VmLifecycleOperation::Create {
            name,
            firmware,
            bios_host,
            ..
        } => {
            validate_create_choices(session, operation).await?;
            if !matches!(firmware, GuestFirmware::Inherited) && !crate::guest_boot::is_hvm(&record)
            {
                return Err(Error::UnsupportedVmLifecycle);
            }
            firmware_safe(&record, *firmware)?;
            validate_firmware_change(session, &record, *firmware).await?;
            if *firmware == GuestFirmware::SecureUefi {
                crate::guest_boot::require_policy_support(session, &source_reference).await?;
            }
            if let Some(host) = bios_host {
                bios_eligible(&record, *firmware)?;
                let (_, h) = fresh(session, "host", host.uuid()).await?;
                let map = string_map(&h["bios_strings"])?;
                if map.is_empty() {
                    return Err(Error::UnsupportedVmLifecycle);
                }
                plan.bios_identity = Some(map);
            }
            if let VmLifecycleOperation::Create { memory, .. } = operation
                && number(&record["memory_static_min"]).is_none_or(|minimum| minimum > *memory)
            {
                return Err(Error::InvalidVmLifecycle);
            }
            let mut has_disks = false;
            for attachment in refs(&record, "VBDs")? {
                let r = session
                    .call("VBD.get_record", vec![json!(attachment)])
                    .await?;
                has_disks |= r["type"] == "Disk";
            }
            let mode = if has_disks {
                let VmLifecycleOperation::Create { sr, .. } = operation else {
                    unreachable!()
                };
                VmCopyMode::Full { sr: *sr }
            } else {
                VmCopyMode::Fast
            };
            permits(&record, if has_disks { "copy" } else { "clone" })?;
            plan.push(Step::Copy {
                mode,
                name: name.clone(),
            });
        }
        VmLifecycleOperation::Clone { name, mode, .. } => {
            permits(
                &record,
                if matches!(mode, VmCopyMode::Fast) {
                    "clone"
                } else {
                    "copy"
                },
            )?;
            if let VmCopyMode::Full { sr: id } = mode {
                sr(session, *id).await?;
            }
            plan.push(Step::Copy {
                mode: mode.clone(),
                name: name.clone(),
            });
        }
        VmLifecycleOperation::ConvertToTemplate => {
            permits(&record, "make_into_template")?;
            plan.push(Step::Set {
                vm: source_id,
                change: Change::Template(true),
            });
        }
        VmLifecycleOperation::EditTemplate { name, description } => {
            plan.push(Step::Set {
                vm: source_id,
                change: Change::Name(name.clone()),
            });
            plan.push(Step::Set {
                vm: source_id,
                change: Change::Description(description.clone()),
            });
        }
        VmLifecycleOperation::Delete {
            disks,
            snapshots: selections,
            ..
        } => {
            permits(&record, "destroy")?;
            validate_owned_disks(session, source_id, disks).await?;
            plan.owned_disks = disks.clone();
            for selection in selections {
                let snapshot = snapshots::deletion_plan(
                    session,
                    source_id,
                    selection.snapshot,
                    &selection.disks,
                )
                .await?;
                if snapshot.checkpoint {
                    return Err(Error::UnsupportedVmLifecycle);
                }
                plan.push(Step::DestroySnapshot {
                    snapshot: selection.snapshot,
                    disks: selection.disks.clone(),
                });
                for disk in &selection.disks {
                    plan.push(Step::DestroyDisk {
                        disk: *disk,
                        snapshot: true,
                    });
                }
            }
            plan.push(Step::DestroyVm);
            for disk in disks {
                plan.push(Step::DestroyDisk {
                    disk: *disk,
                    snapshot: false,
                });
            }
        }
    }
    guard_creation_permissions(session, &plan, &record).await?;
    Ok(plan)
}
async fn guard_creation_permissions(
    session: &Session,
    plan: &Plan,
    record: &Value,
) -> Result<(), Error> {
    let mut methods = plan
        .operation
        .permission_requirements()
        .methods()
        .map(str::to_owned)
        .collect::<Vec<_>>();
    for step in &plan.steps {
        if let Step::Copy { mode, .. } = step {
            methods.push(
                match mode {
                    VmCopyMode::Fast => "VM.clone",
                    VmCopyMode::Full { .. } => "VM.copy",
                }
                .into(),
            );
        }
    }
    if let VmLifecycleOperation::Create { network, iso, .. } = &plan.operation {
        if record["other_config"].get("disks").is_some() {
            methods.push("VM.remove_from_other_config".into());
        }
        let nics = refs(record, "VIFs")?;
        if nics.is_empty() {
            methods.push("VIF.create".into());
        }
        for nic in nics {
            let vif = session.call("VIF.get_record", vec![json!(nic)]).await?;
            let net = session
                .call(
                    "network.get_record",
                    vec![json!(reference(&vif["network"])?)],
                )
                .await?;
            if id::<crate::NetworkId>(&net)? != *network {
                methods.push("VIF.move".into());
            }
        }
        if iso.is_some() {
            match first_cd(session, record).await? {
                None => methods.push("VBD.create".into()),
                Some((_, cd)) if cd["empty"] != true => methods.push("VBD.eject".into()),
                _ => (),
            }
        }
    }
    crate::PermissionRequirements::writes(methods).guard(session.permissions())
}
async fn validate_source_devices(session: &Session, record: &Value) -> Result<(), Error> {
    for field in ["VGPUs", "VUSBs", "VTPMs", "attached_PCIs"] {
        if record[field].as_array().is_some_and(|v| !v.is_empty()) {
            return Err(Error::UnsupportedVmLifecycle);
        }
    }
    for attachment in refs(record, "VBDs")? {
        let r = session
            .call("VBD.get_record", vec![json!(attachment)])
            .await?;
        if r["type"] == "Disk" {
            let disk = session
                .call("VDI.get_record", vec![json!(reference(&r["VDI"])?)])
                .await?;
            if !ordinary_disk(&disk) {
                return Err(Error::UnsupportedVmLifecycle);
            }
        } else if r["type"] != "CD" {
            return Err(Error::UnsupportedVmLifecycle);
        }
    }
    Ok(())
}
fn ordinary_disk(r: &Value) -> bool {
    r["type"] == "user"
        && r["managed"] == true
        && r["missing"] == false
        && r["sharable"] == false
        && r["is_a_snapshot"] == false
}
async fn validate_owned_disks(session: &Session, vm: VmId, disks: &[VdiId]) -> Result<(), Error> {
    let (vm_ref, vm_record) = fresh(session, "VM", vm.uuid()).await?;
    let attachments = refs(&vm_record, "VBDs")?;
    for disk in disks {
        let (disk_ref, record) = fresh(session, "VDI", disk.uuid()).await?;
        idle(&record)?;
        if !ordinary_disk(&record) {
            return Err(Error::DeviceInUse);
        }
        let consumers = refs(&record, "VBDs")?;
        if consumers.is_empty() || consumers.iter().any(|v| !attachments.contains(v)) {
            return Err(Error::DeviceInUse);
        }
        for consumer in consumers {
            let v = session
                .call("VBD.get_record", vec![json!(consumer)])
                .await?;
            if v["VM"] != vm_ref
                || v["VDI"] != disk_ref
                || v["type"] != "Disk"
                || v["currently_attached"] != false
                || !v["other_config"].get("owner").is_some_and(Value::is_string)
            {
                return Err(Error::DeviceInUse);
            }
        }
    }
    Ok(())
}

pub(crate) enum Prepared {
    Call {
        method: String,
        parameters: Vec<Value>,
    },
    Device(devices::Prepared),
    ReadOnly,
}
impl Prepared {
    pub async fn send(self, session: &Session) -> Result<Value, Error> {
        match self {
            Self::Call { method, parameters } => session.call(&method, parameters).await,
            Self::Device(p) => p.send(session).await,
            Self::ReadOnly => Ok(Value::Null),
        }
    }
}
fn call(method: &str, parameters: Vec<Value>) -> Prepared {
    Prepared::Call {
        method: method.into(),
        parameters,
    }
}

pub(crate) async fn prepare(
    session: &Session,
    operation_id: OperationId,
    plan: &mut Plan,
    index: usize,
) -> Result<Prepared, Error> {
    let step = plan.steps[index].clone();
    match step {
        Step::Copy { mode, name } => {
            let (source_ref, r) = source(session, plan).await?;
            validate_source_devices(session, &r).await?;
            let cap = if matches!(mode, VmCopyMode::Fast) {
                "clone"
            } else {
                "copy"
            };
            permits(&r, cap)?;
            validate_create_choices(session, &plan.operation).await?;
            for class in ["VM", "VDI", "VBD", "VIF"] {
                plan.baseline
                    .insert(class.into(), devices::baseline(session, class).await?);
            }
            let (_, current) = source(session, plan).await?;
            permits(&current, cap)?;
            validate_source_devices(session, &current).await?;
            plan.source_graph = graph(session, &current).await?;
            guard_creation_permissions(session, plan, &current).await?;
            if let VmLifecycleOperation::Create {
                firmware,
                bios_host,
                ..
            } = &plan.operation
            {
                validate_firmware_change(session, &current, *firmware).await?;
                if *firmware == GuestFirmware::SecureUefi {
                    crate::guest_boot::require_policy_support(session, &source_ref).await?;
                }
                if let Some(host) = bios_host {
                    bios_eligible(&current, *firmware)?;
                    let (_, h) = fresh(session, "host", host.uuid()).await?;
                    if plan.bios_identity.as_ref() != Some(&string_map(&h["bios_strings"])?) {
                        return Err(Error::VmLifecycleConflict);
                    }
                }
            }
            let mut parameters = vec![json!(source_ref), json!(name)];
            if let VmCopyMode::Full { sr: id } = mode {
                parameters.push(json!(sr(session, id).await?));
            }
            Ok(call(&format!("Async.VM.{cap}"), parameters))
        }
        Step::CopyBios { vm, host } => {
            let (vr, r) = target(session, plan, vm).await?;
            bios_eligible(&r, GuestFirmware::Inherited)?;
            let (hr, h) = fresh(session, "host", host.uuid()).await?;
            if plan.bios_identity.as_ref() != Some(&string_map(&h["bios_strings"])?) {
                return Err(Error::VmLifecycleConflict);
            }
            Ok(call("VM.copy_bios_strings", vec![json!(vr), json!(hr)]))
        }
        Step::Set { vm, change } => {
            let (vm_ref, r) = target(session, plan, vm).await?;
            match change {
                Change::Name(value) => {
                    Ok(call("VM.set_name_label", vec![json!(vm_ref), json!(value)]))
                }
                Change::Description(value) => Ok(call(
                    "VM.set_name_description",
                    vec![json!(vm_ref), json!(value)],
                )),
                Change::Template(value) => {
                    if value {
                        permits(&r, "make_into_template")?;
                    }
                    Ok(call(
                        "VM.set_is_a_template",
                        vec![json!(vm_ref), json!(value)],
                    ))
                }
                Change::Memory(value) => {
                    permits_memory_limits(&r)?;
                    let min = number(&r["memory_static_min"]).ok_or(Error::VmLifecycleConflict)?;
                    if min > value {
                        return Err(Error::InvalidVmLifecycle);
                    }
                    Ok(call(
                        "VM.set_memory_limits",
                        vec![
                            json!(vm_ref),
                            json!(min.to_string()),
                            json!(value.to_string()),
                            json!(value.to_string()),
                            json!(value.to_string()),
                        ],
                    ))
                }
                Change::VcpuMax(value) => {
                    permits_vcpu_change(&r)?;
                    Ok(call(
                        "VM.set_VCPUs_max",
                        vec![json!(vm_ref), json!(value.to_string())],
                    ))
                }
                Change::VcpuStartup(value) => {
                    permits_vcpu_change(&r)?;
                    Ok(call(
                        "VM.set_VCPUs_at_startup",
                        vec![json!(vm_ref), json!(value.to_string())],
                    ))
                }
                Change::SecureBoot(value) => {
                    if value {
                        crate::guest_boot::require_policy_support(session, &vm_ref).await?;
                    }
                    if crate::guest_boot::firmware(&r) != Some("uefi") {
                        return Err(Error::UnsupportedVmLifecycle);
                    }
                    let mut params = string_map(&r["platform"])?;
                    params.insert("secureboot".into(), json!(value.to_string()));
                    Ok(call(
                        "VM.set_platform",
                        vec![json!(vm_ref), Value::Object(params)],
                    ))
                }
                Change::Firmware(value) => {
                    validate_firmware_change(session, &r, value).await?;
                    if !crate::guest_boot::is_hvm(&r) {
                        return Err(Error::UnsupportedVmLifecycle);
                    }
                    firmware_safe(&r, value)?;
                    let mut params = string_map(&r["HVM_boot_params"])?;
                    params.insert(
                        "firmware".into(),
                        json!(match value {
                            GuestFirmware::Bios => "bios",
                            GuestFirmware::Uefi | GuestFirmware::SecureUefi => "uefi",
                            GuestFirmware::Inherited => return Err(Error::InvalidVmLifecycle),
                        }),
                    );
                    Ok(call(
                        "VM.set_HVM_boot_params",
                        vec![json!(vm_ref), Value::Object(params)],
                    ))
                }
                Change::RemoveProvisionDisks => Ok(call(
                    "VM.remove_from_other_config",
                    vec![json!(vm_ref), json!("disks")],
                )),
            }
        }
        Step::Provision { vm } => {
            let (vm_ref, r) = target(session, plan, vm).await?;
            permits(&r, "provision")?;
            if r["other_config"].get("disks").is_some() {
                return Err(Error::VmLifecycleConflict);
            }
            Ok(call("Async.VM.provision", vec![json!(vm_ref)]))
        }
        Step::RefreshAllowedOperations { vm } => {
            let (vm_ref, _) = target(session, plan, vm).await?;
            Ok(call("VM.update_allowed_operations", vec![json!(vm_ref)]))
        }
        Step::CheckHosts { vm } => {
            let (reference, _) = target(session, plan, vm).await?;
            compatible_hosts(session, &reference).await?;
            Ok(Prepared::ReadOnly)
        }
        Step::ConfigureDevices { vm } => {
            target(session, plan, vm).await?;
            configure_devices(session, plan, vm).await?;
            Ok(Prepared::ReadOnly)
        }
        Step::Device {
            vm,
            group,
            index: device_index,
        } => {
            target(session, plan, vm).await?;
            let device_plan = &mut plan.groups[group];
            if let Some(class) = device_plan.steps[device_index].creation_class() {
                device_plan.evidence[device_index] = Some(devices::Evidence {
                    baseline: devices::baseline(session, class).await?,
                    ..Default::default()
                });
            }
            Ok(Prepared::Device(
                devices::prepare_step(session, vm, operation_id, device_plan, device_index).await?,
            ))
        }
        Step::EjectIso { vm, vbd, image } => {
            let (vm_ref, _) = target(session, plan, vm).await?;
            let (vbd_ref, r) = fresh(session, "VBD", vbd.uuid()).await?;
            let (image_ref, _) = fresh(session, "VDI", image.uuid()).await?;
            if r["VM"] != vm_ref
                || r["VDI"] != image_ref
                || r["type"] != "CD"
                || r["currently_attached"] != false
            {
                return Err(Error::VmLifecycleConflict);
            }
            permits(&r, "eject")?;
            Ok(call("VBD.eject", vec![json!(vbd_ref)]))
        }
        Step::InstallIso { vm, image } => {
            let (vm_ref, r) = target(session, plan, vm).await?;
            let (image_ref, image_record) = fresh(session, "VDI", image.uuid()).await?;
            require_iso_image(session, &image_record).await?;
            let (_, cd) = first_cd(session, &r)
                .await?
                .ok_or(Error::VmLifecycleConflict)?;
            if cd["VM"] != vm_ref || cd["currently_attached"] != false || cd["empty"] != true {
                return Err(Error::VmLifecycleConflict);
            }
            let cd_ref = reference(&cd["ref"])?;
            permits(&cd, "insert")?;
            Ok(call("VBD.insert", vec![json!(cd_ref), json!(image_ref)]))
        }
        Step::ClaimDisk { vm, vbd, disk } => {
            let (vm_ref, _) = target(session, plan, vm).await?;
            let (vbd_ref, attachment) = fresh(session, "VBD", vbd.uuid()).await?;
            let (disk_ref, record) = fresh(session, "VDI", disk.uuid()).await?;
            if plan
                .baseline
                .get("VBD")
                .is_none_or(|ids| ids.contains(&vbd.uuid()))
                || plan
                    .baseline
                    .get("VDI")
                    .is_none_or(|ids| ids.contains(&disk.uuid()))
                || attachment["VM"] != vm_ref
                || attachment["VDI"] != disk_ref
                || attachment["type"] != "Disk"
                || !ordinary_disk(&record)
                || refs(&record, "VBDs")? != vec![vbd_ref.clone()]
            {
                return Err(Error::VmLifecycleConflict);
            }
            idle(&attachment)?;
            idle(&record)?;
            if attachment["other_config"]
                .get("owner")
                .is_some_and(Value::is_string)
            {
                return Ok(Prepared::ReadOnly);
            }
            Ok(call(
                "VBD.add_to_other_config",
                vec![json!(vbd_ref), json!("owner"), json!("true")],
            ))
        }
        Step::DestroySnapshot { snapshot, disks } => {
            source(session, plan).await?;
            let checked = snapshots::deletion_plan(session, plan.source, snapshot, &disks).await?;
            if checked.checkpoint {
                return Err(Error::VmLifecycleConflict);
            }
            let (reference, _) = snapshots::snapshot_record(session, plan.source, snapshot).await?;
            Ok(call("VM.destroy", vec![json!(reference)]))
        }
        Step::DestroyVm => {
            let (reference, r) = source(session, plan).await?;
            permits(&r, "destroy")?;
            validate_owned_disks(session, plan.source, &plan.owned_disks).await?;
            Ok(call("VM.destroy", vec![json!(reference)]))
        }
        Step::DestroyDisk { disk, snapshot } => {
            let (reference, r) = fresh(session, "VDI", disk.uuid()).await?;
            if !refs(&r, "VBDs")?.is_empty()
                || (!snapshot && !ordinary_disk(&r))
                || (snapshot
                    && (r["is_a_snapshot"] != true
                        || r["type"] != "user"
                        || r["sharable"] != false
                        || r["managed"] != true
                        || r["missing"] != false))
            {
                return Err(Error::DeviceInUse);
            }
            permits(&r, "destroy")?;
            let vms = session.call("VM.get_all_records", vec![]).await?;
            if vms
                .as_object()
                .filter(|v| v.len() <= 10_000)
                .ok_or(Error::VmLifecycleConflict)?
                .values()
                .any(|v| v["suspend_VDI"] == reference)
            {
                return Err(Error::DeviceInUse);
            }
            Ok(call("VDI.destroy", vec![json!(reference)]))
        }
    }
}
fn string_map(v: &Value) -> Result<serde_json::Map<String, Value>, Error> {
    let map = v
        .as_object()
        .filter(|v| v.len() <= 256)
        .ok_or(Error::VmLifecycleConflict)?;
    if map
        .iter()
        .any(|(k, v)| k.len() > 1024 || v.as_str().is_none_or(|s| s.len() > 64 * 1024))
    {
        return Err(Error::VmLifecycleConflict);
    }
    Ok(map.clone())
}

async fn first_cd(session: &Session, vm: &Value) -> Result<Option<(VbdId, Value)>, Error> {
    for attachment in refs(vm, "VBDs")? {
        let mut r = session
            .call("VBD.get_record", vec![json!(&attachment)])
            .await?;
        if r["type"] == "CD" {
            let id = id(&r)?;
            r["ref"] = json!(attachment);
            return Ok(Some((id, r)));
        }
    }
    Ok(None)
}
async fn slots(session: &Session, vm_ref: &str, class: &str) -> Result<Vec<String>, Error> {
    let values = session
        .call(
            &format!("VM.get_allowed_{class}_devices"),
            vec![json!(vm_ref)],
        )
        .await?;
    let mut slots = values
        .as_array()
        .filter(|v| v.len() <= 256)
        .ok_or(Error::VmLifecycleConflict)?
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| s.parse::<u8>().is_ok_and(|n| n.to_string() == *s))
                .map(str::to_owned)
                .ok_or(Error::VmLifecycleConflict)
        })
        .collect::<Result<Vec<_>, _>>()?;
    slots.sort_by_key(|s| s.parse::<u8>().unwrap());
    slots.dedup();
    Ok(slots)
}
fn push_group(plan: &mut Plan, vm: VmId, group: devices::Plan) {
    let id = plan.groups.len();
    for index in 0..group.steps.len() {
        plan.push(Step::Device {
            vm,
            group: id,
            index,
        });
    }
    plan.groups.push(group);
}
async fn configure_devices(session: &Session, plan: &mut Plan, vm: VmId) -> Result<(), Error> {
    let VmLifecycleOperation::Create {
        sr,
        network,
        iso,
        disk_size,
        name,
        ..
    } = plan.operation.clone()
    else {
        return Err(Error::InvalidVmLifecycle);
    };
    let (vm_ref, vm_record) = target(session, plan, vm).await?;
    if vm_record["is_a_template"] != false {
        return Err(Error::VmLifecycleConflict);
    }
    let mut disk_slots = slots(session, &vm_ref, "VBD").await?;
    if let Some(size) = disk_size {
        let device = disk_slots
            .first()
            .cloned()
            .ok_or(Error::UnavailableAction)?;
        disk_slots.remove(0);
        let group = devices::plan(
            session,
            vm,
            &DeviceOperation::CreateDisk {
                sr,
                name: format!("{} disk", name.chars().take(250).collect::<String>()),
                size,
                device,
            },
        )
        .await?;
        push_group(plan, vm, group);
    }
    let nics = refs(&vm_record, "VIFs")?;
    if nics.is_empty() {
        let device = slots(session, &vm_ref, "VIF")
            .await?
            .first()
            .cloned()
            .ok_or(Error::UnavailableAction)?;
        let group = devices::plan(
            session,
            vm,
            &DeviceOperation::AddNic {
                network,
                device,
                mac: None,
            },
        )
        .await?;
        push_group(plan, vm, group);
    } else {
        for nic in nics {
            let r = session.call("VIF.get_record", vec![json!(nic)]).await?;
            let old_ref = reference(&r["network"])?;
            let old = session
                .call("network.get_record", vec![json!(old_ref)])
                .await?;
            let from = id(&old)?;
            if from != network {
                let mac = r["MAC"]
                    .as_str()
                    .ok_or(Error::VmLifecycleConflict)?
                    .to_owned();
                let group = devices::plan(
                    session,
                    vm,
                    &DeviceOperation::EditNic {
                        vif: id(&r)?,
                        expected_network: from,
                        expected_mac: mac.clone(),
                        network,
                        mac,
                    },
                )
                .await?;
                push_group(plan, vm, group);
            }
        }
    }
    if let Some(image) = iso {
        match first_cd(session, &vm_record).await? {
            Some((vbd, cd)) if cd["empty"] != true => {
                let old = session
                    .call("VDI.get_record", vec![json!(reference(&cd["VDI"])?)])
                    .await?;
                plan.push(Step::EjectIso {
                    vm,
                    vbd,
                    image: id(&old)?,
                });
            }
            None => {
                let device = disk_slots
                    .first()
                    .cloned()
                    .ok_or(Error::UnavailableAction)?;
                let group =
                    devices::plan(session, vm, &DeviceOperation::AddCdDrive { device }).await?;
                push_group(plan, vm, group);
            }
            _ => {}
        }
        plan.push(Step::InstallIso { vm, image });
    }
    plan.push(Step::CheckHosts { vm });
    Ok(())
}
async fn capture_graph(
    session: &Session,
    plan: &mut Plan,
    vm_ref: &str,
    r: &Value,
) -> Result<(), Error> {
    for attachment in refs(r, "VBDs")? {
        let vbd = session
            .call("VBD.get_record", vec![json!(&attachment)])
            .await?;
        let vbd_id: VbdId = id(&vbd)?;
        if vbd["VM"] != vm_ref
            || plan
                .baseline
                .get("VBD")
                .is_none_or(|ids| ids.contains(&vbd_id.uuid()))
        {
            return Err(Error::VmLifecycleConflict);
        }
        if !plan
            .created_devices
            .contains(&CreatedDevice::Attachment(vbd_id))
        {
            plan.created_devices.push(CreatedDevice::Attachment(vbd_id));
        }
        if vbd["type"] == "Disk" {
            let disk_ref = reference(&vbd["VDI"])?;
            let disk = session
                .call("VDI.get_record", vec![json!(disk_ref)])
                .await?;
            let disk_id: VdiId = id(&disk)?;
            if !ordinary_disk(&disk)
                || plan
                    .baseline
                    .get("VDI")
                    .is_none_or(|ids| ids.contains(&disk_id.uuid()))
                || refs(&disk, "VBDs")?
                    .iter()
                    .any(|v| !refs(r, "VBDs").unwrap_or_default().contains(v))
            {
                return Err(Error::VmLifecycleConflict);
            }
            if !plan.created_devices.contains(&CreatedDevice::Disk(disk_id)) {
                plan.created_devices.push(CreatedDevice::Disk(disk_id));
            }
        } else if vbd["type"] != "CD" {
            return Err(Error::UnsupportedVmLifecycle);
        }
    }
    for nic in refs(r, "VIFs")? {
        let vif = session.call("VIF.get_record", vec![json!(nic)]).await?;
        let vif_id: VifId = id(&vif)?;
        if vif["VM"] != vm_ref
            || plan
                .baseline
                .get("VIF")
                .is_none_or(|ids| ids.contains(&vif_id.uuid()))
        {
            return Err(Error::VmLifecycleConflict);
        }
        if !plan.created_devices.contains(&CreatedDevice::Nic(vif_id)) {
            plan.created_devices.push(CreatedDevice::Nic(vif_id));
        }
    }
    Ok(())
}
fn after_copy(plan: &mut Plan, vm: VmId, record: &Value) -> Result<(), Error> {
    match plan.operation.clone() {
        VmLifecycleOperation::Create {
            description,
            vcpus,
            memory,
            firmware,
            bios_host,
            ..
        } => {
            if record["other_config"].get("disks").is_some() {
                plan.push(Step::Set {
                    vm,
                    change: Change::RemoveProvisionDisks,
                });
            }
            if !matches!(firmware, GuestFirmware::Inherited) {
                plan.push(Step::Set {
                    vm,
                    change: Change::Firmware(firmware),
                });
                if matches!(firmware, GuestFirmware::Uefi | GuestFirmware::SecureUefi) {
                    plan.push(Step::Set {
                        vm,
                        change: Change::SecureBoot(firmware == GuestFirmware::SecureUefi),
                    });
                }
            }
            if let Some(host) = bios_host {
                plan.push(Step::CopyBios { vm, host });
            }
            plan.push(Step::Provision { vm });
            plan.push(Step::Set {
                vm,
                change: Change::Description(description),
            });
            plan.push(Step::Set {
                vm,
                change: Change::Memory(memory),
            });
            let old = number(&record["VCPUs_at_startup"]).ok_or(Error::VmLifecycleConflict)?;
            if vcpus < old {
                plan.push(Step::Set {
                    vm,
                    change: Change::VcpuStartup(vcpus),
                });
                plan.push(Step::Set {
                    vm,
                    change: Change::VcpuMax(vcpus),
                });
            } else {
                plan.push(Step::Set {
                    vm,
                    change: Change::VcpuMax(vcpus),
                });
                plan.push(Step::Set {
                    vm,
                    change: Change::VcpuStartup(vcpus),
                });
            }
            plan.push(Step::ConfigureDevices { vm });
        }
        VmLifecycleOperation::Clone { as_template, .. } => {
            if record["is_a_template"] != as_template {
                plan.push(Step::Set {
                    vm,
                    change: Change::Template(as_template),
                });
                if !as_template {
                    plan.push(Step::RefreshAllowedOperations { vm });
                }
            }
        }
        _ => return Err(Error::InvalidVmLifecycle),
    }
    plan.workflow_planned = true;
    Ok(())
}
pub(crate) fn record_result(plan: &mut Plan, index: usize, result: &Value) -> Result<(), Error> {
    if let Step::Device {
        group,
        index: device_index,
        ..
    } = plan.steps[index]
        && let Some(evidence) = &mut plan.groups[group].evidence[device_index]
    {
        evidence.result_reference = Some(reference(result)?);
    }
    Ok(())
}
pub(crate) async fn verify(
    session: &Session,
    operation_id: OperationId,
    plan: &mut Plan,
    index: usize,
    continue_work: bool,
) -> Result<bool, Error> {
    match plan.steps[index].clone() {
        Step::Copy { name, .. } => {
            let Some(reference) = plan.evidence[index].result.clone() else {
                return Ok(false);
            };
            let record = session
                .call("VM.get_record", vec![json!(reference)])
                .await?;
            let vm: VmId = id(&record)?;
            if !ordinary(&record)
                || record["power_state"] != "Halted"
                || record["name_label"] != name
                || plan
                    .baseline
                    .get("VM")
                    .is_none_or(|ids| ids.contains(&vm.uuid()))
                || record["snapshot_of"] != "OpaqueRef:NULL"
            {
                return Err(Error::VmLifecycleConflict);
            }
            idle(&record)?;
            if !plan.evidence[index].completed {
                return Ok(false);
            }
            if (continue_work || !plan.workflow_planned)
                && (record["is_a_template"] != plan.operation.template()
                    || record["other_config"]["default_template"] == "true"
                    || record["is_default_template"] == true)
            {
                return Err(Error::VmLifecycleConflict);
            }
            if continue_work || !plan.workflow_planned {
                let mut expected = plan.source_graph.clone();
                if let Step::Copy {
                    mode: VmCopyMode::Full { sr },
                    ..
                } = &plan.steps[index]
                {
                    for disk in expected.disks.values_mut() {
                        disk.sr = *sr;
                    }
                }
                if graph(session, &record).await? != expected {
                    return Err(Error::VmLifecycleConflict);
                }
            }
            let mut validated = plan.clone();
            capture_graph(session, &mut validated, &reference, &record).await?;
            plan.created_devices = validated.created_devices;
            plan.created = Some(vm);
            if continue_work && plan.steps.len() == 1 {
                for attachment in refs(&record, "VBDs")? {
                    let vbd = session
                        .call("VBD.get_record", vec![json!(attachment)])
                        .await?;
                    if vbd["type"] == "Disk"
                        && !vbd["other_config"]
                            .get("owner")
                            .is_some_and(Value::is_string)
                    {
                        let disk = session
                            .call(
                                "VDI.get_record",
                                vec![json!(crate::snapshots::reference(&vbd["VDI"])?)],
                            )
                            .await?;
                        plan.push(Step::ClaimDisk {
                            vm,
                            vbd: id(&vbd)?,
                            disk: id(&disk)?,
                        });
                    }
                }
                after_copy(plan, vm, &record)?;
            }
            Ok(true)
        }
        Step::CopyBios { vm, host } => {
            let (_, r) = fresh(session, "VM", vm.uuid()).await?;
            let (hr, _) = fresh(session, "host", host.uuid()).await?;
            Ok(ordinary(&r)
                && r["power_state"] == "Halted"
                && r["affinity"] == hr
                && plan.bios_identity.as_ref() == Some(&string_map(&r["bios_strings"])?))
        }
        Step::Set { vm, change } => {
            let Some((_, r)) = snapshots::optional_record(session, "VM", &vm.to_string()).await?
            else {
                return Ok(false);
            };
            if !ordinary(&r) {
                return Ok(false);
            }
            let changed = match &change {
                Change::Name(value) => r["name_label"] == *value,
                Change::Description(value) => r["name_description"] == *value,
                _ => false,
            };
            if changed && let Some(metadata) = &mut plan.metadata {
                match &change {
                    Change::Name(value) => metadata.0 = value.clone(),
                    Change::Description(value) => metadata.1 = value.clone(),
                    _ => {}
                }
            }
            Ok(match change {
                Change::Name(value) => r["name_label"] == value,
                Change::Description(value) => r["name_description"] == value,
                Change::Template(value) => r["is_a_template"] == value,
                Change::Memory(value) => [
                    "memory_static_max",
                    "memory_dynamic_min",
                    "memory_dynamic_max",
                ]
                .iter()
                .all(|k| number(&r[*k]) == Some(value)),
                Change::VcpuMax(value) => number(&r["VCPUs_max"]) == Some(value),
                Change::VcpuStartup(value) => number(&r["VCPUs_at_startup"]) == Some(value),
                Change::SecureBoot(value) => {
                    crate::guest_boot::firmware(&r) == Some("uefi")
                        && crate::guest_boot::policy(&r) == Some(value)
                }
                Change::Firmware(value) => {
                    r["HVM_boot_params"]["firmware"]
                        == match value {
                            GuestFirmware::Bios => "bios",
                            GuestFirmware::Uefi | GuestFirmware::SecureUefi => "uefi",
                            GuestFirmware::Inherited => return Err(Error::InvalidVmLifecycle),
                        }
                }
                Change::RemoveProvisionDisks => r["other_config"].get("disks").is_none(),
            })
        }
        Step::Provision { vm } => {
            let (_, r) = fresh(session, "VM", vm.uuid()).await?;
            Ok(plan.evidence[index].completed
                && r["is_a_template"] == false
                && r["power_state"] == "Halted"
                && r["other_config"].get("disks").is_none())
        }
        Step::ConfigureDevices { .. } => Ok(plan.evidence[index].completed),
        Step::RefreshAllowedOperations { .. } => Ok(plan.evidence[index].completed),
        Step::CheckHosts { vm } => {
            let (reference, _) = fresh(session, "VM", vm.uuid()).await?;
            compatible_hosts(session, &reference).await?;
            Ok(true)
        }
        Step::Device {
            vm,
            group,
            index: device_index,
        } => {
            let ok = devices::verify_step(
                session,
                vm,
                operation_id,
                &mut plan.groups[group],
                device_index,
            )
            .await?;
            for created in plan.groups[group]
                .evidence
                .iter()
                .flatten()
                .filter_map(|e| e.created.clone())
            {
                if !plan.created_devices.contains(&created) {
                    plan.created_devices.push(created);
                }
            }
            Ok(ok)
        }
        Step::InstallIso { vm, image } => {
            let (_, r) = fresh(session, "VM", vm.uuid()).await?;
            let Some((_, cd)) = first_cd(session, &r).await? else {
                return Ok(false);
            };
            if cd["empty"] != false {
                return Ok(false);
            }
            let disk = session
                .call("VDI.get_record", vec![json!(reference(&cd["VDI"])?)])
                .await?;
            Ok(id::<VdiId>(&disk)? == image)
        }
        Step::EjectIso { vm, vbd, .. } => {
            let (vm_ref, _) = fresh(session, "VM", vm.uuid()).await?;
            let (_, r) = fresh(session, "VBD", vbd.uuid()).await?;
            Ok(r["VM"] == vm_ref && r["empty"] == true)
        }
        Step::ClaimDisk { vm, vbd, disk } => {
            let (vm_ref, _) = fresh(session, "VM", vm.uuid()).await?;
            let (_, v) = fresh(session, "VBD", vbd.uuid()).await?;
            let (disk_ref, _) = fresh(session, "VDI", disk.uuid()).await?;
            Ok(v["VM"] == vm_ref
                && v["VDI"] == disk_ref
                && v["other_config"].get("owner").is_some_and(Value::is_string))
        }
        Step::DestroySnapshot { snapshot, .. } => {
            Ok(
                snapshots::optional_record(session, "VM", &snapshot.to_string())
                    .await?
                    .is_none(),
            )
        }
        Step::DestroyVm => Ok(
            snapshots::optional_record(session, "VM", &plan.source.to_string())
                .await?
                .is_none(),
        ),
        Step::DestroyDisk { disk, .. } => {
            Ok(
                snapshots::optional_record(session, "VDI", &disk.to_string())
                    .await?
                    .is_none(),
            )
        }
    }
}

async fn graph(session: &Session, vm: &Value) -> Result<Graph, Error> {
    let mut graph = Graph::default();
    for vbd in refs(vm, "VBDs")? {
        let v = session.call("VBD.get_record", vec![json!(vbd)]).await?;
        let slot = v["userdevice"]
            .as_str()
            .ok_or(Error::VmLifecycleConflict)?
            .to_owned();
        if graph.disks.contains_key(&slot) || graph.cds.contains_key(&slot) {
            return Err(Error::VmLifecycleConflict);
        }
        if v["type"] == "Disk" {
            let d = session
                .call("VDI.get_record", vec![json!(reference(&v["VDI"])?)])
                .await?;
            let sr = session
                .call("SR.get_record", vec![json!(reference(&d["SR"])?)])
                .await?;
            graph.disks.insert(
                slot,
                ExpectedDisk {
                    size: number(&d["virtual_size"]).ok_or(Error::VmLifecycleConflict)?,
                    sr: id(&sr)?,
                    mode: v["mode"].as_str().ok_or(Error::VmLifecycleConflict)?.into(),
                },
            );
        } else if v["type"] == "CD" {
            let image = if v["empty"] == true {
                None
            } else {
                let disk = session
                    .call("VDI.get_record", vec![json!(reference(&v["VDI"])?)])
                    .await?;
                Some(id(&disk)?)
            };
            graph.cds.insert(slot, image);
        } else {
            return Err(Error::UnsupportedVmLifecycle);
        }
    }
    for vif in refs(vm, "VIFs")? {
        let v = session.call("VIF.get_record", vec![json!(vif)]).await?;
        let network = session
            .call("network.get_record", vec![json!(reference(&v["network"])?)])
            .await?;
        let slot = v["device"]
            .as_str()
            .ok_or(Error::VmLifecycleConflict)?
            .to_owned();
        if graph.nics.insert(slot, id(&network)?).is_some() {
            return Err(Error::VmLifecycleConflict);
        }
    }
    Ok(graph)
}
fn bios_eligible(r: &Value, firmware: GuestFirmware) -> Result<(), Error> {
    let bios = firmware == GuestFirmware::Bios
        || firmware == GuestFirmware::Inherited && crate::guest_boot::firmware(r) == Some("bios");
    if !bios
        || !crate::guest_boot::is_hvm(r)
        || !r["bios_strings"].as_object().is_some_and(|v| v.is_empty())
    {
        return Err(Error::UnsupportedVmLifecycle);
    }
    Ok(())
}
async fn validate_firmware_change(
    session: &Session,
    r: &Value,
    mode: GuestFirmware,
) -> Result<(), Error> {
    firmware_safe(r, mode)?;
    let requested = match mode {
        GuestFirmware::Inherited => return Ok(()),
        GuestFirmware::Bios => "bios",
        _ => "uefi",
    };
    if crate::guest_boot::firmware(r) == Some(requested) {
        return Ok(());
    }
    for vr in refs(r, "VBDs")? {
        let v = session.call("VBD.get_record", vec![json!(vr)]).await?;
        if v["type"] != "CD" {
            return Err(Error::UnsupportedVmLifecycle);
        }
    }
    Ok(())
}
fn firmware_safe(record: &Value, firmware: GuestFirmware) -> Result<(), Error> {
    if firmware == GuestFirmware::Bios && record["platform"]["secureboot"] == "true" {
        return Err(Error::UnsupportedVmLifecycle);
    }
    if firmware != GuestFirmware::Inherited && !crate::guest_boot::is_hvm(record) {
        return Err(Error::UnsupportedVmLifecycle);
    }
    Ok(())
}
async fn compatible_hosts(session: &Session, vm_ref: &str) -> Result<(), Error> {
    let hosts = session
        .call("VM.get_possible_hosts", vec![json!(vm_ref)])
        .await?;
    let hosts = hosts
        .as_array()
        .filter(|v| !v.is_empty() && v.len() <= 1024)
        .ok_or(Error::NoCompatibleVmHost)?;
    for host in hosts {
        reference(host)?;
    }
    Ok(())
}
