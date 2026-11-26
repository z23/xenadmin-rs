use super::*;
use crate::model::*;
use std::collections::BTreeSet;
use uuid::Uuid;
use xenadmin_xapi::enumerations::{primary_address_type, vm_operations};
use xenadmin_xapi::records::{
    BondRecord, HostMetricsRecord, HostRecord, MessageRecord, NetworkRecord, PbdRecord,
    PifMetricsRecord, PifRecord, PoolRecord, SmRecord, SrRecord, VbdRecord, VdiRecord, VifRecord,
    VlanRecord, VmApplianceRecord, VmGuestMetricsRecord, VmMetricsRecord, VmssRecord,
};
use xenadmin_xapi::{
    Fits, FlagField, IsClass, MapField, RefField, SetField, TextField, Textual, WholeField, class,
};
use xenadmin_xapi::{Reads, kind};
#[path = "xapi_network_fabric.rs"]
mod network_fabric;

#[derive(Clone, Default)]
pub(super) struct References {
    classes: BTreeMap<String, Arc<BTreeMap<String, Uuid>>>,
    snapshots: Arc<BTreeSet<String>>,
    duplicate_uuids: ClassMask,
}

impl References {
    pub(super) fn new(store: &Store) -> Self {
        let mut refs = Self::default();
        for (class, records) in &store.records {
            let mask = ClassMask::lookup(class).map(|(m, _, _, _)| m);
            let mut ids = BTreeMap::new();
            let mut seen_uuids = BTreeSet::new();
            let mut has_dup = false;
            for (reference, record) in records.iter() {
                if let Some(id) = record["uuid"].as_str().and_then(|value| value.parse().ok()) {
                    if !seen_uuids.insert(id) {
                        has_dup = true;
                    }
                    ids.insert(reference.clone(), id);
                    if class == "vm" && VmRecord::new(record.as_ref()).is_a_snapshot() == true {
                        Arc::make_mut(&mut refs.snapshots).insert(reference.clone());
                    }
                }
            }
            if has_dup && let Some(mask) = mask {
                refs.duplicate_uuids |= mask;
            }
            refs.classes.insert(class.clone(), Arc::new(ids));
        }
        refs
    }

    pub(super) fn rebuild_class(&mut self, store: &Store, class: &str, mask: ClassMask) {
        let mut ids = BTreeMap::new();
        let mut seen_uuids = BTreeSet::new();
        let mut has_dup = false;
        let is_vm = mask == ClassMask::VM;
        let mut snapshots = is_vm.then(BTreeSet::new);
        for (reference, record) in store.class(class) {
            if let Some(id) = record["uuid"].as_str().and_then(|value| value.parse().ok()) {
                if !seen_uuids.insert(id) {
                    has_dup = true;
                }
                ids.insert(reference.clone(), id);
                if let Some(snaps) = snapshots.as_mut()
                    && VmRecord::new(record).is_a_snapshot() == true
                {
                    snaps.insert(reference.clone());
                }
            }
        }
        if has_dup {
            self.duplicate_uuids |= mask;
        } else {
            self.duplicate_uuids = ClassMask(self.duplicate_uuids.0 & !mask.0);
        }
        if let Some(snaps) = snapshots {
            self.snapshots = Arc::new(snaps);
        }
        if ids.is_empty() && !store.records.contains_key(class) {
            self.classes.remove(class);
        } else {
            self.classes.insert(class.to_owned(), Arc::new(ids));
        }
    }

    pub(super) fn class_map(&self, class: &str) -> &BTreeMap<String, Uuid> {
        static EMPTY: std::sync::OnceLock<BTreeMap<String, Uuid>> = std::sync::OnceLock::new();
        self.classes
            .get(class)
            .map(Arc::as_ref)
            .unwrap_or_else(|| EMPTY.get_or_init(BTreeMap::new))
    }

    /// The object a reference field names. The class the API gives the field says where
    /// to look, so a reference is not looked up among the objects of another class.
    pub(super) fn link<T: From<Uuid>, C: IsClass>(&self, reference: RefField<'_, C>) -> Option<T> {
        self.linked(&C::NAME.to_ascii_lowercase(), reference)
    }
    /// The object a reference names, for a reference that is an element of a field.
    pub(super) fn linked<'a, T: From<Uuid>>(
        &self,
        class: &str,
        reference: impl Reads<'a, kind::Reference>,
    ) -> Option<T> {
        let reference = reference.raw();
        self.classes
            .get(class)?
            .get(reference.as_str()?)
            .copied()
            .map(T::from)
    }
    fn vm<C: Fits<class::Vm>>(&self, reference: RefField<'_, C>) -> Option<VmId> {
        if self.snapshots.contains(reference.reference()?) {
            return None;
        }
        self.linked("vm", reference)
    }
    fn snapshot<C: Fits<class::Vm>>(
        &self,
        reference: RefField<'_, C>,
    ) -> Option<crate::SnapshotId> {
        if !self.snapshots.contains(reference.reference()?) {
            return None;
        }
        self.linked("vm", reference)
    }
    fn snapshot_parent<C: Fits<class::Vm>>(&self, value: RefField<'_, C>) -> crate::SnapshotParent {
        if value == "OpaqueRef:NULL" {
            crate::SnapshotParent::Root
        } else if let Some(id) = self.snapshot(value) {
            crate::SnapshotParent::Snapshot(id)
        } else if let Some(id) = self.vm(value) {
            crate::SnapshotParent::Vm(id)
        } else {
            crate::SnapshotParent::Unresolved
        }
    }
    fn links<T: From<Uuid>>(&self, class: &str, references: SetField<'_>) -> Vec<T> {
        references
            .members()
            .into_iter()
            .flatten()
            .filter_map(|r| self.linked(class, r))
            .collect()
    }
}

fn id<T: From<Uuid>>(uuid: TextField<'_>) -> Result<T, Error> {
    required(uuid)?
        .parse::<Uuid>()
        .map(T::from)
        .map_err(|_| Error::Protocol("invalid optional inventory UUID"))
}
fn text_option<'a>(value: impl Textual<'a>) -> Option<String> {
    value
        .text()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
fn map_bool(other_config: MapField<'_>, key: &str) -> Option<bool> {
    match other_config.entries()?.get(key) {
        None => Some(false),
        Some(value) => match value.as_str()? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        },
    }
}
pub(crate) fn decode_server_message(record: &Value) -> Result<ServerMessage, Error> {
    let view = MessageRecord::new(record);
    Ok(ServerMessage {
        id: id(view.uuid())?,
        name: string(view.name()),
        priority: integer(view.priority(), "message.priority")?,
        object_class: string(view.cls()),
        object_uuid: view.obj_uuid().text().and_then(|s| s.parse().ok()),
        timestamp: text_option(view.timestamp()),
        body: string(view.body()),
    })
}
fn strings(value: SetField<'_>) -> Option<Vec<String>> {
    value
        .members()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}
fn string_map(value: MapField<'_>) -> BTreeMap<String, String> {
    value
        .entries()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_owned())))
        .collect()
}

fn blocked_operations(value: MapField<'_>) -> BTreeMap<String, String> {
    value
        .entries()
        .into_iter()
        .flatten()
        .filter_map(|(operation, reason)| {
            reason
                .as_str()
                .map(|reason| (operation.clone(), blocked_reason(reason)))
        })
        .collect()
}

fn blocked_reason(reason: &str) -> String {
    // XAPI uses this field as the administrator's message parameter of OPERATION_BLOCKED.
    let mut characters = reason.chars().filter(|c| {
        (!c.is_control() || c.is_whitespace())
            && !matches!(c, '\u{061c}' | '\u{200e}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
    });
    let bounded: String = characters.by_ref().take(1024).collect();
    let mut display = bounded.split_whitespace().collect::<Vec<_>>().join(" ");
    if display.is_empty() {
        display = "No reason supplied by the host".into();
    }
    if characters.next().is_some() {
        display.push('…');
    }
    display
}

pub(super) fn power_availability(record: &Value, action: crate::PowerAction) -> ActionAvailability {
    let view = VmRecord::new(record);
    VmDetails {
        allowed_operations: strings(view.allowed_operations()),
        current_operations: view
            .current_operations()
            .entries()
            .into_iter()
            .flatten()
            .filter_map(|(_, operation)| {
                operation.as_str().map(|operation| RunningOperation {
                    task: None,
                    operation: operation.into(),
                })
            })
            .collect(),
        blocked_operations: blocked_operations(view.blocked_operations()),
        ..Default::default()
    }
    .power_availability(action, true)
}
fn size(value: WholeField<'_>, field: &'static str) -> Result<Option<u64>, Error> {
    storage_number(value, field)
}
fn boolean(value: FlagField<'_>) -> Result<Option<bool>, Error> {
    match value.value() {
        Value::Null => Ok(None),
        Value::Bool(value) => Ok(Some(*value)),
        _ => Err(Error::Protocol("invalid optional inventory boolean")),
    }
}
fn reset_optional_class_state(
    store: &Store,
    class: &str,
    states: &mut BTreeMap<String, ClassAvailability>,
) {
    let state = store
        .classes
        .get(class)
        .cloned()
        .unwrap_or(ClassAvailability::Unknown);
    states.insert(class.into(), state);
}

fn reset_required_class_state(
    store: &Store,
    class: &str,
    states: &mut BTreeMap<String, ClassAvailability>,
) {
    match store.classes.get(class) {
        Some(state) => {
            states.insert(class.into(), state.clone());
        }
        None => {
            states.remove(class);
        }
    }
}

fn records<I: Ord, R>(
    store: &Store,
    class: &str,
    states: &mut BTreeMap<String, ClassAvailability>,
    decode: impl Fn(&Value) -> Result<(I, R), Error>,
) -> BTreeMap<I, R> {
    if matches!(
        states.get(class),
        Some(ClassAvailability::Unsupported | ClassAvailability::PermissionDenied)
    ) {
        return BTreeMap::new();
    }
    let mut result = BTreeMap::new();
    for (_, value) in store.class(class) {
        match decode(value) {
            Ok((id, value)) => {
                result.insert(id, value);
            }
            Err(_) => {
                states.insert(class.into(), ClassAvailability::InvalidData);
                return BTreeMap::new();
            }
        }
    }
    if !result.is_empty() {
        states.insert(class.into(), ClassAvailability::Available);
    }
    result
}

fn decode_sr_details(record: &Value) -> Result<(SrId, StorageDetails), Error> {
    let view = SrRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        StorageDetails {
            read_cache_override: view.other_config()["o_direct"]
                .as_str()
                .filter(|v| matches!(*v, "true" | "false"))
                .map(str::to_owned),
            read_cache_policy_known: view.other_config().entries().is_some_and(|m| {
                !m.contains_key("o_direct")
                    || matches!(m["o_direct"].as_str(), Some("true" | "false"))
            }),
            id,
            description: string(view.name_description()),
            content_type: string(view.content_type()),
            shared: boolean(view.shared())?,
            virtual_allocation: size(view.virtual_allocation(), "SR.virtual_allocation")?,
            allowed_operations: strings(view.allowed_operations()),
            tags: strings(view.tags()).unwrap_or_default(),
        },
    ))
}

fn decode_network_details(record: &Value) -> Result<(NetworkId, NetworkDetails), Error> {
    let view = NetworkRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        NetworkDetails {
            purposes: crate::network_service::purposes(view.purpose()),
            automatic: view.other_config()["automatic"].as_str().map(str::to_owned),
            automatic_known: view
                .other_config()
                .entries()
                .is_some_and(|m| m.get("automatic").is_none_or(Value::is_string)),
            id,
            description: string(view.name_description()),
            mtu: size(view.mtu(), "network.MTU")?,
            managed: boolean(view.managed())?,
            default_locking_mode: text_option(view.default_locking_mode()),
            allowed_operations: strings(view.allowed_operations()),
            tags: strings(view.tags()).unwrap_or_default(),
        },
    ))
}

fn decode_task(record: &Value, refs: &References) -> Result<(TaskId, ServerTask), Error> {
    let view = TaskRecord::new(record);
    let id = id(view.uuid())?;
    let progress = view
        .progress()
        .number()
        .filter(|n| n.is_finite() && (0.0..=1.0).contains(n));
    Ok((
        id,
        ServerTask {
            id,
            name: label(view.name_label()),
            description: string(view.name_description()),
            status: string(view.status()),
            progress,
            created: text_option(view.created()),
            finished: text_option(view.finished()),
            error_code: view
                .error_info()
                .members()
                .and_then(|values| values.first())
                .and_then(Value::as_str)
                .map(|code| safe_error_code(code).to_owned()),
            allowed_operations: strings(view.allowed_operations()),
            resident_host: refs.link(view.resident_on()),
            parent: refs.link(view.subtask_of()),
            subtasks: refs.links("task", view.subtasks()),
            result_type: text_option(view.type_()),
        },
    ))
}

fn decode_vdi(record: &Value, refs: &References) -> Result<(VdiId, VirtualDisk), Error> {
    let view = VdiRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        VirtualDisk {
            id,
            name: label(view.name_label()),
            description: string(view.name_description()),
            sr: refs.link(view.sr()),
            virtual_size: size(view.virtual_size(), "VDI.virtual_size")?,
            physical_utilisation: size(view.physical_utilisation(), "VDI.physical_utilisation")?,
            kind: string(view.type_()),
            read_only: boolean(view.read_only())?,
            sharable: boolean(view.sharable())?,
            managed: boolean(view.managed())?,
            missing: boolean(view.missing())?,
            is_snapshot: boolean(view.is_a_snapshot())?,
            snapshot_of: refs.link(view.snapshot_of()),
            cbt_enabled: boolean(view.cbt_enabled())?,
            allowed_operations: strings(view.allowed_operations()),
        },
    ))
}

fn decode_vbd(record: &Value, refs: &References) -> Result<(VbdId, VirtualBlockDevice), Error> {
    let view = VbdRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        VirtualBlockDevice {
            qos: crate::DeviceQos::from_record(record),
            qos_supported_algorithms: strings(view.qos_supported_algorithms()),
            id,
            vm: refs.vm(view.vm()),
            snapshot: refs.snapshot(view.vm()),
            vdi: refs.link(view.vdi()),
            device: string(view.device()),
            userdevice: string(view.userdevice()),
            kind: string(view.type_()),
            mode: string(view.mode()),
            currently_attached: boolean(view.currently_attached())?,
            bootable: boolean(view.bootable())?,
            empty: boolean(view.empty())?,
            owned: view
                .other_config()
                .get("owner")
                .is_some_and(Value::is_string),
            allowed_operations: strings(view.allowed_operations()),
        },
    ))
}

fn decode_vif(record: &Value, refs: &References) -> Result<(VifId, VirtualInterface), Error> {
    let view = VifRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        VirtualInterface {
            qos_known: crate::DeviceQos::from_record(record).is_some(),
            qos_supported_algorithms: strings(view.qos_supported_algorithms()),
            id,
            vm: refs.vm(view.vm()),
            network: refs.link(view.network()),
            device: string(view.device()),
            mac: string(view.mac()),
            mtu: size(view.mtu(), "VIF.MTU")?,
            currently_attached: boolean(view.currently_attached())?,
            qos_algorithm: string(view.qos_algorithm_type()),
            qos_parameters: string_map(view.qos_algorithm_params()),
            allowed_operations: strings(view.allowed_operations()),
        },
    ))
}

