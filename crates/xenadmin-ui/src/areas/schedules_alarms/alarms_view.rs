//! Alarms, described: the pages Alerts and Email Options of the kernel's
//! Properties dialogs, and the dialog `alarm-service-refresh`. Nothing here
//! changes the model.
//!
//! Keys and labels are the Rune ones: `alarm-enabled`, `alarm-threshold`,
//! `alarm-duration`, `alarm-repeat`, `alarm-email`, `alarm-relay`,
//! `alarm-review`, `alarm-confirm`.
//!
//! New keys, for elements Rune left without one: `alarm-metric` (the metric
//! buttons, now one choice), `alarm-language` (the three language buttons,
//! now one choice), `alarm-close`, `alarm-back`, `alarm-reload`,
//! `alarm-refresh-service` (the way to the refresh command, which in Rune
//! was the third tab), `alarm-change-review` (so `alarm-change-review-stale`),
//! `alarm-resource`, `alarm-policy-error`, `alarm-custom`, `alarm-trigger`,
//! `alarm-retained`, `alarm-email-error`, `alarm-refresh-about`,
//! `alarm-preview`, `alarm-error`.
//! `alarm-resource-<index>` ("Configure alarms" of the picker) is gone with
//! the picker.

use xenadmin_core::{AlarmMetric, PermissionAssessment};

use super::alarms::{self, ALERTS, EMAIL_OPTIONS, Page, Workspace};
use super::commands::ALARM_SERVICE_REFRESH;
use crate::prelude::*;

pub const REFRESH_DIALOG: &str = "alarm-service-refresh";
pub const REVIEW: &str = "alarm-change-review";

const PREVIEW: &str = "Synthetic preview · no host changes can be submitted";
const DEMO: &str = "Demo mode cannot change infrastructure.";

fn workspace(m: &mut Model) -> Option<&mut Workspace> {
    m.areas.schedules_alarms.alarms.as_mut()
}

/// The pages this area has for the Properties dialog of a target. Alerts:
/// one VM, server or repository that has an alarm policy in the inventory
/// of its pool (Rune listed exactly those in its picker). Email Options: a
/// pool whose alarm email configuration the inventory has. Both are outside
/// the supported part.
pub fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    let Some(object) = target.single() else {
        return Vec::new();
    };
    let Some(inventory) = object.pool.and_then(|pool| m.inventory_for(pool)) else {
        return Vec::new();
    };
    let Some(kind) = PropertiesKind::of(target) else {
        return Vec::new();
    };
    let id = match kind {
        PropertiesKind::Vm | PropertiesKind::Host | PropertiesKind::Storage
            if !alarms::policies_of(&inventory, object).is_empty() =>
        {
            ALERTS
        }
        PropertiesKind::Pool if inventory.details.alarm_email.is_some() => EMAIL_OPTIONS,
        _ => return Vec::new(),
    };
    vec![PropertyPage::planned(kind, id).experimental(true)]
}

/// The page that is showing.
pub fn page(m: &Model, _target: &Target, id: &str) -> PageView {
    let alarms = m.areas.schedules_alarms.alarms.as_ref();
    let Some(w) = alarms.filter(|w| w.page.id() == Some(id)) else {
        return PageView::default();
    };
    let permission = alarms::permission(m, w);
    PageView {
        body: body(m, w),
        footer: footer(m, w, permission.as_ref()),
        footer_note: permission.as_ref().map(permissions::footer_note),
    }
}

/// The dialog of "Refresh Alarm Service". The pages are no surface of this
/// area: the kernel describes its dialog from [`page`].
pub fn surface(m: &Model) -> Option<Surface> {
    let w = m.areas.schedules_alarms.alarms.as_ref()?;
    if w.page != Page::Refresh {
        return None;
    }
    let permission = alarms::permission(m, w);
    let surface = Surface::dialog(REFRESH_DIALOG, "Refresh service")
        .experimental(true)
        .body(body(m, w))
        .footer(footer(m, w, permission.as_ref()))
        .on_close(super::close_alarms);
    Some(match &permission {
        Some(permission) => surface.footer_note(permissions::footer_note(permission)),
        None => surface,
    })
}

fn body(m: &Model, w: &Workspace) -> Vec<Block> {
    blocks![
        (w.page == Page::Refresh).then(|| {
            caption(
                "Request the existing host perfmon service to refresh its alarm settings. This \
                 does not install or restart a service.",
            )
            .key("alarm-refresh-about")
        }),
        match &w.review {
            Some(op) => blocks![
                w.page.id().map(|_| heading(op.label())),
                review(REVIEW, alarms::review_lines(w, op)).stale(alarms::stale(m, w)),
            ],
            None if w.page == Page::Email => email(w),
            None => thresholds(m, w),
        },
        w.error.clone().map(|e| alert(e).key("alarm-error")),
        alarms::synthetic(m, w).then(|| notice(NoticeKind::Preview, PREVIEW).key("alarm-preview")),
    ]
}

/// The option of a metric: the policy it belongs to and its place in
/// [`AlarmMetric::ALL`].
fn metric_id(index: usize, metric: AlarmMetric) -> String {
    let place = AlarmMetric::ALL.iter().position(|m| *m == metric);
    format!("{index}:{}", place.unwrap_or_default())
}

fn parse_metric(id: &str) -> Option<(usize, AlarmMetric)> {
    let (index, place) = id.split_once(':')?;
    let metric = AlarmMetric::ALL.get(place.parse::<usize>().ok()?)?;
    Some((index.parse().ok()?, *metric))
}

