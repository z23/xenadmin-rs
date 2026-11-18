//! The application: the command line, the start, the main window and the
//! one path by which it closes.
//!
//! ```text
//! xenadmin-gpui                      the client
//! xenadmin-gpui --version | -V       prints the version and does nothing else
//! xenadmin-gpui --demo               a window on the synthetic pool
//! xenadmin-gpui --scene NAME         a window on a scene
//!     [--theme dark|light] [--size WxH]
//! xenadmin-gpui --scene NAME --snap FILE.png [--tree FILE.json]
//!     [--theme dark|light] [--size WxH] [--scale 1|2]      no window at all
//! ```
//!
//! Rules carried over from the Rune `main` (`rune-kernel.md`, 16 and 17):
//!
//! - `--version` is answered before anything is claimed or opened.
//! - A preview (`--demo`, `--scene`, `--snap`, `--tree`, `--help`) takes no
//!   lock on the state directory, starts no backend, restores no session,
//!   opens no inspection socket and writes nothing: its model is
//!   `Model::fixture()`.
//! - Otherwise the process must own the state directory. A second start
//!   asks the first to show itself and exits.
//! - Closing the main window saves the preferences, lets go of console
//!   input, closes the console windows and disconnects every session, in
//!   one path that runs before the application quits.

use std::cell::Cell;
use std::path::PathBuf;
use std::process::ExitCode;
use std::rc::Rc;
use std::sync::Arc;

use xenadmin_ui::launch::{self, Launch};
use xenadmin_ui::model::APPLICATION_NAME;
use xenadmin_ui::prefs::{Appearance, Preferences};
use xenadmin_ui::{Model, scene};

use crate::assets::AppAssets;
use crate::console::ConsoleWindows;
use crate::intents::{self, Hook, Hooks};
use crate::kit::*;
use crate::shell::{self, ShellView};
use crate::store::AppStore;

/// The application id of the main window. It must be the name of the
/// installed desktop entry (`packaging/xenadmin-rs.desktop`), or the
/// desktop shows the window without its icon and cannot activate it.
pub const APP_ID: &str = "xenadmin-rs";

/// The smallest size the main window can be given.
pub const MINIMUM_SIZE: (f32, f32) = (800., 600.);

/// The size the main window opens at.
pub const DEFAULT_SIZE: (f32, f32) = (1440., 900.);

pub const USAGE: &str = "usage: xenadmin-gpui [--version] [--demo | --scene NAME] [--theme dark|light] [--size WxH]
       xenadmin-gpui (--demo | --scene NAME) [--snap FILE.png] [--tree FILE.json] [--theme dark|light] [--size WxH] [--scale 1|2]

  --version, -V    print the version and exit
  --demo           show the synthetic pool; nothing is connected or written
  --scene NAME     show a scene; nothing is connected or written
  --theme MODE     dark or light, for --demo and --scene
  --size WxH       the size of the window in logical pixels
  --snap FILE      draw the scene to a PNG instead of opening a window
  --tree FILE      write the element tree of the scene as JSON instead of opening a window
  --scale N        device pixels per logical pixel of --snap: 1 or 2";

/// What the command line asks for.
#[derive(Clone, Debug, Default, PartialEq)]
pub struct CommandLine {
    pub help: bool,
    pub demo: bool,
    pub scene: Option<String>,
    /// `Some(true)`: dark.
    pub dark: Option<bool>,
    pub size: Option<(f32, f32)>,
    pub scale: Option<f32>,
    pub snap: Option<PathBuf>,
    pub tree: Option<PathBuf>,
}

impl CommandLine {
    /// Whether the scene goes to files and no window opens.
    pub fn headless(&self) -> bool {
        self.snap.is_some() || self.tree.is_some()
    }

    /// The scene a preview shows: the one named, or the demo.
    pub fn preview_scene(&self) -> Option<&str> {
        self.scene.as_deref().or(self.demo.then_some("demo"))
    }
}

/// The line `--version` prints. The name is the released client's, which
/// this binary becomes at cutover.
pub fn version_line() -> String {
    format!("xenadmin-rs {}", env!("CARGO_PKG_VERSION"))
}

