//! Checks the XAPI names this crate writes as strings against the generated catalogue of
//! the API the qualified hosts run: the calls it names and the fields its decoders name.
//! A call with its arguments is written as its message from the bindings, which the
//! compiler checks; a test here holds that only the session layer still writes a call as
//! a name and a list.
//! Reads through the generated record views need no check of their names, because a view
//! has no method for a field its class lacks, and a field's type refuses a comparison with
//! a value of another kind and has only the readers of its own. What a view cannot refuse
//! is a field asked for its JSON value by name, so a test here lists where that is done.
//! The values of the API's enumerations are written as the generated constants; the names
//! that are a message's name first stay literals, and the last test holds them to the API.

use std::collections::{BTreeMap, BTreeSet};
use xenadmin_xapi::CLASSES;

/// A class's names in the catalogue.
struct Members {
    messages: BTreeSet<&'static str>,
    fields: BTreeSet<&'static str>,
}

fn catalogue() -> BTreeMap<&'static str, Members> {
    CLASSES
        .iter()
        .map(|class| {
            let members = Members {
                messages: class.messages.iter().map(|m| m.name).collect(),
                fields: class.fields.iter().map(|f| f.name).collect(),
            };
            (class.name, members)
        })
        .collect()
}

/// Every string literal in the crate's sources that has the shape of a name on a class,
/// `Class.name` or `Async.Class.name`, with the file it is in. The crate writes calls that
/// way, and fields where a decoder says which one it could not read.
fn named(classes: &BTreeSet<&str>) -> BTreeSet<(String, String)> {
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let mut found = BTreeSet::new();
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        let name = path.file_name().unwrap().to_string_lossy().into_owned();
        if !name.ends_with(".rs") || name == "xapi_conformance_tests.rs" {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        // Odd pieces of a split on quotes are the insides of string literals. An escaped
        // quote only shortens a literal, which cannot turn it into a name.
        for literal in text.split('"').skip(1).step_by(2) {
            let call = literal.strip_prefix("Async.").unwrap_or(literal);
            let Some((class, member)) = call.split_once('.') else {
                continue;
            };
            let identifier = !member.is_empty()
                && member
                    .bytes()
                    .all(|b| b.is_ascii_alphanumeric() || b == b'_');
            if classes.contains(class) && identifier {
                found.insert((literal.to_owned(), name.clone()));
            }
        }
    }
    found
}

/// Literals of that shape that name nothing in the API: prefixes that tests match setters
/// by, and a host name used in examples.
const NOT_NAMES: &[&str] = &["VM.set_", "pool.set_", "host.example"];

/// Calls the client makes that the description does not list, each with the reason it is
/// made anyway. There are none: the two that an older description lacked are in the one
/// generated for the qualified toolstack.
const UNLISTED: &[(&str, &str)] = &[];

#[test]
fn every_call_and_field_the_client_names_exists_in_the_api_of_the_qualified_toolstack() {
    let known = catalogue();
    let classes: BTreeSet<&str> = known.keys().copied().collect();
    let unlisted: BTreeSet<&str> = UNLISTED.iter().map(|(call, _)| *call).collect();
    let (mut calls, mut fields, mut unknown) = (0, 0, Vec::new());
    let mut seen_exceptions = BTreeSet::new();
    let named = named(&classes);
    for (literal, file) in &named {
        let asynchronous = literal.starts_with("Async.");
        let name = literal.strip_prefix("Async.").unwrap_or(literal);
        let (class, member) = name.split_once('.').unwrap();
        let members = &known[class];
        if members.messages.contains(member) {
            calls += 1;
        } else if members.fields.contains(member) && !asynchronous {
            fields += 1;
        } else if NOT_NAMES.contains(&literal.as_str()) || unlisted.contains(literal.as_str()) {
            seen_exceptions.insert(literal.as_str());
        } else {
            unknown.push(format!("{literal} ({file})"));
        }
    }
    assert!(
        unknown.is_empty(),
        "not in the API the bindings were generated from:\n  {}\n{}",
        unknown.join("\n  "),
        xenadmin_xapi::SOURCE
    );
    // The scan reaches the calls and the decoders: a change that hid them would pass empty.
    assert!(calls > 300 && fields > 20, "{calls} calls, {fields} fields");
    // An exception that no longer applies is removed, so the lists stay honest.
    for exception in NOT_NAMES.iter().chain(unlisted.iter()) {
        assert!(
            seen_exceptions.contains(exception),
            "{exception} is listed as an exception and is no longer named"
        );
    }
}

