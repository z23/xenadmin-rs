//! The four scenes of the area, under their Rune names. Each sets the model
//! up as the Rune scene function did and opens its dialog through the
//! command a user would use.

use std::sync::Arc;

use xenadmin_core::{
    ClassAvailability, GuestToolsMediaSource, PbdId, PowerState, SrId, StorageAttachment, VdiId,
    VmId,
};

use super::command::Catalog;
use super::commands::{CONNECT_EXTERNAL, EXTERNAL_TOOLS, INSTALL_GUEST_TOOLS};
use super::external;
use super::media;
use crate::prelude::*;

const AREA: &str = "vm_tools";

/// The demo VM the guest tools scenes act on.
pub const MEDIA_VM: &str = "30000000-0000-0000-0000-000000000000";

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("guest-connections", AREA, guest_connections),
        SceneSpec::new("external-tools", AREA, external_tools),
        SceneSpec::new("guest-tools", AREA, guest_tools),
        SceneSpec::new("guest-tools-review", AREA, guest_tools_review),
    ]
}

/// The demo pool with what mounting tools media needs and the plain demo
/// does not have: an ISO repository with "guest-tools.iso", advertised by
/// the host as guest tools, and its attachment. The demo VM is selected.
/// Returns it as the target. Tests start from here too.
pub fn prepare_media(m: &mut Model) -> Option<Target> {
    m.load_demo(3, 48);
    let mut i = m.inventory.as_ref()?.as_ref().clone();
    let vm = VmId::parse(MEDIA_VM).ok()?;
    let sr = SrId::from(uuid::Uuid::from_u128(0x76000000000000000000000000000001));
    let image = VdiId::from(uuid::Uuid::from_u128(0x76000000000000000000000000000002));
    let pbd = PbdId::from(uuid::Uuid::from_u128(0x76000000000000000000000000000003));
    let mut storage = i.details.storage.values().next()?.clone();
    storage.id = sr;
    storage.content_type = "iso".into();
    i.details.storage.insert(sr, storage);
    let mut disk = i.details.vdis.values().next()?.clone();
    disk.id = image;
    disk.sr = Some(sr);
    disk.name = "guest-tools.iso".into();
    disk.read_only = Some(true);
    disk.missing = Some(false);
    i.details.vdis.insert(image, disk);
    i.details
        .tools_media
        .insert(image, GuestToolsMediaSource::HostFlag);
    i.details.pbds.insert(
        pbd,
        StorageAttachment {
            id: pbd,
            location: None,
            requires_credentials: false,
            multipathed: None,
            paths: vec![],
            host: i.hosts.keys().next().copied(),
            sr: Some(sr),
            currently_attached: Some(true),
        },
    );
    i.details
        .classes
        .insert("pbd".into(), ClassAvailability::Available);
    let target = Target::one(ObjectRef::vm(i.pool_id, vm));
    m.inventory = Some(Arc::new(i));
    m.select(target.clone());
    Some(target)
}

/// The chooser: the CD drive preselected, the tools ISO not.
fn guest_tools(m: &mut Model) {
    if let Some(target) = prepare_media(m) {
        m.invoke(INSTALL_GUEST_TOOLS, &target);
    }
}

/// The review of the mount.
fn guest_tools_review(m: &mut Model) {
    guest_tools(m);
    if let Some(p) = &mut m.areas.vm_tools.media {
        p.image = p.inventory.details.tools_media.keys().next().copied();
    }
    media::review(m);
}

/// The demo pool with the default declarations and its first running guest
/// selected. Returns that guest as the target.
pub fn prepare_guest(m: &mut Model) -> Option<Target> {
    m.load_demo(3, 48);
    m.preferences.external_tools = Catalog::default();
    let i = m.inventory.clone()?;
    let vm = i
        .guests()
        .find(|v| v.power_state == PowerState::Running)?
        .id;
    let target = Target::one(ObjectRef::vm(i.pool_id, vm));
    m.select(target.clone());
    Some(target)
}

/// The launch dialog for the first running guest.
fn guest_connections(m: &mut Model) {
    let Some(target) = prepare_guest(m) else {
        return;
    };
    m.invoke(CONNECT_EXTERNAL, &target);
    if let Some(p) = external::launch_mut(m) {
        p.address = "192.0.2.23".into();
        p.user = "operator".into();
    }
}

/// The declarations dialog, editing the SSH preset.
fn external_tools(m: &mut Model) {
    m.load_demo(3, 48);
    m.preferences.external_tools = Catalog::default();
    let target = m.target();
    m.invoke(EXTERNAL_TOOLS, &target);
}
