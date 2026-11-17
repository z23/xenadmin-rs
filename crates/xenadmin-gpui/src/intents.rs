//! What the application does for the model: the services of
//! `xenadmin_ui::intent::Intent`.
//!
//! The store drains the model's outbox after every update and hands each
//! intent to the handler [`install`] puts there:
//!
//! - `ChooseFile`: the desktop's file chooser (`cx.prompt_for_paths`,
//!   `cx.prompt_for_new_path`). When no portal answers, a dialog asks for
//!   the path as text instead (plan risk R9).
//! - `OpenUrl`, `CopyText`, `ReadClipboard`: the platform's.
//! - `Quit`: the application's one close path.
//! - `ConsoleWindow`: SEAM, see [`Hooks`].
//!
//! A reply goes back to the model in an update of its own, after the one
//! that queued the intent has finished.

use std::cell::RefCell;
use std::path::{Path, PathBuf};
use std::rc::Rc;

use xenadmin_ui::Model;
use xenadmin_ui::blocks;
use xenadmin_ui::intent::{ConsoleWindowRequest, FileMode, FileRequest, Intent};
use xenadmin_ui::surface::{Block, NoticeKind, Surface, button, input, notice as notice_block};

use crate::kit::*;
use crate::store::AppStore;

/// What the log says for a console window request until the console is
/// ported.
pub const CONSOLE_WINDOWS_NOT_PORTED: &str = "console windows are not ported yet";

/// What the application provides beyond the platform's services.
///
/// SEAM FOR THE CONSOLE ENGINEER: the three console hooks do nothing yet.
/// Replace them where the application is built (`app::run`); nothing else
/// in the shell needs to change.
#[derive(Clone)]
pub struct Hooks {
    /// Runs the close path of the main window and quits. `Intent::Quit`.
    pub quit: Rc<dyn Fn(&mut App)>,
    /// Opens, closes or activates a detached console window.
    /// `Intent::ConsoleWindow`.
    pub console_window: Rc<dyn Fn(ConsoleWindowRequest, &mut App)>,
    /// Releases the keys and buttons a console holds down in its guest.
    /// Called when the main window is deactivated and when it closes.
    pub release_console_input: Rc<dyn Fn(&mut App)>,
    /// Closes every detached console window. Called by the close path.
    pub close_console_windows: Rc<dyn Fn(&mut App)>,
}

impl Default for Hooks {
    fn default() -> Self {
        Self {
            quit: Rc::new(|cx| cx.quit()),
            console_window: Rc::new(|request, _| {
                eprintln!("{CONSOLE_WINDOWS_NOT_PORTED}: {request:?}");
            }),
            release_console_input: Rc::new(|_| {}),
            close_console_windows: Rc::new(|_| {}),
        }
    }
}

/// The id of the dialog that asks for a path as text.
pub const TYPED_PATH: &str = "choose-path";
/// The key of its text field.
pub const TYPED_PATH_INPUT: &str = "choose-path-input";
/// The key of the line that says why the dialog stands in for a chooser.
pub const TYPED_PATH_REASON: &str = "choose-path-reason";
/// The keys of its buttons.
pub const TYPED_PATH_CANCEL: &str = "choose-path-cancel";
pub const TYPED_PATH_CONFIRM: &str = "choose-path-confirm";

/// A file request the desktop's chooser could not answer.
struct TypedPath {
    request: FileRequest,
    text: String,
    /// What the portal said.
    problem: String,
    error: Option<String>,
    /// The path the user was told exists already: confirming it again
    /// replaces the file.
    replace: Option<PathBuf>,
}

/// The state the intent handler keeps between intents.
#[derive(Clone, Default)]
pub struct Services {
    typed: Rc<RefCell<Option<TypedPath>>>,
    /// The folder of the last file chosen, where the next chooser starts.
    directory: Rc<RefCell<Option<PathBuf>>>,
}

fn home() -> PathBuf {
    std::env::var_os("HOME")
        .map(PathBuf::from)
        .filter(|home| home.is_dir())
        .unwrap_or_else(|| PathBuf::from("/"))
}

/// The path a typed text names: `~` is the home folder; anything that does
/// not start at the root is refused.
fn typed_path(text: &str) -> Result<PathBuf, String> {
    let text = text.trim();
    if text.is_empty() {
        return Err("Type a path.".into());
    }
    let path = match text.strip_prefix("~/") {
        Some(rest) => home().join(rest),
        None if text == "~" => home(),
        None => PathBuf::from(text),
    };
    if !path.is_absolute() {
        return Err("Type the whole path, starting with / or ~/.".into());
    }
    Ok(path)
}

