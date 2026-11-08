use super::*;
use std::io::Write;
fn put32(b: &mut [u8], at: usize, n: u32) {
    b[at..at + 4].copy_from_slice(&n.to_be_bytes());
}
fn put64(b: &mut [u8], at: usize, n: u64) {
    b[at..at + 8].copy_from_slice(&n.to_be_bytes());
}
fn sign(b: &mut [u8], at: usize) {
    put32(b, at, 0);
    let sum = b.iter().fold(0u32, |a, v| a.wrapping_add(u32::from(*v)));
    put32(b, at, !sum);
}
fn footer(size: u64, kind: u32) -> Vec<u8> {
    let mut f = vec![0; 512];
    f[..8].copy_from_slice(b"conectix");
    put32(&mut f, 8, 2);
    put32(&mut f, 12, 0x10000);
    put64(&mut f, 16, if kind == 2 { u64::MAX } else { 512 });
    put64(&mut f, 40, size);
    put64(&mut f, 48, size);
    put32(&mut f, 60, kind);
    sign(&mut f, 64);
    f
}
pub(crate) fn fixed() -> Vec<u8> {
    let mut bytes = vec![7; 4096];
    bytes.extend(footer(4096, 2));
    bytes
}
pub(crate) fn dynamic(allocated: bool) -> Vec<u8> {
    let f = footer(8192, 3);
    let mut b = f.clone();
    let mut h = vec![0; 1024];
    h[..8].copy_from_slice(b"cxsparse");
    put64(&mut h, 8, u64::MAX);
    put64(&mut h, 16, 1536);
    put32(&mut h, 24, 0x10000);
    put32(&mut h, 28, 2);
    put32(&mut h, 32, 4096);
    sign(&mut h, 36);
    b.extend(h);
    b.resize(2048, 0xff);
    if allocated {
        put32(&mut b, 1536, 4);
        put32(&mut b, 1540, 13);
        b.resize(2048 + 2 * (512 + 4096), 0);
    }
    b.extend(f);
    b
}
fn inspect_bytes(bytes: &[u8], format: DiskImageFormat) -> Result<DiskImageSummary, Error> {
    let mut f = tempfile::tempfile().unwrap();
    f.write_all(bytes).unwrap();
    f.rewind().unwrap();
    inspect(&mut f, format)
}
#[test]
fn complete_raw_fixed_and_sparse_dynamic_images_are_reviewable() {
    for (bytes, format, size, sparse) in [
        (vec![1; 4096], DiskImageFormat::Raw, 4096, false),
        (fixed(), DiskImageFormat::Vhd, 4096, false),
        (dynamic(false), DiskImageFormat::Vhd, 8192, true),
        (dynamic(true), DiskImageFormat::Vhd, 8192, true),
    ] {
        let d = inspect_bytes(&bytes, format).unwrap();
        assert_eq!(d.virtual_size, size);
        assert_eq!(d.sparse, sparse);
    }
}
#[test]
fn truncated_bad_checksums_and_parent_images_are_refused() {
    let mut bad = fixed();
    bad[0] = 0;
    let end = bad.len();
    bad[end - 512 + 64] ^= 1;
    let mut parent = dynamic(false);
    let len = parent.len();
    put32(&mut parent[len - 512..], 60, 4);
    sign(&mut parent[len - 512..], 64);
    let mut header = dynamic(false);
    header[512 + 40] = 1;
    sign(&mut header[512..1536], 36);
    for b in [vec![], vec![0; 511], bad, parent, header, vec![0; 4096]] {
        assert!(inspect_bytes(&b, DiskImageFormat::Vhd).is_err());
    }
    assert!(inspect_bytes(&vec![0; 513], DiskImageFormat::Raw).is_err());
}
#[test]
fn overlapping_or_out_of_bounds_dynamic_blocks_never_reach_transport() {
    for sector in [0, 1, 3, 4, 5, u32::MAX - 1] {
        let mut b = dynamic(true);
        put32(&mut b, 1540, sector);
        assert!(inspect_bytes(&b, DiskImageFormat::Vhd).is_err(), "{sector}");
    }
    let mut b = dynamic(true);
    put64(&mut b[512..1536], 16, 512);
    sign(&mut b[512..1536], 36);
    assert!(inspect_bytes(&b, DiskImageFormat::Vhd).is_err());
}
#[test]
fn invalid_virtual_size_and_bat_entry_count_are_refused() {
    for size in [0, 513, VHD_LIMIT + 512, u64::MAX] {
        let mut b = fixed();
        let end = b.len();
        put64(&mut b[end - 512..], 48, size);
        sign(&mut b[end - 512..], 64);
        assert!(inspect_bytes(&b, DiskImageFormat::Vhd).is_err());
    }
    let mut b = dynamic(false);
    put32(&mut b[512..1536], 28, u32::MAX);
    sign(&mut b[512..1536], 36);
    assert!(inspect_bytes(&b, DiskImageFormat::Vhd).is_err());
}
#[test]
fn image_selection_captures_identity_and_never_accepts_replacement() {
    let d = tempfile::tempdir().unwrap();
    let path = d.path().join("disk.vhd");
    std::fs::write(&path, fixed()).unwrap();
    let selected = crate::TransferFile::disk_source(&path, DiskImageFormat::Vhd).unwrap();
    assert_eq!(selected.disk_image().unwrap().virtual_size, 4096);
    std::fs::remove_file(&path).unwrap();
    std::fs::write(&path, fixed()).unwrap();
    assert!(crate::transfer::files::source(&selected).is_err());
}
