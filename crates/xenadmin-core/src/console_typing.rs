//! Text typed into a console as key presses, for guests whose tools offer no clipboard
//! exchange. Every character is a press and a release of the key a US keyboard has for it,
//! with Shift held around the ones that need it, so the guest must use a US layout for
//! punctuation to come out as written.

use crate::Error;

/// The longest text one request may type, in characters.
pub const MAX_TYPED_TEXT: usize = 2000;
/// The pause after each typed character. A console's keyboard queue is short, and a guest
/// that is sent keys faster than it reads them drops some.
pub(super) const PACE: std::time::Duration = std::time::Duration::from_millis(30);

const SHIFT: u32 = 0xffe1;
const RETURN: u32 = 0xff0d;
const TAB: u32 = 0xff09;

/// One typed character: the key, and whether Shift is held around it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) struct Stroke {
    pub keysym: u32,
    pub shift: bool,
}
impl Stroke {
    /// The key events of this character in order, as keysym and whether it goes down.
    pub fn events(self) -> impl Iterator<Item = (u32, bool)> {
        let key = [(self.keysym, true), (self.keysym, false)];
        self.shift
            .then_some((SHIFT, true))
            .into_iter()
            .chain(key)
            .chain(self.shift.then_some((SHIFT, false)))
    }
}

/// What typing a text would do, for a review before it is sent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct TypedText {
    /// Characters typed, each line break counted once.
    pub characters: usize,
    /// Line breaks, each typed as Enter.
    pub line_breaks: usize,
}
impl TypedText {
    /// Checks that `text` can be typed and says how much there is of it. Only what a US
    /// keyboard types directly is accepted: printable ASCII, Tab and line breaks. A text
    /// with anything else is refused whole, so that nothing is left out silently.
    pub fn review(text: &str) -> Result<Self, Error> {
        let strokes = strokes(text)?;
        Ok(Self {
            characters: strokes.len(),
            line_breaks: strokes.iter().filter(|s| s.keysym == RETURN).count(),
        })
    }
}

pub(super) fn strokes(text: &str) -> Result<Vec<Stroke>, Error> {
    let mut strokes = Vec::new();
    let mut characters = text.chars().peekable();
    while let Some(character) = characters.next() {
        let stroke = match character {
            '\r' if characters.peek() == Some(&'\n') => continue,
            '\n' | '\r' => Stroke {
                keysym: RETURN,
                shift: false,
            },
            '\t' => Stroke {
                keysym: TAB,
                shift: false,
            },
            ' '..='~' => Stroke {
                keysym: character as u32,
                shift: character.is_ascii_uppercase()
                    || "~!@#$%^&*()_+{}|:\"<>?".contains(character),
            },
            _ => return Err(Error::ConsoleTextUntypable),
        };
        strokes.push(stroke);
        if strokes.len() > MAX_TYPED_TEXT {
            return Err(Error::ConsoleTextTooLong);
        }
    }
    if strokes.is_empty() {
        return Err(Error::ConsoleTextEmpty);
    }
    Ok(strokes)
}

#[cfg(test)]
mod tests {
    use super::*;

    fn typed(text: &str) -> Vec<(u32, bool)> {
        strokes(text)
            .unwrap()
            .into_iter()
            .flat_map(Stroke::events)
            .collect()
    }

    #[test]
    fn letters_digits_and_punctuation_are_typed_as_a_us_keyboard_types_them() {
        assert_eq!(typed("a"), [(0x61, true), (0x61, false)]);
        assert_eq!(
            typed("A"),
            [(SHIFT, true), (0x41, true), (0x41, false), (SHIFT, false)]
        );
        // Every printable ASCII character is accepted, and Shift is held around exactly
        // the upper-case letters and the upper row of each symbol key.
        let shifted: String = (' '..='~')
            .filter(|c| strokes(&c.to_string()).unwrap()[0].shift)
            .collect();
        assert_eq!(shifted, "!\"#$%&()*+:<>?@ABCDEFGHIJKLMNOPQRSTUVWXYZ^_{|}~");
        assert_eq!(strokes(&(' '..='~').collect::<String>()).unwrap().len(), 95);
    }

    #[test]
    fn line_breaks_are_enter_once_each_and_tab_is_tab() {
        assert_eq!(
            typed("a\r\nb\n\tc\r"),
            [
                (0x61, true),
                (0x61, false),
                (RETURN, true),
                (RETURN, false),
                (0x62, true),
                (0x62, false),
                (RETURN, true),
                (RETURN, false),
                (TAB, true),
                (TAB, false),
                (0x63, true),
                (0x63, false),
                (RETURN, true),
                (RETURN, false),
            ]
        );
        assert_eq!(
            TypedText::review("one\r\ntwo\nthree").unwrap(),
            TypedText {
                characters: 13,
                line_breaks: 2
            }
        );
    }

    #[test]
    fn a_text_with_anything_else_is_refused_whole() {
        for text in [
            "caf\u{e9}",
            "a\u{1b}b",
            "\u{201c}quoted\u{201d}",
            "tab\u{b}",
            "nul\0",
        ] {
            assert!(
                matches!(TypedText::review(text), Err(Error::ConsoleTextUntypable)),
                "{text:?}"
            );
        }
        assert!(matches!(
            TypedText::review(""),
            Err(Error::ConsoleTextEmpty)
        ));
        assert!(matches!(
            TypedText::review("\r\n").map(|t| t.characters),
            Ok(1)
        ));
        assert!(TypedText::review(&"x".repeat(MAX_TYPED_TEXT)).is_ok());
        assert!(matches!(
            TypedText::review(&"x".repeat(MAX_TYPED_TEXT + 1)),
            Err(Error::ConsoleTextTooLong)
        ));
    }
}