fn decode_pif(record: &Value, refs: &References) -> Result<(PifId, PhysicalInterface), Error> {
    let view = PifRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        PhysicalInterface {
            metrics: refs.link(view.metrics()),
            capabilities: strings(view.capabilities()).filter(|values| {
                values.len() <= 64
                    && values
                        .iter()
                        .all(|v| v.len() <= 128 && !v.chars().any(char::is_control))
            }),
            managed: boolean(view.managed())?,
            disallow_unplug: boolean(view.disallow_unplug())?,
            management_purpose: view.other_config()["management_purpose"]
                .as_str()
                .filter(|v| v.len() <= 256 && !v.chars().any(char::is_control))
                .map(str::to_owned),
            primary_address_type: view
                .primary_address_type()
                .name()
                .filter(|v| [primary_address_type::IPV4, primary_address_type::IPV6].contains(v))
                .map(str::to_owned),
            id,
            host: refs.link(view.host()),
            network: refs.link(view.network()),
            device: string(view.device()),
            mac: string(view.mac()),
            mtu: size(view.mtu(), "PIF.MTU")?,
            vlan: integer(view.vlan(), "PIF.VLAN")?,
            management: boolean(view.management())?,
            physical: boolean(view.physical())?,
            currently_attached: boolean(view.currently_attached())?,
            ip_configuration_mode: string(view.ip_configuration_mode()),
            ip: string(view.ip()),
            netmask: string(view.netmask()),
            gateway: string(view.gateway()),
            dns: string(view.dns()),
            ipv6: strings(view.ipv6())
                .unwrap_or_default()
                .into_iter()
                .filter(|address| !address.is_empty())
                .collect(),
            ipv6_configuration_mode: string(view.ipv6_configuration_mode()),
            ipv6_gateway: string(view.ipv6_gateway()),
        },
    ))
}

fn decode_pbd(
    record: &Value,
    store: &Store,
    refs: &References,
) -> Result<(PbdId, StorageAttachment), Error> {
    let view = PbdRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        StorageAttachment {
            id,
            location: crate::storage::safe_location_for_kind(
                view.device_config(),
                view.sr().reference().and_then(|reference| {
                    SrRecord::new(store.record("sr", reference)?).type_().text()
                }),
            ),
            requires_credentials: crate::storage::requires_credentials(view.device_config()),
            multipathed: view.other_config()["multipathed"]
                .as_str()
                .and_then(|s| s.parse().ok()),
            paths: crate::storage::path_counts(view.other_config()),
            host: refs.link(view.host()),
            sr: refs.link(view.sr()),
            currently_attached: boolean(view.currently_attached())?,
        },
    ))
}

fn decode_pif_metrics(record: &Value) -> Result<(PifMetricsId, PifMetrics), Error> {
    let view = PifMetricsRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        PifMetrics {
            id,
            carrier: boolean(view.carrier())?,
            speed_mbps: size(view.speed(), "PIF_metrics.speed")?.filter(|v| *v > 0),
            full_duplex: boolean(view.duplex())?,
            vendor_id: string(view.vendor_id()),
            vendor_name: string(view.vendor_name()),
            device_id: string(view.device_id()),
            device_name: string(view.device_name()),
            pci_bus_path: string(view.pci_bus_path()),
            last_updated: text_option(view.last_updated()),
        },
    ))
}

fn decode_bond(record: &Value, refs: &References) -> Result<(BondId, NetworkBond), Error> {
    let view = BondRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        NetworkBond {
            primary_slave: refs.link(view.primary_slave()),
            hashing_algorithm: view.properties()["hashing_algorithm"]
                .as_str()
                .map(str::to_owned),
            hashing_algorithm_known: view
                .properties()
                .entries()
                .is_some_and(|m| m.get("hashing_algorithm").is_none_or(Value::is_string)),
            id,
            master: refs.link(view.master()),
            slaves: refs.links("pif", view.slaves()),
            mode: string(view.mode()),
            links_up: size(view.links_up(), "Bond.links_up")?,
        },
    ))
}

fn decode_vlan(record: &Value, refs: &References) -> Result<(VlanId, Vlan), Error> {
    let view = VlanRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        Vlan {
            id,
            tagged_pif: refs.link(view.tagged_pif()),
            untagged_pif: refs.link(view.untagged_pif()),
            tag: size(view.tag(), "VLAN.tag")?,
        },
    ))
}

fn decode_guest_metrics(record: &Value) -> Result<(GuestMetricsId, GuestMetrics), Error> {
    let view = VmGuestMetricsRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        GuestMetrics {
            id,
            os_version: string_map(view.os_version()),
            tools_version: string_map(view.pv_drivers_version()),
            networks: string_map(view.networks()),
            live: boolean(view.live())?,
            last_updated: text_option(view.last_updated()),
        },
    ))
}

fn decode_vm_metrics(record: &Value) -> Result<(VmMetricsId, VmMetrics), Error> {
    let view = VmMetricsRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        VmMetrics {
            install_time: text_option(view.install_time()),
            last_updated: text_option(view.last_updated()),
            state: strings(view.state()).unwrap_or_default(),
            current_domain_type: text_option(view.current_domain_type()),
            id,
            memory_actual: size(view.memory_actual(), "VM_metrics.memory_actual")?,
            vcpus_number: size(view.vcpus_number(), "VM_metrics.VCPUs_number")?,
            start_time: text_option(view.start_time()),
        },
    ))
}

fn decode_host_metrics(record: &Value) -> Result<(HostMetricsId, HostMetrics), Error> {
    let view = HostMetricsRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        HostMetrics {
            id,
            live: boolean(view.live())?,
            memory_total: size(view.memory_total(), "host_metrics.memory_total")?,
            memory_free: size(view.memory_free(), "host_metrics.memory_free")?,
            last_updated: text_option(view.last_updated()),
        },
    ))
}

fn decode_appliance(
    record: &Value,
    refs: &References,
) -> Result<(ApplianceId, VmAppliance), Error> {
    let view = VmApplianceRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        VmAppliance {
            vms_known: view
                .vms()
                .members()
                .is_some_and(|v| v.iter().all(|r| refs.vm(RefField::of(r)).is_some())),
            id,
            name: label(view.name_label()),
            description: string(view.name_description()),
            vms: view
                .vms()
                .members()
                .into_iter()
                .flatten()
                .filter_map(|r| refs.vm(RefField::of(r)))
                .collect(),
            allowed_operations: strings(view.allowed_operations()),
        },
    ))
}

fn decode_snapshot_policy(
    record: &Value,
    refs: &References,
) -> Result<(SnapshotPolicyId, SnapshotPolicy), Error> {
    let view = VmssRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        SnapshotPolicy {
            vms_known: view
                .vms()
                .members()
                .is_some_and(|v| v.iter().all(|r| refs.vm(RefField::of(r)).is_some())),
            last_run_time: view.last_run_time().text().map(str::to_owned),
            id,
            name: label(view.name_label()),
            description: string(view.name_description()),
            enabled: boolean(view.enabled())?,
            kind: string(view.type_()),
            frequency: string(view.frequency()),
            retained_snapshots: size(view.retained_snapshots(), "VMSS.retained_snapshots")?,
            schedule: string_map(view.schedule()),
            vms: view
                .vms()
                .members()
                .into_iter()
                .flatten()
                .filter_map(|r| refs.vm(RefField::of(r)))
                .collect(),
        },
    ))
}

fn decode_storage_manager(record: &Value) -> Result<(StorageManagerId, StorageManager), Error> {
    let view = SmRecord::new(record);
    let id = id(view.uuid())?;
    Ok((
        id,
        StorageManager {
            id,
            kind: string(view.type_()),
            name: label(view.name_label()),
            capabilities: strings(view.capabilities()).unwrap_or_default(),
        },
    ))
}

fn decode_snapshot_details(
    record: &Value,
    refs: &References,
) -> Option<(SnapshotId, crate::SnapshotDetails)> {
    let view = VmRecord::new(record);
    let id = id(view.uuid()).ok()?;
    Some((
        id,
        crate::SnapshotDetails {
            can_export: view.allowed_operations().members().and_then(|_| {
                view.current_operations()
                    .entries()
                    .map(|ops| ops.is_empty() && advertises(record, vm_operations::EXPORT))
            }),
            parent: refs.snapshot_parent(view.parent()),
            memory_vdi: refs.link(view.suspend_vdi()),
            memory_vdi_known: view.suspend_vdi() == "OpaqueRef:NULL"
                || refs.link::<VdiId, _>(view.suspend_vdi()).is_some(),
            preview_available: view.blobs()[crate::snapshot_preview::KEY]
                .as_str()
                .is_some_and(|s| s != "OpaqueRef:NULL")
                || view.other_config()[crate::snapshot_preview::KEY]
                    .as_str()
                    .is_some_and(|s| !s.is_empty()),
        },
    ))
}

fn decode_vm_details(record: &Value, refs: &References) -> Option<(VmId, VmDetails)> {
    let view = VmRecord::new(record);
    let id = id(view.uuid()).ok()?;
    Some((
        id,
        VmDetails {
            snapshot_parent: refs.snapshot_parent(view.parent()),
            domain_type: text_option(view.domain_type()),
            cores_per_socket: text_option(TextField::of(&view.platform()["cores-per-socket"])),
            topology_known: view
                .platform()
                .entries()
                .is_some_and(|m| m.get("cores-per-socket").is_none_or(Value::is_string)),
            cpu_weight: text_option(TextField::of(&view.vcpus_params()["weight"])),
            cpu_weight_known: view
                .vcpus_params()
                .entries()
                .is_some_and(|m| m.get("weight").is_none_or(Value::is_string)),
            shadow_multiplier: view
                .hvm_shadow_multiplier()
                .number()
                .filter(|v| v.is_finite())
                .map(|v| v.to_string()),
            is_default_template: crate::guest_boot::default_template(record),
            allowed_operations: strings(view.allowed_operations()),
            current_operations: view
                .current_operations()
                .entries()
                .into_iter()
                .flatten()
                .filter_map(|(task, operation)| {
                    operation.as_str().map(|operation| RunningOperation {
                        task: refs.linked("task", &Value::String(task.clone())),
                        operation: operation.into(),
                    })
                })
                .collect(),
            blocked_operations: blocked_operations(view.blocked_operations()),
            vcpus_max: size(view.vcpus_max(), "VM.VCPUs_max").ok().flatten(),
            vcpus_at_startup: size(view.vcpus_at_startup(), "VM.VCPUs_at_startup")
                .ok()
                .flatten(),
            memory_static_min: size(view.memory_static_min(), "VM.memory_static_min")
                .ok()
                .flatten(),
            memory_static_max: size(view.memory_static_max(), "VM.memory_static_max")
                .ok()
                .flatten(),
            memory_dynamic_min: size(view.memory_dynamic_min(), "VM.memory_dynamic_min")
                .ok()
                .flatten(),
            memory_dynamic_max: size(view.memory_dynamic_max(), "VM.memory_dynamic_max")
                .ok()
                .flatten(),
            memory_target: size(view.memory_target(), "VM.memory_target")
                .ok()
                .flatten(),
            boot_order: text_option(TextField::of(&view.hvm_boot_params()["order"])),
            boot_policy: text_option(view.hvm_boot_policy()),
            firmware: text_option(TextField::of(&view.hvm_boot_params()["firmware"]))
                .or_else(|| crate::guest_boot::firmware(record).map(str::to_owned)),
            secure_boot: crate::guest_boot::policy(record),
            bios_identity_empty: view.bios_strings().entries().map(|v| v.is_empty()),
            affinity: refs.link(view.affinity()),
            affinity_known: view.affinity() == "OpaqueRef:NULL"
                || refs.link::<HostId, _>(view.affinity()).is_some(),
            guest_metrics: refs.link(view.guest_metrics()),
            metrics: refs.link(view.metrics()),
            suspend_vdi: refs.link(view.suspend_vdi()),
            appliance: refs.link(view.appliance()),
            appliance_known: view.appliance() == "OpaqueRef:NULL"
                || refs.link::<ApplianceId, _>(view.appliance()).is_some(),
            appliance_order: (|| {
                Some(crate::ApplianceOrder {
                    order: view.order().number()?,
                    start_delay: size(view.start_delay(), "VM.start_delay").ok()??,
                    shutdown_delay: size(view.shutdown_delay(), "VM.shutdown_delay").ok()??,
                })
            })(),
            snapshot_policy: refs.link(view.snapshot_schedule()),
            tags: strings(view.tags()).unwrap_or_default(),
            ha_restart_priority: view.ha_restart_priority().text().map(str::to_owned),
            auto_power_on: view.other_config()["auto_poweron"]
                .as_str()
                .and_then(|v| v.parse().ok()),
        },
    ))
}

fn decode_host_details(record: &Value, refs: &References) -> Option<(HostId, HostDetails)> {
    let view = HostRecord::new(record);
    let id = id(view.uuid()).ok()?;
    Some((
        id,
        HostDetails {
            bios_strings: string_map(view.bios_strings()),
            chipset_info: string_map(view.chipset_info()),
            external_auth_type: text_option(view.external_auth_type()),
            external_auth_service_name: text_option(view.external_auth_service_name()),
            allowed_operations: strings(view.allowed_operations()),
            current_operations: view.current_operations().entries().map(|ops| {
                ops.iter()
                    .map(|(task, operation)| RunningOperation {
                        task: refs.linked("task", &Value::String(task.clone())),
                        operation: operation.as_str().unwrap_or("Unknown").into(),
                    })
                    .collect()
            }),
            maintenance_mode: map_bool(view.other_config(), "MAINTENANCE_MODE"),
            boot_time: view.other_config()["boot_time"]
                .as_str()
                .filter(|s| crate::host_types::valid_epoch(s))
                .map(str::to_owned),
            agent_start_time: view.other_config()["agent_start_time"]
                .as_str()
                .filter(|s| crate::host_types::valid_epoch(s))
                .map(str::to_owned),
            control_domain: refs.vm(view.control_domain()),
            power_configuration: crate::host_power::configuration(record),
            power_on_mode: view.power_on_mode().text().map(str::to_owned),
            power_on_config_empty: view
                .power_on_config()
                .entries()
                .map(|config| config.is_empty()),
            syslog_destination: view.logging()["syslog_destination"]
                .as_str()
                .filter(|s| !s.is_empty())
                .map(str::to_owned),
            syslog_known: view.logging().entries().is_some_and(|map| {
                map.get("syslog_destination")
                    .is_none_or(|value| value.is_string())
            }),
            multipathing: view.multipathing().get(),
            local_cache_sr: refs.link(view.local_cache_sr()),
            standard_multipath_handler: view.other_config().entries().and_then(|m| {
                match m.get("multipathhandle") {
                    None => Some(false),
                    Some(v) if v == "dmp" => Some(true),
                    _ => None,
                }
            }),
            description: string(view.name_description()),
            hostname: string(view.hostname()),
            metrics: refs.link(view.metrics()),
            software_version: string_map(view.software_version()),
            cpu_info: string_map(view.cpu_info()),
            capabilities: strings(view.capabilities()).unwrap_or_default(),
            tags: strings(view.tags()).unwrap_or_default(),
            enabled: view.enabled().get(),
        },
    ))
}

