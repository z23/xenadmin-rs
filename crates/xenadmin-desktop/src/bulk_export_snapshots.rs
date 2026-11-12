//! The snapshot half of the batch export editor: several snapshots of one VM, picked in
//! that VM's Snapshots view, each exported as an archive of its own.
use super::*;
use xenadmin_core::{CheckpointMemory, Snapshot, SnapshotId, VdiId};

/// A snapshot as it was when the editor opened, with the choice made for it there.
#[derive(Clone)]
pub(super) struct SnapshotTarget {
    pub(super) snapshot: Snapshot,
    pub(super) parent: VmId,
    pub(super) parent_name: String,
    /// The memory image of a checkpoint, as the inventory had it when the editor opened.
    pub(super) memory: Option<CheckpointMemory>,
    /// Whether the archive is to hold that memory. Off until it is chosen for this
    /// checkpoint; no choice made for another item changes it.
    pub(super) preserve_memory: bool,
}
impl SnapshotTarget {
    fn capture(i: &Inventory, parent: &Vm, snapshot: &Snapshot) -> Self {
        let memory = match ExportSource::checkpoint(i, snapshot.id, parent.id, false) {
            Some(ExportSource::Checkpoint { memory, .. }) => Some(memory),
            _ => None,
        };
        Self {
            snapshot: snapshot.clone(),
            parent: parent.id,
            parent_name: parent.name.clone(),
            memory,
            preserve_memory: false,
        }
    }
    pub(super) fn is_checkpoint(&self) -> bool {
        self.memory.is_some() || self.snapshot.power_state == PowerState::Suspended
    }
    fn kind(&self) -> &'static str {
        if self.memory.is_some() {
            "Memory checkpoint"
        } else if self.is_checkpoint() {
            "Memory checkpoint without a memory image to review"
        } else {
            "Disk snapshot"
        }
    }
    /// The source the single export of this snapshot would take from the inventory as it
    /// is now. None when a checkpoint's memory image is not the one that was captured.
    fn source(&self, i: &Inventory) -> Option<ExportSource> {
        match &self.memory {
            None => Some(ExportSource::Snapshot {
                snapshot: self.snapshot.id,
                parent: self.parent,
            }),
            Some(captured) => {
                ExportSource::checkpoint(i, self.snapshot.id, self.parent, self.preserve_memory)
                    .filter(|source| {
                        matches!(source, ExportSource::Checkpoint { memory, .. } if memory == captured)
                    })
            }
        }
    }
    pub(super) fn item(
        &self,
        i: &Inventory,
        format: &VmExportFormat,
    ) -> Result<BatchVmWorkflow, String> {
        let name = format!("{} ({})", self.snapshot.name, self.snapshot.id);
        if i.snapshots.get(&self.snapshot.id).is_none_or(|now| {
            now.vm != Some(self.parent) || now.power_state != self.snapshot.power_state
        }) {
            return Err(format!("{name}: identity or state changed"));
        }
        let source = self.source(i).ok_or_else(|| {
            format!("{name}: the checkpoint's memory image changed or is no longer available")
        })?;
        let status = action(source.clone(), format).availability(i);
        if !status.is_available() {
            return Err(format!("{name}: {}", status.explanation()));
        }
        BatchVmWorkflow::snapshot_export(&source, format.clone())
            .ok_or_else(|| format!("{name}: not a snapshot"))
    }
    /// The lines under a snapshot's heading: what it is, and for a checkpoint its memory
    /// image and the choice made for it. The choice can be changed while `editing`.
    pub(super) fn describe(
        &self,
        mut body: El,
        index: usize,
        editing: bool,
        busy: bool,
        k: &Kit,
    ) -> El {
        body = body.child(
            caption(
                format!(
                    "{} · {}{}",
                    self.snapshot.id,
                    self.kind(),
                    self.snapshot
                        .created_at
                        .as_deref()
                        .map(|created| format!(" · {created}"))
                        .unwrap_or_default()
                ),
                k,
            )
            .wrap_anywhere(),
        );
        let Some(memory) = &self.memory else {
            return body;
        };
        body = body.child(
            caption(
                format!(
                    "Checkpoint memory image: {} · {} bytes · storage {}",
                    memory.vdi, memory.virtual_size, memory.sr
                ),
                k,
            )
            .wrap_anywhere(),
        );
        if editing {
            let mut choices = row().w_full().flex_wrap().gap(6.0);
            for (preserve, label, key) in [
                (false, "Disks only", "omit"),
                (true, "Include guest memory", "include"),
            ] {
                choices = choices.child(
                    Button::new(label)
                        .variant(crate::theme::choice(self.preserve_memory == preserve))
                        .disabled(busy)
                        .on_press(move |s: &mut State| set_memory(s, index, preserve))
                        .build(k)
                        .key(format!("batch-export-memory-{index}-{key}")),
                );
            }
            body = body.child(choices);
        }
        body.child(
            caption(
                if self.preserve_memory {
                    "Memory: included. The archive holds guest RAM, which may contain credentials and other sensitive data. Import creates a suspended guest; CPU, hypervisor and guest compatibility must be checked before a separate resume."
                } else {
                    "Memory: left out. Only configuration and disks are exported; import creates a halted guest."
                },
                k,
            )
            .wrap()
            .key(format!("batch-export-memory-{index}")),
        )
    }
}
/// Sets one checkpoint's memory choice. A review made before the change no longer stands.
fn set_memory(s: &mut State, index: usize, preserve: bool) {
    let e = edit(s);
    if let Some(Target::Snapshot(target)) = e.targets.get_mut(index)
        && target.memory.is_some()
    {
        target.preserve_memory = preserve;
        e.review = None;
        e.error = None;
    }
}
fn selected(s: &State) -> Result<Vec<SnapshotTarget>, String> {
    let i = s
        .inventory
        .as_ref()
        .ok_or("Current inventory is unavailable")?;
    let parent = s
        .snapshot_view
        .and_then(|id| i.vms.get(&id))
        .ok_or("Open the snapshots of a VM in the current inventory")?;
    let rows = vm_snapshots::selected_snapshots(s).collect::<Vec<_>>();
    if !(1..=32).contains(&rows.len()) {
        return Err("Select 1–32 snapshots of this VM".into());
    }
    rows.into_iter()
        .map(|row| {
            i.snapshots
                .get(&row.id)
                .filter(|now| now.vm == Some(parent.id))
                .map(|now| SnapshotTarget::capture(i, parent, now))
                .ok_or_else(|| "A selected row is no longer a snapshot of this VM".into())
        })
        .collect()
}
fn open(s: &mut State) {
    match selected(s) {
        Ok(targets) => begin(s, targets.into_iter().map(Target::Snapshot).collect()),
        Err(e) => s.error = Some(e),
    }
}
pub(crate) fn button(s: &State, k: &Kit) -> El {
    Button::new("Export selection…")
        .variant(ButtonVariant::Outline)
        .disabled(selected(s).is_err())
        .on_press(open)
        .build(k)
        .key("snapshot-batch-export-open")
}
/// What the batch does and does not do, under the snapshot list of the dialog.
pub(super) fn explain(body: El, k: &Kit) -> El {
    body.child(caption("Every snapshot and destination is checked before the first transfer. Exports run one at a time and change neither the snapshots nor the VM. Stop finishes the current archive and skips the remainder. Recovery reads existing evidence; it never starts another transfer or publishes a new output. Existing files and directories are never overwritten.", k).wrap())
        .child(caption("Each archive is one independent guest: snapshot ancestry is not included. A checkpoint's archive holds guest memory only where Include guest memory is chosen for that checkpoint. OVF/OVA uses the supported ordinary HVM subset and cannot hold a checkpoint. Read-only connections may export, subject to host permission.", k).wrap())
}
const DEMO_PARENT: &str = "30000000-0000-0000-0000-000000000000";
const DEMO_SNAPSHOT: &str = "60000000-0000-0000-0000-000000000000";
const DEMO_CHECKPOINT: &str = "60000000-0000-0000-0000-000000000001";
/// Turns one of the synthetic inventory's disk snapshots into a memory checkpoint whose
/// memory image can be reviewed.
fn demo_checkpoint(s: &mut State, snapshot: SnapshotId, memory: VdiId) {
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let mut disk = i.details.vdis.values().next().unwrap().clone();
    disk.id = memory;
    disk.name = "Checkpoint memory".into();
    disk.kind = "suspend".into();
    disk.virtual_size = Some(512 * 1024 * 1024);
    disk.managed = Some(true);
    disk.missing = Some(false);
    i.details.vdis.insert(memory, disk);
    i.details.snapshots.insert(
        snapshot,
        xenadmin_core::SnapshotDetails {
            memory_vdi: Some(memory),
            memory_vdi_known: true,
            can_export: Some(true),
            ..Default::default()
        },
    );
    let mut suspended = i.snapshots.get(&snapshot).unwrap().clone();
    suspended.power_state = PowerState::Suspended;
    suspended.description = "Synthetic memory checkpoint".into();
    i.snapshots.insert(suspended);
}
/// Opens the editor on the given snapshots of the synthetic VM. The table has no way to
/// select a second row from code, so every snapshot after the first is captured the way
/// a selected row is and added to the editor.
fn demo_editor(s: &mut State, snapshots: &[SnapshotId]) {
    let parent = VmId::parse(DEMO_PARENT).unwrap();
    s.table.select(parent.to_string());
    vm_snapshots::open(s);
    s.snapshot_table.select(snapshots[0].to_string());
    open(s);
    for snapshot in &snapshots[1..] {
        let i = s.inventory.as_ref().unwrap();
        let target =
            SnapshotTarget::capture(i, &i.vms[&parent], i.snapshots.get(snapshot).unwrap());
        let e = edit(s);
        e.targets.push(Target::Snapshot(target));
        e.files.push(None);
    }
}
/// A synthetic review of one disk snapshot and one memory checkpoint with its memory
/// chosen, for renders and tests.
pub(crate) fn scene(s: &mut State) {
    demo(s);
    let plain = SnapshotId::parse(DEMO_SNAPSHOT).unwrap();
    let checkpoint = SnapshotId::parse(DEMO_CHECKPOINT).unwrap();
    demo_checkpoint(s, checkpoint, VdiId::from(uuid::Uuid::from_u128(0x9902)));
    demo_editor(s, &[plain, checkpoint]);
    set_memory(s, 1, true);
    demo_outputs(s);
    review(s);
}

#[cfg(test)]
#[path = "bulk_export_snapshots_tests.rs"]
mod tests;