/// Where the arguments start when the text after a call's name goes on `, vec![` or
/// `, parameters: vec![`: the two ways the crate writes a call with its arguments.
fn arguments_start(text: &str, after_name: usize) -> Option<usize> {
    let rest = text[after_name..]
        .trim_start()
        .strip_prefix(',')?
        .trim_start();
    let rest = rest
        .strip_prefix("parameters:")
        .map_or(rest, str::trim_start);
    let rest = rest.strip_prefix("vec![")?;
    Some(text.len() - rest.len())
}

/// How many expressions a `vec![` that starts at `start` holds, to its closing bracket.
fn argument_count(text: &str, start: usize) -> Option<usize> {
    let bytes = text.as_bytes();
    let (mut depth, mut count, mut content) = (0_usize, 0_usize, false);
    let mut i = start;
    while i < bytes.len() {
        match bytes[i] {
            b'"' => {
                i += 1;
                while i < bytes.len() && bytes[i] != b'"' {
                    i += if bytes[i] == b'\\' { 2 } else { 1 };
                }
                content = true;
            }
            b'(' | b'[' | b'{' => {
                depth += 1;
                content = true;
            }
            b']' if depth == 0 => return Some(count + usize::from(content)),
            b')' | b']' | b'}' => depth = depth.checked_sub(1)?,
            b',' if depth == 0 => {
                count += usize::from(content);
                content = false;
            }
            byte if !byte.is_ascii_whitespace() => content = true,
            _ => {}
        }
        i += 1;
    }
    None
}

/// Calls made below the session, which write the session's token as their first argument
/// themselves. Everything else hands its arguments to the session, which adds the token.
const SESSION_WRITTEN: &[&str] = &["session.get_record", "event.from", "session.logout"];

#[test]
fn every_call_written_with_its_arguments_passes_as_many_as_the_api_takes() {
    // The fewest and the most arguments each call takes after the session: parameters the
    // API marks as optional may be left out from the end.
    let takes: BTreeMap<String, (usize, usize)> = CLASSES
        .iter()
        .flat_map(|class| {
            class.messages.iter().map(move |message| {
                let session = usize::from(message.parameters.first() == Some(&"session ref"));
                (
                    format!("{}.{}", class.name, message.name),
                    (
                        message.required - session,
                        message.parameters.len() - session,
                    ),
                )
            })
        })
        .collect();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (mut written, mut wrong) = (Vec::new(), Vec::new());
    let mut seen_session_written = BTreeSet::new();
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        // Tests name calls to answer them, not to make them.
        if !file.ends_with(".rs") || file.ends_with("_tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut from = 0;
        while let Some(open) = text[from..].find('"').map(|at| from + at) {
            let Some(close) = text[open + 1..].find('"').map(|at| open + 1 + at) else {
                break;
            };
            from = close + 1;
            let literal = &text[open + 1..close];
            let name = literal.strip_prefix("Async.").unwrap_or(literal);
            let (Some(&(least, most)), Some(start)) =
                (takes.get(name), arguments_start(&text, from))
            else {
                continue;
            };
            let passes = argument_count(&text, start)
                .unwrap_or_else(|| panic!("unbalanced arguments after {literal} in {file}"));
            written.push(format!("{name} ({file})"));
            let below_session = SESSION_WRITTEN
                .iter()
                .find(|call| file == "xapi.rs" && **call == name);
            if let Some(call) = below_session.filter(|_| passes == most + 1) {
                seen_session_written.insert(*call);
            } else if !(least..=most).contains(&passes) {
                wrong.push(format!(
                    "{literal} ({file}) passes {passes}, the API takes {least} to {most}"
                ));
            }
        }
    }
    assert!(
        wrong.is_empty(),
        "{}\n{}",
        wrong.join("\n"),
        xenadmin_xapi::SOURCE
    );
    // A call is written as its message from the bindings and sent with `Session::send`,
    // so the compiler checks what it is given. What is still written as a name and a
    // list of JSON values is what the session layer sends itself, and nothing else may
    // be: a new call written that way fails here.
    written.sort();
    assert_eq!(
        written,
        [
            "event.from (xapi.rs)",
            "session.get_record (xapi.rs)",
            "session.login_with_password (xapi.rs)",
            "session.logout (xapi.rs)",
        ]
    );
    for call in SESSION_WRITTEN {
        assert!(
            seen_session_written.contains(call),
            "{call} is listed as written below the session and no longer is"
        );
    }
}

