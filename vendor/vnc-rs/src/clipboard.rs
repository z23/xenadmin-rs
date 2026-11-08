//! RFB extended clipboard wire format. No OS clipboard is read or written here.
use crate::VncError;
use flate2::{read::ZlibDecoder, write::ZlibEncoder, Compression};
use std::io::{Read, Write};

pub const CLIPBOARD_TEXT: u32 = 1;
pub const CLIPBOARD_CAPS: u32 = 1 << 24;
pub const CLIPBOARD_REQUEST: u32 = 1 << 25;
pub const CLIPBOARD_PEEK: u32 = 1 << 26;
pub const CLIPBOARD_NOTIFY: u32 = 1 << 27;
pub const CLIPBOARD_PROVIDE: u32 = 1 << 28;
const MAX: usize = crate::limits::MAX_TEXT;

#[derive(Clone)]
pub struct ClipboardMessage {
    pub flags: u32,
    /// Maximum unsolicited bytes for each advertised format, in bit order.
    pub sizes: Vec<u32>,
    pub text: Option<String>,
}
impl std::fmt::Debug for ClipboardMessage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("ClipboardMessage")
            .field("flags", &self.flags)
            .finish_non_exhaustive()
    }
}
impl ClipboardMessage {
    pub fn control(flags: u32) -> Self {
        Self {
            flags,
            sizes: Vec::new(),
            text: None,
        }
    }
    pub fn text(text: String) -> Self {
        Self {
            flags: CLIPBOARD_PROVIDE | CLIPBOARD_TEXT,
            sizes: Vec::new(),
            text: Some(text),
        }
    }
    pub(crate) fn encode(&self) -> Result<Vec<u8>, VncError> {
        let mut data = self.flags.to_be_bytes().to_vec();
        if self.flags & CLIPBOARD_CAPS != 0 {
            if self.sizes.len() != (self.flags & 0xffff).count_ones() as usize {
                return Err(VncError::InvalidImageData);
            }
            for size in &self.sizes {
                data.extend_from_slice(&size.to_be_bytes());
            }
        } else if self.flags & CLIPBOARD_PROVIDE != 0 {
            if self.flags != CLIPBOARD_PROVIDE | CLIPBOARD_TEXT {
                return Err(VncError::InvalidImageData);
            }
            let text = self.text.as_ref().ok_or(VncError::InvalidImageData)?;
            if text.contains('\0') || text.len() > MAX / 2 {
                return Err(VncError::InvalidImageData);
            }
            let text = text.replace("\r\n", "\n").replace('\n', "\r\n");
            let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
            encoder.write_all(&((text.len() + 1) as u32).to_be_bytes())?;
            encoder.write_all(text.as_bytes())?;
            encoder.write_all(&[0])?;
            data.extend(encoder.finish()?);
        } else if (self.flags >> 24).count_ones() != 1 {
            return Err(VncError::InvalidImageData);
        }
        if data.len() > MAX {
            return Err(VncError::InvalidImageData);
        }
        Ok(data)
    }
    pub(crate) fn decode(data: &[u8]) -> Result<Self, VncError> {
        if data.len() < 4 || data.len() > MAX {
            return Err(VncError::InvalidImageData);
        }
        let flags = u32::from_be_bytes(data[..4].try_into().unwrap());
        let mut message = Self::control(flags);
        let formats = flags & 0xffff;
        if flags & CLIPBOARD_CAPS != 0 {
            if data.len() != 4 + formats.count_ones() as usize * 4 {
                return Err(VncError::InvalidImageData);
            }
            message.sizes = data[4..]
                .chunks_exact(4)
                .map(|x| u32::from_be_bytes(x.try_into().unwrap()))
                .collect();
        } else if (flags >> 24).count_ones() != 1 {
            return Err(VncError::InvalidImageData);
        } else if flags & CLIPBOARD_PROVIDE != 0 {
            let mut decoded = Vec::new();
            ZlibDecoder::new(&data[4..])
                .take(MAX as u64 + 1)
                .read_to_end(&mut decoded)?;
            if decoded.len() > MAX {
                return Err(VncError::InvalidImageData);
            }
            let mut rest = decoded.as_slice();
            for bit in 0..16 {
                if formats & (1 << bit) == 0 {
                    continue;
                }
                if rest.len() < 4 {
                    return Err(VncError::InvalidImageData);
                }
                let length = u32::from_be_bytes(rest[..4].try_into().unwrap()) as usize;
                rest = &rest[4..];
                if length > rest.len() {
                    return Err(VncError::InvalidImageData);
                }
                if bit == 0 {
                    let text = rest[..length]
                        .strip_suffix(&[0])
                        .ok_or(VncError::InvalidImageData)?;
                    if text.contains(&0) {
                        return Err(VncError::InvalidImageData);
                    }
                    message.text = Some(
                        std::str::from_utf8(text)
                            .map_err(|_| VncError::InvalidImageData)?
                            .replace("\r\n", "\n"),
                    );
                }
                rest = &rest[length..];
            }
            if !rest.is_empty() {
                return Err(VncError::InvalidImageData);
            }
        } else if data.len() != 4 {
            return Err(VncError::InvalidImageData);
        }
        Ok(message)
    }
}

pub(crate) fn latin1(text: &str) -> Result<Vec<u8>, VncError> {
    if text.len() > MAX {
        return Err(VncError::InvalidImageData);
    }
    text.chars()
        .map(|c| u8::try_from(c as u32).map_err(|_| VncError::InvalidImageData))
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn latin1_preserves_bytes_and_refuses_unicode_loss() {
        assert_eq!(latin1("café £").unwrap(), b"caf\xe9 \xa3");
        assert!(latin1("漢字").is_err());
    }
    #[test]
    fn extended_unicode_round_trip_and_caps() {
        let source = "Latin café\n日本語 🦀";
        let packet = ClipboardMessage::text(source.into()).encode().unwrap();
        assert_eq!(&packet[..4], &0x1000_0001u32.to_be_bytes());
        assert_eq!(
            ClipboardMessage::decode(&packet).unwrap().text.as_deref(),
            Some(source)
        );
        let caps = ClipboardMessage {
            flags: CLIPBOARD_CAPS | CLIPBOARD_TEXT | CLIPBOARD_PROVIDE,
            sizes: vec![0],
            text: None,
        };
        assert_eq!(
            ClipboardMessage::decode(&caps.encode().unwrap())
                .unwrap()
                .sizes,
            vec![0]
        );
    }
    #[test]
    fn malformed_and_oversized_clipboards_are_rejected() {
        assert!(ClipboardMessage::decode(&[1, 0, 0, 1]).is_err());
        assert!(ClipboardMessage::decode(&[0x12, 0, 0, 1]).is_err());
        let mut encoder = ZlibEncoder::new(Vec::new(), Compression::fast());
        encoder.write_all(&vec![0; MAX + 1]).unwrap();
        let mut data = (CLIPBOARD_PROVIDE | CLIPBOARD_TEXT).to_be_bytes().to_vec();
        data.extend(encoder.finish().unwrap());
        assert!(ClipboardMessage::decode(&data).is_err());
        assert!(ClipboardMessage::text("nul\0bad".into()).encode().is_err());
    }
}
