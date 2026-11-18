//! What the tree, a grid and a command talk about: an object kind, one
//! object, and the objects a command acts on.

use serde::Serialize;
use uuid::Uuid;
use xenadmin_core::{HostId, PoolId, SnapshotId, VmId};

/// The kind of a selected object. It picks the row of the tab matrix and the
/// context menu.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
#[serde(rename_all = "kebab-case")]
pub enum ObjectKind {
    /// Nothing, or the root node of the tree.
    None,
    Pool,
    Host,
    Vm,
    Template,
    StorageRepository,
    Network,
    VirtualDisk,
    Snapshot,
    /// A vApp.
    Appliance,
    Folder,
    Tag,
    SavedSearch,
    /// A saved profile that is not connected. A connected one is its `Pool`.
    Connection,
    /// Several objects.
    Multi,
}

impl ObjectKind {
    /// Every kind, in the order of the declaration.
    pub const ALL: [Self; 15] = [
        Self::None,
        Self::Pool,
        Self::Host,
        Self::Vm,
        Self::Template,
        Self::StorageRepository,
        Self::Network,
        Self::VirtualDisk,
        Self::Snapshot,
        Self::Appliance,
        Self::Folder,
        Self::Tag,
        Self::SavedSearch,
        Self::Connection,
        Self::Multi,
    ];

    /// The singular noun the interface uses for the kind.
    pub fn label(self) -> &'static str {
        match self {
            Self::None => "Nothing",
            Self::Pool => "Pool",
            Self::Host => "Server",
            Self::Vm => "VM",
            Self::Template => "Template",
            Self::StorageRepository => "Storage repository",
            Self::Network => "Network",
            Self::VirtualDisk => "Virtual disk",
            Self::Snapshot => "Snapshot",
            Self::Appliance => "vApp",
            Self::Folder => "Folder",
            Self::Tag => "Tag",
            Self::SavedSearch => "Saved search",
            Self::Connection => "Connection",
            Self::Multi => "Several objects",
        }
    }
}

/// One object, identified so that it survives inventory updates.
///
/// `pool` is the pool the object lives in. It is `None` for an object that
/// belongs to no single pool: a saved profile that is not connected, a
/// folder, a tag, a saved search. `id` is the object's UUID as text, or for
/// the kinds without one the profile id, the folder path, the tag or the
/// search name.
#[derive(Clone, Debug, PartialEq, Eq, Hash, PartialOrd, Ord, Serialize)]
pub struct ObjectRef {
    pub pool: Option<PoolId>,
    pub kind: ObjectKind,
    pub id: String,
}

impl ObjectRef {
    pub fn new(pool: Option<PoolId>, kind: ObjectKind, id: impl ToString) -> Self {
        Self {
            pool,
            kind,
            id: id.to_string(),
        }
    }

    pub fn pool(pool: PoolId) -> Self {
        Self::new(Some(pool), ObjectKind::Pool, pool)
    }

    pub fn host(pool: PoolId, host: HostId) -> Self {
        Self::new(Some(pool), ObjectKind::Host, host)
    }

    pub fn vm(pool: PoolId, vm: VmId) -> Self {
        Self::new(Some(pool), ObjectKind::Vm, vm)
    }

    pub fn template(pool: PoolId, vm: VmId) -> Self {
        Self::new(Some(pool), ObjectKind::Template, vm)
    }

    pub fn storage(pool: PoolId, uuid: Uuid) -> Self {
        Self::new(Some(pool), ObjectKind::StorageRepository, uuid)
    }

    pub fn network(pool: PoolId, uuid: Uuid) -> Self {
        Self::new(Some(pool), ObjectKind::Network, uuid)
    }

    pub fn disk(pool: PoolId, uuid: Uuid) -> Self {
        Self::new(Some(pool), ObjectKind::VirtualDisk, uuid)
    }

    pub fn snapshot(pool: PoolId, snapshot: SnapshotId) -> Self {
        Self::new(Some(pool), ObjectKind::Snapshot, snapshot)
    }

    /// The id as a UUID, when the kind has one.
    pub fn uuid(&self) -> Option<Uuid> {
        Uuid::parse_str(&self.id).ok()
    }

    /// The VM id when the object is a VM or a template.
    pub fn vm_id(&self) -> Option<VmId> {
        matches!(self.kind, ObjectKind::Vm | ObjectKind::Template)
            .then(|| self.uuid().map(VmId::from))
            .flatten()
    }

    pub fn host_id(&self) -> Option<HostId> {
        (self.kind == ObjectKind::Host)
            .then(|| self.uuid().map(HostId::from))
            .flatten()
    }

    pub fn snapshot_id(&self) -> Option<SnapshotId> {
        (self.kind == ObjectKind::Snapshot)
            .then(|| self.uuid().map(SnapshotId::from))
            .flatten()
    }
}

