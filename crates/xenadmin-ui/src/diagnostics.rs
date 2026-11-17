//! Power availability checks for one VM: the handles and what they were
//! asked about.
//!
//! This is `ActionDiagnostics` of the Rune `main.rs`, carried over unchanged.
//! The model keeps it (`Model::diagnostics`), drops it when it is no longer
//! current and detects new results in `poll`; the `power` area shows it.

use xenadmin_core::{
    ActionAvailability, ActionDiagnostic, ConnectionSnapshot, HostId, Inventory, PoolId,
    PowerAction, PowerState, RunningOperation, VmId,
};

pub struct ActionDiagnostics {
    pub context: ActionDiagnosticContext,
    pub handles: Vec<(PowerAction, ActionDiagnostic)>,
}
#[derive(Debug, PartialEq, Eq)]
pub struct ActionDiagnosticContext {
    pub pool: PoolId,
    pub generation: u64,
    pub vm: VmId,
    pub power_state: PowerState,
    pub host: Option<HostId>,
    pub is_guest: bool,
    pub allowed_actions: Vec<PowerAction>,
    pub current_operations: Vec<RunningOperation>,
    pub availability: Option<Vec<ActionAvailability>>,
}
impl ActionDiagnosticContext {
    pub fn capture(
        current: bool,
        generation: u64,
        inventory: &Inventory,
        vm: VmId,
    ) -> Option<Self> {
        if !current {
            return None;
        }
        let vm = inventory.vms.get(&vm)?;
        Some(Self {
            pool: inventory.pool_id,
            generation,
            vm: vm.id,
            power_state: vm.power_state.clone(),
            host: vm.host,
            is_guest: !vm.is_template && !vm.is_control_domain,
            allowed_actions: vm.allowed_actions.clone(),
            current_operations: inventory
                .details
                .vms
                .get(&vm.id)
                .map(|details| details.current_operations.clone())
                .unwrap_or_default(),
            availability: inventory.details.vms.get(&vm.id).map(|details| {
                PowerAction::ALL
                    .into_iter()
                    .map(|action| details.power_availability(action, true))
                    .collect()
            }),
        })
    }
}
impl ActionDiagnostics {
    pub fn is_current(&self, snapshot: Option<&ConnectionSnapshot>) -> bool {
        snapshot
            .and_then(|snapshot| {
                ActionDiagnosticContext::capture(
                    snapshot.is_current(),
                    snapshot.generation(),
                    snapshot.inventory()?,
                    self.context.vm,
                )
            })
            .as_ref()
            == Some(&self.context)
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn demo_inventory() -> Inventory {
        let mut inventory = crate::demonstration(3, 48);
        crate::populate_demo_details(&mut inventory);
        inventory
    }

    // Rune: main.rs, the same name.
    #[test]
    fn action_checks_follow_vm_availability_without_expiring_on_unrelated_events() {
        let mut inventory = demo_inventory();
        let vm = *inventory.vms.keys().next().unwrap();
        let original = ActionDiagnosticContext::capture(true, 7, &inventory, vm);

        inventory.revision += 1;
        inventory.pool_name = "Renamed pool".into();
        inventory.vms.get_mut(&vm).unwrap().name = "Renamed guest".into();
        let other = *inventory.vms.keys().find(|id| **id != vm).unwrap();
        inventory
            .vms
            .get_mut(&other)
            .unwrap()
            .allowed_actions
            .clear();
        assert_eq!(
            original,
            ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            "unrelated events and display names do not invalidate the check"
        );

        let mut blocked = inventory.clone();
        blocked
            .details
            .vms
            .entry(vm)
            .or_default()
            .blocked_operations
            .insert("clean_shutdown".into(), "Protected by operator".into());
        let blocked_context = ActionDiagnosticContext::capture(true, 7, &blocked, vm);
        assert_ne!(
            original, blocked_context,
            "a newly added block expires Available"
        );
        assert_ne!(
            blocked_context,
            ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            "removing the block expires Blocked"
        );

        inventory.vms.get_mut(&vm).unwrap().allowed_actions = PowerAction::ALL.to_vec();
        let before_task = ActionDiagnosticContext::capture(true, 7, &inventory, vm);
        inventory
            .details
            .vms
            .entry(vm)
            .or_default()
            .current_operations
            .push(RunningOperation {
                task: None,
                operation: "snapshot".into(),
            });
        assert_ne!(
            before_task,
            ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            "starting a task expires earlier checks even if allowed actions have not updated"
        );
    }

    // Rune: main.rs, the same name.
    #[test]
    fn action_checks_require_the_same_current_connection_and_vm_state() {
        let mut inventory = demo_inventory();
        let vm = *inventory.vms.keys().next().unwrap();
        let original = ActionDiagnosticContext::capture(true, 7, &inventory, vm);
        assert!(original.is_some());
        assert_eq!(
            ActionDiagnosticContext::capture(false, 7, &inventory, vm),
            None,
            "a disconnected or stale inventory cannot retain a check"
        );
        assert_ne!(
            original,
            ActionDiagnosticContext::capture(true, 8, &inventory, vm)
        );
        inventory.vms.get_mut(&vm).unwrap().power_state = PowerState::Paused;
        assert_ne!(
            original,
            ActionDiagnosticContext::capture(true, 7, &inventory, vm)
        );
        inventory.vms.remove(&vm);
        assert_eq!(
            ActionDiagnosticContext::capture(true, 7, &inventory, vm),
            None
        );
    }
}
