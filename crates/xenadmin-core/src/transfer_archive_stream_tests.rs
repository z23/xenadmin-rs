use super::*;
use serde_json::json;
use sha2::Sha256;
use std::io::{Cursor, Write};

fn metadata() -> Vec<u8> {
    crate::transfer::tests::xml(&json!({"objects":[
        {"class":"VDI","id":"Ref:10","snapshot":{"type":"user","virtual_size":2*CHUNK+7}},
        {"class":"VDI","id":"Ref:2","snapshot":{"type":"user","virtual_size":4096}}
    ]}))
    .into_bytes()
}

fn entries() -> Vec<(String, Vec<u8>)> {
    let mut entries = vec![("ova.xml".into(), metadata())];
    for (name, data, sha) in [
        ("Ref:10/00000000", vec![1; CHUNK as usize], false),
        ("Ref:10/00000002", vec![2; 7], false),
        ("Ref:2/00000000", vec![3; 4096], true),
    ] {
        let (suffix, digest) = if sha {
            ("checksum", format!("{:x}", Sha1::digest(&data)))
        } else {
            (
                "xxhash",
                format!("{:016X}", xxhash_rust::xxh64::xxh64(&data, 0)),
            )
        };
        entries.push((name.into(), data));
        entries.push((format!("{name}.{suffix}"), digest.into_bytes()));
    }
    entries
}

fn archive(entries: &[(String, Vec<u8>)]) -> Vec<u8> {
    let mut output = tar::Builder::new(Vec::new());
    for (name, data) in entries {
        let mut h = tar::Header::new_ustar();
        h.set_size(data.len() as u64);
        h.set_mode(0o600);
        h.set_cksum();
        output.append_data(&mut h, name, data.as_slice()).unwrap();
    }
    output.into_inner().unwrap()
}

fn check(bytes: &[u8]) -> Result<(), Error> {
    stream(
        &mut Cursor::new(bytes),
        &Sha256::digest(metadata()).into(),
        &CancellationToken::new(),
    )
}

#[test]
fn checks_sparse_disks_and_both_native_inline_checksum_formats() {
    let data = archive(&entries());
    check(&data).unwrap();
    let dir = tempfile::tempdir().unwrap();
    for compressed in [false, true] {
        let mut file = std::fs::File::create(dir.path().join("input.xva")).unwrap();
        if compressed {
            let mut encoder = flate2::write::GzEncoder::new(file, flate2::Compression::default());
            encoder.write_all(&data).unwrap();
            encoder.finish().unwrap();
        } else {
            file.write_all(&data).unwrap();
        }
        let mut file = std::fs::File::open(dir.path().join("input.xva")).unwrap();
        verify(
            &mut file,
            &Sha256::digest(metadata()).into(),
            &CancellationToken::new(),
        )
        .unwrap();
    }
    // Independent standard XXH64 seed-zero vector.
    assert_eq!(xxhash_rust::xxh64::xxh64(b"", 0), 0xef46db3751d8e999);
}

#[test]
fn rejects_corrupt_or_missing_payload_and_checksum_records() {
    for remove in 1..7 {
        let mut e = entries();
        e.remove(remove);
        assert!(check(&archive(&e)).is_err(), "missing entry {remove}");
    }
    for index in 1..7 {
        let mut e = entries();
        e[index].1[0] ^= 1;
        assert!(check(&archive(&e)).is_err(), "corrupted entry {index}");
    }
    let mut e = entries();
    e[1].1.pop();
    assert!(check(&archive(&e)).is_err());
}

#[test]
fn rejects_duplicate_out_of_order_foreign_and_out_of_bounds_chunks() {
    for name in [
        "Ref:10/00000001",
        "Ref:2/00000000",
        "Ref:10/99999999",
        "Ref:10/0",
        "Ref:10/0000000x",
    ] {
        let mut e = entries();
        e[1].0 = name.into();
        assert!(check(&archive(&e)).is_err(), "{name}");
    }
    let mut e = entries();
    e.insert(3, e[1].clone());
    e.insert(4, e[2].clone());
    assert!(check(&archive(&e)).is_err());
    let mut e = entries();
    e.swap(1, 3);
    e.swap(2, 4);
    assert!(check(&archive(&e)).is_err());
    let mut e = entries();
    e.push(("unexpected".into(), vec![0]));
    assert!(check(&archive(&e)).is_err());
}

#[test]
fn requires_complete_regular_tar_and_matching_bounded_metadata() {
    let data = archive(&entries());
    for length in [0, 511, 513, data.len() - 513] {
        assert!(check(&data[..length]).is_err());
    }
    let mut modified = data.clone();
    modified[148] ^= 1;
    assert!(check(&modified).is_err());
    let mut modified = data.clone();
    modified.extend_from_slice(b"extra");
    assert!(check(&modified).is_err());
    let mut modified = data.clone();
    modified.extend_from_slice(&vec![0; CHUNK as usize + 1]);
    assert!(check(&modified).is_err());
    assert!(stream(&mut Cursor::new(&data), &[0; 32], &CancellationToken::new()).is_err());
    let cancel = CancellationToken::new();
    cancel.cancel();
    assert!(matches!(
        stream(
            &mut Cursor::new(&data),
            &Sha256::digest(metadata()).into(),
            &cancel
        ),
        Err(Error::Closed)
    ));
}

#[test]
fn cancellation_interrupts_scanning_empty_gzip_members_before_a_tar_header() {
    struct CancelsDuringRead {
        data: Cursor<Vec<u8>>,
        count: usize,
        cancel: CancellationToken,
    }
    impl Read for CancelsDuringRead {
        fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
            self.count += 1;
            let length = bytes.len().min(7);
            let count = self.data.read(&mut bytes[..length])?;
            if self.count == 8 {
                self.cancel.cancel();
            }
            Ok(count)
        }
    }
    let empty = flate2::write::GzEncoder::new(Vec::new(), flate2::Compression::default())
        .finish()
        .unwrap();
    let cancel = CancellationToken::new();
    let mut reader = CancelsDuringRead {
        data: Cursor::new(empty.repeat(100)),
        count: 0,
        cancel: cancel.clone(),
    };
    assert!(matches!(
        decode(&mut reader, true, &[0; 32], &cancel),
        Err(Error::Closed)
    ));
    assert_eq!(reader.count, 8);
}
