use super::*;
use crate::{
    ManagementBondConsumer, ManagementBondConsumers, ManagementVmLocality, NetworkId, VifId,
    management_bond_consumers as model,
};
use sha2::{Digest, Sha256};
use std::collections::BTreeMap;
use xenadmin_xapi::records::{NetworkRecord, VifRecord, VmRecord};

fn text(v: &Value, key: &str, limit: usize) -> Result<String, Error> {
    v[key]
        .as_str()
        .filter(|s| s.len() <= limit)
        .map(str::to_owned)
        .ok_or(Error::Protocol(
            "Guest interface configuration is unavailable or exceeds bounds",
        ))
}
fn refs(value: &Value) -> Result<BTreeSet<String>, Error> {
    let a = value
        .as_array()
        .filter(|a| a.len() <= 10_000)
        .ok_or(Error::Protocol("Guest interface graph unavailable"))?;
    let ids = a
        .iter()
        .map(|v| {
            v.as_str()
                .filter(|s| s.len() <= 512 && s.starts_with("OpaqueRef:") && *s != "OpaqueRef:NULL")
                .map(str::to_owned)
                .ok_or(Error::Protocol("Guest interface graph invalid"))
        })
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(
        ids.len() == a.len(),
        "Guest interface graph has duplicate references",
    )?;
    Ok(ids)
}
fn idle(v: &Value) -> bool {
    v["current_operations"]
        .as_object()
        .is_some_and(|v| v.is_empty())
}
fn fingerprint(v: &Value) -> Result<[u8; 32], Error> {
    let mut fields = BTreeMap::new();
    for key in [
        "other_config",
        "qos_algorithm_type",
        "qos_algorithm_params",
        "locking_mode",
        "ipv4_allowed",
        "ipv6_allowed",
        "ipv4_configuration_mode",
        "ipv4_addresses",
        "ipv4_gateway",
        "ipv6_configuration_mode",
        "ipv6_addresses",
        "ipv6_gateway",
    ] {
        let mut value = v[key].clone();
        if let Some(values) = value.as_array_mut() {
            require(
                values.len() <= 1024
                    && values
                        .iter()
                        .all(|v| v.as_str().is_some_and(|s| s.len() <= 512) || v.is_number()),
                "Guest interface set exceeds supported bounds",
            )?;
            values.sort_by_key(Value::to_string);
            values.dedup();
        }
        fields.insert(key, value);
    }
    require(
        v["other_config"].is_object()
            && v["qos_algorithm_type"].is_string()
            && v["qos_algorithm_params"].is_object(),
        "Guest interface settings unavailable",
    )?;
    let bytes = serde_json::to_vec(&fields)
        .map_err(|_| Error::Protocol("Guest interface settings invalid"))?;
    require(
        bytes.len() <= 64 * 1024,
        "Guest interface settings exceed supported bounds",
    )?;
    Ok(Sha256::digest(bytes).into())
}
struct Graph {
    vifs: serde_json::Map<String, Value>,
    networks: serde_json::Map<String, Value>,
    hosts: serde_json::Map<String, Value>,
}
impl Graph {
    async fn read(s: &Session) -> Result<Self, Error> {
        let graph = Self {
            vifs: network::all(s, "VIF").await?,
            networks: network::all(s, "network").await?,
            hosts: network::all(s, "host").await?,
        };
        let mut hosts = BTreeSet::new();
        for v in graph.hosts.values() {
            require(
                hosts.insert(network::uuid(v)?),
                "Host identity is ambiguous across guest placement",
            )?;
        }
        let mut ids = BTreeSet::new();
        for v in graph.vifs.values() {
            require(
                ids.insert(network::uuid(v)?),
                "Duplicate guest interface identity",
            )?;
            let n = VifRecord::new(v)
                .network()
                .value()
                .as_str()
                .ok_or(Error::Protocol("Guest interface network is unavailable"))?;
            graph.network(n)?;
            require(
                VifRecord::new(v).vm().as_str().is_some_and(|v| {
                    v.starts_with("OpaqueRef:") && v != "OpaqueRef:NULL" && v.len() <= 512
                }),
                "Guest interface VM identity unavailable",
            )?;
        }
        Ok(graph)
    }
    fn network(&self, reference: &str) -> Result<NetworkId, Error> {
        self.networks
            .get(reference)
            .ok_or(Error::ResourceConflict)
            .and_then(|v| network::uuid(v).map(NetworkId::from))
    }
    fn host(&self, reference: &str) -> Result<HostId, Error> {
        self.hosts
            .get(reference)
            .ok_or(Error::ResourceConflict)
            .and_then(|v| network::uuid(v).map(HostId::from))
    }
    fn policies(
        &self,
        networks: &BTreeSet<NetworkId>,
    ) -> Result<BTreeMap<NetworkId, crate::ManagementBondNetwork>, Error> {
        let mut result = BTreeMap::new();
        for v in self.networks.values() {
            let id = NetworkId::from(network::uuid(v)?);
            if !networks.contains(&id) {
                continue;
            }
            let bytes = serde_json::to_vec(&NetworkRecord::new(v).other_config())
                .map_err(|_| Error::Protocol("Network metadata unavailable"))?;
            require(
                bytes.len() <= 64 * 1024,
                "Network metadata exceeds supported bounds",
            )?;
            result.insert(
                id,
                crate::ManagementBondNetwork {
                    name: text(v, "name_label", 1024)?,
                    mtu: network::number(&NetworkRecord::new(v).mtu())
                        .and_then(|v| u64::try_from(v).ok())
                        .ok_or(Error::ResourceConflict)?,
                    default_locking_mode: NetworkRecord::new(v)
                        .default_locking_mode()
                        .as_str()
                        .map(str::to_owned),
                    settings_fingerprint: Sha256::digest(bytes).into(),
                },
            );
        }
        require(
            result.keys().copied().collect::<BTreeSet<_>>() == *networks,
            "Reviewed network policies are incomplete",
        )?;
        Ok(result)
    }
    fn consumers(
        &self,
        networks: &BTreeSet<NetworkId>,
    ) -> Result<BTreeMap<VifId, (String, &Value)>, Error> {
        let mut result = BTreeMap::new();
        let mut found = BTreeSet::new();
        for (nr, n) in &self.networks {
            let id = NetworkId::from(network::uuid(n)?);
            if !networks.contains(&id) {
                continue;
            }
            require(found.insert(id), "Duplicate network identity")?;
            let members = self
                .vifs
                .iter()
                .filter(|(_, v)| v["network"] == *nr)
                .map(|(r, _)| r.clone())
                .collect::<BTreeSet<_>>();
            require(
                refs(&n["VIFs"])? == members,
                "Network guest interface membership is inconsistent",
            )?;
            for reference in members {
                let v = &self.vifs[&reference];
                let id = VifId::from(network::uuid(v)?);
                require(
                    result.insert(id, (reference, v)).is_none(),
                    "Duplicate guest interface identity",
                )?;
            }
        }
        require(
            found == *networks && result.len() <= 1024,
            "Guest interface impact is incomplete or exceeds supported bounds",
        )?;
        Ok(result)
    }
    async fn vm(&self, s: &Session, reference: &str) -> Result<Value, Error> {
        require(
            reference.starts_with("OpaqueRef:")
                && reference != "OpaqueRef:NULL"
                && reference.len() <= 512,
            "Guest interface VM reference is invalid",
        )?;
        let v = s.call("VM.get_record", vec![json!(reference)]).await?;
        require(
            VmRecord::new(&v).is_control_domain() == false,
            "Control-domain network changes require a separate workflow",
        )?;
        require(
            VmRecord::new(&v)
                .scheduled_to_be_resident_on()
                .as_str()
                .is_some_and(|r| r == "OpaqueRef:NULL")
                && idle(&v),
            "A guest is busy or scheduled for migration",
        )?;
        let interfaces = self
            .vifs
            .iter()
            .filter(|(_, v)| v["VM"] == reference)
            .map(|(r, _)| r.clone())
            .collect::<BTreeSet<_>>();
        require(
            refs(&VmRecord::new(&v).vifs())? == interfaces,
            "Guest VM interface membership is inconsistent",
        )?;
        Ok(v)
    }
    async fn locality(
        &self,
        s: &Session,
        reference: &str,
        vm: &Value,
    ) -> Result<ManagementVmLocality, Error> {
        let resident = vm["resident_on"]
            .as_str()
            .ok_or(Error::Protocol("Guest residency is unavailable"))?;
        if resident != "OpaqueRef:NULL" {
            return Ok(ManagementVmLocality::Resident(self.host(resident)?));
        }
        let value = s
            .call("VM.get_possible_hosts", vec![json!(reference)])
            .await?;
        let references = refs(&value)?;
        require(
            references.len() <= 1024,
            "Guest placement exceeds supported bounds",
        )?;
        let ids = references
            .iter()
            .map(|r| self.host(r))
            .collect::<Result<BTreeSet<_>, _>>()?;
        require(
            ids.len() == references.len(),
            "Guest placement has duplicate host identities",
        )?;
        Ok(ManagementVmLocality::PossibleHosts(
            ids.into_iter().collect(),
        ))
    }
    fn consumer(
        &self,
        id: VifId,
        v: &Value,
        vm: &Value,
        locality: ManagementVmLocality,
        source: &ManagementInterface,
        op: &crate::ManagementBondOperation,
    ) -> Result<ManagementBondConsumer, Error> {
        require(idle(v), "A guest interface is busy")?;
        let network = self.network(v["network"].as_str().ok_or(Error::ResourceConflict)?)?;
        let power_state = text(vm, "power_state", 32)?;
        let attached = v["currently_attached"]
            .as_bool()
            .ok_or(Error::Protocol("Guest attachment state unavailable"))?;
        let (_, moves, reattachments) =
            model::scope(source, op).ok_or(Error::InvalidResourceCommand)?;
        let (destination, change) = model::disposition(
            source.identity.host,
            &moves,
            &reattachments,
            network,
            &locality,
            attached,
            &power_state,
        );
        Ok(ManagementBondConsumer {
            interface: id,
            vm: network::uuid(vm)?.into(),
            vm_name: text(vm, "name_label", 1024)?,
            power_state,
            template: VmRecord::new(vm)
                .is_a_template()
                .as_bool()
                .ok_or(Error::ResourceConflict)?,
            snapshot: VmRecord::new(vm)
                .is_a_snapshot()
                .as_bool()
                .ok_or(Error::ResourceConflict)?,
            locality,
            network,
            destination,
            change,
            device: text(v, "device", 64)?,
            mac: text(v, "MAC", 64)?.to_ascii_lowercase(),
            mtu: network::number(&v["MTU"])
                .and_then(|v| u64::try_from(v).ok())
                .ok_or(Error::ResourceConflict)?,
            attached,
            settings_fingerprint: fingerprint(v)?,
        })
    }
}
pub(in crate::resources) async fn capture(
    s: &Session,
    source: &ManagementInterface,
    op: &crate::ManagementBondOperation,
) -> Result<ManagementBondConsumers, Error> {
    let (networks, _, _) = model::scope(source, op).ok_or(Error::InvalidResourceCommand)?;
    let graph = Graph::read(s).await?;
    let mut vms = BTreeMap::<String, (Value, ManagementVmLocality)>::new();
    let mut result = ManagementBondConsumers {
        networks: graph.policies(&networks)?,
        ..Default::default()
    };
    for (id, (_, v)) in graph.consumers(&networks)? {
        let reference = v["VM"].as_str().ok_or(Error::ResourceConflict)?;
        if !vms.contains_key(reference) {
            let vm = graph.vm(s, reference).await?;
            require(
                !vms.values().any(|(old, _)| old["uuid"] == vm["uuid"]),
                "VM identity is ambiguous across guest consumers",
            )?;
            let locality = graph.locality(s, reference, &vm).await?;
            vms.insert(reference.to_owned(), (vm, locality));
        }
        let (vm, locality) = &vms[reference];
        result
            .interfaces
            .insert(id, graph.consumer(id, v, vm, locality.clone(), source, op)?);
    }
    require(
        result.valid(source, op),
        "Guest interface impact is invalid",
    )?;
    Ok(result)
}
pub(in crate::resources) async fn before(
    s: &Session,
    operation: &ManagementNetworkOperation,
) -> Result<(), Error> {
    let ManagementNetworkOperation::Bond {
        source,
        operation,
        consumers,
    } = operation
    else {
        return Err(Error::InvalidResourceCommand);
    };
    let actual = capture(s, source, operation).await?;
    require(
        consumers
            .as_deref()
            .map_or(actual.interfaces.is_empty(), |v| v == &actual),
        "Guest interfaces, settings or placement changed since review; read their migration again",
    )
}
pub(in crate::resources) async fn verify(
    s: &Session,
    operation: &ManagementNetworkOperation,
) -> Result<(), Error> {
    let ManagementNetworkOperation::Bond {
        source,
        operation,
        consumers,
    } = operation
    else {
        return Err(Error::InvalidResourceCommand);
    };
    let (networks, _, _) = model::scope(source, operation).ok_or(Error::InvalidResourceCommand)?;
    let graph = Graph::read(s).await?;
    let actual = graph.consumers(&networks)?;
    if let Some(expected) = consumers {
        require(
            graph.policies(&networks)? == expected.networks,
            "Network policy changed during bond migration",
        )?;
    }
    let empty = ManagementBondConsumers::default();
    let expected = consumers.as_deref().unwrap_or(&empty);
    require(
        actual.keys().eq(expected.interfaces.keys()),
        "Guest interface consumers changed during bond migration",
    )?;
    let mut vms = BTreeMap::<String, Value>::new();
    for (id, (_, v)) in actual {
        let old = &expected.interfaces[&id];
        let reference = v["VM"].as_str().ok_or(Error::ResourceConflict)?;
        if !vms.contains_key(reference) {
            let vm = graph.vm(s, reference).await?;
            require(
                !vms.values().any(|old| old["uuid"] == vm["uuid"]),
                "VM identity is ambiguous across guest consumers",
            )?;
            vms.insert(reference.into(), vm);
        }
        let vm = &vms[reference];
        let resident = vm["resident_on"].as_str().ok_or(Error::ResourceConflict)?;
        let same_resident = match &old.locality {
            ManagementVmLocality::Resident(host) => {
                resident != "OpaqueRef:NULL" && graph.host(resident)? == *host
            }
            ManagementVmLocality::PossibleHosts(_) => resident == "OpaqueRef:NULL",
        };
        require(
            same_resident,
            "Guest residency changed during bond migration",
        )?;
        let now = graph.consumer(id, v, vm, old.locality.clone(), source, operation)?;
        require(
            now.vm == old.vm
                && now.vm_name == old.vm_name
                && now.power_state == old.power_state
                && now.template == old.template
                && now.snapshot == old.snapshot
                && now.network == old.destination
                && now.device == old.device
                && now.mac == old.mac
                && now.mtu == old.mtu
                && now.attached == old.attached
                && now.settings_fingerprint == old.settings_fingerprint,
            "Guest interface identity, network or settings did not match the reviewed migration",
        )?;
    }
    Ok(())
}