/// The objects a command acts on: the grid selection inside a tab when there
/// is one, otherwise the tree scope. Empty when nothing is selected.
#[derive(Clone, Debug, Default, PartialEq, Eq, Hash, Serialize)]
pub struct Target {
    pub objects: Vec<ObjectRef>,
}

impl Target {
    /// Nothing selected.
    pub fn none() -> Self {
        Self::default()
    }

    pub fn one(object: ObjectRef) -> Self {
        Self {
            objects: vec![object],
        }
    }

    pub fn many(objects: impl IntoIterator<Item = ObjectRef>) -> Self {
        Self {
            objects: objects.into_iter().collect(),
        }
    }

    pub fn is_empty(&self) -> bool {
        self.objects.is_empty()
    }

    pub fn len(&self) -> usize {
        self.objects.len()
    }

    pub fn is_multi(&self) -> bool {
        self.objects.len() > 1
    }

    /// `None` when empty, the object's kind when there is one object, `Multi`
    /// otherwise.
    pub fn kind(&self) -> ObjectKind {
        match self.objects.as_slice() {
            [] => ObjectKind::None,
            [object] => object.kind,
            _ => ObjectKind::Multi,
        }
    }

    /// The one object, when there is exactly one.
    pub fn single(&self) -> Option<&ObjectRef> {
        match self.objects.as_slice() {
            [object] => Some(object),
            _ => None,
        }
    }

    /// The one object when it has this kind.
    pub fn single_of(&self, kind: ObjectKind) -> Option<&ObjectRef> {
        self.single().filter(|object| object.kind == kind)
    }

    /// True when there is at least one object and every object has this kind.
    pub fn all_of(&self, kind: ObjectKind) -> bool {
        !self.objects.is_empty() && self.objects.iter().all(|object| object.kind == kind)
    }

    /// The kind every object shares, when they share one.
    pub fn common_kind(&self) -> Option<ObjectKind> {
        let first = self.objects.first()?.kind;
        self.all_of(first).then_some(first)
    }

    /// The pool every object lives in, when they share one.
    pub fn pool(&self) -> Option<PoolId> {
        let first = self.objects.first()?.pool?;
        self.objects
            .iter()
            .all(|object| object.pool == Some(first))
            .then_some(first)
    }

    /// The VM ids of the VMs and templates in the target, in order.
    pub fn vms(&self) -> Vec<VmId> {
        self.objects.iter().filter_map(ObjectRef::vm_id).collect()
    }

    pub fn hosts(&self) -> Vec<HostId> {
        self.objects.iter().filter_map(ObjectRef::host_id).collect()
    }

    pub fn contains(&self, object: &ObjectRef) -> bool {
        self.objects.contains(object)
    }
}

impl From<ObjectRef> for Target {
    fn from(object: ObjectRef) -> Self {
        Self::one(object)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn pool() -> PoolId {
        PoolId::parse("10000000-0000-0000-0000-000000000001").expect("pool id")
    }

    fn vm(n: u8) -> ObjectRef {
        ObjectRef::vm(
            pool(),
            VmId::parse(&format!("30000000-0000-0000-0000-0000000000{n:02}")).expect("vm id"),
        )
    }

    #[test]
    fn a_target_reports_its_kind_and_its_ids() {
        assert_eq!(Target::none().kind(), ObjectKind::None);
        assert!(Target::none().single().is_none());
        let one = Target::one(vm(1));
        assert_eq!(one.kind(), ObjectKind::Vm);
        assert_eq!(one.single(), Some(&vm(1)));
        assert_eq!(one.single_of(ObjectKind::Host), None);
        assert_eq!(one.vms().len(), 1);
        assert_eq!(one.pool(), Some(pool()));
        let two = Target::many([vm(1), vm(2)]);
        assert_eq!(two.kind(), ObjectKind::Multi);
        assert!(two.is_multi() && two.all_of(ObjectKind::Vm));
        assert_eq!(two.common_kind(), Some(ObjectKind::Vm));
        assert!(two.single().is_none());
        let mixed = Target::many([vm(1), ObjectRef::pool(pool())]);
        assert_eq!(mixed.common_kind(), None);
        assert_eq!(mixed.vms().len(), 1);
    }

    #[test]
    fn an_object_gives_typed_ids_only_for_its_own_kind() {
        let vm = vm(3);
        assert!(vm.vm_id().is_some());
        assert!(vm.host_id().is_none());
        let folder = ObjectRef::new(None, ObjectKind::Folder, "/Production");
        assert!(folder.uuid().is_none() && folder.vm_id().is_none());
        assert_eq!(ObjectKind::Host.label(), "Server");
    }
}
