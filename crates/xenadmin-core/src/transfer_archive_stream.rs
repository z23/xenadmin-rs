use super::{Error, MAX_TRANSFER_BYTES, archive, files};
use sha1::{Digest, Sha1};
use std::{collections::BTreeMap, io::Read};
use tokio_util::sync::CancellationToken;

const CHUNK: u64 = 1024 * 1024;
const METADATA: usize = 8 * 1024 * 1024;

#[cfg(test)]
#[path = "transfer_archive_stream_tests.rs"]
mod tests;

struct Header {
    name: String,
    size: u64,
}

fn decode(
    reader: &mut dyn Read,
    compression: super::XvaCompression,
    expected_metadata: &[u8; 32],
    cancel: &dyn Fn() -> bool,
) -> Result<(), Error> {
    let result = super::compression::decoder(reader, compression, cancel)
        .and_then(|mut reader| stream(&mut reader, expected_metadata, cancel));
    if cancel() { Err(Error::Closed) } else { result }
}

fn invalid<T>() -> Result<T, Error> {
    Err(Error::TransferArchive)
}

fn read_exact(reader: &mut dyn Read, bytes: &mut [u8]) -> Result<(), Error> {
    reader.read_exact(bytes).map_err(|_| Error::TransferArchive)
}

fn octal(bytes: &[u8]) -> Result<u64, Error> {
    let text = std::str::from_utf8(bytes)
        .map_err(|_| Error::TransferArchive)?
        .trim_matches(['\0', ' ']);
    u64::from_str_radix(text, 8).map_err(|_| Error::TransferArchive)
}

fn header(reader: &mut dyn Read) -> Result<Option<Header>, Error> {
    let mut bytes = [0; 512];
    read_exact(reader, &mut bytes)?;
    if bytes.iter().all(|byte| *byte == 0) {
        read_exact(reader, &mut bytes)?;
        if bytes.iter().any(|byte| *byte != 0) {
            return invalid();
        }
        return Ok(None);
    }
    let checksum: u64 = bytes
        .iter()
        .enumerate()
        .map(|(i, byte)| {
            if (148..156).contains(&i) {
                32
            } else {
                u64::from(*byte)
            }
        })
        .sum();
    if octal(&bytes[148..156])? != checksum
        || ![0, b'0'].contains(&bytes[156])
        || bytes[157..257].iter().any(|b| *b != 0)
        || bytes[345..500].iter().any(|b| *b != 0)
    {
        return invalid();
    }
    let end = bytes[..100]
        .iter()
        .position(|b| *b == 0)
        .ok_or(Error::TransferArchive)?;
    if bytes[end..100].iter().any(|b| *b != 0) {
        return invalid();
    }
    let name = std::str::from_utf8(&bytes[..end])
        .map_err(|_| Error::TransferArchive)?
        .to_owned();
    Ok(Some(Header {
        name,
        size: octal(&bytes[124..136])?,
    }))
}

fn padding(reader: &mut dyn Read, size: u64) -> Result<(), Error> {
    let mut bytes = [0; 511];
    let length = ((512 - size % 512) % 512) as usize;
    read_exact(reader, &mut bytes[..length])?;
    if bytes[..length].iter().any(|byte| *byte != 0) {
        return invalid();
    }
    Ok(())
}

fn payload_disks(xml: &[u8]) -> Result<BTreeMap<String, u64>, Error> {
    let value = archive::xml_value(std::str::from_utf8(xml).map_err(|_| Error::TransferArchive)?)?;
    let objects = value["objects"]
        .as_array()
        .filter(|a| a.len() <= 2048)
        .ok_or(Error::TransferArchive)?;
    let _ = super::checkpoint::memory_graph(objects)?;
    // Full XVA exports stream user and reviewed suspend disks; ISO references are metadata only.
    let iso_srs = objects
        .iter()
        .filter(|o| o["class"] == "SR" && o["snapshot"]["content_type"] == "iso")
        .filter_map(|o| o["id"].as_str())
        .collect::<std::collections::BTreeSet<_>>();
    let mut disks = BTreeMap::new();
    let mut total = 0u64;
    for object in objects.iter().filter(|o| o["class"] == "VDI") {
        let record = &object["snapshot"];
        if record["SR"].as_str().is_some_and(|sr| iso_srs.contains(sr)) {
            continue;
        }
        if record["type"] != "user" && record["type"] != "suspend" {
            return invalid();
        }
        let id = object["id"]
            .as_str()
            .filter(|id| {
                id.starts_with("Ref:")
                    && id[4..].bytes().all(|b| b.is_ascii_digit())
                    && id.len() > 4
                    && id.len() <= 128
            })
            .ok_or(Error::TransferArchive)?;
        let size = record["virtual_size"]
            .as_u64()
            .or_else(|| record["virtual_size"].as_str()?.parse().ok())
            .filter(|n| *n > 0 && *n <= MAX_TRANSFER_BYTES)
            .ok_or(Error::TransferArchive)?;
        total = total
            .checked_add(size)
            .filter(|n| *n <= MAX_TRANSFER_BYTES)
            .ok_or(Error::TransferArchive)?;
        if disks.insert(id.to_owned(), size).is_some() || disks.len() > 256 {
            return invalid();
        }
    }
    Ok(disks)
}

