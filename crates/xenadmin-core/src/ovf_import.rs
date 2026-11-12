use super::*;
use crate::{NetworkId, OperationId, SrId, transfer::Plan as TransferPlan, xapi::Session};
use serde_json::{Value, json};
use uuid::Uuid;
use xenadmin_xapi::enumerations::vm_power_state;
use xenadmin_xapi::records::{NetworkRecord, VbdRecord, VdiRecord, VifRecord, VmRecord};
pub(crate) const MARKER: &str = "xenadmin-rs-ovf-import";
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    Guest { guest: usize },
    Disk { guest: usize, disk: usize },
    Upload { guest: usize, disk: usize },
    Attachment { guest: usize, disk: usize },
    Nic { guest: usize, nic: usize },
    Verify { guest: usize },
}
impl Kind {
    pub(crate) fn class(&self) -> Option<&'static str> {
        match self {
            Self::Guest { .. } => Some("VM"),
            Self::Disk { .. } => Some("VDI"),
            Self::Attachment { .. } => Some("VBD"),
            Self::Nic { .. } => Some("VIF"),
            _ => None,
        }
    }
    pub(crate) fn label(&self) -> &'static str {
        match self {
            Self::Guest { .. } => "Create imported guest",
            Self::Disk { .. } => "Create imported disk",
            Self::Upload { .. } => "Upload disk image",
            Self::Attachment { .. } => "Attach imported disk",
            Self::Nic { .. } => "Create mapped network interface",
            Self::Verify { .. } => "Verify guest configuration and compatible hosts",
        }
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct Step {
    pub kind: Kind,
    pub token: OperationId,
    pub dispatched: bool,
    pub reference: Option<String>,
    pub uuid: Option<Uuid>,
    pub complete: bool,
    pub task: Option<String>,
    pub cancelled: bool,
    pub stream: Option<Box<TransferPlan>>,
}
impl Step {
    fn new(kind: Kind) -> Self {
        Self {
            kind,
            token: OperationId::new(),
            dispatched: false,
            reference: None,
            uuid: None,
            complete: false,
            task: None,
            cancelled: false,
            stream: None,
        }
    }
    pub(crate) fn accepts(&self, new: &Self) -> bool {
        self.kind == new.kind
            && self.token == new.token
            && (!self.dispatched || new.dispatched)
            && (!self.complete || new.complete)
            && (!self.cancelled || new.cancelled)
            && (self.reference.is_none() || self.reference == new.reference)
            && (self.uuid.is_none() || self.uuid == new.uuid)
            && (self.task.is_none() || self.task == new.task)
            && match (&self.stream, &new.stream) {
                (None, _) => true,
                (Some(a), Some(b)) => crate::transfer::plan_accepts(a, b),
                _ => false,
            }
    }
}
#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) struct ImportPlan {
    pub summary: OvfSummary,
    pub sr: SrId,
    pub networks: BTreeMap<String, NetworkId>,
    pub baseline: BTreeMap<String, Vec<Uuid>>,
    pub steps: Vec<Step>,
}
impl ImportPlan {
    pub(crate) fn accepts(&self, n: &Self) -> bool {
        self.summary == n.summary
            && self.sr == n.sr
            && self.networks == n.networks
            && self.baseline == n.baseline
            && self.steps.len() == n.steps.len()
            && self.steps.iter().zip(&n.steps).all(|(a, b)| a.accepts(b))
    }
    pub(crate) fn step(&self, kind: Kind) -> Result<&Step, Error> {
        self.steps
            .iter()
            .find(|s| s.kind == kind)
            .ok_or(Error::TransferConflict)
    }
    pub(crate) fn reference(&self, kind: Kind) -> Result<&str, Error> {
        self.step(kind)?
            .reference
            .as_deref()
            .ok_or(Error::TransferConflict)
    }
    pub(crate) fn disk(&self, g: usize, d: usize) -> Result<&OvfDisk, Error> {
        let key = &self
            .summary
            .guests
            .get(g)
            .and_then(|g| g.disks.get(d))
            .ok_or(Error::TransferConflict)?
            .disk;
        self.summary
            .disks
            .iter()
            .find(|d| &d.key == key)
            .ok_or(Error::TransferConflict)
    }
}
fn number(v: &Value) -> Option<u64> {
    v.as_u64().or_else(|| v.as_str()?.parse().ok())
}
fn idle(v: &Value) -> Result<(), Error> {
    if VmRecord::new(v)
        .current_operations()
        .as_object()
        .is_none_or(|m| !m.is_empty())
    {
        Err(Error::Busy)
    } else {
        Ok(())
    }
}
pub(crate) async fn fresh(s: &Session, class: &str, id: Uuid) -> Result<(String, Value), Error> {
    crate::snapshots::optional_record(s, class, &id.to_string())
        .await?
        .ok_or(Error::TransferConflict)
}
async fn network(s: &Session, id: NetworkId) -> Result<(String, u64), Error> {
    let (r, v) = fresh(s, "network", id.uuid()).await?;
    let mtu = number(&NetworkRecord::new(&v).mtu())
        .filter(|n| (576..=65535).contains(n))
        .ok_or(Error::TransferConflict)?;
    Ok((r, mtu))
}
pub(crate) async fn prepare(
    s: &Session,
    pool: crate::PoolId,
    file: &TransferFile,
    sr: SrId,
    networks: &BTreeMap<String, NetworkId>,
) -> Result<ImportPlan, Error> {
    if s.options().read_only {
        return Err(Error::ReadOnly);
    }
    let selection = file.ovf.as_ref().ok_or(Error::TransferFile)?;
    if selection
        .summary
        .networks
        .iter()
        .cloned()
        .collect::<std::collections::BTreeSet<_>>()
        != networks.keys().cloned().collect()
    {
        return Err(Error::Ovf("map every declared network explicitly"));
    }
    crate::transfer::xapi::storage_route(s, pool, sr, true).await?;
    for id in networks.values() {
        network(s, *id).await?;
    }
    for file in selection.files.values() {
        crate::transfer::files::source(file)?;
    }
    let mut baseline = BTreeMap::new();
    for class in ["VM", "VDI", "VBD", "VIF", "task"] {
        baseline.insert(
            class.into(),
            crate::device_lifecycle::baseline(s, class).await?,
        );
    }
    let mut steps = Vec::new();
    for (g, guest) in selection.summary.guests.iter().enumerate() {
        steps.push(Step::new(Kind::Guest { guest: g }));
        for (d, a) in guest.disks.iter().enumerate() {
            steps.push(Step::new(Kind::Disk { guest: g, disk: d }));
            if selection
                .summary
                .disks
                .iter()
                .find(|d| d.key == a.disk)
                .ok_or(Error::TransferConflict)?
                .file
                .is_some()
            {
                steps.push(Step::new(Kind::Upload { guest: g, disk: d }));
            }
            steps.push(Step::new(Kind::Attachment { guest: g, disk: d }));
        }
        for n in 0..guest.nics.len() {
            steps.push(Step::new(Kind::Nic { guest: g, nic: n }));
        }
        steps.push(Step::new(Kind::Verify { guest: g }));
    }
    if steps.len() > 2048 {
        return Err(Error::TransferLimit);
    }
    Ok(ImportPlan {
        summary: selection.summary.clone(),
        sr,
        networks: networks.clone(),
        baseline,
        steps,
    })
}
async fn guest(s: &Session, p: &ImportPlan, g: usize) -> Result<(String, Value), Error> {
    let st = p.step(Kind::Guest { guest: g })?;
    let (r, v) = fresh(s, "VM", st.uuid.ok_or(Error::TransferConflict)?).await?;
    if Some(r.as_str()) != st.reference.as_deref()
        || VmRecord::new(&v).other_config()[MARKER] != st.token.to_string()
        || VmRecord::new(&v).is_a_template() != false
        || VmRecord::new(&v).is_a_snapshot() != false
        || VmRecord::new(&v).is_control_domain() != false
        || VmRecord::new(&v).power_state() != vm_power_state::HALTED
    {
        return Err(Error::TransferConflict);
    }
    idle(&v)?;
    Ok((r, v))
}
async fn disk(
    s: &Session,
    p: &ImportPlan,
    g: usize,
    d: usize,
    unattached: bool,
) -> Result<(String, Value), Error> {
    let st = p.step(Kind::Disk { guest: g, disk: d })?;
    let expected = p.disk(g, d)?;
    let (r, v) = crate::transfer::disk::disk_record(
        s,
        st.uuid.ok_or(Error::TransferConflict)?.into(),
        p.sr,
        expected.format,
        unattached,
    )
    .await?;
    if Some(r.as_str()) != st.reference.as_deref()
        || VdiRecord::new(&v).other_config()[crate::transfer::disk::MARKER] != st.token.to_string()
        || number(&VdiRecord::new(&v).virtual_size())
            .is_none_or(|size| size < expected.virtual_size)
        || VdiRecord::new(&v).read_only() != false
        || VdiRecord::new(&v).is_a_snapshot() != false
    {
        return Err(Error::TransferConflict);
    }
    Ok((r, v))
}
pub(crate) async fn arguments(
    s: &Session,
    p: &ImportPlan,
    index: usize,
) -> Result<(&'static str, Vec<Value>), Error> {
    let step = p.steps.get(index).ok_or(Error::TransferConflict)?;
    match step.kind {
        Kind::Guest { guest: g } => {
            let g = &p.summary.guests[g];
            let n = g.memory_bytes.to_string();
            let cpu = g.cpus.to_string();
            Ok((
                "VM.create",
                vec![
                    json!({"name_label":g.name,"name_description":g.description,"user_version":"1","is_a_template":false,"affinity":"OpaqueRef:NULL","memory_static_min":n,"memory_static_max":n,"memory_dynamic_min":n,"memory_dynamic_max":n,"VCPUs_params":{},"VCPUs_max":cpu,"VCPUs_at_startup":cpu,"actions_after_shutdown":"destroy","actions_after_reboot":"restart","actions_after_crash":"restart","PV_bootloader":"","PV_kernel":"","PV_ramdisk":"","PV_args":"","PV_bootloader_args":"","PV_legacy_args":"","HVM_boot_policy":"BIOS order","HVM_boot_params":{"order":"dc","firmware":if g.firmware==GuestFirmware::Uefi{"uefi"}else{"bios"}},"HVM_shadow_multiplier":1.0,"platform":{"acpi":"true","apic":"true","nx":"true","pae":"true","secureboot":"false"},"PCI_bus":"","other_config":{MARKER:step.token.to_string()},"recommendations":"","xenstore_data":{},"ha_always_run":false,"ha_restart_priority":"","tags":[],"blocked_operations":{}}),
                ],
            ))
        }
        Kind::Disk { guest: g, disk: d } => {
            guest(s, p, g).await?;
            let size = p.disk(g, d)?.virtual_size;
            let name = format!(
                "{} disk {}",
                p.summary.guests[g]
                    .name
                    .chars()
                    .take(220)
                    .collect::<String>(),
                d + 1
            );
            Ok((
                "VDI.create",
                crate::transfer::disk::creation_arguments(s, p.sr, &name, size, step.token).await?,
            ))
        }
        Kind::Attachment { guest: g, disk: d } => {
            let (vr, _) = guest(s, p, g).await?;
            let (dr, _) = disk(s, p, g, d, true).await?;
            let slot = p.summary.guests[g].disks[d].device.to_string();
            allowed_slot(s, &vr, "VBD", &slot).await?;
            Ok((
                "VBD.create",
                vec![
                    json!({"VM":vr,"VDI":dr,"userdevice":slot,"bootable":d==0,"mode":"RW","type":"Disk","unpluggable":true,"empty":false,"other_config":{MARKER:step.token.to_string(),"owner":"true"},"qos_algorithm_type":"","qos_algorithm_params":{}}),
                ],
            ))
        }
        Kind::Nic { guest: g, nic: n } => {
            let (vr, _) = guest(s, p, g).await?;
            let nic = &p.summary.guests[g].nics[n];
            let (nr, mtu) = network(s, p.networks[&nic.network]).await?;
            let slot = p.summary.guests[g].nics[n].device.to_string();
            allowed_slot(s, &vr, "VIF", &slot).await?;
            Ok((
                "VIF.create",
                vec![
                    json!({"VM":vr,"network":nr,"device":slot,"MAC":"","MTU":mtu.to_string(),"other_config":{MARKER:step.token.to_string()},"qos_algorithm_type":"","qos_algorithm_params":{},"locking_mode":"network_default","ipv4_allowed":[],"ipv6_allowed":[]}),
                ],
            ))
        }
        _ => Err(Error::TransferConflict),
    }
}
async fn allowed_slot(s: &Session, vm: &str, class: &str, slot: &str) -> Result<(), Error> {
    let v = s
        .call(&format!("VM.get_allowed_{class}_devices"), vec![json!(vm)])
        .await?;
    if v.as_array()
        .is_none_or(|a| a.len() > 256 || !a.iter().any(|v| v == slot))
    {
        return Err(Error::UnavailableAction);
    }
    Ok(())
}
pub(crate) async fn provenance(
    s: &Session,
    p: &ImportPlan,
    index: usize,
) -> Result<(String, Uuid, Value), Error> {
    let step = p.steps.get(index).ok_or(Error::TransferConflict)?;
    let class = step.kind.class().ok_or(Error::TransferConflict)?;
    let marker = if class == "VDI" {
        crate::transfer::disk::MARKER
    } else {
        MARKER
    };
    let all = s.call(&format!("{class}.get_all_records"), vec![]).await?;
    let records = all
        .as_object()
        .filter(|r| r.len() <= 10000)
        .ok_or(Error::TransferLimit)?;
    let found = records
        .iter()
        .filter(|(_, r)| r["other_config"][marker] == step.token.to_string())
        .collect::<Vec<_>>();
    if found.len() != 1 {
        return Err(Error::TransferConflict);
    }
    let (reference, v) = found[0];
    let id = v["uuid"]
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or(Error::TransferConflict)?;
    if p.baseline
        .get(class)
        .ok_or(Error::TransferConflict)?
        .contains(&id)
        || step.reference.as_ref().is_some_and(|r| r != reference)
        || step.uuid.is_some_and(|old| old != id)
        || p.steps
            .iter()
            .enumerate()
            .any(|(i, s)| i != index && s.uuid == Some(id))
    {
        return Err(Error::TransferConflict);
    }
    let (actual, v) = fresh(s, class, id).await?;
    if actual != *reference || v["other_config"][marker] != step.token.to_string() {
        return Err(Error::TransferConflict);
    }
    Ok((actual, id, v))
}
pub(crate) async fn identify(
    s: &Session,
    p: &ImportPlan,
    index: usize,
) -> Result<(String, Uuid), Error> {
    let (actual, id, v) = provenance(s, p, index).await?;
    let step = p.steps.get(index).ok_or(Error::TransferConflict)?;
    let mut checked = p.clone();
    checked.steps[index].reference = Some(actual.clone());
    checked.steps[index].uuid = Some(id);
    match step.kind {
        Kind::Guest { guest: g } => {
            let (_, v) = guest(s, &checked, g).await?;
            let expected = &p.summary.guests[g];
            if VmRecord::new(&v).name_label() != expected.name
                || VmRecord::new(&v).name_description() != expected.description
                || number(&VmRecord::new(&v).vcpus_max()) != Some(u64::from(expected.cpus))
                || number(&VmRecord::new(&v).vcpus_at_startup()) != Some(u64::from(expected.cpus))
                || [
                    "memory_static_min",
                    "memory_static_max",
                    "memory_dynamic_min",
                    "memory_dynamic_max",
                ]
                .iter()
                .any(|k| number(&v[*k]) != Some(expected.memory_bytes))
                || VmRecord::new(&v).hvm_boot_policy() != "BIOS order"
                || VmRecord::new(&v).hvm_boot_params()["order"] != "dc"
                || VmRecord::new(&v).platform()["secureboot"] != "false"
                || VmRecord::new(&v).hvm_boot_params()["firmware"]
                    != if expected.firmware == GuestFirmware::Uefi {
                        "uefi"
                    } else {
                        "bios"
                    }
            {
                return Err(Error::TransferConflict);
            }
        }
        Kind::Disk { guest: g, disk: d } => {
            disk(s, &checked, g, d, false).await?;
        }
        Kind::Attachment { guest: g, disk: d } => {
            let (vr, _) = guest(s, p, g).await?;
            let (dr, _) = disk(s, p, g, d, false).await?;
            if VbdRecord::new(&v).vm() != vr
                || VbdRecord::new(&v).vdi() != dr
                || number(&VbdRecord::new(&v).userdevice())
                    != Some(u64::from(p.summary.guests[g].disks[d].device))
                || VbdRecord::new(&v).other_config()["owner"] != "true"
                || VbdRecord::new(&v).mode() != "RW"
                || VbdRecord::new(&v).type_() != "Disk"
                || VbdRecord::new(&v).empty() != false
                || VbdRecord::new(&v).currently_attached() != false
                || VbdRecord::new(&v).bootable() != (d == 0)
            {
                return Err(Error::TransferConflict);
            }
        }
        Kind::Nic { guest: g, nic: n } => {
            let (vr, _) = guest(s, p, g).await?;
            let (nr, mtu) = network(s, p.networks[&p.summary.guests[g].nics[n].network]).await?;
            if VifRecord::new(&v).vm() != vr
                || VifRecord::new(&v).network() != nr
                || number(&VifRecord::new(&v).device())
                    != Some(u64::from(p.summary.guests[g].nics[n].device))
                || number(&VifRecord::new(&v).mtu()) != Some(mtu)
                || VifRecord::new(&v).currently_attached() != false
                || VifRecord::new(&v)
                    .mac()
                    .as_str()
                    .is_none_or(|m| !valid_mac(m))
                || VifRecord::new(&v).locking_mode() != "network_default"
                || VifRecord::new(&v)
                    .ipv4_allowed()
                    .as_array()
                    .is_none_or(|a| !a.is_empty())
                || VifRecord::new(&v)
                    .ipv6_allowed()
                    .as_array()
                    .is_none_or(|a| !a.is_empty())
                || VifRecord::new(&v).qos_algorithm_type() != ""
                || VifRecord::new(&v)
                    .qos_algorithm_params()
                    .as_object()
                    .is_none_or(|m| !m.is_empty())
            {
                return Err(Error::TransferConflict);
            }
        }
        _ => return Err(Error::TransferConflict),
    }
    Ok((actual, id))
}
pub(crate) async fn verify_guest(s: &Session, p: &ImportPlan, g: usize) -> Result<(), Error> {
    let (vr, v) = guest(s, p, g).await?;
    for (i, step) in p.steps.iter().enumerate() {
        if matches!(step.kind,Kind::Guest{guest}|Kind::Disk{guest,..}|Kind::Attachment{guest,..}|Kind::Nic{guest,..}if guest==g)
        {
            identify(s, p, i).await?;
        }
    }
    for (class, field) in [("VBD", "VBDs"), ("VIF", "VIFs")] {
        let expected = p
            .steps
            .iter()
            .filter(|st| {
                st.kind.class() == Some(class)
                    && match st.kind {
                        Kind::Attachment { guest, .. } | Kind::Nic { guest, .. } => guest == g,
                        _ => false,
                    }
            })
            .map(|s| s.reference.as_deref().ok_or(Error::TransferConflict))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
        let actual = v[field]
            .as_array()
            .filter(|a| a.len() <= 256)
            .ok_or(Error::TransferConflict)?
            .iter()
            .map(|v| v.as_str().ok_or(Error::TransferConflict))
            .collect::<Result<std::collections::BTreeSet<_>, _>>()?;
        if actual != expected {
            return Err(Error::TransferConflict);
        }
    }
    let hosts = s.call("VM.get_possible_hosts", vec![json!(vr)]).await?;
    let hosts = hosts
        .as_array()
        .filter(|h| !h.is_empty() && h.len() <= 1024)
        .ok_or(Error::NoCompatibleVmHost)?;
    for h in hosts {
        crate::xapi::opaque_reference(h)?;
    }
    Ok(())
}
pub(crate) async fn upload_target(
    s: &Session,
    pool: crate::PoolId,
    p: &ImportPlan,
    g: usize,
    d: usize,
) -> Result<crate::transfer::http::Prepared, Error> {
    guest(s, p, g).await?;
    let (r, _) = disk(s, p, g, d, true).await?;
    let mut route = crate::transfer::xapi::storage_route(s, pool, p.sr, true).await?;
    route.query = vec![
        ("vdi".into(), r),
        ("format".into(), p.disk(g, d)?.format.extension().into()),
    ];
    Ok(route)
}

fn valid_mac(mac: &str) -> bool {
    let bytes = mac
        .split(':')
        .map(|v| {
            if v.len() == 2 {
                u8::from_str_radix(v, 16).ok()
            } else {
                None
            }
        })
        .collect::<Option<Vec<_>>>();
    bytes.is_some_and(|b| b.len() == 6 && b[0] & 1 == 0 && b.iter().any(|v| *v != 0))
}
