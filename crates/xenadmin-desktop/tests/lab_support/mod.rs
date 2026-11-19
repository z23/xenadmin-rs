//! What the live scenarios of `lab_live.rs` share: the lab's manifest, the
//! application started as `app::start` starts it on the Kit's headless
//! platform, the steps a user can take, real-time waiting and the evidence.
//!
//! The application is the real one: `launch::prepare` claims the state
//! directory, `Model::live` starts the core, `app::start` builds the store,
//! the intents, the wake bridge, the console windows and the shell. What is
//! not real is the platform: the Kit's test platform (no compositor, no
//! seat, no portal, no clipboard of a desktop) with, for a PNG, the
//! offscreen renderer the scene tool uses.
//!
//! Time is real. The test platform's clock only moves when it is told to,
//! so every turn of a wait sleeps, moves that clock by what passed and lets
//! the foreground run: timers of the store and of the Kit fire as they
//! would, and a wake from a core thread is received in the next turn.
#![allow(dead_code)]

use std::path::{Path, PathBuf};
use std::sync::Arc;
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};
use xenadmin_core::{
    Backend, CertificateInspectionTask, CertificateObservation, Connection, ConnectionOptions,
    Credentials, Endpoint,
};
use xenadmin_desktop::app;
use xenadmin_desktop::assets::AppAssets;
use xenadmin_desktop::kit::gpui_kit;
use xenadmin_desktop::kit::testing::{
    ElementSnapshot, HeadlessAppContext, PlatformTextSystem, TestAppContext, TestQueryExt as _,
    TestWindowExt as _, current_headless_renderer, current_platform,
};
use xenadmin_desktop::kit::*;
use xenadmin_desktop::scenes;
use xenadmin_desktop::shell::{ShellView, nav as pane, toolbar};
use xenadmin_desktop::store::AppStore;
use xenadmin_ui::Model;
use xenadmin_ui::areas::console::Seat;
use xenadmin_ui::command::{Menu, MenuEntry};
use xenadmin_ui::launch::{self, Launch};
use xenadmin_ui::nav::{self, NavView};
use xenadmin_ui::object::ObjectRef;
use xenadmin_ui::prefs::Preferences;
use xenadmin_ui::surface::Node;
use xenadmin_ui::tabs::TabId;
use xenadmin_ui::testing::TempDir;

use crate::common;

/// The size of the main window: the size the application opens at.
pub const SIZE: (f32, f32) = app::DEFAULT_SIZE;

pub struct LabHost {
    pub name: String,
    pub ip: String,
}

/// The lab of the manifest. The password stays in this struct: it has no
/// `Debug`, and [`Evidence`] refuses to write anything that contains it.
pub struct Lab {
    pub id: String,
    pub hosts: Vec<LabHost>,
    password: String,
    pub evidence: PathBuf,
}

impl Lab {
    /// Reads the manifest and the password file the environment names, as
    /// the core's live harness does.
    pub fn load() -> Lab {
        let manifest = PathBuf::from(
            std::env::var("XENADMIN_LAB_MANIFEST")
                .expect("XENADMIN_LAB_MANIFEST must name the harness lab.json"),
        );
        let document: Value =
            serde_json::from_str(&std::fs::read_to_string(&manifest).expect("readable manifest"))
                .expect("manifest JSON");
        assert_eq!(document["schema"], 1, "unsupported manifest schema");
        let id = document["id"].as_str().expect("manifest id").to_string();
        let name = document["name"].as_str().expect("manifest name");
        assert_eq!(
            name,
            format!("xenadmin-lab-{}", &id[..8]),
            "manifest identity"
        );
        let hosts = document["hosts"]
            .as_array()
            .expect("hosts")
            .iter()
            .map(|host| LabHost {
                name: host["name"].as_str().expect("host name").to_string(),
                ip: host["ip"].as_str().expect("host ip").to_string(),
            })
            .collect::<Vec<_>>();
        assert!(hosts.len() >= 2, "the scenarios need two hosts");
        let password_file = std::env::var("XENADMIN_LAB_PASSWORD_FILE")
            .expect("XENADMIN_LAB_PASSWORD_FILE must name the lab-only root password file");
        let password = std::fs::read_to_string(password_file)
            .expect("readable password file")
            .lines()
            .next()
            .unwrap_or_default()
            .to_string();
        assert!(password.len() >= 8, "lab password missing");
        let evidence = std::env::var("XENADMIN_LAB_EVIDENCE_DIR")
            .map(PathBuf::from)
            .unwrap_or_else(|_| manifest.parent().unwrap().join("evidence"))
            .join("gpui");
        std::fs::create_dir_all(&evidence).expect("evidence directory");
        Lab {
            id,
            hosts,
            password,
            evidence,
        }
    }

    /// The address a user types for a host of the manifest.
    pub fn address(&self, index: usize) -> String {
        format!("https://{}", self.hosts[index].ip)
    }

    pub fn endpoint(&self, index: usize) -> Endpoint {
        Endpoint::parse(&self.address(index)).unwrap()
    }

    fn credentials(&self) -> Credentials {
        Credentials::new("root".into(), self.password.clone())
    }
}

