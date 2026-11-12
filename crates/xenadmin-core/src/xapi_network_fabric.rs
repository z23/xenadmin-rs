use super::*;
use crate::{
    NetworkFabricInventory, NetworkPciDevice, NetworkSriov, NetworkTunnel, SdnController,
    SdnControllerConfiguration, SriovConfigurationMode, TunnelProtocol,
};
use xenadmin_xapi::TextField;
use xenadmin_xapi::records::{
    NetworkSriovRecord, PciRecord, PifRecord, PoolRecord, SdnControllerRecord, TunnelRecord,
};

fn text(field: TextField<'_>, limit: usize) -> Option<String> {
    field
        .as_str()
        .filter(|v| !v.is_empty() && v.len() <= limit && !v.chars().any(char::is_control))
        .map(str::to_owned)
}
fn controller(record: &Value) -> Option<SdnControllerConfiguration> {
    let view = SdnControllerRecord::new(record);
    let port = integer(view.port(), "controller port").ok()??;
    match view.protocol().as_str()? {
        "ssl" => Some(SdnControllerConfiguration::Ssl {
            address: view.address().as_str()?.parse().ok()?,
            port: u16::try_from(port).ok().filter(|port| *port != 0)?,
        }),
        "pssl" if *view.address() == "" && port == 0 => {
            Some(SdnControllerConfiguration::PassiveSsl)
        }
        _ => None,
    }
}
pub(super) fn decode(
    store: &Store,
    refs: &References,
    states: &mut BTreeMap<String, ClassAvailability>,
) -> NetworkFabricInventory {
    let controllers = records(store, "sdn_controller", states, |record| {
        let view = SdnControllerRecord::new(record);
        let id = id(view.uuid())?;
        Ok((
            id,
            SdnController {
                id,
                configuration: controller(record),
            },
        ))
    });
    let tunnels = records(store, "tunnel", states, |record| {
        let view = TunnelRecord::new(record);
        let id = id(view.uuid())?;
        let status = view
            .status()
            .value()
            .as_object()
            .filter(|map| map.len() <= 32 && map.values().all(Value::is_string));
        let active = status
            .and_then(|map| map.get("active"))
            .and_then(Value::as_str)
            .and_then(|v| match v {
                "true" => Some(true),
                "false" => Some(false),
                _ => None,
            });
        Ok((
            id,
            NetworkTunnel {
                id,
                transport: refs.link("pif", view.transport_pif()),
                access: refs.link("pif", view.access_pif()),
                protocol: view.protocol().as_str().and_then(TunnelProtocol::parse),
                controller_active: active,
                controller_error: status.map(|map| {
                    map.get("error")
                        .and_then(Value::as_str)
                        .is_some_and(|v| !v.is_empty())
                }),
            },
        ))
    });
    let sriovs = records(store, "network_sriov", states, |record| {
        let view = NetworkSriovRecord::new(record);
        let id = id(view.uuid())?;
        Ok((
            id,
            NetworkSriov {
                id,
                physical: refs.link("pif", view.physical_pif()),
                logical: refs.link("pif", view.logical_pif()),
                requires_reboot: boolean(view.requires_reboot())?,
                mode: view
                    .configuration_mode()
                    .as_str()
                    .and_then(SriovConfigurationMode::parse),
            },
        ))
    });
    let pci_devices = records(store, "pci", states, |record| {
        let view = PciRecord::new(record);
        let id = id(view.uuid())?;
        Ok((
            id,
            NetworkPciDevice {
                id,
                host: refs.link("host", view.host()),
                address: text(view.pci_id(), 64),
                vendor_id: text(view.vendor_id(), 16),
                device_id: text(view.device_id(), 16),
                driver: text(view.driver_name(), 128),
            },
        ))
    });
    let mut result = NetworkFabricInventory {
        controllers,
        tunnels,
        sriovs,
        pci_devices,
        ..Default::default()
    };
    for (_, record) in store.class("pif") {
        let view = PifRecord::new(record);
        if let (Ok(id), Some(pci)) = (id(view.uuid()), refs.link("pci", view.pci())) {
            result.pif_pci.insert(id, pci);
        }
    }
    result.pool_sriov_restricted = store.class("pool").next().and_then(|(_, p)| {
        let view = PoolRecord::new(p);
        match view.restrictions()["restrict_network_sriov"].as_str()? {
            "true" => Some(true),
            "false" => Some(false),
            _ => None,
        }
    });
    result
}

