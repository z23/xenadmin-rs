//! `AppStore`: the one entity that owns the [`Model`] and is its only
//! writer.
//!
//! Every view reads the model through [`AppStore::model`] in `render` and
//! changes it through [`AppStore::run`]. After every change the store drains
//! the model's intent outbox into the handler the application installed (a
//! test records them instead), schedules the timed tick the model asked for,
//! and notifies, which re-renders every observer.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::{Duration, Instant};

use xenadmin_ui::command::CommandId;
use xenadmin_ui::intent::Intent;
use xenadmin_ui::surface::Surface;
use xenadmin_ui::tabs::{TabBody, TabId};
use xenadmin_ui::{Model, command};

use crate::bridge::{WakeReceiver, wake_channel};
use crate::kit::*;

/// The open surfaces, back to front.
pub type DescribeSurfaces = Rc<dyn Fn(&Model) -> Vec<Surface>>;
/// The body of a tab.
pub type DescribeTab = Rc<dyn Fn(&Model, TabId) -> Option<TabBody>>;
/// The local action row of a tab.
pub type DescribeLocalRow = Rc<dyn Fn(&Model, TabId) -> Vec<command::MenuItem>>;
/// The menu bar: always the ten menus.
pub type DescribeMenuBar = Rc<dyn Fn(&Model) -> Vec<command::MenuView>>;
/// The toolbar, as groups left to right.
pub type DescribeToolbar = Rc<dyn Fn(&Model) -> Vec<Vec<command::MenuItem>>>;
/// Runs a command of the menu bar, the toolbar or a local action row on the
/// model's target.
pub type InvokeCommand = Rc<dyn Fn(&mut Model, CommandId)>;
/// The surfaces the front end shows by itself, in front of the model's:
/// the typed-path prompt that stands in for a file chooser.
pub type FrontSurfaces = Rc<dyn Fn(&Model) -> Vec<Surface>>;

/// Where the descriptions come from. The model answers by default; a test
/// or the scene tool's gallery substitutes its own, to render descriptions
/// no area provides.
#[derive(Clone)]
pub struct Describe {
    pub surfaces: DescribeSurfaces,
    pub tab_body: DescribeTab,
    /// The local action row of a tab.
    pub local_row: DescribeLocalRow,
    /// The menu bar.
    pub menu_bar: DescribeMenuBar,
    /// The toolbar.
    pub toolbar: DescribeToolbar,
    /// What runs a command chosen from the menu bar, the toolbar, a local
    /// action row or by its shortcut.
    pub invoke: InvokeCommand,
}

impl Default for Describe {
    fn default() -> Self {
        Self {
            surfaces: Rc::new(Model::surfaces),
            tab_body: Rc::new(Model::tab_body),
            local_row: Rc::new(command::local_row),
            menu_bar: Rc::new(command::menu_bar),
            toolbar: Rc::new(command::toolbar),
            invoke: Rc::new(|model, id| {
                // A refusal is written to the model's error line.
                let target = model.target();
                model.invoke(id, &target);
            }),
        }
    }
}

type IntentHandler = Box<dyn FnMut(Intent, &mut Context<AppStore>)>;

enum IntentSink {
    /// Tests and the scene tool: the intents are kept for inspection.
    Record(Vec<Intent>),
    /// The application: each intent is performed.
    Handler(IntentHandler),
}

pub struct AppStore {
    model: Model,
    describe: Describe,
    front: Option<FrontSurfaces>,
    intents: IntentSink,
    /// Counts the updates. Caches keyed by it are valid for one state.
    revision: u64,
    surfaces: RefCell<Option<Rc<Vec<Surface>>>>,
    started: Instant,
    /// The clock never reads less than this: a timed tick that fires on a
    /// virtual clock (tests) still polls at the time it was asked for.
    clock_floor: f64,
    /// The pending timed tick: when it is due and the task that waits.
    tick: Option<(f64, Task<()>)>,
    _bridge: Option<Task<()>>,
}

