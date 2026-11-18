//! The scenes of the area, under their Rune names.
//!
//! Each goes the way a user goes (the demo pool, an object selected, a
//! command or a Properties page) and then sets what the Rune scene set by
//! hand: the coordinator's clock and the name of the schedule.

use std::sync::Arc;

use xenadmin_core::{
    AlarmEmailConfiguration, AlarmEmailSettings, AlarmMetric, AlarmPolicy, AlarmRule,
    ResourceTarget, SnapshotPolicyClock, SnapshotPolicySupport,
};

use super::alarms::{self, ALERTS, EMAIL_OPTIONS};
use super::commands::SNAPSHOT_SCHEDULES;
use super::schedules;
use crate::prelude::*;

const AREA: &str = "schedules_alarms";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("snapshot-schedules", AREA, schedules_scene),
        SceneSpec::new("snapshot-schedule-review", AREA, schedule_review_scene),
        SceneSpec::new("alarms", AREA, alarms_scene),
        SceneSpec::new("alarms-review", AREA, alarms_review_scene),
        SceneSpec::new("alarms-email", AREA, alarms_email_scene),
    ]
}

/// The wizard of a new schedule on its first step, named "Daily recovery",
/// with the answer of the coordinator the demo cannot ask for.
fn schedules_scene(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(pool) = m.inventory.as_ref().map(|i| i.pool_id) else {
        return;
    };
    let target = Target::one(ObjectRef::pool(pool));
    m.select(target.clone());
    m.invoke(SNAPSHOT_SCHEDULES, &target);
    let Some(w) = &mut m.areas.schedules_alarms.schedules else {
        return;
    };
    let Some(coordinator) = w.initial.hosts.keys().next().copied() else {
        return;
    };
    w.support = Some(Ok(SnapshotPolicySupport {
        clock: SnapshotPolicyClock {
            coordinator,
            utc_offset_minutes: 600,
        },
        coordinator_name: "Example coordinator".into(),
        local_time: "20261108T10:00:00".into(),
        utc_time: "20261108T00:00:00Z".into(),
    }));
    schedules::edit(m, true);
    if let Some(w) = &mut m.areas.schedules_alarms.schedules {
        w.name = "Daily recovery".into();
    }
}

/// The review of that schedule: the last step of the wizard.
fn schedule_review_scene(m: &mut Model) {
    schedules_scene(m);
    schedules::review(m);
}

/// The demo pool whose first server has an alarm policy with one CPU rule
/// (0.8 for 60 seconds, repeated after 3600) and one other variable. With
/// `email`, the pool's alarm email configuration too. Returns the pool and
/// that server.
///
/// Public for the tests of the area.
pub fn prepare(m: &mut Model, email: bool) -> Option<(Target, Target)> {
    m.load_demo(3, 48);
    let mut inventory = m.inventory.as_ref()?.as_ref().clone();
    let host = *inventory.hosts.keys().next()?;
    inventory.details.alarm_policies.push(AlarmPolicy {
        target: ResourceTarget::Host(host),
        control_domain_host: None,
        revision: "0".repeat(64),
        rules: vec![AlarmRule {
            metric: AlarmMetric::Cpu,
            threshold: "0.8".into(),
            duration_seconds: 60,
            repeat_seconds: 3600,
        }],
        customized: vec![],
        other_variables: 1,
        error: None,
    });
    if email {
        inventory.details.alarm_email = Some(AlarmEmailConfiguration {
            revision: "0".repeat(64),
            settings: Some(AlarmEmailSettings {
                destination: "operator@example.invalid".into(),
                relay: "smtp.example.invalid:25".into(),
                language: "en-US".into(),
            }),
            error: None,
        });
    }
    let pool = inventory.pool_id;
    m.inventory = Some(Arc::new(inventory));
    Some((
        Target::one(ObjectRef::pool(pool)),
        Target::one(ObjectRef::host(pool, host)),
    ))
}

/// The Alerts page of that server: the threshold editor of its CPU alarm.
fn alarms_scene(m: &mut Model) {
    if let Some((_, host)) = prepare(m, false) {
        m.select(host.clone());
        m.open_properties(&host, Some(ALERTS));
    }
}

/// The review of that alarm, unchanged.
fn alarms_review_scene(m: &mut Model) {
    alarms_scene(m);
    alarms::review(m);
}

/// The Email Options page of the pool.
fn alarms_email_scene(m: &mut Model) {
    if let Some((pool, _)) = prepare(m, true) {
        m.select(pool.clone());
        m.open_properties(&pool, Some(EMAIL_OPTIONS));
    }
}