/// The state directory of this run: `XENADMIN_STATE_DIR`, which must be a
/// temporary directory. The user's own state is never used.
pub fn state_directory() -> PathBuf {
    let directory = PathBuf::from(
        std::env::var_os("XENADMIN_STATE_DIR")
            .expect("XENADMIN_STATE_DIR must name a fresh temporary directory for this run"),
    );
    assert!(
        directory.starts_with(std::env::temp_dir()) || directory.starts_with("/tmp"),
        "XENADMIN_STATE_DIR must be under the temporary directory, not {}",
        directory.display()
    );
    directory
}

/// A scenario starts on a state directory nothing was saved in.
pub fn assert_fresh_state() {
    let directory = state_directory();
    let entries = std::fs::read_dir(&directory)
        .map(|entries| entries.count())
        .unwrap_or(0);
    assert_eq!(
        entries,
        0,
        "{} is not empty: give each run a fresh state directory",
        directory.display()
    );
}

// ---------------------------------------------------------------------------
// Evidence
// ---------------------------------------------------------------------------

/// The evidence of one scenario: a directory of numbered PNG frames and
/// element trees, and `summary.json`. Nothing that contains the lab's
/// password is written.
pub struct Evidence {
    pub directory: PathBuf,
    scenario: String,
    lab: String,
    secret: String,
    steps: Vec<Value>,
    notes: serde_json::Map<String, Value>,
    files: Vec<String>,
    started: Instant,
    stamp: u64,
    finished: bool,
}

impl Evidence {
    pub fn new(lab: &Lab, scenario: &str) -> Self {
        let stamp = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_secs();
        let directory = lab.evidence.join(format!("{stamp}-{scenario}"));
        std::fs::create_dir_all(&directory).expect("evidence directory");
        eprintln!("evidence: {}", directory.display());
        Self {
            directory,
            scenario: scenario.to_owned(),
            lab: lab.id.clone(),
            secret: lab.password.clone(),
            steps: Vec::new(),
            notes: serde_json::Map::new(),
            files: Vec::new(),
            started: Instant::now(),
            stamp,
            finished: false,
        }
    }

    fn checked(&self, text: &str) {
        assert!(
            !text.contains(&self.secret),
            "evidence would contain the lab password: not written"
        );
    }

    /// One line of the scenario's account: what was done or seen.
    pub fn step(&mut self, what: &str, detail: Value) {
        let seconds = self.started.elapsed().as_secs_f64();
        eprintln!("[{seconds:7.1}s] {what}: {detail}");
        self.steps.push(json!({
            "at_seconds": (seconds * 10.).round() / 10.,
            "step": what,
            "detail": detail,
        }));
    }

    /// A result the summary names by key.
    pub fn note(&mut self, key: &str, value: Value) {
        self.notes.insert(key.to_owned(), value);
    }

    fn write_text(&mut self, name: &str, text: &str) {
        self.checked(text);
        std::fs::write(self.directory.join(name), text).expect("evidence file");
        self.files.push(name.to_owned());
    }

    fn write_image(&mut self, name: &str, image: &image::RgbaImage) {
        image.save(self.directory.join(name)).expect("evidence PNG");
        self.files.push(name.to_owned());
    }

    /// Keeps a file the scenario made (a saved screenshot) with the
    /// evidence.
    pub fn keep(&mut self, name: &str, bytes: &[u8]) {
        std::fs::write(self.directory.join(name), bytes).expect("evidence file");
        self.files.push(name.to_owned());
    }

    fn summary(&self, outcome: &str) -> String {
        let document = json!({
            "scenario": self.scenario,
            "lab": self.lab,
            "application": env!("CARGO_PKG_VERSION"),
            "front_end": "xenadmin-desktop, headless (Kit test platform, offscreen renderer)",
            "recorded_unix": self.stamp,
            "outcome": outcome,
            "duration_seconds": (self.started.elapsed().as_secs_f64() * 10.).round() / 10.,
            "results": self.notes,
            "steps": self.steps,
            "files": self.files,
        });
        serde_json::to_string_pretty(&document).unwrap() + "\n"
    }

    /// Writes the summary of a scenario that passed.
    pub fn finish(mut self) {
        self.finished = true;
        let text = self.summary("passed");
        self.checked(&text);
        std::fs::write(self.directory.join("summary.json"), text).expect("summary");
        eprintln!(
            "evidence: {} ({} files, {:.1} s)",
            self.directory.join("summary.json").display(),
            self.files.len(),
            self.started.elapsed().as_secs_f64()
        );
    }
}

impl Drop for Evidence {
    fn drop(&mut self) {
        if self.finished {
            return;
        }
        // A scenario that panicked still leaves its account.
        let text = self.summary(if std::thread::panicking() {
            "failed"
        } else {
            "incomplete"
        });
        if !text.contains(&self.secret) {
            let _ = std::fs::write(self.directory.join("summary.json"), text);
        }
    }
}

// ---------------------------------------------------------------------------
// The platform
// ---------------------------------------------------------------------------

/// One platform text system per thread: building a platform starts worker
/// threads that are never joined.
fn text_system() -> Arc<dyn PlatformTextSystem> {
    thread_local! {
        static TEXT_SYSTEM: Arc<dyn PlatformTextSystem> = current_platform(true).text_system();
    }
    TEXT_SYSTEM.with(Arc::clone)
}