/// Parses the arguments after the program name. Both `--flag VALUE` and
/// `--flag=VALUE` are read.
pub fn parse(arguments: &[String]) -> Result<CommandLine, String> {
    let mut line = CommandLine::default();
    let mut arguments = arguments.iter();
    while let Some(argument) = arguments.next() {
        let (flag, attached) = match argument.split_once('=') {
            Some((flag, value)) if flag.starts_with("--") => (flag, Some(value.to_owned())),
            _ => (argument.as_str(), None),
        };
        let mut value = || {
            attached
                .clone()
                .or_else(|| arguments.next().cloned())
                .ok_or_else(|| format!("{flag} needs a value"))
        };
        match flag {
            "--help" | "-h" => line.help = true,
            "--demo" => line.demo = true,
            "--scene" => line.scene = Some(value()?),
            "--theme" => {
                line.dark = Some(match value()?.as_str() {
                    "dark" => true,
                    "light" => false,
                    other => return Err(format!("--theme takes dark or light, not `{other}`")),
                });
            }
            "--size" => {
                let size = value()?;
                let parsed = size
                    .split_once('x')
                    .and_then(|(width, height)| Some((width.parse().ok()?, height.parse().ok()?)));
                match parsed {
                    Some((width, height)) if width >= 200. && height >= 200. => {
                        line.size = Some((width, height));
                    }
                    _ => return Err(format!("--size takes WIDTHxHEIGHT, not `{size}`")),
                }
            }
            "--scale" => {
                line.scale = Some(match value()?.as_str() {
                    "1" => 1.,
                    "2" => 2.,
                    other => return Err(format!("--scale takes 1 or 2, not `{other}`")),
                });
            }
            "--snap" => line.snap = Some(PathBuf::from(value()?)),
            "--tree" => line.tree = Some(PathBuf::from(value()?)),
            other => return Err(format!("unknown argument `{other}`")),
        }
    }
    if line.demo && line.scene.is_some() {
        return Err("--demo and --scene are two previews: give one".into());
    }
    let preview = line.preview_scene().is_some();
    if line.headless() && !preview {
        return Err("--snap and --tree draw a scene: give --scene NAME or --demo".into());
    }
    if line.scale.is_some() && !line.headless() {
        return Err("--scale applies to --snap and --tree only".into());
    }
    if (line.dark.is_some() || line.size.is_some()) && !preview {
        return Err(
            "--theme and --size apply to --demo and --scene: the client keeps its own in its preferences"
                .into(),
        );
    }
    Ok(line)
}

/// Draws a scene to the files the command line names. No window, no
/// backend, no state directory.
#[cfg(feature = "scenes")]
fn render_to_files(line: &CommandLine) -> Result<(), String> {
    use crate::scenes::{self, Options};
    let name = line.preview_scene().ok_or("no scene was named")?;
    let mut options = Options::default();
    if let Some((width, height)) = line.size {
        (options.width, options.height) = (width, height);
    }
    if let Some(dark) = line.dark {
        options.mode = if dark {
            ThemeMode::Dark
        } else {
            ThemeMode::Light
        };
    }
    if let Some(scale) = line.scale {
        options.scale = scale;
    }
    scenes::write(name, &options, line.snap.as_deref(), line.tree.as_deref())
}

#[cfg(not(feature = "scenes"))]
fn render_to_files(_: &CommandLine) -> Result<(), String> {
    Err(
        "this build cannot draw a scene to a file: it was made without the `scenes` feature. Use the xenadmin-scenes tool."
            .into(),
    )
}

/// The model of a preview: the fixture with the scene applied.
fn preview_model(line: &CommandLine) -> Result<Model, String> {
    let mut model = Model::fixture();
    if let Some(name) = line.preview_scene()
        && !scene::apply(name, &mut model)
    {
        let names: Vec<&str> = scene::registry().iter().map(|spec| spec.name).collect();
        return Err(format!(
            "no scene is named `{name}`. The scenes are: {}",
            names.join(", ")
        ));
    }
    if let Some(dark) = line.dark {
        // A preview owns no state directory: this is never written.
        model.preferences.options.appearance = if dark {
            Appearance::Dark
        } else {
            Appearance::Light
        };
    }
    Ok(model)
}

/// The options of the main window.
pub fn window_options(title: &str, (width, height): (f32, f32), cx: &App) -> WindowOptions {
    WindowOptions {
        window_bounds: Some(WindowBounds::Windowed(Bounds::centered(
            None,
            size(px(width), px(height)),
            cx,
        ))),
        titlebar: Some(TitlebarOptions {
            title: Some(SharedString::from(shell::window_title(title))),
            ..TitleBar::title_bar_options()
        }),
        app_id: Some(APP_ID.to_owned()),
        window_min_size: Some(size(px(MINIMUM_SIZE.0), px(MINIMUM_SIZE.1))),
        // The compositor's own frame where it has one. Where it gives
        // client-side decorations instead, the Kit's title bar draws the
        // window controls.
        window_decorations: Some(WindowDecorations::Server),
        focus: true,
        show: true,
        is_resizable: true,
        ..TitleBar::window_options()
    }
}

