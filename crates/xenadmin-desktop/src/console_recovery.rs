use super::*;
use xenadmin_core::{HostId, PowerState};

#[derive(Clone, Debug, PartialEq, Eq)]
struct Binding {
    generation: u64,
    host: HostId,
    started: Option<String>,
}
impl Binding {
    fn same_stream(&self, other: &Self) -> bool {
        self.generation == other.generation
            && self.host == other.host
            && match (&self.started, &other.started) {
                (Some(a), Some(b)) => a == b,
                _ => true,
            }
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum Waiting {
    Connection,
    Guest,
    Power,
    ReadOnly,
}
impl Waiting {
    fn message(self) -> &'static str {
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
#[derive(Clone, PartialEq)]
pub(super) struct Recovery {
    pub enabled: bool,
    binding: Option<Binding>,
    waiting: Option<Waiting>,
    attempts: u8,
    retry_at: Option<f64>,
    needs_reopen: bool,
    stable_since: Option<f64>,
    exhausted: bool,
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
struct Decision {
    close: bool,
    reopen: bool,
    wake: Option<f64>,
}
impl Recovery {
    pub(super) fn with_enabled(enabled: bool) -> Self {
        Self {
            enabled,
            ..Default::default()
        }
    }
    fn advance(
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
    pub(super) fn status(&self) -> Option<String> {
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
fn binding(s: &State) -> Result<Binding, Waiting> {
    let target = s.console_ui.target().ok_or(Waiting::Guest)?;
    let connection = s.connection.as_ref().ok_or(Waiting::Connection)?;
    if connection.is_read_only() {
        return Err(Waiting::ReadOnly);
    }
    let snapshot = s.snapshot.as_ref().ok_or(Waiting::Connection)?;
    let inventory = snapshot
        .inventory()
        .filter(|i| snapshot.is_current() && i.pool_id == target.pool)
        .ok_or(Waiting::Connection)?;
    inventory_binding(inventory, target, snapshot.generation())
}
fn inventory_binding(
    inventory: &xenadmin_core::Inventory,
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
pub(super) fn opened(s: &mut State) {
    s.console_ui.recovery.binding = binding(s).ok();
}
pub(super) fn manual(s: &mut State) {
    let enabled = s.console_ui.recovery.enabled;
    s.console_ui.recovery = Recovery {
        enabled,
        ..Default::default()
    };
    s.reconnect_console();
}
pub(super) fn input_ready(s: &State) -> bool {
    !s.console_ui.recovery.needs_reopen
        && s.console_ui.recovery.waiting.is_none()
        && matches!(
            s.console_snapshot.as_ref().map(|s| &s.state),
            Some(ConsoleState::Connected)
        )
        && binding(s).is_ok_and(|current| {
            s.console_ui
                .recovery
                .binding
                .as_ref()
                .is_some_and(|opened| opened.same_stream(&current))
        })
}
pub(crate) fn poll(s: &mut State, clock: f64, tx: &mut Tx) -> bool {
    if s.console.is_none() || s.console_ui.target().is_none() {
        return false;
    }
    let observation = binding(s);
    let phase = s
        .console_snapshot
        .as_ref()
        .map(|s| s.state.clone())
        .unwrap_or(ConsoleState::Connecting);
    let before = s.console_ui.recovery.clone();
    let decision = s.console_ui.recovery.advance(observation, &phase, clock);
    if decision.close {
        release(s);
        if let Some(console) = &s.console {
            console.close();
        }
        s.console_ui.serial = next_serial();
        s.console_ui.export = Default::default();
    }
    if decision.reopen {
        s.reconnect_console();
    }
    if let Some(seconds) = decision.wake {
        tx.wake_after(std::time::Duration::from_secs_f64(seconds.max(0.001)));
    }
    before != s.console_ui.recovery || decision.close || decision.reopen
}

#[cfg(test)]
#[path = "console_recovery_tests.rs"]
mod tests;
