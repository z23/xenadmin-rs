//! What the tree or a grid has selected. The front end stores the object ids.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum ObjectKind {
    None,
    Pool,
    Host,
    Vm,
    Template,
    StorageRepository,
    Network,
    VirtualDisk,
    Multi,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct Selection {
    pub kind: ObjectKind,
    /// A live host has the console and performance tabs. Ignored for other kinds.
    pub host_live: bool,
}

impl Selection {
    pub fn none() -> Self {
        Self {
            kind: ObjectKind::None,
            host_live: false,
        }
    }
}
