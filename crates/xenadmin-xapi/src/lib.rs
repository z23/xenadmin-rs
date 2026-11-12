// SPDX-License-Identifier: LGPL-2.1-only WITH OCaml-LGPL-linking-exception

//! The XAPI of the qualified toolstack, as Rust: a catalogue of its classes, fields,
//! messages and enumerations, a view of a record for every class, and constants for every
//! enumeration value.
//!
//! All of it is generated when the crate is built, from the API description in
//! `data/xenapi.json`: what upstream's own generator writes for the data model of the
//! toolstack release named in `data/SOURCE`. See `NOTICE` for where it comes from and
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
/// included where the call takes one. The first `required` of them must be given; the
/// rest may be left out from the end, and the host then uses its defaults.
pub struct Message {
    pub name: &'static str,
    pub parameters: &'static [&'static str],
    pub required: usize,
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

/// A check, in debug builds, that the records given to views are of the views' classes.
///
/// A view cannot know that its record is of its class: a record does not say what it is,
/// and a field of the same name in another class reads just as well. A record does carry
/// every field of its class, though, so a record of another class has keys the view's
/// class lacks, and so has a test fixture that carries a field the API does not have.
///
/// When the environment variable `XENADMIN_XAPI_VIEW_AUDIT` names a file, every view made
/// in a debug build compares the keys of its record with the fields of its class and
/// appends a line for each key the class lacks: the class, the key and where the view was
/// made, separated by tabs, once per process. Only names are written, never a value.
/// Without the variable nothing is compared, and release builds contain none of this.
///
/// The same file receives what is wrong with the arguments of the calls a debug build
/// sends, when the caller hands them to [`audit::observe_call`]: a line that starts with
/// `call`, then the call's name, the position of the parameter in the API's order (the
/// session is the first) and the type the API gives it with the kind of value it was
/// given. A call given more arguments than the API takes, or fewer than it requires, is
/// reported with `count` for its position; parameters the API marks as optional may be
/// left out from the end. Here too only names and kinds are written, never a value.
/// Every call observed is also listed once per process, as `sent` and its name, so that
/// what the check covered can be counted afterwards.
#[cfg(debug_assertions)]
pub mod audit {
    use std::collections::BTreeSet;
    use std::io::Write as _;
    use std::sync::{Mutex, OnceLock};

    struct Sink {
        file: std::fs::File,
        seen: BTreeSet<(&'static str, String, &'static str, u32)>,
        calls: BTreeSet<String>,
    }
    static SINK: OnceLock<Option<Mutex<Sink>>> = OnceLock::new();