/// The two headless contexts of the Kit. [`Cx::Headless`] has the offscreen
/// renderer, so its frames can be captured, but nothing answers its path
/// prompts; [`Cx::Test`] has the test platform's prompts and no renderer.
enum Cx {
    Headless(HeadlessAppContext),
    Test(TestAppContext),
}

impl Cx {
    fn update<R>(&mut self, act: impl FnOnce(&mut App) -> R) -> R {
        match self {
            Cx::Headless(cx) => cx.update(act),
            Cx::Test(cx) => cx.update(act),
        }
    }

    fn window<R>(
        &mut self,
        handle: AnyWindowHandle,
        act: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> Option<R> {
        match self {
            Cx::Headless(cx) => cx.update_window(handle, |_, window, cx| act(window, cx)),
            Cx::Test(cx) => cx.update_window(handle, |_, window, cx| act(window, cx)),
        }
        .ok()
    }

    fn park(&mut self) {
        match self {
            Cx::Headless(cx) => cx.run_until_parked(),
            Cx::Test(cx) => cx.run_until_parked(),
        }
    }

    fn advance(&mut self, duration: Duration) {
        match self {
            Cx::Headless(cx) => cx.advance_clock(duration),
            Cx::Test(cx) => cx.executor().advance_clock(duration),
        }
    }

    fn capture(&mut self, handle: AnyWindowHandle) -> Option<image::RgbaImage> {
        match self {
            Cx::Headless(cx) => Some(cx.capture_screenshot(handle).expect("the frame")),
            Cx::Test(_) => None,
        }
    }
}

/// Which of the two contexts a client runs on.
#[derive(Clone, Copy, Debug, PartialEq)]
pub enum Platform {
    /// Frames are drawn by the offscreen renderer and captured as PNGs.
    Rendered,
    /// No renderer; the test platform's path prompts can be answered.
    Prompts,
}

// ---------------------------------------------------------------------------
// The application
// ---------------------------------------------------------------------------

/// The application under test. Fields drop in order: the entities before
/// the context that owns the application.
pub struct Client {
    pub view: Entity<ShellView>,
    pub store: Entity<AppStore>,
    pub window: AnyWindowHandle,
    clock: Instant,
    frame: usize,
    cx: Cx,
}

fn id(key: &str) -> ElementId {
    ElementId::Name(SharedString::from(key.to_owned()))
}

impl Client {
    /// Starts the application on the state directory of this run, as
    /// `app::main` does after the command line: the launch claims the
    /// directory, the model is the live one with the stored preferences.
    ///
    /// One difference: the lock on the state directory is not pinned to the
    /// process (`retain_lock_until_exit`), so that a scenario can close the
    /// application and start it again in the same test process.
    pub fn start(platform: Platform) -> Client {
        state_directory();
        let owner = match launch::prepare().expect("the launch") {
            Launch::Owner(mut owner) => {
                owner
                    .start_activation(Arc::new(|_| {}))
                    .expect("the activation service");
                owner
            }
            Launch::Forwarded => panic!("another client owns the state directory of this run"),
            Launch::Preview => panic!("the test arguments read as a preview"),
        };
        let model = Model::live(owner, Preferences::load());
        assert!(model.owns_state());
        assert!(
            model.backend.is_ok(),
            "the core did not start: {:?}",
            model.error
        );
        let mut cx = match platform {
            Platform::Rendered => Cx::Headless(HeadlessAppContext::with_platform(
                text_system(),
                Arc::new(AppAssets),
                current_headless_renderer,
            )),
            Platform::Prompts => Cx::Test(TestAppContext::build_with_text_system(
                gpui_kit::TestDispatcher::new(0),
                None,
                text_system(),
            )),
        };
        let (window, view) = cx
            .update(|cx| {
                gpui_kit::init(cx);
                // A capture must show the final state, not a dialog on its
                // way in.
                cx.set_reduce_motion(true);
                app::start(model, SIZE, None, cx)
            })
            .expect("the application starts");
        let store = cx.update(|cx| view.read(cx).ui().store.clone());
        let mut client = Client {
            view,
            store,
            window,
            clock: Instant::now(),
            frame: 0,
            cx,
        };
        // A desktop gives a new window the keyboard; the test platform
        // leaves every window inactive until it is told.
        client
            .cx
            .window(window, |window, _| window.activate_window());
        client.cx.park();
        client.settle();
        client
    }

    /// Closes the application: the context's shutdown runs the close path
    /// `app::start` installed (preferences saved, console windows closed,
    /// every session disconnected), then everything is dropped.
    pub fn close(self) {
        let Client {
            view, store, cx, ..
        } = self;
        drop(view);
        drop(store);
        match cx {
            Cx::Headless(cx) => drop(cx),
            Cx::Test(cx) => {
                cx.quit();
                drop(cx);
            }
        }
    }

    // -- Time ---------------------------------------------------------------

    /// One turn of real time: sleeps, moves the platform's clock by what
    /// passed and runs what became ready.
    pub fn turn(&mut self) {
        std::thread::sleep(Duration::from_millis(30));
        let now = Instant::now();
        self.cx.advance(now - self.clock);
        self.clock = now;
        self.cx.park();
    }

