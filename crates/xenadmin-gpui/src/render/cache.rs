//! The cache of widget state.
//!
//! A description is rebuilt on every render and holds no widget. The Kit's
//! text fields, selects and tables keep state in entities that must live
//! across frames (caret, focus, scroll offset, open menu). They are kept
//! here, one per scope and element key.
//!
//! A scope is one surface (`surface:<id>`) or the tab body (`tab`). An
//! entry that a render pass of its scope did not use is dropped at the end
//! of that pass, so a field that left the description leaves the cache; a
//! scope is dropped when its surface closes. Before a secret field's state
//! is dropped its buffer is emptied.

use std::cell::RefCell;
use std::collections::{HashMap, HashSet};
use std::rc::Rc;

use xenadmin_ui::SecretText;
use xenadmin_ui::surface::{Act, ActText, OptionItem};

use crate::kit::*;
use crate::store::{AppStore, dispatch};
use crate::widgets::object_grid::ObjectGrid;

/// The prefix of the scope of a tab body: `tab:<tab key>`.
pub const TAB_SCOPE: &str = "tab";

/// The scope of a surface.
pub fn surface_scope(id: &str) -> String {
    format!("surface:{id}")
}

#[derive(Default)]
pub struct WidgetCache {
    scopes: HashMap<String, Scope>,
}

#[derive(Default)]
struct Scope {
    texts: HashMap<String, TextEntry>,
    selects: HashMap<String, SelectEntry>,
    grids: HashMap<String, ObjectGrid>,
    /// Whether a collapsible group is folded, by its path.
    folded: HashMap<String, bool>,
    /// The focus handles of the renderer's own pressable elements.
    handles: HashMap<String, FocusHandle>,
    /// The entries the current pass used.
    used: HashSet<String>,
    /// The first text field the current pass drew, for the initial focus.
    first_text: Option<String>,
}

/// The latest handlers of a text field. A description is rebuilt on every
/// render, so the subscription reads them from here.
pub(crate) struct TextHandlers {
    pub on_edit: ActText,
    pub on_submit: Option<Act>,
    /// An edit was handed to the model since the last pass.
    pub delivered: bool,
}

/// A single-line or multi-line text widget.
#[derive(Clone)]
pub(crate) enum TextWidget {
    Line(Entity<InputState>),
    Area(Entity<TextareaState>),
}

impl TextWidget {
    pub fn value(&self, cx: &App) -> SharedString {
        match self {
            Self::Line(state) => state.read(cx).value(),
            Self::Area(state) => state.read(cx).value(),
        }
    }

    pub fn set_value(&self, value: &str, window: &mut Window, cx: &mut App) {
        let value = SharedString::from(value.to_owned());
        match self {
            Self::Line(state) => state.update(cx, |state, cx| state.set_value(value, window, cx)),
            Self::Area(state) => state.update(cx, |state, cx| state.set_value(value, window, cx)),
        }
    }

    pub fn set_placeholder(&self, placeholder: &str, window: &mut Window, cx: &mut App) {
        let placeholder = SharedString::from(placeholder.to_owned());
        match self {
            Self::Line(state) => {
                state.update(cx, |state, cx| {
                    state.set_placeholder(placeholder, window, cx)
                });
            }
            Self::Area(state) => {
                state.update(cx, |state, cx| {
                    state.set_placeholder(placeholder, window, cx)
                });
            }
        }
    }

    pub fn focus(&self, window: &mut Window, cx: &mut App) {
        match self {
            Self::Line(state) => state.update(cx, |state, cx| state.focus(window, cx)),
            Self::Area(state) => state.update(cx, |state, cx| state.focus(window, cx)),
        }
    }
}

struct TextEntry {
    widget: TextWidget,
    secret: bool,
    /// The placeholder the widget shows: the Kit has no getter for it.
    placeholder: String,
    /// The model's text at the last pass. For a secret, only its length
    /// and whether it was empty are kept, never the text.
    seen: Seen,
    handlers: Rc<RefCell<TextHandlers>>,
    _subscription: Subscription,
}

/// What a pass remembers of the model's text, to tell a change made by the
/// model from an edit of the user that has not reached the model yet.
#[derive(Clone, PartialEq, Eq)]
enum Seen {
    Text(String),
    /// A secret is not copied: a fingerprint that changes when it does.
    Secret(u64),
}