impl AppStore {
    /// A store over `model` that records intents. Call [`Self::on_intent`]
    /// to perform them and [`Self::connect_core`] to receive the core's
    /// wakes.
    pub fn new(model: Model) -> Self {
        Self {
            model,
            describe: Describe::default(),
            front: None,
            intents: IntentSink::Record(Vec::new()),
            revision: 0,
            surfaces: RefCell::new(None),
            started: Instant::now(),
            clock_floor: 0.0,
            tick: None,
            _bridge: None,
        }
    }

    /// The model, for reading in `render`.
    pub fn model(&self) -> &Model {
        &self.model
    }

    /// Increases with every update.
    pub fn revision(&self) -> u64 {
        self.revision
    }

    /// Applies a handler to the model, then drains the outbox, schedules the
    /// tick and notifies. Every change of the model goes through here.
    pub fn run(&mut self, act: impl FnOnce(&mut Model), cx: &mut Context<Self>) {
        act(&mut self.model);
        self.after_update(cx);
        cx.notify();
    }

    /// Lets the model read its core handles. Called on every wake and timed
    /// tick. Notifies when something the model shows changed.
    pub fn poll(&mut self, cx: &mut Context<Self>) {
        let clock = self.clock();
        let changed = self.model.poll(clock);
        self.after_update(cx);
        if changed {
            cx.notify();
        }
    }

    /// Monotonic seconds, the clock `Model::poll` is given.
    pub fn clock(&self) -> f64 {
        self.started.elapsed().as_secs_f64().max(self.clock_floor)
    }

    fn after_update(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        *self.surfaces.borrow_mut() = None;
        self.drain_intents(cx);
        self.schedule_tick(cx);
    }

    fn drain_intents(&mut self, cx: &mut Context<Self>) {
        let intents = self.model.take_intents();
        if intents.is_empty() {
            return;
        }
        match &mut self.intents {
            IntentSink::Record(recorded) => recorded.extend(intents),
            IntentSink::Handler(_) => {
                // The handler is given the context, so it is taken out while
                // it runs. It must not call `run` re-entrantly: it performs
                // the service and answers later through the entity.
                let IntentSink::Handler(mut handler) =
                    std::mem::replace(&mut self.intents, IntentSink::Record(Vec::new()))
                else {
                    return;
                };
                for intent in intents {
                    handler(intent, cx);
                }
                self.intents = IntentSink::Handler(handler);
            }
        }
    }

    fn schedule_tick(&mut self, cx: &mut Context<Self>) {
        let Some(seconds) = self.model.take_tick_request() else {
            return;
        };
        let seconds = seconds.max(0.0);
        let due = self.clock() + seconds;
        // The earliest request wins; a later one is covered by the poll the
        // pending tick causes, which lets the model ask again.
        if self
            .tick
            .as_ref()
            .is_some_and(|(pending, _)| *pending <= due)
        {
            return;
        }
        let task = cx.spawn(async move |this, cx| {
            cx.background_executor()
                .timer(Duration::from_secs_f64(seconds))
                .await;
            let _ = this.update(cx, |store, cx| {
                store.clock_floor = store.clock_floor.max(due);
                // Dropping the handle of the running task is harmless: no
                // await follows.
                let pending = store.tick.take();
                store.poll(cx);
                drop(pending);
            });
        });
        self.tick = Some((due, task));
    }

    /// Installs what performs the model's intents (file prompts, URLs, the
    /// clipboard, console windows, quit). Until then they are recorded.
    pub fn on_intent(&mut self, handler: impl FnMut(Intent, &mut Context<Self>) + 'static) {
        self.intents = IntentSink::Handler(Box::new(handler));
    }

    /// The intents recorded so far, oldest first. Empty once a handler is
    /// installed.
    pub fn recorded_intents(&self) -> &[Intent] {
        match &self.intents {
            IntentSink::Record(recorded) => recorded,
            IntentSink::Handler(_) => &[],
        }
    }

    /// Takes the recorded intents.
    pub fn take_recorded_intents(&mut self) -> Vec<Intent> {
        match &mut self.intents {
            IntentSink::Record(recorded) => std::mem::take(recorded),
            IntentSink::Handler(_) => Vec::new(),
        }
    }