    fn draw(&mut self, handle: AnyWindowHandle) {
        self.cx.window(handle, |window, cx| {
            window.set_scale_factor(1.);
            window.render_frame(cx);
        });
        self.cx.park();
    }

    /// Draws the main window until what the last event caused is on screen.
    pub fn settle(&mut self) {
        for _ in 0..3 {
            self.draw(self.window);
        }
    }

    /// Lets real time pass, drawing as it goes.
    pub fn pass(&mut self, duration: Duration) {
        let start = Instant::now();
        while start.elapsed() < duration {
            self.turn();
            self.draw(self.window);
        }
    }

    /// Waits in real time until `done` holds. The time it took.
    #[track_caller]
    pub fn wait(
        &mut self,
        what: &str,
        limit: Duration,
        mut done: impl FnMut(&mut Client) -> bool,
    ) -> Duration {
        match self.wait_for(limit, &mut done) {
            Some(taken) => taken,
            None => panic!("after {limit:?}: {what}\n{}", self.outline()),
        }
    }

    /// [`Self::wait`] that says whether it held instead of panicking.
    pub fn wait_for(
        &mut self,
        limit: Duration,
        done: &mut dyn FnMut(&mut Client) -> bool,
    ) -> Option<Duration> {
        let start = Instant::now();
        loop {
            self.turn();
            self.draw(self.window);
            if done(self) {
                self.settle();
                return Some(start.elapsed());
            }
            if start.elapsed() > limit {
                return None;
            }
        }
    }

    // -- Reading ------------------------------------------------------------

    pub fn model<R>(&mut self, read: impl FnOnce(&Model) -> R) -> R {
        let store = self.store.clone();
        self.cx.update(|cx| read(store.read(cx).model()))
    }

    /// The id of the surface in front: a dialog of the model or the shell's
    /// typed-path dialog.
    pub fn front(&mut self) -> Option<String> {
        let store = self.store.clone();
        self.cx.update(|cx| {
            let surfaces = store.read(cx).surfaces();
            surfaces.last().map(|surface| surface.id.clone())
        })
    }

    /// Every node the window describes: the surfaces and the active tab.
    pub fn nodes(&mut self) -> Vec<Node> {
        let store = self.store.clone();
        self.cx.update(|cx| {
            scenes::described(store.read(cx))
                .into_values()
                .flatten()
                .collect()
        })
    }

    /// What is described under a key: of the surface in front when there is
    /// one, else of the tab.
    pub fn node(&mut self, key: &str) -> Option<Node> {
        let front = self.front();
        let store = self.store.clone();
        self.cx.update(|cx| {
            let described = scenes::described(store.read(cx));
            let wanted = |node: &&Node| node.key.as_deref() == Some(key);
            match front.and_then(|front| described.get(&front).cloned()) {
                Some(nodes) => nodes.iter().find(wanted).cloned(),
                None => described.values().flatten().find(wanted).cloned(),
            }
        })
    }

    /// Whether a node of what the window describes says the text, in its
    /// text, label or value.
    pub fn describes(&mut self, text: &str) -> bool {
        self.nodes().iter().any(|node| {
            [&node.text, &node.label, &node.value]
                .into_iter()
                .any(|part| part.as_deref().is_some_and(|part| part.contains(text)))
        })
    }

    /// Whether an element of the main window says the text, in its name or
    /// its value: what the frame shows, not only what is described.
    pub fn says(&mut self, text: &str) -> bool {
        self.on(|window, _| common::with_text(window, None, text).is_ok())
    }

    /// The element with a key (see `common::locate`), without scrolling.
    pub fn element(&mut self, key: &str) -> Option<ElementSnapshot> {
        let scope = self.front();
        self.on(|window, _| common::locate(window, scope.as_deref(), key).ok())
    }

    /// The element with a key anywhere in the window, the surface in front
    /// or not: the status bar, the toolbar, the tree.
    pub fn chrome(&mut self, key: &str) -> Option<ElementSnapshot> {
        self.on(|window, _| window.try_find(id(key)))
    }

    /// One line per element of the scope in front that has a name: for the
    /// message of a failure.
    pub fn outline(&mut self) -> String {
        let scope = self.front();
        self.on(|window, _| {
            let mut lines = vec![format!("front: {scope:?}")];
            for element in window.elements() {
                if scope
                    .as_deref()
                    .is_some_and(|scope| !common::under(&element, scope))
                {
                    continue;
                }
                let (Some(key), Some(label)) = (common::key_of(&element), element.label()) else {
                    continue;
                };
                lines.push(format!(
                    "  {key}: {label:?}{}",
                    if element.disabled() == Some(true) {
                        format!(" (disabled: {:?})", element.description().unwrap_or(""))
                    } else {
                        String::new()
                    }
                ));
            }
            lines.truncate(30);
            lines.join("\n")
        })
    }

    // -- Acting -------------------------------------------------------------

    /// Runs on the main window and draws what it caused.
    pub fn on<R>(&mut self, act: impl FnOnce(&mut Window, &mut App) -> R) -> R {
        self.on_window(self.window, act)
    }

    /// Runs on a window of the application (a console window) and draws
    /// what it caused there and in the main window.
    pub fn on_window<R>(
        &mut self,
        handle: AnyWindowHandle,
        act: impl FnOnce(&mut Window, &mut App) -> R,
    ) -> R {
        let result = self.cx.window(handle, act).expect("the window is open");
        self.cx.park();
        for _ in 0..3 {
            self.draw(handle);
        }
        if handle != self.window {
            self.settle();
        }
        result
    }