fn rebuild_sr_disks(result: &mut InventoryDetails) {
    result.sr_disks.clear();
    for (id, disk) in &result.vdis {
        if let Some(sr) = disk.sr {
            result.sr_disks.entry(sr).or_default().push(*id);
        }
    }
}

fn rebuild_vbd_indexes(result: &mut InventoryDetails) {
    result.vm_disks.clear();
    result.snapshot_disks.clear();
    result.vdi_attachments.clear();
    for (id, disk) in &result.vbds {
        if let Some(vm) = disk.vm {
            result.vm_disks.entry(vm).or_default().push(*id);
        }
        if let Some(snapshot) = disk.snapshot {
            result.snapshot_disks.entry(snapshot).or_default().push(*id);
        }
        if let Some(vdi) = disk.vdi {
            result.vdi_attachments.entry(vdi).or_default().push(*id);
        }
    }
}

fn rebuild_vif_indexes(result: &mut InventoryDetails) {
    result.vm_interfaces.clear();
    result.network_interfaces.clear();
    for (id, interface) in &result.vifs {
        if let Some(vm) = interface.vm {
            result.vm_interfaces.entry(vm).or_default().push(*id);
        }
        if let Some(network) = interface.network {
            result
                .network_interfaces
                .entry(network)
                .or_default()
                .push(*id);
        }
    }
}

fn rebuild_pif_indexes(result: &mut InventoryDetails) {
    result.host_interfaces.clear();
    result.network_uplinks.clear();
    for (id, interface) in &result.pifs {
        if let Some(host) = interface.host {
            result.host_interfaces.entry(host).or_default().push(*id);
        }
        if let Some(network) = interface.network {
            result.network_uplinks.entry(network).or_default().push(*id);
        }
    }
}

fn rebuild_pbd_indexes(result: &mut InventoryDetails) {
    result.sr_attachments.clear();
    for (id, attachment) in &result.pbds {
        if let Some(sr) = attachment.sr {
            result.sr_attachments.entry(sr).or_default().push(*id);
        }
    }
}

fn decode_pool_details(store: &Store, refs: &References) -> (Option<PoolDetails>, bool) {
    let mut has_ha_statefiles = false;
    let pool = store.class("pool").next().and_then(|(_, record)| {
        let view = PoolRecord::new(record);
        has_ha_statefiles = view
            .ha_statefiles()
            .members()
            .is_some_and(|v| !v.is_empty());
        let default_sr = refs.link(view.default_sr());
        Some(PoolDetails {
            administration: pool_administration(record, refs, store),
            auto_poweron: map_bool(view.other_config(), "auto_poweron"),
            ha_enabled: view.ha_enabled().get(),
            igmp_snooping_enabled: view.igmp_snooping_enabled().get(),
            id: id(view.uuid()).ok()?,
            name: string(view.name_label()),
            description: string(view.name_description()),
            default_sr,
            default_sr_known: view.default_sr() == "OpaqueRef:NULL" || default_sr.is_some(),
        })
    });
    (pool, has_ha_statefiles)
}

pub(super) fn decode(store: &Store) -> InventoryDetails {
    let refs = References::new(store);
    decode_with_refs(store, &refs).0
}

fn decode_with_refs(
    store: &Store,
    refs: &References,
) -> (InventoryDetails, bool, BTreeSet<String>, BTreeSet<String>) {
    let mut result = InventoryDetails {
        classes: store.classes.clone(),
        ..Default::default()
    };
    let (pool, pool_has_ha_statefiles) = decode_pool_details(store, refs);
    result.pool = pool;
    decode_organization(store, &mut result);
    decode_alarms(store, refs, &mut result);
    for class in OPTIONAL_CLASSES {
        result
            .classes
            .entry(class.to_ascii_lowercase())
            .or_insert(ClassAvailability::Unknown);
    }
    result.network_fabric = network_fabric::decode(store, refs, &mut result.classes);
    result.storage = records(store, "sr", &mut result.classes, decode_sr_details);
    result.networks = records(
        store,
        "network",
        &mut result.classes,
        decode_network_details,
    );
    result.tasks = records(store, "task", &mut result.classes, |r| decode_task(r, refs));
    result.vdis = records(store, "vdi", &mut result.classes, |r| decode_vdi(r, refs));
    for (_, record) in store.class("vdi") {
        if let (Ok(id), Some(source)) = (
            id::<VdiId>(VdiRecord::new(record).uuid()),
            crate::guest_tools::source(record),
        ) && result.vdis.contains_key(&id)
        {
            result.tools_media.insert(id, source);
        }
    }
    result.vbds = records(store, "vbd", &mut result.classes, |r| decode_vbd(r, refs));
    result.vifs = records(store, "vif", &mut result.classes, |r| decode_vif(r, refs));
    result.pifs = records(store, "pif", &mut result.classes, |r| decode_pif(r, refs));
    result.pbds = records(store, "pbd", &mut result.classes, |r| {
        decode_pbd(r, store, refs)
    });
    result.pif_metrics = records(
        store,
        "pif_metrics",
        &mut result.classes,
        decode_pif_metrics,
    );
    result.bonds = records(store, "bond", &mut result.classes, |r| decode_bond(r, refs));
    result.vlans = records(store, "vlan", &mut result.classes, |r| decode_vlan(r, refs));
    result.guest_metrics = records(
        store,
        "vm_guest_metrics",
        &mut result.classes,
        decode_guest_metrics,
    );
    result.vm_metrics = records(store, "vm_metrics", &mut result.classes, decode_vm_metrics);
    result.host_metrics = records(
        store,
        "host_metrics",
        &mut result.classes,
        decode_host_metrics,
    );
    result.messages = records(store, "message", &mut result.classes, |record| {
        let message = decode_server_message(record)?;
        Ok((message.id, message))
    });
    result.appliances = records(store, "vm_appliance", &mut result.classes, |r| {
        decode_appliance(r, refs)
    });
    result.snapshot_policies = records(store, "vmss", &mut result.classes, |r| {
        decode_snapshot_policy(r, refs)
    });
    result.storage_managers = records(store, "sm", &mut result.classes, decode_storage_manager);
    let mut vms_with_current_ops = BTreeSet::new();
    for (reference, record) in store.class("vm") {
        if VmRecord::new(record).is_a_snapshot() == true {
            if let Some((id, details)) = decode_snapshot_details(record, refs) {
                result.snapshots.insert(id, details);
            }
        } else if let Some((id, details)) = decode_vm_details(record, refs) {
            if !details.current_operations.is_empty() {
                vms_with_current_ops.insert(reference.clone());
            }
            result.vms.insert(id, details);
        }
    }
    let mut hosts_with_current_ops = BTreeSet::new();
    for (reference, record) in store.class("host") {
        if let Some((id, details)) = decode_host_details(record, refs) {
            if details
                .current_operations
                .as_ref()
                .is_some_and(|ops| !ops.is_empty())
            {
                hosts_with_current_ops.insert(reference.clone());
            }
            result.hosts.insert(id, details);
        }
    }
    rebuild_sr_disks(&mut result);
    rebuild_vbd_indexes(&mut result);
    rebuild_vif_indexes(&mut result);
    rebuild_pif_indexes(&mut result);
    rebuild_pbd_indexes(&mut result);
    (
        result,
        pool_has_ha_statefiles,
        vms_with_current_ops,
        hosts_with_current_ops,
    )
}

#[derive(Clone)]
pub(super) struct DecodedCache {
    refs: References,
    pool_id: PoolId,
    pool_name: String,
    coordinator: Option<HostId>,
    hosts: BTreeMap<HostId, Host>,
    vms: BTreeMap<VmId, Vm>,
    snapshots: crate::SnapshotInventory,
    storage: Vec<Storage>,
    networks: Vec<Network>,
    details: InventoryDetails,
    pool_has_ha_statefiles: bool,
    vms_with_current_ops: BTreeSet<String>,
    hosts_with_current_ops: BTreeSet<String>,
}

pub(super) struct StagedTop {
    refs: Option<References>,
    pool: Option<(PoolId, String, Option<HostId>)>,
    hosts: Option<BTreeMap<HostId, Host>>,
    vm_patch: Option<Vec<(VmId, Vm)>>,
    vms_and_snapshots: Option<(BTreeMap<VmId, Vm>, crate::SnapshotInventory)>,
    storage: Option<Vec<Storage>>,
    networks: Option<Vec<Network>>,
}

impl DecodedCache {
    pub(super) fn to_inventory(&self, revision: u64) -> Inventory {
        Inventory {
            pool_id: self.pool_id,
            pool_name: self.pool_name.clone(),
            coordinator: self.coordinator,
            hosts: self.hosts.clone(),
            vms: self.vms.clone(),
            snapshots: self.snapshots.clone(),
            storage: self.storage.clone(),
            networks: self.networks.clone(),
            details: self.details.clone(),
            revision,
        }
    }

    pub(super) fn build_full(store: &Store) -> Result<Self, Error> {
        let refs = References::new(store);
        let (pool_id, pool_name, coordinator) = Self::decode_pool_top(store, &refs)?;
        let hosts = Self::decode_hosts_top(store)?;
        let (vms, snapshots) = Self::decode_vms_and_snapshots_top(store, &refs)?;
        let storage = Self::decode_storage_top(store)?;
        let networks = Self::decode_networks_top(store)?;
        let (details, pool_has_ha_statefiles, vms_with_current_ops, hosts_with_current_ops) =
            decode_with_refs(store, &refs);
        Ok(Self {
            refs,
            pool_id,
            pool_name,
            coordinator,
            hosts,
            vms,
            snapshots,
            storage,
            networks,
            details,
            pool_has_ha_statefiles,
            vms_with_current_ops,
            hosts_with_current_ops,
        })
    }

    fn decode_pool_top(
        store: &Store,
        refs: &References,
    ) -> Result<(PoolId, String, Option<HostId>), Error> {
        if store.class_len("pool") != 1 {
            return Err(Error::Protocol(
                "expected one pool in the initial inventory",
            ));
        }
        let (_, pool) = store.class("pool").next().ok_or(Error::Protocol(
            "expected one pool in the initial inventory",
        ))?;
        let view = PoolRecord::new(pool);
        let pool_id = PoolId::parse(required(view.uuid())?)
            .map_err(|_| Error::Protocol("invalid pool UUID"))?;
        let coordinator = pool
            .get("master")
            .and_then(Value::as_str)
            .and_then(|r| refs.class_map("host").get(r))
            .copied()
            .map(HostId::from);
        Ok((pool_id, label(view.name_label()), coordinator))
    }

    fn decode_hosts_top(store: &Store) -> Result<BTreeMap<HostId, Host>, Error> {
        let mut hosts = BTreeMap::new();
        for (_, value) in store.class("host") {
            let (id, host) = decode_top_host(value)?;
            hosts.insert(id, host);
        }
        Ok(hosts)
    }

    fn decode_vms_and_snapshots_top(
        store: &Store,
        refs: &References,
    ) -> Result<(BTreeMap<VmId, Vm>, crate::SnapshotInventory), Error> {
        let host_refs = refs.class_map("host");
        let vm_refs = refs.class_map("vm");
        let mut vms = BTreeMap::new();
        let mut snapshots = crate::SnapshotInventory::default();
        for (_, value) in store.class("vm") {
            if VmRecord::new(value).is_a_snapshot() == true {
                snapshots.insert(decode_top_snapshot(value, vm_refs, &refs.snapshots)?);
            } else {
                let (id, vm) = decode_top_vm(value, host_refs)?;
                vms.insert(id, vm);
            }
        }
        Ok((vms, snapshots))
    }

    fn decode_storage_top(store: &Store) -> Result<Vec<Storage>, Error> {
        let mut storage = Vec::with_capacity(store.class_len("sr"));
        for (_, value) in store.class("sr") {
            storage.push(decode_top_storage(value)?);
        }
        Ok(storage)
    }

    fn decode_networks_top(store: &Store) -> Result<Vec<Network>, Error> {
        let mut networks = Vec::with_capacity(store.class_len("network"));
        for (_, value) in store.class("network") {
            networks.push(decode_top_network(value)?);
        }
        Ok(networks)
    }

    fn can_patch_class(
        &self,
        delta: &StoreDelta,
        mask: ClassMask,
        class: &str,
        ref_deps: ClassMask,
    ) -> bool {
        !delta.ref_dirty.contains(mask)
            && !delta.classes_changed.contains(mask)
            && !delta.ref_dirty.intersects(ref_deps)
            && !self.refs.duplicate_uuids.contains(mask)
            && self.details.classes.get(class) == Some(&ClassAvailability::Available)
    }

    /// Validates and decodes every fallible top-level section against `store` without
    /// mutating `self`, so a malformed batch leaves `self` completely untouched.
    pub(super) fn stage_top(&self, store: &Store, delta: &StoreDelta) -> Result<StagedTop, Error> {
        let mut refs = if delta.ref_dirty != ClassMask::EMPTY {
            let mut next_refs = self.refs.clone();
            for &(mask, canonical) in &ClassMask::ALL_CLASSES {
                if delta.ref_dirty.contains(mask) {
                    next_refs.rebuild_class(store, canonical, mask);
                }
            }
            Some(next_refs)
        } else {
            None
        };
        let active_refs = refs.as_ref().unwrap_or(&self.refs);
        let pool = if delta.data_dirty.contains(ClassMask::POOL)
            || delta.ref_dirty.contains(ClassMask::HOST)
        {
            Some(Self::decode_pool_top(store, active_refs)?)
        } else {
            None
        };
        let hosts = if delta.data_dirty.contains(ClassMask::HOST) {
            Some(Self::decode_hosts_top(store)?)
        } else {
            None
        };
        let mut vm_patch = None;
        let mut vms_and_snapshots = None;
        if delta.data_dirty.contains(ClassMask::VM) || delta.ref_dirty.contains(ClassMask::HOST) {
            let touched_vms = &delta.touched[ClassMask::VM.index()];
            let can_patch_vms = !delta.ref_dirty.contains(ClassMask::VM)
                && !delta.ref_dirty.contains(ClassMask::HOST)
                && !active_refs.duplicate_uuids.contains(ClassMask::VM)
                && !touched_vms
                    .iter()
                    .any(|r| active_refs.snapshots.contains(r.as_str()));
            if can_patch_vms {
                let host_refs = active_refs.class_map("host");
                let mut patched = Vec::with_capacity(touched_vms.len());
                for vm_ref in touched_vms {
                    if let Some(record) = store.record("vm", vm_ref) {
                        patched.push(decode_top_vm(record, host_refs)?);
                    }
                }
                vm_patch = Some(patched);
            } else {
                vms_and_snapshots = Some(Self::decode_vms_and_snapshots_top(store, active_refs)?);
            }
        }
        let storage = if delta.data_dirty.contains(ClassMask::SR) {
            Some(Self::decode_storage_top(store)?)
        } else {
            None
        };
        let networks = if delta.data_dirty.contains(ClassMask::NETWORK) {
            Some(Self::decode_networks_top(store)?)
        } else {
            None
        };
        Ok(StagedTop {
            refs: refs.take(),
            pool,
            hosts,
            vm_patch,
            vms_and_snapshots,
            storage,
            networks,
        })
    }

