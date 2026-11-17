//! What the model asks the front end to do.
//!
//! Opening a surface is NOT an intent: a command sets the area's editor and
//! the surface appears in [`Model::surfaces`]. An intent is only for a
//! service the model cannot perform itself: a file prompt, opening a URL,
//! the clipboard, a console window, quitting.
//!
//! The model queues intents in its outbox ([`Model::send`] and the helpers
//! next to it). The front end drains the outbox with
//! [`Model::take_intents`] after every update, performs each intent, and for
//! the ones that carry a reply calls it with the answer.

use std::any::Any;
use std::fmt;
use std::path::PathBuf;
use std::rc::Rc;

use crate::model::Model;

/// The reply to a file prompt: the chosen path, or `None` when the user
/// cancelled or no portal answered.
pub type FileReply = Rc<dyn Fn(&mut Model, Option<PathBuf>)>;
/// The reply to a clipboard read: the text, or `None` when the clipboard
/// holds no text.
pub type ClipboardReply = Rc<dyn Fn(&mut Model, Option<String>)>;

#[derive(Clone)]
pub enum Intent {
    /// Ask the user for a file or a directory.
    ChooseFile(FileRequest),
    /// Open a URL in the user's browser.
    OpenUrl(String),
    /// Put text on the clipboard. Only on an explicit user action.
    CopyText(String),
    /// Read the clipboard text. Only on an explicit user action.
    ReadClipboard(ClipboardReply),
    /// Open, close or raise a detached console window.
    ConsoleWindow(ConsoleWindowRequest),
    /// Run the close sequence and exit.
    Quit,
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum FileMode {
    /// Pick an existing file.
    Open,
    /// Pick a path to write to.
    Save,
    /// Pick an existing directory.
    Directory,
}

/// A named set of file extensions, without the dot: `("Disk images", ["vhd", "qcow2"])`.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct FileFilter {
    pub name: String,
    pub extensions: Vec<String>,
}

#[derive(Clone)]
pub struct FileRequest {
    pub title: String,
    pub mode: FileMode,
    /// The file name offered in a save prompt.
    pub suggested_name: Option<String>,
    pub filters: Vec<FileFilter>,
    pub reply: FileReply,
}

impl FileRequest {
    fn new(
        title: impl Into<String>,
        mode: FileMode,
        reply: impl Fn(&mut Model, Option<PathBuf>) + 'static,
    ) -> Self {
        Self {
            title: title.into(),
            mode,
            suggested_name: None,
            filters: Vec::new(),
            reply: Rc::new(reply),
        }
    }

    pub fn open(
        title: impl Into<String>,
        reply: impl Fn(&mut Model, Option<PathBuf>) + 'static,
    ) -> Self {
        Self::new(title, FileMode::Open, reply)
    }

    pub fn save(
        title: impl Into<String>,
        suggested_name: impl Into<String>,
        reply: impl Fn(&mut Model, Option<PathBuf>) + 'static,
    ) -> Self {
        Self {
            suggested_name: Some(suggested_name.into()),
            ..Self::new(title, FileMode::Save, reply)
        }
    }

    pub fn directory(
        title: impl Into<String>,
        reply: impl Fn(&mut Model, Option<PathBuf>) + 'static,
    ) -> Self {
        Self::new(title, FileMode::Directory, reply)
    }

    /// Adds a filter: a name and extensions without the dot.
    pub fn filter(
        mut self,
        name: impl Into<String>,
        extensions: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        self.filters.push(FileFilter {
            name: name.into(),
            extensions: extensions.into_iter().map(Into::into).collect(),
        });
        self
    }
}

/// The id of a detached console window. The Rune client had eight slots.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, PartialOrd, Ord)]
pub struct ConsoleWindowId(pub u32);

#[derive(Clone)]
pub enum ConsoleWindowRequest {
    /// Open the window. `payload` is whatever the console area and the
    /// console view agree on (the connection clone, the target); the kernel
    /// does not look at it.
    Open {
        id: ConsoleWindowId,
        payload: Option<Rc<dyn Any>>,
    },
    Close(ConsoleWindowId),
    /// Raise and focus the window.
    Activate(ConsoleWindowId),
}

impl fmt::Debug for Intent {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ChooseFile(request) => f.debug_tuple("ChooseFile").field(request).finish(),
            Self::OpenUrl(url) => f.debug_tuple("OpenUrl").field(url).finish(),
            Self::CopyText(text) => f.debug_tuple("CopyText").field(text).finish(),
            Self::ReadClipboard(_) => f.write_str("ReadClipboard"),
            Self::ConsoleWindow(request) => f.debug_tuple("ConsoleWindow").field(request).finish(),
            Self::Quit => f.write_str("Quit"),
        }
    }
}

impl fmt::Debug for FileRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("FileRequest")
            .field("title", &self.title)
            .field("mode", &self.mode)
            .field("suggested_name", &self.suggested_name)
            .field("filters", &self.filters)
            .finish_non_exhaustive()
    }
}

impl fmt::Debug for ConsoleWindowRequest {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Open { id, payload } => f
                .debug_struct("Open")
                .field("id", id)
                .field("payload", &payload.is_some())
                .finish(),
            Self::Close(id) => f.debug_tuple("Close").field(id).finish(),
            Self::Activate(id) => f.debug_tuple("Activate").field(id).finish(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_outbox_is_drained_in_order_and_replies_reach_the_model() {
        let mut m = Model::fixture();
        m.choose_file(
            FileRequest::save("Export report", "report.csv", |m, path| {
                m.error = path.map(|path| path.display().to_string());
            })
            .filter("CSV", ["csv"]),
        );
        m.open_url("https://example.invalid/help");
        m.copy_text("uuid");
        m.read_clipboard(|m, text| m.error = text);
        m.send(Intent::ConsoleWindow(ConsoleWindowRequest::Activate(
            ConsoleWindowId(2),
        )));
        m.quit();
        let intents = m.take_intents();
        assert!(m.take_intents().is_empty());
        assert_eq!(intents.len(), 6);
        let Intent::ChooseFile(request) = &intents[0] else {
            panic!("expected a file request, got {:?}", intents[0]);
        };
        assert_eq!(request.mode, FileMode::Save);
        assert_eq!(request.suggested_name.as_deref(), Some("report.csv"));
        assert_eq!(request.filters[0].extensions, ["csv"]);
        (request.reply)(&mut m, Some(PathBuf::from("/tmp/report.csv")));
        assert_eq!(m.error.as_deref(), Some("/tmp/report.csv"));
        assert!(matches!(&intents[1], Intent::OpenUrl(url) if url.ends_with("/help")));
        assert!(matches!(&intents[2], Intent::CopyText(text) if text == "uuid"));
        let Intent::ReadClipboard(reply) = &intents[3] else {
            panic!("expected a clipboard read");
        };
        reply(&mut m, Some("pasted".into()));
        assert_eq!(m.error.as_deref(), Some("pasted"));
        assert_eq!(
            format!("{:?}", intents[4]),
            "ConsoleWindow(Activate(ConsoleWindowId(2)))"
        );
        assert!(matches!(intents[5], Intent::Quit));
    }
}