    /// The windows of the application other than the main one: the console
    /// windows.
    pub fn other_windows(&mut self) -> Vec<AnyWindowHandle> {
        let main = self.window;
        self.cx
            .update(|cx| cx.windows())
            .into_iter()
            .filter(|handle| *handle != main)
            .collect()
    }

    fn press_in(&mut self, scope: Option<&str>, key: &str) -> Result<(), String> {
        self.on(|window, cx| {
            let element = common::reach(window, cx, scope, key)?;
            if element.disabled() == Some(true) {
                return Err(format!(
                    "`{key}` is disabled: {}",
                    element.description().unwrap_or("no reason is given")
                ));
            }
            common::click_at(window, cx, element.bounds().center(), MouseButton::Left);
            Ok(())
        })
    }

    /// A press of the primary button on the element with the key, in the
    /// surface in front (or the window when none is open). `Err` when it is
    /// not there, cannot be brought into view or is disabled (with its
    /// reason).
    pub fn try_press(&mut self, key: &str) -> Result<(), String> {
        let scope = self.front();
        self.press_in(scope.as_deref(), key)
    }

    #[track_caller]
    pub fn press(&mut self, key: &str) {
        if let Err(error) = self.try_press(key) {
            panic!("{error}\n{}", self.outline());
        }
    }

    /// A press with the secondary modifier held on the element with the
    /// key: the press that adds a row to the selection of a table.
    #[track_caller]
    pub fn press_adding(&mut self, key: &str) {
        let scope = self.front();
        let result = self.on(|window, cx| {
            common::reach(window, cx, scope.as_deref(), key)?;
            let modifiers = Modifiers::secondary_key(cx);
            window.click_with_modifiers(ElementId::Name(key.to_owned().into()), modifiers, cx);
            Ok::<(), String>(())
        });
        if let Err(error) = result {
            panic!("{error}\n{}", self.outline());
        }
    }

    /// A press on a field, then the text from the keyboard. What the field
    /// held is replaced.
    #[track_caller]
    pub fn type_into(&mut self, key: &str, text: &str) {
        self.press(key);
        self.on(|window, cx| {
            window.press("ctrl-a", cx);
            window.input(text, cx);
        });
    }

    /// A keystroke to whatever has the keyboard.
    pub fn key(&mut self, key: &str) {
        self.on(|window, cx| window.press(key, cx));
    }

    /// Opens a drop-down of the window with the pointer and picks from its
    /// list with the keyboard: Down as many times as asked, then Return.
    pub fn pick_with_keys(&mut self, key: &str, downs: usize) {
        let select = id(key);
        self.on(move |window, cx| window.within(select).click("input", cx));
        self.on(|window, cx| {
            for _ in 0..downs {
                window.press("down", cx);
            }
            window.press("enter", cx);
        });
    }

    /// The reason the element with the key is disabled, `None` when it can
    /// be used.
    #[track_caller]
    pub fn disabled_reason(&mut self, scope: Option<&str>, key: &str) -> Option<String> {
        let element = self
            .on(|window, _| common::locate(window, scope, key))
            .unwrap_or_else(|error| panic!("{error}"));
        (element.disabled() == Some(true))
            .then(|| element.description().unwrap_or_default().to_owned())
    }

    /// A press on a button of the toolbar, by its command id.
    pub fn try_toolbar(&mut self, command: &str) -> Result<(), String> {
        self.press_in(Some(toolbar::TOOLBAR_KEY), command)
    }

    /// A press on a button of the local action row of the tab.
    pub fn try_local(&mut self, command: &str) -> Result<(), String> {
        self.press_in(None, &format!("local:{command}"))
    }

    /// Shows a tab by a press on its place in the tab strip.
    #[track_caller]
    pub fn tab(&mut self, tab: TabId) {
        if let Err(error) = self.press_in(Some(xenadmin_desktop::scene_view::TABS_KEY), tab.key()) {
            panic!("{error}");
        }
        assert_eq!(self.model(Model::active_tab), tab);
    }

    /// Shows a navigation view by a press in the switcher.
    #[track_caller]
    pub fn nav_view(&mut self, view: NavView) {
        if let Err(error) = self.press_in(Some(pane::SWITCHER_KEY), view.key()) {
            panic!("{error}");
        }
    }

    /// The row of the tree that selects the object.
    pub fn tree_row(&mut self, object: &ObjectRef) -> Option<nav::NavRow> {
        self.model(|m| {
            nav::rows(m)
                .into_iter()
                .find(|row| row.object.as_ref() == Some(object))
        })
    }

    /// The labels of the rows the tree has now.
    pub fn tree_labels(&mut self) -> Vec<String> {
        self.model(|m| nav::rows(m).into_iter().map(|row| row.label).collect())
    }

    /// Whether the tree draws a row with this name.
    pub fn tree_shows(&mut self, label: &str) -> bool {
        self.on(|window, _| {
            window
                .within(pane::TREE_KEY)
                .try_find_by_label(label)
                .is_some_and(|row| row.visible())
        })
    }

