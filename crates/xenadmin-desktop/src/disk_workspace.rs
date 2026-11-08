use super::*;
use xenadmin_core::{
    DiskOperation, ObservedResource, OwnedDiskSource, ResourceAction, ResourceCommand,
    ResourceTarget, ReviewedDisk, SrId, VdiId,
};
#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Grow,
    Copy,
    Cleanup,
}
#[derive(Clone)]
struct Review {
    target: Option<ObservedResource>,
    disk: VdiId,
    name: String,
    operation: DiskOperation,
}
pub(super) struct Panel {
    pool: PoolId,
    observed: Option<ObservedResource>,
    sr: Option<SrId>,
    selected: Option<VdiId>,
    search: TextEdit,
    page: usize,
    mode: Mode,
    size: TextEdit,
    name: TextEdit,
    destination: Option<SrId>,
    review: Option<Review>,
    confirmation: TextEdit,
    error: Option<String>,
}
fn close(s: &mut State) {
    s.disk_workspace = None;
}
fn history(s: &mut State) {
    if s.disk_workspace
        .as_ref()
        .is_some_and(|p| s.inventory.as_ref().is_some_and(|i| i.pool_id == p.pool))
    {
        close(s);
        crate::resource_admin::open_pool(s, crate::resource_admin::Kind::History);
    }
}
fn current(s: &State, p: &Panel) -> bool {
    s.inventory.as_ref().is_some_and(|i| i.pool_id == p.pool)
        && (s.demo
            || p.observed.is_some_and(|o| {
                s.snapshot
                    .as_ref()
                    .and_then(|v| v.observe_resource(ResourceTarget::Pool(p.pool)).ok())
                    == Some(o)
            }))
}
pub(super) fn open(s: &mut State, sr: Option<SrId>, disk: Option<VdiId>) {
    let Some(i) = &s.inventory else {
        return;
    };
    s.disk_workspace = Some(Box::new(Panel {
        pool: i.pool_id,
        observed: s
            .snapshot
            .as_ref()
            .and_then(|v| v.observe_resource(ResourceTarget::Pool(i.pool_id)).ok()),
        sr,
        selected: disk,
        search: TextEdit::new(""),
        page: 0,
        mode: Mode::Grow,
        size: TextEdit::new(""),
        name: TextEdit::new(""),
        destination: sr,
        review: None,
        confirmation: TextEdit::new(""),
        error: None,
    }));
    s.show_details = false;
}
pub(super) fn storage_button(s: &State, k: &Kit) -> El {
    if s.page != Page::Storage {
        return el();
    }
    let sr = s.selected().and_then(|r| SrId::parse(&r.id).ok());
    Button::new("Virtual disks…")
        .on_press(move |s: &mut State| open(s, sr, None))
        .build(k)
        .key("disks-open")
}
pub(super) fn disk_button(id: VdiId, k: &Kit) -> El {
    Button::new("Grow, copy or clean up disk…")
        .variant(ButtonVariant::Outline)
        .on_press(move |s: &mut State| open(s, None, Some(id)))
        .build(k)
        .key(format!("disk-manage-{id}"))
}
fn source(s: &State, pool: PoolId, disk: VdiId) -> Option<OwnedDiskSource> {
    s.resource_rows
        .iter()
        .find(|r| r.pool == pool && r.owned_disk_candidates().contains(&disk))
        .map(|r| OwnedDiskSource::ResourceOperation(r.id))
        .or_else(|| {
            s.operation_rows
                .iter()
                .find(|r| r.pool == pool && r.owned_disk_candidates().contains(&disk))
                .map(|r| OwnedDiskSource::VmOperation(r.id))
        })
}
fn review(s: &mut State) {
    let result = (|| {
        let p = s
            .disk_workspace
            .as_ref()
            .ok_or("Disk workspace is closed")?;
        if !current(s, p) {
            return Err("The captured connection changed; close and reopen this workspace".into());
        }
        let i = s.inventory.as_ref().ok_or("Inventory is unavailable")?;
        let disk = p.selected.ok_or("Choose a disk")?;
        let expected = ReviewedDisk::from_inventory(i, disk)
            .ok_or("Complete ordinary disk and attachment data is required")?;
        let operation=match p.mode{
            Mode::Grow=>DiskOperation::Grow{expected,size:crate::vm_devices::actions::parse_size(&p.size.text)?},
            Mode::Copy=>DiskOperation::Copy{expected,destination:p.destination.ok_or("Choose destination storage")?,name:p.name.text.trim().into()},
            Mode::Cleanup=>DiskOperation::Cleanup{expected,source:source(s,p.pool,disk).ok_or("No terminal local creation record proves ownership of this disk. Resolve its originating operation first; arbitrary detached disks cannot be deleted here")?},
        };
        let availability = operation.availability(i, disk);
        if !availability.is_available() {
            return Err(availability.explanation());
        }
        Ok(Review {
            target: s
                .snapshot
                .as_ref()
                .and_then(|v| v.observe_resource(ResourceTarget::Disk(disk)).ok()),
            disk,
            name: i.details.vdis.get(&disk).unwrap().name.clone(),
            operation,
        })
    })();
    if let Some(p) = &mut s.disk_workspace {
        match result {
            Ok(r) => {
                p.review = Some(r);
                p.confirmation = TextEdit::new("");
                p.error = None;
            }
            Err(e) => p.error = Some(e),
        }
    }
}
fn apply(s: &mut State) {
    let Some(p) = &s.disk_workspace else {
        return;
    };
    let Some(r) = p.review.clone() else {
        return;
    };
    if !current(s, p) || !s.can_mutate() || s.resource_operations.len() >= 128 {
        return;
    }
    if matches!(r.operation, DiskOperation::Cleanup { .. })
        && p.confirmation.text != r.disk.to_string()
    {
        return;
    }
    let Some(target) = r.target else {
        return;
    };
    if s.snapshot
        .as_ref()
        .and_then(|v| v.observe_resource(ResourceTarget::Disk(r.disk)).ok())
        != Some(target)
        || !s
            .inventory
            .as_ref()
            .is_some_and(|i| r.operation.availability(i, r.disk).is_available())
        || !session_permissions_view::check(s, &r.operation.permission_requirements())
    {
        return;
    }
    let result = s
        .connection
        .as_ref()
        .ok_or(xenadmin_core::Error::Closed)
        .and_then(|c| {
            c.submit_resource(
                xenadmin_core::OperationId::new(),
                ResourceCommand::Disk {
                    target,
                    operation: r.operation,
                },
            )
        });
    match result {
        Ok(op) => {
            if let Some(w) = &s.waker {
                op.notify_on_change(w.clone());
            }
            s.resource_operations.push(op);
            if let Some(p) = &mut s.disk_workspace {
                p.review = None;
                p.error = None;
            }
        }
        Err(e) => {
            if let Some(p) = &mut s.disk_workspace {
                p.error = Some(e.to_string());
            }
        }
    }
}
fn paragraph(value: impl Into<String>, k: &Kit) -> El {
    caption(value, k).wrap_anywhere()
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(p) = &s.disk_workspace else {
        return el();
    };
    let i = s.inventory.as_ref().filter(|i| i.pool_id == p.pool);
    let mut body = col().gap(12.0).w_full();
    let mut footer = row().gap(8.0).flex_wrap().child(
        Button::new("Close")
            .on_press(close)
            .build(k)
            .key("disks-close"),
    );
    footer = footer.child(
        Button::new("Operation history")
            .disabled(i.is_none())
            .on_press(history)
            .build(k)
            .key("disk-history"),
    );
    let mut permission = None;
    if !current(s, p) {
        body=body.child(paragraph("The captured connection changed. Close and reopen this workspace before reviewing a change.",k));
    }
    if let Some(r) = &p.review {
        permission = Some(session_permissions_view::assess(
            s,
            &r.operation.permission_requirements(),
        ));
        body = body.child(heading(r.operation.label(), k)).child(paragraph(
            format!(
                "Disk: {}\nUUID: {}\nPool: {}\nStorage: {}\nCurrent size: {} bytes",
                r.name,
                r.disk,
                p.pool,
                r.operation.expected().sr,
                r.operation.expected().size
            ),
            k,
        ));
        match &r.operation {
            DiskOperation::Grow { size, expected } => {
                body=body.child(paragraph(format!("Grow to at least {size} bytes. The storage driver may round upward. Guest partitions and filesystems must be expanded separately. Shrinking and online resize are not offered."),k));
                for c in &expected.consumers {
                    body = body.child(paragraph(
                        format!("Inactive attachment {} on VM {}", c.attachment, c.vm),
                        k,
                    ));
                }
            }
            DiskOperation::Copy {
                destination, name, ..
            } => {
                body=body.child(paragraph(format!("Create a detached copy named {name} on storage {destination}. The original disk and attachments remain. Copy and naming are separate tracked steps; an interrupted copy may retain a disk even if naming fails."),k));
            }
            DiskOperation::Cleanup { source, .. } => {
                body=body.child(paragraph(format!("Permanently delete this detached disk and its data. Ownership comes from local operation {}. The backend rechecks every attachment, snapshot and suspend-image reference. This is not reversible.",source.operation()),k)).child(paragraph("Type the disk UUID to confirm",k)).child(TextInput::new(&p.confirmation).on_edit(|s:&mut State|&mut s.disk_workspace.as_mut().unwrap().confirmation).build(k).key("disk-delete-confirmation"));
            }
        }
        let availability = i.map(|i| r.operation.availability(i, r.disk));
        if let Some(a) = &availability
            && !a.is_available()
        {
            body = body.child(paragraph(a.explanation(), k));
        }
        let destructive = matches!(r.operation, DiskOperation::Cleanup { .. });
        footer = footer
            .child(
                Button::new("Back")
                    .on_press(|s: &mut State| s.disk_workspace.as_mut().unwrap().review = None)
                    .build(k)
                    .key("disk-review-back"),
            )
            .child(
                Button::new("Confirm")
                    .disabled(
                        !current(s, p)
                            || !s.can_mutate()
                            || r.target.is_none()
                            || availability.is_none_or(|a| !a.is_available())
                            || permission
                                .as_ref()
                                .is_some_and(|v| v.decision().is_denied())
                            || s.resource_operations.len() >= 128
                            || (destructive && p.confirmation.text != r.disk.to_string()),
                    )
                    .on_press(apply)
                    .build(k)
                    .key("disk-confirm"),
            );
    } else if let Some(i) = i {
        if let Some(disk) = p.selected.and_then(|id| i.details.vdis.get(&id)) {
            body = body.child(heading(disk.name.clone(), k)).child(paragraph(
                format!(
                    "UUID: {}\nStorage: {}\nSize: {}",
                    disk.id,
                    disk.sr
                        .map(|s| s.to_string())
                        .unwrap_or_else(|| "Unknown".into()),
                    disk.virtual_size
                        .map(|n| format!("{n} bytes"))
                        .unwrap_or_else(|| "Unknown".into())
                ),
                k,
            ));
            body = body.child(
                row().gap(8.0).flex_wrap().children(
                    [
                        (Mode::Grow, "Grow"),
                        (Mode::Copy, "Copy"),
                        (Mode::Cleanup, "Owned cleanup"),
                    ]
                    .into_iter()
                    .map(|(mode, label)| {
                        Button::new(label)
                            .variant(if p.mode == mode {
                                ButtonVariant::Outline
                            } else {
                                ButtonVariant::Ghost
                            })
                            .on_press(move |s: &mut State| {
                                let p = s.disk_workspace.as_mut().unwrap();
                                p.mode = mode;
                                p.error = None;
                            })
                            .build(k)
                            .key(format!("disk-mode-{label}"))
                    }),
                ),
            );
            match p.mode {
                Mode::Grow => {
                    body=body.child(paragraph("New total capacity: bytes, KiB, MiB or GiB. Every attachment must be inactive and unreserved. A running VM may retain an unplugged attachment.",k)).child(TextInput::new(&p.size).on_edit(|s:&mut State|&mut s.disk_workspace.as_mut().unwrap().size).build(k).key("disk-size"));
                }
                Mode::Copy => {
                    body = body
                        .child(paragraph("Copy name", k))
                        .child(
                            TextInput::new(&p.name)
                                .on_edit(|s: &mut State| {
                                    &mut s.disk_workspace.as_mut().unwrap().name
                                })
                                .build(k)
                                .key("disk-copy-name"),
                        )
                        .child(paragraph("Destination storage", k));
                    for sr in &i.storage {
                        if let Ok(id) = SrId::parse(&sr.uuid.to_string()) {
                            body = body.child(
                                Button::new(format!(
                                    "{}{}",
                                    if p.destination == Some(id) {
                                        "✓ "
                                    } else {
                                        ""
                                    },
                                    sr.name
                                ))
                                .on_press(move |s: &mut State| {
                                    s.disk_workspace.as_mut().unwrap().destination = Some(id)
                                })
                                .build(k)
                                .key(format!("disk-destination-{id}")),
                            );
                        }
                    }
                }
                Mode::Cleanup => {
                    body=body.child(paragraph(if let Some(origin)=source(s,p.pool,disk.id){format!("Recorded creation: {}. Cleanup is available only after the disk is fully detached. Unknown originating operations must be resolved or explicitly acknowledged first.",origin.operation())}else{"No terminal local creation record proves ownership. Untracked detached disks cannot be deleted here.".into()},k));
                }
            }
            footer = footer
                .child(
                    Button::new("All disks")
                        .on_press(|s: &mut State| {
                            let p = s.disk_workspace.as_mut().unwrap();
                            p.selected = None;
                            p.error = None;
                        })
                        .build(k)
                        .key("disk-list-back"),
                )
                .child(
                    Button::new("Review")
                        .disabled(!current(s, p))
                        .on_press(review)
                        .build(k)
                        .key("disk-review"),
                );
        } else {
            body = body.child(
                TextInput::new(&p.search)
                    .placeholder("Search disk name or UUID")
                    .on_edit(|s: &mut State| {
                        let p = s.disk_workspace.as_mut().unwrap();
                        p.page = 0;
                        &mut p.search
                    })
                    .build(k)
                    .key("disk-search"),
            );
            let query = p.search.text.to_lowercase();
            let mut disks = i
                .details
                .vdis
                .values()
                .filter(|d| {
                    p.sr.is_none_or(|sr| d.sr == Some(sr))
                        && (d.name.to_lowercase().contains(&query)
                            || d.id.to_string().contains(&query))
                })
                .collect::<Vec<_>>();
            disks.sort_by(|a, b| a.name.cmp(&b.name).then(a.id.cmp(&b.id)));
            body = body.child(paragraph(
                format!(
                    "{} disks{}",
                    disks.len(),
                    if p.sr.is_some() {
                        " on selected storage"
                    } else {
                        ""
                    }
                ),
                k,
            ));
            for d in disks.iter().skip(p.page * 20).take(20) {
                let id = d.id;
                let name = d.name.clone();
                body = body.child(
                    Button::new(format!(
                        "{} · {}",
                        d.name,
                        d.virtual_size
                            .map(|n| format!("{} MiB", n / (1024 * 1024)))
                            .unwrap_or_else(|| "size unknown".into())
                    ))
                    .on_press(move |s: &mut State| {
                        let p = s.disk_workspace.as_mut().unwrap();
                        p.selected = Some(id);
                        p.name = TextEdit::new(format!("{name} copy"));
                        p.error = None;
                    })
                    .build(k)
                    .key(format!("disk-select-{id}")),
                );
            }
            footer = footer
                .child(
                    Button::new("Previous")
                        .disabled(p.page == 0)
                        .on_press(|s: &mut State| {
                            let p = s.disk_workspace.as_mut().unwrap();
                            p.page = p.page.saturating_sub(1);
                        })
                        .build(k),
                )
                .child(
                    Button::new("Next")
                        .disabled((p.page + 1) * 20 >= disks.len())
                        .on_press(|s: &mut State| s.disk_workspace.as_mut().unwrap().page += 1)
                        .build(k),
                );
        }
    }
    if let Some(error) = &p.error {
        body = body.child(paragraph(error.clone(), k).color(k.c.danger.icon));
    }
    for r in s
        .resource_rows
        .iter()
        .filter(|r| r.pool == p.pool && matches!(r.action, ResourceAction::Disk(_)))
        .take(15)
    {
        body = body
            .child(heading(format!("{} · {:?}", r.action.label(), r.state), k))
            .child(paragraph(r.detail.clone(), k));
        if let Some(ResourceTarget::Disk(id)) = r.created {
            body = body.child(paragraph(format!("Created disk: {id}"), k));
        }
    }
    body=body.child(paragraph("Unknown outcomes reserve the pool. Operation history provides rechecks, stop controls and explicit acknowledgement. Stop remaining steps does not cancel in-flight disk I/O.",k));
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Virtual disks")
        .modal(false, close)
        .child(
            col()
                .w((s.window_width - 32.).clamp(280., 760.))
                .h((s.window_height - 56.).clamp(280., 820.))
                .padding(20.)
                .gap(14.)
                .radius(10.)
                .fill(k.c.bg_base)
                .child(heading("Virtual disks", k).key("disk-heading"))
                .child(body.grow(1.).min_h(0.).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(session_permissions_view::footer(
                    footer,
                    permission.as_ref(),
                    (s.window_width - 32.).clamp(280., 760.) - 40.,
                    k,
                )),
        )
}
pub(super) fn scene(s: &mut State, copy: bool) {
    demo(s);
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    let Some(disk) = i.details.vdis.values_mut().find(|d| d.kind == "user") else {
        return;
    };
    let id = disk.id;
    disk.cbt_enabled = Some(false);
    disk.allowed_operations = Some(vec!["resize".into(), "copy".into(), "destroy".into()]);
    i.details.vbds.retain(|_, v| v.vdi != Some(id));
    let sr = disk.sr;
    open(s, None, Some(id));
    let p = s.disk_workspace.as_mut().unwrap();
    p.mode = if copy { Mode::Copy } else { Mode::Grow };
    p.size = TextEdit::new("64 GiB");
    p.name = TextEdit::new("Fixture disk copy");
    p.destination = sr;
    review(s);
}
pub(super) fn cleanup_scene(s: &mut State) {
    scene(s, true);
    if let Some(p) = &mut s.disk_workspace
        && let Some(r) = &mut p.review
    {
        r.operation = DiskOperation::Cleanup {
            expected: r.operation.expected().clone(),
            source: OwnedDiskSource::ResourceOperation(xenadmin_core::OperationId::new()),
        };
        p.mode = Mode::Cleanup;
    }
}
#[cfg(test)]
#[path = "disk_workspace_tests.rs"]
mod tests;