/// The Alerts page: the alarms of the object, one metric at a time.
fn thresholds(m: &Model, w: &Workspace) -> Vec<Block> {
    let (Some(p), Some(object)) = (&w.policy, &w.object) else {
        return blocks![caption(
            "No inspectable alarm resources match this search"
        )];
    };
    let policies = &w.initial.details.alarm_policies;
    let options: Vec<OptionItem> = alarms::policies_of(&w.initial, object)
        .into_iter()
        .flat_map(|index| {
            let policy = &policies[index];
            AlarmMetric::ALL
                .into_iter()
                .filter(|m| m.supported(policy.target, policy.control_domain_host))
                .map(move |metric| option(metric_id(index, metric), metric.label()))
        })
        .collect();
    let chosen = w.index.map(|index| metric_id(index, w.metric));
    blocks![
        heading(alarms::label(&w.initial, p)),
        caption(p.target.uuid().to_string()).key("alarm-resource"),
        p.error.clone().map(|error| {
            caption(error)
                .tone(Tone::Warning)
                .key("alarm-policy-error")
        }),
        choice("alarm-metric", options, chosen.as_deref(), |m, id| {
            if let Some((index, metric)) = parse_metric(id) {
                alarms::pick(m, index, metric);
            }
        })
        .label("Alarm"),
        checkbox("alarm-enabled", "Enable this alarm", w.enabled, |m| {
            if let Some(w) = workspace(m) {
                w.enabled = !w.enabled;
            }
        }),
        p.customized.contains(&w.metric).then(|| {
            caption(
                "Custom source/direction: editing is unavailable; explicit disabling is \
                 supported.",
            )
            .key("alarm-custom")
        }),
        caption(format!(
            "Trigger when {} is {} the threshold ({})",
            w.metric.label(),
            if w.metric.low_threshold() {
                "below"
            } else {
                "above"
            },
            alarms::units(w.metric)
        ))
        .key("alarm-trigger"),
        input(
            "alarm-threshold",
            "Alarm threshold",
            &w.threshold,
            |m, text| {
                if let Some(w) = workspace(m) {
                    w.threshold = text.to_owned();
                }
            }
        ),
        input(
            "alarm-duration",
            "Duration in seconds",
            &w.duration,
            |m, text| {
                if let Some(w) = workspace(m) {
                    w.duration = text.to_owned();
                }
            }
        ),
        input(
            "alarm-repeat",
            "Minimum repeat interval in seconds",
            &w.repeat,
            |m, text| {
                if let Some(w) = workspace(m) {
                    w.repeat = text.to_owned();
                }
            }
        ),
        caption(format!(
            "{} other/custom variables are retained. Host monitoring runs independently of \
             this desktop.",
            p.other_variables
        ))
        .key("alarm-retained"),
        refresh_link(m, w),
    ]
}

/// The way from the Alerts page to the refresh of the service, which in
/// Rune was the tab beside it. It follows the command's eligibility; the
/// Properties dialog gives way to the command's dialog.
fn refresh_link(m: &Model, w: &Workspace) -> Option<Button> {
    let target = Target::one(ObjectRef::pool(w.pool_id()));
    let reason = match m.eligibility(ALARM_SERVICE_REFRESH, &target) {
        Eligibility::Hidden => return None,
        Eligibility::Disabled(reason) => Some(reason),
        Eligibility::Enabled => None,
    };
    Some(
        button("alarm-refresh-service", "Refresh service…", move |m| {
            m.invoke_from_properties(ALARM_SERVICE_REFRESH, &target);
        })
        .disabled_opt(reason),
    )
}

/// The Email Options page of the pool.
fn email(w: &Workspace) -> Vec<Block> {
    let configuration = w.initial.details.alarm_email.as_ref();
    blocks![
        configuration.and_then(|e| e.error.clone()).map(|error| {
            caption(error)
                .tone(Tone::Warning)
                .key("alarm-email-error")
        }),
        input(
            "alarm-email",
            "Destination email",
            &w.destination,
            |m, text| {
                if let Some(w) = workspace(m) {
                    w.destination = text.to_owned();
                }
            }
        )
        .help("Destination email (empty disables delivery)"),
        input("alarm-relay", "SMTP relay", &w.relay, |m, text| {
            if let Some(w) = workspace(m) {
                w.relay = text.to_owned();
            }
        })
        .help("SMTP relay host[:port] (empty uses host default)"),
        choice(
            "alarm-language",
            [
                ("en-US", "English"),
                ("ja-JP", "Japanese"),
                ("zh-CN", "Simplified Chinese"),
            ],
            Some(w.language.as_str()),
            |m, id| {
                if let Some(w) = workspace(m) {
                    w.language = id.to_owned();
                }
            }
        )
        .label("Email language"),
        caption(
            "Email language applies to host-generated alarm messages. Existing SMTP credentials \
             and sender settings are preserved."
        ),
    ]
}

fn footer(m: &Model, w: &Workspace, permission: Option<&PermissionAssessment>) -> Vec<Button> {
    let mut footer = vec![
        button("alarm-close", "Close", super::close_alarms),
        button("alarm-reload", "Reload settings", alarms::reload),
    ];
    if w.review.is_none() {
        footer.push(button("alarm-review", "Review", alarms::review).primary());
        return footer;
    }
    // The refresh has no draft to go back to: it opened on its review.
    if w.page != Page::Refresh {
        footer.push(button("alarm-back", "Back", alarms::back));
    }
    footer.push(
        button("alarm-confirm", "Confirm change", alarms::submit)
            .danger()
            // The first reason wins: stale, then a known denial, then the demo.
            .disabled_opt(alarms::stale(m, w))
            .disabled_opt(permission.and_then(permissions::denial))
            .disabled_if(alarms::synthetic(m, w), DEMO),
    );
    footer
}
