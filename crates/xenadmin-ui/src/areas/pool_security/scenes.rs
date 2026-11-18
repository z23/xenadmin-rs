//! The scenes of the area, under their Rune names.
//!
//! Rune `pool_security_view::scene(s, review)`: the demo, the security
//! editor opened, and a synthetic overview (local superuser, two supported
//! methods, every member on local authentication). The editor is the Users
//! tab of the demo pool now; the review scene has the Join dialog on its
//! review, staged as Rune staged it.

use xenadmin_core::{DirectoryHostState, PoolSecurityOperation as SecurityOp, SecurityOverview};

use super::security::{self, Kind};
use crate::prelude::*;

pub fn scenes() -> Vec<SceneSpec> {
    vec![
        SceneSpec::new("pool-security", "pool_security", editing),
        SceneSpec::new("pool-security-review", "pool_security", reviewing),
    ]
}

fn editing(m: &mut Model) {
    scene(m, false);
}

fn reviewing(m: &mut Model) {
    scene(m, true);
}

pub fn scene(m: &mut Model, review: bool) {
    m.load_demo(3, 48);
    let Some(inventory) = m.inventory.clone() else {
        return;
    };
    let pool = inventory.pool_id;
    m.select(Target::one(ObjectRef::pool(pool)));
    m.set_active_tab(TabId::Users);
    if !security::open(m, pool) {
        return;
    }
    let Some(e) = &mut m.areas.pool_security.editor else {
        return;
    };
    e.error = None;
    e.overview = Some(SecurityOverview {
        is_local_superuser: true,
        supported_methods: vec![
            "pool.enable_external_auth".into(),
            "host.reset_server_certificate".into(),
        ],
        directory_hosts: e
            .inventory
            .hosts
            .keys()
            .map(|host| DirectoryHostState {
                host: *host,
                auth_type: String::new(),
                service_name: String::new(),
            })
            .collect(),
        ..Default::default()
    });
    if review {
        let expected_hosts = e
            .overview
            .as_ref()
            .map(|v| v.directory_hosts.clone())
            .unwrap_or_default();
        security::begin(m, pool, Kind::Join, None);
        if let Some(e) = &mut m.areas.pool_security.editor {
            e.domain = "example.test".into();
            e.username = "directory-admin".into();
        }
        security::stage(
            m,
            SecurityOp::JoinDirectory {
                domain: "example.test".into(),
                expected_hosts,
            },
        );
    }
}
