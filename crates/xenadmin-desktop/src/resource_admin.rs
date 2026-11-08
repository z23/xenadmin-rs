use super::*;
use xenadmin_core::{
    ObservedResource, OperationControl, OperationId, OperationState, ResourceCommand,
    ResourceTarget, SrId,
};

#[derive(Clone, Copy, PartialEq, Eq)]
pub(super) enum Kind {
    History,
    Rename,
    CreateNetwork,
    Mtu,
    DeleteNetwork,
    ScanStorage,
    HostEnabled(bool),
    DefaultStorage,
    Acknowledge(OperationId),
}
impl Kind {
    fn title(self) -> &'static str {
        match self {
            Self::History => "Pool administration",
            Self::Rename => "Edit name and description",
            Self::CreateNetwork => "Create internal network",
            Self::Mtu => "Change internal network MTU",
            Self::DeleteNetwork => "Delete internal network",
            Self::ScanStorage => "Rescan storage repository",
            Self::HostEnabled(true) => "Enable host scheduling",
            Self::HostEnabled(false) => "Disable host scheduling",
            Self::DefaultStorage => "Choose default storage",
            Self::Acknowledge(_) => "Acknowledge unknown outcome",
        }
    }
    fn danger(self) -> bool {
        matches!(self, Self::DeleteNetwork | Self::Acknowledge(_))
    }
}
struct Review {
    name: String,
    description: String,
    mtu: u64,
    sr: Option<SrId>,
    lines: Vec<String>,
}
pub(super) struct Editor {
    kind: Kind,
    target: ResourceTarget,
    observed: Option<ObservedResource>,
    pool: PoolId,
    label: String,
    expected_name: String,
    expected_description: String,
    expected_mtu: Option<u64>,
    expected_enabled: Option<bool>,
    expected_default: Option<SrId>,
    name: TextEdit,
    description: TextEdit,
    mtu: TextEdit,
    selected_sr: Option<SrId>,
    review: Option<Review>,
    error: Option<String>,
}
fn metadata(inventory: &Inventory, target: ResourceTarget) -> Option<(String, String)> {
    match target {
        ResourceTarget::Pool(id) if id == inventory.pool_id => Some((
            inventory.pool_name.clone(),
            inventory.details.pool.as_ref()?.description.clone(),
        )),
        ResourceTarget::Host(id) => Some((
            inventory.hosts.get(&id)?.name.clone(),
            inventory.details.hosts.get(&id)?.description.clone(),
        )),
        ResourceTarget::Storage(id) => Some((
            inventory
                .storage
                .iter()
                .find(|s| s.uuid == id.uuid())?
                .name
                .clone(),
            inventory.details.storage.get(&id)?.description.clone(),
        )),
        ResourceTarget::Network(id) => Some((
            inventory
                .networks
                .iter()
                .find(|n| n.uuid == id.uuid())?
                .name
                .clone(),
            inventory.details.networks.get(&id)?.description.clone(),
        )),
        _ => None,
    }
}
fn target_label(target: ResourceTarget) -> String {
    match target {
        ResourceTarget::Pool(id) => format!("Pool {id}"),
        ResourceTarget::Host(id) => format!("Host {id}"),
        ResourceTarget::Storage(id) => format!("Storage {id}"),
        ResourceTarget::Network(id) => format!("Network {id}"),
    }
}
fn display_name(inventory: &Inventory, target: ResourceTarget) -> Option<String> {
    match target {
        ResourceTarget::Pool(id) if id == inventory.pool_id => Some(inventory.pool_name.clone()),
        ResourceTarget::Host(id) => inventory.hosts.get(&id).map(|host| host.name.clone()),
        ResourceTarget::Storage(id) => inventory
            .storage
            .iter()
            .find(|sr| sr.uuid == id.uuid())
            .map(|sr| sr.name.clone()),
        ResourceTarget::Network(id) => inventory
            .networks
            .iter()
            .find(|network| network.uuid == id.uuid())
            .map(|network| network.name.clone()),
        _ => None,
    }
}
pub(super) fn selected_target(s: &State) -> Option<ResourceTarget> {
    let row = s.selected()?;
    match s.page {
        Page::Hosts => HostId::parse(&row.id).ok().map(ResourceTarget::Host),
        Page::Storage => SrId::parse(&row.id).ok().map(ResourceTarget::Storage),
        Page::Networks => xenadmin_core::NetworkId::parse(&row.id)
            .ok()
            .map(ResourceTarget::Network),
        _ => None,
    }
}
pub(super) fn open_selected(s: &mut State, kind: Kind) {
    if let Some(target) = selected_target(s) {
        open(s, target, kind);
    }
}
pub(super) fn open_pool(s: &mut State, kind: Kind) {
    if let Some(inventory) = &s.inventory {
        open(s, ResourceTarget::Pool(inventory.pool_id), kind);
    }
}
fn open(s: &mut State, target: ResourceTarget, kind: Kind) {
    let result = (|| {
        let inventory = s
            .inventory
            .as_ref()
            .ok_or("Current inventory is unavailable")?;
        let observed = if s.demo || kind == Kind::History {
            None
        } else {
            Some(
                s.snapshot
                    .as_ref()
                    .ok_or("The connection is not current")?
                    .observe_resource(target)
                    .map_err(|e| e.to_string())?,
            )
        };
        let (name, description) = metadata(inventory, target).unwrap_or_else(|| {
            (
                display_name(inventory, target).unwrap_or_else(|| target_label(target)),
                String::new(),
            )
        });
        if kind == Kind::Rename && metadata(inventory, target).is_none() {
            return Err(
                "The host did not supply the current name and description. Refresh before editing."
                    .into(),
            );
        }
        let expected_mtu = match target {
            ResourceTarget::Network(id) => inventory.details.networks.get(&id).and_then(|n| n.mtu),
            _ => None,
        };
        let expected_enabled = match target {
            ResourceTarget::Host(id) => inventory.hosts.get(&id).map(|h| h.enabled),
            _ => None,
        };
        let expected_default = inventory.details.pool.as_ref().and_then(|p| p.default_sr);
        if kind == Kind::DefaultStorage
            && !inventory
                .details
                .pool
                .as_ref()
                .is_some_and(|p| p.default_sr_known)
        {
            return Err(
                "The current default storage repository is unknown. Refresh before changing it."
                    .into(),
            );
        }
        Ok(Editor {
            kind,
            target,
            observed,
            pool: inventory.pool_id,
            label: name.clone(),
            expected_name: name.clone(),
            expected_description: description.clone(),
            expected_mtu,
            expected_enabled,
            expected_default,
            name: TextEdit::new(if kind == Kind::CreateNetwork {
                ""
            } else {
                &name
            }),
            description: TextEdit::new(if kind == Kind::CreateNetwork {
                ""
            } else {
                &description
            }),
            mtu: TextEdit::new(expected_mtu.unwrap_or(1500).to_string()),
            selected_sr: expected_default,
            review: None,
            error: None,
        })
    })();
    match result {
        Ok(editor) => {
            s.resource_editor = Some(editor);
            s.error = None;
        }
        Err(error) => {
            if let Some(editor) = &mut s.resource_editor {
                editor.error = Some(error);
            } else {
                s.error = Some(error);
            }
        }
    }
}
fn valid_name(value: &str) -> bool {
    !value.trim().is_empty() && value.chars().count() <= 255 && !value.chars().any(char::is_control)
}
fn review(s: &mut State) {
    let Some(editor) = &mut s.resource_editor else {
        return;
    };
    let result = (|| {
        let mut review = Review {
            name: editor.name.text.trim().into(),
            description: editor.description.text.clone(),
            mtu: 1500,
            sr: editor.selected_sr,
            lines: Vec::new(),
        };
        match editor.kind {
            Kind::Rename | Kind::CreateNetwork => {
                if !valid_name(&review.name) || review.description.chars().count() > 4096 || review.description.chars().any(|c| c.is_control() && c != '\n' && c != '\t') {
                    return Err("Use a name of 1–255 characters and a description of at most 4096 characters.".into());
                }
                if editor.kind == Kind::Rename && review.name == editor.expected_name && review.description == editor.expected_description {
                    return Err("No name or description changes to apply.".into());
                }
                review.lines.push(format!("Name: {}", review.name));
                review.lines.push(format!("Description: {}", if review.description.is_empty() { "(empty)" } else { &review.description }));
            }
            Kind::DefaultStorage => {
                let id = review.sr.ok_or("Choose a storage repository")?;
                if Some(id) == editor.expected_default { return Err("This repository is already the default.".into()); }
                let sr = s.inventory.as_ref().filter(|i| i.pool_id == editor.pool).and_then(|i| i.storage.iter().find(|sr| sr.uuid == id.uuid())).ok_or("The selected storage repository is no longer available")?;
                review.lines.push(format!("New default: {} ({id})", sr.name));
            }
            Kind::HostEnabled(enabled) => {
                if editor.expected_enabled.is_none() || editor.expected_enabled == Some(enabled) { return Err("The host's scheduling state changed or is unavailable.".into()); }
                review.lines.push(format!("Allow new VM placement on this host: {}", if enabled { "Yes" } else { "No" }));
                review.lines.push("This controls scheduling. Running guests are not evacuated or shut down.".into());
            }
            Kind::DeleteNetwork => review.lines.push("The internal network record will be deleted. The host must report no physical or VM interfaces on it.".into()),
            Kind::ScanStorage => review.lines.push("Ask the storage backend to refresh its disk inventory. The operation will appear in history.".into()),
            Kind::Acknowledge(_) => review.lines.push("This releases the pool's write guard without establishing the server outcome. Inspect the affected resources and any host task before continuing.".into()),
            Kind::Mtu => {}
            Kind::History => return Err("Choose an administration action first.".into()),
        }
        if matches!(editor.kind, Kind::CreateNetwork | Kind::Mtu) {
            review.mtu = editor
                .mtu
                .text
                .parse::<u64>()
                .ok()
                .filter(|n| (576..=65535).contains(n))
                .ok_or("Enter an MTU between 576 and 65535")?;
            if editor.kind == Kind::Mtu && editor.expected_mtu == Some(review.mtu) {
                return Err("The MTU is unchanged.".into());
            }
            if editor.kind == Kind::Mtu && editor.expected_mtu.is_none() {
                return Err("The current MTU is unknown. Refresh first.".into());
            }
            review.lines.push(format!("MTU: {}", review.mtu));
            if editor.kind == Kind::Mtu {
                review.lines.push("Changing MTU requires an unused internal network; attached physical and VM interfaces must be absent.".into());
            }
        }
        Ok::<_, String>(review)
    })();
    match result {
        Ok(review) => {
            editor.review = Some(review);
            editor.error = None;
        }
        Err(error) => editor.error = Some(error),
    }
}
fn current(s: &State, editor: &Editor) -> bool {
    s.can_mutate()
        && editor.observed.is_some()
        && editor.observed
            == s.snapshot
                .as_ref()
                .and_then(|snapshot| snapshot.observe_resource(editor.target).ok())
}
fn submit(s: &mut State) {
    let result = (|| {
        let editor = s
            .resource_editor
            .as_ref()
            .ok_or(xenadmin_core::Error::Stale)?;
        if !current(s, editor) {
            return Err(xenadmin_core::Error::Stale);
        }
        let target = editor.observed.ok_or(xenadmin_core::Error::Stale)?;
        let review = editor.review.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let connection = s.connection.as_ref().ok_or(xenadmin_core::Error::Closed)?;
        if let Kind::Acknowledge(id) = editor.kind {
            connection.control_resource(id, OperationControl::AcknowledgeUnknown)?;
            return Ok(None);
        }
        let command = match editor.kind {
            Kind::Rename => ResourceCommand::RenameMetadata {
                target,
                expected_name: editor.expected_name.clone(),
                expected_description: editor.expected_description.clone(),
                name: review.name.clone(),
                description: review.description.clone(),
            },
            Kind::CreateNetwork => ResourceCommand::CreateInternalNetwork {
                pool: target,
                name: review.name.clone(),
                description: review.description.clone(),
                mtu: review.mtu,
            },
            Kind::Mtu => ResourceCommand::ConfigureInternalNetwork {
                network: target,
                expected_mtu: editor.expected_mtu.ok_or(xenadmin_core::Error::Stale)?,
                mtu: review.mtu,
            },
            Kind::DeleteNetwork => ResourceCommand::DeleteInternalNetwork {
                network: target,
                expected_name: editor.expected_name.clone(),
            },
            Kind::ScanStorage => ResourceCommand::ScanStorage { sr: target },
            Kind::HostEnabled(enabled) => ResourceCommand::SetHostEnabled {
                host: target,
                expected_enabled: editor.expected_enabled.ok_or(xenadmin_core::Error::Stale)?,
                enabled,
            },
            Kind::DefaultStorage => ResourceCommand::SetDefaultStorage {
                pool: target,
                sr: review.sr.ok_or(xenadmin_core::Error::Stale)?,
                expected_default: editor.expected_default,
            },
            Kind::Acknowledge(_) | Kind::History => return Err(xenadmin_core::Error::Stale),
        };
        connection
            .submit_resource(OperationId::new(), command)
            .map(Some)
    })();
    match result {
        Ok(operation) => {
            if let Some(operation) = operation {
                if let Some(waker) = &s.waker {
                    operation.notify_on_change(waker.clone());
                }
                s.resource_operations.push(operation);
            }
            s.resource_editor = None;
            s.error = None;
        }
        Err(error) => {
            if let Some(editor) = &mut s.resource_editor {
                editor.error = Some(error.to_string());
            }
        }
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let mut rows: BTreeMap<_, _> = s
        .resource_rows
        .iter()
        .map(|record| (record.id, record.clone()))
        .collect();
    if let Some(connection) = &s.connection {
        for record in connection.resource_history().iter() {
            if rows
                .get(&record.id)
                .is_none_or(|old| record.supersedes(old))
            {
                rows.insert(record.id, record.clone());
            }
        }
    }
    for operation in &s.resource_operations {
        let record = operation.latest();
        if rows
            .get(&record.id)
            .is_none_or(|old| record.supersedes(old))
        {
            rows.insert(record.id, record.as_ref().clone());
        }
    }
    let mut rows: Vec<_> = rows.into_values().collect();
    rows.sort_by_key(|record| record.created_at_ms);
    let mut terminal_remaining = 100;
    rows.reverse();
    rows.retain(|record| {
        if !record.state.terminal() {
            return true;
        }
        if terminal_remaining == 0 {
            return false;
        }
        terminal_remaining -= 1;
        true
    });
    rows.reverse();
    if rows == s.resource_rows {
        false
    } else {
        s.resource_rows = rows;
        true
    }
}
pub(super) fn buttons(s: &State, k: &Kit) -> El {
    let mut buttons = col().gap(8.0);
    let Some(target) = selected_target(s) else {
        return buttons;
    };
    let mut kinds = vec![Kind::Rename];
    let network_used = match target {
        ResourceTarget::Network(id) => s.inventory.as_ref().is_some_and(|inventory| {
            inventory
                .details
                .pifs
                .values()
                .any(|interface| interface.network == Some(id))
                || inventory
                    .details
                    .vifs
                    .values()
                    .any(|interface| interface.network == Some(id))
        }),
        _ => false,
    };
    match target {
        ResourceTarget::Host(id) => {
            if let Some(host) = s.inventory.as_ref().and_then(|i| i.hosts.get(&id)) {
                kinds.push(Kind::HostEnabled(!host.enabled));
            }
        }
        ResourceTarget::Storage(_) => kinds.push(Kind::ScanStorage),
        ResourceTarget::Network(_) => kinds.extend([Kind::Mtu, Kind::DeleteNetwork]),
        _ => {}
    }
    for kind in kinds {
        buttons = buttons.child(
            Button::new(kind.title())
                .variant(if kind.danger() {
                    ButtonVariant::Ghost
                } else {
                    ButtonVariant::Outline
                })
                .disabled(
                    (!s.demo && !s.can_mutate())
                        || (network_used && matches!(kind, Kind::Mtu | Kind::DeleteNetwork)),
                )
                .on_press(move |s: &mut State| open_selected(s, kind))
                .build(k)
                .key(format!("resource-{}", kind.title())),
        );
    }
    if network_used {
        buttons = buttons.child(caption("MTU changes and deletion require an unused internal network. This network has host or VM interfaces.", k).wrap());
    }
    if let Some(inventory) = &s.inventory {
        buttons = buttons.child(history(s, target, inventory.pool_id, k));
    }
    buttons
}
fn history(s: &State, target: ResourceTarget, pool: PoolId, k: &Kit) -> El {
    let mut view = col().gap(8.0);
    let can_control = s.can_mutate() && s.inventory.as_ref().is_some_and(|i| i.pool_id == pool);
    for record in s
        .resource_rows
        .iter()
        .rev()
        .filter(|r| {
            r.pool == pool && (r.target == target || matches!(target, ResourceTarget::Pool(_)))
        })
        .take(12)
    {
        view = view
            .child(heading(
                format!("{} · {:?}", record.action.label(), record.state),
                k,
            ))
            .child(caption(target_label(record.target), k).wrap_anywhere())
            .child(caption(&record.detail, k).wrap());
        if let Some(error) = &record.local_error {
            view = view.child(caption(error, k).wrap().color(k.c.danger.icon));
        }
        if let Some(created) = record.created {
            view = view
                .child(caption(format!("Created {}", target_label(created)), k).wrap_anywhere());
        }
        for step in &record.steps {
            view = view.child(
                caption(
                    format!("{} · {:?}: {}", step.label, step.state, step.detail),
                    k,
                )
                .wrap(),
            );
        }
        let id = record.id;
        if record.state == OperationState::OutcomeUnknown {
            view = view.child(
                Button::new("Recheck outcome")
                    .disabled(!can_control)
                    .on_press(move |s: &mut State| control(s, id, OperationControl::Recheck))
                    .build(k),
            );
            view = view.child(
                Button::new("Acknowledge unknown outcome…")
                    .variant(ButtonVariant::Ghost)
                    .disabled(!can_control)
                    .on_press(move |s: &mut State| open_pool(s, Kind::Acknowledge(id)))
                    .build(k),
            );
        } else if record.can_cancel && !record.state.terminal() {
            view = view.child(
                Button::new("Request cancellation")
                    .variant(ButtonVariant::Ghost)
                    .disabled(!can_control)
                    .on_press(move |s: &mut State| {
                        control(s, id, OperationControl::RequestCancellation)
                    })
                    .build(k),
            );
        }
    }
    view
}
fn control(s: &mut State, id: OperationId, control: OperationControl) {
    s.error = s
        .connection
        .as_ref()
        .ok_or(xenadmin_core::Error::Closed)
        .and_then(|connection| connection.control_resource(id, control))
        .err()
        .map(|error| error.to_string());
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(editor) = &s.resource_editor else {
        return el();
    };
    let mut body = col()
        .gap(12.0)
        .child(heading(editor.kind.title(), k))
        .child(
            caption(
                format!(
                    "{}\n{}{}",
                    editor.label,
                    target_label(editor.target),
                    if matches!(editor.target, ResourceTarget::Pool(_)) {
                        String::new()
                    } else {
                        format!("\nPool {}", editor.pool)
                    }
                ),
                k,
            )
            .wrap_anywhere(),
        );
    if editor.kind == Kind::History {
        let same_pool = s
            .inventory
            .as_ref()
            .is_some_and(|i| i.pool_id == editor.pool);
        body = body
            .child(
                Button::new("Edit pool name and description")
                    .disabled(!same_pool || (!s.demo && !s.can_mutate()))
                    .on_press(|s: &mut State| open_pool(s, Kind::Rename))
                    .build(k),
            )
            .child(
                Button::new("Change default storage…")
                    .disabled(!same_pool || (!s.demo && !s.can_mutate()))
                    .on_press(|s: &mut State| open_pool(s, Kind::DefaultStorage))
                    .build(k),
            );
        if !s
            .resource_rows
            .iter()
            .any(|record| record.pool == editor.pool)
        {
            body = body
                .child(caption("No administration operations recorded for this pool.", k).wrap());
        }
    } else if let Some(review) = &editor.review {
        for line in &review.lines {
            body = body.child(caption(line, k).wrap());
        }
    } else {
        if matches!(editor.kind, Kind::Rename | Kind::CreateNetwork) {
            body = body.child(
                TextInput::new(&editor.name)
                    .label("Name")
                    .on_edit(|s: &mut State| &mut s.resource_editor.as_mut().unwrap().name)
                    .build(k)
                    .key("resource-name"),
            );
            body = body.child(
                TextInput::new(&editor.description)
                    .label("Description")
                    .on_edit(|s: &mut State| &mut s.resource_editor.as_mut().unwrap().description)
                    .build(k),
            );
        }
        if matches!(editor.kind, Kind::CreateNetwork | Kind::Mtu) {
            body = body.child(
                TextInput::new(&editor.mtu)
                    .label("MTU")
                    .on_edit(|s: &mut State| &mut s.resource_editor.as_mut().unwrap().mtu)
                    .build(k)
                    .key("resource-mtu"),
            );
        }
        if editor.kind == Kind::DefaultStorage {
            if let Some(inventory) = &s.inventory {
                for sr in &inventory.storage {
                    let id = SrId::from(sr.uuid);
                    if inventory
                        .details
                        .storage
                        .get(&id)
                        .is_none_or(|details| details.content_type != "user")
                    {
                        continue;
                    }
                    body = body.child(
                        Button::new(format!(
                            "{}{} · {}",
                            if editor.selected_sr == Some(id) {
                                "✓ "
                            } else {
                                ""
                            },
                            sr.name,
                            id
                        ))
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            if let Some(editor) = &mut s.resource_editor {
                                editor.selected_sr = Some(id);
                            }
                        })
                        .build(k),
                    );
                }
            }
        }
    }
    if editor.kind != Kind::History && !s.demo && !current(s, editor) {
        body = body.child(caption("The connection or resource changed. Close and reopen this review before applying it.", k).wrap());
    }
    if s.demo {
        body =
            body.child(caption("Synthetic preview · host changes cannot be submitted.", k).wrap());
    }
    if let Some(error) = &editor.error {
        body = body.child(
            caption(error, k)
                .wrap()
                .role("alert")
                .color(k.c.danger.icon),
        );
    }
    if matches!(editor.target, ResourceTarget::Pool(_)) {
        body = body.child(history(s, editor.target, editor.pool, k));
    }
    let mut controls = row().gap(8.0).justify_end().child(
        Button::new(if editor.kind == Kind::History {
            "Close"
        } else {
            "Cancel"
        })
        .variant(ButtonVariant::Ghost)
        .on_press(|s: &mut State| s.resource_editor = None)
        .build(k),
    );
    if editor.review.is_some() {
        controls = controls
            .child(
                Button::new("Back")
                    .variant(ButtonVariant::Ghost)
                    .on_press(|s: &mut State| {
                        if let Some(editor) = &mut s.resource_editor {
                            editor.review = None;
                        }
                    })
                    .build(k),
            )
            .child(
                Button::new("Confirm")
                    .variant(if editor.kind.danger() {
                        ButtonVariant::Danger
                    } else {
                        ButtonVariant::Outline
                    })
                    .disabled(!current(s, editor))
                    .on_press(submit)
                    .build(k)
                    .key("resource-confirm"),
            );
    } else if editor.kind != Kind::History {
        controls = controls.child(
            Button::new("Review")
                .variant(ButtonVariant::Outline)
                .on_press(review)
                .build(k)
                .key("resource-review"),
        );
    }
    let width = (s.window_width - 32.0).clamp(280.0, 620.0);
    let height = (s.window_height - 56.0).clamp(280.0, 780.0);
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label(editor.kind.title())
        .modal(false, |s: &mut State| s.resource_editor = None)
        .child(
            col()
                .w(width)
                .h(height)
                .padding(20.0)
                .gap(16.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(controls),
        )
}