    /// Selects an object by a press on its row of the tree.
    #[track_caller]
    pub fn select(&mut self, object: &ObjectRef) {
        let row = self
            .tree_row(object)
            .unwrap_or_else(|| panic!("the tree has no row for {object:?}"));
        if let Err(error) = self.press_in(Some(pane::TREE_KEY), &row.id) {
            panic!("{error}");
        }
        assert_eq!(
            self.model(|m| m.scope().single().cloned()).as_ref(),
            Some(object),
            "the press on the row selected it"
        );
    }

    /// Where a command is in a popup built from groups of entries: its
    /// place at the top level, and its place in the submenu when it is in
    /// one. The disabled reason of the item comes with it.
    fn place(
        groups: &[Vec<MenuEntry>],
        command: &str,
    ) -> Option<(usize, Option<usize>, Option<String>)> {
        let mut index = 0;
        for (group, entries) in groups.iter().enumerate() {
            if group > 0 {
                index += 1;
            }
            for entry in entries {
                match entry {
                    MenuEntry::Item(item) if item.id.0 == command => {
                        return Some((index, None, item.disabled.clone()));
                    }
                    MenuEntry::Submenu { items, .. } => {
                        if let Some(inner) = items.iter().position(|item| item.id.0 == command) {
                            return Some((index, Some(inner), items[inner].disabled.clone()));
                        }
                    }
                    MenuEntry::Item(_) => {}
                }
                index += 1;
            }
        }
        None
    }

    /// Chooses the command from the popup that is open, built from `groups`.
    fn choose(&mut self, groups: &[Vec<MenuEntry>], command: &str) -> Result<(), String> {
        let Some((index, inner, disabled)) = Self::place(groups, command) else {
            self.key("escape");
            return Err(format!("the menu has no `{command}`"));
        };
        if let Some(reason) = disabled {
            self.key("escape");
            self.wait("the menu closed", Duration::from_secs(3), |client| {
                client.chrome("popup-menu").is_none()
            });
            return Err(reason);
        }
        if self.chrome("popup-menu").is_none() {
            return Err("no menu opened".into());
        }
        self.on(|window, cx| window.within("popup-menu").click(index, cx));
        if let Some(inner) = inner {
            self.wait("the submenu opened", Duration::from_secs(3), |client| {
                client.chrome("submenu").is_some()
            });
            self.on(|window, cx| window.within("submenu").click(inner, cx));
        }
        self.wait("the menu closed", Duration::from_secs(3), |client| {
            client.chrome("popup-menu").is_none()
        });
        Ok(())
    }

    /// Chooses a command from a menu of the menu bar with the pointer.
    /// `Err` with the reason when the item is disabled, or when the menu
    /// does not have it.
    pub fn try_menu(&mut self, menu: Menu, command: &str) -> Result<(), String> {
        let position = Menu::ALL
            .iter()
            .position(|other| *other == menu)
            .expect("menu");
        let store = self.store.clone();
        let groups = self.cx.update(|cx| {
            store
                .read(cx)
                .menu_bar()
                .into_iter()
                .find(|view| view.menu == menu)
                .map(|view| view.groups)
                .unwrap_or_default()
        });
        self.on(|window, cx| {
            window
                .within("app-menu-bar")
                .within(position)
                .click("menu", cx);
        });
        self.choose(&groups, command)
    }

    /// The label a menu of the menu bar gives a command now.
    pub fn menu_label(&mut self, menu: Menu, command: &str) -> Option<String> {
        let store = self.store.clone();
        self.cx.update(|cx| {
            store
                .read(cx)
                .menu_bar()
                .into_iter()
                .find(|view| view.menu == menu)?
                .items()
                .find(|item| item.id.0 == command)
                .map(|item| item.label.clone())
        })
    }

    #[track_caller]
    pub fn menu(&mut self, menu: Menu, command: &str) {
        if let Err(error) = self.try_menu(menu, command) {
            panic!("{menu:?} > {command}: {error}");
        }
    }

    /// Opens the context menu of an object's row of the tree with the
    /// secondary button and chooses a command from it.
    pub fn try_context(&mut self, object: &ObjectRef, command: &str) -> Result<(), String> {
        let row = self
            .tree_row(object)
            .ok_or_else(|| format!("the tree has no row for {object:?}"))?;
        self.on(|window, cx| window.right_click(id(&row.id), cx));
        let groups = self.model(Model::context_menu);
        self.choose(&groups, command)
    }

    // -- Evidence -----------------------------------------------------------

    /// Captures the main window: `NN-name.png` (on the rendered platform)
    /// and `NN-name.tree.json`.
    pub fn snap(&mut self, evidence: &mut Evidence, name: &str) {
        self.settle();
        let store = self.store.clone();
        let tree = self
            .cx
            .window(self.window, |window, cx| {
                scenes::tree_json(window, cx, &store)
            })
            .expect("the main window");
        self.record(evidence, self.window, name, tree);
    }

    /// Captures a console window.
    pub fn snap_window(&mut self, evidence: &mut Evidence, handle: AnyWindowHandle, name: &str) {
        for _ in 0..3 {
            self.draw(handle);
        }
        let tree = self
            .cx
            .window(handle, |window, _| {
                let elements: Vec<Value> = window
                    .elements()
                    .iter()
                    .map(|element| {
                        json!({
                            "path": scenes::id_path(element),
                            "label": element.label(),
                            "value": element.value(),
                            "disabled": element.disabled(),
                        })
                    })
                    .collect();
                json!({ "elements": elements })
            })
            .expect("the window");
        self.record(evidence, handle, name, tree);
    }