    /// Commits a previously staged top-level update and incrementally updates all dirty
    /// detail sections in place (infallible).
    pub(super) fn commit_delta(&mut self, store: &Store, delta: &StoreDelta, staged: StagedTop) {
        if delta.data_dirty == ClassMask::EMPTY
            && delta.ref_dirty == ClassMask::EMPTY
            && delta.classes_changed == ClassMask::EMPTY
        {
            return;
        }
        if let Some(refs) = staged.refs {
            self.refs = refs;
        }
        for &(mask, canonical) in &[
            (ClassMask::POOL, "pool"),
            (ClassMask::HOST, "host"),
            (ClassMask::VM, "vm"),
        ] {
            if delta.classes_changed.contains(mask) {
                reset_required_class_state(store, canonical, &mut self.details.classes);
            }
        }
        if let Some((pool_id, pool_name, coordinator)) = staged.pool {
            self.pool_id = pool_id;
            self.pool_name = pool_name;
            self.coordinator = coordinator;
        }
        if let Some(hosts) = staged.hosts {
            self.hosts = hosts;
        }
        if let Some(patched) = staged.vm_patch {
            for (id, vm) in patched {
                self.vms.insert(id, vm);
            }
        } else if let Some((vms, snapshots)) = staged.vms_and_snapshots {
            self.vms = vms;
            self.snapshots = snapshots;
        }
        if let Some(storage) = staged.storage {
            self.storage = storage;
        }
        if let Some(networks) = staged.networks {
            self.networks = networks;
        }

        // Update InventoryDetails sections incrementally.
        let pool_needs_vdi = self.pool_has_ha_statefiles
            && (delta.data_dirty.contains(ClassMask::VDI)
                || delta.ref_dirty.contains(ClassMask::VDI));
        if delta
            .data_dirty
            .intersects(ClassMask::POOL | ClassMask::HOST)
            || delta.ref_dirty.intersects(ClassMask::HOST | ClassMask::SR)
            || pool_needs_vdi
        {
            let (pool, has_ha_statefiles) = decode_pool_details(store, &self.refs);
            self.details.pool = pool;
            self.pool_has_ha_statefiles = has_ha_statefiles;
        }

        self.update_organization(store, delta);
        self.update_alarms(store, delta);

        network_fabric::update(
            &mut self.details.network_fabric,
            store,
            &self.refs,
            &mut self.details.classes,
            delta.data_dirty,
            delta.ref_dirty,
        );

        if delta.data_dirty.contains(ClassMask::SR) {
            reset_required_class_state(store, "sr", &mut self.details.classes);
            self.details.storage =
                records(store, "sr", &mut self.details.classes, decode_sr_details);
        }
        if delta.data_dirty.contains(ClassMask::NETWORK) {
            reset_required_class_state(store, "network", &mut self.details.classes);
            self.details.networks = records(
                store,
                "network",
                &mut self.details.classes,
                decode_network_details,
            );
        }

        self.update_simple_optional(
            store,
            delta,
            ClassMask::TASK,
            "task",
            ClassMask::HOST,
            |d| &mut d.tasks,
            decode_task,
        );
        self.update_vdis(store, delta);
        self.update_vbds(store, delta);
        self.update_vifs(store, delta);
        self.update_pifs(store, delta);
        self.update_pbds(store, delta);
        self.update_simple_optional(
            store,
            delta,
            ClassMask::PIF_METRICS,
            "pif_metrics",
            ClassMask::EMPTY,
            |d| &mut d.pif_metrics,
            |r, _| decode_pif_metrics(r),
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::BOND,
            "bond",
            ClassMask::PIF,
            |d| &mut d.bonds,
            decode_bond,
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::VLAN,
            "vlan",
            ClassMask::PIF,
            |d| &mut d.vlans,
            decode_vlan,
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::VM_GUEST_METRICS,
            "vm_guest_metrics",
            ClassMask::EMPTY,
            |d| &mut d.guest_metrics,
            |r, _| decode_guest_metrics(r),
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::VM_METRICS,
            "vm_metrics",
            ClassMask::EMPTY,
            |d| &mut d.vm_metrics,
            |r, _| decode_vm_metrics(r),
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::HOST_METRICS,
            "host_metrics",
            ClassMask::EMPTY,
            |d| &mut d.host_metrics,
            |r, _| decode_host_metrics(r),
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::MESSAGE,
            "message",
            ClassMask::EMPTY,
            |d| &mut d.messages,
            |r, _| {
                let message = decode_server_message(r)?;
                Ok((message.id, message))
            },
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::VM_APPLIANCE,
            "vm_appliance",
            ClassMask::VM,
            |d| &mut d.appliances,
            decode_appliance,
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::VMSS,
            "vmss",
            ClassMask::VM,
            |d| &mut d.snapshot_policies,
            decode_snapshot_policy,
        );
        self.update_simple_optional(
            store,
            delta,
            ClassMask::SM,
            "sm",
            ClassMask::EMPTY,
            |d| &mut d.storage_managers,
            |r, _| decode_storage_manager(r),
        );

        self.update_vm_and_snapshot_details(store, delta);
        self.update_host_details(store, delta);
    }

    #[allow(clippy::too_many_arguments)]
    fn update_simple_optional<I: Ord, R>(
        &mut self,
        store: &Store,
        delta: &StoreDelta,
        mask: ClassMask,
        class: &str,
        ref_deps: ClassMask,
        field: impl Fn(&mut InventoryDetails) -> &mut BTreeMap<I, R>,
        decode_one: impl Fn(&Value, &References) -> Result<(I, R), Error>,
    ) {
        if !delta.data_dirty.contains(mask) && !delta.ref_dirty.intersects(ref_deps) {
            return;
        }
        if self.can_patch_class(delta, mask, class, ref_deps) {
            let map = field(&mut self.details);
            for reference in &delta.touched[mask.index()] {
                let Some(record) = store.record(class, reference) else {
                    continue;
                };
                match decode_one(record, &self.refs) {
                    Ok((id, value)) => {
                        map.insert(id, value);
                    }
                    Err(_) => {
                        map.clear();
                        self.details
                            .classes
                            .insert(class.into(), ClassAvailability::InvalidData);
                        return;
                    }
                }
            }
        } else {
            reset_optional_class_state(store, class, &mut self.details.classes);
            let decoded = records(store, class, &mut self.details.classes, |r| {
                decode_one(r, &self.refs)
            });
            *field(&mut self.details) = decoded;
        }
    }

    fn update_vdis(&mut self, store: &Store, delta: &StoreDelta) {
        let mask = ClassMask::VDI;
        if !delta.data_dirty.contains(mask) && !delta.ref_dirty.contains(ClassMask::SR) {
            return;
        }
        if self.can_patch_class(delta, mask, "vdi", ClassMask::SR) {
            let mut sr_changed = false;
            for reference in &delta.touched[mask.index()] {
                let Some(record) = store.record("vdi", reference) else {
                    continue;
                };
                match decode_vdi(record, &self.refs) {
                    Ok((id, disk)) => {
                        let new_sr = disk.sr;
                        let old_sr = self.details.vdis.insert(id, disk).and_then(|old| old.sr);
                        if old_sr != new_sr {
                            sr_changed = true;
                        }
                        if let Some(source) = crate::guest_tools::source(record) {
                            self.details.tools_media.insert(id, source);
                        } else {
                            self.details.tools_media.remove(&id);
                        }
                    }
                    Err(_) => {
                        self.details.vdis.clear();
                        self.details.tools_media.clear();
                        self.details.sr_disks.clear();
                        self.details
                            .classes
                            .insert("vdi".into(), ClassAvailability::InvalidData);
                        return;
                    }
                }
            }
            if sr_changed {
                rebuild_sr_disks(&mut self.details);
            }
        } else {
            reset_optional_class_state(store, "vdi", &mut self.details.classes);
            self.details.vdis = records(store, "vdi", &mut self.details.classes, |r| {
                decode_vdi(r, &self.refs)
            });
            self.details.tools_media.clear();
            for (_, record) in store.class("vdi") {
                if let (Ok(id), Some(source)) = (
                    id::<VdiId>(VdiRecord::new(record).uuid()),
                    crate::guest_tools::source(record),
                ) && self.details.vdis.contains_key(&id)
                {
                    self.details.tools_media.insert(id, source);
                }
            }
            rebuild_sr_disks(&mut self.details);
        }
    }

    fn update_vbds(&mut self, store: &Store, delta: &StoreDelta) {
        let mask = ClassMask::VBD;
        let deps = ClassMask::VM | ClassMask::VDI;
        if !delta.data_dirty.contains(mask) && !delta.ref_dirty.intersects(deps) {
            return;
        }
        if self.can_patch_class(delta, mask, "vbd", deps) {
            let mut links_changed = false;
            for reference in &delta.touched[mask.index()] {
                let Some(record) = store.record("vbd", reference) else {
                    continue;
                };
                match decode_vbd(record, &self.refs) {
                    Ok((id, vbd)) => {
                        let new_links = (vbd.vm, vbd.snapshot, vbd.vdi);
                        let old_links = self
                            .details
                            .vbds
                            .insert(id, vbd)
                            .map(|old| (old.vm, old.snapshot, old.vdi));
                        if old_links != Some(new_links) {
                            links_changed = true;
                        }
                    }
                    Err(_) => {
                        self.details.vbds.clear();
                        rebuild_vbd_indexes(&mut self.details);
                        self.details
                            .classes
                            .insert("vbd".into(), ClassAvailability::InvalidData);
                        return;
                    }
                }
            }
            if links_changed {
                rebuild_vbd_indexes(&mut self.details);
            }
        } else {
            reset_optional_class_state(store, "vbd", &mut self.details.classes);
            self.details.vbds = records(store, "vbd", &mut self.details.classes, |r| {
                decode_vbd(r, &self.refs)
            });
            rebuild_vbd_indexes(&mut self.details);
        }
    }

    fn update_vifs(&mut self, store: &Store, delta: &StoreDelta) {
        let mask = ClassMask::VIF;
        let deps = ClassMask::VM | ClassMask::NETWORK;
        if !delta.data_dirty.contains(mask) && !delta.ref_dirty.intersects(deps) {
            return;
        }
        if self.can_patch_class(delta, mask, "vif", deps) {
            let mut links_changed = false;
            for reference in &delta.touched[mask.index()] {
                let Some(record) = store.record("vif", reference) else {
                    continue;
                };
                match decode_vif(record, &self.refs) {
                    Ok((id, vif)) => {
                        let new_links = (vif.vm, vif.network);
                        let old_links = self
                            .details
                            .vifs
                            .insert(id, vif)
                            .map(|old| (old.vm, old.network));
                        if old_links != Some(new_links) {
                            links_changed = true;
                        }
                    }
                    Err(_) => {
                        self.details.vifs.clear();
                        rebuild_vif_indexes(&mut self.details);
                        self.details
                            .classes
                            .insert("vif".into(), ClassAvailability::InvalidData);
                        return;
                    }
                }
            }
            if links_changed {
                rebuild_vif_indexes(&mut self.details);
            }
        } else {
            reset_optional_class_state(store, "vif", &mut self.details.classes);
            self.details.vifs = records(store, "vif", &mut self.details.classes, |r| {
                decode_vif(r, &self.refs)
            });
            rebuild_vif_indexes(&mut self.details);
        }
    }

    fn update_pifs(&mut self, store: &Store, delta: &StoreDelta) {
        let mask = ClassMask::PIF;
        let deps = ClassMask::HOST | ClassMask::NETWORK | ClassMask::PIF_METRICS;
        if !delta.data_dirty.contains(mask) && !delta.ref_dirty.intersects(deps) {
            return;
        }
        if self.can_patch_class(delta, mask, "pif", deps) {
            let mut links_changed = false;
            for reference in &delta.touched[mask.index()] {
                let Some(record) = store.record("pif", reference) else {
                    continue;
                };
                match decode_pif(record, &self.refs) {
                    Ok((id, pif)) => {
                        let new_links = (pif.host, pif.network);
                        let old_links = self
                            .details
                            .pifs
                            .insert(id, pif)
                            .map(|old| (old.host, old.network));
                        if old_links != Some(new_links) {
                            links_changed = true;
                        }
                    }
                    Err(_) => {
                        self.details.pifs.clear();
                        rebuild_pif_indexes(&mut self.details);
                        self.details
                            .classes
                            .insert("pif".into(), ClassAvailability::InvalidData);
                        return;
                    }
                }
            }
            if links_changed {
                rebuild_pif_indexes(&mut self.details);
            }
        } else {
            reset_optional_class_state(store, "pif", &mut self.details.classes);
            self.details.pifs = records(store, "pif", &mut self.details.classes, |r| {
                decode_pif(r, &self.refs)
            });
            rebuild_pif_indexes(&mut self.details);
        }
    }

    fn update_pbds(&mut self, store: &Store, delta: &StoreDelta) {
        let mask = ClassMask::PBD;
        let deps = ClassMask::HOST | ClassMask::SR;
        if !delta.data_dirty.intersects(mask | ClassMask::SR) && !delta.ref_dirty.intersects(deps) {
            return;
        }
        if !delta.data_dirty.contains(ClassMask::SR)
            && self.can_patch_class(delta, mask, "pbd", deps)
        {
            let mut sr_changed = false;
            for reference in &delta.touched[mask.index()] {
                let Some(record) = store.record("pbd", reference) else {
                    continue;
                };
                match decode_pbd(record, store, &self.refs) {
                    Ok((id, pbd)) => {
                        let new_sr = pbd.sr;
                        let old_sr = self.details.pbds.insert(id, pbd).and_then(|old| old.sr);
                        if old_sr != new_sr {
                            sr_changed = true;
                        }
                    }
                    Err(_) => {
                        self.details.pbds.clear();
                        rebuild_pbd_indexes(&mut self.details);
                        self.details
                            .classes
                            .insert("pbd".into(), ClassAvailability::InvalidData);
                        return;
                    }
                }
            }
            if sr_changed {
                rebuild_pbd_indexes(&mut self.details);
            }
        } else {
            reset_optional_class_state(store, "pbd", &mut self.details.classes);
            self.details.pbds = records(store, "pbd", &mut self.details.classes, |r| {
                decode_pbd(r, store, &self.refs)
            });
            rebuild_pbd_indexes(&mut self.details);
        }
    }