/// Whether a typed path can answer a request of this mode. `Ok(false)`:
/// the file exists and saving would replace it.
fn check(mode: FileMode, path: &Path) -> Result<bool, String> {
    match mode {
        FileMode::Open if path.is_file() => Ok(true),
        FileMode::Open if path.is_dir() => Err("This path is a folder, not a file.".into()),
        FileMode::Open => Err("No file is at this path.".into()),
        FileMode::Directory if path.is_dir() => Ok(true),
        FileMode::Directory => Err("No folder is at this path.".into()),
        FileMode::Save if path.is_dir() => Err("This path is a folder. Add a file name.".into()),
        FileMode::Save if !path.parent().is_some_and(Path::is_dir) => {
            Err("The folder of this path does not exist.".into())
        }
        FileMode::Save => Ok(!path.exists()),
    }
}

impl Services {
    fn start_directory(&self) -> PathBuf {
        self.directory
            .borrow()
            .clone()
            .filter(|directory| directory.is_dir())
            .unwrap_or_else(home)
    }

    fn remember(&self, path: &Path, mode: FileMode) {
        let directory = match mode {
            FileMode::Directory => Some(path),
            FileMode::Open | FileMode::Save => path.parent(),
        };
        if let Some(directory) = directory {
            *self.directory.borrow_mut() = Some(directory.to_owned());
        }
    }

    /// Whether the typed-path dialog is open.
    pub fn typed_path_open(&self) -> bool {
        self.typed.borrow().is_some()
    }

    /// Asks for the path as text, because the desktop's chooser did not
    /// answer. The caller makes the store draw again. Returns the request
    /// the dialog was still open for, if any: its owner is to hear that it
    /// was cancelled.
    #[must_use]
    pub fn ask_typed_path(&self, request: FileRequest, problem: String) -> Option<FileRequest> {
        let start = self.start_directory();
        let text = match (&request.mode, &request.suggested_name) {
            (FileMode::Save, Some(name)) => start.join(name).display().to_string(),
            _ => format!("{}/", start.display().to_string().trim_end_matches('/')),
        };
        self.typed
            .borrow_mut()
            .replace(TypedPath {
                request,
                text,
                problem,
                error: None,
                replace: None,
            })
            .map(|replaced| replaced.request)
    }

    fn cancel(&self, model: &mut Model) {
        if let Some(typed) = self.typed.borrow_mut().take() {
            (typed.request.reply)(model, None);
        }
    }

    fn confirm(&self, model: &mut Model) {
        let answered = {
            let mut slot = self.typed.borrow_mut();
            let Some(typed) = slot.as_mut() else {
                return;
            };
            let checked = typed_path(&typed.text)
                .and_then(|path| check(typed.request.mode, &path).map(|free| (path, free)));
            match checked {
                Err(error) => {
                    typed.error = Some(error);
                    typed.replace = None;
                    None
                }
                Ok((path, false)) if typed.replace.as_ref() != Some(&path) => {
                    typed.error =
                        Some("A file with this name exists. Confirm again to replace it.".into());
                    typed.replace = Some(path);
                    None
                }
                Ok((path, _)) => Some(path),
            }
        };
        let Some(path) = answered else {
            return;
        };
        if let Some(typed) = self.typed.borrow_mut().take() {
            self.remember(&path, typed.request.mode);
            (typed.request.reply)(model, Some(path));
        }
    }

    /// The typed-path dialog, when it is open: what the store shows in
    /// front of the model's surfaces.
    pub fn surfaces(&self) -> Vec<Surface> {
        let slot = self.typed.borrow();
        let Some(typed) = slot.as_ref() else {
            return Vec::new();
        };
        let (label, confirm) = match typed.request.mode {
            FileMode::Open => ("File", "Open"),
            FileMode::Save => ("Save as", "Save"),
            FileMode::Directory => ("Folder", "Choose"),
        };
        let expected: Vec<String> = typed
            .request
            .filters
            .iter()
            .map(|filter| {
                let extensions: Vec<String> = filter
                    .extensions
                    .iter()
                    .map(|extension| format!(".{extension}"))
                    .collect();
                format!("{} ({})", filter.name, extensions.join(", "))
            })
            .collect();
        let edit = {
            let services = self.clone();
            move |_: &mut Model, text: &str| {
                if let Some(typed) = services.typed.borrow_mut().as_mut() {
                    typed.text = text.to_owned();
                    typed.error = None;
                }
            }
        };
        let (on_submit, on_confirm, on_cancel, on_close) =
            (self.clone(), self.clone(), self.clone(), self.clone());
        let mut field = input(TYPED_PATH_INPUT, label, &typed.text, edit)
            .placeholder("/path/to/file")
            .error_opt(typed.error.clone())
            .on_submit(move |m| on_submit.confirm(m));
        if !expected.is_empty() {
            field = field.help(format!("Expected: {}.", expected.join("; ")));
        }
        let body: Vec<Block> = blocks![
            notice_block(
                NoticeKind::Info,
                format!(
                    "The desktop's file chooser did not answer ({}). Type the path instead.",
                    typed.problem.trim_end_matches('.')
                ),
            )
            .key(TYPED_PATH_REASON),
            field,
        ];
        vec![
            Surface::dialog(TYPED_PATH, typed.request.title.clone())
                .body(body)
                .footer([
                    button(TYPED_PATH_CANCEL, "Cancel", move |m| on_cancel.cancel(m)),
                    button(TYPED_PATH_CONFIRM, confirm, move |m| on_confirm.confirm(m)).primary(),
                ])
                .on_close(move |m| on_close.cancel(m)),
        ]
    }
}

