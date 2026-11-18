//! Performance alarms and their email: the logic carried over from Rune
//! `alarms_view.rs`. Validation, refusals and review texts are word for
//! word.
//!
//! The Rune workspace was one dialog with three tabs and, on the first, a
//! picker of every resource of the pool. Here the same workspace is created
//! for ONE place:
//!
//! - [`Page::Policies`]: the page Alerts of the Properties dialog of a VM, a
//!   server or a repository. The picker is gone: the policies are those of
//!   the object ([`policies_of`]). A server has two, its own and the memory
//!   of its control domain; no metric belongs to both, so the metric chosen
//!   says which policy is edited ([`pick`]).
//! - [`Page::Email`]: the page Email Options of the pool.
//! - [`Page::Refresh`]: the dialog of the command "Refresh Alarm Service",
//!   which has nothing to enter and opens on its review.
//!
//! Also changed: the workspace follows the pool of its target; the
//! submission looks at the permission again (Rune only disabled the button).

use std::sync::Arc;

use xenadmin_core::{
    AlarmEmailSettings, AlarmMetric, AlarmOperation, AlarmPolicy, AlarmPolicyChange, AlarmRule,
    Error, Inventory, ObservedResource, OperationId, PermissionAssessment, PoolId, ResourceAction,
    ResourceCommand, ResourceOperation, ResourceTarget,
};

use crate::prelude::*;

/// Shown with a review that is no longer current (Rune disabled the button
/// without a word).
pub const STALE: &str = "The connection or the reviewed object changed, or too many operations \
    are unfinished. Reload settings before applying this review.";

/// The ids of the two Properties pages.
pub const ALERTS: &str = "alerts";
pub const EMAIL_OPTIONS: &str = "email-options";

#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Page {
    Policies,
    Email,
    Refresh,
}

impl Page {
    /// The Properties page this is. `None`: a dialog of its own.
    pub fn id(self) -> Option<&'static str> {
        match self {
            Self::Policies => Some(ALERTS),
            Self::Email => Some(EMAIL_OPTIONS),
            Self::Refresh => None,
        }
    }
}

pub struct Workspace {
    pub initial: Arc<Inventory>,
    /// The pool, observed at open: what Email and Refresh are reviewed
    /// against.
    pub pool: Option<ObservedResource>,
    pub page: Page,
    /// The object whose Alerts page this is.
    pub object: Option<ObjectRef>,
    /// The index of `policy` in `initial.details.alarm_policies`.
    pub index: Option<usize>,
    pub policy: Option<AlarmPolicy>,
    /// The target of `policy`, observed when the policy was chosen.
    pub target: Option<ObservedResource>,
    pub metric: AlarmMetric,
    pub threshold: String,
    pub duration: String,
    pub repeat: String,
    pub enabled: bool,
    pub destination: String,
    pub relay: String,
    pub language: String,
    pub review: Option<AlarmOperation>,
    pub error: Option<String>,
}

impl Workspace {
    pub fn pool_id(&self) -> PoolId {
        self.initial.pool_id
    }
}

/// The Rune `open`: the pool as it is, the first tab, nothing chosen.
pub fn capture(m: &Model, pool: PoolId, page: Page) -> Option<Workspace> {
    let initial = m.inventory_for(pool)?;
    let email = initial
        .details
        .alarm_email
        .as_ref()
        .and_then(|v| v.settings.clone())
        .unwrap_or(AlarmEmailSettings {
            destination: String::new(),
            relay: String::new(),
            language: "en-US".into(),
        });
    let observed = m.snapshot_for(pool).and_then(|v| {
        v.observe_resource(ResourceTarget::Pool(initial.pool_id))
            .ok()
    });
    Some(Workspace {
        initial,
        pool: observed,
        page,
        object: None,
        index: None,
        policy: None,
        target: None,
        metric: AlarmMetric::Cpu,
        threshold: "80".into(),
        duration: "60".into(),
        repeat: "3600".into(),
        enabled: true,
        destination: email.destination,
        relay: email.relay,
        language: email.language,
        review: None,
        error: None,
    })
}

