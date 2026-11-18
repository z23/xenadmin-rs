//! The reconnect state machine of a console, carried over from the Rune
//! `console_recovery.rs`.
//!
//! [`Recovery::advance`] is pure: it is given what the inventory says about
//! the pinned target (a [`Binding`], or why there is none: [`Waiting`]), the
//! phase of the stream and the poll clock, and it answers with a
//! [`Decision`]: close the stream, reopen it, wake again after so many
//! seconds. The backoff (1, 2, 4, 8, 16 seconds, five attempts) and the
//! stability window (fifteen seconds connected before the budget is given
//! back) are measured on the poll clock, never on the wall clock.
//!
//! The glue that reads the observation and acts on the decision is
//! [`super::session::Session::poll`].

use xenadmin_core::{ConsoleState, HostId, Inventory, PowerState};

use super::session::ConsoleTarget;

/// Which stream a console is: the connection generation it was opened in,
/// the host its guest runs on and the guest's boot.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Binding {
    pub(super) generation: u64,
    pub(super) host: HostId,
    pub(super) started: Option<String>,
}

impl Binding {
    /// A missing boot marker matches anything.
    pub(super) fn same_stream(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.host == other.host
            && match (&self.started, &other.started) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}

/// Why the pinned target has no stream to offer just now.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Waiting {
    Connection,
    Guest,
    Power,
    ReadOnly,
}

impl Waiting {
    pub fn message(self) -> &'static str {
        match self {
            Self::Connection => {
                "Waiting for a current connection to this pool. Console input is released."
            }
            Self::Guest => "The guest or host is unavailable. The console target is unchanged.",
            Self::Power => "Waiting for the guest or host to run. Console input is released.",
            Self::ReadOnly => "Interactive consoles require a connection with write access.",
        }
    }
}

#[derive(Clone, Debug, PartialEq)]
pub struct Recovery {
    pub enabled: bool,
    pub(super) binding: Option<Binding>,
    pub(super) waiting: Option<Waiting>,
    pub(super) attempts: u8,
    pub(super) retry_at: Option<f64>,
    pub(super) needs_reopen: bool,
    pub(super) stable_since: Option<f64>,
    pub(super) exhausted: bool,
}

impl Default for Recovery {
    fn default() -> Self {
        Self {
            enabled: true,
            binding: None,
            waiting: None,
            attempts: 0,
            retry_at: None,
            needs_reopen: false,
            stable_since: None,
            exhausted: false,
        }
    }
}

#[derive(Default)]
pub(super) struct Decision {
    pub(super) close: bool,
    pub(super) reopen: bool,
    pub(super) wake: Option<f64>,
}

impl Recovery {
    pub fn with_enabled(enabled: bool) -> Self {
        Self {
            enabled,
            ..Default::default()
        }
    }

    pub(super) fn advance(
        &mut self,
        observation: Result<Binding, Waiting>,
        phase: &ConsoleState,
        clock: f64,
    ) -> Decision {
        let mut decision = Decision::default();
        self.exhausted = false;
        let mut binding = match observation {
            Ok(binding) => binding,
            Err(reason) => {
                decision.close = self.waiting != Some(reason) || !self.needs_reopen;
                self.waiting = Some(reason);
                self.needs_reopen = true;
                self.retry_at = None;
                self.stable_since = None;
                return decision;
            }
        };
        let resumed = self.waiting.take() == Some(Waiting::Power);
        if self
            .binding
            .as_ref()
            .is_some_and(|old| !old.same_stream(&binding))
        {
            decision.close = true;
            self.needs_reopen = true;
            self.attempts = 0;
            self.retry_at = None;
            self.stable_since = None;
        } else if binding.started.is_none() {
            binding.started = self.binding.as_ref().and_then(|old| old.started.clone());
        }
        self.binding = Some(binding);
        if resumed {
            self.attempts = 0;
        }
        if matches!(phase, ConsoleState::Closed | ConsoleState::Failed(_)) && !self.needs_reopen {
            decision.close = true;
            self.needs_reopen = true;
        }
        if !self.needs_reopen {
            match phase {
                ConsoleState::Connected => {
                    self.retry_at = None;
                    let since = *self.stable_since.get_or_insert(clock);
                    if clock - since >= 15.0 {
                        self.attempts = 0;
                    } else if self.attempts > 0 {
                        decision.wake = Some(15.0 - (clock - since));
                    }
                    return decision;
                }
                ConsoleState::Connecting => return decision,
                _ => {}
            }
        }
        self.stable_since = None;
        if !self.enabled || self.attempts >= 5 {
            self.retry_at = None;
            self.exhausted = self.enabled && self.attempts >= 5;
            return decision;
        }
        let deadline = *self
            .retry_at
            .get_or_insert(clock + f64::from(1u32 << self.attempts));
        if clock >= deadline {
            self.attempts += 1;
            self.retry_at = None;
            self.needs_reopen = false;
            decision.reopen = true;
            decision.wake = Some(1.0);
        } else {
            decision.wake = Some(deadline - clock);
        }
        decision
    }

