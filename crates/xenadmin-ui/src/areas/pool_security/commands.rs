//! The commands of the area: what the buttons of the Rune "Pool security"
//! dialog became.
//!
//! The six commands of a pool are on the local row of its Users tab (the
//! plan's Menus table has no item for them); the two of a subject are also
//! the row menu of the subject grid and read the row chosen there (a
//! subject is no tree object). The two certificate commands act on a server
//! and are in the Server menu.
//!
//! Eligibility is one rule: `Hidden` when the target is not one pool (one
//! server); `Disabled` with the reason of [`security::gate`] when nothing
//! the user can enter would let the change be reviewed, or with the reason
//! a query would be refused while no security state has been read;
//! `Enabled` otherwise. The open dialog makes every check again.
//!
//! The labels are new: in Rune each change was a "Review …" button beside
//! its inputs, and that button, with its label, is now the review button
//! of the dialog a command opens.

use xenadmin_core::{HostId, PoolId};

use super::security::{self, Editor, Kind};
use crate::prelude::*;

const AREA: &str = "pool_security";

pub const REFRESH: CommandId = CommandId("pool.security.refresh");
pub const DIRECTORY_JOIN: CommandId = CommandId("pool.directory.join");
pub const DIRECTORY_LEAVE: CommandId = CommandId("pool.directory.leave");
pub const SUBJECT_ADD: CommandId = CommandId("pool.subject.add");
pub const SUBJECT_ROLE: CommandId = CommandId("pool.subject.role");
pub const SUBJECT_REMOVE: CommandId = CommandId("pool.subject.remove");
pub const CERTIFICATE_INSTALL: CommandId = CommandId("server.certificate.install");
pub const CERTIFICATE_RESET: CommandId = CommandId("server.certificate.reset");

/// The `connections` area's command the tab hands over to.
pub const RECONNECT_AS: CommandId = CommandId("pool.reconnect-as");

/// In the order of the local row.
pub fn commands() -> Vec<CommandSpec> {
    vec![
        CommandSpec::new(REFRESH.0, "Refresh security status", AREA)
            .local(TabId::Users, 10)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(can_refresh)
            .invoke(|m, target| {
                if let Some(pool) = pool_of(target) {
                    security::open(m, pool);
                }
            }),
        CommandSpec::new(DIRECTORY_JOIN.0, "Join Active Directory…", AREA)
            .local(TabId::Users, 20)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| of_pool(m, target, Kind::Join))
            .invoke(|m, target| open(m, target, Kind::Join)),
        CommandSpec::new(DIRECTORY_LEAVE.0, "Leave Active Directory…", AREA)
            .local(TabId::Users, 30)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| of_pool(m, target, Kind::Leave))
            .invoke(|m, target| open(m, target, Kind::Leave)),
        CommandSpec::new(SUBJECT_ADD.0, "Add Directory Subject…", AREA)
            .local(TabId::Users, 40)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| of_pool(m, target, Kind::AddSubject))
            .invoke(|m, target| open(m, target, Kind::AddSubject)),
        CommandSpec::new(SUBJECT_ROLE.0, "Change Subject Roles…", AREA)
            .local(TabId::Users, 50)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| of_pool(m, target, Kind::Role))
            .invoke(|m, target| open(m, target, Kind::Role)),
        CommandSpec::new(SUBJECT_REMOVE.0, "Remove Directory Subject…", AREA)
            .local(TabId::Users, 60)
            .local_scopes(&[ObjectKind::Pool])
            .eligibility(|m, target| of_pool(m, target, Kind::RemoveSubject))
            .invoke(|m, target| open(m, target, Kind::RemoveSubject)),
        CommandSpec::new(CERTIFICATE_INSTALL.0, "Install TLS Certificate…", AREA)
            .menu(MenuPath::new(Menu::Server, 5, 50))
            .eligibility(|m, target| of_server(m, target, Kind::InstallCertificate))
            .invoke(|m, target| open_for_server(m, target, Kind::InstallCertificate)),
        CommandSpec::new(CERTIFICATE_RESET.0, "Reset TLS Certificate…", AREA)
            .menu(MenuPath::new(Menu::Server, 5, 60))
            .eligibility(|m, target| of_server(m, target, Kind::ResetCertificate))
            .invoke(|m, target| open_for_server(m, target, Kind::ResetCertificate)),
    ]
}

/// The pool a command of the Users tab acts on.
fn pool_of(target: &Target) -> Option<PoolId> {
    target.single_of(ObjectKind::Pool)?.pool
}

/// The server a certificate command acts on, with its pool, when the
/// inventory of that pool has it.
fn server_of(m: &Model, target: &Target) -> Option<(PoolId, HostId)> {
    let object = target.single_of(ObjectKind::Host)?;
    let (pool, host) = (object.pool?, object.host_id()?);
    m.inventory_for(pool)?
        .hosts
        .contains_key(&host)
        .then_some((pool, host))
}

/// The security state that has been read for a pool.
fn state(m: &Model, pool: PoolId) -> Option<&Editor> {
    m.areas
        .pool_security
        .editor
        .as_ref()
        .filter(|e| e.pool == pool)
}

/// Rune disabled the button while a query ran; one that could only be
/// refused says why before it is pressed.
fn can_refresh(m: &Model, target: &Target) -> Eligibility {
    let Some(pool) = pool_of(target) else {
        return Eligibility::Hidden;
    };
    if state(m, pool).is_some_and(|e| e.query.is_some()) {
        return Eligibility::disabled(security::READING);
    }
    Eligibility::enabled_unless(security::refusal(m, pool).map(str::to_owned))
}

fn of_pool(m: &Model, target: &Target, kind: Kind) -> Eligibility {
    let Some(pool) = pool_of(target) else {
        return Eligibility::Hidden;
    };
    match state(m, pool) {
        Some(e) => Eligibility::enabled_unless(security::gate(e, kind)),
        None => Eligibility::disabled(security::refusal(m, pool).unwrap_or(security::UNREAD)),
    }
}

fn open(m: &mut Model, target: &Target, kind: Kind) {
    if let Some(pool) = pool_of(target) {
        security::begin(m, pool, kind, None);
    }
}

/// A certificate command can run before the security state of its pool was
/// read: its dialog reads it. Once it is there, it decides.
fn of_server(m: &Model, target: &Target, kind: Kind) -> Eligibility {
    let Some((pool, _)) = server_of(m, target) else {
        return Eligibility::Hidden;
    };
    match state(m, pool).filter(|e| e.overview.is_some()) {
        Some(e) => Eligibility::enabled_unless(security::gate(e, kind)),
        None => Eligibility::enabled_unless(security::refusal(m, pool).map(str::to_owned)),
    }
}

fn open_for_server(m: &mut Model, target: &Target, kind: Kind) {
    if let Some((pool, host)) = server_of(m, target) {
        security::begin(m, pool, kind, Some(host));
    }
}