/// The policies of an object, as indices into `details.alarm_policies`: the
/// one whose target it is and, for a server, the one of its control domain.
pub fn policies_of(inventory: &Inventory, object: &ObjectRef) -> Vec<usize> {
    let target = resource_target(object);
    let host = (object.kind == ObjectKind::Host)
        .then(|| object.host_id())
        .flatten();
    inventory
        .details
        .alarm_policies
        .iter()
        .enumerate()
        .filter(|(_, p)| match p.control_domain_host {
            Some(dom0) => host == Some(dom0),
            None => target == Some(p.target),
        })
        .map(|(index, _)| index)
        .collect()
}

pub fn label(inv: &Inventory, policy: &AlarmPolicy) -> String {
    let name = match policy.target {
        ResourceTarget::Vm(id) => inv
            .vms
            .get(&id)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "Control domain".into()),
        ResourceTarget::Host(id) => inv
            .hosts
            .get(&id)
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "Host".into()),
        ResourceTarget::Storage(id) => inv
            .storage
            .iter()
            .find(|v| v.uuid == id.uuid())
            .map(|v| v.name.clone())
            .unwrap_or_else(|| "Storage".into()),
        _ => "Resource".into(),
    };
    if let Some(host) = policy.control_domain_host {
        format!(
            "{} · control-domain memory",
            inv.hosts
                .get(&host)
                .map(|v| v.name.as_str())
                .unwrap_or("Host")
        )
    } else {
        format!(
            "{name} · {}",
            match policy.target {
                ResourceTarget::Vm(_) => "VM",
                ResourceTarget::Host(_) => "Host",
                ResourceTarget::Storage(_) => "Storage",
                _ => "Resource",
            }
        )
    }
}

pub fn units(metric: AlarmMetric) -> &'static str {
    match metric {
        AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory => "percent (0–100)",
        _ => metric.unit(),
    }
}

/// Another metric of the chosen policy: the draft is that metric's rule, or
/// the defaults. Percent metrics are shown multiplied by 100.
pub fn load_metric(w: &mut Workspace, metric: AlarmMetric) {
    w.metric = metric;
    w.review = None;
    w.error = None;
    let rule = w
        .policy
        .as_ref()
        .and_then(|p| p.rules.iter().find(|r| r.metric == metric));
    w.enabled = rule.is_some();
    let native = rule
        .map(|r| r.threshold.parse::<f64>().unwrap_or(0.0))
        .unwrap_or(
            if matches!(metric, AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory) {
                0.8
            } else {
                1024.0
            },
        );
    w.threshold = (if matches!(metric, AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory) {
        native * 100.0
    } else {
        native
    })
    .to_string();
    w.duration = rule.map(|r| r.duration_seconds).unwrap_or(60).to_string();
    w.repeat = rule.map(|r| r.repeat_seconds).unwrap_or(3600).to_string();
}

/// Chooses a policy of the captured inventory. Its target is observed now,
/// not when the workspace opened.
pub fn select(m: &mut Model, index: usize) {
    let Some(w) = &m.areas.schedules_alarms.alarms else {
        return;
    };
    let Some(policy) = w.initial.details.alarm_policies.get(index).cloned() else {
        return;
    };
    let target = m
        .snapshot_for(w.pool_id())
        .and_then(|v| v.observe_resource(policy.target).ok());
    let metric = AlarmMetric::ALL
        .into_iter()
        .find(|m| m.supported(policy.target, policy.control_domain_host))
        .unwrap_or(AlarmMetric::Cpu);
    let Some(w) = &mut m.areas.schedules_alarms.alarms else {
        return;
    };
    w.target = target;
    w.index = Some(index);
    w.policy = Some(policy);
    load_metric(w, metric);
}

