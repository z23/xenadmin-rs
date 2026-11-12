use crate::{
    ActionAvailability, ClassAvailability, DeviceOperation, Error, Inventory, PowerState, SrId,
    VbdId, VdiId, VmId,
    device_lifecycle::{Plan, Prepared},
    xapi::Session,
};
use serde::{Deserialize, Serialize};
use serde_json::{Value, json};
use xenadmin_xapi::enumerations::{vbd_operations, vbd_type, vm_power_state};
use xenadmin_xapi::records::{PbdRecord, SrRecord, VbdRecord, VdiRecord, VmRecord};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum GuestToolsMediaSource {
    HostFlag,
    LegacyName,
}
impl GuestToolsMediaSource {
    pub fn label(self) -> &'static str {
        match self {
            Self::HostFlag => "Advertised by the host as guest tools",
            Self::LegacyName => "Recognized legacy tools filename",
        }
    }
}
pub(crate) fn source(record: &Value) -> Option<GuestToolsMediaSource> {
    if VdiRecord::new(record).is_tools_iso() == true {
        Some(GuestToolsMediaSource::HostFlag)
    } else if matches!(
        VdiRecord::new(record).name_label().as_str(),
        Some("xswindrivers.iso" | "xs-tools.iso" | "guest-tools.iso")
    ) {
        Some(GuestToolsMediaSource::LegacyName)
    } else {
        None
    }
}
fn known_error(reason: &str) -> ActionAvailability {
    ActionAvailability::Prerequisite {
        reason: reason.into(),
    }
}
pub(crate) fn availability(
    operation: &DeviceOperation,
    i: &Inventory,
    vm: VmId,
) -> ActionAvailability {
    let DeviceOperation::MountGuestTools {
        vbd,
        image,
        sr,
        previous,
    } = operation
    else {
        return ActionAvailability::Unavailable;
    };
    let Some(guest) = i.vms.get(&vm) else {
        return ActionAvailability::Stale;
    };
    if !matches!(guest.power_state, PowerState::Running | PowerState::Halted) {
        return known_error("Guest tools media requires a running or halted VM");
    }
    if let Some(operations) = i
        .details
        .vms
        .get(&vm)
        .map(|v| &v.current_operations)
        .filter(|ops| !ops.is_empty())
    {
        return ActionAvailability::Busy {
            operations: operations.clone(),
        };
    }
    if ["vbd", "vdi", "sr", "pbd"]
        .iter()
        .any(|c| i.details.classes.get(*c) != Some(&ClassAvailability::Available))
    {
        return ActionAvailability::Unknown;
    }
    let Some(cd) = i.details.vbds.get(vbd) else {
        return ActionAvailability::Unknown;
    };
    let Some(disk) = i.details.vdis.get(image) else {
        return ActionAvailability::Unknown;
    };
    if cd.vm != Some(vm)
        || cd.kind != "CD"
        || cd.empty != Some(previous.is_none())
        || cd.vdi != *previous
        || previous == &Some(*image)
    {
        return known_error("The reviewed CD drive or mounted image changed");
    }
    if !i.details.tools_media.contains_key(image)
        || disk.sr != Some(*sr)
        || disk.missing != Some(false)
        || disk.read_only != Some(true)
        || i.details
            .storage
            .get(sr)
            .is_none_or(|s| s.content_type != "iso")
    {
        return known_error("Select an available read-only guest tools ISO");
    }
    let reachable = i.details.pbds.values().any(|p| {
        p.sr == Some(*sr)
            && p.currently_attached == Some(true)
            && p.host.is_some_and(|h| {
                if guest.power_state == PowerState::Running {
                    guest.host == Some(h)
                } else {
                    i.hosts.get(&h).is_some_and(|h| h.enabled)
                }
            })
    });
    if !reachable {
        return known_error("Attach the tools ISO repository on the VM's host before mounting it");
    }
    let action = if previous.is_some() {
        "eject"
    } else {
        "insert"
    };
    if cd
        .allowed_operations
        .as_ref()
        .is_some_and(|ops| ops.iter().any(|op| op == action))
    {
        ActionAvailability::Available
    } else {
        ActionAvailability::Unavailable
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Step {
    Eject {
        cd: VbdId,
        previous: VdiId,
        image: VdiId,
        sr: SrId,
    },
    Insert {
        cd: VbdId,
        image: VdiId,
        sr: SrId,
    },
}
impl Step {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Eject { .. } => "Eject reviewed installation media",
            Self::Insert { .. } => "Mount selected guest tools ISO",
        }
    }
}
async fn fresh(session: &Session, class: &str, id: uuid::Uuid) -> Result<(String, Value), Error> {
    crate::snapshots::optional_record(session, class, &id.to_string())
        .await?
        .ok_or(Error::DeviceConflict)
}
fn idle(record: &Value) -> Result<(), Error> {
    if record["current_operations"]
        .as_object()
        .is_none_or(|m| !m.is_empty())
    {
        Err(Error::Busy)
    } else {
        Ok(())
    }
}
/// An idle CD drive whose host offers `operation` on it now.
fn permits(drive: &Value, operation: &str) -> Result<(), Error> {
    idle(drive)?;
    if crate::xapi::offers(VbdRecord::new(drive).allowed_operations(), operation) {
        Ok(())
    } else {
        Err(Error::UnavailableAction)
    }
}
async fn guest(session: &Session, vm: VmId, writing: bool) -> Result<(String, Value), Error> {
    let (reference, record) = fresh(session, "VM", vm.uuid()).await?;
    if VmRecord::new(&record).is_a_snapshot() != false
        || VmRecord::new(&record).is_a_template() != false
        || VmRecord::new(&record).is_control_domain() != false
    {
        return Err(Error::Stale);
    }
    if writing {
        if !matches!(
            VmRecord::new(&record).power_state().as_str(),
            Some(vm_power_state::RUNNING | vm_power_state::HALTED)
        ) {
            return Err(Error::UnavailableAction);
        }
        idle(&record)?;
    }
    Ok((reference, record))
}
async fn media(session: &Session, guest: &Value, image: VdiId, sr: SrId) -> Result<String, Error> {
    let (image_ref, disk) = fresh(session, "VDI", image.uuid()).await?;
    let (sr_ref, storage) = fresh(session, "SR", sr.uuid()).await?;
    if source(&disk).is_none()
        || VdiRecord::new(&disk).sr() != sr_ref
        || VdiRecord::new(&disk).missing() != false
        || VdiRecord::new(&disk).read_only() != true
        || SrRecord::new(&storage).content_type() != "iso"
    {
        return Err(Error::DeviceConflict);
    }
    idle(&disk)?;
    let pbds = session.call("PBD.get_all_records", vec![]).await?;
    let pbds = pbds
        .as_object()
        .filter(|r| r.len() <= 10_000)
        .ok_or(Error::DeviceConflict)?;
    let mut reachable = false;
    for pbd in pbds.values().filter(|p| {
        PbdRecord::new(p).sr() == sr_ref && PbdRecord::new(p).currently_attached() == true
    }) {
        let host = crate::snapshots::reference(&pbd["host"])?;
        if VmRecord::new(guest).power_state() == vm_power_state::RUNNING {
            if VmRecord::new(guest).resident_on() == host {
                reachable = true;
                break;
            }
        } else {
            let record = session.call("host.get_record", vec![json!(host)]).await?;
            if record["enabled"] == true {
                reachable = true;
                break;
            }
        }
    }
    if !reachable {
        return Err(Error::DeviceInUse);
    }
    Ok(image_ref)
}
async fn cd(
    session: &Session,
    vm: &str,
    id: VbdId,
    previous: Option<VdiId>,
) -> Result<String, Error> {
    let (reference, record) = fresh(session, "VBD", id.uuid()).await?;
    if VbdRecord::new(&record).vm() != vm
        || VbdRecord::new(&record).type_() != vbd_type::CD
        || VbdRecord::new(&record).empty() != previous.is_none()
    {
        return Err(Error::DeviceConflict);
    }
    if let Some(previous) = previous {
        let (previous_ref, _) = fresh(session, "VDI", previous.uuid()).await?;
        if VbdRecord::new(&record).vdi() != previous_ref {
            return Err(Error::DeviceConflict);
        }
    } else if VbdRecord::new(&record).vdi() != "OpaqueRef:NULL" {
        return Err(Error::DeviceConflict);
    }
    permits(
        &record,
        if previous.is_some() {
            vbd_operations::EJECT
        } else {
            vbd_operations::INSERT
        },
    )?;
    Ok(reference)
}
pub(crate) async fn plan(
    session: &Session,
    vm: VmId,
    operation: &DeviceOperation,
) -> Result<Plan, Error> {
    let DeviceOperation::MountGuestTools {
        vbd,
        image,
        sr,
        previous,
    } = operation
    else {
        return Err(Error::InvalidDeviceOperation);
    };
    if previous == &Some(*image) {
        return Err(Error::InvalidDeviceConfiguration);
    }
    let (vm_ref, record) = guest(session, vm, true).await?;
    media(session, &record, *image, *sr).await?;
    cd(session, &vm_ref, *vbd, *previous).await?;
    let mut steps = Vec::new();
    if let Some(previous) = previous {
        steps.push(crate::device_lifecycle::Step::GuestTools(Step::Eject {
            cd: *vbd,
            previous: *previous,
            image: *image,
            sr: *sr,
        }));
    }
    steps.push(crate::device_lifecycle::Step::GuestTools(Step::Insert {
        cd: *vbd,
        image: *image,
        sr: *sr,
    }));
    let evidence = vec![None; steps.len()];
    Ok(Plan { steps, evidence })
}
pub(crate) async fn prepare(session: &Session, vm: VmId, step: &Step) -> Result<Prepared, Error> {
    let (vm_ref, record) = guest(session, vm, true).await?;
    match step {
        Step::Eject {
            cd: id,
            previous,
            image,
            sr,
        } => {
            media(session, &record, *image, *sr).await?;
            let reference = cd(session, &vm_ref, *id, Some(*previous)).await?;
            Ok(Prepared {
                method: "VBD.eject",
                parameters: vec![json!(reference)],
            })
        }
        Step::Insert { cd: id, image, sr } => {
            let image = media(session, &record, *image, *sr).await?;
            let reference = cd(session, &vm_ref, *id, None).await?;
            Ok(Prepared {
                method: "VBD.insert",
                parameters: vec![json!(reference), json!(image)],
            })
        }
    }
}
pub(crate) async fn verify(session: &Session, vm: VmId, step: &Step) -> Result<bool, Error> {
    let (vm_ref, _) = guest(session, vm, false).await?;
    let id = match step {
        Step::Eject { cd, .. } | Step::Insert { cd, .. } => *cd,
    };
    let (_, record) = fresh(session, "VBD", id.uuid()).await?;
    if VbdRecord::new(&record).vm() != vm_ref || VbdRecord::new(&record).type_() != vbd_type::CD {
        return Err(Error::DeviceConflict);
    }
    match step {
        Step::Eject { .. } => Ok(VbdRecord::new(&record).empty() == true
            && VbdRecord::new(&record).vdi() == "OpaqueRef:NULL"),
        Step::Insert { image, sr, .. } => {
            let (image_ref, disk) = fresh(session, "VDI", image.uuid()).await?;
            let (sr_ref, _) = fresh(session, "SR", sr.uuid()).await?;
            Ok(VbdRecord::new(&record).empty() == false
                && VbdRecord::new(&record).vdi() == image_ref
                && disk["SR"] == sr_ref)
        }
    }
}