impl Seen {
    fn of(value: &str, secret: bool) -> Self {
        if secret {
            use std::hash::{Hash, Hasher};
            let mut hasher = std::collections::hash_map::DefaultHasher::new();
            value.hash(&mut hasher);
            Self::Secret(hasher.finish())
        } else {
            Self::Text(value.to_owned())
        }
    }
}

/// One option of a select control, as the Kit's select wants it.
#[derive(Clone, PartialEq)]
pub(crate) struct SelectOption {
    pub id: SharedString,
    pub label: SharedString,
    pub disabled: bool,
}

impl SelectItem for SelectOption {
    type Value = SharedString;

    fn title(&self) -> SharedString {
        self.label.clone()
    }

    fn value(&self) -> &Self::Value {
        &self.id
    }

    fn disabled(&self) -> bool {
        self.disabled
    }
}

pub(crate) type SelectWidget = Entity<SelectState<SearchableVec<SelectOption>>>;

struct SelectEntry {
    widget: SelectWidget,
    options: Vec<OptionItem>,
    on_pick: Rc<RefCell<ActText>>,
    _subscription: Subscription,
}

fn select_options(options: &[OptionItem]) -> SearchableVec<SelectOption> {
    SearchableVec::new(
        options
            .iter()
            .map(|option| SelectOption {
                id: option.id.clone().into(),
                label: match &option.detail {
                    Some(detail) => format!("{} ({detail})", option.label).into(),
                    None => option.label.clone().into(),
                },
                disabled: option.disabled.is_some(),
            })
            .collect::<Vec<_>>(),
    )
}

/// What a text field of a description is, for [`WidgetCache::text`].
pub(crate) struct TextSpec<'a> {
    pub key: &'a str,
    /// The model's text. For a secret field, the secret's text.
    pub value: &'a str,
    pub secret: bool,
    pub multiline: bool,
    pub placeholder: &'a str,
    pub on_edit: &'a ActText,
    pub on_submit: Option<&'a Act>,
}

impl WidgetCache {
    fn scope(&mut self, scope: &str) -> &mut Scope {
        self.scopes.entry(scope.to_owned()).or_default()
    }

    /// Starts a render pass of a scope.
    pub fn begin(&mut self, scope: &str) {
        let scope = self.scope(scope);
        scope.used.clear();
        scope.first_text = None;
    }

    /// Ends a render pass of a scope: drops what the pass did not use.
    pub fn end(&mut self, scope: &str, window: &mut Window, cx: &mut App) {
        let Some(scope) = self.scopes.get_mut(scope) else {
            return;
        };
        let used = std::mem::take(&mut scope.used);
        scope.texts.retain(|key, entry| {
            let keep = used.contains(key);
            if !keep && entry.secret {
                entry.widget.set_value("", window, cx);
            }
            keep
        });
        scope.selects.retain(|key, _| used.contains(key));
        scope.grids.retain(|key, _| used.contains(key));
        scope.used = used;
    }

    /// Drops a scope, emptying the buffer of every secret field first.
    pub fn close(&mut self, scope: &str, window: &mut Window, cx: &mut App) {
        if let Some(scope) = self.scopes.remove(scope) {
            for entry in scope.texts.values().filter(|entry| entry.secret) {
                entry.widget.set_value("", window, cx);
            }
        }
    }

    /// Drops the scope of every tab body but `scope`: the state of a tab
    /// goes when another tab is shown.
    pub fn keep_tab_scope(&mut self, scope: &str, window: &mut Window, cx: &mut App) {
        let prefix = format!("{TAB_SCOPE}:");
        let stale: Vec<String> = self
            .scopes
            .keys()
            .filter(|key| key.starts_with(&prefix) && key.as_str() != scope)
            .cloned()
            .collect();
        for key in stale {
            self.close(&key, window, cx);
        }
    }

    /// The scopes that hold state now.
    pub fn scopes(&self) -> Vec<String> {
        let mut scopes: Vec<String> = self.scopes.keys().cloned().collect();
        scopes.sort();
        scopes
    }

    /// The text widget of a field, if the scope holds one. For tests.
    pub fn text_value(&self, scope: &str, key: &str, cx: &App) -> Option<String> {
        let entry = self.scopes.get(scope)?.texts.get(key)?;
        Some(entry.widget.value(cx).to_string())
    }