/// The metric choice of the Alerts page: a metric of another policy of the
/// same object chooses that policy first.
pub fn pick(m: &mut Model, index: usize, metric: AlarmMetric) {
    let Some(w) = &m.areas.schedules_alarms.alarms else {
        return;
    };
    if w.index != Some(index) {
        select(m, index);
    }
    if let Some(w) = &mut m.areas.schedules_alarms.alarms
        && w.index == Some(index)
    {
        load_metric(w, metric);
    }
}

pub fn intent(w: &Workspace) -> Result<AlarmOperation, String> {
    match w.page {
        Page::Policies => {
            let p = w.policy.as_ref().ok_or("Choose a resource")?;
            if let Some(error) = &p.error {
                return Err(error.clone());
            }
            let rule = if w.enabled {
                if p.customized.contains(&w.metric) {
                    return Err("This alarm has a custom source or direction. Keep its definition or explicitly disable it; this editor cannot replace it.".into());
                }
                let mut value = w
                    .threshold
                    .parse::<f64>()
                    .map_err(|_| "Enter a numeric threshold")?;
                if matches!(
                    w.metric,
                    AlarmMetric::Cpu | AlarmMetric::ControlDomainMemory
                ) {
                    value /= 100.0;
                }
                let rule = AlarmRule {
                    metric: w.metric,
                    threshold: value.to_string(),
                    duration_seconds: w
                        .duration
                        .parse()
                        .map_err(|_| "Duration must be a whole number of seconds")?,
                    repeat_seconds: w
                        .repeat
                        .parse()
                        .map_err(|_| "Repeat interval must be a whole number of seconds")?,
                };
                rule.validate().map_err(|_| {
                    "Check the threshold and use duration 1–604800 seconds and repeat interval \
                     1–2592000 seconds"
                })?;
                Some(rule)
            } else {
                if !p.rules.iter().any(|r| r.metric == w.metric) {
                    return Err("This alarm is already disabled".into());
                }
                None
            };
            Ok(AlarmOperation::SetPolicy(AlarmPolicyChange {
                expected_revision: p.revision.clone(),
                control_domain_host: p.control_domain_host,
                metric: w.metric,
                rule,
            }))
        }
        Page::Email => {
            let current = w
                .initial
                .details
                .alarm_email
                .as_ref()
                .ok_or("Pool email settings unavailable")?;
            if let Some(error) = &current.error {
                return Err(error.clone());
            }
            let settings = AlarmEmailSettings {
                destination: w.destination.clone(),
                relay: w.relay.clone(),
                language: w.language.clone(),
            };
            settings.validate().map_err(|_| {
                "Use one email address (or leave it empty to disable), a relay host[:port], and \
                 a supported language"
            })?;
            Ok(AlarmOperation::SetEmail {
                expected_revision: current.revision.clone(),
                settings,
            })
        }
        Page::Refresh => {
            let hosts = w.initial.hosts.keys().copied().collect::<Vec<_>>();
            if hosts.is_empty() || hosts.len() > 64 {
                return Err("Refresh supports 1–64 captured hosts".into());
            }
            Ok(AlarmOperation::Refresh { hosts })
        }
    }
}

pub fn review(m: &mut Model) {
    if let Some(w) = &mut m.areas.schedules_alarms.alarms {
        match intent(w) {
            Ok(op) => {
                w.review = Some(op);
                w.error = None;
            }
            Err(e) => w.error = Some(e),
        }
    }
}

/// "Back": from the review to the draft. (In Rune a second Back returned to
/// the picker, which no longer exists.)
pub fn back(m: &mut Model) {
    if let Some(w) = &mut m.areas.schedules_alarms.alarms {
        w.review = None;
        w.error = None;
    }
}

