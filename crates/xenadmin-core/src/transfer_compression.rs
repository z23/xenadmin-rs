use super::{Error, files};
use serde::{Deserialize, Serialize};
use std::io::Read;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum XvaCompression {
    #[default]
    Uncompressed,
    Gzip,
    Zstd,
}
impl XvaCompression {
    pub const ALL: [Self; 3] = [Self::Uncompressed, Self::Gzip, Self::Zstd];
    pub fn label(self) -> &'static str {
        match self {
            Self::Uncompressed => "Uncompressed",
            Self::Gzip => "gzip",
            Self::Zstd => "zstd",
        }
    }
    pub fn extension(self) -> &'static str {
        match self {
            Self::Uncompressed => "xva",
            Self::Gzip => "xva.gz",
            Self::Zstd => "xva.zst",
        }
    }
    pub(crate) fn query(self) -> &'static str {
        match self {
            Self::Uncompressed => "false",
            Self::Gzip => "gzip",
            Self::Zstd => "zstd",
        }
    }
}

pub(super) fn detect(file: &mut std::fs::File) -> Result<XvaCompression, Error> {
    files::rewind(file)?;
    let mut magic = [0; 4];
    let result = file
        .read_exact(&mut magic)
        .map_err(|_| Error::TransferArchive);
    files::rewind(file)?;
    result?;
    Ok(if magic == [0x28, 0xb5, 0x2f, 0xfd] {
        XvaCompression::Zstd
    } else if magic[..2] == [0x1f, 0x8b] {
        XvaCompression::Gzip
    } else {
        XvaCompression::Uncompressed
    })
}

struct Cancellable<'a, R> {
    reader: R,
    cancel: &'a dyn Fn() -> bool,
}
impl<R: Read> Read for Cancellable<'_, R> {
    fn read(&mut self, bytes: &mut [u8]) -> std::io::Result<usize> {
        if (self.cancel)() {
            return Err(std::io::Error::other("archive verification cancelled"));
        }
        let length = bytes.len().min(64 * 1024);
        self.reader.read(&mut bytes[..length])
    }
}

pub(super) fn decoder<'a>(
    reader: impl Read + 'a,
    compression: XvaCompression,
    cancel: &'a dyn Fn() -> bool,
) -> Result<Box<dyn Read + 'a>, Error> {
    let input = Cancellable { reader, cancel };
    match compression {
        XvaCompression::Uncompressed => Ok(Box::new(input)),
        XvaCompression::Gzip => Ok(Box::new(flate2::read::MultiGzDecoder::new(input))),
        XvaCompression::Zstd => {
            let mut decoder =
                zstd::stream::read::Decoder::new(input).map_err(|_| Error::TransferArchive)?;
            // Bound decoder history to 128 MiB; larger custom windows are refused.
            decoder
                .window_log_max(27)
                .map_err(|_| Error::TransferArchive)?;
            Ok(Box::new(decoder))
        }
    }
}