fn stream(
    reader: &mut dyn Read,
    expected_metadata: &[u8; 32],
    cancel: &dyn Fn() -> bool,
) -> Result<(), Error> {
    use sha2::Sha256;
    let first = header(reader)?.ok_or(Error::TransferArchive)?;
    if first.name != "ova.xml" || first.size == 0 || first.size > METADATA as u64 {
        return invalid();
    }
    let mut xml = vec![0; first.size as usize];
    read_exact(reader, &mut xml)?;
    padding(reader, first.size)?;
    if Sha256::digest(&xml).as_slice() != expected_metadata {
        return invalid();
    }
    let disks = payload_disks(&xml)?;
    let mut next = header(reader)?;
    let mut bytes = vec![0; CHUNK as usize];
    for (disk, size) in disks {
        let mut previous = None;
        let last = (size - 1) / CHUNK;
        loop {
            if cancel() {
                return Err(Error::Closed);
            }
            let current = next.take().ok_or(Error::TransferArchive)?;
            let (prefix, suffix) = current.name.split_once('/').ok_or(Error::TransferArchive)?;
            if prefix != disk || suffix.len() != 8 || !suffix.bytes().all(|b| b.is_ascii_digit()) {
                return invalid();
            }
            let index: u64 = suffix.parse().map_err(|_| Error::TransferArchive)?;
            if index > last
                || previous.is_none() && index != 0
                || previous.is_some_and(|p| index <= p)
            {
                return invalid();
            }
            let length = (size - index * CHUNK).min(CHUNK);
            if current.size != length {
                return invalid();
            }
            read_exact(reader, &mut bytes[..length as usize])?;
            padding(reader, length)?;
            let checksum = header(reader)?.ok_or(Error::TransferArchive)?;
            let digest = if checksum.name == format!("{}.xxhash", current.name)
                && checksum.size == 16
            {
                format!(
                    "{:016X}",
                    xxhash_rust::xxh64::xxh64(&bytes[..length as usize], 0)
                )
            } else if checksum.name == format!("{}.checksum", current.name) && checksum.size == 40 {
                format!("{:x}", Sha1::digest(&bytes[..length as usize]))
            } else {
                return invalid();
            };
            let mut saved = [0; 40];
            read_exact(reader, &mut saved[..checksum.size as usize])?;
            if !saved[..checksum.size as usize].eq_ignore_ascii_case(digest.as_bytes()) {
                return invalid();
            }
            padding(reader, checksum.size)?;
            next = header(reader)?;
            if index == last {
                break;
            }
            previous = Some(index);
        }
    }
    if next.is_some() {
        return invalid();
    }
    // Permit bounded conventional zero record padding, but never appended archives or data.
    let mut trailing = 0usize;
    loop {
        if cancel() {
            return Err(Error::Closed);
        }
        let count = reader
            .read(&mut bytes)
            .map_err(|_| Error::TransferArchive)?;
        if count == 0 {
            break;
        }
        trailing += count;
        if trailing > CHUNK as usize || bytes[..count].iter().any(|b| *b != 0) {
            return invalid();
        }
    }
    Ok(())
}

pub(crate) fn verify_with_stop(
    file: &mut std::fs::File,
    expected_metadata: &[u8; 32],
    cancel: &dyn Fn() -> bool,
) -> Result<(), Error> {
    files::rewind(file)?;
    let compression = super::compression::detect(file)?;
    let result = decode(file, compression, expected_metadata, cancel);
    files::rewind(file)?;
    result
}

pub(crate) fn verify(
    file: &mut std::fs::File,
    expected_metadata: &[u8; 32],
    cancel: &CancellationToken,
) -> Result<(), Error> {
    verify_with_stop(file, expected_metadata, &|| cancel.is_cancelled())
}