    /// The line under the console's state, when recovery has something to
    /// say.
    pub fn status(&self) -> Option<String> {
        if let Some(waiting) = self.waiting {
            Some(waiting.message().into())
        } else if !self.enabled {
            Some(
                "Automatic reconnect is off. Use Reconnect when the guest or host is available."
                    .into(),
            )
        } else if self.exhausted {
            Some("Automatic reconnect stopped after five attempts. Check the connection and use Reconnect to try again.".into())
        } else if self.retry_at.is_some() {
            Some(format!(
                "Waiting to reconnect to this console (attempt {} of 5).",
                self.attempts + 1
            ))
        } else {
            None
        }
    }
}

/// What the inventory says about the pinned target. The table selection has
/// no part in it: only the pool, the VM and (for a host console) the host
/// the console was opened for.
pub fn inventory_binding(
    inventory: &Inventory,
    target: &ConsoleTarget,
    generation: u64,
) -> Result<Binding, Waiting> {
    if inventory.pool_id != target.pool {
        return Err(Waiting::Connection);
    }
    let vm = inventory
        .vms
        .get(&target.vm)
        .filter(|v| {
            !v.is_template
                && match target.host {
                    Some(host) => {
                        v.is_control_domain
                            && v.host == Some(host)
                            && inventory
                                .details
                                .hosts
                                .get(&host)
                                .and_then(|h| h.control_domain)
                                == Some(target.vm)
                    }
                    None => !v.is_control_domain,
                }
        })
        .ok_or(Waiting::Guest)?;
    if !matches!(vm.power_state, PowerState::Running | PowerState::Paused) {
        return Err(Waiting::Power);
    }
    let host = vm
        .host
        .filter(|h| inventory.hosts.contains_key(h))
        .ok_or(Waiting::Guest)?;
    let started = inventory
        .details
        .vms
        .get(&vm.id)
        .and_then(|v| v.metrics)
        .and_then(|id| inventory.details.vm_metrics.get(&id))
        .and_then(|m| m.start_time.clone());
    Ok(Binding {
        generation,
        host,
        started,
    })
}

#[cfg(test)]
mod tests {
    //! The Rune `console_recovery_tests.rs`, moved. The seventh test of that
    //! file (the toggle on screen) is a harness test in `super::super::tests`.

    use xenadmin_core::PoolId;

    use super::*;
    use crate::model::Model;

