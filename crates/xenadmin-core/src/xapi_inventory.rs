use super::*;
use crate::model::*;
use std::collections::BTreeSet;
use uuid::Uuid;

struct References {
    classes: BTreeMap<String, BTreeMap<String, Uuid>>,
    snapshots: BTreeSet<String>,
}
impl References {
    fn new(store: &Store) -> Self {
        let mut classes = BTreeMap::new();
        let mut snapshots = BTreeSet::new();
        for (class, records) in &store.records {
            let mut ids = BTreeMap::new();
            for (reference, record) in records {
                if let Some(id) = record["uuid"].as_str().and_then(|value| value.parse().ok()) {
                    ids.insert(reference.clone(), id);
                    if class == "vm" && record["is_a_snapshot"] == true {
                        snapshots.insert(reference.clone());
                    }
                }
            }
            classes.insert(class.clone(), ids);
        }
        Self { classes, snapshots }
    }
    fn link<T: From<Uuid>>(&self, class: &str, reference: &Value) -> Option<T> {
        self.classes
            .get(class)?
            .get(reference.as_str()?)
            .copied()
            .map(T::from)
    }
    fn vm(&self, reference: &Value) -> Option<VmId> {
        if self.snapshots.contains(reference.as_str()?) {
            return None;
        }
        self.link("vm", reference)
    }
    fn snapshot(&self, reference: &Value) -> Option<crate::SnapshotId> {
        if !self.snapshots.contains(reference.as_str()?) {
            return None;
        }
        self.link("vm", reference)
    }
    fn links<T: From<Uuid>>(&self, class: &str, references: &Value) -> Vec<T> {
        references
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|r| self.link(class, r))
            .collect()
    }
}

fn id<T: From<Uuid>>(record: &Value) -> Result<T, Error> {
    required(record, "uuid")?
        .parse::<Uuid>()
        .map(T::from)
        .map_err(|_| Error::Protocol("invalid optional inventory UUID"))
}
fn text_option(record: &Value, key: &str) -> Option<String> {
    record[key]
        .as_str()
        .filter(|value| !value.is_empty())
        .map(str::to_owned)
}
fn strings(value: &Value) -> Option<Vec<String>> {
    value
        .as_array()?
        .iter()
        .map(|v| v.as_str().map(str::to_owned))
        .collect()
}
fn string_map(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
        .into_iter()
        .flatten()
        .filter_map(|(key, value)| value.as_str().map(|value| (key.clone(), value.to_owned())))
        .collect()
}

fn blocked_operations(value: &Value) -> BTreeMap<String, String> {
    value
        .as_object()
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
    VmDetails {
        allowed_operations: strings(&record["allowed_operations"]),
        current_operations: record["current_operations"]
            .as_object()
            .into_iter()
            .flatten()
            .filter_map(|(_, operation)| {
                operation.as_str().map(|operation| RunningOperation {
                    task: None,
                    operation: operation.into(),
                })
            })
            .collect(),
        blocked_operations: blocked_operations(&record["blocked_operations"]),
        ..Default::default()
    }
    .power_availability(action, true)
}
fn size(record: &Value, key: &str, field: &'static str) -> Result<Option<u64>, Error> {
    storage_number(record, key, field)
}
fn boolean(record: &Value, key: &str) -> Result<Option<bool>, Error> {
    match record.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::Bool(value)) => Ok(Some(*value)),
        _ => Err(Error::Protocol("invalid optional inventory boolean")),
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

