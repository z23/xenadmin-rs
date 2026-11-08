# Inventory menus across rows and keyboard navigation

Right-clicking any visible inventory column or row padding opens the clicked object's menu. The Menu key and Shift+F10 open the same menu for the table cursor, including after virtual scrolling. Escape returns focus to the inventory table so arrow-key selection continues. Header cells and empty table space do not select an arbitrary object.

The adapter uses the pinned Rune runtime's presented-element picking within the inventory table; it does not maintain a second row-coordinate cache or replace the virtualized table. Keyboard positioning reads only the selected name cell. Existing single/multiple selection behavior remains: right-clicking an already selected row preserves the selection, while another row becomes the sole selection.

Menu targets capture pool, page and current connection observation. Every action rechecks the target's visible identity and observation; bulk power additionally requires the captured selection to remain unchanged. Copying a UUID follows the same validity check. VM-specific entries are absent on non-VM pages.

## Verification

Five Rune cases cover wide/compact row padding and non-name columns, Menu/Shift+F10, selection preservation, focus restoration, virtual scrolling, stale pool/row targets, changed bulk selection and header/blank-space behavior. The full desktop suite passed 275 tests. The final five focused tests, strict workspace/all-target Clippy, formatting and diff checks passed; release build succeeded. Wide (1440×900) and compact (513×652) native synthetic renders were inspected; their first pass revealed truncated labels, corrected by widening the menu before the final renders. Independent source review found no blocker. These tests do not establish physical keyboard or screen-reader behavior on Wayland/X11; those qualification gates remain open.

This completes the row/keyboard menu access gap in P02-12. Additional bulk property, delete, move and export workflows remain in scope.
