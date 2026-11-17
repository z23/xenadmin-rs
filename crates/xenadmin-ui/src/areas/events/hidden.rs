//! The key of a history entry the user hid: the data `Preferences` stores.
//!
//! These are `Family` and `HiddenKey` of the Rune `activity.rs`, moved here
//! ahead of the `events` area because `Preferences` holds a list of them
//! and the stored file format must not change. Hiding and restoring
//! entries in the Events tab is still to be ported and builds on this.

use serde::{Deserialize, Serialize};
use xenadmin_core::{OperationId, PoolId};

/// Which of the two record families an operation belongs to.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Family {
    Vm,
    Resource,
}
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct HiddenKey {
    pub pool: PoolId,
    pub id: OperationId,
    pub family: Family,
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_hidden_key_keeps_its_stored_form() {
        let stored = r#"{"pool":"10000000-0000-0000-0000-000000000001","id":"81000000-0000-0000-0000-000000000001","family":"Vm"}"#;
        let key: HiddenKey = serde_json::from_str(stored).unwrap();
        assert_eq!(key.family, Family::Vm);
        assert_eq!(key.pool.to_string(), "10000000-0000-0000-0000-000000000001");
        assert_eq!(serde_json::to_string(&key).unwrap(), stored);
        let other: HiddenKey = serde_json::from_str(&stored.replace("Vm", "Resource")).unwrap();
        assert!(other != key);
    }
}
