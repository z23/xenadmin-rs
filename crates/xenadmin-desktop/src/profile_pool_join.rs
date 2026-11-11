use super::*;
use xenadmin_core::{ObservedResource, OperationId, PoolJoinSelection, ResourceTarget};

#[derive(Clone)]
struct Captured {
    profile: ConnectionProfile,
    observed: Option<ObservedResource>,
    pool: PoolId,
    name: String,
    host: HostId,
    host_name: String,
    address: String,
}
impl Captured {
    fn capture(s: &State, id: ProfileId, standalone: bool) -> Option<Self> {
        let connection = s.workspace.active.get(&id)?;
        if connection.is_read_only() {
            return None;
        }
        let snapshot = connection.latest();
        let inventory = snapshot.inventory()?;
        if standalone && inventory.hosts.len() != 1 {
            return None;
        }
        let host = inventory.hosts.get(&inventory.coordinator?)?;
        Some(Self {
            profile: s.workspace.profile(id)?.clone(),
            observed: Some(
                snapshot
                    .observe_resource(ResourceTarget::Pool(inventory.pool_id))
                    .ok()?,
            ),
            pool: inventory.pool_id,
            name: inventory.pool_name.clone(),
            host: host.id,
            host_name: host.name.clone(),
            address: host.address.clone(),
        })
    }
    fn current(&self, s: &State) -> bool {
        s.workspace.profile(self.profile.id) == Some(&self.profile)
            && s.workspace.active.get(&self.profile.id).is_some_and(|c| {
                !c.is_read_only()
                    && self.observed.is_some()
                    && c.latest()
                        .observe_resource(ResourceTarget::Pool(self.pool))
                        .ok()
                        == self.observed
            })
    }
}
pub(super) struct Editor {
    source: Captured,
    destinations: Vec<Captured>,
    destination: Option<usize>,
    review: bool,
    username: TextEdit,
    password: TextEdit,
    confirmation: TextEdit,
    error: Option<String>,
}
pub(super) fn open(s: &mut State) {
    let result = (|| {
        let pool = s
            .inventory
            .as_ref()
            .ok_or("Connect the standalone source host first")?
            .pool_id;
        let source = s
            .workspace
            .active
            .keys()
            .find_map(|id| Captured::capture(s, *id, true).filter(|v| v.pool == pool))
            .ok_or("Joining requires a writable, current connection to a standalone source host")?;
        let mut pools = std::collections::BTreeSet::new();
        let destinations = s
            .workspace
            .active
            .keys()
            .filter_map(|id| Captured::capture(s, *id, false))
            .filter(|c| c.pool != pool && pools.insert(c.pool))
            .take(128)
            .collect();
        Ok::<_, String>(Editor {
            source,
            destinations,
            destination: None,
            review: false,
            username: TextEdit::new("root"),
            password: secure_field(),
            confirmation: TextEdit::new(""),
            error: None,
        })
    })();
    match result {
        Ok(editor) => {
            s.workspace.join = Some(editor);
            s.workspace.reconnect = None;
            s.workspace.show_manager = true;
            s.pool_editor = None;
            s.pool_security = None;
        }
        Err(error) => s.error = Some(error),
    }
}
fn choose(s: &mut State, index: usize) {
    if let Some(e) = &mut s.workspace.join
        && index < e.destinations.len()
    {
        e.destination = Some(index);
        e.review = true;
        e.password = secure_field();
        e.username = TextEdit::new("root");
        e.confirmation = TextEdit::new("");
        e.error = None;
    }
}
fn current(s: &State, e: &Editor) -> bool {
    e.source.current(s)
        && e.destination
            .and_then(|i| e.destinations.get(i))
            .is_some_and(|d| d.current(s))
}
pub(super) fn poll(s: &mut State) -> bool {
    if s.demo {
        return false;
    }
    let stale = s
        .workspace
        .join
        .as_ref()
        .is_some_and(|e| e.review && !current(s, e));
    if stale && let Some(e) = &mut s.workspace.join {
        let changed = !e.password.text.is_empty() || e.error.is_none();
        e.password = secure_field();
        e.error=Some("Connection settings or pool identity changed. Close this review and reconnect both pools.".into());
        return changed;
    }
    false
}
fn apply(s: &mut State) {
    let result = (|| {
        let e = s.workspace.join.as_ref().ok_or("Join review is closed")?;
        if s.demo || !s.workspace.writable || !current(s, e) {
            return Err(
                "Connection settings or pool identity changed; reopen the join review".into(),
            );
        }
        let destination = e
            .destinations
            .get(e.destination.ok_or("Choose a destination pool")?)
            .ok_or("Choose a destination pool")?;
        if e.confirmation.text != e.source.host.to_string() {
            return Err("Enter the source host UUID exactly".into());
        }
        let selected = PoolJoinSelection {
            source: e.source.observed.ok_or("Source connection is stale")?,
            source_host: e.source.host,
            destination: destination
                .observed
                .ok_or("Destination connection is stale")?,
            destination_coordinator: destination.host,
            destination_address: destination.address.clone(),
        };
        let source_id = e.source.profile.id;
        let destination_id = destination.profile.id;
        let username = e.username.text.trim().to_string();
        let password = std::mem::take(&mut s.workspace.join.as_mut().unwrap().password.text);
        let source = s
            .workspace
            .active
            .get(&source_id)
            .ok_or("Source connection was closed")?;
        let destination = s
            .workspace
            .active
            .get(&destination_id)
            .ok_or("Destination connection was closed")?;
        source
            .join_pool(
                OperationId::new(),
                selected,
                destination,
                Credentials::new(username, password),
            )
            .map_err(|e| e.to_string())
    })();
    match result {
        Ok(handle) => {
            if let Some(waker) = &s.waker {
                handle.notify_on_change(waker.clone());
            }
            s.resource_operations.push(handle);
            s.workspace.join = None;
            s.workspace.show_manager = false;
            resource_admin::open_pool(s, resource_admin::Kind::History);
        }
        Err(error) => {
            if let Some(e) = &mut s.workspace.join {
                e.password = secure_field();
                e.error = Some(error);
            }
        }
    }
}
fn close(s: &mut State) {
    s.workspace.join = None;
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let e = s.workspace.join.as_ref().unwrap();
    let mut body = col()
        .gap(12.0)
        .child(heading("Join another pool", k))
        .child(
            caption(
                format!(
                    "Source: {}\nHost: {} ({})\nOriginal pool: {}",
                    e.source.name, e.source.host_name, e.source.host, e.source.pool
                ),
                k,
            )
            .wrap_anywhere(),
        );
    if let Some(d) = e
        .destination
        .and_then(|i| e.destinations.get(i))
        .filter(|_| e.review)
    {
        body=body.child(heading("Review destination",k))
            .child(caption(format!("{}\nPool: {}\nCoordinator: {} ({})\nHost-to-host address: {}\nDesktop connection: {}",d.name,d.pool,d.host_name,d.host,d.address,d.profile.endpoint),k).wrap_anywhere())
            .child(caption("The standalone host and its halted guests move into this pool. Both pools must already have HA disabled; source guests must be halted, with no shared guest storage, bonds or tunnels. These prerequisites are checked again; this action does not prepare them automatically.",k).wrap())
            .child(caption("Destination credentials are sent to the source host for XAPI's pool join. XAPI initially bootstraps the host-to-host connection without certificate validation before exchanging pool certificates. Desktop certificate trust does not secure that bootstrap. Proceed only on a trusted management network.",k).wrap().color(k.c.danger.icon))
            .child(caption("The destination account is checked using its existing desktop connection's trust settings first. Credentials are used only for this join and are never saved. The old source connection remains bound to its original pool; a disconnect or lost reply may require rechecking through the destination pool.",k).wrap())
            .child(caption("Destination local administrator",k))
            .child(TextInput::new(&e.username).label("Destination local administrator").on_edit(|s:&mut State|&mut s.workspace.join.as_mut().unwrap().username).build(k).key("join-username"))
            .child(caption("Destination password · this join only",k))
            .child(TextInput::new(&e.password).secure().label("Destination password · this join only").placeholder("Password").on_edit(|s:&mut State|&mut s.workspace.join.as_mut().unwrap().password).build(k).key("join-password"))
            .child(caption(format!("Type source host UUID to confirm: {}",e.source.host),k).wrap_anywhere())
            .child(TextInput::new(&e.confirmation).label("Source host UUID").on_edit(|s:&mut State|&mut s.workspace.join.as_mut().unwrap().confirmation).build(k).key("join-confirmation"));
    } else {
        body=body.child(caption("Choose an independently connected destination pool. Both connections must be writable. Only a standalone source host can join.",k).wrap());
        if e.destinations.is_empty() {
            body = body.child(
                caption(
                    "Connect the destination pool first, then reopen this view.",
                    k,
                )
                .wrap(),
            );
        }
        for (index, d) in e.destinations.iter().enumerate() {
            body = body.child(
                Button::new(format!("{} · {}", d.name, d.pool))
                    .disabled(!d.current(s))
                    .on_press(move |s: &mut State| choose(s, index))
                    .build(k)
                    .key(format!("join-destination-{index}")),
            );
        }
    }
    if let Some(error) = &e.error {
        body = body.child(caption(error, k).wrap().role("alert"));
    }
    let mut footer = row().gap(8.0).justify_end().child(
        Button::new("Cancel")
            .variant(ButtonVariant::Outline)
            .on_press(close)
            .build(k),
    );
    if e.review {
        footer = footer
            .child(
                Button::new("Back")
                    .variant(ButtonVariant::Outline)
                    .on_press(|s: &mut State| {
                        let e = s.workspace.join.as_mut().unwrap();
                        e.review = false;
                        e.destination = None;
                        e.password = secure_field();
                        e.confirmation = TextEdit::new("");
                    })
                    .build(k),
            )
            .child(
                Button::new("Join pool")
                    .variant(ButtonVariant::Danger)
                    .disabled(
                        s.demo
                            || !s.workspace.writable
                            || !current(s, e)
                            || e.username.text.trim().is_empty()
                            || e.password.text.is_empty()
                            || e.confirmation.text != e.source.host.to_string(),
                    )
                    .on_press(apply)
                    .build(k)
                    .key("join-submit"),
            );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Join another pool")
        .modal(false, close)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 760.0))
                .h((s.window_height - 56.0).clamp(280.0, 800.0))
                .padding(20.0)
                .gap(16.0)
                .fill(k.c.bg_base)
                .radius(10.0)
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}
#[cfg(any(test, debug_assertions))]
pub(super) fn scene(s: &mut State) {
    let make = |name: &str, pool: &str, host: &str, address: &str| Captured {
        profile: ConnectionProfile::new(
            name.into(),
            Endpoint::parse(address).unwrap(),
            "root".into(),
        ),
        observed: None,
        pool: PoolId::parse(pool).unwrap(),
        name: name.into(),
        host: HostId::parse(host).unwrap(),
        host_name: format!("{name} host"),
        address: address.into(),
    };
    s.workspace.join = Some(Editor {
        source: make(
            "Standalone lab",
            "11111111-1111-1111-1111-111111111111",
            "22222222-2222-2222-2222-222222222222",
            "192.0.2.10",
        ),
        destinations: vec![make(
            "Destination lab",
            "aaaaaaaa-aaaa-aaaa-aaaa-aaaaaaaaaaaa",
            "bbbbbbbb-bbbb-bbbb-bbbb-bbbbbbbbbbbb",
            "192.0.2.20",
        )],
        destination: Some(0),
        review: true,
        username: TextEdit::new("root"),
        password: secure_field(),
        confirmation: TextEdit::new(""),
        error: None,
    });
    s.demo = true;
    s.workspace.show_manager = true;
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn compact_join_review_keeps_footer_reachable_and_never_dispatches_demo() {
        for size in [(513.0, 652.0), (1440.0, 900.0)] {
            let mut s = crate::init();
            scene(&mut s);
            let mut runtime = rune::Runtime::new(
                s,
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
            let (x, y) = runtime.find("join-submit").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert!(runtime.state.resource_operations.is_empty());
            runtime.update(|_, tx| tx.scroll_into_view("join-password"));
            for _ in 0..30 {
                runtime.frame(1.0 / 60.0);
            }
            let (x, y) = runtime.find("join-password").unwrap();
            assert!(x > 0.0 && x < size.0 && y > 0.0 && y < size.1);
        }
    }
    #[test]
    fn destination_change_and_stale_connection_clear_transient_credentials() {
        let mut s = crate::init();
        scene(&mut s);
        let e = s.workspace.join.as_mut().unwrap();
        e.password.text = "synthetic-password".into();
        e.confirmation.text = e.source.host.to_string();
        choose(&mut s, 0);
        let e = s.workspace.join.as_ref().unwrap();
        assert!(e.password.text.is_empty());
        assert!(e.confirmation.text.is_empty());
        s.workspace.join.as_mut().unwrap().password.text = "synthetic-password".into();
        s.demo = false;
        assert!(poll(&mut s));
        assert!(s.workspace.join.as_ref().unwrap().password.text.is_empty());
        assert!(!current(&s, s.workspace.join.as_ref().unwrap()));
        apply(&mut s);
        assert!(s.resource_operations.is_empty());
    }
}