    fn at(generation: u64, host: u128, started: Option<&str>) -> Binding {
        Binding {
            generation,
            host: HostId::from(uuid::Uuid::from_u128(host)),
            started: started.map(str::to_owned),
        }
    }
    fn opened() -> Recovery {
        Recovery {
            binding: Some(at(1, 1, Some("boot-1"))),
            ..Default::default()
        }
    }
    #[test]
    fn external_move_reboot_and_generation_change_close_then_reopen_once() {
        for next in [
            at(1, 2, Some("boot-1")),
            at(1, 1, Some("boot-2")),
            at(2, 1, Some("boot-1")),
        ] {
            let mut recovery = opened();
            let d = recovery.advance(Ok(next.clone()), &ConsoleState::Connected, 0.0);
            assert!(d.close);
            assert!(!d.reopen);
            assert_eq!(d.wake, Some(1.0));
            let d = recovery.advance(Ok(next.clone()), &ConsoleState::Closed, 0.5);
            assert!(!d.close && !d.reopen);
            let d = recovery.advance(Ok(next.clone()), &ConsoleState::Closed, 1.0);
            assert!(d.reopen);
            assert!(
                !recovery
                    .advance(Ok(next.clone()), &ConsoleState::Connecting, 2.0)
                    .reopen
            );
            assert!(
                !recovery
                    .advance(Ok(next), &ConsoleState::Connected, 3.0)
                    .reopen
            );
        }
    }
    #[test]
    fn retry_budget_is_bounded_and_brief_connections_do_not_reset_it() {
        let mut recovery = opened();
        let b = recovery.binding.clone().unwrap();
        let mut clock = 0.0;
        for attempt in 1..=5 {
            let d = recovery.advance(
                Ok(b.clone()),
                &ConsoleState::Failed("unavailable".into()),
                clock,
            );
            assert!(!d.reopen);
            assert_eq!(d.wake, Some(f64::from(1u32 << (attempt - 1))));
            clock += d.wake.unwrap();
            assert!(
                recovery
                    .advance(Ok(b.clone()), &ConsoleState::Closed, clock)
                    .reopen
            );
            assert_eq!(recovery.attempts, attempt);
            clock += 0.25;
            recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock);
            assert!(!recovery.exhausted);
            clock += 0.25;
        }
        let d = recovery.advance(Ok(b.clone()), &ConsoleState::Closed, clock);
        assert!(!d.reopen && d.wake.is_none());
        assert!(recovery.exhausted);
        recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 1.0);
        recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 16.0);
        assert_eq!(
            recovery.attempts, 5,
            "a late frame on a closed stream cannot reopen it"
        );
        recovery = opened();
        recovery.attempts = 3;
        recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 1.0);
        recovery.advance(Ok(b.clone()), &ConsoleState::Connected, clock + 16.0);
        assert_eq!(recovery.attempts, 0);
        assert!(
            recovery
                .advance(Ok(b), &ConsoleState::Closed, clock + 17.0)
                .wake
                .is_some()
        );
    }
    #[test]
    fn stale_missing_halted_and_read_only_targets_cannot_reopen_or_reset_failures() {
        for reason in [
            Waiting::Connection,
            Waiting::Guest,
            Waiting::Power,
            Waiting::ReadOnly,
        ] {
            let mut recovery = opened();
            recovery.attempts = 3;
            let d = recovery.advance(Err(reason), &ConsoleState::Connected, 1.0);
            assert!(d.close && !d.reopen && d.wake.is_none());
            let d = recovery.advance(Err(reason), &ConsoleState::Closed, 100.0);
            assert!(!d.close && !d.reopen && d.wake.is_none());
            assert_eq!(recovery.attempts, 3);
            let d = recovery.advance(Ok(at(2, 1, Some("boot-2"))), &ConsoleState::Closed, 101.0);
            assert!(!d.reopen);
            assert!(
                recovery
                    .advance(Ok(at(2, 1, Some("boot-2"))), &ConsoleState::Closed, 102.0)
                    .reopen
            );
        }
    }
    #[test]
    fn optional_boot_metadata_never_causes_a_spurious_reopen_and_opt_out_is_honored() {
        let mut recovery = opened();
        assert!(
            !recovery
                .advance(Ok(at(1, 1, None)), &ConsoleState::Connected, 1.0)
                .close
        );
        assert_eq!(
            recovery.binding.as_ref().unwrap().started.as_deref(),
            Some("boot-1")
        );
        assert!(
            !recovery
                .advance(Ok(at(1, 1, Some("boot-1"))), &ConsoleState::Connected, 2.0)
                .close
        );
        recovery.enabled = false;
        let d = recovery.advance(Ok(at(1, 2, Some("boot-1"))), &ConsoleState::Connected, 3.0);
        assert!(d.close && !d.reopen && d.wake.is_none());
        assert!(
            !recovery
                .advance(Ok(at(1, 2, Some("boot-1"))), &ConsoleState::Closed, 100.0)
                .reopen
        );
        recovery.enabled = true;
        let d = recovery.advance(Ok(at(1, 2, Some("boot-1"))), &ConsoleState::Closed, 101.0);
        assert_eq!(d.wake, Some(1.0));
    }

    #[test]
    fn synchronous_admission_failure_consumes_attempts_and_releases_old_input() {
        let mut recovery = opened();
        let binding = recovery.binding.clone().unwrap();
        let mut clock = 0.0;
        for attempt in 1..=5 {
            let d = recovery.advance(Ok(binding.clone()), &ConsoleState::Closed, clock);
            assert!(d.close && !d.reopen);
            clock += d.wake.unwrap();
            assert!(
                recovery
                    .advance(Ok(binding.clone()), &ConsoleState::Closed, clock)
                    .reopen
            );
            assert_eq!(recovery.attempts, attempt);
            clock += 1.0;
        }
        let d = recovery.advance(Ok(binding), &ConsoleState::Closed, clock);
        assert!(d.close && !d.reopen && d.wake.is_none());
        assert!(recovery.exhausted);
    }

    #[test]
    fn inventory_observation_uses_pinned_pool_and_vm_and_rejects_missing_or_wrong_kind() {
        let mut m = Model::fixture();
        m.load_demo(3, 48);
        let mut inventory = m.inventory.as_ref().unwrap().as_ref().clone();
        let vm = inventory
            .vms
            .values()
            .find(|v| v.power_state == PowerState::Running)
            .unwrap()
            .clone();
        let target = ConsoleTarget {
            pool: inventory.pool_id,
            vm: vm.id,
            name: vm.name.clone(),
            host: None,
        };
        // The Rune test moved the table cursor here ("different selection"):
        // the observation takes no selection at all, so there is none to move.
        assert_eq!(
            inventory_binding(&inventory, &target, 7).unwrap().host,
            vm.host.unwrap()
        );
        inventory.pool_id = PoolId::from(uuid::Uuid::new_v4());
        assert_eq!(
            inventory_binding(&inventory, &target, 7),
            Err(Waiting::Connection)
        );
        inventory.pool_id = target.pool;
        inventory.vms.get_mut(&vm.id).unwrap().is_template = true;
        assert_eq!(
            inventory_binding(&inventory, &target, 7),
            Err(Waiting::Guest)
        );
        inventory.vms.get_mut(&vm.id).unwrap().is_template = false;
        inventory.vms.get_mut(&vm.id).unwrap().is_control_domain = true;
        assert_eq!(
            inventory_binding(&inventory, &target, 7),
            Err(Waiting::Guest)
        );
        inventory.vms.get_mut(&vm.id).unwrap().is_control_domain = false;
        inventory.vms.get_mut(&vm.id).unwrap().power_state = PowerState::Halted;
        assert_eq!(
            inventory_binding(&inventory, &target, 7),
            Err(Waiting::Power)
        );
        inventory.vms.get_mut(&vm.id).unwrap().power_state = PowerState::Paused;
        assert!(inventory_binding(&inventory, &target, 7).is_ok());
        inventory.hosts.remove(&vm.host.unwrap());
        assert_eq!(
            inventory_binding(&inventory, &target, 7),
            Err(Waiting::Guest)
        );
        inventory.vms.remove(&vm.id);
        assert_eq!(
            inventory_binding(&inventory, &target, 7),
            Err(Waiting::Guest)
        );
    }
}