    /// The Kit state of a single-line text field, if the scope holds one.
    /// For tests: it outlives the scope, so a test can see what became of
    /// the buffer after the scope was closed.
    pub fn text_input(&self, scope: &str, key: &str) -> Option<Entity<InputState>> {
        match &self.scopes.get(scope)?.texts.get(key)?.widget {
            TextWidget::Line(state) => Some(state.clone()),
            TextWidget::Area(_) => None,
        }
    }

    /// The first text field the last pass of the scope drew.
    pub(crate) fn first_text(&self, scope: &str) -> Option<TextWidget> {
        let scope = self.scopes.get(scope)?;
        let key = scope.first_text.as_ref()?;
        scope.texts.get(key).map(|entry| entry.widget.clone())
    }

    /// The widget of a text field: created on first use, then corrected
    /// where it differs from the model.
    pub(crate) fn text(
        &mut self,
        scope: &str,
        spec: TextSpec<'_>,
        store: &Entity<AppStore>,
        window: &mut Window,
        cx: &mut App,
    ) -> TextWidget {
        let scope = self.scope(scope);
        scope.used.insert(spec.key.to_owned());
        if scope.first_text.is_none() && !spec.multiline {
            scope.first_text = Some(spec.key.to_owned());
        }
        // A field that changed its kind under the same key starts again.
        let stale = scope.texts.get(spec.key).is_some_and(|entry| {
            entry.secret != spec.secret
                || matches!(entry.widget, TextWidget::Area(_)) != spec.multiline
        });
        if stale && let Some(entry) = scope.texts.remove(spec.key) {
            entry.widget.set_value("", window, cx);
        }
        let entry = scope.texts.entry(spec.key.to_owned()).or_insert_with(|| {
            let handlers = Rc::new(RefCell::new(TextHandlers {
                on_edit: spec.on_edit.clone(),
                on_submit: spec.on_submit.cloned(),
                delivered: false,
            }));
            let value = SharedString::from(spec.value.to_owned());
            let placeholder = SharedString::from(spec.placeholder.to_owned());
            let secret = spec.secret;
            let (widget, subscription) = if spec.multiline {
                let state = cx.new(|cx| {
                    TextareaState::new(window, cx)
                        .rows(4)
                        .placeholder(placeholder)
                        .default_value(value)
                });
                let subscription = cx.subscribe(&state, {
                    let store = store.clone();
                    let handlers = Rc::clone(&handlers);
                    move |state, event: &InputEvent, cx| {
                        if matches!(event, InputEvent::Change) {
                            let value = state.read(cx).value();
                            edited(&store, &handlers, &value, false, cx);
                        }
                    }
                });
                (TextWidget::Area(state), subscription)
            } else {
                let state = cx.new(|cx| {
                    InputState::new(window, cx)
                        .masked(secret)
                        .placeholder(placeholder)
                        .default_value(value)
                });
                let subscription = cx.subscribe(&state, {
                    let store = store.clone();
                    let handlers = Rc::clone(&handlers);
                    move |state, event: &InputEvent, cx| match event {
                        InputEvent::Change => {
                            let value = state.read(cx).value();
                            edited(&store, &handlers, &value, secret, cx);
                        }
                        InputEvent::PressEnter { .. } => {
                            let on_submit = handlers.borrow().on_submit.clone();
                            if let Some(on_submit) = on_submit {
                                dispatch(&store, cx, move |m| on_submit(m));
                            }
                        }
                        InputEvent::Focus | InputEvent::Blur => {}
                    }
                });
                (TextWidget::Line(state), subscription)
            };
            TextEntry {
                widget,
                secret,
                placeholder: spec.placeholder.to_owned(),
                seen: Seen::of(spec.value, secret),
                handlers,
                _subscription: subscription,
            }
        });
        let delivered = {
            let mut handlers = entry.handlers.borrow_mut();
            handlers.on_edit = spec.on_edit.clone();
            handlers.on_submit = spec.on_submit.cloned();
            std::mem::take(&mut handlers.delivered)
        };
        let widget = entry.widget.clone();
        // The model's text is the truth, and the widget is written only
        // where it differs: `set_value` moves the caret to the end.
        //
        // The widget may also differ because the user has just typed and
        // the edit is still on its way to the model (the Kit delivers it
        // when the current update ends, and a frame can be drawn before
        // that). Writing then would undo the keystroke. So the widget is
        // corrected when the model's text changed since the last pass, or
        // when an edit has been delivered and the model answered with
        // another text than the widget has.
        let seen = Seen::of(spec.value, spec.secret);
        let model_changed = entry.seen != seen;
        entry.seen = seen;
        if (model_changed || delivered) && widget.value(cx).as_ref() != spec.value {
            widget.set_value(spec.value, window, cx);
        }
        if entry.placeholder != spec.placeholder {
            entry.placeholder = spec.placeholder.to_owned();
            widget.set_placeholder(spec.placeholder, window, cx);
        }
        widget
    }

