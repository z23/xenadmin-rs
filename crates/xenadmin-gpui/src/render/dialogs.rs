//! The dialog host: keeps the window's dialogs equal to `model.surfaces()`.
//!
//! The model is the one truth for which surfaces are open and in which
//! order. The host watches the store and, whenever the list of surface ids
//! differs from the dialogs it has presented, closes the Kit dialogs above
//! the common part and opens the missing ones.
//!
//! A Kit dialog never closes by itself here: Escape, a press on the overlay
//! and the close button run the surface's `on_close` and veto the Kit's own
//! close; Enter is vetoed too, so Enter in a text field only does what the
//! field's `on_submit` says. When the model then drops the surface, the
//! host closes the dialog.

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use xenadmin_ui::surface::{Surface, SurfaceKind, SurfaceSize};

use super::blocks::{Pass, blocks, button_row};
use super::cache::surface_scope;
use super::{Ui, element_id, join};
use crate::kit::*;
use crate::store::dispatch;
use crate::widgets;

/// The width a surface asks for. The Kit never lets a dialog be wider than
/// the window less a margin.
pub fn surface_width(size: SurfaceSize) -> Pixels {
    match size {
        SurfaceSize::Small => px(460.),
        SurfaceSize::Medium => px(640.),
        SurfaceSize::Large => px(880.),
    }
}

/// The padding of a dialog, on every side.
const PADDING: f32 = 20.0;
/// The margin the Kit keeps around a dialog.
const MARGIN: f32 = 16.0;

pub struct DialogHost {
    ui: Ui,
    /// The ids of the surfaces the Kit shows, back to front.
    presented: Vec<String>,
    _subscription: Subscription,
}

impl DialogHost {
    /// A host for the window. It opens the dialogs of the surfaces the
    /// model has already, at the end of the current update.
    pub fn new(ui: Ui, window: &mut Window, cx: &mut Context<Self>) -> Self {
        let subscription = cx.observe_in(&ui.store, window, |host, _, window, cx| {
            host.reconcile(window, cx);
        });
        cx.defer_in(window, |host, window, cx| host.reconcile(window, cx));
        Self {
            ui,
            presented: Vec::new(),
            _subscription: subscription,
        }
    }

    /// The ids of the surfaces shown as dialogs, back to front.
    pub fn presented(&self) -> &[String] {
        &self.presented
    }

    /// Makes the window's dialogs equal to the model's surfaces.
    pub fn reconcile(&mut self, window: &mut Window, cx: &mut Context<Self>) {
        let wanted: Vec<String> = self
            .ui
            .store
            .read(cx)
            .surfaces()
            .iter()
            .map(|surface| surface.id.clone())
            .collect();
        // Nothing but this host closes a dialog. Should the Kit have lost
        // them all the same, present them again.
        if !self.presented.is_empty() && !window.has_active_dialog(cx) {
            self.presented.clear();
        }
        if wanted == self.presented {
            return;
        }
        let keep = wanted
            .iter()
            .zip(&self.presented)
            .take_while(|(wanted, presented)| wanted == presented)
            .count();
        // The Kit closes from the front only.
        let closed: Vec<String> = self.presented.drain(keep..).collect();
        for _ in &closed {
            window.close_dialog(cx);
        }
        for id in closed.iter().filter(|id| !wanted.contains(id)) {
            // The surface is gone: its widget state goes, and the buffer of
            // a secret field is emptied first.
            self.ui
                .cache
                .borrow_mut()
                .close(&surface_scope(id), window, cx);
        }
        for id in &wanted[keep..] {
            self.open(id.clone(), window, cx);
            self.presented.push(id.clone());
        }
        cx.notify();
    }

    fn open(&self, id: String, window: &mut Window, cx: &mut App) {
        let ui = self.ui.clone();
        let first_frame = Rc::new(Cell::new(true));
        // What the dialog showed last: it is drawn while the Kit animates
        // the dialog out, when the model has the surface no more.
        let last: Rc<RefCell<Option<Surface>>> = Rc::default();
        window.open_dialog(cx, move |dialog, window, cx| {
            let live = ui.store.read(cx).surface(&id);
            let closing = live.is_none();
            let surface = live.or_else(|| last.borrow().clone());
            let Some(surface) = surface else {
                return dialog.close_button(false).keyboard(false);
            };
            *last.borrow_mut() = Some(surface.clone());
            surface_dialog(dialog, &ui, &surface, closing, &first_frame, window, cx)
        });
    }
}

impl Render for DialogHost {
    fn render(&mut self, _: &mut Window, _: &mut Context<Self>) -> impl IntoElement {
        // The Kit draws the dialogs in its own layer above the view.
        div()
    }
}

