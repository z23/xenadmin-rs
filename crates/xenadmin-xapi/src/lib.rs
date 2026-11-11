// SPDX-License-Identifier: LGPL-2.1-only WITH OCaml-LGPL-linking-exception

//! The XAPI of the qualified toolstack, as Rust: a catalogue of its classes, fields,
//! messages and enumerations, a view of a record for every class, and constants for every
//! enumeration value.
//!
//! All of it is generated when the crate is built, from the upstream API description kept
//! unchanged in `data/xenapi.json`. See `NOTICE` for where that file comes from and
//! `LICENSE` for the terms of this crate, which differ from the rest of the workspace.
//!
//! The views return values as the host sent them. What a value must look like, how large
//! it may be and what happens when it is malformed are for the caller to decide.

/// One class of the API.
pub struct Class {
    pub name: &'static str,
    pub fields: &'static [Field],
    pub messages: &'static [Message],
    /// The names of the enumerations its fields and messages use.
    pub enumerations: &'static [&'static str],
}
/// One field of a class, with its type as the description writes it.
pub struct Field {
    pub name: &'static str,
    pub kind: &'static str,
    /// `RW`, `RO/runtime` or `RO/constructor`.
    pub qualifier: &'static str,
    /// Where the field is in its lifecycle, such as `Published_s` or `Deprecated_s`.
    pub state: &'static str,
}
/// One message of a class: a call. `parameters` are the types in order, the session
/// included where the call takes one.
pub struct Message {
    pub name: &'static str,
    pub parameters: &'static [&'static str],
    pub result: &'static str,
    pub state: &'static str,
}
/// One enumeration with its values in the API's order.
pub struct Enumeration {
    pub name: &'static str,
    pub values: &'static [&'static str],
}

/// One field of a record, as the host sent it: null when the record does not carry it.
///
/// A field is not a record. Code that reads a field takes this type, so a whole record
/// cannot be handed to it by mistake. It dereferences to the JSON value for reading.
#[derive(Clone, Copy, Debug)]
pub struct FieldValue<'a>(&'a serde_json::Value);
impl<'a> FieldValue<'a> {
    /// A value that did not come through a record view: a key inside a map-valued field,
    /// or a read by key where the record's class is not known.
    pub fn of(value: &'a serde_json::Value) -> Self {
        Self(value)
    }
    /// The value with the record's lifetime, for a result that outlives this field.
    pub fn value(self) -> &'a serde_json::Value {
        self.0
    }
}
impl std::ops::Deref for FieldValue<'_> {
    type Target = serde_json::Value;
    fn deref(&self) -> &serde_json::Value {
        self.0
    }
}
impl<T> PartialEq<T> for FieldValue<'_>
where
    serde_json::Value: PartialEq<T>,
{
    fn eq(&self, other: &T) -> bool {
        self.0 == other
    }
}
/// A JSON value compares with a field as with the value the field wraps. With the
/// comparison above, this also lets two fields be compared.
impl PartialEq<FieldValue<'_>> for serde_json::Value {
    fn eq(&self, other: &FieldValue<'_>) -> bool {
        self == other.0
    }
}
/// A field is written out as the value it wraps, so it can be sent back in a call.
impl serde::Serialize for FieldValue<'_> {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        self.0.serialize(serializer)
    }
}

/// The upstream tag, commit and digest of the description the bindings were generated from.
pub const SOURCE: &str = include_str!("../data/SOURCE");

include!(concat!(env!("OUT_DIR"), "/bindings.rs"));

#[cfg(test)]
#[path = "../generator.rs"]
mod generator;

#[cfg(test)]
mod tests {
    use super::*;
    use serde_json::{Value, json};

    #[test]
    fn the_catalogue_is_the_whole_description_in_order() {
        assert_eq!(CLASSES.len(), 67);
        assert_eq!(CLASSES.iter().map(|c| c.fields.len()).sum::<usize>(), 845);
        assert_eq!(
            CLASSES.iter().map(|c| c.messages.len()).sum::<usize>(),
            1828
        );
        assert!(CLASSES.windows(2).all(|pair| pair[0].name < pair[1].name));
        assert!(
            ENUMERATIONS
                .windows(2)
                .all(|pair| pair[0].name < pair[1].name)
        );
        assert!(SOURCE.contains("tag=v26.1.11"));
        let vm = CLASSES.iter().find(|c| c.name == "VM").unwrap();
        let start = vm.messages.iter().find(|m| m.name == "start").unwrap();
        assert_eq!(start.parameters, ["session ref", "VM ref", "bool", "bool"]);
        assert_eq!(start.result, "void");
        assert!(vm.enumerations.contains(&"vm_power_state"));
        let field = vm.fields.iter().find(|f| f.name == "VCPUs_max").unwrap();
        assert_eq!((field.kind, field.qualifier), ("int", "RO/constructor"));
    }

