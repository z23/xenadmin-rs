use super::*;
use xenadmin_xapi::records::{ClusterHostRecord, PciRecord, VlanRecord};

// The capacity observation deliberately has none of the destructive fabric-operation
// prerequisites: an enabled host with running guests can still be inspected.
pub(crate) async fn capacity(
    session: &Session,
    pool: crate::PoolId,
    id: crate::NetworkSriovId,
) -> Result<crate::SriovCapacity, Error> {
    let before = capacity_records(session).await?;
    let (reference, mut observation) = capacity_identity(&before, pool, id)?;
    let value = session
        .send(calls::network_sriov::GetRemainingCapacity(
            refs::network_sriov(&reference),
        ))
        .await?;
    let remaining = value
        .as_i64()
        .or_else(|| value.as_str()?.parse::<i64>().ok())
        .and_then(|n| u64::try_from(n).ok())
        .ok_or(Error::Protocol("Invalid remaining VF capacity"))?;
    let after = capacity_records(session).await?;
    let (current_reference, current) = capacity_identity(&after, pool, id)?;
    if reference != current_reference
        || observation != current
        || before
            .inventory
            .details
            .network_fabric
            .pci_devices
            .get(&observation.pci)
            != after
                .inventory
                .details
                .network_fabric
                .pci_devices
                .get(&observation.pci)
        || [observation.physical, observation.logical]
            .iter()
            .any(|id| {
                before
                    .inventory
                    .details
                    .pifs
                    .get(id)
                    .and_then(crate::PifIdentity::from_inventory)
                    != after
                        .inventory
                        .details
                        .pifs
                        .get(id)
                        .and_then(crate::PifIdentity::from_inventory)
            })
    {
        return Err(Error::ResourceConflict);
    }
    observation.remaining_vfs = remaining;
    observation.observed_at_ms = super::super::topology::now();
    Ok(observation)
}

async fn capacity_records(session: &Session) -> Result<Fresh, Error> {
    Fresh::read_classes(
        session,
        &["pool", "host", "network", "PIF", "network_sriov", "PCI"],
    )
    .await
}

fn capacity_identity(
    f: &Fresh,
    pool: crate::PoolId,
    id: crate::NetworkSriovId,
) -> Result<(String, crate::SriovCapacity), Error> {
    if f.inventory.pool_id != pool {
        return Err(Error::ResourceConflict);
    }
    let fabric = &f.inventory.details.network_fabric;
    let o = fabric.sriovs.get(&id).ok_or(Error::ResourceConflict)?;
    let physical = o.physical.ok_or(Error::ResourceConflict)?;
    let logical = o.logical.ok_or(Error::ResourceConflict)?;
    if physical == logical {
        return Err(Error::ResourceConflict);
    }
    let parent = f
        .inventory
        .details
        .pifs
        .get(&physical)
        .ok_or(Error::ResourceConflict)?;
    let child = f
        .inventory
        .details
        .pifs
        .get(&logical)
        .ok_or(Error::ResourceConflict)?;
    let host = parent.host.ok_or(Error::ResourceConflict)?;
    let network = child.network.ok_or(Error::ResourceConflict)?;
    let pci = *fabric
        .pif_pci
        .get(&physical)
        .ok_or(Error::ResourceConflict)?;
    let hardware = fabric
        .pci_devices
        .get(&pci)
        .ok_or(Error::ResourceConflict)?;
    let (reference, raw) = f.by_id("network_sriov", id.uuid())?;
    let (pr, p) = f.by_id("PIF", physical.uuid())?;
    let (cr, c) = f.by_id("PIF", logical.uuid())?;
    let host_ref = f.by_id("host", host.uuid())?.0;
    let (pci_ref, pci_raw) = f.by_id("PCI", pci.uuid())?;
    if child.host != Some(host)
        || hardware.host != Some(host)
        || parent.physical != Some(true)
        || child.physical != Some(false)
        || NetworkSriovRecord::new(raw).physical_pif() != pr
        || NetworkSriovRecord::new(raw).logical_pif() != cr
        || PifRecord::new(p).host() != host_ref
        || PifRecord::new(c).host() != host_ref
        || PciRecord::new(pci_raw).host() != host_ref
        || PifRecord::new(p).pci() != pci_ref
        || PifRecord::new(c).network() != f.by_id("network", network.uuid())?.0
        || network::refs(PifRecord::new(p).sriov_physical_pif_of())?
            != [reference.to_string()].into()
        || network::refs(PifRecord::new(c).sriov_logical_pif_of())?
            != [reference.to_string()].into()
        || f.raw["network_sriov"].iter().any(|(r, v)| {
            r != reference
                && (NetworkSriovRecord::new(v).physical_pif() == pr
                    || NetworkSriovRecord::new(v).logical_pif() == cr)
        })
    {
        return Err(Error::ResourceConflict);
    }
    Ok((
        reference.into(),
        crate::SriovCapacity {
            pool,
            sriov: id,
            host,
            network,
            physical,
            logical,
            pci,
            remaining_vfs: 0,
            observed_at_ms: 0,
            requires_reboot: o.requires_reboot,
            mode: o.mode,
            attached: child.currently_attached,
        },
    ))
}

