// SPDX-License-Identifier: LGPL-2.1-only WITH OCaml-LGPL-linking-exception

//! Turns the upstream API description into Rust source: the catalogue, a record view for
//! every class and constants for every enumeration. The output depends on the description
//! alone and is sorted, so the same description always gives the same source.

use serde::Deserialize;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt::Write as _;

#[derive(Deserialize)]
struct Class {
    name: String,
    fields: Vec<Field>,
    messages: Vec<Message>,
    enums: Vec<Enumeration>,
}
#[derive(Deserialize)]
struct Field {
    name: String,
    #[serde(rename = "type")]
    kind: String,
    qualifier: String,
    lifecycle: Lifecycle,
}
#[derive(Deserialize)]
struct Message {
    name: String,
    params: Vec<Parameter>,
    result: Vec<String>,
    lifecycle: Lifecycle,
}
#[derive(Deserialize)]
struct Parameter {
    #[serde(rename = "type")]
    kind: String,
}
#[derive(Deserialize)]
struct Enumeration {
    name: String,
    values: Vec<EnumerationValue>,
}
#[derive(Deserialize)]
struct EnumerationValue {
    name: String,
}
#[derive(Deserialize)]
struct Lifecycle {
    state: String,
}

const KEYWORDS: &[&str] = &[
    "abstract", "as", "async", "await", "become", "box", "break", "const", "continue", "crate",
    "do", "dyn", "else", "enum", "extern", "false", "final", "fn", "for", "gen", "if", "impl",
    "in", "let", "loop", "macro", "match", "mod", "move", "mut", "override", "priv", "pub", "ref",
    "return", "self", "static", "struct", "super", "trait", "true", "try", "type", "typeof",
    "unsafe", "unsized", "use", "virtual", "where", "while", "yield",
];

/// A Rust string literal for `text`.
fn literal(text: &str) -> String {
    format!("{text:?}")
}
fn list(items: impl Iterator<Item = String>) -> String {
    items.collect::<Vec<_>>().join(", ")
}
/// A lower-case identifier for a field or an enumeration: `VCPUs_max` is `vcpus_max`, and
/// a name that is a Rust keyword or one of the view's own methods gains an underscore.
fn snake(name: &str) -> String {
    let mut identifier: String = name
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    if identifier.is_empty() || identifier.starts_with(|c: char| c.is_ascii_digit()) {
        identifier.insert(0, '_');
    }
    if KEYWORDS.contains(&identifier.as_str()) || identifier == "new" || identifier == "record" {
        identifier.push('_');
    }
    identifier
}
/// The name of a class's view: `VM_guest_metrics` is `VmGuestMetricsRecord`.
fn view(class: &str) -> String {
    let mut name = String::new();
    for part in class.split('_').filter(|part| !part.is_empty()) {
        let mut characters = part.chars();
        if let Some(first) = characters.next() {
            name.push(first.to_ascii_uppercase());
            name.extend(characters.map(|c| c.to_ascii_lowercase()));
        }
    }
    name + "Record"
}
/// An upper-case constant for an enumeration value: `hard_shutdown` is `HARD_SHUTDOWN`.
fn constant(value: &str) -> String {
    let mut identifier: String = value
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_uppercase()
            } else {
                '_'
            }
        })
        .collect();
    if identifier.is_empty() || identifier.starts_with(|c: char| c.is_ascii_digit()) {
        identifier.insert(0, '_');
    }
    identifier
}
/// Fails when two different names would become the same identifier.
fn distinct<'a>(what: &str, names: impl Iterator<Item = (&'a str, String)>) -> Result<(), String> {
    let mut seen = BTreeMap::new();
    for (name, identifier) in names {
        if let Some(other) = seen.insert(identifier.clone(), name)
            && other != name
        {
            return Err(format!(
                "{what}: {other} and {name} both become {identifier}"
            ));
        }
    }
    Ok(())
}

