use crate::{
    Error, HostId, Inventory, NetworkFabricChange as Change, NetworkFabricKind as Kind,
    NetworkFabricOperation as Operation, NetworkFabricReview as Review, NetworkId, PifId,
    ResourceTarget,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// A captured, ordered change to all missing or all existing host members of one network.
/// Existing members remain bound even when only missing coverage needs creation.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct NetworkFabricPool {
    pub operations: Vec<Operation>,
}
impl NetworkFabricPool {
    pub fn review_input(
        i: &Inventory,
        network: NetworkId,
        kind: Kind,
        input: PifId,
    ) -> Result<Operation, Error> {
        let op = Operation {
            review: Review::capture(i, network, input, kind, true)?,
            change: Change::Create,
        };
        op.validate(ResourceTarget::Network(network))?;
        if op
            .review
            .members
            .values()
            .any(|m| m.interface.host == Some(op.review.host))
        {
            return Err(Error::ResourceConflict);
        }
        Ok(op)
    }
    pub fn create(
        i: &Inventory,
        network: NetworkId,
        kind: Kind,
        inputs: Vec<PifId>,
    ) -> Result<Self, Error> {
        let mut operations = inputs
            .into_iter()
            .map(|input| {
                Ok(Operation {
                    review: Review::capture(i, network, input, kind, true)?,
                    change: Change::Create,
                })
            })
            .collect::<Result<Vec<_>, Error>>()?;
        operations.sort_by_key(|op| (op.review.host != op.review.coordinator, op.review.host));
        let result = Self { operations };
        result.validate(ResourceTarget::Network(network))?;
        result.availability(i)?;
        Ok(result)
    }
    pub fn remove(i: &Inventory, network: NetworkId, kind: Kind) -> Result<Self, Error> {
        let mut operations = Vec::new();
        for p in i
            .details
            .pifs
            .values()
            .filter(|p| p.network == Some(network))
        {
            let (input, object) = match kind {
                Kind::Tunnel(protocol) => {
                    let t = i
                        .details
                        .network_fabric
                        .tunnels
                        .values()
                        .find(|t| t.access == Some(p.id) && t.protocol == Some(protocol))
                        .ok_or(Error::ResourceConflict)?;
                    (
                        t.transport.ok_or(Error::ResourceConflict)?,
                        crate::NetworkFabricObject::Tunnel(t.id),
                    )
                }
                Kind::Sriov => {
                    let t = i
                        .details
                        .network_fabric
                        .sriovs
                        .values()
                        .find(|t| t.logical == Some(p.id))
                        .ok_or(Error::ResourceConflict)?;
                    (
                        t.physical.ok_or(Error::ResourceConflict)?,
                        crate::NetworkFabricObject::Sriov(t.id),
                    )
                }
            };
            operations.push(Operation::capture(
                i,
                network,
                input,
                kind,
                Change::Delete(object),
            )?);
        }
        operations.sort_by_key(|op| (op.review.host == op.review.coordinator, op.review.host));
        let result = Self { operations };
        result.validate(ResourceTarget::Network(network))?;
        result.availability(i)?;
        Ok(result)
    }
    pub fn availability(&self, i: &Inventory) -> Result<(), Error> {
        let first = self
            .operations
            .first()
            .ok_or(Error::InvalidResourceCommand)?;
        self.validate(ResourceTarget::Network(first.review.network))?;
        for op in &self.operations {
            if op.review
                != Review::capture(
                    i,
                    op.review.network,
                    op.review.input.id,
                    op.review.kind,
                    op.creates(),
                )?
            {
                return Err(Error::ResourceConflict);
            }
        }
        Ok(())
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let first = self
            .operations
            .first()
            .ok_or(Error::InvalidResourceCommand)?;
        let r = &first.review;
        let hosts = self
            .operations
            .iter()
            .map(|op| op.review.host)
            .collect::<BTreeSet<HostId>>();
        let covered = r
            .members
            .values()
            .map(|m| m.interface.host.ok_or(Error::InvalidResourceCommand))
            .collect::<Result<BTreeSet<_>, _>>()?;
        if self.operations.len() > 128
            || r.hosts.len() > 128
            || hosts.len() != self.operations.len()
            || covered.len() != r.members.len()
            || !covered.is_subset(&r.hosts)
            || first.creates()
                && (!hosts.is_disjoint(&covered)
                    || hosts.union(&covered).copied().collect::<BTreeSet<_>>() != r.hosts)
            || !first.creates() && hosts != covered
        {
            return Err(Error::InvalidResourceCommand);
        }
        let mut previous = None;
        for op in &self.operations {
            op.validate(target)?;
            let v = &op.review;
            let order = (
                if first.creates() {
                    v.host != v.coordinator
                } else {
                    v.host == v.coordinator
                },
                v.host,
            );
            if previous.is_some_and(|p| p >= order)
                || op.creates() != first.creates()
                || v.pool != r.pool
                || v.network != r.network
                || v.kind != r.kind
                || v.hosts != r.hosts
                || v.coordinator != r.coordinator
                || v.members != r.members
                || v.controller != r.controller
                || v.mtu != r.mtu
                || v.locking_mode != r.locking_mode
                || v.kind == Kind::Sriov
                    && v.hardware
                        .as_ref()
                        .zip(r.hardware.as_ref())
                        .is_none_or(|(a, b)| {
                            a.vendor_id != b.vendor_id || a.device_id != b.device_id
                        })
            {
                return Err(Error::InvalidResourceCommand);
            }
            if let Change::Delete(object) = op.change
                && !v.members.values().any(|m| {
                    m.object == object && m.input == v.input.id && m.interface.host == Some(v.host)
                })
            {
                return Err(Error::InvalidResourceCommand);
            }
            previous = Some(order);
        }
        Ok(())
    }
    pub fn label(&self) -> &'static str {
        match self
            .operations
            .first()
            .map(|op| (op.review.kind, op.creates()))
        {
            Some((Kind::Tunnel(_), true)) => "Complete pool tunnel coverage",
            Some((Kind::Tunnel(_), false)) => "Remove pool tunnel interfaces",
            Some((Kind::Sriov, true)) => "Complete pool SR-IOV coverage",
            Some((Kind::Sriov, false)) => "Remove pool SR-IOV interfaces",
            None => "Pool network coverage",
        }
    }
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes(
            self.operations
                .iter()
                .map(|op| op.method().trim_start_matches("Async.")),
        )
    }
}
