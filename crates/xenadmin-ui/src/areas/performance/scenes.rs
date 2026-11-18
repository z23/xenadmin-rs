//! The scenes of the area, under their Rune names. Each sets the model up
//! as the Rune scene function did, going the way a user goes where there is
//! one (a selection, a tab, a command).

use xenadmin_core::{HostDataSource, HostRecordingAction, VmId};

use super::commands::MONITORING;
use super::{metrics, monitoring, presets};
use crate::prelude::*;

const AREA: &str = "performance";

/// The guest of the Rune scene `performance`.
pub const GUEST: &str = "30000000-0000-0000-0000-000000000000";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("performance", AREA, performance),
        SceneSpec::new("metric-presets", AREA, metric_presets),
        SceneSpec::new("host-monitoring", AREA, host_monitoring),
        SceneSpec::new("host-monitoring-catalog", AREA, host_monitoring_catalog),
    ]
}

/// The demo pool, one guest selected, its Performance tab: the synthetic
/// preview with four charts, each with a gap.
fn performance(m: &mut Model) {
    m.load_demo(3, 48);
    let Some(pool) = m.inventory.as_ref().map(|i| i.pool_id) else {
        return;
    };
    let Ok(vm) = VmId::parse(GUEST) else {
        return;
    };
    m.select(Target::one(ObjectRef::vm(pool, vm)));
    m.set_active_tab(TabId::Performance);
    // The tab change opened the view; this is for a model whose hooks did
    // not run.
    metrics::ensure(m);
}

/// `performance` with the preset "CPU and memory" (`cpu0`, `memory`) saved
/// and the presets shown.
fn metric_presets(m: &mut Model) {
    performance(m);
    let Some(view) = &mut m.areas.performance.metrics else {
        return;
    };
    view.preset_name = "CPU and memory".into();
    view.options.sources = vec!["cpu0".into(), "memory".into()];
    presets::save(m);
    presets::toggle(m);
}

/// The first server of the demo pool, its recording settings, one source
/// (`cpu7`, recording, standard), and the review of deleting its history.
fn host_monitoring(m: &mut Model) {
    let Some(target) = crate::areas::host_admin::scenes::prepare(m) else {
        return;
    };
    m.invoke(MONITORING, &target);
    let Some(p) = &mut m.areas.performance.monitoring else {
        return;
    };
    p.sources = Some(vec![HostDataSource {
        name: "cpu7".into(),
        description: "Processor 7 utilization".into(),
        units: "fraction".into(),
        standard: true,
        enabled: true,
        minimum: "0".into(),
        maximum: "1".into(),
    }]);
    p.selected = Some("cpu7".into());
    monitoring::review(m, HostRecordingAction::ForgetHistory);
}

/// The same without the review, and a second source (`memory_free_kib`,
/// not recording, maximum "Infinity") selected.
fn host_monitoring_catalog(m: &mut Model) {
    host_monitoring(m);
    let Some(p) = &mut m.areas.performance.monitoring else {
        return;
    };
    p.review = None;
    let Some(sources) = &mut p.sources else {
        return;
    };
    let Some(mut second) = sources.first().cloned() else {
        return;
    };
    second.name = "memory_free_kib".into();
    second.description = "Host free memory".into();
    second.units = "KiB".into();
    second.maximum = "Infinity".into();
    second.standard = false;
    second.enabled = false;
    p.selected = Some(second.name.clone());
    sources.push(second);
}