/// What the operation is reviewed against: the chosen resource on the
/// Alerts page, the pool otherwise.
pub fn observed(w: &Workspace) -> Option<ObservedResource> {
    if w.page == Page::Policies {
        w.target
    } else {
        w.pool
    }
}

pub fn synthetic(m: &Model, w: &Workspace) -> bool {
    m.is_synthetic_pool(w.pool_id())
}

/// Fewer than 64 unfinished resource operations (handles that are not
/// terminal, unlike the crashdump cap, which counts every handle), a
/// writable current connection, and the observation still the one a fresh
/// look gives.
pub fn current(m: &Model, w: &Workspace) -> bool {
    let pool = w.pool_id();
    !synthetic(m, w)
        && m.can_mutate_pool(pool)
        && m.resource_operations
            .iter()
            .filter(|h| !h.latest().state.terminal())
            .count()
            < 64
        && observed(w).is_some_and(|o| {
            m.snapshot_for(pool)
                .and_then(|v| v.observe_resource(o.target()).ok())
                == Some(o)
        })
}

pub fn stale(m: &Model, w: &Workspace) -> Option<String> {
    (w.review.is_some() && !synthetic(m, w) && !current(m, w)).then(|| STALE.to_owned())
}

/// The permission of the review, assessed with the action's requirements
/// for its target (not the operation's own). `None` before the review and
/// while nothing was observed.
pub fn permission(m: &Model, w: &Workspace) -> Option<PermissionAssessment> {
    let target = observed(w).map(|o| o.target())?;
    let op = w.review.as_ref()?;
    Some(permissions::assess_for(
        m,
        w.pool_id(),
        &ResourceAction::Alarm(op.clone()).permission_requirements(target),
    ))
}

/// "Confirm change". On success the handle goes to the model and the
/// surface closes; on a refusal it stays with the reason and the review.
pub fn submit(m: &mut Model) {
    let Some(w) = &m.areas.schedules_alarms.alarms else {
        return;
    };
    match submission(m, w) {
        Ok(op) => {
            m.track_resource(op);
            super::close_alarms(m);
        }
        Err(e) => {
            if let Some(w) = &mut m.areas.schedules_alarms.alarms {
                w.error = Some(e);
            }
        }
    }
}

fn submission(m: &Model, w: &Workspace) -> Result<ResourceOperation, String> {
    let stale = || Error::Stale.to_string();
    if !current(m, w) {
        return Err(stale());
    }
    if let Some(denial) = permission(m, w).as_ref().and_then(permissions::denial) {
        return Err(denial);
    }
    m.connection_for(w.pool_id())
        .ok_or_else(|| Error::Closed.to_string())?
        .submit_resource(
            OperationId::new(),
            ResourceCommand::Alarm {
                target: observed(w).ok_or_else(stale)?,
                operation: w.review.clone().ok_or_else(stale)?,
            },
        )
        .map_err(|e| e.to_string())
}