    fn record(
        &mut self,
        evidence: &mut Evidence,
        handle: AnyWindowHandle,
        name: &str,
        tree: Value,
    ) {
        self.frame += 1;
        let base = format!("{:02}-{name}", self.frame);
        let text = serde_json::to_string_pretty(&tree).expect("tree") + "\n";
        evidence.write_text(&format!("{base}.tree.json"), &text);
        let size = self
            .cx
            .window(handle, |window, _| window.viewport_size())
            .expect("the window");
        if let Some(full) = self.cx.capture(handle) {
            // The offscreen target has the size of the 2x test window; the
            // 1x frame is its top left quarter.
            let width = (f32::from(size.width) as u32).min(full.width());
            let height = (f32::from(size.height) as u32).min(full.height());
            let frame = image::imageops::crop_imm(&full, 0, 0, width, height).to_image();
            evidence.write_image(&format!("{base}.png"), &frame);
        }
        evidence.step("captured", json!(base));
    }

    /// The canvas of a console drawn in a window: how many pictures it
    /// uploaded, whether it has the keyboard, how many keys it forwarded.
    pub fn canvas(&mut self, handle: AnyWindowHandle, seat: Seat) -> Option<(u64, bool, usize)> {
        self.cx.update(|cx| {
            let canvas = xenadmin_desktop::console::canvas_of(handle.window_id(), seat, cx)?;
            let canvas = canvas.read(cx);
            Some((
                canvas.uploaded(),
                canvas.has_keyboard(),
                canvas.sent_keys().len(),
            ))
        })
    }

    /// What the plot of a chart painted in the last frame of the main
    /// window: the geometry the element computed from the chart's data.
    pub fn painted_chart(&mut self, key: &str) -> Option<xenadmin_desktop::charts::Painted> {
        let window = self.window.window_id();
        self.cx
            .update(|cx| xenadmin_desktop::charts::painted(window, key, cx))
    }

    /// What the main window draws inside the element with the key: the
    /// pixels of the captured frame within its bounds.
    pub fn pixels_of(&mut self, key: &str) -> Option<image::RgbaImage> {
        let bounds = self.chrome(key)?.bounds();
        let full = self.frame()?;
        let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        Some(
            image::imageops::crop_imm(
                &full,
                x.max(0.) as u32,
                y.max(0.) as u32,
                width as u32,
                height as u32,
            )
            .to_image(),
        )
    }

    /// Keys typed one by one to whatever has the keyboard: the characters
    /// of a US keyboard that need no Shift.
    pub fn type_keys(&mut self, text: &str) {
        for character in text.chars() {
            let key = match character {
                ' ' => "space".to_owned(),
                other => other.to_string(),
            };
            // As a person types: one key, a moment, the next; the window
            // is drawn when the text is done.
            self.cx
                .window(self.window, |window, cx| window.press(&key, cx));
            self.cx.park();
            self.turn();
        }
    }

    // -- Another window of the application ------------------------------------

    /// Puts text on the platform's clipboard, as copying it in another
    /// application does. On this platform the clipboard is the test
    /// platform's own, not a desktop's.
    pub fn copy_to_clipboard(&mut self, text: &str) {
        let item = ClipboardItem::new_string(text.to_owned());
        self.cx.update(|cx| cx.write_to_clipboard(item));
    }

    /// The element with a key in a window of the application.
    pub fn element_in(&mut self, handle: AnyWindowHandle, key: &str) -> Option<ElementSnapshot> {
        self.cx
            .window(handle, |window, _| window.try_find(id(key)))
            .flatten()
    }

    /// A press of the primary button on the element with the key in a
    /// window of the application (a console window).
    pub fn try_press_in(&mut self, handle: AnyWindowHandle, key: &str) -> Result<(), String> {
        let element = self
            .element_in(handle, key)
            .ok_or_else(|| format!("the window has no `{key}`"))?;
        if element.disabled() == Some(true) {
            return Err(format!(
                "`{key}` is disabled: {}",
                element.description().unwrap_or("no reason is given")
            ));
        }
        let at = element.bounds().center();
        // A press in a window of a desktop makes it the active one first;
        // the test platform activates a window only when it is told.
        self.activate(handle);
        self.on_window(handle, |window, cx| {
            common::click_at(window, cx, at, MouseButton::Left)
        });
        Ok(())
    }

    /// Makes a window of the application the active one, as a press in it
    /// or the window switcher of a desktop does.
    pub fn activate(&mut self, handle: AnyWindowHandle) {
        self.cx.window(handle, |window, _| window.activate_window());
        self.cx.park();
        for _ in 0..2 {
            self.draw(handle);
        }
    }

    /// A keystroke to whatever has the keyboard in a window.
    pub fn key_in(&mut self, handle: AnyWindowHandle, key: &str) {
        self.on_window(handle, |window, cx| window.press(key, cx));
    }

