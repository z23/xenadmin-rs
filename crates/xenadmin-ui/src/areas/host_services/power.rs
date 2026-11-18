//! The remote power-on configuration of a server: the logic of the Rune
//! module `host_power_view.rs`, carried over.
//!
//! One editor for one server. Its draft is a mode (Disabled, Wake-on-LAN,
//! IPMI) and, for IPMI, the address and account of the BMC and a NEW
//! password. A review turns the draft into a
//! [`HostPowerOperation::Configure`] against the configuration captured
//! when the editor opened. A second, separate operation removes a password
//! Secret that an unfinished or failed configuration left behind
//! ([`HostPowerOperation::CleanupSecret`]); it never changes settings.
//!
//! What must not be lost (map, hazards 10 to 14):
//!
//! - every edit of the draft bumps `epoch`; a review is valid only while
//!   `epoch == review_epoch`;
//! - after every submission attempt, also a refused one, the password is
//!   dropped, the review cleared and the epoch bumped; the error asks for
//!   the password again;
//! - unchanged Disabled or Wake-on-LAN is refused, unchanged IPMI is not (a
//!   new password is always part of it);
//! - only terminal, not succeeded, uncleaned records of the same pool and
//!   server offer the cleanup;
//! - the permission is checked again inside the submission.

use std::sync::Arc;

use xenadmin_core::{
    Error, HostId, HostPowerConfiguration, HostPowerOperation, HostPowerPassword,
    HostPowerSettings, Inventory, ObservedResource, OperationId, OperationState, PoolId,
    ResourceAction, ResourceCommand, ResourceOperationRecord, ResourceTarget,
};

use crate::prelude::*;

/// The line of a review that can no longer be confirmed (Rune wording).
pub const STALE: &str =
    "Connection or reviewed host settings changed. Reopen this editor before applying.";

/// The two fixed paragraphs of a configuration review. Decision 10 of
/// AREAS.md: "Activity" is renamed in one sweep at the end, not here.
pub const CONFIGURE_EFFECT: &str = "The host stores the new configuration and removes its previous password Secret. Shared Secret consumers are checked first. IPMI creates a new server Secret; its clear password is never saved in application history. This does not power on the host or verify BMC reachability.";
pub const CONFIGURE_UNCERTAIN: &str = "Lost replies remain uncertain. No password or configuration write is automatically repeated. Use the Events tab of the Notifications view to recheck evidence and review retained Secret cleanup separately.";
/// The fixed paragraph of a cleanup review.
pub const CLEANUP_EFFECT: &str = "Cleanup requires the original write to be finished or never dispatched, verified ownership, and no host or storage consumers. It never changes power-on settings. Missing evidence prevents deletion.";

pub struct Editor {
    pub initial: Arc<Inventory>,
    pub target: ResourceTarget,
    pub observed: Option<ObservedResource>,
    pub expected: Option<HostPowerConfiguration>,
    /// 0 Disabled, 1 Wake-on-LAN, 2 IPMI.
    pub mode: u8,
    pub address: String,
    pub username: String,
    /// Zeroized when it is cleared and when the editor is dropped.
    pub password: SecretText,
    pub epoch: u64,
    pub review_epoch: u64,
    pub review: Option<HostPowerOperation>,
    pub error: Option<String>,
}

impl Editor {
    /// The editor for a server of a pool whose inventory is known. `None`:
    /// the object is no such server.
    pub fn capture(m: &Model, object: &ObjectRef) -> Option<Self> {
        if object.kind != ObjectKind::Host {
            return None;
        }
        let pool = object.pool?;
        let host = object.host_id()?;
        let initial = m.inventory_for(pool)?;
        if !initial.hosts.contains_key(&host) {
            return None;
        }
        let target = ResourceTarget::Host(host);
        let expected = initial
            .details
            .hosts
            .get(&host)
            .and_then(|h| h.power_configuration.clone());
        let (mode, address, username) = match expected.as_ref().map(|c| &c.settings) {
            Some(HostPowerSettings::Ipmi { address, username }) => {
                (2, address.to_string(), username.clone())
            }
            Some(HostPowerSettings::WakeOnLan) => (1, String::new(), String::new()),
            _ => (0, String::new(), String::new()),
        };
        let observed = m
            .snapshot_for(pool)
            .and_then(|s| s.observe_resource(target).ok());
        Some(Self {
            initial,
            target,
            observed,
            expected,
            mode,
            address,
            username,
            password: SecretText::new(),
            epoch: 0,
            review_epoch: 0,
            review: None,
            error: None,
        })
    }

