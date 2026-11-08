use crate::Error;
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
};
const VHD_LIMIT: u64 = 2040 * 1024 * 1024 * 1024;
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum DiskImageFormat {
    Raw,
    Vhd,
}
impl DiskImageFormat {
    pub fn extension(self) -> &'static str {
        match self {
            Self::Raw => "raw",
            Self::Vhd => "vhd",
        }
    }
    pub(crate) fn limit(self) -> u64 {
        match self {
            Self::Raw => crate::transfer::MAX_TRANSFER_BYTES,
            Self::Vhd => VHD_LIMIT,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct DiskImageSummary {
    pub format: DiskImageFormat,
    pub virtual_size: u64,
    pub sparse: bool,
}
fn u32be(b: &[u8], at: usize) -> u32 {
    u32::from_be_bytes(b[at..at + 4].try_into().expect("bounded image field"))
}
fn u64be(b: &[u8], at: usize) -> u64 {
    u64::from_be_bytes(b[at..at + 8].try_into().expect("bounded image field"))
}
fn checksum(b: &[u8], at: usize) -> bool {
    let sum = b
        .iter()
        .enumerate()
        .filter(|(i, _)| !(at..at + 4).contains(i))
        .fold(0u32, |sum, (_, v)| sum.wrapping_add(u32::from(*v)));
    !sum == u32be(b, at)
}
fn read_at<const N: usize>(f: &mut (impl Read + Seek), offset: u64) -> Result<[u8; N], Error> {
    f.seek(SeekFrom::Start(offset))
        .map_err(|_| Error::TransferFile)?;
    let mut data = [0; N];
    f.read_exact(&mut data).map_err(|_| Error::DiskImage)?;
    Ok(data)
}
fn size_ok(n: u64, format: DiskImageFormat) -> bool {
    n > 0 && n <= format.limit() && n.is_multiple_of(512)
}
pub(crate) fn inspect(f: &mut File, format: DiskImageFormat) -> Result<DiskImageSummary, Error> {
    let length = f.metadata().map_err(|_| Error::TransferFile)?.len();
    inspect_reader(f, length, format)
}
pub(crate) fn inspect_reader(
    f: &mut (impl Read + Seek),
    length: u64,
    format: DiskImageFormat,
) -> Result<DiskImageSummary, Error> {
    if length == 0 || length > crate::transfer::MAX_TRANSFER_BYTES {
        return Err(Error::TransferLimit);
    }
    if format == DiskImageFormat::Raw {
        if !size_ok(length, format) {
            return Err(Error::DiskImage);
        }
        return Ok(DiskImageSummary {
            format,
            virtual_size: length,
            sparse: false,
        });
    }
    if length < 1024 || !length.is_multiple_of(512) {
        return Err(Error::DiskImage);
    }
    let footer = read_at::<512>(f, length - 512)?;
    if &footer[..8] != b"conectix"
        || u32be(&footer, 12) != 0x10000
        || u32be(&footer, 8) & !3 != 0
        || !checksum(&footer, 64)
        || footer[84] != 0
    {
        return Err(Error::DiskImage);
    }
    let virtual_size = u64be(&footer, 48);
    if !size_ok(virtual_size, format) || !size_ok(u64be(&footer, 40), format) {
        return Err(Error::DiskImage);
    }
    let sparse = match u32be(&footer, 60) {
        2 => {
            if u64be(&footer, 16) != u64::MAX
                || length != virtual_size.checked_add(512).ok_or(Error::DiskImage)?
            {
                return Err(Error::DiskImage);
            }
            false
        }
        3 => {
            if read_at::<512>(f, 0)? != footer {
                return Err(Error::DiskImage);
            }
            let header_at = u64be(&footer, 16);
            if header_at < 512
                || !header_at.is_multiple_of(512)
                || header_at.checked_add(1024).is_none_or(|n| n > length - 512)
            {
                return Err(Error::DiskImage);
            }
            let header = read_at::<1024>(f, header_at)?;
            if &header[..8] != b"cxsparse"
                || u64be(&header, 8) != u64::MAX
                || u32be(&header, 24) != 0x10000
                || !checksum(&header, 36)
                || header[40..].iter().any(|v| *v != 0)
            {
                return Err(Error::DiskImage);
            }
            let block = u64::from(u32be(&header, 32));
            if !(512..=256 * 1024 * 1024).contains(&block) || !block.is_power_of_two() {
                return Err(Error::DiskImage);
            }
            let entries = u64::from(u32be(&header, 28));
            if entries != virtual_size.div_ceil(block) || entries > 2 * 1024 * 1024 {
                return Err(Error::DiskImage);
            }
            let bat_at = u64be(&header, 16);
            let bat_size = (entries * 4).div_ceil(512) * 512;
            if !bat_at.is_multiple_of(512)
                || bat_at < header_at + 1024
                || bat_at
                    .checked_add(bat_size)
                    .is_none_or(|n| n > length - 512)
            {
                return Err(Error::DiskImage);
            }
            let mut bat = vec![0u8; (entries * 4) as usize];
            f.seek(SeekFrom::Start(bat_at))
                .map_err(|_| Error::TransferFile)?;
            f.read_exact(&mut bat).map_err(|_| Error::DiskImage)?;
            let bitmap = block.div_ceil(512).div_ceil(8).div_ceil(512) * 512;
            let mut ranges = Vec::new();
            for entry in bat.as_chunks::<4>().0 {
                let sector = u32be(entry, 0);
                if sector == u32::MAX {
                    continue;
                }
                let at = u64::from(sector) * 512;
                let end = at
                    .checked_add(bitmap)
                    .and_then(|n| n.checked_add(block))
                    .ok_or(Error::DiskImage)?;
                if at < bat_at + bat_size || end > length - 512 {
                    return Err(Error::DiskImage);
                }
                ranges.push((at, end));
            }
            ranges.sort_unstable();
            if ranges.windows(2).any(|pair| pair[0].1 > pair[1].0) {
                return Err(Error::DiskImage);
            }
            true
        }
        _ => return Err(Error::DiskImage),
    };
    f.rewind().map_err(|_| Error::TransferFile)?;
    Ok(DiskImageSummary {
        format,
        virtual_size,
        sparse,
    })
}

#[cfg(test)]
#[path = "disk_image_tests.rs"]
pub(crate) mod tests;