pub(crate) async fn vlan_uplink(
    session: &Session,
    review: &crate::SriovVlanUplink,
) -> Result<(String, Value), Error> {
    let f = Fresh::read(session, Kind::Sriov, false).await?;
    let logical = review.object.logical.ok_or(Error::InvalidResourceCommand)?;
    let current = crate::SriovVlanUplink::capture(&f.inventory, logical, review.target)?;
    if current.as_ref() != Some(review) {
        return Err(Error::ResourceConflict);
    }
    capacity_identity(&f, review.fabric.pool, review.object.id)?;
    let (host_ref, host) = f.by_id("host", review.fabric.host.uuid())?;
    if HostRecord::new(host).enabled() != false
        || HostRecord::new(host)
            .current_operations()
            .entries()
            .is_none_or(|m| !m.is_empty())
        || f.raw["pool"]
            .values()
            .any(|p| PoolRecord::new(p).ha_enabled() != false)
        || f.raw["vm"].values().any(|v| {
            VmRecord::new(v).resident_on() == host_ref
                && VmRecord::new(v).is_control_domain() != true
        })
        || HostRecord::new(host)
            .metrics()
            .reference()
            .and_then(|r| f.raw["host_metrics"].get(r))
            .is_none_or(|m| HostMetricsRecord::new(m).live() != true)
    {
        return Err(Error::Protocol(
            "SR-IOV VLAN changes require disabled evacuated live hosts and disabled pool HA",
        ));
    }
    let clusters = match network::all(session, "Cluster_host").await {
        Ok(rows) => rows,
        Err(Error::Api(code)) if code == "MESSAGE_METHOD_UNKNOWN" || code == "NOT_IMPLEMENTED" => {
            Default::default()
        }
        Err(e) => return Err(e),
    };
    let affected = review
        .fabric
        .driver_interfaces
        .keys()
        .copied()
        .chain([logical])
        .chain(
            f.inventory
                .details
                .vlans
                .values()
                .filter(|v| v.tagged_pif == Some(logical))
                .filter_map(|v| v.untagged_pif),
        )
        .map(|id| f.by_id("PIF", id.uuid()).map(|(r, _)| r))
        .collect::<Result<BTreeSet<_>, _>>()?;
    if clusters.values().any(|c| {
        ClusterHostRecord::new(c).host() == host_ref
            || ClusterHostRecord::new(c)
                .pif()
                .reference()
                .is_some_and(|r| affected.contains(r))
    }) {
        return Err(Error::Protocol(
            "SR-IOV VLAN changes with host clustering require a separate driver impact review",
        ));
    }
    let mut networks = BTreeSet::from([review.target, review.fabric.network]);
    for p in review.fabric.driver_interfaces.values() {
        let (pr, raw) = f.by_id("PIF", p.id.uuid())?;
        let view = PifRecord::new(raw);
        if view.bond_slave_of() != "OpaqueRef:NULL"
            || view.vlan_master_of() != "OpaqueRef:NULL"
            || network::refs(view.bond_master_of()).map_or(true, |v| !v.is_empty())
            || network::refs(view.vlan_slave_of()).map_or(true, |v| !v.is_empty())
            || network::refs(view.tunnel_transport_pif_of()).map_or(true, |v| !v.is_empty())
            || network::refs(view.tunnel_access_pif_of()).map_or(true, |v| !v.is_empty())
            || network::refs(view.sriov_logical_pif_of()).map_or(true, |v| !v.is_empty())
            || f.raw["network_sriov"].values().any(|s| {
                NetworkSriovRecord::new(s).physical_pif() == pr
                    && network::uuid(s).ok() != Some(review.object.id.uuid())
            })
            || !host_default_configuration(raw)
        {
            return Err(Error::Protocol(
                "A same-driver physical interface has unreviewed configuration or topology",
            ));
        }
        let expected = f.raw["network_sriov"]
            .iter()
            .filter(|(_, s)| NetworkSriovRecord::new(s).physical_pif() == pr)
            .map(|(r, _)| r.clone())
            .collect();
        if network::refs(view.sriov_physical_pif_of())? != expected {
            return Err(Error::ResourceConflict);
        }
        networks.insert(p.network.ok_or(Error::ResourceConflict)?);
    }
    let (lr, l) = f.by_id("PIF", logical.uuid())?;
    let logical_view = PifRecord::new(l);
    if logical_view.bond_slave_of() != "OpaqueRef:NULL"
        || logical_view.vlan_master_of() != "OpaqueRef:NULL"
        || network::refs(logical_view.bond_master_of()).map_or(true, |v| !v.is_empty())
        || network::refs(logical_view.tunnel_transport_pif_of()).map_or(true, |v| !v.is_empty())
        || network::refs(logical_view.tunnel_access_pif_of()).map_or(true, |v| !v.is_empty())
        || network::refs(logical_view.sriov_physical_pif_of()).map_or(true, |v| !v.is_empty())
        || logical_view
            .other_config()
            .entries()
            .is_none_or(|m| !m.is_empty())
        || logical_view
            .properties()
            .entries()
            .is_none_or(|m| !m.is_empty())
    {
        return Err(Error::ResourceInUse);
    }
    let children = f.raw["vlan"]
        .iter()
        .filter(|(_, v)| VlanRecord::new(v).tagged_pif() == lr)
        .collect::<Vec<_>>();
    if network::refs(logical_view.vlan_slave_of())?
        != children.iter().map(|(r, _)| (*r).clone()).collect()
    {
        return Err(Error::ResourceConflict);
    }
    for (vr, v) in children {
        let vlan_view = VlanRecord::new(v);
        let cr = vlan_view
            .untagged_pif()
            .reference()
            .ok_or(Error::ResourceConflict)?;
        let c = f.raw["pif"].get(cr).ok_or(Error::ResourceConflict)?;
        let child_view = PifRecord::new(c);
        if child_view.vlan_master_of() != *vr
            || child_view.host() != host_ref
            || network::number(child_view.vlan()) != network::number(vlan_view.tag())
        {
            return Err(Error::ResourceConflict);
        }
        if review
            .preserved
            .contains_key(&crate::VlanId::from(network::uuid(v)?))
        {
            if child_view
                .other_config()
                .entries()
                .is_none_or(|m| !m.is_empty())
                || child_view
                    .properties()
                    .entries()
                    .is_none_or(|m| !m.is_empty())
                || vlan_view
                    .other_config()
                    .entries()
                    .is_none_or(|m| !m.is_empty())
            {
                return Err(Error::ResourceInUse);
            }
            networks.insert(
                f.inventory.details.pifs[&crate::PifId::from(network::uuid(c)?)]
                    .network
                    .ok_or(Error::ResourceConflict)?,
            );
        }
    }
    for id in networks {
        let (nr, n) = f.by_id("network", id.uuid())?;
        let net_view = NetworkRecord::new(n);
        let pifs = f.raw["pif"]
            .values()
            .filter(|p| PifRecord::new(p).network() == nr)
            .map(network::uuid)
            .collect::<Result<BTreeSet<_>, _>>()?;
        if f.ref_ids("PIF", net_view.pifs())? != pifs
            || !network::refs(net_view.vifs())?.is_empty()
            || f.raw["vif"]
                .values()
                .any(|v| VifRecord::new(v).network() == nr)
            || net_view.managed() != true
            || net_view.purpose().members().is_none_or(|p| !p.is_empty())
        {
            return Err(Error::ResourceInUse);
        }
    }
    Ok((lr.into(), l.clone()))
}

pub(crate) async fn vlan_identity(
    session: &Session,
    review: &crate::SriovVlanUplink,
) -> Result<(String, Value), Error> {
    let f = capacity_records(session).await?;
    let (_, o) = capacity_identity(&f, review.fabric.pool, review.object.id)?;
    if Some(o.logical) != review.object.logical
        || Some(o.physical) != review.object.physical
        || o.host != review.fabric.host
        || o.network != review.fabric.network
        || review
            .fabric
            .hardware
            .as_ref()
            .is_none_or(|p| p.id != o.pci)
    {
        return Err(Error::ResourceConflict);
    }
    let (reference, pif) = f.by_id("PIF", o.logical.uuid())?;
    Ok((reference.into(), pif.clone()))
}
