use super::*;
use std::sync::Arc;
fn state() -> State {
    let mut s = crate::init();
    scene(&mut s);
    let vm = s.devices.as_ref().unwrap().vm;
    let i = Arc::make_mut(s.inventory.as_mut().unwrap());
    i.vms.get_mut(&vm).unwrap().power_state = xenadmin_core::PowerState::Halted;
    for v in i.details.vbds.values_mut().filter(|v| v.vm == Some(vm)) {
        v.currently_attached = Some(false);
    }
    for v in i.details.vifs.values_mut().filter(|v| v.vm == Some(vm)) {
        v.currently_attached = Some(false);
    }
    s
}
fn runtime(mut s: State, size: (f32, f32)) -> rune::Runtime<State> {
    s.window_width = size.0;
    s.window_height = size.1;
    let mut r = rune::Runtime::new(
        s,
        crate::view,
        size,
        rune::Env {
            system_dark: true,
            scale: 1.0,
        },
    );
    settle(&mut r);
    r
}
fn settle(r: &mut rune::Runtime<State>) {
    for _ in 0..30 {
        r.frame(1.0 / 60.0);
    }
}
fn click(r: &mut rune::Runtime<State>, key: &str) {
    r.update(|_, tx| tx.scroll_into_view(key));
    settle(r);
    let (x, y) = r.find(key).unwrap_or_else(|| panic!("missing {key}"));
    r.pointer_down(x, y);
    r.pointer_up(x, y);
    r.frame(0.0);
    settle(r);
}
fn form(s: &State) -> &Form {
    s.devices.as_ref().unwrap().properties.as_ref().unwrap()
}
#[test]
fn priority_review_keeps_device_identity_at_both_widths_and_cannot_submit_synthetic_changes() {
    for size in [(1440.0, 900.0), (513.0, 652.0)] {
        let mut r = runtime(state(), size);
        let original = form(&r.state).choice.clone().unwrap();
        form_mut(&mut r.state).unwrap().value = TextEdit::new("7");
        r.frame(0.0);
        click(&mut r, "property-review");
        let reviewed = form(&r.state).review.clone().unwrap();
        assert!(matches!(
            reviewed,
            Change::DiskPriority {
                priority: Some(7),
                ..
            }
        ));
        if let (
            Change::DiskPriority { vbd: a, vdi: b, .. },
            Change::DiskPriority { vbd: c, vdi: d, .. },
        ) = (original, &reviewed)
        {
            assert_eq!((a, b), (*c, *d));
        } else {
            panic!("wrong action")
        }
        let vm = form(&r.state).vm;
        r.state.table.select("30000000-0000-0000-0000-000000000001");
        r.frame(0.0);
        assert_eq!(form(&r.state).vm, vm);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("0 highest, 7 lowest")
        );
        click(&mut r, "property-apply");
        assert!(r.state.operations.is_empty());
        assert_eq!(form(&r.state).review.as_ref(), Some(&reviewed));
        r.update(|s, _| s.demo = false);
        r.frame(0.0);
        assert!(
            r.accessibility_json()
                .to_string()
                .contains("connection or VM changed")
        );
        apply(&mut r.state);
        assert!(r.state.operations.is_empty());
    }
}
#[test]
fn nic_rate_review_has_correct_units_and_preserves_captured_qos() {
    let mut s = state();
    let vm = s.devices.as_ref().unwrap().vm;
    let nic = s
        .inventory
        .as_ref()
        .unwrap()
        .details
        .vifs
        .values()
        .find(|v| v.vm == Some(vm))
        .unwrap()
        .id;
    close(&mut s);
    let mut r = runtime(s, (513.0, 652.0));
    click(&mut r, &format!("nic-properties-{nic}"));
    click(&mut r, "property-enabled");
    form_mut(&mut r.state).unwrap().value = TextEdit::new("12500");
    r.frame(0.0);
    click(&mut r, "property-review");
    assert!(
        matches!(form(&r.state).review, Some(Change::NicRate { vif, kilobytes_per_second: Some(12500), .. }) if vif == nic)
    );
    assert!(
        r.accessibility_json()
            .to_string()
            .contains("kilobytes per second")
    );
    click(&mut r, "property-back");
    click(&mut r, "property-enabled");
    click(&mut r, "property-review");
    assert!(form(&r.state).review.is_none());
    assert!(
        form(&r.state)
            .error
            .as_ref()
            .unwrap()
            .contains("Change the rate limit")
    );
}
#[test]
fn metadata_cbt_confirmation_and_invalid_values_require_explicit_valid_review() {
    let mut s = state();
    let choices = form(&s).choices.clone();
    let metadata = choices
        .iter()
        .find(|c| matches!(c, Change::DiskMetadata { .. }))
        .unwrap()
        .clone();
    choose(&mut s, metadata);
    assert!(build(form(&s)).is_err());
    let f = form_mut(&mut s).unwrap();
    f.name = TextEdit::new("Database data");
    f.description = TextEdit::new("Reviewed description");
    review(&mut s);
    assert!(matches!(form(&s).review, Some(Change::DiskMetadata { .. })));
    let (vbd, vdi) = match &choices[0] {
        Change::DiskMetadata { vbd, vdi, .. } => (*vbd, *vdi),
        _ => unreachable!(),
    };
    choose(&mut s, Change::DisableCbt { vbd, vdi });
    assert!(build(form(&s)).unwrap_err().contains("exact disk UUID"));
    form_mut(&mut s).unwrap().value = TextEdit::new(vdi.to_string());
    assert!(build(form(&s)).is_ok());
    assert!(summary(&build(form(&s)).unwrap()).contains("incremental backup chain"));
    let priority = choices
        .iter()
        .find(|c| matches!(c, Change::DiskPriority { .. }))
        .unwrap()
        .clone();
    choose(&mut s, priority);
    for invalid in ["8", "-1", "", "1.5"] {
        form_mut(&mut s).unwrap().value = TextEdit::new(invalid);
        assert!(build(form(&s)).is_err());
    }
    choose(
        &mut s,
        Change::DiskPosition {
            vbd,
            vdi,
            previous: "0".into(),
            value: String::new(),
        },
    );
    for invalid in ["0", "256", "01", ""] {
        form_mut(&mut s).unwrap().value = TextEdit::new(invalid);
        assert!(build(form(&s)).is_err());
    }
}
