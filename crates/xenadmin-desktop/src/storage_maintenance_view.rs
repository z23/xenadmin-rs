use super::*;
use xenadmin_core::{ActionAvailability, ReviewedStorageAttachment, StorageMaintenance};
fn stage(s: &mut State, cache: bool) {
    let result = (|| {
        let e = s.storage_editor.as_ref().ok_or("Storage view closed")?;
        let ResourceTarget::Storage(sr) = e.target else {
            return Err("Select a storage repository");
        };
        let i = s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == e.pool)
            .ok_or("The original pool is no longer current")?;
        if !s.demo && !current(s.snapshot.as_deref(), e) {
            return Err("The captured connection changed; reopen storage administration");
        }
        let op = if cache {
            let d = i
                .details
                .storage
                .get(&sr)
                .ok_or("Storage details unavailable")?;
            if !d.read_cache_policy_known {
                return Err("Read-cache policy was not provided by the host");
            }
            StorageMaintenance::ReadCaching {
                expected: d.read_cache_override.clone(),
                enabled: d.read_cache_override.as_deref() == Some("true"),
            }
        } else {
            let attachments = e
                .repair_selection
                .iter()
                .map(|id| ReviewedStorageAttachment::from_inventory(i, sr, *id))
                .collect::<Option<Vec<_>>>()
                .ok_or("A selected attachment changed; select it again")?;
            StorageMaintenance::Repair {
                coordinator: i.coordinator.ok_or("Pool coordinator is unavailable")?,
                attachments,
            }
        };
        if !op.availability(i, sr).is_available() {
            return Err("Choose current detached host attachments or a changed cache policy");
        }
        Ok(op)
    })();
    let e = s.storage_editor.as_mut().unwrap();
    match result {
        Ok(op) => {
            e.maintenance_review = Some(op);
            e.error = None;
        }
        Err(err) => e.error = Some(err.into()),
    }
}
pub(super) fn controls(s: &State, k: &Kit) -> El {
    let e = s.storage_editor.as_ref().unwrap();
    let ResourceTarget::Storage(sr) = e.target else {
        return el();
    };
    let Some(i) = s.inventory.as_ref().filter(|i| i.pool_id == e.pool) else {
        return el();
    };
    let mut out=col().gap(10.).child(heading("Repair across hosts",k)).child(caption("Select existing detached attachments. The coordinator runs first when selected. Missing attachments are created separately with an explicit location and credentials.",k).wrap());
    let candidates = i
        .details
        .pbds
        .values()
        .filter(|p| p.sr == Some(sr) && p.currently_attached == Some(false))
        .collect::<Vec<_>>();
    for p in candidates.iter().take(128) {
        let id = p.id;
        let name = p
            .host
            .and_then(|h| i.hosts.get(&h))
            .map(|h| h.name.clone())
            .unwrap_or_else(|| "Unknown host".into());
        out = out
            .child(heading(name, k).wrap_anywhere())
            .child(caption(format!("Attachment {id}"), k).wrap_anywhere())
            .child(
                Checkbox::new(e.repair_selection.contains(&id))
                    .label("Include this host attachment")
                    .on_toggle(move |s: &mut State| {
                        let e = s.storage_editor.as_mut().unwrap();
                        if !e.repair_selection.remove(&id) {
                            e.repair_selection.insert(id);
                        }
                    })
                    .build(k)
                    .key(format!("repair-pbd-{id}")),
            );
    }
    if candidates.is_empty() {
        out = out.child(caption("No existing detached attachments were reported.", k).wrap());
    }
    if candidates.len() > 128 {
        out = out.child(
            caption(
                "Only the first 128 attachments are shown. Repair smaller reviewed groups.",
                k,
            )
            .wrap(),
        );
    }
    out = out.child(
        Button::new("Review selected repairs…")
            .disabled(e.repair_selection.is_empty())
            .on_press(|s: &mut State| stage(s, false))
            .build(k)
            .key("storage-repair-review"),
    );
    let kind = i
        .storage
        .iter()
        .find(|v| v.uuid == sr.uuid())
        .map(|v| v.kind.as_str())
        .unwrap_or("");
    if let Some(b) = e.backends.iter().find(|b| b.kind == kind) {
        let thin = b.features.iter().any(|f| f == "THIN_PROVISIONING");
        out=out.child(heading("Provisioning and read caching",k)).child(caption(format!("Driver: {} · Thin provisioning: {}",b.name,if thin{"advertised"}else{"not advertised"}),k).wrap()).child(caption("Provisioning is determined by the repository driver. This view does not convert existing storage between thin and thick allocation.",k).wrap());
        if matches!(kind, "nfs" | "ext" | "smb")
            && b.features.iter().any(|f| f == "VDI_READ_CACHING")
        {
            if let Some(d) = i.details.storage.get(&sr) {
                let disabled = d.read_cache_override.as_deref() == Some("true");
                out=out.child(caption(format!("Read-cache policy: {}",if !d.read_cache_policy_known{"Unknown"}else if disabled{"Disabled"}else{"Permitted"}),k).wrap()).child(caption("The driver applies this policy when opening disk backing storage. Licensing, disk type and host state can still prevent caching; active disks are not reopened.",k).wrap()).child(Button::new(if disabled{"Review enabling read caching…"}else{"Review disabling read caching…"}).disabled(!d.read_cache_policy_known).on_press(|s:&mut State|stage(s,true)).build(k).key("storage-cache-review"));
            }
        } else {
            out=out.child(caption("This driver does not advertise supported read caching. No cache setting is offered.",k).wrap());
        }
    } else {
        out = out
            .child(
                caption(
                    "Read driver capabilities to inspect provisioning and read-cache support.",
                    k,
                )
                .wrap(),
            )
            .child(
                Button::new("Refresh storage capabilities")
                    .on_press(|s: &mut State| query(s, 0))
                    .build(k),
            );
    }
    out
}
pub(super) fn review_body(s: &State, k: &Kit) -> El {
    let e = s.storage_editor.as_ref().unwrap();
    let op = e.maintenance_review.as_ref().unwrap();
    let mut out = col().gap(10.).child(heading(op.label(), k)).child(
        caption(
            format!("Repository: {}\nUUID: {}", e.name.text, e.target.uuid()),
            k,
        )
        .wrap_anywhere(),
    );
    match op {
        StorageMaintenance::Repair {
            coordinator,
            attachments,
        } => {
            let mut attachments = attachments.clone();
            attachments.sort_by_key(|p| (p.host != *coordinator, p.host));
            out=out.child(caption("Only these existing attachments will be plugged, in order. A failure stops later hosts; earlier successful attachments remain connected. Stop remaining steps does not cancel an in-flight host task. Attached status does not prove storage health.",k).wrap());
            for (n, p) in attachments.iter().enumerate() {
                let name = s
                    .inventory
                    .as_ref()
                    .filter(|i| i.pool_id == e.pool)
                    .and_then(|i| i.hosts.get(&p.host))
                    .map(|h| h.name.as_str())
                    .unwrap_or("Host");
                out = out.child(heading(format!("{}. {name}", n + 1), k)).child(
                    caption(
                        format!(
                            "Host {}{}\nAttachment {}",
                            p.host,
                            if p.host == *coordinator {
                                " · coordinator"
                            } else {
                                ""
                            },
                            p.attachment
                        ),
                        k,
                    )
                    .wrap_anywhere(),
                );
                if let Some(location) = &p.location {
                    for line in location_lines(location) {
                        out = out.child(caption(line, k).wrap_anywhere());
                    }
                }
                out=out.child(caption(if p.requires_credentials{"Uses credentials already stored by the host. No credentials are copied or replaced."}else{"Uses the existing host attachment configuration."},k).wrap());
            }
        }
        StorageMaintenance::ReadCaching { enabled, .. } => {
            out=out.child(caption(if *enabled{"Permit read caching by removing the repository override."}else{"Disable read caching with the repository override."},k).wrap()).child(caption("Preserves other repository settings. This changes policy only; it does not establish effective caching, alter driver provisioning, or reopen active disks. No VM restart is performed.",k).wrap());
        }
    }
    if let ResourceTarget::Storage(sr) = e.target {
        let availability = s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == e.pool)
            .map(|i| op.availability(i, sr))
            .unwrap_or(ActionAvailability::Stale);
        if !availability.is_available() {
            out = out.child(
                caption(availability.explanation(), k)
                    .wrap()
                    .color(k.c.danger.icon),
            );
        }
    }
    if !s.demo && !current(s.snapshot.as_deref(), e) {
        out=out.child(caption("The captured connection changed; close and reopen storage administration before submitting.",k).wrap().color(k.c.danger.icon));
    }
    out
}
pub(super) fn footer(s: &State, k: &Kit) -> El {
    let e = s.storage_editor.as_ref().unwrap();
    let op = e.maintenance_review.as_ref().unwrap();
    let available = if let ResourceTarget::Storage(sr) = e.target {
        s.inventory
            .as_ref()
            .filter(|i| i.pool_id == e.pool)
            .is_some_and(|i| op.availability(i, sr).is_available())
    } else {
        false
    };
    row()
        .gap(8.)
        .flex_wrap()
        .justify_end()
        .child(
            Button::new("Back")
                .on_press(|s: &mut State| {
                    s.storage_editor.as_mut().unwrap().maintenance_review = None
                })
                .build(k)
                .key("storage-maintenance-back"),
        )
        .child(
            Button::new("Confirm change")
                .disabled(
                    s.demo
                        || !s.can_mutate()
                        || !current(s.snapshot.as_deref(), e)
                        || !available
                        || crate::session_permissions_view::assess(
                            s,
                            &op.permission_requirements(),
                        )
                        .decision()
                        .is_denied(),
                )
                .on_press(apply)
                .build(k)
                .key("storage-maintenance-confirm"),
        )
}
fn apply(s: &mut State) {
    s.resource_operations
        .retain(|o| !o.latest().state.terminal());
    let result = (|| {
        let e = s
            .storage_editor
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?;
        if s.demo || !current(s.snapshot.as_deref(), e) {
            return Err(xenadmin_core::Error::Stale);
        }
        if s.resource_operations.len() >= 64 {
            return Err(xenadmin_core::Error::Busy);
        }
        let op = e
            .maintenance_review
            .as_ref()
            .ok_or(xenadmin_core::Error::InvalidResourceCommand)?;
        let ResourceTarget::Storage(sr) = e.target else {
            return Err(xenadmin_core::Error::InvalidResourceCommand);
        };
        if !s
            .inventory
            .as_ref()
            .filter(|i| i.pool_id == e.pool)
            .is_some_and(|i| op.availability(i, sr).is_available())
        {
            return Err(xenadmin_core::Error::Stale);
        }
        s.connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .submit_resource(
                OperationId::new(),
                ResourceCommand::StorageMaintenance {
                    target: e.observed.ok_or(xenadmin_core::Error::Stale)?,
                    operation: op.clone(),
                },
            )
    })();
    match result {
        Ok(o) => {
            if let Some(w) = &s.waker {
                o.notify_on_change(w.clone());
            }
            s.resource_operations.push(o);
            s.storage_editor = None;
        }
        Err(err) => {
            if let Some(e) = &mut s.storage_editor {
                e.error = Some(err.to_string());
            }
        }
    }
}
pub(super) fn scene(s: &mut State, cache: bool) {
    crate::demo(s);
    s.page = Page::Storage;
    s.rebuild();
    let sr = s
        .inventory
        .as_ref()
        .unwrap()
        .details
        .storage
        .keys()
        .next()
        .copied()
        .unwrap();
    {
        let i = Arc::make_mut(s.inventory.as_mut().unwrap());
        let hosts = i.hosts.keys().copied().take(2).collect::<Vec<_>>();
        for (n, host) in hosts.into_iter().enumerate() {
            let id = xenadmin_core::PbdId::from(uuid::Uuid::from_u128(950 + n as u128));
            i.details.pbds.insert(
                id,
                xenadmin_core::StorageAttachment {
                    id,
                    host: Some(host),
                    sr: Some(sr),
                    currently_attached: Some(false),
                    location: Some(StorageLocation::Nfs {
                        server: "storage.example.test".into(),
                        path: "/guests".into(),
                        version: NfsVersion::V4,
                    }),
                    requires_credentials: false,
                    multipathed: None,
                    paths: vec![],
                },
            );
        }
        if let Some(row) = i.storage.iter_mut().find(|r| r.uuid == sr.uuid()) {
            row.kind = "nfs".into();
        }

        for p in i.details.pbds.values_mut().filter(|p| p.sr == Some(sr)) {
            p.currently_attached = Some(false);
        }
        i.details
            .classes
            .insert("pbd".into(), xenadmin_core::ClassAvailability::Available);
    }
    open(s, Some(sr));
    let selected = s
        .inventory
        .as_ref()
        .unwrap()
        .details
        .pbds
        .values()
        .filter(|p| p.sr == Some(sr))
        .map(|p| p.id)
        .collect();
    s.storage_editor.as_mut().unwrap().repair_selection = selected;
    s.storage_editor.as_mut().unwrap().backends = vec![StorageBackend {
        kind: "nfs".into(),
        name: "NFS".into(),
        features: vec!["VDI_READ_CACHING".into(), "THIN_PROVISIONING".into()],
        configuration: Default::default(),
    }];
    stage(s, cache);
}
#[cfg(test)]
#[path = "storage_maintenance_view_tests.rs"]
mod tests;
