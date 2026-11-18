//! The tests the area could not have without an observed VM: against the
//! fixture server with a VM (`crate::testing::Live`), a real `Backend` and
//! a current session.
//!
//! The server has one host and one halted VM without disks, interfaces or
//! networks, and serves no migration: the wizard is tested for what it
//! captures, for its eligibility on a writable and on a read-only session,
//! and up to the review the core refuses. No migration and no batch is
//! submitted here.

use uuid::Uuid;
use xenadmin_core::{Error, PoolId, ResourceTarget};

use super::commands::{MIGRATE, MOVE};
use super::{batch, view, wizard};
use crate::prelude::*;
use crate::testing::{Live, Server};

#[test]
fn a_writable_current_session_opens_the_wizard_on_the_observed_vm() {
    let mut live = Live::with_vm();
    let (pool, vm, host) = (live.pool, live.server.vm(), live.server.host());
    live.h.select(Target::one(live.vm()));
    assert_eq!(live.h.eligibility(MIGRATE.0), Eligibility::Enabled);
    assert_eq!(live.h.eligibility(MOVE.0), Eligibility::Enabled);
    live.h.invoke(MOVE.0);
    let surface = live.h.surface();
    assert_eq!(
        (surface.id.as_str(), surface.title.as_str()),
        (view::WIZARD, "Move halted VM")
    );
    let observed = live
        .h
        .model
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_vm(vm).ok());
    assert!(observed.is_some());
    let f = live.h.model.areas.migration.wizard.as_deref().unwrap();
    assert_eq!((f.pool, f.source.id, f.target), (pool, vm, observed));
    assert!(wizard::current(&live.h.model, f));
    assert!(live.h.find("migration-stale").is_none());
    assert_eq!(live.h.node("migration-next").disabled, None);

    // A halted move needs a transfer network, and this pool has none: the
    // core refuses the review, in the wizard's error line.
    live.h.click(&format!("migration-host-{host}"));
    live.h.click("migration-next");
    live.h.click("migration-next");
    let f = live.h.model.areas.migration.wizard.as_deref().unwrap();
    assert_eq!((f.stage, f.review.is_none()), (1, true));
    assert!(f.error.is_some());
    assert!(live.h.find("migration-error").is_some());
    // Nothing was submitted, and applying without a review is inert.
    wizard::apply(&mut live.h.model);
    assert!(live.h.model.operations.is_empty());
    assert!(
        live.server
            .calls()
            .iter()
            .all(|call| !call.to_lowercase().contains("migrate"))
    );
}

#[test]
fn a_read_only_session_cannot_open_the_wizard() {
    let pool = PoolId::from(Uuid::new_v4());
    let mut live = Live::over(Server::with_vm(pool), pool, true);
    live.h.select(Target::one(live.vm()));
    for command in [MIGRATE, MOVE] {
        assert_eq!(
            live.h.eligibility(command.0),
            Eligibility::disabled(wizard::NOT_WRITABLE)
        );
        assert_eq!(
            live.h.try_invoke(command.0),
            Err(wizard::NOT_WRITABLE.to_owned())
        );
    }
    assert!(live.h.model.areas.migration.wizard.is_none());
}

/// The batch of a live pool captures the pool's observation and is
/// current; without a review its submission is refused before anything
/// reaches the connection.
#[test]
fn a_batch_of_a_live_pool_is_current_and_submits_nothing_without_a_review() {
    let mut live = Live::with_vm();
    let pool = live.pool;
    let target = Target::one(live.vm());
    live.h.select(target.clone());
    batch::open(&mut live.h.model, &target);
    let e = live.h.model.areas.migration.batch.as_ref().unwrap();
    let observed = live
        .h
        .model
        .snapshot_for(pool)
        .and_then(|snapshot| snapshot.observe_resource(ResourceTarget::Pool(pool)).ok());
    assert!(observed.is_some());
    assert_eq!((e.pool, e.observed, e.choices.len()), (pool, observed, 1));
    assert!(batch::current(&live.h.model, e));
    assert!(live.h.find("batch-move-stale").is_none());
    assert!(live.h.find("batch-move-preview").is_none());
    batch::submit(&mut live.h.model);
    let e = live.h.model.areas.migration.batch.as_ref().unwrap();
    assert_eq!(e.error, Some(Error::Stale.to_string()));
    assert!(live.h.model.resource_operations.is_empty());
}