/// The chooser's answer: a path, a cancellation, or why there was none.
type Chosen = Result<Option<PathBuf>, String>;

/// Waits for a chooser. The platform answers on a one-shot channel with
/// three layers: the channel, the portal, the user.
async fn answer<T, E: std::fmt::Display, C>(
    receiver: impl Future<Output = Result<Result<Option<T>, E>, C>>,
    path: impl FnOnce(T) -> Option<PathBuf>,
) -> Chosen {
    match receiver.await {
        Ok(Ok(chosen)) => Ok(chosen.and_then(path)),
        // No portal, or the portal failed.
        Ok(Err(error)) => Err(error.to_string()),
        // The platform dropped the request: as a cancellation.
        Err(_) => Ok(None),
    }
}

fn choose_file(services: &Services, request: FileRequest, cx: &mut Context<AppStore>) {
    let directory = services.start_directory();
    let services = services.clone();
    let mode = request.mode;
    // Both calls return at once; the portal answers later.
    let chosen: std::pin::Pin<Box<dyn Future<Output = Chosen>>> = match mode {
        FileMode::Save => Box::pin(answer(
            cx.prompt_for_new_path(&directory, request.suggested_name.as_deref()),
            Some,
        )),
        FileMode::Open | FileMode::Directory => Box::pin(answer(
            cx.prompt_for_paths(PathPromptOptions {
                files: mode == FileMode::Open,
                directories: mode == FileMode::Directory,
                multiple: false,
                prompt: None,
            }),
            |paths: Vec<PathBuf>| paths.into_iter().next(),
        )),
    };
    cx.spawn(async move |store, cx| {
        let chosen = chosen.await;
        let _ = store.update(cx, |store, cx| match chosen {
            Ok(path) => {
                if let Some(path) = &path {
                    services.remember(path, mode);
                }
                let reply = request.reply.clone();
                store.run(move |m| reply(m, path), cx);
            }
            Err(problem) => match services.ask_typed_path(request, problem) {
                Some(replaced) => store.run(move |m| (replaced.reply)(m, None), cx),
                None => store.touch(cx),
            },
        });
    })
    .detach();
}

/// Performs one intent. It runs inside the store's update: anything that
/// goes back to the model does so later, in an update of its own.
pub fn perform(services: &Services, hooks: &Hooks, intent: Intent, cx: &mut Context<AppStore>) {
    match intent {
        Intent::ChooseFile(request) => choose_file(services, request, cx),
        Intent::OpenUrl(url) => cx.open_url(&url),
        Intent::CopyText(text) => cx.write_to_clipboard(ClipboardItem::new_string(text)),
        Intent::ReadClipboard(reply) => {
            let text = cx.read_from_clipboard().and_then(|item| item.text());
            cx.spawn(async move |store, cx| {
                let _ = store.update(cx, |store, cx| store.run(move |m| reply(m, text), cx));
            })
            .detach();
        }
        Intent::ConsoleWindow(request) => {
            let open = Rc::clone(&hooks.console_window);
            cx.defer(move |cx| open(request, cx));
        }
        Intent::Quit => {
            let quit = Rc::clone(&hooks.quit);
            cx.defer(move |cx| quit(cx));
        }
    }
}

