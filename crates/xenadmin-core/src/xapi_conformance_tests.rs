//! Checks the XAPI names this crate writes as strings against the generated catalogue of
//! the API the qualified hosts run: the calls it makes and the fields its decoders name,
//! and for a call written with its arguments beside it, how many it passes.
//! Reads through the generated record views need no check, because a view has no method
//! for a field its class lacks.

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
    let takes: BTreeMap<String, usize> = CLASSES
        .iter()
        .flat_map(|class| {
            class.messages.iter().map(move |message| {
                let session = usize::from(message.parameters.first() == Some(&"session ref"));
                (
                    format!("{}.{}", class.name, message.name),
                    message.parameters.len() - session,
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
            let (Some(&takes), Some(start)) = (takes.get(name), arguments_start(&text, from))
            else {
                continue;
            };
            let passes = argument_count(&text, start)
                .unwrap_or_else(|| panic!("unbalanced arguments after {literal} in {file}"));
            checked += 1;
            let below_session = SESSION_WRITTEN
                .iter()
                .find(|call| file == "xapi.rs" && **call == name);
            if let Some(call) = below_session.filter(|_| passes == takes + 1) {
                seen_session_written.insert(*call);
            } else if passes != takes {
                wrong.push(format!(
                    "{literal} ({file}) passes {passes}, the API takes {takes}"
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