pub(super) fn notice(s: &State, k: &Kit) -> Option<El> {
    let pool = s.inventory.as_ref()?.pool_id;
    let record = s
        .resource_rows
        .iter()
        .rev()
        .find(|record| record.pool == pool)?;
    Some(
        Button::new(format!(
            "{} · {:?} · View operation history",
            record.action.label(),
            record.state
        ))
        .variant(ButtonVariant::Ghost)
        .on_press(|s: &mut State| open_pool(s, Kind::History))
        .build(k)
        .key("resource-operation-notice"),
    )
}

pub(super) fn scene(s: &mut State, pool: bool) {
    crate::demo(s);
    s.page = Page::Networks;
    s.rebuild();
    if pool {
        open_pool(s, Kind::History);
    } else {
        open_pool(s, Kind::CreateNetwork);
        if let Some(editor) = &mut s.resource_editor {
            editor.name = TextEdit::new("Isolated lab network");
            editor.description = TextEdit::new("Synthetic preview");
        }
        review(s);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn state() -> State {
        let mut state = crate::init();
        crate::demo(&mut state);
        state
    }
    #[test]
    fn network_review_captures_pool_name_mtu_and_never_submits_from_preview() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut state = state();
            open_pool(&mut state, Kind::CreateNetwork);
            let editor = state.resource_editor.as_mut().unwrap();
            editor.name = TextEdit::new("Isolated test network");
            editor.description = TextEdit::new("Synthetic network");
            editor.mtu = TextEdit::new("9000");
            let pool = editor.pool;
            let mut runtime = rune::Runtime::new(
                state,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let (x, y) = runtime.find("resource-review").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            let editor = runtime.state.resource_editor.as_ref().unwrap();
            let reviewed = editor.review.as_ref().unwrap();
            assert_eq!(reviewed.name, "Isolated test network");
            assert_eq!(reviewed.mtu, 9000);
            assert_eq!(editor.target, ResourceTarget::Pool(pool));
            assert!(!current(&runtime.state, editor));
            let (x, y) = runtime.find("resource-confirm").unwrap();
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
            assert!(runtime.state.resource_editor.is_some());
        }
    }
    #[test]
    fn resource_review_keeps_original_selection_and_rejects_invalid_or_unchanged_values() {
        let mut state = state();
        state.page = Page::Networks;
        state.rebuild();
        state.table.select(state.rows[0].id.clone());
        open_selected(&mut state, Kind::Rename);
        review(&mut state);
        assert!(state.resource_editor.as_ref().unwrap().error.is_some());
        let target = state.resource_editor.as_ref().unwrap().target;
        state.resource_editor.as_mut().unwrap().name = TextEdit::new("Updated network");
        review(&mut state);
        assert!(state.resource_editor.as_ref().unwrap().review.is_some());
        state.page = Page::Hosts;
        state.rebuild();
        state.table.select(state.rows[0].id.clone());
        assert_eq!(state.resource_editor.as_ref().unwrap().target, target);
        open_pool(&mut state, Kind::CreateNetwork);
        state.resource_editor.as_mut().unwrap().name = TextEdit::new("Network");
        for value in ["", "0", "1500.5", "65536", "text"] {
            state.resource_editor.as_mut().unwrap().mtu = TextEdit::new(value);
            review(&mut state);
            assert!(state.resource_editor.as_ref().unwrap().review.is_none());
        }
    }
    #[test]
    fn used_network_explains_disabled_mtu_and_delete_and_offline_pool_history_opens() {
        let mut state = state();
        state.page = Page::Networks;
        state.rebuild();
        state.table.select(state.rows[0].id.clone());
        let mut runtime = rune::Runtime::new(
            state,
            crate::view,
            (1440.0, 1100.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..30 {
            runtime.frame(1.0 / 60.0);
        }
        assert!(
            runtime
                .accessibility_json()
                .to_string()
                .contains("This network has host or VM interfaces")
        );
        let (x, y) = runtime.find("resource-Delete internal network").unwrap();
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.resource_editor.is_none());
        runtime.state.demo = false;
        open_pool(&mut runtime.state, Kind::History);
        runtime.frame(0.0);
        assert!(runtime.state.resource_editor.is_some());
        assert!(runtime.find("resource-review").is_none());
        assert!(runtime.find("resource-confirm").is_none());
    }

    #[test]
    fn compact_network_controls_remain_inside_the_window() {
        for size in [(480.0, 640.0), (513.0, 652.0)] {
            let mut state = state();
            state.page = Page::Networks;
            state.rebuild();
            let mut runtime = rune::Runtime::new(
                state,
                crate::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            for key in [
                "manage-pool",
                "create-network",
                "add-connection",
                "show-details",
                "disconnect",
            ] {
                let (x, y) = runtime.find(key).unwrap();
                assert!(
                    x > 0.0 && x < size.0 && y > 0.0 && y < size.1,
                    "{key} is outside {size:?}: {x}, {y}"
                );
            }
        }
    }
}