    /// [`Self::type_keys`] in a window of the application.
    pub fn type_keys_in(&mut self, handle: AnyWindowHandle, text: &str) {
        for character in text.chars() {
            let key = match character {
                ' ' => "space".to_owned(),
                other => other.to_string(),
            };
            self.cx.window(handle, |window, cx| window.press(&key, cx));
            self.cx.park();
            self.turn();
        }
    }

    /// Whether a window of the application is fullscreen, as the platform
    /// holds it.
    pub fn is_fullscreen(&mut self, handle: AnyWindowHandle) -> bool {
        self.cx
            .window(handle, |window, _| window.is_fullscreen())
            .unwrap_or(false)
    }

    /// What a window draws inside the element with the key: the pixels of
    /// its captured frame within the element's bounds.
    pub fn pixels_in(&mut self, handle: AnyWindowHandle, key: &str) -> Option<image::RgbaImage> {
        for _ in 0..3 {
            self.draw(handle);
        }
        let bounds = self.element_in(handle, key)?.bounds();
        let full = self.cx.capture(handle)?;
        let (x, y) = (f32::from(bounds.origin.x), f32::from(bounds.origin.y));
        let (width, height) = (f32::from(bounds.size.width), f32::from(bounds.size.height));
        Some(
            image::imageops::crop_imm(
                &full,
                x.max(0.) as u32,
                y.max(0.) as u32,
                width as u32,
                height as u32,
            )
            .to_image(),
        )
    }

    /// The last frame of the main window as pixels, for a comparison.
    pub fn frame(&mut self) -> Option<image::RgbaImage> {
        self.settle();
        self.cx.capture(self.window)
    }

    /// Whether the test platform was asked for a path to save to.
    pub fn asked_for_new_path(&mut self) -> bool {
        match &self.cx {
            Cx::Test(cx) => cx.did_prompt_for_new_path(),
            Cx::Headless(_) => false,
        }
    }

    /// Answers the test platform's path prompt.
    pub fn answer_new_path(&mut self, path: Option<PathBuf>) {
        match &self.cx {
            Cx::Test(cx) => cx.simulate_new_path_selection(move |_| path),
            Cx::Headless(_) => panic!("the rendered platform has no path prompts to answer"),
        }
        self.cx.park();
        self.settle();
    }
}

// ---------------------------------------------------------------------------
// The lab from outside this client
// ---------------------------------------------------------------------------

/// Credential-free TLS inspection through the core, as the core's live
/// harness performs it.
pub fn inspect(endpoint: &Endpoint) -> CertificateObservation {
    let task = CertificateInspectionTask::start(
        ConnectionOptions::new(endpoint.clone()),
        endpoint.clone(),
        None,
    );
    let start = Instant::now();
    loop {
        if let Some(outcome) = task.take_result() {
            return outcome.expect("certificate observed");
        }
        assert!(
            start.elapsed() < Duration::from_secs(30),
            "certificate inspection did not finish"
        );
        std::thread::sleep(Duration::from_millis(50));
    }
}

/// A second client of the lab: a core backend with a state directory of its
/// own and a connection through the coordinator, with every member's
/// certificate pinned after inspection. What a scenario changes "from
/// outside" and reads back independently goes through it.
pub struct Outside {
    pub connection: Connection,
    _backend: Backend,
    _state: TempDir,
}

impl Outside {
    pub fn connect(lab: &Lab) -> Outside {
        let state = TempDir::new("gpui-lab-outside");
        let backend = Backend::with_state_directory(state.path().into()).expect("backend");
        let endpoint = lab.endpoint(0);
        let mut options = ConnectionOptions::new(endpoint.clone());
        options.certificate_fingerprint = Some(inspect(&endpoint).details.fingerprint.clone());
        let other = lab.endpoint(1);
        options.add_host_certificate(other.clone(), inspect(&other).details.fingerprint.clone());
        let connection = backend.connect(options, lab.credentials());
        let start = Instant::now();
        while !connection.latest().is_current() {
            assert!(
                start.elapsed() < Duration::from_secs(90),
                "the outside connection did not become ready: {:?}",
                connection.latest().error()
            );
            std::thread::sleep(Duration::from_millis(100));
        }
        Outside {
            connection,
            _backend: backend,
            _state: state,
        }
    }

    /// Waits until the outside connection's inventory satisfies `ready`.
    pub fn settle(
        &self,
        limit: Duration,
        ready: impl Fn(&xenadmin_core::Inventory) -> bool,
    ) -> bool {
        let start = Instant::now();
        loop {
            if self
                .connection
                .latest()
                .inventory()
                .is_some_and(|inventory| ready(inventory))
            {
                return true;
            }
            if start.elapsed() > limit {
                return false;
            }
            std::thread::sleep(Duration::from_millis(200));
        }
    }
}

/// Types the lab's password into the field with the key. The text goes from
/// the lab's struct to the field and nowhere else.
#[track_caller]
pub fn type_password(client: &mut Client, lab: &Lab, key: &str) {
    client.type_into(key, &lab.password);
}

/// Whether a file of a path holds the password: for a check of the state
/// directory.
pub fn holds_password(lab: &Lab, path: &Path) -> bool {
    std::fs::read(path).is_ok_and(|bytes| {
        let secret = lab.password.as_bytes();
        bytes.windows(secret.len()).any(|window| window == secret)
    })
}