/// Builds the Kit dialog of a surface.
fn surface_dialog(
    dialog: Dialog,
    ui: &Ui,
    surface: &Surface,
    closing: bool,
    first_frame: &Rc<Cell<bool>>,
    window: &mut Window,
    cx: &mut App,
) -> Dialog {
    let scope = surface_scope(&surface.id);
    let viewport = window.viewport_size();
    let width = surface_width(surface.size).min((viewport.width - px(MARGIN * 2.)).max(px(0.)));
    let inner = (width - px(PADDING * 2.)).max(px(120.));
    let close = {
        let store = ui.store.clone();
        let on_close = surface.on_close.clone();
        Rc::new(move |cx: &mut App| {
            let on_close = on_close.clone();
            dispatch(&store, cx, move |m| on_close(m));
        })
    };
    let pass = Pass {
        ui,
        scope: &scope,
        width: inner,
        fill: false,
        on_cancel: Some(close.clone()),
    };

    if !closing {
        ui.cache.borrow_mut().begin(&scope);
    }
    let header = header(surface, cx);
    let body = match &surface.kind {
        SurfaceKind::Properties {
            pages,
            current,
            on_page,
        } => {
            // The page list and the page scroll by themselves in a frame
            // of a fixed height, so the list stays put while a page moves.
            let height = (viewport.height - px(250.)).clamp(px(200.), px(520.));
            let page_width = (inner - px(widgets::PAGE_LIST_WIDTH + 17.)).max(px(120.));
            let page_pass = Pass {
                width: page_width,
                ..pass.clone()
            };
            let page = v_flex()
                .w_full()
                .gap_3()
                .pr_3()
                .children(blocks(&page_pass, &surface.id, &surface.body, window, cx))
                .into_any_element();
            let store = ui.store.clone();
            let on_page = on_page.clone();
            widgets::properties_frame(
                ui,
                &scope,
                &surface.pages_key(),
                pages,
                current,
                move |id: &str, _: &mut Window, cx: &mut App| {
                    let (on_page, id) = (on_page.clone(), id.to_owned());
                    dispatch(&store, cx, move |m| on_page(m, &id));
                },
                page,
                height,
                cx,
            )
        }
        SurfaceKind::Dialog | SurfaceKind::Wizard { .. } => div()
            .flex_1()
            .min_h_0()
            .overflow_hidden()
            .child(
                v_flex()
                    .id(SharedString::from(format!("{}-body", surface.id)))
                    .size_full()
                    .overflow_y_scrollbar()
                    .gap_3()
                    // Room for the focus ring of a control at the edge.
                    .p_0p5()
                    .children(blocks(&pass, &surface.id, &surface.body, window, cx)),
            )
            .into_any_element(),
    };
    let footer = footer(ui, surface, cx);
    if !closing {
        ui.cache.borrow_mut().end(&scope, window, cx);
        if first_frame.replace(false) {
            // The first text field takes the keyboard when a surface opens.
            let first = ui.cache.borrow().first_text(&scope);
            if let Some(first) = first {
                first.focus(window, cx);
            }
        }
    }

    let title = SharedString::from(surface.title.clone());
    let id = SharedString::from(surface.id.clone());
    // Everything of the surface is one element under the surface's id: the
    // header and the footer keep their place and the body between them
    // scrolls. The Kit's `content` builder is an `Fn` that it calls once
    // per frame; the element of this frame is handed over through a slot.
    let content = v_flex()
        .id(id)
        .test_support()
        .role(Role::Group)
        .aria_label(title)
        .w_full()
        .flex_1()
        .min_h_0()
        .overflow_hidden()
        .gap_4()
        .child(header)
        .child(body)
        .children(footer)
        .into_any_element();
    let slot = RefCell::new(Some(content));
    dialog
        .width(width)
        .p(px(PADDING))
        .overlay_closable(true)
        .close_button(true)
        .keyboard(true)
        // The model decides when a surface closes: both paths are vetoed.
        .on_cancel(move |_, _, cx| {
            close(cx);
            false
        })
        .on_ok(|_, _, _| false)
        .content(move |content, _, _| {
            content
                .min_h_0()
                .overflow_hidden()
                .children(slot.borrow_mut().take())
        })
}

/// The title, the subtitle, the experimental notice and the steps of a
/// wizard: the part of a surface that does not scroll.
fn header(surface: &Surface, cx: &App) -> AnyElement {
    let muted = cx.theme().muted_foreground;
    v_flex()
        .w_full()
        .flex_none()
        .gap_3()
        .child(
            v_flex()
                .gap_1()
                // Room for the close button.
                .pr_8()
                .child(
                    div()
                        .id(SharedString::from(join(&surface.id, "title")))
                        .test_support()
                        .role(Role::Heading)
                        .aria_label(SharedString::from(surface.title.clone()))
                        .text_lg()
                        .font_semibold()
                        .child(SharedString::from(surface.title.clone())),
                )
                .when_some(surface.subtitle.clone(), |column, subtitle| {
                    let subtitle = SharedString::from(subtitle);
                    column.child(
                        div()
                            .id(SharedString::from(join(&surface.id, "subtitle")))
                            .test_support()
                            .role(Role::Label)
                            .aria_label(subtitle.clone())
                            .text_sm()
                            .font_normal()
                            .text_color(muted)
                            .child(subtitle),
                    )
                }),
        )
        .when(surface.experimental, |column| {
            column.child(widgets::experimental_notice(cx))
        })
        .when_some(
            match &surface.kind {
                SurfaceKind::Wizard { steps, current } => Some((steps, *current)),
                _ => None,
            },
            |column, (steps, current)| {
                column.child(widgets::wizard_frame(&surface.id, steps, current, cx))
            },
        )
        .into_any_element()
}

/// The footer note and the footer buttons, under a rule. The last button
/// is the default action.
fn footer(ui: &Ui, surface: &Surface, cx: &App) -> Option<AnyElement> {
    if surface.footer.is_empty() && surface.footer_note.is_none() {
        return None;
    }
    let base = join(&surface.id, "footer");
    let border = cx.theme().border;
    Some(
        v_flex()
            .w_full()
            .flex_none()
            .gap_3()
            .pt_3()
            .border_t_1()
            .border_color(border)
            .font_normal()
            .when_some(surface.footer_note.as_ref(), |column, note| {
                column.child(widgets::text(
                    element_id(&base, note.key.as_ref(), 0),
                    note,
                    cx,
                ))
            })
            .when(!surface.footer.is_empty(), |column| {
                column.child(button_row(ui, &surface.footer, cx).justify_end())
            })
            .into_any_element(),
    )
}
