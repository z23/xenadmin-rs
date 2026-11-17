//! Every Kit import the views use goes through this module, so a later Kit
//! upgrade touches one file.
//!
//! The front end reaches GPUI only through the `gpui_kit` facade. Views
//! write `use crate::kit::*;` and name nothing from `gpui_kit` themselves.
//! Names are listed one by one: a glob of `gpui_kit` would also bring in the
//! `test` attribute macro and shadow `#[test]`.

/// The reviewed facade.
pub use gpui_kit;

// Element traits: Render, IntoElement, ParentElement, Styled,
// InteractiveElement, StatefulInteractiveElement, FluentBuilder, AppContext.
pub use gpui_kit::prelude::*;

// GPUI itself.
pub use gpui_kit::{
    AnyElement, AnyView, AnyWindowHandle, App, AsyncApp, Bounds, ClickEvent, Context, Div,
    ElementId, Entity, EventEmitter, FocusHandle, Focusable, FontWeight, Hsla, KeyBinding,
    Modifiers, MouseButton, Pixels, Role, ScrollHandle, SharedString, Stateful, Subscription, Task,
    TestSupportExt, WeakEntity, Window, WindowBounds, WindowOptions, canvas, div, point, px,
    relative, rems, size,
};

// The channel of the wake bridge.
pub use gpui_kit::base::async_util::{Receiver, Sender, unbounded};

// Theme.
pub use gpui_kit::component::{ActiveTheme, Theme, ThemeMode, oklch};

// Layout and shared component traits.
pub use gpui_kit::component::scroll::ScrollableElement;
pub use gpui_kit::component::{
    Disableable, Icon, IconName, Selectable, Sizable, Size, StyledExt, WindowExt, h_flex, v_flex,
};

// Components.
pub use gpui_kit::component::button::{Button, ButtonGroup, ButtonVariants};
pub use gpui_kit::component::checkbox::Checkbox;
pub use gpui_kit::component::dialog::Dialog;
pub use gpui_kit::component::input::{Input, InputEvent, InputState, Textarea, TextareaState};
pub use gpui_kit::component::menu::{PopupMenu, PopupMenuItem};
pub use gpui_kit::component::progress::Progress;
pub use gpui_kit::component::radio::Radio;
pub use gpui_kit::component::select::{
    SearchableVec, Select, SelectEvent, SelectItem, SelectState,
};
pub use gpui_kit::component::separator::Separator;
pub use gpui_kit::component::spinner::Spinner;
pub use gpui_kit::component::stepper::{Stepper, StepperItem};
pub use gpui_kit::component::table::{
    Column as KitColumn, ColumnSort, DataTable, TableDelegate, TableEvent, TableState,
};
pub use gpui_kit::component::tooltip::Tooltip;
