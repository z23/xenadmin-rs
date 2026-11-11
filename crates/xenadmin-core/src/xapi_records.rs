//! Views of XAPI records by class.
//!
//! A view wraps the record a host sent and gives every field the client reads from that
//! class a method, so that a read says which class it is of and a field that is not
//! listed here cannot be read. The methods return the value as the host sent it: bounds
//! and the refusal of malformed values stay with the decoders.
//!
//! The views are written by hand. The conformance check compares each one with the
//! upstream API description of the qualified toolstack, which is fetched for that check
//! and is not kept in this repository.

use serde_json::Value;

macro_rules! record_views {
    ($(
        $(#[$meta:meta])*
        $view:ident = $class:literal { $( $method:ident = $field:literal ),+ $(,)? }
    )+) => {
        $(
            $(#[$meta])*
            #[derive(Clone, Copy)]
            pub(crate) struct $view<'a>(&'a Value);
            impl<'a> $view<'a> {
                pub(crate) fn new(record: &'a Value) -> Self {
                    Self(record)
                }
                $(
                    pub(crate) fn $method(self) -> &'a Value {
                        &self.0[$field]
                    }
                )+
            }
        )+
        /// Every view's class with the fields it reads, for the conformance check.
        #[cfg(test)]
        pub(crate) const VIEWS: &[(&str, &[&str])] = &[ $( ($class, &[ $( $field ),+ ]) ),+ ];
    };
}

record_views! {
    /// A guest, a template, a snapshot or a control domain: XAPI keeps all four in one class.
    VmRecord = "VM" {
        uuid = "uuid",
        is_a_snapshot = "is_a_snapshot",
        is_a_template = "is_a_template",
        is_control_domain = "is_control_domain",
        power_state = "power_state",
        allowed_operations = "allowed_operations",
        snapshot_of = "snapshot_of",
        snapshot_time = "snapshot_time",
    }
    HostRecord = "host" {
        uuid = "uuid",
        enabled = "enabled",
        software_version = "software_version",
    }
    TaskRecord = "task" {
        result = "result",
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn a_view_reads_the_named_field_and_a_missing_one_is_null() {
        let record = json!({"uuid": "u", "power_state": "Halted", "is_a_snapshot": false});
        let vm = VmRecord::new(&record);
        assert_eq!(vm.uuid(), "u");
        assert_eq!(vm.power_state(), "Halted");
        assert_eq!(*vm.is_a_snapshot(), false);
        assert!(vm.snapshot_of().is_null());
        assert!(VmRecord::new(&Value::Null).uuid().is_null());
        assert!(
            VIEWS
                .iter()
                .any(|(class, fields)| *class == "VM" && fields.contains(&"uuid"))
        );
    }
}
