use super::*;
use crate::{ClassAvailability, ManagementTopology, PifId};
use std::collections::BTreeSet;

pub(crate) struct ManagementTopologyRecords {
    store: Store,
    details: crate::InventoryDetails,
}
fn require(ok: bool) -> Result<(), Error> {
    if ok {
        Ok(())
    } else {
        Err(Error::Protocol(
            "Management VLAN/bond topology is incomplete or inconsistent",
        ))
    }
}
fn reference<'a>(
    value: &'a Value,
    class: &str,
    store: &Store,
    nullable: bool,
) -> Result<Option<&'a str>, Error> {
    let r = value
        .as_str()
        .ok_or(Error::Protocol("Management topology reference missing"))?;
    if nullable && r == "OpaqueRef:NULL" {
        return Ok(None);
    }
    require(
        store
            .records
            .get(class)
            .is_some_and(|records| records.contains_key(r)),
    )?;
    Ok(Some(r))
}
fn references<'a>(
    value: &'a Value,
    class: &str,
    store: &Store,
) -> Result<BTreeSet<&'a str>, Error> {
    let values = value
        .as_array()
        .filter(|v| v.len() <= 1024)
        .ok_or(Error::Protocol("Management topology references missing"))?;
    let refs = values
        .iter()
        .map(|v| reference(v, class, store, false).map(Option::unwrap))
        .collect::<Result<BTreeSet<_>, _>>()?;
    require(refs.len() == values.len())?;
    Ok(refs)
}
impl ManagementTopologyRecords {
    pub(crate) async fn read(s: &Session) -> Result<Self, Error> {
        let mut store = Store::default();
        for class in ["host", "network", "PIF", "Bond", "VLAN"] {
            let value = s.call(&format!("{class}.get_all_records"), vec![]).await?;
            let values = value
                .as_object()
                .filter(|v| v.len() <= 10_000)
                .ok_or(Error::Protocol(
                    "Management topology inventory exceeds limit or is unavailable",
                ))?;
            let mut ids = BTreeSet::new();
            for (r, v) in values {
                require(r.starts_with("OpaqueRef:") && r != "OpaqueRef:NULL" && r.len() <= 1024)?;
                let id = v["uuid"]
                    .as_str()
                    .and_then(|v| v.parse::<uuid::Uuid>().ok())
                    .ok_or(Error::Protocol("Management topology UUID unavailable"))?;
                require(!id.is_nil() && ids.insert(id))?;
            }
            store.records.insert(
                class.to_ascii_lowercase(),
                values.iter().map(|(k, v)| (k.clone(), v.clone())).collect(),
            );
            store
                .classes
                .insert(class.to_ascii_lowercase(), ClassAvailability::Available);
        }
        // Check both directions before the ordinary inventory decoder resolves references.
        // Its partial optional-class projection must not hide missing topology relationships.
        let mut masters_by_pif: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut bonds_by_member: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut vlan_by_child: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        let mut vlans_by_parent: BTreeMap<&str, BTreeSet<&str>> = BTreeMap::new();
        for (r, b) in &store.records["bond"] {
            let master = reference(&b["master"], "pif", &store, false)?.unwrap();
            let slaves = references(&b["slaves"], "pif", &store)?;
            require(
                reference(&b["primary_slave"], "pif", &store, false)?
                    .is_some_and(|p| slaves.contains(p)),
            )?;
            masters_by_pif.entry(master).or_default().insert(r);
            for slave in slaves {
                bonds_by_member.entry(slave).or_default().insert(r);
            }
        }
        for (r, v) in &store.records["vlan"] {
            let parent = reference(&v["tagged_PIF"], "pif", &store, false)?.unwrap();
            let child = reference(&v["untagged_PIF"], "pif", &store, false)?.unwrap();
            vlans_by_parent.entry(parent).or_default().insert(r);
            vlan_by_child.entry(child).or_default().insert(r);
        }
        let empty = BTreeSet::new();
        for (r, p) in &store.records["pif"] {
            reference(&p["host"], "host", &store, false)?;
            reference(&p["network"], "network", &store, false)?;
            let masters = references(&p["bond_master_of"], "bond", &store)?;
            let member = reference(&p["bond_slave_of"], "bond", &store, true)?;
            let vlan = reference(&p["VLAN_master_of"], "vlan", &store, true)?;
            let vlans = references(&p["VLAN_slave_of"], "vlan", &store)?;
            require(
                &masters == masters_by_pif.get(r.as_str()).unwrap_or(&empty)
                    && &member.into_iter().collect::<BTreeSet<_>>()
                        == bonds_by_member.get(r.as_str()).unwrap_or(&empty)
                    && &vlan.into_iter().collect::<BTreeSet<_>>()
                        == vlan_by_child.get(r.as_str()).unwrap_or(&empty)
                    && &vlans == vlans_by_parent.get(r.as_str()).unwrap_or(&empty),
            )?;
            for field in [
                "tunnel_access_PIF_of",
                "tunnel_transport_PIF_of",
                "sriov_physical_PIF_of",
                "sriov_logical_PIF_of",
            ] {
                require(p[field].as_array().is_some_and(|v| v.len() <= 1024))?;
            }
            let config = p["other_config"]
                .as_object()
                .filter(|v| v.len() <= 1024)
                .ok_or(Error::Protocol("Management topology metadata unavailable"))?;
            require(config.get("management_purpose").is_none_or(|v| {
                v.as_str()
                    .is_some_and(|v| v.len() <= 256 && !v.chars().any(char::is_control))
            }))?;
        }
        let details = inventory::decode(&store);
        Ok(Self { store, details })
    }
    pub(crate) fn topology(&self, root: PifId) -> Result<ManagementTopology, Error> {
        let t = ManagementTopology::capture(&self.details, root).ok_or(Error::Protocol(
            "Management topology could not be captured completely",
        ))?;
        for id in t.pifs.keys() {
            let (_, p) = self.pif(*id)?;
            for field in [
                "tunnel_access_PIF_of",
                "tunnel_transport_PIF_of",
                "sriov_physical_PIF_of",
                "sriov_logical_PIF_of",
            ] {
                require(p[field].as_array().is_some_and(|v| v.is_empty()))?;
            }
            let network = self.store.records["network"]
                .get(p["network"].as_str().unwrap())
                .ok_or(Error::ResourceConflict)?;
            require(
                network["managed"] == true
                    && network["MTU"]
                        .as_u64()
                        .or_else(|| network["MTU"].as_str()?.parse().ok())
                        == Some(1500),
            )?;
        }
        Ok(t)
    }
    pub(crate) fn pif(&self, id: PifId) -> Result<(&str, &Value), Error> {
        self.pifs()
            .iter()
            .find(|(_, v)| {
                v["uuid"]
                    .as_str()
                    .and_then(|v| v.parse::<uuid::Uuid>().ok())
                    == Some(id.uuid())
            })
            .map(|(r, v)| (r.as_str(), v))
            .ok_or(Error::ResourceConflict)
    }
    pub(crate) fn records(&self, class: &str) -> &BTreeMap<String, Value> {
        &self.store.records[class]
    }
    pub(crate) fn pifs(&self) -> &BTreeMap<String, Value> {
        &self.store.records["pif"]
    }
}
