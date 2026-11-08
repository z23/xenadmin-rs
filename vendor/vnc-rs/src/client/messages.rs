use crate::{PixelFormat, Rect, VncEncoding, VncError};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt};

#[derive(Debug)]
pub(super) enum ClientMsg {
    SetPixelFormat(PixelFormat),
    SetEncodings(Vec<VncEncoding>),
    FramebufferUpdateRequest(Rect, u8),
    KeyEvent(u32, bool),
    ExtendedKeyEvent(u32, u32, bool),
    Clipboard(crate::ClipboardMessage),
    PointerEvent(u16, u16, u8),
    ClientCutText(String),
    #[cfg(not(target_arch = "wasm32"))]
    SetDesktopSize(crate::DesktopLayout),
}

impl ClientMsg {
    pub(super) async fn write<S>(self, writer: &mut S) -> Result<(), VncError>
    where
        S: AsyncWrite + Unpin,
    {
        match self {
            ClientMsg::SetPixelFormat(pf) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [0]       | message-type |
                // | 3            |              | padding      |
                // | 16           | PIXEL_FORMAT | pixel-format |
                // +--------------+--------------+--------------+
                let mut payload = vec![0_u8, 0, 0, 0];
                payload.extend(<PixelFormat as Into<Vec<u8>>>::into(pf));
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::SetEncodings(encodings) => {
                //  +--------------+--------------+---------------------+
                // | No. of bytes | Type [Value] | Description         |
                // +--------------+--------------+---------------------+
                // | 1            | U8 [2]       | message-type        |
                // | 1            |              | padding             |
                // | 2            | U16          | number-of-encodings |
                // +--------------+--------------+---------------------+

                // This is followed by number-of-encodings repetitions of the following:
                // +--------------+--------------+---------------+
                // | No. of bytes | Type [Value] | Description   |
                // +--------------+--------------+---------------+
                // | 4            | S32          | encoding-type |
                // +--------------+--------------+---------------+
                let mut payload = vec![2, 0];
                payload.extend_from_slice(&(encodings.len() as u16).to_be_bytes());
                for e in encodings {
                    payload.write_u32(e.into()).await?;
                }
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::FramebufferUpdateRequest(rect, incremental) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [3]       | message-type |
                // | 1            | U8           | incremental  |
                // | 2            | U16          | x-position   |
                // | 2            | U16          | y-position   |
                // | 2            | U16          | width        |
                // | 2            | U16          | height       |
                // +--------------+--------------+--------------+
                let mut payload = vec![3, incremental];
                payload.extend_from_slice(&rect.x.to_be_bytes());
                payload.extend_from_slice(&rect.y.to_be_bytes());
                payload.extend_from_slice(&rect.width.to_be_bytes());
                payload.extend_from_slice(&rect.height.to_be_bytes());
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::ExtendedKeyEvent(keysym, keycode, down) => {
                let mut payload = vec![255, 0];
                payload.extend_from_slice(&u16::from(down).to_be_bytes());
                payload.extend_from_slice(&keysym.to_be_bytes());
                payload.extend_from_slice(&keycode.to_be_bytes());
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::Clipboard(message) => {
                let data = message.encode()?;
                let mut payload = vec![6, 0, 0, 0];
                payload.extend_from_slice(&(-(data.len() as i32)).to_be_bytes());
                payload.extend_from_slice(&data);
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::KeyEvent(keycode, down) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [4]       | message-type |
                // | 1            | U8           | down-flag    |
                // | 2            |              | padding      |
                // | 4            | U32          | key          |
                // +--------------+--------------+--------------+
                let mut payload = vec![4, down as u8, 0, 0];
                payload.write_u32(keycode).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::PointerEvent(x, y, mask) => {
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [5]       | message-type |
                // | 1            | U8           | button-mask  |
                // | 2            | U16          | x-position   |
                // | 2            | U16          | y-position   |
                // +--------------+--------------+--------------+
                let mut payload = vec![5, mask];
                payload.write_u16(x).await?;
                payload.write_u16(y).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
            #[cfg(not(target_arch = "wasm32"))]
            ClientMsg::SetDesktopSize(layout) => {
                layout.validate()?;
                let mut payload = vec![251, 0];
                payload.extend_from_slice(&layout.width.to_be_bytes());
                payload.extend_from_slice(&layout.height.to_be_bytes());
                payload.extend_from_slice(&[layout.screens.len() as u8, 0]);
                for screen in layout.screens {
                    payload.extend_from_slice(&screen.id.to_be_bytes());
                    payload.extend_from_slice(&screen.x.to_be_bytes());
                    payload.extend_from_slice(&screen.y.to_be_bytes());
                    payload.extend_from_slice(&screen.width.to_be_bytes());
                    payload.extend_from_slice(&screen.height.to_be_bytes());
                    payload.extend_from_slice(&screen.flags.to_be_bytes());
                }
                // Some servers release resize confirmations only with an outstanding
                // update request. Keep it ordered with the resize and bounded to one pixel.
                payload.extend_from_slice(&[3, 1, 0, 0, 0, 0, 0, 1, 0, 1]);
                writer.write_all(&payload).await?;
                Ok(())
            }
            ClientMsg::ClientCutText(s) => {
                //   +--------------+--------------+--------------+
                //   | No. of bytes | Type [Value] | Description  |
                //   +--------------+--------------+--------------+
                //   | 1            | U8 [6]       | message-type |
                //   | 3            |              | padding      |
                //   | 4            | U32          | length       |
                //   | length       | U8 array     | text         |
                //   +--------------+--------------+--------------+
                let mut payload = vec![6_u8, 0, 0, 0];
                let text = crate::clipboard::latin1(&s)?;
                payload.write_u32(text.len() as u32).await?;
                payload.write_all(&text).await?;
                writer.write_all(&payload).await?;
                Ok(())
            }
        }
    }
}

#[derive(Debug)]
pub(super) enum ServerMsg {
    FramebufferUpdate(u16),
    // SetColorMapEntries,
    Bell,
    ServerCutText(String),
    Clipboard(crate::ClipboardMessage),
}

impl ServerMsg {
    pub(super) async fn read<S>(reader: &mut S) -> Result<Self, VncError>
    where
        S: AsyncRead + Unpin,
    {
        let server_msg = reader.read_u8().await?;

        match server_msg {
            0 => {
                // FramebufferUpdate
                //   +--------------+--------------+----------------------+
                //   | No. of bytes | Type [Value] | Description          |
                //   +--------------+--------------+----------------------+
                //   | 1            | U8 [0]       | message-type         |
                //   | 1            |              | padding              |
                //   | 2            | U16          | number-of-rectangles |
                //   +--------------+--------------+----------------------+
                let _padding = reader.read_u8().await?;
                let rects = reader.read_u16().await?;
                Ok(ServerMsg::FramebufferUpdate(rects))
            }
            1 => {
                // SetColorMapEntries
                // +--------------+--------------+------------------+
                // | No. of bytes | Type [Value] | Description      |
                // +--------------+--------------+------------------+
                // | 1            | U8 [1]       | message-type     |
                // | 1            |              | padding          |
                // | 2            | U16          | first-color      |
                // | 2            | U16          | number-of-colors |
                // +--------------+--------------+------------------+
                Err(VncError::WrongServerMessage)
            }
            2 => {
                // Bell
                //   +--------------+--------------+--------------+
                //   | No. of bytes | Type [Value] | Description  |
                //   +--------------+--------------+--------------+
                //   | 1            | U8 [2]       | message-type |
                //   +--------------+--------------+--------------+
                Ok(ServerMsg::Bell)
            }
            3 => {
                // ServerCutText
                // +--------------+--------------+--------------+
                // | No. of bytes | Type [Value] | Description  |
                // +--------------+--------------+--------------+
                // | 1            | U8 [3]       | message-type |
                // | 3            |              | padding      |
                // | 4            | U32          | length       |
                // | length       | U8 array     | text         |
                // +--------------+--------------+--------------+
                let mut padding = [0; 3];
                reader.read_exact(&mut padding).await?;
                let length = reader.read_i32().await?;
                let size = length.unsigned_abs() as usize;
                if size > crate::limits::MAX_TEXT || (length < 0 && size < 4) {
                    return Err(VncError::InvalidImageData);
                }
                let mut data = vec![0; size];
                reader.read_exact(&mut data).await?;
                if length < 0 {
                    Ok(Self::Clipboard(crate::ClipboardMessage::decode(&data)?))
                } else {
                    Ok(Self::ServerCutText(
                        data.into_iter().map(char::from).collect(),
                    ))
                }
            }
            _ => Err(VncError::WrongServerMessage),
        }
    }
}

#[cfg(test)]
mod console_wire_tests {
    use super::*;
    #[tokio::test]
    async fn key_events_preserve_the_negotiated_wire_format() {
        let mut baseline = Vec::new();
        ClientMsg::KeyEvent(0xff53, true)
            .write(&mut baseline)
            .await
            .unwrap();
        assert_eq!(baseline, [4, 1, 0, 0, 0, 0, 0xff, 0x53]);
        let mut extended = Vec::new();
        ClientMsg::ExtendedKeyEvent(0xff53, 0xcd, false)
            .write(&mut extended)
            .await
            .unwrap();
        assert_eq!(extended, [255, 0, 0, 0, 0, 0, 0xff, 0x53, 0, 0, 0, 0xcd]);
    }
    #[tokio::test]
    async fn baseline_clipboard_is_latin1_in_both_directions() {
        let mut packet = Vec::new();
        ClientMsg::ClientCutText("café".into())
            .write(&mut packet)
            .await
            .unwrap();
        assert_eq!(packet, b"\x06\0\0\0\0\0\0\x04caf\xe9");
        packet[0] = 3;
        assert!(
            matches!(ServerMsg::read(&mut packet.as_slice()).await.unwrap(), ServerMsg::ServerCutText(text) if text == "café")
        );
        let mut packet = Vec::new();
        assert!(ClientMsg::ClientCutText("漢字".into())
            .write(&mut packet)
            .await
            .is_err());
        assert!(packet.is_empty());
    }
    #[tokio::test]
    async fn extended_clipboard_has_negative_length_and_utf8_payload() {
        let mut packet = Vec::new();
        ClientMsg::Clipboard(crate::ClipboardMessage::text("日本語\n🦀".into()))
            .write(&mut packet)
            .await
            .unwrap();
        assert_eq!(packet[0], 6);
        assert_eq!(
            i32::from_be_bytes(packet[4..8].try_into().unwrap()),
            -((packet.len() - 8) as i32)
        );
        packet[0] = 3;
        assert!(
            matches!(ServerMsg::read(&mut packet.as_slice()).await.unwrap(), ServerMsg::Clipboard(message) if message.text.as_deref() == Some("日本語\n🦀"))
        );
    }
}