    /// Gives the model its waker and starts the task that polls the model
    /// whenever the core signals. Call it once, before the first poll.
    pub fn connect_core(&mut self, cx: &mut Context<Self>) {
        let (waker, receiver) = wake_channel();
        self.model.setup(waker);
        self._bridge = Some(Self::bridge(receiver, cx));
        self.after_update(cx);
        cx.notify();
    }

    fn bridge(receiver: WakeReceiver, cx: &mut Context<Self>) -> Task<()> {
        cx.spawn(async move |this, cx| {
            // The flag is cleared when `woken` returns: poll now.
            while receiver.woken().await {
                if this.update(cx, |store, cx| store.poll(cx)).is_err() {
                    break;
                }
            }
        })
    }

    /// Substitutes the source of descriptions.
    pub fn set_describe(&mut self, describe: Describe, cx: &mut Context<Self>) {
        self.describe = describe;
        self.revision += 1;
        *self.surfaces.borrow_mut() = None;
        cx.notify();
    }

    /// Says which surfaces the front end shows by itself. They are in front
    /// of the model's. Their handlers run through [`Self::run`] like any
    /// other, so a change of what they show is drawn.
    pub fn set_front_surfaces(&mut self, front: Option<FrontSurfaces>, cx: &mut Context<Self>) {
        self.front = front;
        self.touch(cx);
    }

    /// Draws again without a change of the model: something the
    /// descriptions read besides the model has changed.
    pub fn touch(&mut self, cx: &mut Context<Self>) {
        self.revision += 1;
        *self.surfaces.borrow_mut() = None;
        cx.notify();
    }

    /// The open surfaces, back to front. Built once per update: the dialog
    /// host and every dialog read the same list.
    pub fn surfaces(&self) -> Rc<Vec<Surface>> {
        if let Some(surfaces) = self.surfaces.borrow().as_ref() {
            return Rc::clone(surfaces);
        }
        let mut surfaces = (self.describe.surfaces)(&self.model);
        if let Some(front) = &self.front {
            surfaces.extend(front(&self.model));
        }
        let surfaces = Rc::new(surfaces);
        *self.surfaces.borrow_mut() = Some(Rc::clone(&surfaces));
        surfaces
    }

    /// The surface with this id, when it is open.
    pub fn surface(&self, id: &str) -> Option<Surface> {
        self.surfaces()
            .iter()
            .find(|surface| surface.id == id)
            .cloned()
    }

    /// The body of a tab for the current scope.
    pub fn tab_body(&self, tab: TabId) -> Option<TabBody> {
        (self.describe.tab_body)(&self.model, tab)
    }

    /// The local action row of a tab: the commands registered for it,
    /// resolved on the model's target.
    pub fn local_row(&self, tab: TabId) -> Vec<command::MenuItem> {
        (self.describe.local_row)(&self.model, tab)
    }

    /// The menu bar, resolved on the model's target.
    pub fn menu_bar(&self) -> Vec<command::MenuView> {
        (self.describe.menu_bar)(&self.model)
    }

    /// The toolbar, resolved on the model's target.
    pub fn toolbar(&self) -> Vec<Vec<command::MenuItem>> {
        (self.describe.toolbar)(&self.model)
    }

    /// Runs a command on the model's target: a menu item, a toolbar button,
    /// a button of a local action row or a shortcut. The command checks its
    /// eligibility again; a refusal is in the model's error line.
    pub fn invoke(&mut self, id: CommandId, cx: &mut Context<Self>) {
        let invoke = Rc::clone(&self.describe.invoke);
        self.run(move |model| invoke(model, id), cx);
    }
}

/// Applies a handler through the store. The form every widget callback
/// uses: `dispatch(&store, cx, move |m| on_press(m))`.
pub fn dispatch(store: &Entity<AppStore>, cx: &mut App, act: impl FnOnce(&mut Model)) {
    store.update(cx, |store, cx| store.run(act, cx));
}

/// A new store entity over `model`.
pub fn new_store(model: Model, cx: &mut App) -> Entity<AppStore> {
    cx.new(|_| AppStore::new(model))
}