#[cfg(test)]
mod tests {
    use super::*;
    fn store() -> Store {
        let mut store = Store::default();
        let rows = [
            (
                "pool",
                "pool",
                json!({"uuid":Uuid::from_u128(1),"restrictions":{"restrict_network_sriov":"false"}}),
            ),
            ("host", "host", json!({"uuid":Uuid::from_u128(2)})),
            (
                "pif",
                "physical",
                json!({"uuid":Uuid::from_u128(3),"PCI":"OpaqueRef:pci"}),
            ),
            ("pif", "logical", json!({"uuid":Uuid::from_u128(4)})),
            (
                "pci",
                "pci",
                json!({"uuid":Uuid::from_u128(5),"host":"OpaqueRef:host","pci_id":"0000:01:00.0","vendor_id":"8086","device_id":"1234","driver_name":"fixture"}),
            ),
            (
                "sdn_controller",
                "controller",
                json!({"uuid":Uuid::from_u128(6),"protocol":"ssl","address":"192.0.2.50","port":"6632"}),
            ),
            (
                "tunnel",
                "tunnel",
                json!({"uuid":Uuid::from_u128(7),"transport_PIF":"OpaqueRef:physical","access_PIF":"OpaqueRef:logical","protocol":"vxlan","status":{"active":"false"}}),
            ),
            (
                "network_sriov",
                "sriov",
                json!({"uuid":Uuid::from_u128(8),"physical_PIF":"OpaqueRef:physical","logical_PIF":"OpaqueRef:logical","requires_reboot":true,"configuration_mode":"modprobe"}),
            ),
        ];
        for (class, reference, value) in rows {
            store
                .records
                .entry(class.into())
                .or_default()
                .insert(format!("OpaqueRef:{reference}"), value);
            store
                .classes
                .insert(class.into(), ClassAvailability::Available);
        }
        store
    }
    #[test]
    fn fabric_inventory_resolves_links_without_confusing_configuration_with_activity() {
        let details = super::super::decode(&store());
        let fabric = &details.network_fabric;
        assert_eq!(fabric.pool_sriov_restricted, Some(false));
        let tunnel = &fabric.tunnels[&TunnelId::from(Uuid::from_u128(7))];
        assert_eq!(tunnel.transport, Some(PifId::from(Uuid::from_u128(3))));
        assert_eq!(tunnel.access, Some(PifId::from(Uuid::from_u128(4))));
        assert_eq!(tunnel.protocol, Some(TunnelProtocol::Vxlan));
        assert_eq!(tunnel.controller_active, Some(false));
        assert_eq!(
            fabric.sriovs[&NetworkSriovId::from(Uuid::from_u128(8))].requires_reboot,
            Some(true)
        );
        assert_eq!(
            fabric.pif_pci[&PifId::from(Uuid::from_u128(3))],
            PciId::from(Uuid::from_u128(5))
        );
    }
    #[test]
    fn fabric_inventory_preserves_unknown_unsupported_denied_and_empty_classes() {
        for state in [
            ClassAvailability::Unknown,
            ClassAvailability::Available,
            ClassAvailability::Unsupported,
            ClassAvailability::PermissionDenied,
        ] {
            let mut store = Store::default();
            for class in ["tunnel", "sdn_controller", "network_sriov", "pci"] {
                store.classes.insert(class.into(), state.clone());
            }
            let details = super::super::decode(&store);
            for class in ["tunnel", "sdn_controller", "network_sriov", "pci"] {
                assert_eq!(details.classes[class], state);
            }
            assert!(details.network_fabric.controllers.is_empty());
            assert_eq!(details.network_fabric.pool_sriov_restricted, None);
        }
    }
    #[test]
    fn unknown_controller_protocol_and_tunnel_status_remain_unknown_not_usable_defaults() {
        let mut store = store();
        store
            .records
            .get_mut("sdn_controller")
            .unwrap()
            .get_mut("OpaqueRef:controller")
            .unwrap()["protocol"] = json!("future-protocol");
        store
            .records
            .get_mut("tunnel")
            .unwrap()
            .get_mut("OpaqueRef:tunnel")
            .unwrap()["status"] = json!({"active":"future-state"});
        store
            .records
            .get_mut("network_sriov")
            .unwrap()
            .get_mut("OpaqueRef:sriov")
            .unwrap()
            .as_object_mut()
            .unwrap()
            .remove("requires_reboot");
        let details = super::super::decode(&store);
        assert!(
            details
                .network_fabric
                .controllers
                .values()
                .next()
                .unwrap()
                .configuration
                .is_none()
        );
        assert!(
            details
                .network_fabric
                .tunnels
                .values()
                .next()
                .unwrap()
                .controller_active
                .is_none()
        );
        assert!(
            details
                .network_fabric
                .sriovs
                .values()
                .next()
                .unwrap()
                .requires_reboot
                .is_none()
        );
    }
}
