//! `AppStore`: the one writer of the model. A handler is applied and
//! observers are told, the intents the model queued are handed on, a tick
//! the model asked for polls it when it is due, and a wake from the core
//! polls it.

use std::cell::RefCell;
use std::rc::Rc;
use std::time::Duration;

use gpui_kit::{Entity, TestAppContext};
use xenadmin_desktop::{AppStore, dispatch, new_store};
use xenadmin_ui::Model;
use xenadmin_ui::intent::Intent;

/// A store over a demo model. Demo mode restores no saved session, so
/// nothing here reads the user's connections.
fn store(cx: &mut TestAppContext) -> Entity<AppStore> {
    cx.update(|cx| {
        let mut model = Model::fixture();
        model.load_demo(1, 2);
        new_store(model, cx)
    })
}

#[gpui_kit::test]
fn a_handler_is_applied_and_observers_are_told(cx: &mut TestAppContext) {
    let store = store(cx);
    let seen = Rc::new(RefCell::new(0));
    let subscription = cx.update(|cx| {
        let seen = seen.clone();
        cx.observe(&store, move |_, _| *seen.borrow_mut() += 1)
    });
    let before = cx.update(|cx| store.read(cx).revision());
    cx.update(|cx| dispatch(&store, cx, |m| m.error = Some("from a handler".into())));
    cx.run_until_parked();
    cx.update(|cx| {
        let store = store.read(cx);
        assert_eq!(store.model().error.as_deref(), Some("from a handler"));
        assert!(store.revision() > before);
    });
    assert_eq!(*seen.borrow(), 1);
    drop(subscription);
}

#[gpui_kit::test]
fn intents_are_recorded_until_a_handler_performs_them(cx: &mut TestAppContext) {
    let store = store(cx);
    cx.update(|cx| dispatch(&store, cx, |m| m.copy_text("first")));
    cx.update(|cx| {
        let recorded = store.read(cx).recorded_intents().to_vec();
        assert!(matches!(recorded.as_slice(), [Intent::CopyText(text)] if text == "first"));
        // The model's outbox was drained.
        store.update(cx, |store, _| {
            assert_eq!(store.take_recorded_intents().len(), 1);
            assert!(store.recorded_intents().is_empty());
        });
    });

    let performed = Rc::new(RefCell::new(Vec::new()));
    cx.update(|cx| {
        let performed = performed.clone();
        store.update(cx, |store, _| {
            store.on_intent(move |intent, _| {
                if let Intent::CopyText(text) = intent {
                    performed.borrow_mut().push(text);
                }
            });
        });
        dispatch(&store, cx, |m| {
            m.copy_text("second");
            m.copy_text("third");
        });
    });
    assert_eq!(*performed.borrow(), ["second", "third"], "in order");
    cx.update(|cx| assert!(store.read(cx).recorded_intents().is_empty()));
}

#[gpui_kit::test]
fn a_tick_the_model_asked_for_polls_it_when_it_is_due(cx: &mut TestAppContext) {
    let store = store(cx);
    cx.update(|cx| dispatch(&store, cx, |m| m.request_tick(2.0)));
    cx.run_until_parked();
    let asked = cx.update(|cx| store.read(cx).revision());

    cx.executor().advance_clock(Duration::from_millis(1500));
    cx.run_until_parked();
    assert_eq!(
        cx.update(|cx| store.read(cx).revision()),
        asked,
        "not before it is due"
    );

    cx.executor().advance_clock(Duration::from_millis(600));
    cx.run_until_parked();
    cx.update(|cx| {
        let store = store.read(cx);
        assert!(store.revision() > asked, "the model was polled");
        // The poll saw the time the tick was asked for, whatever the wall
        // clock says.
        assert!(store.clock() >= 2.0);
    });
}

#[gpui_kit::test]
fn a_wake_from_the_core_polls_the_model(cx: &mut TestAppContext) {
    let store = store(cx);
    cx.update(|cx| store.update(cx, |store, cx| store.connect_core(cx)));
    cx.run_until_parked();
    let (waker, connected) = cx.update(|cx| {
        let store = store.read(cx);
        (
            store
                .model()
                .waker
                .clone()
                .expect("the model has the waker"),
            store.revision(),
        )
    });
    // A burst, as the core sends one.
    for _ in 0..50 {
        waker();
    }
    cx.run_until_parked();
    let after = cx.update(|cx| store.read(cx).revision());
    assert_eq!(after, connected + 1, "one poll for the burst");
    waker();
    cx.run_until_parked();
    assert_eq!(cx.update(|cx| store.read(cx).revision()), after + 1);
}
