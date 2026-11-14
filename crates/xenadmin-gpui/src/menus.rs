//! Stub command rows for the shell spike.
//!
//! This is not the product command table. Phase 1 owns that. These three
//! rows exist so the menu bar can show a disabled item and a checked item
//! that both follow the selection.

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct StubItem {
    pub label: &'static str,
    pub disabled: bool,
    pub checked: bool,
}

/// File-menu stand-in. Import is disabled until a VM is selected. The toolbar
/// item stays checked while the selection is a VM.
pub fn stub_menus(vm_selected: bool) -> [StubItem; 3] {
    [
        StubItem {
            label: "Import",
            disabled: !vm_selected,
            checked: false,
        },
        StubItem {
            label: "Export",
            disabled: false,
            checked: false,
        },
        StubItem {
            label: "Toolbar",
            disabled: false,
            checked: vm_selected,
        },
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn eligibility_follows_the_selection() {
        let idle = stub_menus(false);
        assert!(idle[0].disabled);
        assert!(!idle[2].checked);
        let selected = stub_menus(true);
        assert!(!selected[0].disabled);
        assert!(selected[2].checked);
        assert_eq!(selected[1].label, "Export");
    }
}
