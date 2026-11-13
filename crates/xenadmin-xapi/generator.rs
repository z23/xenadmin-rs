// SPDX-License-Identifier: LGPL-2.1-only WITH OCaml-LGPL-linking-exception

//! Turns the upstream API description into Rust source: the catalogue, a record view for
//! every class, constants for every enumeration and a type for every message that says
//! what the call takes. The output depends on the description alone and is sorted, so the
//! same description always gives the same source.

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
    /// Absent in a description that does not name its parameters.
    #[serde(default)]
    name: String,
    #[serde(rename = "type")]
    kind: String,
    /// Absent in a description that does not say: every parameter is then required.
    #[serde(default = "yes")]
    required: bool,
}
fn yes() -> bool {
    true
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
/// The name of a class's marker and of a message's type: `VM_guest_metrics` is
/// `VmGuestMetrics` and `set_VCPUs_max` is `SetVcpusMax`.
fn camel(name: &str) -> String {
    let mut identifier = String::new();
    for part in name
        .split(|c: char| !c.is_ascii_alphanumeric())
        .filter(|part| !part.is_empty())
    {
        let mut characters = part.chars();
        if let Some(first) = characters.next() {
            identifier.push(first.to_ascii_uppercase());
            identifier.extend(characters.map(|c| c.to_ascii_lowercase()));
        }
    }
    if identifier.is_empty() || identifier.starts_with(|c: char| c.is_ascii_digit()) {
        identifier.insert(0, '_');
    }
    identifier
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

/// The type a view returns for a field of the API type `kind`: one for each kind of
/// value, and the untyped `FieldValue` for a kind there is no type for.
fn typed(kind: &str) -> &'static str {
    match kind {
        "bool" => "FlagField",
        "string" | "string option" => "TextField",
        "datetime" => "TimeField",
        "int" => "WholeField",
        "float" => "RealField",
        _ if kind.ends_with(" set") => "SetField",
        _ if kind.starts_with('(') && kind.ends_with(") map") => "MapField",
        _ if kind.starts_with("enum ") => "ChoiceField",
        _ if kind.ends_with(" ref") => "RefField",
        _ => "FieldValue",
    }
}

/// The type of a message's field for a parameter of the API type `kind`, whether that
/// type borrows, and the expression that turns `value` into the JSON the call sends.
/// A flag, a number, text and a reference are typed; a set, a map and a record are JSON
/// values handed over whole. An enumeration's value is text: its constants are strings.
fn argument(kind: &str, value: &str) -> (String, bool, String) {
    match kind {
        "bool" => ("bool".into(), false, format!("Value::Bool({value})")),
        "int" => ("Int".into(), false, format!("{value}.json()")),
        "float" => ("f64".into(), false, format!("Value::from({value})")),
        "string" | "datetime" => (
            "&'a str".into(),
            true,
            format!("Value::String({value}.to_owned())"),
        ),
        _ if kind.starts_with("enum ") => (
            "&'a str".into(),
            true,
            format!("Value::String({value}.to_owned())"),
        ),
        _ if kind.ends_with(" ref") && kind.split(' ').count() == 2 => (
            format!("Ref<'a, class::{}>", camel(kind.trim_end_matches(" ref"))),
            true,
            format!("{value}.json()"),
        ),
        _ => ("Value".into(), false, value.to_owned()),
    }
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
            // A caller may leave parameters out only from the end, so the optional ones
            // have to be the last: one count then says which they are.
            let required = message.params.iter().take_while(|p| p.required).count();
            if message.params[required..].iter().any(|p| p.required) {
                return Err(format!(
                    "{}.{} has a required parameter after an optional one",
                    class.name, message.name
                ));
            }
            let _ = writeln!(
                out,
                "            Message {{ name: {}, parameters: &[{}], required: {required}, result: {}, state: {} }},",
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
         pub mod records {\n    #[allow(unused_imports)]\n    use super::{ChoiceField, FieldValue, FlagField, MapField, RealField, RefField, SetField, TextField, TimeField, WholeField};\n    use serde_json::Value;\n",
    );
    for class in &classes {
        let name = view(&class.name);
        let _ = write!(
            out,
            "\n    /// A `{class}` record.\n    #[derive(Clone, Copy)]\n    pub struct {name}<'a>(&'a Value);\n    \
             #[allow(clippy::wrong_self_convention)]\n    impl<'a> {name}<'a> {{\n        \
             /// The class these records are of.\n        pub const CLASS: &'static str = {literal};\n        \
             #[cfg_attr(debug_assertions, track_caller)]\n        \
             pub fn new(record: &'a Value) -> Self {{\n            \
             #[cfg(debug_assertions)]\n            \
             crate::audit::observe(Self::CLASS, record, std::panic::Location::caller());\n            \
             Self(record)\n        }}\n        \
             /// The whole record, as the host sent it.\n        pub fn record(self) -> &'a Value {{\n            self.0\n        }}\n",
            class = class.name,
            literal = literal(&class.name),
        );
        for field in &class.fields {
            let _ = write!(
                out,
                "        /// `{field}`: `{kind}`, {qualifier}.\n        pub fn {method}(self) -> {typed}<'a> {{\n            \
                 {typed}(&self.0[{literal}])\n        }}\n",
                field = field.name,
                typed = typed(&field.kind),
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
    out.push_str("}\n\n");

    distinct(
        "class markers",
        classes.iter().map(|c| (c.name.as_str(), camel(&c.name))),
    )?;
    distinct(
        "class modules",
        classes.iter().map(|c| (c.name.as_str(), snake(&c.name))),
    )?;
    let known: BTreeSet<&str> = classes.iter().map(|c| c.name.as_str()).collect();
    out.push_str(
        "/// A marker for every class, which says of which class a reference is.\npub mod class {\n",
    );
    for class in &classes {
        let _ = writeln!(
            out,
            "    /// `{}`.\n    pub struct {};",
            class.name,
            camel(&class.name)
        );
    }
    out.push_str(
        "}\n\n/// For every class, a reference that the caller says is of that class, made from\n\
         /// text or from a value a host sent. The class is checked against the parameter the\n\
         /// reference is given to; that it is a reference of that class is the caller's\n\
         /// word, written where it is used.\n\
         pub mod refs {\n    use super::{Ref, RefSource, class};\n",
    );
    for class in &classes {
        let _ = writeln!(
            out,
            "    /// A reference to a `{name}`.\n    pub fn {function}<'a>(reference: impl Into<RefSource<'a>>) -> Ref<'a, class::{marker}> {{\n        Ref::new(reference)\n    }}",
            name = class.name,
            function = snake(&class.name),
            marker = camel(&class.name)
        );
    }
    out.push_str(
        "}\n\n/// A type for every message that takes the session first: its fields are the call's\n\
         /// other parameters, by the API's names and in the types of their kinds. A message\n\
         /// with one parameter is a tuple, one with none a unit. Where the last parameters\n\
         /// may be left out, the type named for the message takes the required ones and a\n\
         /// type `…With…`, named for the last parameter it adds, takes more.\n\
         pub mod calls {\n",
    );
    for class in &classes {
        let mut body = String::new();
        let mut names = Vec::new();
        for message in &class.messages {
            let Some((session, parameters)) = message.params.split_first() else {
                continue;
            };
            if session.kind != "session ref" {
                continue;
            }
            let required = parameters.iter().take_while(|p| p.required).count();
            let fields: Vec<String> = parameters
                .iter()
                .enumerate()
                .map(|(at, p)| {
                    if p.name.is_empty() {
                        format!("argument_{at}")
                    } else {
                        snake(&p.name)
                    }
                })
                .collect();
            distinct(
                &format!("parameters of {}.{}", class.name, message.name),
                parameters
                    .iter()
                    .zip(&fields)
                    .map(|(p, field)| (p.name.as_str(), field.clone())),
            )?;
            for count in required..=parameters.len() {
                let mut name = camel(&message.name);
                if count > required {
                    name.push_str("With");
                    name.push_str(&camel(&fields[count - 1]));
                }
                names.push((format!("{}:{count}", message.name), name.clone()));
                let typed: Vec<(String, bool, String)> = parameters[..count]
                    .iter()
                    .zip(&fields)
                    .enumerate()
                    .map(|(at, (p, field))| {
                        let kind = match p.kind.strip_suffix(" ref") {
                            // A reference to something the description has no class for.
                            Some(class) if !known.contains(class) => "",
                            _ => p.kind.as_str(),
                        };
                        let value = if count == 1 {
                            format!("self.{at}")
                        } else {
                            format!("self.{field}")
                        };
                        argument(kind, &value)
                    })
                    .collect();
                let lifetime = if typed.iter().any(|(_, borrows, _)| *borrows) {
                    "<'a>"
                } else {
                    ""
                };
                let _ = writeln!(
                    body,
                    "\n        /// `{class}.{message}`, which gives `{result}`.",
                    class = class.name,
                    message = message.name,
                    result = message.result.first().map_or("void", String::as_str),
                );
                match count {
                    0 => {
                        let _ = writeln!(body, "        pub struct {name};");
                    }
                    1 => {
                        let _ = writeln!(
                            body,
                            "        pub struct {name}{lifetime}(\n            /// `{field}`: `{kind}`.\n            pub {typed},\n        );",
                            field = parameters[0].name,
                            kind = parameters[0].kind,
                            typed = typed[0].0,
                        );
                    }
                    _ => {
                        let _ = writeln!(body, "        pub struct {name}{lifetime} {{");
                        for ((p, field), (typed, _, _)) in
                            parameters[..count].iter().zip(&fields).zip(&typed)
                        {
                            let _ = writeln!(
                                body,
                                "            /// `{kind}`.\n            pub {field}: {typed},",
                                kind = p.kind,
                            );
                        }
                        body.push_str("        }\n");
                    }
                }
                let _ = writeln!(
                    body,
                    "        impl Call for {name}{anonymous} {{\n            \
                     const NAME: &'static str = {plain};\n            \
                     const ASYNC: &'static str = {asynchronous};\n            \
                     const RESULT: &'static str = {result};\n            \
                     fn arguments(self) -> Vec<Value> {{\n                vec![{arguments}]\n            }}\n        }}",
                    anonymous = if lifetime.is_empty() { "" } else { "<'_>" },
                    plain = literal(&format!("{}.{}", class.name, message.name)),
                    asynchronous = literal(&format!("Async.{}.{}", class.name, message.name)),
                    result = literal(message.result.first().map_or("void", String::as_str)),
                    arguments = list(typed.iter().map(|(_, _, json)| json.clone())),
                );
            }
        }
        if body.is_empty() {
            continue;
        }
        distinct(
            &format!("messages of {}", class.name),
            names
                .iter()
                .map(|(message, name)| (message.as_str(), name.clone())),
        )?;
        let _ = write!(
            out,
            "\n    /// The messages of `{class}`.\n    pub mod {module} {{\n        \
             #[allow(unused_imports)]\n        use crate::{{Call, Int, Ref, class}};\n        \
             #[allow(unused_imports)]\n        use serde_json::Value;\n{body}    }}\n",
            class = class.name,
            module = snake(&class.name),
        );
    }
    out.push_str("}\n");
    Ok(out)
}
