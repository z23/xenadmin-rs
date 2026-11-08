use crate::{Inventory, SnapshotId, VdiId, VmId};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum SnapshotParent {
    Root,
    Snapshot(SnapshotId),
    Vm(VmId),
    #[default]
    Unresolved,
}

#[derive(Clone, Debug, Default, Serialize, Deserialize)]
pub struct SnapshotDetails {
    #[serde(default)]
    pub can_export: Option<bool>,
    pub parent: SnapshotParent,
    pub memory_vdi: Option<VdiId>,
    pub memory_vdi_known: bool,
    pub preview_available: bool,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SnapshotAncestryRow {
    pub id: SnapshotId,
    pub depth: usize,
    pub current_parent: bool,
    pub issue: Option<&'static str>,
}

impl Inventory {
    pub fn snapshot_ancestry(&self, vm: VmId) -> Vec<SnapshotAncestryRow> {
        let mut snapshots: Vec<_> = self.snapshots.for_vm(vm).collect();
        snapshots.sort_by(|a, b| {
            a.created_at
                .cmp(&b.created_at)
                .then(a.name.cmp(&b.name))
                .then(a.id.cmp(&b.id))
        });
        let ids: BTreeSet<_> = snapshots.iter().map(|s| s.id).collect();
        let mut children: BTreeMap<SnapshotId, Vec<SnapshotId>> = BTreeMap::new();
        let mut roots = Vec::new();
        for snapshot in &snapshots {
            match self
                .details
                .snapshots
                .get(&snapshot.id)
                .map(|d| d.parent)
                .unwrap_or_default()
            {
                SnapshotParent::Root => roots.push((snapshot.id, None)),
                SnapshotParent::Snapshot(parent) if ids.contains(&parent) => {
                    children.entry(parent).or_default().push(snapshot.id);
                }
                SnapshotParent::Snapshot(_) => roots.push((
                    snapshot.id,
                    Some("Parent is outside this VM's snapshot inventory"),
                )),
                SnapshotParent::Vm(_) => {
                    roots.push((snapshot.id, Some("Parent is a VM, not a snapshot")))
                }
                SnapshotParent::Unresolved => {
                    roots.push((snapshot.id, Some("Parent is not reported or unavailable")))
                }
            }
        }
        let current = self
            .details
            .vms
            .get(&vm)
            .map(|d| d.snapshot_parent)
            .unwrap_or_default();
        let mut visited = BTreeSet::new();
        let mut rows = Vec::with_capacity(ids.len());
        // A malformed cycle has no root. Visit it once after the ordinary forest.
        roots.extend(
            snapshots
                .iter()
                .map(|s| (s.id, Some("Ancestry contains a cycle"))),
        );
        for (root, issue) in roots {
            let mut stack = vec![(root, 0, issue)];
            while let Some((id, depth, issue)) = stack.pop() {
                if !visited.insert(id) {
                    continue;
                }
                rows.push(SnapshotAncestryRow {
                    id,
                    depth,
                    current_parent: current == SnapshotParent::Snapshot(id),
                    issue,
                });
                if let Some(children) = children.get(&id) {
                    stack.extend(children.iter().rev().map(|id| {
                        (
                            *id,
                            depth + 1,
                            issue.filter(|s| *s == "Ancestry contains a cycle"),
                        )
                    }));
                }
            }
        }
        rows
    }
}
