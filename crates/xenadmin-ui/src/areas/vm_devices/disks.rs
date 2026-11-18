//! One virtual disk: grow it, copy it, or delete it when a local operation
//! record proves it is ours. Carried over from `disk_workspace.rs`.
//!
//! The Rune workspace was a pool-wide dialog: a searchable, paged list, then
//! a form with three modes. The list is the Storage tab of a repository now
//! and each mode is a command, so the panel names its disk and its mode from
//! the start; what it reviews and submits is unchanged.

use std::sync::Arc;

use xenadmin_core::{
    DiskOperation, Error, Inventory, ObservedResource, OperationId, OwnedDiskSource, PoolId,
    ResourceCommand, ResourceTarget, ReviewedDisk, SrId, VdiId,
};

use crate::prelude::*;

pub const STALE: &str =
    "The captured connection changed. Close and reopen this workspace before reviewing a change.";
pub const INCOMPLETE: &str = "Complete ordinary disk and attachment data is required";
pub const NO_OWNERSHIP: &str = "No terminal local creation record proves ownership of this disk. Resolve its originating operation first; arbitrary detached disks cannot be deleted here";
pub const BUSY: &str = "Too many storage operations are unresolved. Resolve some in the operation history first.";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Mode {
    Grow,
    Copy,
    Cleanup,
}

impl Mode {
    pub fn title(self) -> &'static str {
        match self {
            Mode::Grow => "Resize virtual disk",
            Mode::Copy => "Copy virtual disk",
            Mode::Cleanup => "Delete owned virtual disk",
        }
    }
}

#[derive(Clone)]
pub struct Review {
    pub target: Option<ObservedResource>,
    pub disk: VdiId,
    pub name: String,
    pub operation: DiskOperation,
}

pub struct Panel {
    pub pool: PoolId,
    /// The pool as observed when the panel opened.
    pub observed: Option<ObservedResource>,
    pub disk: VdiId,
    pub mode: Mode,
    pub size: String,
    pub name: String,
    pub destination: Option<SrId>,
    pub review: Option<Review>,
    pub confirmation: String,
    pub error: Option<String>,
}

pub fn close(m: &mut Model) {
    m.areas.vm_devices.disks = None;
}

/// "Operation history": the Events tab of the Notifications view (Rune
/// opened the pool history of the resource editor).
pub fn history(m: &mut Model) {
    let shown = m
        .areas
        .vm_devices
        .disks
        .as_ref()
        .is_some_and(|p| m.inventory_for(p.pool).is_some());
    if shown {
        close(m);
        m.show_operations();
    }
}

/// The inventory of the panel's pool, while that pool is still shown.
pub fn inventory(m: &Model, p: &Panel) -> Option<Arc<Inventory>> {
    m.inventory_for(p.pool)
}

pub fn current(m: &Model, p: &Panel) -> bool {
    inventory(m, p).is_some()
        && (m.is_synthetic_pool(p.pool)
            || p.observed.is_some_and(|o| {
                m.snapshot_for(p.pool)
                    .and_then(|v| v.observe_resource(ResourceTarget::Pool(p.pool)).ok())
                    == Some(o)
            }))
}

/// Opens the panel for a disk in one mode. The copy is named after the disk
/// and goes to `destination` unless the user picks another repository.
pub fn open(m: &mut Model, pool: PoolId, disk: VdiId, mode: Mode, destination: Option<SrId>) -> bool {
    let Some(i) = m.inventory_for(pool) else {
        return false;
    };
    let Some(record) = i.details.vdis.get(&disk) else {
        return false;
    };
    let panel = Panel {
        pool,
        observed: m
            .snapshot_for(pool)
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(pool)).ok()),
        disk,
        mode,
        size: String::new(),
        name: format!("{} copy", record.name),
        destination,
        review: None,
        confirmation: String::new(),
        error: None,
    };
    super::clear(m);
    m.areas.vm_devices.disks = Some(Box::new(panel));
    true
}

/// The local record that proves a disk is ours to delete.
pub fn source(m: &Model, pool: PoolId, disk: VdiId) -> Option<OwnedDiskSource> {
    m.resource_rows
        .iter()
        .find(|r| r.pool == pool && r.owned_disk_candidates().contains(&disk))
        .map(|r| OwnedDiskSource::ResourceOperation(r.id))
        .or_else(|| {
            m.operation_rows
                .iter()
                .find(|r| r.pool == pool && r.owned_disk_candidates().contains(&disk))
                .map(|r| OwnedDiskSource::VmOperation(r.id))
        })
}

