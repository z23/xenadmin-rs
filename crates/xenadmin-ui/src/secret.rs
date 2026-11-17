//! Password drafts.
//!
//! The Rune editors held a secure `TextEdit` and "cleared" it by replacing the
//! buffer. Here a draft is a [`SecretText`]: its bytes are overwritten when it
//! is cleared, replaced or dropped, and it never prints its text.

use std::fmt;

use zeroize::Zeroize;

/// A secret being typed. Zeroized on [`SecretText::clear`], on
/// [`SecretText::set`] and on drop.
#[derive(Default)]
pub struct SecretText(String);

impl SecretText {
    pub fn new() -> Self {
        Self(String::new())
    }

    /// Replaces the text. The previous text is overwritten first.
    pub fn set(&mut self, text: &str) {
        self.0.zeroize();
        self.0.push_str(text);
    }

    /// Overwrites the text and leaves the draft empty.
    pub fn clear(&mut self) {
        self.0.zeroize();
    }

    /// The text, for handing to the core. Do not copy it into a `String`
    /// that outlives the call.
    pub fn expose(&self) -> &str {
        &self.0
    }

    pub fn is_empty(&self) -> bool {
        self.0.is_empty()
    }

    /// Length in characters, for a masked display.
    pub fn len(&self) -> usize {
        self.0.chars().count()
    }

    /// Moves the text out and leaves this draft empty. The returned value is
    /// zeroized when it is dropped.
    pub fn take(&mut self) -> SecretText {
        SecretText(std::mem::take(&mut self.0))
    }
}

impl From<&str> for SecretText {
    fn from(text: &str) -> Self {
        Self(text.to_owned())
    }
}

/// A copy is a second zeroizing buffer. Descriptions carry one so the front
/// end can keep its input in step with the draft.
impl Clone for SecretText {
    fn clone(&self) -> Self {
        Self(self.0.clone())
    }
}

impl Drop for SecretText {
    fn drop(&mut self) {
        self.0.zeroize();
    }
}

impl PartialEq for SecretText {
    fn eq(&self, other: &Self) -> bool {
        self.0 == other.0
    }
}

impl Eq for SecretText {}

impl fmt::Debug for SecretText {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        if self.0.is_empty() {
            f.write_str("SecretText(empty)")
        } else {
            f.write_str("SecretText(<redacted>)")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn debug_never_prints_the_text() {
        let mut secret = SecretText::from("hunter2");
        assert_eq!(format!("{secret:?}"), "SecretText(<redacted>)");
        assert!(!format!("{:?}", Some(&secret)).contains("hunter2"));
        secret.clear();
        assert_eq!(format!("{secret:?}"), "SecretText(empty)");
    }

    #[test]
    fn clear_set_and_take_leave_no_text_behind() {
        let mut secret = SecretText::new();
        assert!(secret.is_empty());
        secret.set("first");
        secret.set("second");
        assert_eq!(secret.expose(), "second");
        assert_eq!(secret.len(), 6);
        let taken = secret.take();
        assert!(secret.is_empty());
        assert_eq!(taken.expose(), "second");
        let mut copy = taken.clone();
        assert_eq!(copy, taken);
        copy.clear();
        assert!(copy.is_empty());
        assert_eq!(copy.expose(), "");
        assert_eq!(taken.expose(), "second");
    }
}
