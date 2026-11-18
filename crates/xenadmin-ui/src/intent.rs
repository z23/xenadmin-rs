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
/// cancelled. A prompt that could not be shown at all answers through
/// [`FileRequest::on_failure`] when the request has one, and like a
/// cancelled prompt otherwise.
pub type FileReply = Rc<dyn Fn(&mut Model, Option<PathBuf>)>;
/// The reply to a file prompt that failed: what the desktop said.
pub type FileFailure = Rc<dyn Fn(&mut Model, &str)>;
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
    /// Toggle the fullscreen state of a window: into fullscreen when it is
    /// not, out of it when it is.
    ToggleFullscreen(WindowRef),
    /// Run the close sequence and exit.
    Quit,
}

/// A window of the application.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash)]
pub enum WindowRef {
    /// The main window.
    Main,
    /// A detached console window.
    Console(ConsoleWindowId),
}

/// How a file prompt ended. The front end hands it to
/// [`FileRequest::answer`].
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum FileAnswer {
    Chosen(PathBuf),
    /// The user closed the prompt without choosing.
    Cancelled,
    /// No prompt could be shown and nothing was chosen in its place: what
    /// the desktop said.
    Failed(String),
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
    /// What a prompt that could not be shown answers with
    /// ([`FileRequest::on_failure`]). `None`: it answers like a cancel.
    pub failed: Option<FileFailure>,
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
            failed: None,
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

    /// What to do when the prompt could not be shown: the handler gets the
    /// desktop's error text ("Cannot open save dialog: {error}"). Without
    /// it a failed prompt answers the reply with `None`, like a cancel.
    pub fn on_failure(mut self, failed: impl Fn(&mut Model, &str) + 'static) -> Self {
        self.failed = Some(Rc::new(failed));
        self
    }

    /// Hands the request its answer: the front end's one call, whatever
    /// happened. A failure without a failure handler is a cancel.
    pub fn answer(&self, m: &mut Model, answer: FileAnswer) {
        match (answer, &self.failed) {
            (FileAnswer::Chosen(path), _) => (self.reply)(m, Some(path)),
            (FileAnswer::Failed(error), Some(failed)) => failed(m, &error),
            (FileAnswer::Cancelled | FileAnswer::Failed(_), _) => (self.reply)(m, None),
        }
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
            Self::ToggleFullscreen(window) => {
                f.debug_tuple("ToggleFullscreen").field(window).finish()
            }
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
            .field("on_failure", &self.failed.is_some())
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

    #[test]
    fn a_failed_prompt_reaches_its_failure_handler_and_is_a_cancel_without_one() {
        let mut m = Model::fixture();
        let plain = FileRequest::open("Import", |m, path| {
            m.error = Some(format!("plain: {path:?}"));
        });
        plain.answer(&mut m, FileAnswer::Failed("no portal".into()));
        assert_eq!(m.error.as_deref(), Some("plain: None"));
        let request = FileRequest::save("Export", "x.json", |m, path| {
            m.error = Some(format!("reply: {path:?}"));
        })
        .on_failure(|m, error| m.error = Some(format!("Cannot open save dialog: {error}")));
        assert!(format!("{request:?}").contains("on_failure: true"));
        request.answer(&mut m, FileAnswer::Failed("no portal".into()));
        assert_eq!(
            m.error.as_deref(),
            Some("Cannot open save dialog: no portal")
        );
        request.answer(&mut m, FileAnswer::Cancelled);
        assert_eq!(m.error.as_deref(), Some("reply: None"));
        request.answer(&mut m, FileAnswer::Chosen(PathBuf::from("/tmp/x.json")));
        assert_eq!(m.error.as_deref(), Some("reply: Some(\"/tmp/x.json\")"));
        // A fullscreen request names its window.
        m.toggle_fullscreen(WindowRef::Console(ConsoleWindowId(3)));
        m.toggle_fullscreen(WindowRef::Main);
        assert_eq!(
            format!("{:?}", m.take_intents()),
            "[ToggleFullscreen(Console(ConsoleWindowId(3))), ToggleFullscreen(Main)]"
        );
    }
}
