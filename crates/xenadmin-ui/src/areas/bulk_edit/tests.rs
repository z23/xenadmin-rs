//! Tests of the area. The Rune tests of `bulk_delete_tests.rs` and
//! `bulk_properties_tests.rs` are here under their names; the others are
//! new for what the port changed (commands, the page).

use std::sync::Arc;

use xenadmin_core::{ClassAvailability, Inventory, PowerState, VmId, VmSettingEdit, VmWorkflow};

use super::commands::{DELETE, TEMPLATES_DELETE};
use super::property::Property;
use super::{delete, edit};
use crate::{Harness, prelude::*};

fn amend(h: &mut Harness, change: impl FnOnce(&mut Inventory)) {
    super::scenes::amend(&mut h.model, change);
}