#[test]
fn the_argument_count_reads_nested_expressions_and_strings_as_one_argument() {
    let count = |text: &str| argument_count(text, 0);
    assert_eq!(count("]"), Some(0));
    assert_eq!(count("json!(a)]"), Some(1));
    assert_eq!(
        count("json!(a), json!([b, c]), json!({\"k\": \"x, y]\"}),\n]"),
        Some(3)
    );
    assert_eq!(count("f(a, b), g(&[c, d]),]"), Some(2));
    assert_eq!(count("json!(a), json!(b)"), None, "no closing bracket");
    let text = "call(\"VM.start\", vec![json!(vm), json!(false), json!(false)])";
    let start = arguments_start(text, text.find(',').unwrap()).unwrap();
    assert_eq!(argument_count(text, start), Some(3));
    assert_eq!(arguments_start("\"VM.start\" => true", 10), None);
}

/// The name the generator gives a class's view, and a field's method on it.
fn view_name(class: &str) -> String {
    let mut name: String = class
        .split('_')
        .filter(|part| !part.is_empty())
        .map(|part| part[..1].to_uppercase() + &part[1..].to_lowercase())
        .collect();
    name.push_str("Record");
    name
}
fn method_name(field: &str) -> String {
    let name: String = field
        .chars()
        .map(|c| {
            if c.is_ascii_alphanumeric() {
                c.to_ascii_lowercase()
            } else {
                '_'
            }
        })
        .collect();
    // The generator sets apart the names Rust or the view itself already uses.
    if matches!(name.as_str(), "type" | "ref" | "new" | "record") {
        name + "_"
    } else {
        name
    }
}

/// Whether a field of the API type `kind` has a type of its own in the bindings, with
/// that kind's readers and no others.
fn typed(kind: &str) -> bool {
    matches!(
        kind,
        "bool" | "string" | "string option" | "datetime" | "int" | "float"
    ) || kind.ends_with(" set")
        || (kind.starts_with('(') && kind.ends_with(") map"))
        || kind.starts_with("enum ")
        || kind.ends_with(" ref")
}