    /// The widget of a select control, kept in step with the options and
    /// the selected option of the description.
    pub(crate) fn select(
        &mut self,
        scope: &str,
        key: &str,
        options: &[OptionItem],
        selected: Option<&String>,
        on_pick: &ActText,
        store: &Entity<AppStore>,
        window: &mut Window,
        cx: &mut App,
    ) -> SelectWidget {
        let scope = self.scope(scope);
        scope.used.insert(key.to_owned());
        let entry = scope.selects.entry(key.to_owned()).or_insert_with(|| {
            let handler = Rc::new(RefCell::new(on_pick.clone()));
            let widget = cx.new(|cx| SelectState::new(select_options(options), None, window, cx));
            let subscription = cx.subscribe(&widget, {
                let store = store.clone();
                let handler = Rc::clone(&handler);
                move |_, event: &SelectEvent<SearchableVec<SelectOption>>, cx| {
                    let SelectEvent::Confirm(value) = event;
                    // A cleared select reports `None`; the model keeps its
                    // selection and the next render shows it again.
                    if let Some(id) = value.clone() {
                        let on_pick = handler.borrow().clone();
                        dispatch(&store, cx, move |m| on_pick(m, &id));
                    }
                }
            });
            SelectEntry {
                widget,
                options: options.to_vec(),
                on_pick: handler,
                _subscription: subscription,
            }
        });
        *entry.on_pick.borrow_mut() = on_pick.clone();
        let widget = entry.widget.clone();
        if entry.options != options {
            entry.options = options.to_vec();
            let items = select_options(options);
            widget.update(cx, |state, cx| state.set_items(items, window, cx));
        }
        let wanted: Option<SharedString> = selected
            .filter(|id| options.iter().any(|option| &option.id == *id))
            .map(|id| id.clone().into());
        if widget.read(cx).selected_value() != wanted.as_ref() {
            widget.update(cx, |state, cx| match &wanted {
                Some(id) => state.set_selected_value(id, window, cx),
                None => state.set_selected_index(None, window, cx),
            });
        }
        widget
    }

    /// The grid of a table, created on first use.
    pub(crate) fn grid(
        &mut self,
        scope: &str,
        key: &str,
        create: impl FnOnce() -> ObjectGrid,
    ) -> ObjectGrid {
        let scope = self.scope(scope);
        scope.used.insert(key.to_owned());
        scope
            .grids
            .entry(key.to_owned())
            .or_insert_with(create)
            .clone()
    }

    /// The focus handle of a pressable element the renderer draws itself
    /// (a tab, a page of a properties dialog, the title of a section that
    /// folds). It is a Tab stop.
    pub(crate) fn focus_handle(&mut self, scope: &str, key: &str, cx: &mut App) -> FocusHandle {
        self.scope(scope)
            .handles
            .entry(key.to_owned())
            .or_insert_with(|| cx.focus_handle().tab_stop(true))
            .clone()
    }

    /// Whether the group at `path` is folded. `initial` is what the
    /// description asks for before the user touched it.
    pub(crate) fn folded(&mut self, scope: &str, path: &str, initial: bool) -> bool {
        *self
            .scope(scope)
            .folded
            .entry(path.to_owned())
            .or_insert(initial)
    }

    pub(crate) fn toggle_folded(&mut self, scope: &str, path: &str) {
        if let Some(folded) = self.scope(scope).folded.get_mut(path) {
            *folded = !*folded;
        }
    }
}

/// Hands an edit to the model. The copy of a secret is zeroized when it is
/// dropped.
fn edited(
    store: &Entity<AppStore>,
    handlers: &Rc<RefCell<TextHandlers>>,
    value: &SharedString,
    secret: bool,
    cx: &mut App,
) {
    let on_edit = {
        let mut handlers = handlers.borrow_mut();
        handlers.delivered = true;
        handlers.on_edit.clone()
    };
    if secret {
        let value = SecretText::from(value.as_ref());
        dispatch(store, cx, move |m| on_edit(m, value.expose()));
    } else {
        let value = value.to_string();
        dispatch(store, cx, move |m| on_edit(m, &value));
    }
}
