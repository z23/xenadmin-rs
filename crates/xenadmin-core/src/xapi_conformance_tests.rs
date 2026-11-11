//! Checks the XAPI names this crate uses against the upstream API description of the
//! toolstack the qualified hosts run: the calls it makes, the fields its decoders name and
//! the fields its record views read. The description is fetched by
//! `scripts/xapi-description.sh` and is not kept in this repository; without it the check
//! says so and is skipped, unless `XENADMIN_XAPI_CONFORMANCE=required`.

use serde::Deserialize;
use sha2::{Digest, Sha256};
use std::collections::{BTreeMap, BTreeSet};

const PIN: &str = include_str!("../xapi-description.pin");

fn pinned(key: &str) -> &'static str {
    PIN.lines()
        .find_map(|line| line.strip_prefix(key)?.strip_prefix('='))
        .unwrap_or_else(|| panic!("the pin has no {key}"))
}

#[derive(Deserialize)]
struct Class {
    name: String,
    fields: Vec<Named>,
    messages: Vec<Named>,
}
#[derive(Deserialize)]
struct Named {
    name: String,
}

/// A class's names in the description.
struct Members {
    messages: BTreeSet<String>,
    fields: BTreeSet<String>,
}

/// Every class of the description by name, or `None` when the description has not been
/// fetched.
fn description() -> Option<BTreeMap<String, Members>> {
    let path = std::env::var_os("XENADMIN_XAPI_DESCRIPTION")
        .map(std::path::PathBuf::from)
        .unwrap_or_else(|| {
            std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../target/xapi-description")
                .join(format!("{}.json", pinned("sha256")))
        });
    let Ok(bytes) = std::fs::read(&path) else {
        assert!(
            std::env::var("XENADMIN_XAPI_CONFORMANCE").as_deref() != Ok("required"),
            "the API description is required and {} cannot be read",
            path.display()
        );
        eprintln!(
            "skipped: no API description at {}; run scripts/xapi-description.sh",
            path.display()
        );
        return None;
    };
    let digest: String = Sha256::digest(&bytes)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        digest,
        pinned("sha256"),
        "{} is not the pinned description",
        path.display()
    );
    let classes: Vec<Class> = serde_json::from_slice(&bytes).expect("the description parses");
    Some(
        classes
            .into_iter()
            .map(|class| {
                let names = |list: Vec<Named>| list.into_iter().map(|n| n.name).collect();
                let members = Members {
                    messages: names(class.messages),
                    fields: names(class.fields),
                };
                (class.name, members)
            })
            .collect(),
    )
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
/// made anyway.
const UNLISTED: &[(&str, &str)] = &[
    (
        "VM.get_secureboot_readiness",
        "XCP-ng's own addition to XAPI, absent from the upstream description; it answered on the lab",
    ),
    (
        "pool.get_guest_secureboot_readiness",
        "XCP-ng's own addition to XAPI, absent from the upstream description; it answered on the lab",
    ),
];

#[test]
fn the_pin_names_one_description_of_the_qualified_toolstack() {
    assert_eq!(pinned("repository"), "xapi-project/xen-api");
    assert_eq!(pinned("tag"), "v26.1.11");
    assert_eq!(pinned("commit").len(), 40);
    assert_eq!(pinned("sha256").len(), 64);
    assert!(pinned("path").ends_with(".json"));
}

#[test]
fn every_call_and_field_the_client_names_exists_in_the_api_of_the_qualified_toolstack() {
    let Some(known) = description() else {
        return;
    };
    assert_eq!(known.len(), 67, "classes in the description");
    let classes: BTreeSet<&str> = known.keys().map(String::as_str).collect();
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
        "not in the API description of {} {}:\n  {}",
        pinned("repository"),
        pinned("tag"),
        unknown.join("\n  ")
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

#[test]
fn every_field_a_record_view_reads_is_a_field_of_its_class() {
    let Some(known) = description() else {
        return;
    };
    let mut unknown = Vec::new();
    for (class, fields) in crate::xapi_records::VIEWS {
        let Some(members) = known.get(*class) else {
            unknown.push(format!("class {class}"));
            continue;
        };
        for field in *fields {
            if !members.fields.contains(*field) {
                unknown.push(format!("{class}.{field}"));
            }
        }
    }
    assert!(
        unknown.is_empty(),
        "record views read what the API description does not have: {unknown:?}"
    );
    assert!(!crate::xapi_records::VIEWS.is_empty());
}