/// The review text of an operation, word for word from Rune. The rule line
/// prints the threshold as it was typed, not the converted value.
pub fn review_lines(w: &Workspace, op: &AlarmOperation) -> Vec<String> {
    let mut lines = vec![format!("Pool {}", w.initial.pool_id)];
    match op {
        AlarmOperation::SetPolicy(change) => {
            if let Some(p) = &w.policy {
                lines.push(format!("{} ({})", label(&w.initial, p), p.target.uuid()));
            }
            lines.push(match &change.rule {
                Some(rule) => format!(
                    "{}: {} {} {}, sustained for {} seconds; repeat no sooner than {} seconds",
                    rule.metric.label(),
                    if rule.metric.low_threshold() {
                        "below"
                    } else {
                        "above"
                    },
                    w.threshold,
                    units(rule.metric),
                    rule.duration_seconds,
                    rule.repeat_seconds
                ),
                None => format!("Disable {}", change.metric.label()),
            });
            lines.push("Only this alarm is changed. Other alarm definitions and metadata are retained. Saving does not verify the host monitoring service. Request a service refresh separately, or wait for its periodic reload (up to 30 minutes).".into());
        }
        AlarmOperation::SetEmail { settings, .. } => {
            lines.push(if settings.destination.is_empty() {
                "Disable pool alarm email delivery".into()
            } else {
                format!(
                    "Destination: {} · relay: {} · language: {}",
                    settings.destination,
                    if settings.relay.is_empty() {
                        "host default"
                    } else {
                        &settings.relay
                    },
                    settings.language
                )
            });
            lines.push("This saves pool email configuration. It does not send a test message or verify SMTP delivery. Existing sender, credentials and other SMTP settings are retained.".into());
        }
        AlarmOperation::Refresh { hosts } => {
            for host in hosts {
                lines.push(format!(
                    "{} ({host})",
                    w.initial
                        .hosts
                        .get(host)
                        .map(|h| h.name.as_str())
                        .unwrap_or("Host")
                ));
            }
            lines.push("Request the installed perfmon service to reload configuration on these hosts. Acceptance confirms only that a refresh notification was sent; it does not confirm alarm evaluation or email delivery. Failed or uncertain requests are not replayed.".into());
        }
    }
    lines
}

// -- The three places ---------------------------------------------------------------

/// The workspace of a Properties page, for the object of the dialog.
fn for_page(m: &Model, target: &Target, id: &str) -> Option<Workspace> {
    let object = target.single()?;
    let pool = object.pool?;
    match (id, object.kind) {
        (EMAIL_OPTIONS, ObjectKind::Pool) => capture(m, pool, Page::Email),
        (ALERTS, _) => {
            let mut w = capture(m, pool, Page::Policies)?;
            w.object = Some(object.clone());
            Some(w)
        }
        _ => None,
    }
}

/// The Properties dialog tells the area about its pages. Entering a page
/// captures afresh, as opening the Rune dialog did (and as "Reload
/// settings" does); leaving it drops the workspace.
pub fn page_event(m: &mut Model, target: &Target, event: PageEvent) {
    match event {
        PageEvent::Entered(id) => {
            let workspace = for_page(m, target, id);
            let first = workspace.as_ref().and_then(|w| {
                let object = w.object.as_ref()?;
                policies_of(&w.initial, object).first().copied()
            });
            m.areas.schedules_alarms.alarms = workspace;
            if let Some(index) = first {
                select(m, index);
            }
        }
        PageEvent::Left(id) => {
            let alarms = &mut m.areas.schedules_alarms.alarms;
            if alarms.as_ref().is_some_and(|w| w.page.id() == Some(id)) {
                *alarms = None;
            }
        }
        PageEvent::Closed => {}
    }
}

/// The dialog of "Refresh Alarm Service": captured and already reviewed.
/// `Err`: why there is nothing to review.
pub fn refresh(m: &Model, pool: PoolId) -> Option<Result<Workspace, String>> {
    let mut w = capture(m, pool, Page::Refresh)?;
    Some(match intent(&w) {
        Ok(op) => {
            w.review = Some(op);
            Ok(w)
        }
        Err(e) => Err(e),
    })
}

/// "Reload settings": the Rune `open` again. A page is entered again, which
/// captures afresh; the refresh dialog is captured and reviewed again.
pub fn reload(m: &mut Model) {
    let Some(w) = &m.areas.schedules_alarms.alarms else {
        return;
    };
    match w.page.id() {
        Some(id) => {
            m.show_properties_page(id);
        }
        None => {
            let pool = w.pool_id();
            match refresh(m, pool) {
                Some(Ok(fresh)) => m.areas.schedules_alarms.alarms = Some(fresh),
                Some(Err(e)) => {
                    if let Some(w) = &mut m.areas.schedules_alarms.alarms {
                        w.error = Some(e);
                    }
                }
                None => super::close_alarms(m),
            }
        }
    }
}
