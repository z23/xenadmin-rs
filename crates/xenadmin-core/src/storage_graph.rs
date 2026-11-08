use crate::{
    ActionAvailability, ClassAvailability, Error, HostId, Inventory, PbdId, ResourceAction,
    ResourceOperationRecord, ResourceTarget, SecretId, SrId, StorageLocation, TaskId,
};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct ReviewedPbd {
    pub pbd: PbdId,
    pub host: HostId,
    pub sr: SrId,
    pub attached: bool,
    pub location: Option<StorageLocation>,
    pub requires_credentials: bool,
}
impl ReviewedPbd {
    pub fn from_inventory(i: &Inventory, pbd: PbdId) -> Option<Self> {
        let p = i.details.pbds.get(&pbd)?;
        Some(Self {
            pbd,
            host: p.host?,
            sr: p.sr?,
            attached: p.currently_attached?,
            location: p.location.clone(),
            requires_credentials: p.requires_credentials,
        })
    }
    fn valid(&self) -> bool {
        !self.pbd.uuid().is_nil()
            && !self.host.uuid().is_nil()
            && !self.sr.uuid().is_nil()
            && self.location.as_ref().is_none_or(StorageLocation::validate)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageRepairHost {
    pub host: HostId,
    pub attachment: Option<ReviewedPbd>,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum StorageGraph {
    Repair {
        coordinator: HostId,
        source: ReviewedPbd,
        hosts: Vec<StorageRepairHost>,
    },
    Multipath {
        host: HostId,
        expected: bool,
        expected_handler: bool,
        enabled: bool,
        attachments: Vec<ReviewedPbd>,
    },
}
impl StorageGraph {
    pub fn label(&self) -> &'static str {
        match self {
            Self::Repair { .. } => "Repair shared storage connections",
            Self::Multipath { .. } => "Change multipathing and reconnect storage",
        }
    }
    pub fn permission_requirements(&self) -> crate::PermissionRequirements {
        crate::PermissionRequirements::writes(match self {
            Self::Repair { hosts, .. } => {
                let mut m = vec!["PBD.plug"];
                if hosts.iter().any(|h| h.attachment.is_none()) {
                    m.push("PBD.create");
                }
                m
            }
            Self::Multipath {
                expected_handler,
                enabled,
                attachments,
                ..
            } => {
                let mut m = vec!["host.set_multipathing"];
                if *expected_handler {
                    m.push("host.remove_from_other_config");
                }
                if *enabled {
                    m.push("host.add_to_other_config");
                }
                if attachments.iter().any(|p| p.attached) {
                    m.extend(["PBD.unplug", "PBD.plug"]);
                }
                m
            }
        })
    }
    pub(crate) fn validate(&self, target: ResourceTarget) -> Result<(), Error> {
        let valid = match self {
            Self::Repair {
                coordinator,
                source,
                hosts,
            } => {
                target == ResourceTarget::Storage(source.sr)
                    && source.valid()
                    && !coordinator.uuid().is_nil()
                    && source
                        .location
                        .as_ref()
                        .is_some_and(StorageLocation::shared)
                    && (1..=32).contains(&hosts.len())
                    && hosts.iter().map(|h| h.host).collect::<BTreeSet<_>>().len() == hosts.len()
                    && hosts.iter().all(|h| {
                        !h.host.uuid().is_nil()
                            && h.attachment.as_ref().is_none_or(|p| {
                                p.valid() && p.host == h.host && p.sr == source.sr && !p.attached
                            })
                    })
                    && hosts
                        .iter()
                        .all(|h| h.host != source.host || h.attachment.as_ref() == Some(source))
            }
            Self::Multipath {
                host,
                expected,
                enabled,
                attachments,
                ..
            } => {
                target == ResourceTarget::Host(*host)
                    && !host.uuid().is_nil()
                    && expected != enabled
                    && attachments.len() <= 64
                    && attachments.iter().all(|p| p.valid() && p.host == *host)
                    && attachments
                        .iter()
                        .map(|p| p.pbd)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == attachments.len()
                    && attachments
                        .iter()
                        .map(|p| p.sr)
                        .collect::<BTreeSet<_>>()
                        .len()
                        == attachments.len()
            }
        };
        if valid {
            Ok(())
        } else {
            Err(Error::InvalidResourceCommand)
        }
    }
    pub fn availability(&self, i: &Inventory, target: ResourceTarget) -> ActionAvailability {
        if self.validate(target).is_err() {
            return ActionAvailability::Unavailable;
        }
        if !matches!(
            i.details.classes.get("pbd"),
            Some(ClassAvailability::Available)
        ) {
            return ActionAvailability::Unknown;
        }
        let same = |p: &ReviewedPbd| ReviewedPbd::from_inventory(i, p.pbd).as_ref() == Some(p);
        let current = match self {
            Self::Repair {
                coordinator,
                source,
                hosts,
            } => {
                i.coordinator == Some(*coordinator)
                    && same(source)
                    && i.details
                        .storage
                        .get(&source.sr)
                        .is_some_and(|s| s.shared == Some(true))
                    && hosts.iter().all(|h| {
                        i.hosts.contains_key(&h.host)
                            && match &h.attachment {
                                Some(p) => same(p),
                                None => !i
                                    .details
                                    .pbds
                                    .values()
                                    .any(|p| p.host == Some(h.host) && p.sr == Some(source.sr)),
                            }
                    })
            }
            Self::Multipath {
                host,
                expected,
                expected_handler,
                attachments,
                ..
            } => {
                i.details.hosts.get(host).is_some_and(|h| {
                    h.enabled == Some(false)
                        && h.multipathing == Some(*expected)
                        && h.standard_multipath_handler == Some(*expected_handler)
                        && h.maintenance_mode == Some(true)
                }) && i.details.pool.as_ref().and_then(|p| p.ha_enabled) == Some(false)
                    && !i
                        .vms
                        .values()
                        .any(|v| !v.is_control_domain && v.host == Some(*host))
                    && attachments.iter().all(same)
                    && i.details
                        .pbds
                        .values()
                        .filter(|p| p.host == Some(*host))
                        .count()
                        == attachments.len()
            }
        };
        if current {
            ActionAvailability::Available
        } else {
            ActionAvailability::Stale
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct StorageGraphResult {
    pub host: HostId,
    pub sr: SrId,
    pub pbd: PbdId,
    pub created: bool,
    pub attached: bool,
    #[serde(default)]
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub(crate) enum Kind {
    Create(HostId),
    Plug(HostId, SrId),
    Unplug(PbdId),
    Mode,
    RemoveHandler,
    AddHandler,
}
impl Kind {
    pub(crate) fn label(&self) -> String {
        match self {
            Self::Create(h) => format!("Create missing storage connection on {h}"),
            Self::Plug(h, s) => format!("Connect storage {s} on {h}"),
            Self::Unplug(p) => format!("Disconnect reviewed attachment {p}"),
            Self::Mode => "Set reviewed host multipathing mode".into(),
            Self::RemoveHandler => "Remove the standard multipath handler".into(),
            Self::AddHandler => "Set the standard dmp handler".into(),
        }
    }
    pub(crate) fn asynchronous(&self) -> bool {
        matches!(self, Self::Create(_) | Self::Plug(..) | Self::Unplug(_))
    }
}
#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Evidence {
    pub task: Option<String>,
    pub task_uuid: Option<TaskId>,
    pub result: Option<String>,
    pub created: Option<PbdId>,
    pub secrets: Vec<SecretId>,
    pub completed: bool,
    pub ended: bool,
    pub verified: bool,
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub pool: crate::PoolId,
    pub target: ResourceTarget,
    pub operation: StorageGraph,
    pub kinds: Vec<Kind>,
    pub evidence: Vec<Evidence>,
    pub configuration: BTreeMap<PbdId, String>,
    pub tasks_before: BTreeSet<TaskId>,
    pub pbds_before: BTreeSet<PbdId>,
    pub secrets_before: BTreeSet<SecretId>,
    pub expires_at_ms: u64,
}
impl Plan {
    pub(crate) fn kinds(op: &StorageGraph) -> Vec<Kind> {
        match op {
            StorageGraph::Repair {
                coordinator,
                source,
                hosts,
            } => {
                let mut hosts = hosts.clone();
                hosts.sort_by_key(|h| (h.host != *coordinator, h.host));
                hosts
                    .into_iter()
                    .flat_map(|h| {
                        let mut k = vec![];
                        if h.attachment.is_none() {
                            k.push(Kind::Create(h.host));
                        }
                        k.push(Kind::Plug(h.host, source.sr));
                        k
                    })
                    .collect()
            }
            StorageGraph::Multipath {
                expected_handler,
                enabled,
                attachments,
                ..
            } => {
                let mut a = attachments
                    .iter()
                    .filter(|p| p.attached)
                    .collect::<Vec<_>>();
                a.sort_by_key(|p| p.sr);
                let mut k = a.iter().map(|p| Kind::Unplug(p.pbd)).collect::<Vec<_>>();
                k.push(Kind::Mode);
                if *expected_handler {
                    k.push(Kind::RemoveHandler);
                }
                if *enabled {
                    k.push(Kind::AddHandler);
                }
                k.extend(a.iter().map(|p| Kind::Plug(p.host, p.sr)));
                k
            }
        }
    }
    pub(crate) fn extends(&self, old: &Self) -> bool {
        self.pool == old.pool
            && self.target == old.target
            && self.operation == old.operation
            && self.kinds == old.kinds
            && self.configuration == old.configuration
            && self.tasks_before == old.tasks_before
            && self.pbds_before == old.pbds_before
            && self.secrets_before == old.secrets_before
            && self.expires_at_ms == old.expires_at_ms
            && self.evidence.len() == old.evidence.len()
            && self.evidence.iter().zip(&old.evidence).all(|(n, o)| {
                (o.task.is_none() || n.task == o.task)
                    && (o.task_uuid.is_none() || n.task_uuid == o.task_uuid)
                    && (o.result.is_none() || n.result == o.result)
                    && (o.created.is_none() || n.created == o.created)
                    && (o.secrets.is_empty() || n.secrets == o.secrets)
                    && (!o.completed || n.completed)
                    && (!o.ended || n.ended)
                    && (!o.verified || n.verified)
            })
    }
    pub(crate) fn results(&self) -> Vec<StorageGraphResult> {
        let mut results = BTreeMap::new();
        for (k, e) in self.kinds.iter().zip(&self.evidence) {
            if !e.verified && !matches!(k, Kind::Create(_)) {
                continue;
            }
            match k {
                Kind::Create(host) => {
                    if let (StorageGraph::Repair { source, .. }, Some(pbd)) =
                        (&self.operation, e.created)
                    {
                        results.insert(
                            (*host, source.sr),
                            StorageGraphResult {
                                host: *host,
                                sr: source.sr,
                                pbd,
                                created: true,
                                attached: false,
                                verified: e.verified,
                            },
                        );
                    }
                }
                Kind::Plug(host, sr) => {
                    if let Some(pbd) = self.attachment(*host, *sr) {
                        let r = results.entry((*host, *sr)).or_insert(StorageGraphResult {
                            host: *host,
                            sr: *sr,
                            pbd,
                            created: false,
                            attached: false,
                            verified: true,
                        });
                        r.attached = true;
                        r.verified = true;
                    }
                }
                Kind::Unplug(pbd) => {
                    if let StorageGraph::Multipath { attachments, .. } = &self.operation
                        && let Some(p) = attachments.iter().find(|p| p.pbd == *pbd)
                    {
                        results.insert(
                            (p.host, p.sr),
                            StorageGraphResult {
                                host: p.host,
                                sr: p.sr,
                                pbd: *pbd,
                                created: false,
                                attached: false,
                                verified: true,
                            },
                        );
                    }
                }
                _ => {}
            }
        }
        results.into_values().collect()
    }
    pub(crate) fn attachment(&self, host: HostId, sr: SrId) -> Option<PbdId> {
        match &self.operation {
            StorageGraph::Repair { hosts, .. } => hosts
                .iter()
                .find(|h| h.host == host)?
                .attachment
                .as_ref()
                .map(|p| p.pbd)
                .or_else(|| {
                    self.kinds.iter().zip(&self.evidence).find_map(|(k, e)| {
                        (*k == Kind::Create(host)).then_some(e.created).flatten()
                    })
                }),
            StorageGraph::Multipath { attachments, .. } => attachments
                .iter()
                .find(|p| p.host == host && p.sr == sr)
                .map(|p| p.pbd),
        }
    }
}
pub(crate) fn validate_record(r: &ResourceOperationRecord) -> Result<(), Error> {
    use crate::{OperationState, StepState};
    let Some(p) = &r.storage_graph_plan else {
        return if matches!(r.action, ResourceAction::StorageGraph(_))
            && (!r.storage_graph.is_empty()
                || !r.steps.is_empty()
                || !matches!(
                    r.state,
                    OperationState::Preparing
                        | OperationState::Rejected
                        | OperationState::Cancelled
                        | OperationState::OutcomeUnknown
                        | OperationState::AcknowledgedUnknown
                ))
        {
            Err(Error::Journal)
        } else {
            Ok(())
        };
    };
    if r.action != ResourceAction::StorageGraph(p.operation.clone())
        || r.pool != p.pool
        || r.target != p.target
        || p.operation.validate(r.target).is_err()
        || p.kinds != Plan::kinds(&p.operation)
        || p.kinds.is_empty()
        || p.kinds.len() != p.evidence.len()
        || r.steps.len() != p.kinds.len()
        || p.tasks_before.len() > 100_000
        || p.pbds_before.len() > 10000
        || p.secrets_before.len() > 4096
        || p.tasks_before.iter().any(|id| id.uuid().is_nil())
        || p.pbds_before.iter().any(|id| id.uuid().is_nil())
        || p.secrets_before.iter().any(|id| id.uuid().is_nil())
        || r.task.is_some()
        || r.result_reference.is_some()
        || r.storage_graph != p.results()
    {
        return Err(Error::Journal);
    }
    let expected = match &p.operation {
        StorageGraph::Repair { source, hosts, .. } => std::iter::once(source.pbd)
            .chain(
                hosts
                    .iter()
                    .filter_map(|h| h.attachment.as_ref().map(|p| p.pbd)),
            )
            .collect::<BTreeSet<_>>(),
        StorageGraph::Multipath { attachments, .. } => attachments.iter().map(|p| p.pbd).collect(),
    };
    if p.configuration.keys().copied().collect::<BTreeSet<_>>() != expected
        || p.configuration
            .values()
            .any(|s| s.len() != 64 || !s.bytes().all(|b| b.is_ascii_hexdigit()))
    {
        return Err(Error::Journal);
    }
    let mut tasks = BTreeSet::new();
    let mut created = BTreeSet::new();
    let mut secrets = BTreeSet::new();
    for (index, ((kind, e), step)) in p.kinds.iter().zip(&p.evidence).zip(&r.steps).enumerate() {
        if step.label != kind.label()
            || (step.state == StepState::Verified && !e.verified)
            || (e.verified
                && (!e.completed || matches!(kind, Kind::Create(_)) && e.created.is_none()))
            || (e.completed && !e.ended)
            || (e.task_uuid.is_some() && e.task.is_none())
            || (kind.asynchronous() && e.ended && e.task_uuid.is_none())
            || (!kind.asynchronous() && e.task.is_some())
            || e.task_uuid.is_some_and(|id| {
                id.uuid().is_nil() || p.tasks_before.contains(&id) || !tasks.insert(id)
            })
            || e.task
                .as_ref()
                .is_some_and(|v| crate::snapshots::reference(&serde_json::json!(v)).is_err())
            || e.result
                .as_ref()
                .is_some_and(|v| crate::snapshots::reference(&serde_json::json!(v)).is_err())
            || ((e.result.is_some() || e.created.is_some() || !e.secrets.is_empty())
                && !matches!(kind, Kind::Create(_)))
            || (e.result.is_some() && !e.completed)
            || (e.created.is_some() && (e.result.is_none() || !e.completed))
            || (!e.secrets.is_empty() && e.created.is_none())
            || e.created.is_some_and(|id| {
                id.uuid().is_nil() || p.pbds_before.contains(&id) || !created.insert(id)
            })
            || e.secrets.iter().any(|id| {
                id.uuid().is_nil() || p.secrets_before.contains(id) || !secrets.insert(*id)
            })
            || (step.state == StepState::Pending && (e.task.is_some() || e.completed || e.ended))
            || (index > 0 && step.state != StepState::Pending && !p.evidence[index - 1].verified)
            || (r.state == OperationState::Succeeded
                && (!e.verified || step.state != StepState::Verified))
        {
            return Err(Error::Journal);
        }
    }
    Ok(())
}
