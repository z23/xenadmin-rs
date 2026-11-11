// SPDX-License-Identifier: LGPL-2.1-only WITH OCaml-LGPL-linking-exception

//! Generates the bindings from the description kept in this crate. The description is
//! checked against the digest recorded beside it first, so the bindings can only come
//! from the file that `data/SOURCE` names.

use sha2::{Digest, Sha256};
use std::path::Path;

#[path = "generator.rs"]
mod generator;

fn main() {
    println!("cargo:rerun-if-changed=build.rs");
    println!("cargo:rerun-if-changed=generator.rs");
    println!("cargo:rerun-if-changed=data/xenapi.json");
    println!("cargo:rerun-if-changed=data/SOURCE");
    let source = std::fs::read_to_string("data/SOURCE").expect("data/SOURCE is readable");
    let pinned = source
        .lines()
        .find_map(|line| line.strip_prefix("sha256="))
        .expect("data/SOURCE names a digest");
    let description = std::fs::read("data/xenapi.json").expect("data/xenapi.json is readable");
    let digest: String = Sha256::digest(&description)
        .iter()
        .map(|byte| format!("{byte:02x}"))
        .collect();
    assert_eq!(
        digest, pinned,
        "data/xenapi.json is not the description that data/SOURCE names"
    );
    let bindings = generator::generate(&description).expect("the description generates");
    let out = std::env::var_os("OUT_DIR").expect("cargo sets OUT_DIR");
    std::fs::write(Path::new(&out).join("bindings.rs"), bindings)
        .expect("the bindings are written");
}
