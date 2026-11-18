//! The commands of the area: the buttons of the Rune Performance view's
//! control row, and the way to a server's recording settings.
//!
//! The three `performance.*` commands act on the view of the Performance
//! tab, which is pinned to the scope object. They are on that tab's local
//! row only: `Hidden` for a target that is not one server or one VM,
//! `Disabled` while no view is open for it (its connection is not current).
//!
//! `server.monitoring-recording` is `Hidden` unless the target is one
//! server its pool's inventory has. It is never disabled: the dialog it
//! opens reads the catalog, which needs no write access, and says inside
//! what cannot be changed, as the Rune dialog did.

use super::{metrics, monitoring, presets};
use crate::prelude::*;

const AREA: &str = "performance";

pub const DATA_SOURCES: CommandId = CommandId("performance.data-sources");
pub const PRESETS: CommandId = CommandId("performance.presets");
pub const RECONNECT: CommandId = CommandId("performance.reconnect");
pub const MONITORING: CommandId = CommandId("server.monitoring-recording");

/// Why a command of the view cannot run while there is no view: what
/// disabled the Rune "Performance" button without saying so.
pub const NOT_CURRENT: &str =
    "The connection is not current. Performance data resumes when it reconnects.";
/// Why there is nothing to reconnect in a synthetic pool (Rune disabled the
/// button without saying so).
pub const NO_SUBSCRIPTION: &str =
    "Synthetic performance preview: there is no subscription to reconnect.";

/// The Server menu has no item of the plan for it; it sits with the other
/// changes of a server (group 5), after them.
const MENU: MenuPath = MenuPath::new(Menu::Server, 5, 50);

pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(DATA_SOURCES.0, "Data Sources…", AREA)
            .local(TabId::Performance, 10)
            .local_scopes(&[ObjectKind::Host, ObjectKind::Vm])
            .eligibility(view_eligibility)
            .checked(|m| view(m).is_some_and(|view| view.choosing_sources))
            .invoke(|m, _| {
                if view(m).is_some_and(|view| view.choosing_sources) {
                    metrics::cancel_sources(m);
                } else {
                    metrics::choose_sources(m);
                }
            }),
        CommandSpec::new(PRESETS.0, "Graph Presets…", AREA)
            .local(TabId::Performance, 20)
            .local_scopes(&[ObjectKind::Host, ObjectKind::Vm])
            .eligibility(view_eligibility)
            .checked(|m| view(m).is_some_and(|view| view.show_presets))
            .invoke(|m, _| presets::toggle(m)),
        CommandSpec::new(RECONNECT.0, "Reconnect Metrics", AREA)
            .local(TabId::Performance, 30)
            .local_scopes(&[ObjectKind::Host, ObjectKind::Vm])
            .eligibility(can_reconnect)
            .invoke(|m, _| metrics::reload(m)),
        CommandSpec::new(MONITORING.0, "Monitoring Recording…", AREA)
            .menu(MENU)
            .local(TabId::Performance, 40)
            .local_scopes(&[ObjectKind::Host])
            .eligibility(|m, target| {
                let known = target
                    .single()
                    .is_some_and(|object| monitoring::capture(m, object).is_some());
                if known {
                    Eligibility::Enabled
                } else {
                    Eligibility::Hidden
                }
            })
            .invoke(monitoring::open),
    ]
}

fn view(m: &Model) -> Option<&metrics::MetricsUi> {
    m.areas.performance.metrics.as_ref()
}

/// A command of the view: for one server or one VM, while its view is open.
fn view_eligibility(m: &Model, target: &Target) -> Eligibility {
    let Some((pool, wanted)) = target.single().and_then(metrics::target_of) else {
        return Eligibility::Hidden;
    };
    Eligibility::enabled_if(metrics::view_of(m, pool, wanted).is_some(), NOT_CURRENT)
}

/// "Reconnect metrics": not in a synthetic pool, and only with a current
/// snapshot, as in Rune.
fn can_reconnect(m: &Model, target: &Target) -> Eligibility {
    let Some((pool, wanted)) = target.single().and_then(metrics::target_of) else {
        return Eligibility::Hidden;
    };
    if m.is_synthetic_pool(pool) {
        return Eligibility::disabled(NO_SUBSCRIPTION);
    }
    let current = m.snapshot_for(pool).is_some_and(|s| s.is_current());
    Eligibility::enabled_if(
        current && metrics::view_of(m, pool, wanted).is_some(),
        NOT_CURRENT,
    )
}