/// The identifier that ends at `end` in `text`.
fn identifier_before(text: &str, end: usize) -> &str {
    let start = text[..end]
        .rfind(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .map_or(0, |at| at + 1);
    &text[start..end]
}

/// After `from`, skipping white space: `.name(` and where its arguments start.
fn method_after(text: &str, from: usize) -> Option<(&str, usize)> {
    let rest = text[from..].trim_start();
    let rest = rest.strip_prefix('.')?.trim_start();
    let length = rest
        .find(|c: char| !(c.is_ascii_alphanumeric() || c == '_'))
        .unwrap_or(rest.len());
    let after = &rest[length..];
    after
        .starts_with('(')
        .then(|| (&rest[..length], text.len() - after.len() + 1))
}

/// A typed field has the readers of its kind and no JSON accessor, so a read of the wrong
/// kind does not compile; the bindings' own examples hold that. What the compiler no
/// longer sees is a field asked for its JSON value by name. This lists every place that
/// does so on a view, so that another one is a decision and not a habit.
#[test]
fn the_json_value_of_a_typed_field_is_asked_for_only_where_it_is_listed() {
    // Text that holds a number: the position of a disk or an interface on its guest is a
    // string of digits, handed to the reader of whole numbers.
    const LISTED: &[&str] = &[
        "ovf_export.rs: VbdRecord.userdevice",
        "ovf_export.rs: VbdRecord.userdevice",
        "ovf_export.rs: VifRecord.device",
        "ovf_import.rs: VbdRecord.userdevice",
        "ovf_import.rs: VifRecord.device",
    ];
    let fields: BTreeMap<(String, String), &'static str> = CLASSES
        .iter()
        .flat_map(|class| {
            class
                .fields
                .iter()
                .map(move |field| ((view_name(class.name), method_name(field.name)), field.kind))
        })
        .collect();
    let directory = std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let (mut reads, mut found) = (0, Vec::new());
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        if !file.ends_with(".rs") || file.ends_with("tests.rs") {
            continue;
        }
        let text = std::fs::read_to_string(&path).unwrap();
        let mut from = 0;
        while let Some(at) = text[from..].find("Record::new(").map(|at| from + at) {
            from = at + "Record::new(".len();
            let view = identifier_before(&text, at + "Record".len());
            // The view's argument is one expression: skip to its closing parenthesis.
            let mut depth = 0usize;
            let Some(length) = text[from - 1..].char_indices().find_map(|(i, c)| {
                match c {
                    '(' => depth += 1,
                    ')' => depth -= 1,
                    _ => {}
                }
                (depth == 0).then_some(i)
            }) else {
                continue;
            };
            let Some((field, arguments)) = method_after(&text, from + length) else {
                continue;
            };
            let Some(kind) = fields.get(&(view.to_owned(), field.to_owned())) else {
                continue;
            };
            if !typed(kind) {
                continue;
            }
            reads += 1;
            if let Some(("value", _)) = method_after(&text, arguments + 1) {
                found.push(format!("{file}: {view}.{field}"));
            }
        }
    }
    found.sort();
    assert_eq!(found, LISTED);
    // The scan reaches the reads: a change in how they are written would pass empty.
    println!("{reads} reads of typed fields on views");
    assert!(reads > 1000, "{reads} reads of typed fields on views");
}

/// A power action is kept under one name, which is the call that performs it
/// (`VM.clean_shutdown`) and also the operation a record offers or refuses. It is written
/// as a literal because it is a message's name first; this holds each to being both.
#[test]
fn the_names_kept_for_both_a_call_and_an_operation_are_both_in_the_api() {
    use xenadmin_xapi::enumerations::{vm_appliance_operation, vm_operations};
    let catalogue = catalogue();
    let both = |class: &str, operations: &[&str], name: &str| {
        assert!(catalogue[class].messages.contains(name), "{class}.{name}");
        assert!(operations.contains(&name), "{class} operation {name}");
    };
    for action in crate::PowerAction::ALL {
        both("VM", vm_operations::ALL, action.wire());
    }
    for action in [
        crate::ForceRecoveryAction::Shutdown,
        crate::ForceRecoveryAction::Reboot,
    ] {
        both("VM", vm_operations::ALL, action.wire());
    }
    for action in [
        crate::AppliancePower::Start,
        crate::AppliancePower::CleanShutdown,
        crate::AppliancePower::HardShutdown,
    ] {
        both("VM_appliance", vm_appliance_operation::ALL, action.wire());
    }
}

#[test]
fn the_names_of_views_and_their_methods_are_the_generators() {
    assert_eq!(view_name("VM"), "VmRecord");
    assert_eq!(view_name("VM_appliance"), "VmApplianceRecord");
    assert_eq!(view_name("host_metrics"), "HostMetricsRecord");
    assert_eq!(method_name("VCPUs_max"), "vcpus_max");
    assert_eq!(method_name("type"), "type_");
    assert_eq!(method_name("MAC"), "mac");
    assert_eq!(method_after("x) .as_str()", 2), Some(("as_str", 11)));
    assert_eq!(
        method_after("x)\n    .value()\n.as_bool()", 2),
        Some(("value", 14))
    );
    assert_eq!(method_after("x) == 1", 2), None);
    assert_eq!(identifier_before("a.VmRecord::new(", 10), "VmRecord");
    assert!(typed("VM ref set") && typed("bool") && typed("(string -> string) map"));
    assert!(typed("enum vm_power_state") && typed("host ref") && !typed("VM record"));
}