/// What closing the main window does before the application quits: what
/// the Rune `App::on_close` did. It runs once, whichever way the close was
/// asked for; `closed` remembers. Returns whether this call did it.
fn shut_down(store: &Entity<AppStore>, console: &Hooks, closed: &Cell<bool>, cx: &mut App) -> bool {
    if closed.replace(true) {
        return false;
    }
    (console.release_console_input)(cx);
    (console.close_console_windows)(cx);
    // Saves the preferences when the process owns the state directory, and
    // disconnects every session.
    store.update(cx, |store, cx| store.run(Model::close, cx));
    true
}

/// The hooks of an application over `store`, and what a quit that did not
/// come through them still has to do.
///
/// `Hooks::quit` is the close path: the shutdown above, then `cx.quit()`.
/// Every way of closing leads to it: the window manager
/// (`on_window_should_close`), the close button of a client-side decoration,
/// File > Exit (`Intent::Quit`), and the window going away under the
/// application. The shutdown runs before the quit because a quit observer
/// is given 200 ms in all, and `Window::remove_window` asks no one.
pub fn hooks(store: &Entity<AppStore>) -> (Hooks, Hook) {
    hooks_over(store, Hooks::default())
}

/// [`hooks`] over the console hooks given: `console` provides what opens
/// and closes the console windows and releases a console's input.
pub fn hooks_over(store: &Entity<AppStore>, console: Hooks) -> (Hooks, Hook) {
    let closed = Rc::new(Cell::new(false));
    let quit = {
        let (store, console, closed) = (store.clone(), console.clone(), closed.clone());
        Rc::new(move |cx: &mut App| {
            shut_down(&store, &console, &closed, cx);
            cx.quit();
        })
    };
    let last_resort = {
        let (store, console) = (store.clone(), console.clone());
        Rc::new(move |cx: &mut App| {
            shut_down(&store, &console, &closed, cx);
        })
    };
    (Hooks { quit, ..console }, last_resort)
}

/// Builds the application inside a running GPUI application: the store
/// over `model`, the intents, the wake bridge and the main window.
/// `activations` delivers the requests of later starts to show the window.
pub fn start(
    model: Model,
    window_size: (f32, f32),
    activations: Option<Receiver<Option<String>>>,
    cx: &mut App,
) -> Result<(AnyWindowHandle, Entity<ShellView>), String> {
    gpui_kit::init(cx);
    crate::init(cx);
    let title = model.window_title();
    let store = cx.new(|_| AppStore::new(model));
    let consoles = ConsoleWindows::new(&store);
    let (hooks, last_resort) = hooks_over(&store, consoles.hooks(Hooks::default()));
    intents::install(&store, hooks.clone(), cx);
    // The waker goes to the model, which restores the sessions that ask
    // for it (never in a preview). Console frames have a waker of their
    // own and do not come through here.
    store.update(cx, |store, cx| store.connect_core(cx));
    let options = window_options(&title, window_size, cx);
    let (window, shell) = gpui_kit::open_window(options, cx, {
        let (store, hooks) = (store.clone(), hooks.clone());
        move |window, cx| {
            cx.new(|cx| {
                let mut shell = ShellView::new(store, hooks, window, cx);
                shell.follow_system_appearance(window, cx);
                shell
            })
        }
    })
    .map_err(|error| format!("The main window did not open: {error}"))?;
    consoles.install(Some(window), cx);
    // The window went away without being asked to close.
    let quit = Rc::clone(&hooks.quit);
    let main = window.window_id();
    cx.on_window_closed(move |cx, id| {
        if id == main {
            quit(cx);
        }
    })
    .detach();
    // A quit that did not come through the close path still saves and
    // disconnects, as far as the time a quit observer is given allows.
    cx.on_app_quit(move |cx| {
        last_resort(cx);
        async {}
    })
    .detach();
    if let Some(activations) = activations {
        cx.spawn(async move |cx| {
            // The token of the request cannot be handed to the toolkit: the
            // window asks the compositor for activation itself.
            while activations.recv().await.is_ok() {
                cx.update(|cx| {
                    cx.activate(true);
                    let _ = cx.update_window(window, |_, window, _| window.activate_window());
                });
            }
        })
        .detach();
    }
    Ok((window, shell))
}