pub(super) fn decode(store: &Store) -> InventoryDetails {
    let refs = References::new(store);
    let mut result = InventoryDetails {
        classes: store.classes.clone(),
        ..Default::default()
    };
    result.pool = store.class("pool").next().and_then(|(_, record)| {
        let default_sr = refs.link("sr", &record["default_SR"]);
        Some(PoolDetails {
            id: id(record).ok()?,
            description: string(record, "name_description"),
            default_sr,
            default_sr_known: record["default_SR"] == "OpaqueRef:NULL" || default_sr.is_some(),
        })
    });
    for class in OPTIONAL_CLASSES {
        result
            .classes
            .entry(class.to_ascii_lowercase())
            .or_insert(ClassAvailability::Unknown);
    }
    result.storage = records(store, "sr", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            StorageDetails {
                id,
                description: string(record, "name_description"),
                content_type: string(record, "content_type"),
                shared: boolean(record, "shared")?,
                virtual_allocation: size(record, "virtual_allocation", "SR.virtual_allocation")?,
                allowed_operations: strings(&record["allowed_operations"]),
                tags: strings(&record["tags"]).unwrap_or_default(),
            },
        ))
    });
    result.networks = records(store, "network", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            NetworkDetails {
                id,
                description: string(record, "name_description"),
                mtu: size(record, "MTU", "network.MTU")?,
                managed: boolean(record, "managed")?,
                default_locking_mode: text_option(record, "default_locking_mode"),
                allowed_operations: strings(&record["allowed_operations"]),
                tags: strings(&record["tags"]).unwrap_or_default(),
            },
        ))
    });
    result.tasks = records(store, "task", &mut result.classes, |record| {
        let id = id(record)?;
        let progress = record["progress"]
            .as_f64()
            .or_else(|| record["progress"].as_str()?.parse().ok())
            .filter(|n| n.is_finite() && (0.0..=1.0).contains(n));
        Ok((
            id,
            ServerTask {
                id,
                name: label(record),
                description: string(record, "name_description"),
                status: string(record, "status"),
                progress,
                created: text_option(record, "created"),
                finished: text_option(record, "finished"),
                error_code: record["error_info"]
                    .as_array()
                    .and_then(|values| values.first())
                    .and_then(Value::as_str)
                    .map(|code| safe_error_code(code).to_owned()),
                allowed_operations: strings(&record["allowed_operations"]),
            },
        ))
    });
    result.vdis = records(store, "vdi", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            VirtualDisk {
                id,
                name: label(record),
                description: string(record, "name_description"),
                sr: refs.link("sr", &record["SR"]),
                virtual_size: size(record, "virtual_size", "VDI.virtual_size")?,
                physical_utilisation: size(
                    record,
                    "physical_utilisation",
                    "VDI.physical_utilisation",
                )?,
                kind: string(record, "type"),
                read_only: boolean(record, "read_only")?,
                sharable: boolean(record, "sharable")?,
                managed: boolean(record, "managed")?,
                missing: boolean(record, "missing")?,
                is_snapshot: boolean(record, "is_a_snapshot")?,
                snapshot_of: refs.link("vdi", &record["snapshot_of"]),
                cbt_enabled: boolean(record, "cbt_enabled")?,
                allowed_operations: strings(&record["allowed_operations"]),
            },
        ))
    });
    result.vbds = records(store, "vbd", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            VirtualBlockDevice {
                id,
                vm: refs.vm(&record["VM"]),
                snapshot: refs.snapshot(&record["VM"]),
                vdi: refs.link("vdi", &record["VDI"]),
                device: string(record, "device"),
                userdevice: string(record, "userdevice"),
                kind: string(record, "type"),
                mode: string(record, "mode"),
                currently_attached: boolean(record, "currently_attached")?,
                bootable: boolean(record, "bootable")?,
                empty: boolean(record, "empty")?,
                owned: record["other_config"]
                    .get("owner")
                    .is_some_and(Value::is_string),
                allowed_operations: strings(&record["allowed_operations"]),
            },
        ))
    });
    result.vifs = records(store, "vif", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            VirtualInterface {
                id,
                vm: refs.vm(&record["VM"]),
                network: refs.link("network", &record["network"]),
                device: string(record, "device"),
                mac: string(record, "MAC"),
                mtu: size(record, "MTU", "VIF.MTU")?,
                currently_attached: boolean(record, "currently_attached")?,
                qos_algorithm: string(record, "qos_algorithm_type"),
                qos_parameters: string_map(&record["qos_algorithm_params"]),
                allowed_operations: strings(&record["allowed_operations"]),
            },
        ))
    });
    result.pifs = records(store, "pif", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            PhysicalInterface {
                id,
                host: refs.link("host", &record["host"]),
                network: refs.link("network", &record["network"]),
                device: string(record, "device"),
                mac: string(record, "MAC"),
                mtu: size(record, "MTU", "PIF.MTU")?,
                vlan: integer(record, "VLAN", "PIF.VLAN")?,
                management: boolean(record, "management")?,
                physical: boolean(record, "physical")?,
                currently_attached: boolean(record, "currently_attached")?,
                ip_configuration_mode: string(record, "ip_configuration_mode"),
                ip: string(record, "IP"),
                netmask: string(record, "netmask"),
                gateway: string(record, "gateway"),
                dns: string(record, "DNS"),
                ipv6: strings(&record["IPv6"]).unwrap_or_default(),
                ipv6_configuration_mode: string(record, "ipv6_configuration_mode"),
                ipv6_gateway: string(record, "ipv6_gateway"),
            },
        ))
    });
    result.pbds = records(store, "pbd", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            StorageAttachment {
                id,
                host: refs.link("host", &record["host"]),
                sr: refs.link("sr", &record["SR"]),
                currently_attached: boolean(record, "currently_attached")?,
            },
        ))
    });
    result.bonds = records(store, "bond", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            NetworkBond {
                id,
                master: refs.link("pif", &record["master"]),
                slaves: refs.links("pif", &record["slaves"]),
                mode: string(record, "mode"),
                links_up: size(record, "links_up", "Bond.links_up")?,
            },
        ))
    });
    result.vlans = records(store, "vlan", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            Vlan {
                id,
                tagged_pif: refs.link("pif", &record["tagged_PIF"]),
                untagged_pif: refs.link("pif", &record["untagged_PIF"]),
                tag: size(record, "tag", "VLAN.tag")?,
            },
        ))
    });
    result.guest_metrics = records(store, "vm_guest_metrics", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            GuestMetrics {
                id,
                os_version: string_map(&record["os_version"]),
                tools_version: string_map(&record["PV_drivers_version"]),
                networks: string_map(&record["networks"]),
                live: boolean(record, "live")?,
                last_updated: text_option(record, "last_updated"),
            },
        ))
    });
    result.vm_metrics = records(store, "vm_metrics", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            VmMetrics {
                id,
                memory_actual: size(record, "memory_actual", "VM_metrics.memory_actual")?,
                vcpus_number: size(record, "VCPUs_number", "VM_metrics.VCPUs_number")?,
                start_time: text_option(record, "start_time"),
            },
        ))
    });
    result.host_metrics = records(store, "host_metrics", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            HostMetrics {
                id,
                live: boolean(record, "live")?,
                memory_total: size(record, "memory_total", "host_metrics.memory_total")?,
                memory_free: size(record, "memory_free", "host_metrics.memory_free")?,
                last_updated: text_option(record, "last_updated"),
            },
        ))
    });
    result.messages = records(store, "message", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            ServerMessage {
                id,
                name: string(record, "name"),
                priority: integer(record, "priority", "message.priority")?,
                object_class: string(record, "cls"),
                object_uuid: record["obj_uuid"].as_str().and_then(|s| s.parse().ok()),
                timestamp: text_option(record, "timestamp"),
                body: string(record, "body"),
            },
        ))
    });
    result.appliances = records(store, "vm_appliance", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            VmAppliance {
                id,
                name: label(record),
                description: string(record, "name_description"),
                vms: record["VMs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| refs.vm(r))
                    .collect(),
                allowed_operations: strings(&record["allowed_operations"]),
            },
        ))
    });
    result.snapshot_policies = records(store, "vmss", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            SnapshotPolicy {
                id,
                name: label(record),
                description: string(record, "name_description"),
                enabled: boolean(record, "enabled")?,
                kind: string(record, "type"),
                frequency: string(record, "frequency"),
                retained_snapshots: size(record, "retained_snapshots", "VMSS.retained_snapshots")?,
                schedule: string_map(&record["schedule"]),
                vms: record["VMs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| refs.vm(r))
                    .collect(),
            },
        ))
    });
    result.storage_managers = records(store, "sm", &mut result.classes, |record| {
        let id = id(record)?;
        Ok((
            id,
            StorageManager {
                id,
                kind: string(record, "type"),
                name: label(record),
                capabilities: strings(&record["capabilities"]).unwrap_or_default(),
            },
        ))
    });
    for (_, record) in store
        .class("vm")
        .filter(|(_, r)| r["is_a_snapshot"] != true)
    {
        if let Ok(id) = id(record) {
            result.vms.insert(
                id,
                VmDetails {
                    is_default_template: Some(
                        record["is_default_template"] == true
                            || record["other_config"]["default_template"] == "true",
                    ),
                    allowed_operations: strings(&record["allowed_operations"]),
                    current_operations: record["current_operations"]
                        .as_object()
                        .into_iter()
                        .flatten()
                        .filter_map(|(task, operation)| {
                            operation.as_str().map(|operation| RunningOperation {
                                task: refs.link("task", &json!(task)),
                                operation: operation.into(),
                            })
                        })
                        .collect(),
                    blocked_operations: blocked_operations(&record["blocked_operations"]),
                    vcpus_max: size(record, "VCPUs_max", "VM.VCPUs_max").ok().flatten(),
                    vcpus_at_startup: size(record, "VCPUs_at_startup", "VM.VCPUs_at_startup")
                        .ok()
                        .flatten(),
                    memory_static_min: size(record, "memory_static_min", "VM.memory_static_min")
                        .ok()
                        .flatten(),
                    memory_static_max: size(record, "memory_static_max", "VM.memory_static_max")
                        .ok()
                        .flatten(),
                    memory_dynamic_min: size(record, "memory_dynamic_min", "VM.memory_dynamic_min")
                        .ok()
                        .flatten(),
                    memory_dynamic_max: size(record, "memory_dynamic_max", "VM.memory_dynamic_max")
                        .ok()
                        .flatten(),
                    memory_target: size(record, "memory_target", "VM.memory_target")
                        .ok()
                        .flatten(),
                    boot_order: text_option(&record["HVM_boot_params"], "order"),
                    boot_policy: text_option(record, "HVM_boot_policy"),
                    firmware: text_option(&record["HVM_boot_params"], "firmware"),
                    affinity: refs.link("host", &record["affinity"]),
                    guest_metrics: refs.link("vm_guest_metrics", &record["guest_metrics"]),
                    metrics: refs.link("vm_metrics", &record["metrics"]),
                    suspend_vdi: refs.link("vdi", &record["suspend_VDI"]),
                    appliance: refs.link("vm_appliance", &record["appliance"]),
                    snapshot_policy: refs.link("vmss", &record["snapshot_schedule"]),
                    tags: strings(&record["tags"]).unwrap_or_default(),
                    ha_restart_priority: text_option(record, "ha_restart_priority"),
                    auto_power_on: record["other_config"]["auto_poweron"]
                        .as_str()
                        .and_then(|v| v.parse().ok()),
                },
            );
        }
    }
    for (_, record) in store.class("host") {
        if let Ok(id) = id(record) {
            result.hosts.insert(
                id,
                HostDetails {
                    description: string(record, "name_description"),
                    hostname: string(record, "hostname"),
                    metrics: refs.link("host_metrics", &record["metrics"]),
                    software_version: string_map(&record["software_version"]),
                    cpu_info: string_map(&record["cpu_info"]),
                    capabilities: strings(&record["capabilities"]).unwrap_or_default(),
                    tags: strings(&record["tags"]).unwrap_or_default(),
                    enabled: record["enabled"].as_bool(),
                },
            );
        }
    }
    for (id, disk) in &result.vdis {
        if let Some(sr) = disk.sr {
            result.sr_disks.entry(sr).or_default().push(*id);
        }
    }
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
    for (id, interface) in &result.pifs {
        if let Some(host) = interface.host {
            result.host_interfaces.entry(host).or_default().push(*id);
        }
        if let Some(network) = interface.network {
            result.network_uplinks.entry(network).or_default().push(*id);
        }
    }
    for (id, attachment) in &result.pbds {
        if let Some(sr) = attachment.sr {
            result.sr_attachments.entry(sr).or_default().push(*id);
        }
    }
    result
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
}