/// Makes the store perform the model's intents instead of recording them,
/// and show the typed-path dialog when it is asked for.
pub fn install(store: &Entity<AppStore>, hooks: Hooks, cx: &mut App) -> Services {
    let services = Services::default();
    store.update(cx, |store, cx| {
        let front = services.clone();
        store.set_front_surfaces(Some(Rc::new(move |_| front.surfaces())), cx);
        let services = services.clone();
        store.on_intent(move |intent, cx| perform(&services, &hooks, intent, cx));
    });
    services
}

#[cfg(test)]
mod tests {
    use std::cell::Cell;

    use super::*;

    fn directory(name: &str) -> PathBuf {
        let directory = std::env::temp_dir().join(format!(
            "xenadmin-gpui-{name}-{}-{:?}",
            std::process::id(),
            std::thread::current().id()
        ));
        let _ = std::fs::remove_dir_all(&directory);
        std::fs::create_dir_all(&directory).expect("temporary directory");
        directory
    }

    #[test]
    fn a_typed_path_starts_at_the_root_or_at_home() {
        assert_eq!(typed_path("  /tmp/x.csv "), Ok(PathBuf::from("/tmp/x.csv")));
        assert_eq!(typed_path(""), Err("Type a path.".into()));
        assert!(typed_path("x.csv").is_err());
        assert_eq!(typed_path("~/x.csv"), Ok(home().join("x.csv")));
        assert_eq!(typed_path("~"), Ok(home()));
    }

    #[test]
    fn a_typed_path_is_checked_against_the_mode_of_the_request() {
        let directory = directory("check");
        let file = directory.join("report.csv");
        std::fs::write(&file, "x").expect("file");
        assert_eq!(check(FileMode::Open, &file), Ok(true));
        assert!(check(FileMode::Open, &directory).is_err());
        assert!(check(FileMode::Open, &directory.join("missing")).is_err());
        assert_eq!(check(FileMode::Directory, &directory), Ok(true));
        assert!(check(FileMode::Directory, &file).is_err());
        // Saving over a file is asked about; into a missing folder refused.
        assert_eq!(check(FileMode::Save, &file), Ok(false));
        assert_eq!(check(FileMode::Save, &directory.join("new.csv")), Ok(true));
        assert!(check(FileMode::Save, &directory).is_err());
        assert!(check(FileMode::Save, &directory.join("no/new.csv")).is_err());
        let _ = std::fs::remove_dir_all(&directory);
    }

    #[test]
    fn the_typed_path_dialog_answers_the_request_once_and_asks_before_replacing() {
        let directory = directory("typed");
        let existing = directory.join("report.csv");
        std::fs::write(&existing, "x").expect("file");
        let services = Services::default();
        *services.directory.borrow_mut() = Some(directory.clone());
        let answers: Rc<RefCell<Vec<Option<PathBuf>>>> = Rc::default();
        let request = {
            let answers = answers.clone();
            FileRequest::save("Export report", "report.csv", move |_, path| {
                answers.borrow_mut().push(path);
            })
            .filter("CSV", ["csv"])
        };
        let mut model = Model::fixture();
        assert!(services.surfaces().is_empty());
        assert!(
            services
                .ask_typed_path(request, "no portal".into())
                .is_none()
        );
        let surfaces = services.surfaces();
        assert_eq!(surfaces.len(), 1);
        assert_eq!(surfaces[0].id, TYPED_PATH);
        assert_eq!(surfaces[0].title, "Export report");
        let keys: Vec<String> = surfaces[0]
            .nodes()
            .into_iter()
            .filter_map(|node| node.key)
            .collect();
        for key in [
            TYPED_PATH_REASON,
            TYPED_PATH_INPUT,
            TYPED_PATH_CANCEL,
            TYPED_PATH_CONFIRM,
        ] {
            assert!(keys.iter().any(|other| other == key), "{key} in {keys:?}");
        }
        // The suggested name in the folder used last: it exists, so the
        // first confirmation asks and the second replaces.
        services.confirm(&mut model);
        assert!(answers.borrow().is_empty());
        assert!(services.typed_path_open());
        services.confirm(&mut model);
        assert_eq!(*answers.borrow(), [Some(existing)]);
        assert!(!services.typed_path_open());
        // Closing answers with nothing, once.
        let cancelled = Rc::new(Cell::new(0));
        let request = {
            let cancelled = cancelled.clone();
            FileRequest::open("Import", move |_, path| {
                assert!(path.is_none());
                cancelled.set(cancelled.get() + 1);
            })
        };
        assert!(
            services
                .ask_typed_path(request, "no portal".into())
                .is_none()
        );
        services.cancel(&mut model);
        services.cancel(&mut model);
        assert_eq!(cancelled.get(), 1);
        let _ = std::fs::remove_dir_all(&directory);
    }
}