    /// The pool of the server: the editor follows it, not the scope session.
    pub fn pool(&self) -> PoolId {
        self.initial.pool_id
    }

    pub fn host(&self) -> Option<HostId> {
        match self.target {
            ResourceTarget::Host(host) => Some(host),
            _ => None,
        }
    }

    pub fn label(&self) -> &str {
        self.host()
            .and_then(|id| self.initial.hosts.get(&id))
            .map(|h| h.name.as_str())
            .unwrap_or("Unknown host")
    }
}

/// Opens the editor on its draft. `false`: the object is no known server.
pub fn open(m: &mut Model, object: &ObjectRef) -> bool {
    let Some(editor) = Editor::capture(m, object) else {
        return false;
    };
    m.areas.host_services.power = Some(editor);
    true
}

pub fn close(m: &mut Model) {
    m.areas.host_services.power = None;
}

pub fn change_mode(m: &mut Model, mode: u8) {
    if let Some(e) = &mut m.areas.host_services.power {
        e.mode = mode;
        e.epoch += 1;
        e.review = None;
        e.error = None;
        e.password.clear();
    }
}

/// An edit of one of the three IPMI fields. Every edit invalidates a review.
pub fn edit(m: &mut Model, apply: impl FnOnce(&mut Editor)) {
    if let Some(e) = &mut m.areas.host_services.power {
        e.epoch += 1;
        apply(e);
    }
}

pub fn settings(e: &Editor) -> Result<HostPowerSettings, String> {
    Ok(match e.mode {
        0 => HostPowerSettings::Disabled,
        1 => HostPowerSettings::WakeOnLan,
        _ => {
            let address = e
                .address
                .trim()
                .parse::<std::net::IpAddr>()
                .map_err(|_| "Enter a valid BMC IP address")?;
            if address.is_unspecified() || address.is_multicast() {
                return Err("Enter a unicast BMC IP address".into());
            }
            if e.username.is_empty()
                || e.username.len() > 255
                || e.username.chars().any(char::is_control)
            {
                return Err("Enter a valid BMC username".into());
            }
            HostPowerPassword::new(e.password.expose().to_owned())
                .map_err(|_| "Enter a new BMC password")?;
            HostPowerSettings::Ipmi {
                address,
                username: e.username.clone(),
            }
        }
    })
}

pub fn review(m: &mut Model) {
    let Some(e) = &mut m.areas.host_services.power else {
        return;
    };
    let result = (|| {
        let expected = e.expected.clone().ok_or(
            "Current power-on settings are unavailable or use an unsupported custom plugin",
        )?;
        let settings = settings(e)?;
        if !settings.needs_password() && expected.settings == settings {
            return Err("Choose a different power-on mode".into());
        }
        Ok(HostPowerOperation::Configure { expected, settings })
    })();
    match result {
        Ok(op) => {
            e.review = Some(op);
            e.review_epoch = e.epoch;
            e.error = None;
        }
        Err::<_, String>(error) => e.error = Some(error),
    }
}

/// Back from a review to the draft. The typed password stays.
pub fn back(m: &mut Model) {
    if let Some(e) = &mut m.areas.host_services.power {
        e.review = None;
        e.error = None;
    }
}

/// The records of configurations of this server that may have left a
/// password Secret behind: not succeeded, with an uncleaned Secret and no
/// succeeded cleanup. The newest first, at most twenty.
pub fn retained<'a>(m: &'a Model, e: &Editor) -> Vec<&'a ResourceOperationRecord> {
    m.resource_rows
        .iter()
        .rev()
        .filter(|r| {
            r.pool == e.initial.pool_id
                && r.target == e.target
                && matches!(
                    r.action,
                    ResourceAction::HostPower(HostPowerOperation::Configure { .. })
                )
                && r.state != OperationState::Succeeded
                && !m.resource_rows.iter().any(|cleanup| {
                    matches!(
                        cleanup.action,
                        ResourceAction::HostPower(HostPowerOperation::CleanupSecret {
                            operation,
                            ..
                        }) if operation == r.id
                    ) && cleanup.state == OperationState::Succeeded
                })
                && r.host_power_secret.as_ref().is_some_and(|v| !v.cleaned)
        })
        .take(20)
        .collect()
}

/// Reviews the removal of the Secret one finished record left behind.
pub fn cleanup(m: &mut Model, id: OperationId) {
    let Some(e) = &mut m.areas.host_services.power else {
        return;
    };
    let Some(r) = m.resource_rows.iter().find(|r| {
        r.id == id && r.target == e.target && r.pool == e.initial.pool_id && r.state.terminal()
    }) else {
        return;
    };
    let Some(secret) = r.host_power_secret.as_ref().filter(|v| !v.cleaned) else {
        return;
    };
    e.review = Some(HostPowerOperation::CleanupSecret {
        operation: id,
        secret: secret.secret,
    });
    e.review_epoch = e.epoch;
    e.password.clear();
    e.error = None;
}