    fn update_vm_and_snapshot_details(&mut self, store: &Store, delta: &StoreDelta) {
        const VM_DETAIL_REF_DEPS: ClassMask = ClassMask::HOST
            .union(ClassMask::VM_GUEST_METRICS)
            .union(ClassMask::VM_METRICS)
            .union(ClassMask::VDI)
            .union(ClassMask::VM_APPLIANCE)
            .union(ClassMask::VMSS);

        if delta.data_dirty.contains(ClassMask::VM) || delta.ref_dirty.contains(ClassMask::VDI) {
            let can_patch_snapshots = !delta.ref_dirty.contains(ClassMask::VM)
                && !delta.ref_dirty.contains(ClassMask::VDI)
                && !self.refs.duplicate_uuids.contains(ClassMask::VM);
            if can_patch_snapshots {
                for reference in &delta.touched[ClassMask::VM.index()] {
                    if self.refs.snapshots.contains(reference.as_str())
                        && let Some(record) = store.record("vm", reference)
                        && let Some((id, details)) = decode_snapshot_details(record, &self.refs)
                    {
                        self.details.snapshots.insert(id, details);
                    }
                }
            } else {
                self.details.snapshots.clear();
                for (_, record) in store
                    .class("vm")
                    .filter(|(_, r)| VmRecord::new(r).is_a_snapshot() == true)
                {
                    if let Some((id, details)) = decode_snapshot_details(record, &self.refs) {
                        self.details.snapshots.insert(id, details);
                    }
                }
            }
        }

        let need_vm_rebuild = delta.ref_dirty.contains(ClassMask::VM)
            || delta.ref_dirty.intersects(VM_DETAIL_REF_DEPS)
            || (delta.data_dirty.contains(ClassMask::VM)
                && self.refs.duplicate_uuids.contains(ClassMask::VM));
        if need_vm_rebuild {
            self.details.vms.clear();
            self.vms_with_current_ops.clear();
            for (reference, record) in store
                .class("vm")
                .filter(|(_, r)| VmRecord::new(r).is_a_snapshot() != true)
            {
                if let Some((id, details)) = decode_vm_details(record, &self.refs) {
                    if !details.current_operations.is_empty() {
                        self.vms_with_current_ops.insert(reference.clone());
                    }
                    self.details.vms.insert(id, details);
                }
            }
        } else {
            if delta.data_dirty.contains(ClassMask::VM) {
                for reference in &delta.touched[ClassMask::VM.index()] {
                    if self.refs.snapshots.contains(reference.as_str()) {
                        continue;
                    }
                    if let Some(record) = store.record("vm", reference)
                        && let Some((id, details)) = decode_vm_details(record, &self.refs)
                    {
                        if details.current_operations.is_empty() {
                            self.vms_with_current_ops.remove(reference.as_str());
                        } else {
                            self.vms_with_current_ops.insert(reference.clone());
                        }
                        self.details.vms.insert(id, details);
                    }
                }
            }
            if delta.ref_dirty.contains(ClassMask::TASK) {
                for reference in &self.vms_with_current_ops {
                    if let Some(record) = store.record("vm", reference)
                        && let Some((id, details)) = decode_vm_details(record, &self.refs)
                    {
                        self.details.vms.insert(id, details);
                    }
                }
            }
        }
    }

    fn update_host_details(&mut self, store: &Store, delta: &StoreDelta) {
        const HOST_DETAIL_REF_DEPS: ClassMask = ClassMask::VM
            .union(ClassMask::SR)
            .union(ClassMask::HOST_METRICS);

        if delta.data_dirty.contains(ClassMask::HOST)
            || delta.ref_dirty.intersects(HOST_DETAIL_REF_DEPS)
        {
            self.details.hosts.clear();
            self.hosts_with_current_ops.clear();
            for (reference, record) in store.class("host") {
                if let Some((id, details)) = decode_host_details(record, &self.refs) {
                    if details
                        .current_operations
                        .as_ref()
                        .is_some_and(|ops| !ops.is_empty())
                    {
                        self.hosts_with_current_ops.insert(reference.clone());
                    }
                    self.details.hosts.insert(id, details);
                }
            }
        } else if delta.ref_dirty.contains(ClassMask::TASK) {
            for reference in &self.hosts_with_current_ops {
                if let Some(record) = store.record("host", reference)
                    && let Some((id, details)) = decode_host_details(record, &self.refs)
                {
                    self.details.hosts.insert(id, details);
                }
            }
        }
    }

    fn update_organization(&mut self, store: &Store, delta: &StoreDelta) {
        const ORG_CLASSES: ClassMask = ClassMask::POOL
            .union(ClassMask::HOST)
            .union(ClassMask::VM)
            .union(ClassMask::SR)
            .union(ClassMask::NETWORK)
            .union(ClassMask::VDI);
        const PATCHABLE_ORG: ClassMask = ClassMask::VM.union(ClassMask::VDI);

        let org_dirty = delta.data_dirty.intersects(ORG_CLASSES);
        if !org_dirty && !delta.classes_changed.contains(ClassMask::VDI) {
            return;
        }
        let dirty_org_bits = ClassMask(delta.data_dirty.0 & ORG_CLASSES.0);
        let can_patch = dirty_org_bits.is_subset_of(PATCHABLE_ORG)
            && !delta.ref_dirty.intersects(PATCHABLE_ORG)
            && !delta.classes_changed.contains(ClassMask::VDI)
            && !self.refs.duplicate_uuids.intersects(PATCHABLE_ORG)
            && store.class_len("vm") <= 50_000
            && store.class_len("vdi") <= 50_000;
        if can_patch {
            use crate::{OrganizationObjectId as Id, organization};
            let mut need_full = false;
            let mut name_changed = false;
            for (class, mask) in [("vm", ClassMask::VM), ("vdi", ClassMask::VDI)] {
                if !dirty_org_bits.contains(mask) {
                    continue;
                }
                for reference in &delta.touched[mask.index()] {
                    let Some(record) = store.record(class, reference) else {
                        need_full = true;
                        break;
                    };
                    let Some(uuid) = record["uuid"].as_str() else {
                        need_full = true;
                        break;
                    };
                    let id = if class == "vdi" {
                        VdiId::parse(uuid).ok().map(Id::Disk)
                    } else {
                        let view = VmRecord::new(record);
                        if view.is_control_domain() == true {
                            // Ensure control domain wasn't previously in organization.objects.
                            if let Ok(vm_id) = VmId::parse(uuid)
                                && self
                                    .details
                                    .organization
                                    .objects
                                    .iter()
                                    .any(|o| o.id == Id::Vm(vm_id))
                            {
                                need_full = true;
                                break;
                            }
                            continue;
                        }
                        if view.is_control_domain() == false && view.is_a_snapshot() == true {
                            SnapshotId::parse(uuid).ok().map(Id::Snapshot)
                        } else if view.is_control_domain() == false && view.is_a_snapshot() == false
                        {
                            VmId::parse(uuid).ok().map(Id::Vm)
                        } else {
                            None
                        }
                    };
                    let Some(id) = id else {
                        need_full = true;
                        break;
                    };
                    let Some(slot) = self
                        .details
                        .organization
                        .objects
                        .iter_mut()
                        .find(|o| o.id == id)
                    else {
                        need_full = true;
                        break;
                    };
                    let updated = organization::object(id, record);
                    if slot.name != updated.name {
                        name_changed = true;
                    }
                    *slot = updated;
                }
                if need_full {
                    break;
                }
            }
            if !need_full {
                if name_changed {
                    self.details
                        .organization
                        .objects
                        .sort_by_cached_key(|o| (o.name.to_lowercase(), o.id));
                }
                return;
            }
        }
        decode_organization(store, &mut self.details);
    }

    fn update_alarms(&mut self, store: &Store, delta: &StoreDelta) {
        if delta.data_dirty.contains(ClassMask::POOL) {
            self.details.alarm_email = store
                .class("pool")
                .next()
                .map(|(_, r)| crate::alarm_policies::project_email(r));
        }
        const ALARM_CLASSES: ClassMask = ClassMask::HOST.union(ClassMask::SR).union(ClassMask::VM);
        if !delta.data_dirty.intersects(ALARM_CLASSES) && !delta.ref_dirty.contains(ClassMask::HOST)
        {
            return;
        }
        let dirty_alarm_bits = ClassMask(delta.data_dirty.0 & ALARM_CLASSES.0);
        let can_patch = dirty_alarm_bits == ClassMask::VM
            && !delta.ref_dirty.intersects(ClassMask::HOST | ClassMask::VM)
            && !self.refs.duplicate_uuids.contains(ClassMask::VM);
        if can_patch {
            let mut need_full = false;
            for reference in &delta.touched[ClassMask::VM.index()] {
                let Some(record) = store.record("vm", reference) else {
                    need_full = true;
                    break;
                };
                let Some(uuid) = record["uuid"]
                    .as_str()
                    .and_then(|s| uuid::Uuid::parse_str(s).ok())
                else {
                    need_full = true;
                    break;
                };
                let view = VmRecord::new(record);
                let target = crate::ResourceTarget::Vm(uuid.into());
                if view.is_a_template() != false || view.is_a_snapshot() != false {
                    if self
                        .details
                        .alarm_policies
                        .iter()
                        .any(|p| p.target == target)
                    {
                        need_full = true;
                        break;
                    }
                    continue;
                }
                if view.is_control_domain().get() == Some(false)
                    && let Some(slot) = self
                        .details
                        .alarm_policies
                        .iter_mut()
                        .find(|p| p.target == target && p.control_domain_host.is_none())
                {
                    *slot = crate::alarm_policies::project(target, None, record);
                    continue;
                }
                need_full = true;
                break;
            }
            if !need_full {
                return;
            }
        }
        decode_alarm_policies(store, &self.refs, &mut self.details);
    }
}

fn pool_administration(
    record: &Value,
    refs: &References,
    store: &Store,
) -> crate::PoolAdministration {
    let view = PoolRecord::new(record);
    let statefiles = view.ha_statefiles().members().filter(|v| v.len() <= 64);
    let heartbeat_srs: Option<Vec<SrId>> = statefiles.and_then(|values| {
        values
            .iter()
            .map(|value| {
                let vdi = store
                    .class("vdi")
                    .find(|(reference, r)| {
                        value.as_str() == Some(*reference) || VdiRecord::new(r).uuid() == *value
                    })?
                    .1;
                refs.link(VdiRecord::new(vdi).sr())
            })
            .collect()
    });
    crate::PoolAdministration {
        coordinator: refs.link(view.master()),
        ha_enabled: view.ha_enabled().get(),
        ha_failures_to_tolerate: view.ha_host_failures_to_tolerate().unsigned(),
        ha_plan_exists_for: view.ha_plan_exists_for().unsigned(),
        ha_overcommitted: view.ha_overcommitted().get(),
        ha_allow_overcommit: view.ha_allow_overcommit().get(),
        ha_timeout_known: view
            .other_config()
            .entries()
            .is_some_and(|m| m.get("default_ha_timeout").is_none_or(Value::is_string))
            && view
                .ha_configuration()
                .entries()
                .is_some_and(|m| m.get("timeout").is_none_or(Value::is_string)),
        ha_timeout_override: view.other_config()["default_ha_timeout"]
            .as_str()
            .map(str::to_owned),
        ha_configuration_timeout: view.ha_configuration()["timeout"]
            .as_str()
            .map(str::to_owned),
        heartbeat_srs_known: heartbeat_srs.is_some(),
        heartbeat_srs: heartbeat_srs
            .unwrap_or_default()
            .into_iter()
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect(),
        migration_compression: view.migration_compression().get(),
        // XAPI keeps this on each host and has no pool field for it; the pool has a value
        // only where every host agrees.
        https_only: {
            let mut hosts = store
                .class("host")
                .map(|(_, host)| HostRecord::new(host).https_only().get());
            match hosts.next() {
                Some(Some(first)) if hosts.all(|host| host == Some(first)) => Some(first),
                _ => None,
            }
        },
        tls_verification_enabled: view.tls_verification_enabled().get(),
        secret_rotation_pending: view.is_psr_pending().get(),
    }
}

fn decode_organization(store: &Store, result: &mut InventoryDetails) {
    use crate::{OrganizationObjectId as Id, organization};
    result.organization.objects.clear();
    result.organization.empty_folders = None;
    result.organization.definitions = None;
    result.organization.complete =
        matches!(store.classes.get("vdi"), Some(ClassAvailability::Available));
    for class in ["pool", "host", "vm", "sr", "network", "vdi"] {
        if store.class_len(class) > 50_000 {
            result.organization.complete = false;
        }
        for (_, record) in store.class(class).take(50_000) {
            let Some(uuid) = record["uuid"].as_str() else {
                continue;
            };
            let id = match class {
                "pool" => PoolId::parse(uuid).ok().map(Id::Pool),
                "host" => HostId::parse(uuid).ok().map(Id::Host),
                "vm" if VmRecord::new(record).is_control_domain() == false
                    && VmRecord::new(record).is_a_snapshot() == true =>
                {
                    SnapshotId::parse(uuid).ok().map(Id::Snapshot)
                }
                "vm" if VmRecord::new(record).is_control_domain() == false
                    && VmRecord::new(record).is_a_snapshot() == false =>
                {
                    VmId::parse(uuid).ok().map(Id::Vm)
                }
                "sr" => SrId::parse(uuid).ok().map(Id::Storage),
                "network" => NetworkId::parse(uuid).ok().map(Id::Network),
                "vdi" => VdiId::parse(uuid).ok().map(Id::Disk),
                _ => None,
            };
            if id.is_none() && !(class == "vm" && VmRecord::new(record).is_control_domain() == true)
            {
                result.organization.complete = false;
            }
            if let Some(id) = id {
                result
                    .organization
                    .objects
                    .push(organization::object(id, record));
            }
            if class == "pool" {
                result.organization.empty_folders = PoolRecord::new(record)
                    .other_config()
                    .entries()
                    .and_then(|m| match m.get("EMPTY_FOLDERS") {
                        None => organization::folders(None),
                        Some(v) => organization::folders(Some(v.as_str()?)),
                    });
                result.organization.definitions = PoolRecord::new(record)
                    .gui_config()
                    .entries()
                    .and_then(|m| match m.get(organization::FIELD_KEY) {
                        None => organization::definitions(None),
                        Some(v) => organization::definitions(Some(v.as_str()?)),
                    });
            }
        }
    }
    result
        .organization
        .objects
        .sort_by_cached_key(|o| (o.name.to_lowercase(), o.id));
}

fn decode_alarms(store: &Store, refs: &References, result: &mut InventoryDetails) {
    result.alarm_email = store
        .class("pool")
        .next()
        .map(|(_, r)| crate::alarm_policies::project_email(r));
    decode_alarm_policies(store, refs, result);
}