/// What no draft can fix: the reason the command of a mode is disabled.
pub fn blocked(m: &Model, pool: PoolId, i: &Inventory, disk: VdiId, mode: Mode) -> Option<String> {
    if ReviewedDisk::from_inventory(i, disk).is_none() {
        return Some(INCOMPLETE.into());
    }
    (mode == Mode::Cleanup && source(m, pool, disk).is_none()).then(|| NO_OWNERSHIP.into())
}

pub fn review(m: &mut Model) {
    let result = (|| {
        let p = m
            .areas
            .vm_devices
            .disks
            .as_ref()
            .ok_or("Disk workspace is closed")?;
        if !current(m, p) {
            return Err("The captured connection changed; close and reopen this workspace".into());
        }
        let i = inventory(m, p).ok_or("Inventory is unavailable")?;
        let disk = p.disk;
        let record = i.details.vdis.get(&disk).ok_or("Choose a disk")?;
        let expected = ReviewedDisk::from_inventory(&i, disk).ok_or(INCOMPLETE)?;
        let operation = match p.mode {
            Mode::Grow => DiskOperation::Grow {
                expected,
                size: super::actions::parse_size(&p.size)?,
            },
            Mode::Copy => DiskOperation::Copy {
                expected,
                destination: p.destination.ok_or("Choose destination storage")?,
                name: p.name.trim().into(),
            },
            Mode::Cleanup => DiskOperation::Cleanup {
                expected,
                source: source(m, p.pool, disk).ok_or(NO_OWNERSHIP)?,
            },
        };
        let availability = operation.availability(&i, disk);
        if !availability.is_available() {
            return Err(availability.explanation());
        }
        Ok(Review {
            target: m
                .snapshot_for(p.pool)
                .and_then(|v| v.observe_resource(ResourceTarget::Disk(disk)).ok()),
            disk,
            name: record.name.clone(),
            operation,
        })
    })();
    if let Some(p) = &mut m.areas.vm_devices.disks {
        match result {
            Ok(r) => {
                p.review = Some(r);
                p.confirmation = String::new();
                p.error = None;
            }
            Err(e) => p.error = Some(e),
        }
    }
}

pub fn back(m: &mut Model) {
    if let Some(p) = &mut m.areas.vm_devices.disks {
        p.review = None;
    }
}

/// Whether the host still allows the reviewed operation; `None` when the
/// pool is no longer shown.
pub fn available(m: &Model, p: &Panel, r: &Review) -> Option<Result<(), String>> {
    let i = inventory(m, p)?;
    let a = r.operation.availability(&i, r.disk);
    Some(if a.is_available() {
        Ok(())
    } else {
        Err(a.explanation())
    })
}

/// Submits the reviewed operation. Every refusal returns without a word, as
/// in Rune: the Confirm button carries the reason.
pub fn apply(m: &mut Model) {
    let Some(p) = &m.areas.vm_devices.disks else {
        return;
    };
    let Some(r) = p.review.clone() else {
        return;
    };
    let pool = p.pool;
    if !current(m, p) || !m.can_mutate_pool(pool) || m.resource_operations.len() >= 128 {
        return;
    }
    if matches!(r.operation, DiskOperation::Cleanup { .. }) && p.confirmation != r.disk.to_string()
    {
        return;
    }
    let Some(target) = r.target else {
        return;
    };
    if m.snapshot_for(pool)
        .and_then(|v| v.observe_resource(ResourceTarget::Disk(r.disk)).ok())
        != Some(target)
        || !matches!(available(m, p, &r), Some(Ok(())))
        || !permissions::check_for(m, pool, &r.operation.permission_requirements())
    {
        return;
    }
    let result = m
        .connection_for(pool)
        .ok_or(Error::Closed)
        .and_then(|c| {
            c.submit_resource(
                OperationId::new(),
                ResourceCommand::Disk {
                    target,
                    operation: r.operation,
                },
            )
        });
    match result {
        Ok(op) => {
            m.track_resource(op);
            if let Some(p) = &mut m.areas.vm_devices.disks {
                p.review = None;
                p.error = None;
            }
        }
        Err(e) => {
            if let Some(p) = &mut m.areas.vm_devices.disks {
                p.error = Some(e.to_string());
            }
        }
    }
}