/// The whole program. Returns the exit status.
pub fn main() -> ExitCode {
    let arguments: Vec<String> = std::env::args().skip(1).collect();
    // Before anything is claimed or opened.
    if launch::version_requested(&arguments) {
        println!("{}", version_line());
        return ExitCode::SUCCESS;
    }
    let line = match parse(&arguments) {
        Ok(line) => line,
        Err(error) => {
            eprintln!("xenadmin-gpui: {error}\n{USAGE}");
            return ExitCode::from(2);
        }
    };
    if line.help {
        println!("{USAGE}");
        return ExitCode::SUCCESS;
    }
    if line.headless() {
        return match render_to_files(&line) {
            Ok(()) => ExitCode::SUCCESS,
            Err(error) => {
                eprintln!("xenadmin-gpui: {error}");
                ExitCode::FAILURE
            }
        };
    }
    let (sender, activations) = unbounded::<Option<String>>();
    let (model, activations) = match launch::prepare() {
        Ok(Launch::Forwarded) => return ExitCode::SUCCESS,
        Ok(Launch::Preview) => match preview_model(&line) {
            Ok(model) => (model, None),
            Err(error) => {
                eprintln!("xenadmin-gpui: {error}");
                return ExitCode::from(2);
            }
        },
        Ok(Launch::Owner(owner)) => {
            // Runs on a core thread: it only queues the request.
            let activate = Arc::new(move |token: Option<String>| {
                let _ = sender.try_send(token);
            });
            match launch::attach(owner, activate) {
                Ok(owner) => (Model::live(owner, Preferences::load()), Some(activations)),
                Err(error) => {
                    eprintln!("{error}");
                    return ExitCode::FAILURE;
                }
            }
        }
        Err(error) => {
            eprintln!("{error}");
            return ExitCode::FAILURE;
        }
    };
    let window_size = line.size.unwrap_or(DEFAULT_SIZE);
    let failed = Rc::new(Cell::new(false));
    let report = failed.clone();
    gpui_kit::application()
        .with_assets(AppAssets)
        // The main window closes the application through its close path;
        // a console window closing must not.
        .with_quit_mode(QuitMode::Explicit)
        .run(move |cx| {
            if let Err(error) = start(model, window_size, activations, cx) {
                eprintln!("{APPLICATION_NAME}: {error}");
                report.set(true);
                cx.quit();
            }
        });
    if failed.get() {
        ExitCode::FAILURE
    } else {
        ExitCode::SUCCESS
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn arguments(line: &str) -> Vec<String> {
        line.split_whitespace().map(str::to_owned).collect()
    }

    #[test]
    fn the_flags_of_a_preview_are_read_in_both_spellings() {
        let line = parse(&arguments(
            "--scene demo --theme light --size 800x600 --scale 2 --snap out.png --tree=out.json",
        ))
        .expect("command line");
        assert_eq!(line.scene.as_deref(), Some("demo"));
        assert_eq!(line.dark, Some(false));
        assert_eq!(line.size, Some((800., 600.)));
        assert_eq!(line.scale, Some(2.));
        assert_eq!(line.snap, Some(PathBuf::from("out.png")));
        assert_eq!(line.tree, Some(PathBuf::from("out.json")));
        assert!(line.headless());
        let line = parse(&arguments("--demo")).expect("command line");
        assert!(line.demo && !line.headless());
        assert_eq!(line.preview_scene(), Some("demo"));
        assert_eq!(parse(&[]), Ok(CommandLine::default()));
    }

    #[test]
    fn a_flag_that_would_be_ignored_is_refused() {
        for line in [
            "--snap out.png",
            "--tree out.json",
            "--scale 2 --demo",
            "--theme dark",
            "--size 800x600",
            "--demo --scene demo",
            "--theme blue --demo",
            "--size 10x10 --demo",
            "--scale 3 --demo --snap out.png",
            "--scene",
            "--frobnicate",
            "demo",
        ] {
            assert!(parse(&arguments(line)).is_err(), "{line}");
        }
    }

    /// Every flag that draws to a file or shows a preview must also be one
    /// the model treats as a preview, or the start would claim the state
    /// directory and restore sessions for it.
    #[test]
    fn every_preview_of_the_command_line_is_a_preview_for_the_launch() {
        for line in [
            "--demo",
            "--scene demo",
            "--scene=demo",
            "--demo --snap out.png",
            "--scene demo --tree out.json",
            "--help",
            "-h",
        ] {
            let arguments = arguments(line);
            let parsed = parse(&arguments).expect(line);
            assert!(parsed.help || parsed.preview_scene().is_some(), "{line}");
            assert!(launch::preview(&arguments), "{line}");
        }
        assert!(!launch::preview(&[]));
    }

    #[test]
    fn the_version_is_answered_as_the_released_client_answers_it() {
        assert_eq!(
            version_line(),
            format!("xenadmin-rs {}", env!("CARGO_PKG_VERSION"))
        );
        assert!(launch::version_requested(&arguments("--demo --version")));
    }

    #[test]
    fn a_preview_model_is_a_fixture_with_the_scene_applied() {
        let line = parse(&arguments("--scene demo --theme light")).expect("command line");
        let model = preview_model(&line).expect("model");
        assert!(model.demo);
        assert!(!model.owns_state());
        assert!(model.backend.is_err());
        assert_eq!(model.preferences.options.appearance, Appearance::Light);
        let unknown = parse(&arguments("--scene no-such-scene")).expect("command line");
        let error = preview_model(&unknown).err().expect("refused");
        assert!(error.contains("no-such-scene") && error.contains("demo"));
    }
}