    fn sink() -> Option<&'static Mutex<Sink>> {
        SINK.get_or_init(|| {
            let path = std::env::var_os("XENADMIN_XAPI_VIEW_AUDIT")?;
            let file = std::fs::OpenOptions::new()
                .create(true)
                .append(true)
                .open(path)
                .ok()?;
            Some(Mutex::new(Sink {
                file,
                seen: BTreeSet::new(),
                calls: BTreeSet::new(),
            }))
        })
        .as_ref()
    }

    fn given(value: &serde_json::Value) -> &'static str {
        match value {
            serde_json::Value::Null => "null",
            serde_json::Value::Bool(_) => "a boolean",
            serde_json::Value::Number(_) => "a number",
            serde_json::Value::String(_) => "a string",
            serde_json::Value::Array(_) => "a list",
            serde_json::Value::Object(_) => "a map",
        }
    }

    /// What `value` is, when it is not what the API's type `kind` asks for: the kind of
    /// JSON value, or for a list or a map the first member that does not fit. A type this
    /// does not know fits anything.
    ///
    /// Whole numbers may be sent as numbers or as strings of digits, which is how the
    /// toolstack reads them. A reference is any string: a record's reference and the
    /// null reference look alike to it.
    pub fn misfit(kind: &str, value: &serde_json::Value) -> Option<String> {
        use serde_json::Value;
        if let Some(member) = kind.strip_suffix(" set") {
            let Value::Array(members) = value else {
                return Some(given(value).into());
            };
            return members
                .iter()
                .find_map(|v| misfit(member, v))
                .map(|what| format!("a list holding {what}"));
        }
        if let Some((key, member)) = kind
            .strip_prefix('(')
            .and_then(|k| k.strip_suffix(") map"))
            .and_then(|k| k.split_once(" -> "))
        {
            let Value::Object(members) = value else {
                return Some(given(value).into());
            };
            return members.iter().find_map(|(k, v)| {
                if let Some(what) = misfit(key, &Value::String(k.clone())) {
                    return Some(format!("a map with a key that is {what}"));
                }
                misfit(member, v).map(|what| format!("a map holding {what}"))
            });
        }
        if let Some(name) = kind.strip_prefix("enum ") {
            let Value::String(text) = value else {
                return Some(given(value).into());
            };
            let known = super::ENUMERATIONS
                .binary_search_by(|e| e.name.cmp(name))
                .is_ok_and(|at| super::ENUMERATIONS[at].values.contains(&text.as_str()));
            return (!known).then(|| "a string that is none of its values".into());
        }
        let fits = match kind {
            "string" | "datetime" => value.is_string(),
            "bool" => value.is_boolean(),
            "int" => {
                value.is_i64()
                    || value.is_u64()
                    || value.as_str().is_some_and(|v| v.parse::<i64>().is_ok())
            }
            "float" => {
                value.is_number() || value.as_str().is_some_and(|v| v.parse::<f64>().is_ok())
            }
            _ if kind.ends_with(" ref") => value.is_string(),
            _ if kind.ends_with(" record") => value.is_object(),
            _ => true,
        };
        if fits {
            None
        } else if kind == "int" && value.is_string() {
            Some("a string that is not a whole number".into())
        } else {
            Some(given(value).into())
        }
    }

    /// What is wrong with the arguments of one call, the session's token included where
    /// the call takes a session: for each, the position and what [`misfit`] says, as the
    /// lines' last two fields. Nothing for a call the catalogue does not have.
    pub fn call_misfits(method: &str, arguments: &[serde_json::Value]) -> Vec<String> {
        let name = method.strip_prefix("Async.").unwrap_or(method);
        let Some(message) = name.split_once('.').and_then(|(class, message)| {
            let class = &super::CLASSES[super::CLASSES
                .binary_search_by(|c| c.name.cmp(class))
                .ok()?];
            let at = class
                .messages
                .binary_search_by(|m| m.name.cmp(message))
                .ok()?;
            Some(&class.messages[at])
        }) else {
            return Vec::new();
        };
        if !(message.required..=message.parameters.len()).contains(&arguments.len()) {
            let takes = if message.required == message.parameters.len() {
                message.required.to_string()
            } else {
                format!("{} to {}", message.required, message.parameters.len())
            };
            return vec![format!("count\ttakes {takes}, given {}", arguments.len())];
        }
        message
            .parameters
            .iter()
            .zip(arguments)
            .enumerate()
            .filter_map(|(at, (kind, value))| {
                misfit(kind, value).map(|what| format!("{}\t{kind} given {what}", at + 1))
            })
            .collect()
    }

    /// Called by whatever sends a call, with the arguments as they go on the wire.
    pub fn observe_call(method: &str, arguments: &[serde_json::Value]) {
        let Some(sink) = sink() else {
            return;
        };
        let misfits = call_misfits(method, arguments);
        let Ok(mut sink) = sink.lock() else {
            return;
        };
        let lines = std::iter::once(format!("sent\t{method}\n")).chain(
            misfits
                .into_iter()
                .map(|misfit| format!("call\t{method}\t{misfit}\n")),
        );
        for line in lines {
            if sink.calls.insert(line.clone()) {
                let _ = sink.file.write_all(line.as_bytes());
            }
        }
    }

    /// The keys of `record` that are not fields of `class`; none for a value that is not
    /// an object or a class the catalogue does not have.
    pub fn foreign_keys<'a>(class: &str, record: &'a serde_json::Value) -> Vec<&'a str> {
        let (Some(object), Ok(index)) = (
            record.as_object(),
            super::CLASSES.binary_search_by(|c| c.name.cmp(class)),
        ) else {
            return Vec::new();
        };
        let fields = super::CLASSES[index].fields;
        object
            .keys()
            .map(String::as_str)
            .filter(|key| fields.binary_search_by(|f| f.name.cmp(key)).is_err())
            .collect()
    }

    /// Called by every view when it is made, with the place that made it.
    pub fn observe(
        class: &'static str,
        record: &serde_json::Value,
        place: &'static std::panic::Location<'static>,
    ) {
        let Some(sink) = sink() else {
            return;
        };
        let foreign = foreign_keys(class, record);
        if foreign.is_empty() {
            return;
        }
        let Ok(mut sink) = sink.lock() else {
            return;
        };
        for key in foreign {
            if sink
                .seen
                .insert((class, key.to_owned(), place.file(), place.line()))
            {
                // One short line at a time, so that processes appending to one file do not
                // interleave within a line.
                let line = format!("{class}\t{key}\t{}:{}\n", place.file(), place.line());
                let _ = sink.file.write_all(line.as_bytes());
            }
        }
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
        assert_eq!(CLASSES.len(), 70);
        assert_eq!(CLASSES.iter().map(|c| c.fields.len()).sum::<usize>(), 899);
        assert_eq!(
            CLASSES.iter().map(|c| c.messages.len()).sum::<usize>(),
            1943
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
        assert_eq!((start.required, start.result), (4, "void"));
        // A snapshot may leave out the disks to ignore, which the description marks.
        let snapshot = vm.messages.iter().find(|m| m.name == "snapshot").unwrap();
        assert_eq!((snapshot.required, snapshot.parameters.len()), (3, 4));
        assert_eq!(
            CLASSES
                .iter()
                .flat_map(|c| c.messages)
                .map(|m| m.parameters.len() - m.required)
                .sum::<usize>(),
            77
        );
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

    #[cfg(debug_assertions)]
    #[test]
    fn the_audit_names_the_keys_a_class_lacks_and_nothing_else() {
        // A VM's own fields are not reported, whatever their values.
        let vm =
            json!({"uuid": "u", "power_state": "Halted", "VBDs": [], "other_config": {"k": "v"}});
        assert!(audit::foreign_keys("VM", &vm).is_empty());
        // A repository's record read as a VM is told apart by the keys only it has, and a
        // field the API does not have at all is reported the same way.
        let sr = json!({"uuid": "u", "allowed_operations": [], "PBDs": [], "content_type": "user"});
        assert_eq!(audit::foreign_keys("VM", &sr), ["PBDs", "content_type"]);
        assert!(audit::foreign_keys("SR", &sr).is_empty());
        assert_eq!(
            audit::foreign_keys("VBD", &json!({"uuid": "u", "reserved": false})),
            ["reserved"]
        );
        // Values that are not records, and classes the catalogue lacks, report nothing.
        assert!(audit::foreign_keys("VM", &Value::Null).is_empty());
        assert!(audit::foreign_keys("VM", &json!("OpaqueRef:NULL")).is_empty());
        assert!(audit::foreign_keys("no such class", &sr).is_empty());
        // Making a view without the variable set compares nothing and changes nothing.
        assert_eq!(records::VmRecord::new(&sr).uuid(), "u");
    }

    #[cfg(debug_assertions)]
    #[test]
    fn the_audit_judges_a_call_by_the_types_the_api_gives_its_parameters() {
        use audit::{call_misfits, misfit};
        // VM.start takes the session, the VM, and two booleans.
        let good = [
            json!("OpaqueRef:s"),
            json!("OpaqueRef:vm"),
            json!(false),
            json!(true),
        ];
        assert!(call_misfits("VM.start", &good).is_empty());
        assert!(call_misfits("Async.VM.start", &good).is_empty());
        // Two arguments in each other's places are both named, by position and type.
        let swapped = [
            json!("OpaqueRef:s"),
            json!(false),
            json!("OpaqueRef:vm"),
            json!(true),
        ];
        assert_eq!(
            call_misfits("VM.start", &swapped),
            ["2\tVM ref given a boolean", "3\tbool given a string"]
        );
        // One too few is a matter of count, and nothing more is said about it.
        assert_eq!(
            call_misfits("VM.start", &good[..3]),
            ["count\ttakes 4, given 3"]
        );
        // A parameter the API marks as optional may be left out, and is judged when given.
        let snapshot = [json!("OpaqueRef:s"), json!("OpaqueRef:vm"), json!("name")];
        assert!(call_misfits("VM.snapshot", &snapshot).is_empty());
        assert_eq!(
            call_misfits("VM.snapshot", &snapshot[..2]),
            ["count\ttakes 3 to 4, given 2"]
        );
        let mut with_disks = snapshot.to_vec();
        with_disks.push(json!("OpaqueRef:vdi"));
        assert_eq!(
            call_misfits("VM.snapshot", &with_disks),
            ["4\tVDI ref set given a string"]
        );
        with_disks.push(json!(true));
        assert_eq!(
            call_misfits("VM.snapshot", &with_disks),
            ["count\ttakes 3 to 4, given 5"]
        );
        // A call the API does not have is for the conformance test, not for this one.
        assert!(call_misfits("VM.no_such_call", &good).is_empty());
        assert!(call_misfits("nothing", &[]).is_empty());
        // Whole numbers go as numbers or as digits; anything else is named.
        assert_eq!(misfit("int", &json!(4)), None);
        assert_eq!(misfit("int", &json!("2147483648")), None);
        assert_eq!(
            misfit("int", &json!("4 GiB")).as_deref(),
            Some("a string that is not a whole number")
        );
        assert_eq!(misfit("int", &json!(1.5)).as_deref(), Some("a number"));
        assert_eq!(misfit("float", &json!(1.5)), None);
        // An enumeration takes its own values and no others, in their own case.
        assert_eq!(misfit("enum vm_power_state", &json!("Halted")), None);
        assert_eq!(
            misfit("enum vm_power_state", &json!("halted")).as_deref(),
            Some("a string that is none of its values")
        );
        // Lists and maps are judged by their members, and say which kind did not fit.
        assert_eq!(misfit("string set", &json!(["a", "b"])), None);
        assert_eq!(
            misfit("string set", &json!(["a", 1])).as_deref(),
            Some("a list holding a number")
        );
        assert_eq!(
            misfit("VM ref set", &json!("OpaqueRef:vm")).as_deref(),
            Some("a string")
        );
        assert_eq!(misfit("(string -> string) map", &json!({"k": "v"})), None);
        assert_eq!(
            misfit("(string -> string) map", &json!({"k": true})).as_deref(),
            Some("a map holding a boolean")
        );
        assert_eq!(
            misfit(
                "(enum vm_power_state -> string) map",
                &json!({"Stopped": "v"})
            )
            .as_deref(),
            Some("a map with a key that is a string that is none of its values")
        );
        assert_eq!(misfit("VM record", &json!({})), None);
        assert_eq!(misfit("VM record", &json!([])).as_deref(), Some("a list"));
        assert_eq!(misfit("datetime", &Value::Null).as_deref(), Some("null"));
        // A type this does not know is not judged.
        assert_eq!(misfit("some new type", &json!(1)), None);
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
           "messages":[{"name":"start","params":[{"type":"session ref"},{"type":"A ref"},
                                                 {"type":"bool","required":false}],
                        "result":["void"],"lifecycle":{"state":"Published_s"}},
                       {"name":"get","params":[],"result":["string","the value"],"lifecycle":{"state":"Published_s"}}]}
        ]"#;
        let out = generator::generate(description).unwrap();
        assert!(out.find("name: \"A\"").unwrap() < out.find("name: \"b_thing\"").unwrap());
        assert!(out.contains(
            "Message { name: \"start\", parameters: &[\"session ref\", \"A ref\", \"bool\"], required: 2, result: \"void\", state: \"Published_s\" },"
        ));
        assert!(out.contains("Message { name: \"get\", parameters: &[], required: 0, result: \"string\", state: \"Published_s\" },"));
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

        // Arguments can only be left out from the end, so a required parameter after an
        // optional one could not be described by a count and is refused.
        let gap = br#"[{"name":"A","enums":[],"fields":[],
          "messages":[{"name":"go","params":[{"type":"bool","required":false},{"type":"A ref"}],
                       "result":["void"],"lifecycle":{"state":"Published_s"}}]}]"#;
        let error = generator::generate(gap).unwrap_err();
        assert!(
            error.contains("A.go has a required parameter after an optional one"),
            "{error}"
        );
    }
}
