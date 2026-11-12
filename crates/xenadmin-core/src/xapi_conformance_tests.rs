//! Checks the XAPI names this crate writes as strings against the generated catalogue of
//! the API the qualified hosts run: the calls it makes and the fields its decoders name,
//! and for a call written with its arguments beside it, how many it passes.
//! Reads through the generated record views need no check of their names, because a view
//! has no method for a field its class lacks, and a field's type refuses a comparison with
//! a value of another kind. A field still reads as the JSON value it wraps, so the last
//! test here checks that each is asked for the kind of value the API gives it.

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
    let (mut checked, mut wrong) = (0, Vec::new());
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
            checked += 1;
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
    // The scan reaches the calls: a change in how they are written would pass empty.
    assert!(checked > 300, "{checked} calls with their arguments");
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

/// The JSON accessors that fit a field of the API type `kind`; none for a kind that is
/// not judged.
fn accessors(kind: &str) -> &'static [&'static str] {
    match kind {
        "bool" => &["as_bool", "is_boolean"],
        "string" | "string option" | "datetime" => &["as_str", "is_string"],
        // Hosts send whole numbers as numbers or as strings of digits.
        "int" => &[
            "as_i64",
            "as_u64",
            "as_str",
            "is_i64",
            "is_u64",
            "is_number",
            "is_string",
        ],
        "float" => &["as_f64", "as_str", "is_f64", "is_number", "is_string"],
        _ if kind.ends_with(" set") => &["as_array", "is_array"],
        _ if kind.starts_with('(') && kind.ends_with(") map") => &["as_object", "is_object"],
        _ if kind.starts_with("enum ") || kind.ends_with(" ref") => &["as_str", "is_string"],
        _ => &[],
    }
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

#[test]
fn every_field_read_through_a_view_is_asked_for_the_kind_of_value_the_api_gives_it() {
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
    let (mut judged, mut wrong) = (0, Vec::new());
    for entry in std::fs::read_dir(&directory).unwrap() {
        let path = entry.unwrap().path();
        let file = path.file_name().unwrap().to_string_lossy().into_owned();
        if !file.ends_with(".rs") || file == "xapi_conformance_tests.rs" {
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
            // `.value()` only lends the value for longer; what follows it is the read.
            let mut next = method_after(&text, arguments + 1);
            if let Some(("value", after)) = next {
                next = method_after(&text, after + 1);
            }
            let Some((accessor, _)) = next else {
                continue;
            };
            let fits = accessors(kind);
            if fits.is_empty() || !(accessor.starts_with("as_") || accessor.starts_with("is_")) {
                continue;
            }
            judged += 1;
            if accessor != "is_null" && !fits.contains(&accessor) {
                let line = text[..at].matches('\n').count() + 1;
                wrong.push(format!(
                    "{file}:{line}: {view}.{field} is `{kind}` and is read with {accessor}()"
                ));
            }
        }
    }
    assert!(wrong.is_empty(), "{}", wrong.join("\n"));
    // The scan reaches the reads: a change in how they are written would pass empty.
    println!("{judged} reads judged");
    assert!(judged > 300, "{judged} reads judged");
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
    assert_eq!(accessors("VM ref set"), ["as_array", "is_array"]);
    assert_eq!(accessors("bool"), ["as_bool", "is_boolean"]);
    assert!(accessors("VM record").is_empty());
}
