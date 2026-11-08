use crate::Error;
use vnc::{
    CLIPBOARD_CAPS, CLIPBOARD_NOTIFY, CLIPBOARD_PEEK, CLIPBOARD_PROVIDE, CLIPBOARD_REQUEST,
    CLIPBOARD_TEXT, ClipboardMessage, VncClient, X11Event,
};

pub(super) const MAX_TEXT: usize = 256 * 1024;
#[derive(Default)]
pub(super) struct Clipboard {
    pub flags: u32,
    pub guest: Option<String>,
    outgoing: Option<String>,
    unsolicited_limit: u32,
}
impl Clipboard {
    pub fn utf8(&self) -> bool {
        self.flags & (CLIPBOARD_TEXT | CLIPBOARD_PROVIDE) == CLIPBOARD_TEXT | CLIPBOARD_PROVIDE
    }
    pub async fn send(&mut self, client: &VncClient, text: String) -> Result<(), vnc::VncError> {
        if !self.utf8() {
            return client.input(X11Event::CopyText(text)).await;
        }
        // CRLF normalization may double byte length, with one final NUL.
        let size = text
            .len()
            .saturating_add(text.bytes().filter(|b| *b == b'\n').count())
            .saturating_add(1);
        self.outgoing = Some(text.clone());
        if size <= self.unsolicited_limit as usize {
            client
                .input(X11Event::Clipboard(ClipboardMessage::text(text)))
                .await
        } else if self.flags & CLIPBOARD_NOTIFY != 0 {
            client
                .input(X11Event::Clipboard(ClipboardMessage::control(
                    CLIPBOARD_NOTIFY | CLIPBOARD_TEXT,
                )))
                .await
        } else {
            Err(vnc::VncError::InvalidImageData)
        }
    }
    pub async fn receive(
        &mut self,
        client: &VncClient,
        message: ClipboardMessage,
    ) -> Result<(), vnc::VncError> {
        if message.flags & CLIPBOARD_CAPS != 0 {
            self.flags = message.flags;
            self.unsolicited_limit = if message.flags & CLIPBOARD_TEXT != 0 {
                message.sizes.first().copied().unwrap_or(0)
            } else {
                0
            };
            let caps = ClipboardMessage {
                flags: CLIPBOARD_CAPS
                    | CLIPBOARD_TEXT
                    | CLIPBOARD_REQUEST
                    | CLIPBOARD_PEEK
                    | CLIPBOARD_NOTIFY
                    | CLIPBOARD_PROVIDE,
                sizes: vec![0],
                text: None,
            };
            client.input(X11Event::Clipboard(caps)).await?;
        } else if self.flags == 0 {
            return Err(vnc::VncError::InvalidImageData);
        } else if message.flags & CLIPBOARD_PROVIDE != 0 {
            if let Some(text) = message.text {
                if text.len() > MAX_TEXT {
                    return Err(vnc::VncError::InvalidImageData);
                }
                self.guest = Some(text);
            }
        } else if message.flags & CLIPBOARD_NOTIFY != 0 {
            // Retain in-app only. Copying to the OS clipboard is always a user action.
            self.guest = None;
            if message.flags & CLIPBOARD_TEXT != 0 && self.flags & CLIPBOARD_REQUEST != 0 {
                client
                    .input(X11Event::Clipboard(ClipboardMessage::control(
                        CLIPBOARD_REQUEST | CLIPBOARD_TEXT,
                    )))
                    .await?;
            }
        } else if message.flags & CLIPBOARD_REQUEST != 0 && message.flags & CLIPBOARD_TEXT != 0 {
            if let Some(text) = &self.outgoing {
                client
                    .input(X11Event::Clipboard(ClipboardMessage::text(text.clone())))
                    .await?;
            }
        } else if message.flags & CLIPBOARD_PEEK != 0 && self.flags & CLIPBOARD_NOTIFY != 0 {
            client
                .input(X11Event::Clipboard(ClipboardMessage::control(
                    CLIPBOARD_NOTIFY
                        | if self.outgoing.is_some() {
                            CLIPBOARD_TEXT
                        } else {
                            0
                        },
                )))
                .await?;
        }
        Ok(())
    }
}
pub(super) fn validate(text: &str, utf8: bool) -> Result<(), Error> {
    if text.len() > MAX_TEXT {
        return Err(Error::ConsoleClipboardTooLarge);
    }
    if text.contains('\0') || (!utf8 && text.chars().any(|c| c as u32 > 255)) {
        return Err(Error::ConsoleClipboardUnsupported);
    }
    Ok(())
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn clipboard_validation_never_substitutes_or_truncates() {
        assert!(validate("café £", false).is_ok());
        assert!(matches!(
            validate("日本語", false),
            Err(Error::ConsoleClipboardUnsupported)
        ));
        assert!(validate("日本語", true).is_ok());
        assert!(validate("nul\0bad", true).is_err());
        assert!(matches!(
            validate(&"a".repeat(MAX_TEXT + 1), true),
            Err(Error::ConsoleClipboardTooLarge)
        ));
    }
}