/// The generated source for a description, or why it cannot be generated.
pub fn generate(description: &[u8]) -> Result<String, String> {
    let mut classes: Vec<Class> =
        serde_json::from_slice(description).map_err(|error| error.to_string())?;
    classes.sort_by(|a, b| a.name.cmp(&b.name));
    distinct(
        "classes",
        classes.iter().map(|c| (c.name.as_str(), view(&c.name))),
    )?;
    let mut enumerations: BTreeMap<&str, Vec<&str>> = BTreeMap::new();
    for class in &mut classes {
        class.fields.sort_by(|a, b| a.name.cmp(&b.name));
        class.messages.sort_by(|a, b| a.name.cmp(&b.name));
        class.enums.sort_by(|a, b| a.name.cmp(&b.name));
    }
    for class in &classes {
        distinct(
            &format!("fields of {}", class.name),
            class
                .fields
                .iter()
                .map(|f| (f.name.as_str(), snake(&f.name))),
        )?;
        for enumeration in &class.enums {
            let values: Vec<&str> = enumeration.values.iter().map(|v| v.name.as_str()).collect();
            match enumerations.get(enumeration.name.as_str()) {
                Some(known) if *known != values => {
                    return Err(format!(
                        "enumeration {} differs between classes",
                        enumeration.name
                    ));
                }
                Some(_) => {}
                None => {
                    enumerations.insert(&enumeration.name, values);
                }
            }
        }
    }
    distinct(
        "enumerations",
        enumerations.keys().map(|name| (*name, snake(name))),
    )?;

    let mut out = String::from(
        "// Generated from data/xenapi.json by generator.rs when this crate is built.\n\n",
    );
    out.push_str("/// Every class of the API, by name.\npub static CLASSES: &[Class] = &[\n");
    for class in &classes {
        let _ = writeln!(
            out,
            "    Class {{\n        name: {},\n        fields: &[",
            literal(&class.name)
        );
        for field in &class.fields {
            let _ = writeln!(
                out,
                "            Field {{ name: {}, kind: {}, qualifier: {}, state: {} }},",
                literal(&field.name),
                literal(&field.kind),
                literal(&field.qualifier),
                literal(&field.lifecycle.state)
            );
        }
        out.push_str("        ],\n        messages: &[\n");
        for message in &class.messages {
            let _ = writeln!(
                out,
                "            Message {{ name: {}, parameters: &[{}], result: {}, state: {} }},",
                literal(&message.name),
                list(message.params.iter().map(|p| literal(&p.kind))),
                literal(message.result.first().map_or("void", String::as_str)),
                literal(&message.lifecycle.state)
            );
        }
        let _ = writeln!(
            out,
            "        ],\n        enumerations: &[{}],\n    }},",
            list(class.enums.iter().map(|e| literal(&e.name)))
        );
    }
    out.push_str(
        "];\n\n/// Every enumeration of the API, by name, with its values in the API's order.\n",
    );
    out.push_str("pub static ENUMERATIONS: &[Enumeration] = &[\n");
    for (name, values) in &enumerations {
        let _ = writeln!(
            out,
            "    Enumeration {{ name: {}, values: &[{}] }},",
            literal(name),
            list(values.iter().map(|v| literal(v)))
        );
    }
    out.push_str("];\n\n");

    out.push_str(
        "/// A view of a record for every class: one method for each field, which returns the\n\
         /// field as the host sent it, null when the record does not carry it.\n\
         pub mod records {\n    use super::FieldValue;\n    use serde_json::Value;\n",
    );
    for class in &classes {
        let name = view(&class.name);
        let _ = write!(
            out,
            "\n    /// A `{class}` record.\n    #[derive(Clone, Copy)]\n    pub struct {name}<'a>(&'a Value);\n    \
             #[allow(clippy::wrong_self_convention)]\n    impl<'a> {name}<'a> {{\n        \
             /// The class these records are of.\n        pub const CLASS: &'static str = {literal};\n        \
             pub fn new(record: &'a Value) -> Self {{\n            Self(record)\n        }}\n        \
             /// The whole record, as the host sent it.\n        pub fn record(self) -> &'a Value {{\n            self.0\n        }}\n",
            class = class.name,
            literal = literal(&class.name),
        );
        for field in &class.fields {
            let _ = write!(
                out,
                "        /// `{field}`: `{kind}`, {qualifier}.\n        pub fn {method}(self) -> FieldValue<'a> {{\n            \
                 FieldValue(&self.0[{literal}])\n        }}\n",
                field = field.name,
                kind = field.kind,
                qualifier = field.qualifier,
                method = snake(&field.name),
                literal = literal(&field.name),
            );
        }
        out.push_str("    }\n");
    }
    out.push_str("}\n\n/// The values of every enumeration, as the strings the API uses.\npub mod enumerations {\n");
    for (name, values) in &enumerations {
        let _ = write!(out, "\n    /// `{name}`.\n    pub mod {} {{\n", snake(name));
        let mut used = BTreeSet::new();
        let mut constants = Vec::new();
        for value in values {
            let mut identifier = constant(value);
            // Two values that differ only in case or punctuation keep both, in order.
            while !used.insert(identifier.clone()) {
                identifier.push('_');
            }
            let _ = writeln!(
                out,
                "        pub const {identifier}: &str = {};",
                literal(value)
            );
            constants.push(identifier);
        }
        let _ = writeln!(
            out,
            "        /// Every value, in the API's order.\n        pub const ALL: &[&str] = &[{}];\n    }}",
            constants.join(", ")
        );
    }
    out.push_str("}\n");
    Ok(out)
}
