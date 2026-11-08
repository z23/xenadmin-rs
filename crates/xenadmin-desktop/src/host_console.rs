use super::*;
use xenadmin_core::ResourceTarget;

pub(super) fn button(s: &State, k: &Kit) -> El {
    let Some(ResourceTarget::Host(host)) = resource_admin::selected_target(s) else {
        return el();
    };
    let available = s
        .inventory
        .as_ref()
        .and_then(|i| control_domain(i, host))
        .is_some();
    Button::new("Open host text console")
        .disabled(s.demo || !available || !s.can_mutate())
        .on_press(move |s: &mut State| open(s, host))
        .build(k)
        .key("open-host-console")
}
fn control_domain(inventory: &Inventory, host: HostId) -> Option<VmId> {
    let id = inventory.details.hosts.get(&host)?.control_domain?;
    inventory
        .vms
        .get(&id)
        .filter(|v| {
            v.is_control_domain
                && !v.is_template
                && v.host == Some(host)
                && v.power_state == PowerState::Running
        })
        .map(|v| v.id)
}
fn open(s: &mut State, host: HostId) {
    let result = (|| {
        let snapshot = s.snapshot.as_ref().ok_or(xenadmin_core::Error::Stale)?;
        let observed = snapshot.observe_resource(ResourceTarget::Host(host))?;
        let inventory = snapshot.inventory().ok_or(xenadmin_core::Error::Stale)?;
        let vm = control_domain(inventory, host).ok_or(xenadmin_core::Error::NoConsole)?;
        let name = inventory
            .hosts
            .get(&host)
            .ok_or(xenadmin_core::Error::Stale)?
            .name
            .clone();
        let pool = inventory.pool_id;
        let console = s
            .connection
            .as_ref()
            .ok_or(xenadmin_core::Error::Closed)?
            .open_host_console(observed, vm)?;
        if let Some(waker) = &s.waker {
            console.notify_on_change(waker.clone());
        }
        console_view::opened_host(s, pool, host, vm, name);
        s.metrics = None;
        s.snapshot_view = None;
        s.devices = None;
        s.console = Some(console);
        s.console_snapshot = None;
        Ok::<_, xenadmin_core::Error>(())
    })();
    s.error = result.err().map(|error| error.to_string());
}
pub(super) fn selected(s: &State, host: HostId) -> Option<DisplayRow> {
    let target = s.console_ui.target()?;
    let inventory = s.inventory.as_ref().filter(|i| i.pool_id == target.pool)?;
    let host = inventory.hosts.get(&host)?;
    Some(DisplayRow {
        id: host.id.to_string(),
        name: host.name.clone(),
        state: if host.enabled { "Enabled" } else { "Disabled" }.into(),
        location: host.address.clone(),
        cpu: "—".into(),
        memory: "—".into(),
        description: format!("XCP-ng {}", host.version),
        vm: None,
    })
}
pub(super) fn scene(s: &mut State) {
    crate::demo(s);
    let inventory = Arc::make_mut(s.inventory.as_mut().unwrap());
    let host = inventory.hosts.values().next().unwrap().clone();
    let mut vm = inventory
        .vms
        .values()
        .find(|v| v.power_state == PowerState::Running)
        .unwrap()
        .clone();
    vm.id = VmId::from(uuid::Uuid::from_u128(900));
    vm.is_control_domain = true;
    vm.host = Some(host.id);
    inventory
        .details
        .hosts
        .entry(host.id)
        .or_default()
        .control_domain = Some(vm.id);
    let id = vm.id;
    inventory.vms.insert(id, vm);
    let pool = inventory.pool_id;
    console_view::opened_host(s, pool, host.id, id, host.name);
    s.page = Page::Hosts;
    s.filter_rows();
    s.table.select(host.id.to_string());
    s.show_details = false;
    s.console_snapshot = Some(Arc::new(xenadmin_core::ConsoleSnapshot {
        state: xenadmin_core::ConsoleState::Closed,
        frame: None,
        cursor: None,
        guest_clipboard: None,
        clipboard_utf8: false,
        extended_keys: false,
    }));
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn host_console_keeps_host_details_and_never_offers_guest_power_or_device_actions() {
        for size in [(1440.0, 900.0), (513.0, 652.0)] {
            let mut s = init();
            scene(&mut s);
            let target = s.console_ui.target().unwrap().clone();
            let host = target.host.unwrap();
            s.page = Page::Guests;
            s.filter_rows();
            s.table.select(s.rows[0].id.clone());
            assert_eq!(s.selected().unwrap().id, host.to_string());
            assert!(s.selected().unwrap().vm.is_none());
            assert_eq!(
                resource_admin::selected_target(&s),
                Some(ResourceTarget::Host(host))
            );
            let mut rt = rune::Runtime::new(
                s,
                super::super::view,
                size,
                rune::Env {
                    system_dark: true,
                    scale: 1.0,
                },
            );
            rt.frame(0.0);
            assert!(rt.find("open-console").is_none());
            assert!(rt.find("open-properties").is_none());
            assert!(rt.find("open-snapshots").is_none());
            assert_eq!(rt.state.console_ui.target().unwrap().vm, target.vm);
            let inventory = Arc::make_mut(rt.state.inventory.as_mut().unwrap());
            inventory
                .details
                .hosts
                .get_mut(&host)
                .unwrap()
                .control_domain = None;
            assert_eq!(control_domain(inventory, host), None);
        }
    }
}