    #[test]
    fn a_view_reads_its_class_fields_and_a_missing_field_is_null() {
        let record =
            json!({"uuid": "u", "power_state": "Halted", "VCPUs_max": 2, "is_a_snapshot": false});
        let vm = records::VmRecord::new(&record);
        assert_eq!(records::VmRecord::CLASS, "VM");
        assert_eq!(vm.uuid(), "u");
        assert_eq!(vm.power_state(), enumerations::vm_power_state::HALTED);
        assert_eq!(vm.vcpus_max(), 2);
        assert_eq!(vm.is_a_snapshot(), false);
        assert!(vm.snapshot_of().is_null());
        assert!(records::HostRecord::new(&Value::Null).enabled().is_null());
        assert_eq!(vm.record(), &record);
        // A field reads like the value and keeps the record's lifetime when asked.
        let name: &str = vm.uuid().value().as_str().unwrap();
        assert_eq!(name, "u");
        assert_eq!(FieldValue::of(&record["VCPUs_max"]), 2);
        assert_eq!(vm.vcpus_max().as_u64(), Some(2));
        // Two fields compare as their values do.
        assert!(vm.uuid() == records::VmRecord::new(&record).uuid());
        assert!(vm.uuid() != vm.power_state());
        // A JSON value compares with a field, and a field serializes as its value.
        assert!(json!("u") == vm.uuid());
        assert_eq!(json!([vm.uuid(), vm.vcpus_max()]), json!(["u", 2]));
        // A field whose name is a Rust keyword keeps its meaning with an underscore.
        assert_eq!(
            records::VdiRecord::new(&json!({"type": "user"})).type_(),
            "user"
        );
    }

    #[test]
    fn enumeration_constants_are_the_api_values() {
        use enumerations::vm_power_state;
        assert_eq!(
            vm_power_state::ALL,
            ["Halted", "Paused", "Running", "Suspended"]
        );
        assert_eq!(vm_power_state::RUNNING, "Running");
        let listed = ENUMERATIONS
            .iter()
            .find(|e| e.name == "vm_power_state")
            .unwrap();
        assert_eq!(listed.values, vm_power_state::ALL);
        assert_eq!(enumerations::task_status_type::PENDING, "pending");
    }

    #[test]
    fn the_generator_sorts_names_keeps_types_in_order_and_refuses_clashes() {
        let description = br#"[
          {"name":"b_thing","fields":[],"messages":[],"enums":[]},
          {"name":"A","enums":[{"name":"mode","values":[{"name":"on"},{"name":"On"},{"name":"9p"}]}],
           "fields":[{"name":"type","type":"string","qualifier":"RW","lifecycle":{"state":"Published_s"}},
                     {"name":"Max_size","type":"int","qualifier":"RO/runtime","lifecycle":{"state":"Deprecated_s"}}],
           "messages":[{"name":"start","params":[{"type":"session ref"},{"type":"A ref"}],
                        "result":["void"],"lifecycle":{"state":"Published_s"}},
                       {"name":"get","params":[],"result":["string","the value"],"lifecycle":{"state":"Published_s"}}]}
        ]"#;
        let out = generator::generate(description).unwrap();
        assert!(out.find("name: \"A\"").unwrap() < out.find("name: \"b_thing\"").unwrap());
        assert!(out.contains(
            "Message { name: \"start\", parameters: &[\"session ref\", \"A ref\"], result: \"void\", state: \"Published_s\" },"
        ));
        assert!(out.contains("Message { name: \"get\", parameters: &[], result: \"string\", state: \"Published_s\" },"));
        assert!(out.contains("pub struct ARecord<'a>(&'a Value);"));
        assert!(out.contains("pub struct BThingRecord<'a>(&'a Value);"));
        assert!(out.contains("pub fn type_(self) -> FieldValue<'a> {"));
        assert!(out.contains("pub fn max_size(self) -> FieldValue<'a> {"));
        assert!(out.contains("pub const ON: &str = \"on\";"));
        assert!(out.contains("pub const ON_: &str = \"On\";"));
        assert!(out.contains("pub const _9P: &str = \"9p\";"));
        assert!(out.contains("pub const ALL: &[&str] = &[ON, ON_, _9P];"));
        assert_eq!(out, generator::generate(description).unwrap());

        let clash = br#"[{"name":"A","enums":[],"messages":[],
          "fields":[{"name":"MTU","type":"int","qualifier":"RW","lifecycle":{"state":"Published_s"}},
                    {"name":"mtu","type":"int","qualifier":"RW","lifecycle":{"state":"Published_s"}}]}]"#;
        let error = generator::generate(clash).unwrap_err();
        assert!(error.contains("MTU and mtu both become mtu"), "{error}");
    }
}