fn decode_alarm_policies(store: &Store, refs: &References, result: &mut InventoryDetails) {
    use crate::ResourceTarget;
    result.alarm_policies.clear();
    let mut host_control_domains: BTreeMap<&str, &str> = BTreeMap::new();
    for (reference, record) in store.class("host") {
        if let Some(control) = HostRecord::new(record).control_domain().reference() {
            host_control_domains.insert(reference.as_str(), control);
        }
        let Some(uuid) = record["uuid"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
        else {
            continue;
        };
        result.alarm_policies.push(crate::alarm_policies::project(
            ResourceTarget::Host(uuid.into()),
            None,
            record,
        ));
    }
    for (_, record) in store.class("sr") {
        let Some(uuid) = record["uuid"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
        else {
            continue;
        };
        result.alarm_policies.push(crate::alarm_policies::project(
            ResourceTarget::Storage(uuid.into()),
            None,
            record,
        ));
    }
    for (reference, record) in store.class("vm") {
        let Some(uuid) = record["uuid"]
            .as_str()
            .and_then(|s| uuid::Uuid::parse_str(s).ok())
        else {
            continue;
        };
        let view = VmRecord::new(record);
        if view.is_a_template() != false || view.is_a_snapshot() != false {
            continue;
        }
        let dom0 = match view.is_control_domain().get() {
            Some(false) => None,
            Some(true) => {
                let Some(host_id) = refs.link(view.resident_on()) else {
                    continue;
                };
                if view
                    .resident_on()
                    .reference()
                    .and_then(|host_ref| host_control_domains.get(host_ref).copied())
                    != Some(reference.as_str())
                {
                    continue;
                }
                Some(host_id)
            }
            None => continue,
        };
        result.alarm_policies.push(crate::alarm_policies::project(
            ResourceTarget::Vm(uuid.into()),
            dom0,
            record,
        ));
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn uuid(n: u128) -> Uuid {
        Uuid::from_u128(n)
    }
    fn event(class: &str, reference: &str, record: Value) -> Value {
        json!({"class":class,"operation":"add","ref":format!("OpaqueRef:{reference}"),"snapshot":record})
    }
    fn batch(events: Vec<Value>) -> EventBatch {
        serde_json::from_value(json!({"token":"fixture-token","events":events})).unwrap()
    }
    fn vm(n: u128) -> Value {
        json!({"uuid":uuid(n),"name_label":"Fixture VM","power_state":"Halted",
            "is_a_snapshot":false,"is_a_template":false,"is_control_domain":false,
            "VCPUs_at_startup":"2","memory_static_max":"2147483648"})
    }
    fn initial() -> Vec<Value> {
        vec![
            event(
                "pool",
                "pool",
                json!({"uuid":uuid(1),"name_label":"Fixture pool"}),
            ),
            event("VM", "vm", vm(2)),
        ]
    }

    #[test]
    fn alarm_inventory_separates_host_control_domain_and_storage_scopes() {
        let mut events = initial();
        let mut ordinary = vm(2);
        ordinary["other_config"] = json!({});
        events[1] = event("VM", "vm", ordinary);
        let mut dom0 = vm(3);
        dom0["is_control_domain"] = json!(true);
        dom0["resident_on"] = json!("OpaqueRef:host");
        dom0["other_config"] =
            json!({"perfmon":"<config><variable><name value=\"mem_usage\"/></variable></config>"});
        events.push(event("VM", "dom0", dom0));
        events.push(event("host","host",json!({"uuid":uuid(4),"name_label":"Host","control_domain":"OpaqueRef:dom0","other_config":{"perfmon":"<config><variable><name value=\"memory_free_kib\"/><alarm_trigger_level value=\"2048\"/></variable></config>"}})));
        events.push(event("SR","sr",json!({"uuid":uuid(5),"name_label":"Storage","other_config":{"perfmon":"<config><variable><name value=\"sr_io_throughput_total_per_host\"/><alarm_trigger_level value=\"12\"/></variable></config>"}})));
        let inventory = Store::default().apply(&batch(events)).unwrap();
        let policies = &inventory.details.alarm_policies;
        assert_eq!(policies.len(), 4);
        let dom0 = policies
            .iter()
            .find(|p| p.control_domain_host.is_some())
            .unwrap();
        assert_eq!(dom0.control_domain_host, Some(uuid(4).into()));
        assert_eq!(dom0.rules[0].threshold, "0.95");
        assert!(policies.iter().any(|p| {
            p.rules
                .iter()
                .any(|r| r.metric == crate::AlarmMetric::FreeHostMemory && r.threshold == "2048")
        }));
        assert!(policies.iter().any(|p| {
            p.rules
                .iter()
                .any(|r| r.metric == crate::AlarmMetric::StorageThroughput && r.threshold == "12")
        }));
    }
    #[test]
    fn guest_tools_media_keeps_host_flags_and_known_legacy_names_separate() {
        let mut events = initial();
        for (n, name, flag) in [
            (20, "vendor-tools.iso", json!(true)),
            (21, "xs-tools.iso", json!(false)),
            (22, "guest-tools.iso", Value::Null),
            (23, "installer.iso", json!(false)),
            (24, "unrecognized.iso", json!("true")),
        ] {
            events.push(event(
                "VDI",
                &format!("disk-{n}"),
                json!({"uuid":uuid(n),"name_label":name,"is_tools_iso":flag}),
            ));
        }
        let inventory = Store::default().apply(&batch(events)).unwrap();
        assert_eq!(
            inventory.details.tools_media,
            BTreeMap::from([
                (uuid(20).into(), crate::GuestToolsMediaSource::HostFlag),
                (uuid(21).into(), crate::GuestToolsMediaSource::LegacyName),
                (uuid(22).into(), crate::GuestToolsMediaSource::LegacyName),
            ])
        );
        assert!(
            !serde_json::to_string(&inventory.details.tools_media)
                .unwrap()
                .contains("OpaqueRef:")
        );
    }

    #[test]
    fn snapshot_ancestry_keeps_branches_missing_parents_cycles_and_checkpoint_storage() {
        use crate::{SnapshotDetails, SnapshotParent};
        let mut events = initial();
        let mut current = vm(2);
        current["parent"] = json!("OpaqueRef:b");
        events.push(event("VM", "vm", current));
        events.push(event(
            "VDI",
            "memory",
            json!({"uuid":uuid(20),"virtual_size":"1024"}),
        ));
        for (name, n, parent) in [
            ("a", 3, "OpaqueRef:NULL"),
            ("b", 4, "OpaqueRef:a"),
            ("c", 5, "OpaqueRef:a"),
            ("lost", 6, "OpaqueRef:missing"),
            ("cycle-a", 7, "OpaqueRef:cycle-b"),
            ("cycle-b", 8, "OpaqueRef:cycle-a"),
            ("foreign", 9, "OpaqueRef:vm"),
        ] {
            let mut record = vm(n);
            record["is_a_snapshot"] = json!(true);
            record["snapshot_of"] = json!("OpaqueRef:vm");
            record["name_label"] = json!(name);
            record["parent"] = json!(parent);
            record["suspend_VDI"] = json!("OpaqueRef:memory");
            record["blobs"] = json!({"XenCenter.VNCSnapshot":"OpaqueRef:preview"});
            events.push(event("VM", name, record));
        }
        let i = Store::default().apply(&batch(events)).unwrap();
        let rows = i.snapshot_ancestry(uuid(2).into());
        assert_eq!(rows.len(), 7);
        assert_eq!(rows.iter().map(|r| r.id).collect::<BTreeSet<_>>().len(), 7);
        let row = |n| rows.iter().find(|r| r.id == uuid(n).into()).unwrap();
        assert_eq!(row(3).depth, 0);
        assert_eq!(row(4).depth, 1);
        assert_eq!(row(5).depth, 1);
        assert!(row(4).current_parent);
        assert!(row(6).issue.is_some());
        assert!(row(7).issue.is_some());
        assert!(row(8).issue.is_some());
        assert!(row(9).issue.is_some());
        let details = &i.details.snapshots[&uuid(3).into()];
        assert_eq!(details.parent, SnapshotParent::Root);
        assert_eq!(details.memory_vdi, Some(uuid(20).into()));
        assert!(details.preview_available);
        let missing: SnapshotDetails = Default::default();
        assert_eq!(missing.parent, SnapshotParent::Unresolved);
        let public = serde_json::to_string(&i.details).unwrap();
        assert!(!public.contains("OpaqueRef:"));
        assert!(!public.contains("XenCenter.VNCSnapshot"));
    }

    #[test]
    fn checkpoint_export_capture_uses_snapshot_inventory_and_exact_memory_identity() {
        let mut events = initial();
        let mut snapshot = vm(3);
        snapshot["is_a_snapshot"] = json!(true);
        snapshot["snapshot_of"] = json!("OpaqueRef:vm");
        snapshot["power_state"] = json!("Suspended");
        snapshot["suspend_VDI"] = json!("OpaqueRef:memory");
        snapshot["allowed_operations"] = json!(["export"]);
        snapshot["current_operations"] = json!({});
        snapshot["blocked_operations"] = json!({});
        events.push(event("VM", "checkpoint", snapshot.clone()));
        events.push(event(
            "SR",
            "storage",
            json!({"uuid":uuid(4),"content_type":"user"}),
        ));
        events.push(event("VDI", "memory", json!({"uuid":uuid(20),"type":"suspend","SR":"OpaqueRef:storage","virtual_size":"4096","managed":true,"missing":false})));
        let mut store = Store::default();
        let i = store.apply(&batch(events)).unwrap();
        assert!(!i.details.vms.contains_key(&uuid(3).into()));
        assert_eq!(
            crate::ExportSource::checkpoint(&i, uuid(3).into(), uuid(2).into(), true),
            Some(crate::ExportSource::Checkpoint {
                snapshot: uuid(3).into(),
                parent: uuid(2).into(),
                memory: crate::CheckpointMemory {
                    vdi: uuid(20).into(),
                    sr: uuid(4).into(),
                    virtual_size: 4096
                },
                preserve_memory: true,
            })
        );
        snapshot["blocked_operations"] = json!({"export":"fixture block"});
        let i = store
            .apply(&batch(vec![event("VM", "checkpoint", snapshot)]))
            .unwrap();
        assert!(
            crate::ExportSource::checkpoint(&i, uuid(3).into(), uuid(2).into(), true).is_none()
        );
    }

    #[test]
    fn a_disk_snapshot_is_offered_for_export_only_when_the_host_offers_it() {
        use crate::{ActionAvailability, ExportSource, TransferAction, XvaCompression};
        let export = TransferAction::ExportXva {
            source: ExportSource::Snapshot {
                snapshot: uuid(3).into(),
                parent: uuid(2).into(),
            },
            compression: XvaCompression::Uncompressed,
        };
        let mut snapshot = vm(3);
        snapshot["is_a_snapshot"] = json!(true);
        snapshot["snapshot_of"] = json!("OpaqueRef:vm");
        snapshot["power_state"] = json!("Halted");
        snapshot["allowed_operations"] = json!(["export"]);
        snapshot["current_operations"] = json!({});
        snapshot["blocked_operations"] = json!({});
        let mut events = initial();
        events.push(event("VM", "snapshot", snapshot.clone()));
        let mut store = Store::default();
        let i = store.apply(&batch(events)).unwrap();
        assert_eq!(export.availability(&i), ActionAvailability::Available);
        // The host stops offering the export, has the snapshot busy, or blocks it: each is
        // seen before a review instead of at the transfer's preflight.
        for (key, value) in [
            ("allowed_operations", json!(["destroy"])),
            ("current_operations", json!({"OpaqueRef:task": "export"})),
            ("blocked_operations", json!({"export": "fixture block"})),
        ] {
            let mut changed = snapshot.clone();
            changed[key] = value;
            let i = store
                .apply(&batch(vec![event("VM", "snapshot", changed)]))
                .unwrap();
            assert_eq!(
                export.availability(&i),
                ActionAvailability::Unavailable,
                "{key}"
            );
        }
        // Back as it was, it is offered again; a snapshot of another VM never is.
        let i = store
            .apply(&batch(vec![event("VM", "snapshot", snapshot)]))
            .unwrap();
        assert_eq!(export.availability(&i), ActionAvailability::Available);
        let foreign = TransferAction::ExportXva {
            source: ExportSource::Snapshot {
                snapshot: uuid(3).into(),
                parent: uuid(9).into(),
            },
            compression: XvaCompression::Uncompressed,
        };
        assert!(!matches!(
            foreign.availability(&i),
            ActionAvailability::Available
        ));
    }

    #[test]
    fn graph_resolves_out_of_order_records_and_preserves_shared_disk_consumers() {
        let mut events = vec![
            event(
                "VBD",
                "disk-a",
                json!({"uuid":uuid(10),"VM":"OpaqueRef:vm","VDI":"OpaqueRef:shared","type":"Disk","userdevice":"0"}),
            ),
            event(
                "VBD",
                "disk-b",
                json!({"uuid":uuid(11),"VM":"OpaqueRef:other","VDI":"OpaqueRef:shared","type":"Disk","userdevice":"0"}),
            ),
            event(
                "VBD",
                "snapshot-disk",
                json!({"uuid":uuid(12),"VM":"OpaqueRef:snapshot","VDI":"OpaqueRef:shared","type":"Disk"}),
            ),
            event(
                "VIF",
                "nic",
                json!({"uuid":uuid(13),"VM":"OpaqueRef:vm","network":"OpaqueRef:net","MTU":"1500"}),
            ),
            event(
                "PBD",
                "mount",
                json!({"uuid":uuid(14),"host":"OpaqueRef:host","SR":"OpaqueRef:sr","currently_attached":true}),
            ),
            event(
                "PIF",
                "uplink",
                json!({"uuid":uuid(15),"host":"OpaqueRef:host","network":"OpaqueRef:net","VLAN":"-1"}),
            ),
            event(
                "VDI",
                "shared",
                json!({"uuid":uuid(20),"SR":"OpaqueRef:sr","virtual_size":"9007199254740993","sharable":true}),
            ),
            event(
                "SR",
                "sr",
                json!({"uuid":uuid(21),"name_label":"Fixture SR","physical_size":"-1"}),
            ),
            event(
                "network",
                "net",
                json!({"uuid":uuid(22),"name_label":"Fixture network","MTU":"9000"}),
            ),
            event("host", "host", json!({"uuid":uuid(23),"enabled":true})),
            event("VM", "other", vm(3)),
            event(
                "VM",
                "snapshot",
                json!({"uuid":uuid(4),"power_state":"Halted","is_a_snapshot":true,"snapshot_of":"OpaqueRef:vm"}),
            ),
        ];
        events.extend(initial());
        let mut store = Store::default();
        let inventory = store.apply(&batch(events)).unwrap();
        let details = &inventory.details;
        assert_eq!(
            details.vm_disks[&VmId::from(uuid(2))],
            vec![VbdId::from(uuid(10))]
        );
        assert_eq!(
            details.vm_disks[&VmId::from(uuid(3))],
            vec![VbdId::from(uuid(11))]
        );
        assert_eq!(
            details.snapshot_disks[&SnapshotId::from(uuid(4))],
            vec![VbdId::from(uuid(12))]
        );
        assert_eq!(details.vdi_attachments[&VdiId::from(uuid(20))].len(), 3);
        assert_eq!(
            details.sr_disks[&SrId::from(uuid(21))],
            vec![VdiId::from(uuid(20))]
        );
        assert_eq!(
            details.vdis[&VdiId::from(uuid(20))].virtual_size,
            Some(9_007_199_254_740_993)
        );
        assert_eq!(
            details.network_interfaces[&NetworkId::from(uuid(22))],
            vec![VifId::from(uuid(13))]
        );
        assert_eq!(
            details.network_uplinks[&NetworkId::from(uuid(22))],
            vec![PifId::from(uuid(15))]
        );
        assert_eq!(
            details.sr_attachments[&SrId::from(uuid(21))],
            vec![PbdId::from(uuid(14))]
        );
        assert_eq!(details.pifs[&PifId::from(uuid(15))].vlan, Some(-1));
        let deleted = store
            .apply(&batch(vec![
                json!({"class":"VM","operation":"del","ref":"OpaqueRef:other"}),
                json!({"class":"VBD","operation":"del","ref":"OpaqueRef:disk-a"}),
            ]))
            .unwrap();
        assert!(!deleted.details.vm_disks.contains_key(&VmId::from(uuid(2))));
        assert!(!deleted.details.vm_disks.contains_key(&VmId::from(uuid(3))));
        assert!(deleted.details.vbds[&VbdId::from(uuid(11))].vm.is_none());
        assert_eq!(
            deleted.details.vdi_attachments[&VdiId::from(uuid(20))].len(),
            2
        );
    }

    #[test]
    fn optional_corruption_is_local_and_unknown_numbers_are_not_zero() {
        let mut events = initial();
        events.push(event(
            "VDI",
            "disk",
            json!({"uuid":uuid(10),"virtual_size":"-1"}),
        ));
        events.push(event(
            "VIF",
            "nic",
            json!({"uuid":uuid(11),"VM":"OpaqueRef:vm"}),
        ));
        let mut store = Store::default();
        let inventory = store.apply(&batch(events)).unwrap();
        assert_eq!(
            inventory.details.vdis[&VdiId::from(uuid(10))].virtual_size,
            None
        );
        assert_eq!(inventory.details.vifs[&VifId::from(uuid(11))].mtu, None);
        for value in [json!("bad-size"), json!(1.5), json!(-2), json!(true)] {
            let inventory = store
                .apply(&batch(vec![event(
                    "VDI",
                    "disk",
                    json!({"uuid":uuid(10),"virtual_size":value}),
                )]))
                .unwrap();
            assert_eq!(inventory.guests().count(), 1);
            assert_eq!(
                inventory.details.classes["vdi"],
                ClassAvailability::InvalidData
            );
            assert!(inventory.details.vdis.is_empty());
            assert_eq!(inventory.details.vifs.len(), 1);
        }
        let inventory = store
            .apply(&batch(vec![event(
                "VDI",
                "disk",
                json!({"uuid":uuid(10),"virtual_size":0}),
            )]))
            .unwrap();
        assert_eq!(
            inventory.details.classes["vdi"],
            ClassAvailability::Available
        );
        assert_eq!(
            inventory.details.vdis[&VdiId::from(uuid(10))].virtual_size,
            Some(0)
        );
    }

    #[test]
    fn permission_changes_and_full_resync_remove_old_optional_data() {
        let mut events = initial();
        events.push(event("VDI", "disk", json!({"uuid":uuid(10)})));
        let mut store = Store::default();
        store.apply(&batch(events)).unwrap();
        let mut denied = batch(vec![]);
        denied
            .classes
            .insert("vdi".into(), ClassAvailability::PermissionDenied);
        let inventory = store.apply(&denied).unwrap();
        assert!(inventory.details.vdis.is_empty());
        assert_eq!(
            inventory.details.classes["vdi"],
            ClassAvailability::PermissionDenied
        );
        assert_eq!(inventory.guests().count(), 1);
        let mut reset = batch(vec![event("pool", "replacement", json!({"uuid":uuid(1)}))]);
        reset.reset = true;
        let inventory = store.apply(&reset).unwrap();
        assert!(inventory.vms.is_empty());
        assert!(inventory.details.vms.is_empty());
    }

    #[test]
    fn vm_editing_values_and_task_links_are_typed_without_secret_configuration() {
        let mut record = vm(2);
        record["VCPUs_max"] = json!(8);
        record["memory_static_min"] = json!("1073741824");
        record["memory_dynamic_min"] = json!("1073741824");
        record["memory_dynamic_max"] = json!("2147483648");
        record["HVM_boot_params"] = json!({"order":"dc","firmware":"uefi"});
        record["affinity"] = json!("OpaqueRef:host");
        record["guest_metrics"] = json!("OpaqueRef:guest");
        record["current_operations"] = json!({"OpaqueRef:task":"snapshot"});
        let mut events = initial();
        events.push(event("VM", "vm", record));
        events.push(event("host", "host", json!({"uuid":uuid(3)})));
        events.push(event(
            "VM_guest_metrics",
            "guest",
            json!({"uuid":uuid(4),"live":true,"os_version":{"name":"Fixture OS"}}),
        ));
        events.push(event("task","task",json!({"uuid":uuid(5),"progress":"0.5","error_info":["SR_BACKEND_FAILURE","secret-error-details"]})));
        events.push(event(
            "PBD",
            "mount",
            json!({"uuid":uuid(6),"device_config":{"password":"secret-storage-password"}}),
        ));
        let inventory = Store::default().apply(&batch(events)).unwrap();
        let details = &inventory.details.vms[&VmId::from(uuid(2))];
        assert_eq!(details.vcpus_max, Some(8));
        assert_eq!(details.memory_dynamic_max, Some(2_147_483_648));
        assert_eq!(details.boot_order.as_deref(), Some("dc"));
        assert_eq!(details.firmware.as_deref(), Some("uefi"));
        assert_eq!(details.affinity, Some(HostId::from(uuid(3))));
        assert_eq!(details.guest_metrics, Some(GuestMetricsId::from(uuid(4))));
        assert_eq!(
            details.current_operations[0].task,
            Some(TaskId::from(uuid(5)))
        );
        assert_eq!(
            inventory.details.tasks[&TaskId::from(uuid(5))].progress,
            Some(0.5)
        );
        let encoded = serde_json::to_string(&inventory.details).unwrap();
        assert!(!encoded.contains("secret-"));
        assert!(!encoded.contains("OpaqueRef:"));
    }

    #[test]
    fn blocked_action_messages_survive_inventory_and_changes() {
        let mut record = vm(2);
        record["allowed_operations"] = json!([]);
        record["blocked_operations"] = json!({
            "start": "Maintenance: contact the VM owner (équipe).",
            "hard_shutdown": ""
        });
        let mut events = initial();
        events.push(event("VM", "vm", record.clone()));
        let mut store = Store::default();
        let inventory = store.apply(&batch(events)).unwrap();
        let details = &inventory.details.vms[&VmId::from(uuid(2))];
        assert_eq!(
            details.power_availability(crate::PowerAction::Start, true),
            ActionAvailability::Blocked {
                reason: "Maintenance: contact the VM owner (équipe).".into()
            }
        );
        assert_eq!(
            details.blocked_operations["hard_shutdown"],
            "No reason supplied by the host"
        );
        assert!(
            inventory.vms[&VmId::from(uuid(2))]
                .allowed_actions
                .is_empty()
        );
        record["blocked_operations"] = json!({});
        record["allowed_operations"] = json!(["start"]);
        let inventory = store
            .apply(&batch(vec![event("VM", "vm", record)]))
            .unwrap();
        let details = &inventory.details.vms[&VmId::from(uuid(2))];
        assert!(details.blocked_operations.is_empty());
        assert_eq!(
            details.power_availability(crate::PowerAction::Start, true),
            ActionAvailability::Available
        );
    }

    #[test]
    fn blocked_action_messages_are_bounded_plain_text() {
        assert_eq!(
            blocked_reason(" \tmaintenance\nwindow\r\n"),
            "maintenance window"
        );
        assert_eq!(blocked_reason("Hold\0\u{202e} this\u{2069}"), "Hold this");
        assert_eq!(
            blocked_reason("\0\u{202e}\n"),
            "No reason supplied by the host"
        );
        assert_eq!(blocked_reason(&"é".repeat(1024)), "é".repeat(1024));
        assert_eq!(
            blocked_reason(&"é".repeat(1025)),
            format!("{}…", "é".repeat(1024))
        );
    }

    #[test]
    fn power_availability_distinguishes_missing_empty_busy_blocked_and_stale() {
        let mut details = VmDetails::default();
        assert_eq!(
            details.power_availability(crate::PowerAction::Start, true),
            ActionAvailability::Unknown
        );
        details.allowed_operations = Some(vec![]);
        assert_eq!(
            details.power_availability(crate::PowerAction::Start, true),
            ActionAvailability::Unavailable
        );
        details.current_operations.push(RunningOperation {
            task: None,
            operation: "snapshot".into(),
        });
        assert!(matches!(
            details.power_availability(crate::PowerAction::Start, true),
            ActionAvailability::Busy { .. }
        ));
        details.allowed_operations = Some(vec!["start".into()]);
        assert_eq!(
            details.power_availability(crate::PowerAction::Start, true),
            ActionAvailability::Available
        );
        details
            .blocked_operations
            .insert("start".into(), "BLOCKED_BY_POLICY".into());
        assert!(matches!(
            details.power_availability(crate::PowerAction::Start, true),
            ActionAvailability::Blocked { .. }
        ));
        assert_eq!(
            details.power_availability(crate::PowerAction::Start, false),
            ActionAvailability::Stale
        );
        assert_eq!(
            power_availability(
                &json!({"allowed_operations":["start",7]}),
                crate::PowerAction::Start
            ),
            ActionAvailability::Unknown
        );
    }

    #[test]
    fn malformed_optional_events_disable_only_their_class_until_corrected() {
        let mut store = Store::default();
        store.apply(&batch(initial())).unwrap();
        for event in [
            json!({"class":"VDI","operation":"mod","ref":"OpaqueRef:disk","snapshot":[]}),
            json!({"class":"VDI","operation":"future_operation","ref":"OpaqueRef:disk"}),
        ] {
            let inventory = store.apply(&batch(vec![event])).unwrap();
            assert_eq!(inventory.guests().count(), 1);
            assert!(inventory.details.vdis.is_empty());
            assert_eq!(
                inventory.details.classes["vdi"],
                ClassAvailability::InvalidData
            );
            assert_eq!(
                store.apply(&batch(vec![])).unwrap().details.classes["vdi"],
                ClassAvailability::InvalidData
            );
        }
        let inventory = store
            .apply(&batch(vec![event("VDI", "disk", json!({"uuid":uuid(10)}))]))
            .unwrap();
        assert_eq!(
            inventory.details.classes["vdi"],
            ClassAvailability::Available
        );
        assert_eq!(inventory.details.vdis.len(), 1);
    }
    #[test]
    fn link_metrics_are_scoped_and_unavailable_values_do_not_invent_link_health() {
        let mut events = initial();
        events.push(event(
            "PIF",
            "nic",
            json!({"uuid":uuid(30),"device":"eth0","metrics":"OpaqueRef:link","capabilities":[]}),
        ));
        events.push(event("PIF_metrics","link",json!({"uuid":uuid(31),"carrier":false,"speed":"-1","duplex":true,"vendor_name":"Fixture vendor","device_name":"Fixture NIC","pci_bus_path":"0000:01:00.0"})));
        let mut store = Store::default();
        let i = store.apply(&batch(events)).unwrap();
        let p = &i.details.pifs[&PifId::from(uuid(30))];
        assert_eq!(p.capabilities, Some(vec![]));
        let m = &i.details.pif_metrics[&p.metrics.unwrap()];
        assert_eq!(m.carrier, Some(false));
        assert_eq!(m.speed_mbps, None);
        assert_eq!(m.vendor_name, "Fixture vendor");
        let i = store
            .apply(&batch(vec![event(
                "PIF_metrics",
                "link",
                json!({"uuid":uuid(31),"speed":"100000","carrier":true}),
            )]))
            .unwrap();
        assert_eq!(
            i.details.pif_metrics[&PifMetricsId::from(uuid(31))].speed_mbps,
            Some(100000)
        );
        let i = store
            .apply(&batch(vec![event(
                "PIF_metrics",
                "link",
                json!({"uuid":uuid(31),"speed":"invalid"}),
            )]))
            .unwrap();
        assert!(i.details.pif_metrics.is_empty());
        assert_eq!(
            i.details.classes["pif_metrics"],
            ClassAvailability::InvalidData
        );
        assert!(i.details.pifs.contains_key(&PifId::from(uuid(30))));
        assert_eq!(i.guests().count(), 1);
        let i = store
            .apply(&batch(vec![event("PIF", "nic", json!({"uuid":uuid(30)}))]))
            .unwrap();
        assert!(
            i.details.pifs[&PifId::from(uuid(30))]
                .capabilities
                .is_none()
        );
    }
    #[test]
    fn runtime_projection_keeps_reported_values_and_rejects_future_or_sentinel_boot_times() {
        let mut events = initial();
        let mut v = vm(2);
        v["metrics"] = json!("OpaqueRef:runtime");
        events.push(event("VM", "vm", v));
        events.push(event("VM_metrics","runtime",json!({"uuid":uuid(32),"memory_actual":"1073741824","VCPUs_number":"3","start_time":"20260101T00:00:00Z","install_time":"2025-01-01T00:00:00Z","last_updated":"2026-01-01T01:00:00Z","state":["running"],"current_domain_type":"hvm"})));
        let i = Store::default().apply(&batch(events)).unwrap();
        let d = &i.details.vms[&VmId::from(uuid(2))];
        let mut m = i.details.vm_metrics[&d.metrics.unwrap()].clone();
        assert_eq!(m.memory_actual, Some(1 << 30));
        assert_eq!(m.vcpus_number, Some(3));
        assert_eq!(m.state, vec!["running"]);
        assert_eq!(m.current_domain_type.as_deref(), Some("hvm"));
        let start =
            crate::snapshot_policy_support::clock_seconds("20260101T00:00:00Z").unwrap() as u64;
        let now = std::time::UNIX_EPOCH + std::time::Duration::from_secs(start + 3600);
        assert_eq!(m.elapsed_since_boot(now), Some(3600));
        for invalid in [
            "19700101T00:00:00Z",
            "20270101T00:00:00Z",
            "20260230T00:00:00Z",
            "",
        ] {
            m.start_time = Some(invalid.into());
            assert!(m.elapsed_since_boot(now).is_none());
        }
    }

    #[test]
    fn incremental_store_apply_matches_full_decode_across_large_cluster_deltas() {
        let mut events = vec![event(
            "pool",
            "pool",
            json!({"uuid":uuid(1),"name_label":"Scale Pool","master":"OpaqueRef:host-0","default_SR":"OpaqueRef:sr-0"}),
        )];
        for h in 0u128..8 {
            events.push(event(
                "host",
                &format!("host-{h}"),
                json!({
                    "uuid": uuid(100 + h),
                    "name_label": format!("host-{h}"),
                    "address": format!("10.0.0.{}", h + 1),
                    "enabled": true,
                    "control_domain": format!("OpaqueRef:dom0-{h}"),
                    "metrics": format!("OpaqueRef:hm-{h}"),
                    "software_version": {"product_version": "8.3.0"},
                    "current_operations": {}
                }),
            ));
            events.push(event(
                "host_metrics",
                &format!("hm-{h}"),
                json!({"uuid": uuid(200 + h), "live": true, "memory_total": "137438953472", "memory_free": "68719476736"}),
            ));
            events.push(event(
                "VM",
                &format!("dom0-{h}"),
                json!({
                    "uuid": uuid(300 + h),
                    "name_label": format!("Control domain on host-{h}"),
                    "power_state": "Running",
                    "resident_on": format!("OpaqueRef:host-{h}"),
                    "is_a_snapshot": false,
                    "is_a_template": false,
                    "is_control_domain": true,
                    "VCPUs_at_startup": "4",
                    "memory_static_max": "4294967296"
                }),
            ));
        }
        for s in 0u128..4 {
            events.push(event(
                "SR",
                &format!("sr-{s}"),
                json!({
                    "uuid": uuid(400 + s),
                    "name_label": format!("SR-{s}"),
                    "type": "nfs",
                    "physical_size": "1099511627776",
                    "physical_utilisation": "10737418240"
                }),
            ));
        }
        for n in 0u128..4 {
            events.push(event(
                "network",
                &format!("net-{n}"),
                json!({"uuid": uuid(500 + n), "name_label": format!("Net-{n}"), "bridge": format!("xenbr{n}")}),
            ));
        }
        for i in 0u128..120 {
            let host = i % 8;
            let sr = i % 4;
            let net = i % 4;
            events.push(event(
                "VM",
                &format!("vm-{i}"),
                json!({
                    "uuid": uuid(10_000 + i),
                    "name_label": format!("vm-{i:03}"),
                    "power_state": "Running",
                    "resident_on": format!("OpaqueRef:host-{host}"),
                    "metrics": format!("OpaqueRef:vmm-{i}"),
                    "guest_metrics": format!("OpaqueRef:vmgm-{i}"),
                    "is_a_snapshot": false,
                    "is_a_template": false,
                    "is_control_domain": false,
                    "VCPUs_at_startup": "2",
                    "memory_static_max": "2147483648",
                    "allowed_operations": ["clean_shutdown", "snapshot"],
                    "current_operations": {}
                }),
            ));
            events.push(event(
                "VM_metrics",
                &format!("vmm-{i}"),
                json!({"uuid": uuid(20_000 + i), "memory_actual": "2147483648", "VCPUs_number": "2"}),
            ));
            events.push(event(
                "VM_guest_metrics",
                &format!("vmgm-{i}"),
                json!({"uuid": uuid(30_000 + i), "live": true, "networks": {"0/ip": format!("10.1.0.{}", (i % 250) + 1)}}),
            ));
            events.push(event(
                "VDI",
                &format!("vdi-{i}"),
                json!({
                    "uuid": uuid(40_000 + i),
                    "name_label": format!("disk-{i}"),
                    "SR": format!("OpaqueRef:sr-{sr}"),
                    "virtual_size": "21474836480",
                    "physical_utilisation": "1073741824"
                }),
            ));
            events.push(event(
                "VBD",
                &format!("vbd-{i}"),
                json!({
                    "uuid": uuid(50_000 + i),
                    "VM": format!("OpaqueRef:vm-{i}"),
                    "VDI": format!("OpaqueRef:vdi-{i}"),
                    "device": "xvda",
                    "userdevice": "0",
                    "type": "Disk",
                    "mode": "RW",
                    "currently_attached": true
                }),
            ));
            events.push(event(
                "VIF",
                &format!("vif-{i}"),
                json!({
                    "uuid": uuid(60_000 + i),
                    "VM": format!("OpaqueRef:vm-{i}"),
                    "network": format!("OpaqueRef:net-{net}"),
                    "device": "0",
                    "MAC": "02:00:00:00:00:01",
                    "currently_attached": true
                }),
            ));
        }

        let mut store = Store::default();
        let assert_matches_full = |label: &str, store: &Store, inc: &Inventory| {
            let full = store.full_inventory().unwrap();
            assert_eq!(
                serde_json::to_value(inc).unwrap(),
                serde_json::to_value(&full).unwrap(),
                "incremental inventory diverged from full decode after {label}"
            );
        };

        let mut init_batch = batch(events);
        for class in OPTIONAL_CLASSES {
            init_batch
                .classes
                .insert(class.to_ascii_lowercase(), ClassAvailability::Available);
        }
        let inv = store.apply(&init_batch).unwrap();
        assert_matches_full("initial bootstrap", &store, &inv);

        // 1. Task add -> progress mod -> VM referencing task -> task del
        let inv = store
            .apply(&batch(vec![event(
                "task",
                "t-1",
                json!({"uuid": uuid(90_001), "name_label": "Snapshot", "status": "pending", "progress": 0.1, "resident_on": "OpaqueRef:host-0"}),
            )]))
            .unwrap();
        assert_matches_full("task add", &store, &inv);

        let inv = store
            .apply(&batch(vec![event(
                "task",
                "t-1",
                json!({"uuid": uuid(90_001), "name_label": "Snapshot", "status": "pending", "progress": 0.65, "resident_on": "OpaqueRef:host-0"}),
            )]))
            .unwrap();
        assert_matches_full("task progress mod", &store, &inv);

        let inv = store
            .apply(&batch(vec![event(
                "VM",
                "vm-5",
                json!({
                    "uuid": uuid(10_005),
                    "name_label": "aaa-renamed-vm-5",
                    "power_state": "Running",
                    "resident_on": "OpaqueRef:host-2",
                    "metrics": "OpaqueRef:vmm-5",
                    "guest_metrics": "OpaqueRef:vmgm-5",
                    "is_a_snapshot": false,
                    "is_a_template": false,
                    "is_control_domain": false,
                    "VCPUs_at_startup": "4",
                    "memory_static_max": "4294967296",
                    "allowed_operations": [],
                    "current_operations": {"OpaqueRef:t-1": "snapshot"}
                }),
            )]))
            .unwrap();
        assert_matches_full("vm rename + current_operations", &store, &inv);
        assert_eq!(
            inv.details.vms[&VmId::from(uuid(10_005))].current_operations[0].task,
            Some(TaskId::from(uuid(90_001)))
        );

        let inv = store
            .apply(&batch(vec![
                json!({"class":"task","operation":"del","ref":"OpaqueRef:t-1"}),
            ]))
            .unwrap();
        assert_matches_full(
            "task del clears vm current_operations task link",
            &store,
            &inv,
        );
        assert_eq!(
            inv.details.vms[&VmId::from(uuid(10_005))].current_operations[0].task,
            None
        );

        // 2. Metrics & VDI in-place mod (same SR and moved SR)
        let inv = store
            .apply(&batch(vec![
                event(
                    "VM_metrics",
                    "vmm-10",
                    json!({"uuid": uuid(20_010), "memory_actual": "1073741824", "VCPUs_number": "4"}),
                ),
                event(
                    "VDI",
                    "vdi-10",
                    json!({
                        "uuid": uuid(40_010),
                        "name_label": "disk-10",
                        "SR": "OpaqueRef:sr-1",
                        "virtual_size": "21474836480",
                        "physical_utilisation": "2147483648"
                    }),
                ),
            ]))
            .unwrap();
        assert_matches_full("metrics + vdi sr change", &store, &inv);

        // 3. Snapshot add & mod & VM del
        let inv = store
            .apply(&batch(vec![event(
                "VM",
                "snap-1",
                json!({
                    "uuid": uuid(80_001),
                    "name_label": "snap-1",
                    "is_a_snapshot": true,
                    "is_a_template": true,
                    "is_control_domain": false,
                    "snapshot_of": "OpaqueRef:vm-5",
                    "parent": "OpaqueRef:vm-5",
                    "power_state": "Halted",
                    "allowed_operations": ["revert", "clone"]
                }),
            )]))
            .unwrap();
        assert_matches_full("snapshot add", &store, &inv);

        let inv = store
            .apply(&batch(vec![
                json!({"class":"VM","operation":"del","ref":"OpaqueRef:vm-5"}),
            ]))
            .unwrap();
        assert_matches_full("vm del orphans snapshot + vbd + vif", &store, &inv);

        // 4. Optional class corruption -> unrelated batch -> recovery
        let inv = store
            .apply(&batch(vec![event(
                "VDI",
                "vdi-1",
                json!({"uuid": uuid(40_001), "virtual_size": "not-a-number"}),
            )]))
            .unwrap();
        assert_eq!(inv.details.classes["vdi"], ClassAvailability::InvalidData);
        assert_matches_full("vdi corruption", &store, &inv);

        let inv = store
            .apply(&batch(vec![event(
                "VM_metrics",
                "vmm-1",
                json!({"uuid": uuid(20_001), "memory_actual": "536870912"}),
            )]))
            .unwrap();
        assert_eq!(inv.details.classes["vdi"], ClassAvailability::InvalidData);
        assert_matches_full("unrelated batch keeps vdi invalid", &store, &inv);

        let inv = store
            .apply(&batch(vec![event(
                "VDI",
                "vdi-1",
                json!({"uuid": uuid(40_001), "name_label": "disk-1", "SR": "OpaqueRef:sr-1", "virtual_size": "21474836480"}),
            )]))
            .unwrap();
        assert_eq!(inv.details.classes["vdi"], ClassAvailability::Available);
        assert_matches_full("vdi recovery", &store, &inv);

        // 5. Reset batch rebuilds cleanly
        let mut reset = batch(initial());
        reset.rebuild();
        let inv = store.apply(&reset).unwrap();
        assert_matches_full("reset batch", &store, &inv);
    }

    #[test]
    #[ignore = "measurement: run in release with -- --ignored --nocapture"]
    fn measure_large_cluster_store_apply() {
        let mut events = vec![event(
            "pool",
            "pool",
            json!({"uuid":uuid(1),"name_label":"Production Pool","master":"OpaqueRef:host-0","default_SR":"OpaqueRef:sr-0"}),
        )];
        for h in 0u128..16 {
            events.push(event(
                "host",
                &format!("host-{h}"),
                json!({
                    "uuid": uuid(100 + h),
                    "name_label": format!("host-{h:02}"),
                    "address": format!("10.0.0.{}", h + 1),
                    "enabled": true,
                    "control_domain": format!("OpaqueRef:dom0-{h}"),
                    "metrics": format!("OpaqueRef:hm-{h}"),
                    "software_version": {"product_version": "8.3.0"},
                    "current_operations": {}
                }),
            ));
            events.push(event(
                "host_metrics",
                &format!("hm-{h}"),
                json!({"uuid": uuid(200 + h), "live": true, "memory_total": "274877906944", "memory_free": "137438953472"}),
            ));
            events.push(event(
                "VM",
                &format!("dom0-{h}"),
                json!({
                    "uuid": uuid(300 + h),
                    "name_label": format!("Control domain on host-{h:02}"),
                    "power_state": "Running",
                    "resident_on": format!("OpaqueRef:host-{h}"),
                    "is_a_snapshot": false,
                    "is_a_template": false,
                    "is_control_domain": true,
                    "VCPUs_at_startup": "8",
                    "memory_static_max": "8589934592"
                }),
            ));
        }
        for s in 0u128..8 {
            events.push(event(
                "SR",
                &format!("sr-{s}"),
                json!({
                    "uuid": uuid(400 + s),
                    "name_label": format!("SR-{s}"),
                    "type": "nfs",
                    "physical_size": "10995116277760",
                    "physical_utilisation": "1099511627776"
                }),
            ));
        }
        for n in 0u128..16 {
            events.push(event(
                "network",
                &format!("net-{n}"),
                json!({"uuid": uuid(500 + n), "name_label": format!("VLAN-{n}"), "bridge": format!("xenbr{n}")}),
            ));
        }
        for i in 0u128..500 {
            let host = i % 16;
            let sr = i % 8;
            let net = i % 16;
            events.push(event(
                "VM",
                &format!("vm-{i}"),
                json!({
                    "uuid": uuid(10_000 + i),
                    "name_label": format!("vm-{i:04}"),
                    "power_state": "Running",
                    "resident_on": format!("OpaqueRef:host-{host}"),
                    "metrics": format!("OpaqueRef:vmm-{i}"),
                    "guest_metrics": format!("OpaqueRef:vmgm-{i}"),
                    "is_a_snapshot": false,
                    "is_a_template": false,
                    "is_control_domain": false,
                    "VCPUs_at_startup": "4",
                    "memory_static_max": "8589934592",
                    "allowed_operations": ["clean_shutdown", "snapshot", "pool_migrate"],
                    "current_operations": {}
                }),
            ));
            events.push(event(
                "VM_metrics",
                &format!("vmm-{i}"),
                json!({"uuid": uuid(20_000 + i), "memory_actual": "8589934592", "VCPUs_number": "4"}),
            ));
            events.push(event(
                "VM_guest_metrics",
                &format!("vmgm-{i}"),
                json!({"uuid": uuid(30_000 + i), "live": true, "networks": {"0/ip": format!("10.2.{}.{}", i / 250, (i % 250) + 1)}}),
            ));
            for d in 0u128..3 {
                let did = i * 3 + d;
                events.push(event(
                    "VDI",
                    &format!("vdi-{did}"),
                    json!({
                        "uuid": uuid(40_000 + did),
                        "name_label": format!("disk-{did:04}"),
                        "SR": format!("OpaqueRef:sr-{sr}"),
                        "virtual_size": "107374182400",
                        "physical_utilisation": "10737418240"
                    }),
                ));
                events.push(event(
                    "VBD",
                    &format!("vbd-{did}"),
                    json!({
                        "uuid": uuid(50_000 + did),
                        "VM": format!("OpaqueRef:vm-{i}"),
                        "VDI": format!("OpaqueRef:vdi-{did}"),
                        "device": format!("xvd{}", (b'a' + d as u8) as char),
                        "userdevice": d.to_string(),
                        "type": "Disk",
                        "mode": "RW",
                        "currently_attached": true
                    }),
                ));
            }
            for nic in 0u128..2 {
                let nid = i * 2 + nic;
                events.push(event(
                    "VIF",
                    &format!("vif-{nid}"),
                    json!({
                        "uuid": uuid(60_000 + nid),
                        "VM": format!("OpaqueRef:vm-{i}"),
                        "network": format!("OpaqueRef:net-{}", (net + nic) % 16),
                        "device": nic.to_string(),
                        "MAC": format!("02:00:00:00:{:02x}:{:02x}", (nid >> 8) & 0xff, nid & 0xff),
                        "currently_attached": true
                    }),
                ));
            }
        }
        let mut store = Store::default();
        let mut init_batch = batch(events);
        for class in OPTIONAL_CLASSES {
            init_batch
                .classes
                .insert(class.to_ascii_lowercase(), ClassAvailability::Available);
        }
        let t0 = std::time::Instant::now();
        let _ = store.apply(&init_batch).unwrap();
        let init_ms = t0.elapsed().as_secs_f64() * 1000.0;

        let t_full = std::time::Instant::now();
        for _ in 0..20 {
            let _ = std::hint::black_box(store.full_inventory().unwrap());
        }
        let full_ms = t_full.elapsed().as_secs_f64() * 1000.0 / 20.0;

        let task_batch = batch(vec![event(
            "task",
            "t-bench",
            json!({"uuid": uuid(99_999), "name_label": "Snapshot", "status": "pending", "progress": 0.5, "resident_on": "OpaqueRef:host-0"}),
        )]);
        let _ = store.apply(&task_batch).unwrap();
        let t_task = std::time::Instant::now();
        for _ in 0..50 {
            let _ = std::hint::black_box(store.apply(&task_batch).unwrap());
        }
        let task_ms = t_task.elapsed().as_secs_f64() * 1000.0 / 50.0;

        let vm_batch = batch(vec![event(
            "VM",
            "vm-42",
            json!({
                "uuid": uuid(10_042),
                "name_label": "vm-0042",
                "power_state": "Running",
                "resident_on": "OpaqueRef:host-10",
                "metrics": "OpaqueRef:vmm-42",
                "guest_metrics": "OpaqueRef:vmgm-42",
                "is_a_snapshot": false,
                "is_a_template": false,
                "is_control_domain": false,
                "VCPUs_at_startup": "4",
                "memory_static_max": "8589934592",
                "allowed_operations": ["clean_shutdown", "snapshot"],
                "current_operations": {}
            }),
        )]);
        let t_vm = std::time::Instant::now();
        for _ in 0..50 {
            let _ = std::hint::black_box(store.apply(&vm_batch).unwrap());
        }
        let vm_ms = t_vm.elapsed().as_secs_f64() * 1000.0 / 50.0;

        eprintln!(
            "16 hosts / 500 VMs / 1500 VDIs / 1500 VBDs / 1000 VIFs: init={init_ms:.2}ms full_decode={full_ms:.2}ms task_delta={task_ms:.2}ms vm_delta={vm_ms:.2}ms"
        );
    }
}
