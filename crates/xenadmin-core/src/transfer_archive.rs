use super::*;
use serde_json::{Map, Value};
use sha2::{Digest, Sha256};
use std::{collections::BTreeSet, io::Read};
const METADATA_LIMIT: usize = 8 * 1024 * 1024;
fn octal(bytes: &[u8]) -> Result<usize, Error> {
    let s = std::str::from_utf8(bytes)
        .map_err(|_| Error::TransferArchive)?
        .trim_matches(['\0', ' ']);
    usize::from_str_radix(s, 8).map_err(|_| Error::TransferArchive)
}
fn header(reader: &mut dyn Read) -> Result<Vec<u8>, Error> {
    let mut h = [0u8; 512];
    reader
        .read_exact(&mut h)
        .map_err(|_| Error::TransferArchive)?;
    if &h[..8] != b"ova.xml\0"
        || h[8..100].iter().any(|n| *n != 0)
        || h[345..500].iter().any(|n| *n != 0)
        || ![0, b'0'].contains(&h[156])
    {
        return Err(Error::TransferArchive);
    }
    let checksum = h
        .iter()
        .enumerate()
        .map(|(i, b)| {
            if (148..156).contains(&i) {
                32
            } else {
                usize::from(*b)
            }
        })
        .sum::<usize>();
    if octal(&h[148..156])? != checksum {
        return Err(Error::TransferArchive);
    }
    let size = octal(&h[124..136])?;
    if size == 0 || size > METADATA_LIMIT {
        return Err(Error::TransferArchive);
    }
    let mut xml = vec![0; size];
    reader
        .read_exact(&mut xml)
        .map_err(|_| Error::TransferArchive)?;
    Ok(xml)
}
fn element<'a, 'i>(node: roxmltree::Node<'a, 'i>) -> Result<roxmltree::Node<'a, 'i>, Error> {
    let mut e = node.children().filter(|n| n.is_element());
    let n = e.next().ok_or(Error::TransferArchive)?;
    if e.next().is_some() {
        return Err(Error::TransferArchive);
    }
    Ok(n)
}
fn value(node: roxmltree::Node<'_, '_>, depth: usize) -> Result<Value, Error> {
    if depth > 24 || !node.has_tag_name("value") {
        return Err(Error::TransferArchive);
    }
    if !node.children().any(|n| n.is_element()) {
        return Ok(Value::String(node.text().unwrap_or_default().into()));
    }
    let n = element(node)?;
    let text = n.text().unwrap_or_default();
    match n.tag_name().name() {
        "string" | "dateTime.iso8601" => Ok(Value::String(text.into())),
        "boolean" => match text {
            "0" => Ok(Value::Bool(false)),
            "1" => Ok(Value::Bool(true)),
            _ => Err(Error::TransferArchive),
        },
        "int" | "i4" | "i8" => text
            .parse::<i64>()
            .map(Value::from)
            .map_err(|_| Error::TransferArchive),
        "double" => text
            .parse::<f64>()
            .ok()
            .and_then(serde_json::Number::from_f64)
            .map(Value::Number)
            .ok_or(Error::TransferArchive),
        "array" => {
            let data = element(n)?;
            if !data.has_tag_name("data") {
                return Err(Error::TransferArchive);
            }
            let children = data
                .children()
                .filter(|n| n.is_element())
                .collect::<Vec<_>>();
            if children.len() > 4096 {
                return Err(Error::TransferArchive);
            }
            Ok(Value::Array(
                children
                    .into_iter()
                    .map(|v| value(v, depth + 1))
                    .collect::<Result<_, _>>()?,
            ))
        }
        "struct" => {
            let mut map = Map::new();
            for m in n.children().filter(|n| n.is_element()) {
                if !m.has_tag_name("member") {
                    return Err(Error::TransferArchive);
                }
                let e = m.children().filter(|n| n.is_element()).collect::<Vec<_>>();
                if e.len() != 2 || !e[0].has_tag_name("name") {
                    return Err(Error::TransferArchive);
                }
                let key = e[0].text().ok_or(Error::TransferArchive)?;
                if key.len() > 256 || map.insert(key.into(), value(e[1], depth + 1)?).is_some() {
                    return Err(Error::TransferArchive);
                }
            }
            Ok(Value::Object(map))
        }
        _ => Err(Error::TransferArchive),
    }
}
pub(crate) fn xml_value(xml: &str) -> Result<Value, Error> {
    if xml.len() > METADATA_LIMIT {
        return Err(Error::TransferArchive);
    }
    let doc = roxmltree::Document::parse_with_options(
        xml,
        roxmltree::ParsingOptions {
            allow_dtd: false,
            nodes_limit: 100_000,
        },
    )
    .map_err(|_| Error::TransferArchive)?;
    value(doc.root_element(), 0)
}
pub(crate) fn inspect_with_stop(
    file: &mut std::fs::File,
    cancel: &dyn Fn() -> bool,
) -> Result<ArchiveSummary, Error> {
    let compression = super::compression::detect(file)?;
    let result = header(&mut super::compression::decoder(
        (&mut *file).take((2 * METADATA_LIMIT) as u64),
        compression,
        cancel,
    )?);
    files::rewind(file)?;
    if cancel() {
        return Err(Error::Closed);
    }
    let xml = result?;
    let data = xml_value(std::str::from_utf8(&xml).map_err(|_| Error::TransferArchive)?)?;
    let objects = data["objects"]
        .as_array()
        .filter(|a| a.len() <= 2048)
        .ok_or(Error::TransferArchive)?;
    let memories = super::checkpoint::memory_graph(objects)?;
    let mut guests = vec![];
    let mut disks = 0;
    let mut virtual_bytes = 0u64;
    let mut networks = 0;
    let mut ids = BTreeSet::new();
    for object in objects {
        let id = object["id"]
            .as_str()
            .filter(|id| id.starts_with("Ref:") && id.len() <= 128)
            .ok_or(Error::TransferArchive)?;
        if !ids.insert(id) {
            return Err(Error::TransferArchive);
        }
        let record = object["snapshot"]
            .as_object()
            .ok_or(Error::TransferArchive)?;
        match object["class"].as_str().ok_or(Error::TransferArchive)? {
            "VM" => {
                if record.get("is_control_domain") != Some(&Value::Bool(false))
                    || record.get("is_a_snapshot") == Some(&Value::Bool(true))
                {
                    return Err(Error::TransferArchive);
                }
                let template = record
                    .get("is_a_template")
                    .and_then(Value::as_bool)
                    .ok_or(Error::TransferArchive)?;
                let name = record
                    .get("name_label")
                    .and_then(Value::as_str)
                    .filter(|s| s.len() <= 1024)
                    .ok_or(Error::TransferArchive)?
                    .to_owned();
                let uuid = record
                    .get("uuid")
                    .and_then(Value::as_str)
                    .and_then(|s| s.parse().ok())
                    .ok_or(Error::TransferArchive)?;
                if record.get("other_config").is_some_and(|v| {
                    v["default_template"] == "true" || v["is_default_template"] == "true"
                }) {
                    return Err(Error::TransferArchive);
                }
                guests.push(ArchiveGuest {
                    memory: memories.get(id).cloned(),
                    uuid,
                    name,
                    template,
                });
            }
            "VDI" => {
                if matches!(
                    record.get("type").and_then(Value::as_str),
                    Some("user" | "suspend")
                ) {
                    let bytes = record
                        .get("virtual_size")
                        .and_then(|v| v.as_u64().or_else(|| v.as_str()?.parse().ok()))
                        .filter(|n| *n > 0 && *n <= MAX_TRANSFER_BYTES)
                        .ok_or(Error::TransferArchive)?;
                    virtual_bytes = virtual_bytes
                        .checked_add(bytes)
                        .filter(|n| *n <= MAX_TRANSFER_BYTES)
                        .ok_or(Error::TransferArchive)?;
                    if record.get("type").and_then(Value::as_str) == Some("user") {
                        disks += 1;
                    }
                }
            }
            "network" => networks += 1,
            "host" | "SR" | "VBD" | "VIF" | "VM_metrics" | "VM_guest_metrics" => {}
            _ => return Err(Error::TransferArchive),
        }
    }
    if guests.is_empty() || guests.len() > 64 || disks > 256 || networks > 256 {
        return Err(Error::TransferArchive);
    }
    Ok(ArchiveSummary {
        compression: Some(compression),
        guests,
        data_disks: disks,
        virtual_bytes,
        networks,
        metadata_digest: Sha256::digest(&xml).into(),
    })
}
