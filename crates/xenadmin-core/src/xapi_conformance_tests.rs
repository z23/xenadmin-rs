//! Checks the XAPI names this crate writes as strings against the generated catalogue of
//! the API the qualified hosts run: the calls it makes and the fields its decoders name.
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