pub fn synthetic(m: &Model, e: &Editor) -> bool {
    m.is_synthetic_pool(e.pool())
}

pub fn current(m: &Model, e: &Editor) -> bool {
    let pool = e.pool();
    m.can_mutate_pool(pool)
        && e.epoch == e.review_epoch
        && e.observed.is_some()
        && e.observed
            == m.snapshot_for(pool)
                .and_then(|s| s.observe_resource(e.target).ok())
        && match &e.review {
            Some(HostPowerOperation::Configure { expected, .. }) => {
                let ResourceTarget::Host(id) = e.target else {
                    return false;
                };
                m.inventory_for(pool).is_some_and(|i| {
                    i.details
                        .hosts
                        .get(&id)
                        .and_then(|d| d.power_configuration.as_ref())
                        == Some(expected)
                })
            }
            Some(HostPowerOperation::CleanupSecret { .. }) => true,
            None => false,
        }
}

/// Why the open review cannot be confirmed any more. A synthetic pool has
/// no observation and says so with its own notice.
pub fn stale(m: &Model, e: &Editor) -> Option<String> {
    (e.review.is_some() && !synthetic(m, e) && !current(m, e)).then(|| STALE.to_owned())
}

pub fn submit(m: &mut Model) {
    let result = (|| {
        let e = m.areas.host_services.power.as_ref().ok_or(Error::Stale)?;
        if !current(m, e) {
            return Err(Error::Stale);
        }
        let review = e.review.clone().ok_or(Error::Stale)?;
        let permission = permissions::assess_for(m, e.pool(), &review.permission_requirements());
        if permission.decision().is_denied() {
            return Err(Error::Api("RBAC_PERMISSION_DENIED".into()));
        }
        let connection = m.connection_for(e.pool()).ok_or(Error::Closed)?;
        let host = e.observed.ok_or(Error::Stale)?;
        match review {
            HostPowerOperation::Configure { expected, settings } => {
                let password = if settings.needs_password() {
                    Some(HostPowerPassword::new(e.password.expose().to_owned())?)
                } else {
                    None
                };
                connection.configure_host_power(
                    OperationId::new(),
                    host,
                    expected,
                    settings,
                    password,
                )
            }
            operation @ HostPowerOperation::CleanupSecret { .. } => connection.submit_resource(
                OperationId::new(),
                ResourceCommand::HostPower { host, operation },
            ),
        }
    })();
    if let Some(e) = &mut m.areas.host_services.power {
        e.password.clear();
        e.review = None;
        e.epoch += 1;
    }
    match result {
        Ok(op) => {
            m.track_resource(op);
            m.areas.host_services.power = None;
        }
        Err(error) => {
            if let Some(e) = &mut m.areas.host_services.power {
                e.error = Some(format!("{error}. Enter the password and review again."));
            }
        }
    }
}

pub fn summary(c: &HostPowerConfiguration) -> String {
    match &c.settings {
        HostPowerSettings::Disabled => "Disabled".into(),
        HostPowerSettings::WakeOnLan => "Wake-on-LAN".into(),
        HostPowerSettings::Ipmi { address, username } => format!(
            "IPMI at {address}, user {username}, Secret {}",
            c.secret
                .map(|v| v.to_string())
                .unwrap_or_else(|| "unknown".into())
        ),
    }
}

pub fn requested(settings: &HostPowerSettings) -> String {
    match settings {
        HostPowerSettings::Disabled => "Disabled".into(),
        HostPowerSettings::WakeOnLan => "Wake-on-LAN".into(),
        HostPowerSettings::Ipmi { address, username } => {
            format!("IPMI at {address}, user {username}, with a new password")
        }
    }
}

/// The lines of a review: what Rune showed as one caption of two lines and
/// its fixed paragraphs.
pub fn lines(op: &HostPowerOperation) -> Vec<String> {
    match op {
        HostPowerOperation::Configure { expected, settings } => vec![
            format!("Current: {}", summary(expected)),
            format!("Requested: {}", requested(settings)),
            CONFIGURE_EFFECT.into(),
            CONFIGURE_UNCERTAIN.into(),
        ],
        HostPowerOperation::CleanupSecret { operation, secret } => vec![
            format!("Remove retained Secret {secret}"),
            format!("Original operation {operation}"),
            CLEANUP_EFFECT.into(),
        ],
    }
}
