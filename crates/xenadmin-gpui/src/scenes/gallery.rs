//! The renderer's own fixtures: descriptions that use every piece of the
//! vocabulary, for the renderer tests and for looking at the renderer
//! before the areas that will use it exist.
//!
//! A [`Lab`] is a small state the descriptions are rebuilt from, as an
//! area's state is in the model. Its handlers change the lab and write what
//! happened to [`Lab::log`]; the store notifies after every handler, so the
//! next frame shows the new description. The table shows the guests of the
//! model's inventory and selects them in the model's grid, so the row menu
//! and the commands resolve as they will in the product.
//!
//! The gallery scenes are not product scenes: their names start with
//! `gallery-` and they are listed after the registered ones.

use std::cell::RefCell;
use std::rc::Rc;

use serde_json::json;
use xenadmin_ui::command::{self, CommandId};
use xenadmin_ui::object::{ObjectKind, ObjectRef, Target};
use xenadmin_ui::surface::{
    Block, NoticeKind, OutcomeState, Row, Surface, Tone, actions, alert, button, caption,
    checkbox, choice, column, columns, custom, divider, group, heading, input, notice, option,
    outcome, properties, property, review, row, secret_input, select, table, text, textarea,
};
use xenadmin_ui::tabs::TabBody;
use xenadmin_ui::{Model, SecretText, blocks, memory_label};

use crate::store::Describe;

/// The ids of the lab's surfaces.
pub const DIALOG: &str = "lab-dialog";
pub const WIZARD: &str = "lab-wizard";
pub const PROPERTIES: &str = "lab-properties";
pub const CONFIRM: &str = "lab-confirm";
/// The key of the lab's table.
pub const TABLE: &str = "lab-vms";

/// The steps of the lab's wizard.
pub const STEPS: [&str; 4] = ["Template", "Name", "Resources", "Review"];

/// What the tab body of the lab is.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum Body {
    /// Nothing provides the tab.
    None,
    /// A notice and the guests in a table with a single selection.
    Table,
    /// The same table with a multi selection.
    MultiTable,
    /// One of every block kind.
    Blocks,
    /// A bespoke element by name.
    Custom,
}

pub struct Lab {
    /// The open surfaces, back to front.
    pub open: Vec<&'static str>,
    pub name: String,
    pub notes: String,
    pub password: SecretText,
    pub start: bool,
    pub template: Option<String>,
    pub mode: String,
    pub view: String,
    pub storage: Option<String>,
    pub step: usize,
    pub page: String,
    pub selected: Option<String>,
    pub marked: Vec<String>,
    /// The column the table is ordered by, and whether descending.
    pub sort: (String, bool),
    /// The table has no rows.
    pub empty: bool,
    pub stale: bool,
    pub body: Body,
    /// The commands of the local action row.
    pub local: Vec<CommandId>,
    /// What the handlers did, oldest first.
    pub log: Vec<String>,
}

pub type LabRef = Rc<RefCell<Lab>>;

impl Lab {
    pub fn new() -> LabRef {
        Rc::new(RefCell::new(Self {
            open: Vec::new(),
            name: "web-01".into(),
            notes: String::new(),
            password: SecretText::new(),
            start: true,
            template: Some("debian".into()),
            mode: "full".into(),
            view: "list".into(),
            storage: None,
            step: 0,
            page: "general".into(),
            selected: None,
            marked: Vec::new(),
            sort: ("name".into(), false),
            empty: false,
            stale: false,
            body: Body::Table,
            local: vec![
                CommandId("nav.back"),
                CommandId("view.zoom-in"),
                CommandId("view.zoom-out"),
                CommandId("view.zoom-reset"),
            ],
            log: Vec::new(),
        }))
    }

    /// The descriptions of the lab in place of the model's.
    pub fn describe(lab: &LabRef) -> Describe {
        let (for_surfaces, for_body, for_local) = (lab.clone(), lab.clone(), lab.clone());
        Describe {
            surfaces: Rc::new(move |_| surfaces(&for_surfaces)),
            tab_body: Rc::new(move |m, _| tab_body(&for_body, m)),
            local_row: Rc::new(move |m, _| {
                let lab = for_local.borrow();
                if lab.body == Body::None {
                    return Vec::new();
                }
                command::resolve(m, &lab.local, &m.target())
            }),
        }
    }
}

fn log(lab: &LabRef, entry: impl Into<String>) {
    lab.borrow_mut().log.push(entry.into());
}

/// A handler that only writes to the log.
fn logged(lab: &LabRef, entry: &'static str) -> impl Fn(&mut Model) + 'static {
    let lab = lab.clone();
    move |_| log(&lab, entry)
}

fn close(lab: &LabRef, id: &'static str) -> impl Fn(&mut Model) + 'static {
    let lab = lab.clone();
    move |_| {
        let mut lab = lab.borrow_mut();
        lab.open.retain(|open| *open != id);
        lab.log.push(format!("close:{id}"));
    }
}

pub fn surfaces(lab: &LabRef) -> Vec<Surface> {
    let open = lab.borrow().open.clone();
    open.iter()
        .map(|id| match *id {
            WIZARD => wizard(lab),
            PROPERTIES => properties_dialog(lab),
            CONFIRM => confirm(lab),
            _ => dialog(lab),
        })
        .collect()
}

/// A form with every control, its review and the permission line.
fn dialog(lab: &LabRef) -> Surface {
    let state = lab.borrow();
    let name_error = state
        .name
        .trim()
        .is_empty()
        .then_some("Enter a name for the VM.");
    let (l1, l2, l3, l4, l5, l6, l7, l8) = (
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
    );
    let body = blocks![
        text("Copies the VM and its disks to the chosen storage repository."),
        input("lab-name", "Name", &state.name, move |_, value| {
            l1.borrow_mut().name = value.to_owned();
        })
        .placeholder("VM name")
        .help("Shown in the tree and in every list.")
        .error_opt(name_error)
        .on_submit(move |_| log(&l8, "submit:lab-name")),
        secret_input("lab-password", "Password", &state.password, move |_, value| {
            l2.borrow_mut().password.set(value);
        })
        .placeholder("Password of the remote pool"),
        textarea("lab-notes", "Description", &state.notes, move |_, value| {
            l3.borrow_mut().notes = value.to_owned();
        })
        .placeholder("What the copy is for"),
        select(
            "lab-template",
            "Template",
            [
                option("debian", "Debian 12"),
                option("alma", "AlmaLinux 9"),
                option("windows", "Windows Server 2022")
                    .disabled_if(true, "The template has no disk on shared storage."),
            ],
            state.template.as_deref(),
            move |_, id| l4.borrow_mut().template = Some(id.to_owned()),
        ),
        choice(
            "lab-mode",
            [
                option("full", "Full copy").detail("Independent disks. Takes longer."),
                option("fast", "Fast clone")
                    .detail("Shares unchanged blocks with the original."),
                option("linked", "Linked clone")
                    .disabled_if(true, "The storage repository cannot link disks."),
            ],
            Some(state.mode.as_str()),
            move |_, id| l5.borrow_mut().mode = id.to_owned(),
        )
        .label("Copy mode"),
        checkbox(
            "lab-start",
            "Start the VM after it is copied",
            state.start,
            move |_| {
                let mut lab = l6.borrow_mut();
                lab.start = !lab.start;
            }
        ),
        checkbox("lab-locked", "Keep the MAC addresses", true, |_| {})
            .disabled_if(true, "The pool policy keeps the addresses."),
        review(
            "lab-review",
            [
                format!("Copy web-01 to {}.", state.name),
                "The copy stays halted until it is started.".to_owned(),
            ],
        )
        .affected([
            property("VM", "web-01"),
            property("UUID", "6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13").copyable(),
            property("Storage", "Local storage on xcp-lab-01"),
        ])
        .warning("The original keeps running while its disks are copied.")
        .stale(state.stale.then_some(
            "The inventory changed after this review was prepared. Review it again."
        )),
    ];
    Surface::dialog(DIALOG, "Copy VM")
        .subtitle("web-01 • 6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13")
        .experimental(true)
        .body(body)
        .footer_note(caption(
            "Signed in as root. The role allows this operation.",
        ))
        .footer([
            button("lab-cancel", "Cancel", close(lab, DIALOG)),
            button("lab-copy", "Copy", move |_| log(&l7, "press:lab-copy"))
                .primary()
                .disabled_opt(name_error),
        ])
        .on_close(close(lab, DIALOG))
}

/// A wizard: the step decides the page and the footer.
fn wizard(lab: &LabRef) -> Surface {
    let state = lab.borrow();
    let step = state.step.min(STEPS.len() - 1);
    let (l1, l2, l3, l4, l5, l6) = (
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
    );
    let body = match step {
        0 => blocks![
            heading("Choose a template"),
            choice(
                "lab-template",
                [
                    option("debian", "Debian 12").detail("Linux, 2 vCPUs, 2 GiB"),
                    option("alma", "AlmaLinux 9").detail("Linux, 2 vCPUs, 4 GiB"),
                    option("windows", "Windows Server 2022").detail("Windows, 4 vCPUs, 8 GiB"),
                ],
                state.template.as_deref(),
                move |_, id| l1.borrow_mut().template = Some(id.to_owned()),
            ),
        ],
        1 => blocks![
            heading("Name the VM"),
            input("lab-name", "Name", &state.name, move |_, value| {
                l1.borrow_mut().name = value.to_owned();
            })
            .error_opt(
                state
                    .name
                    .trim()
                    .is_empty()
                    .then_some("Enter a name for the VM.")
            ),
            textarea("lab-notes", "Description", &state.notes, move |_, value| {
                l2.borrow_mut().notes = value.to_owned();
            }),
        ],
        2 => blocks![
            heading("Resources"),
            columns([
                blocks![
                    choice(
                        "lab-view",
                        [option("list", "2 vCPUs"), option("tree", "4 vCPUs")],
                        Some(state.view.as_str()),
                        move |_, id| l1.borrow_mut().view = id.to_owned(),
                    )
                    .label("Processors")
                    .segmented()
                ],
                blocks![
                    choice(
                        "lab-storage",
                        [
                            option("local", "Local storage"),
                            option("nfs", "NFS library"),
                        ],
                        state.storage.as_deref(),
                        move |_, id| l2.borrow_mut().storage = Some(id.to_owned()),
                    )
                    .label("Storage repository")
                    .dropdown()
                ],
            ]),
        ],
        _ => blocks![
            review(
                "lab-review",
                [
                    format!("Create the VM {}.", state.name),
                    "It is created halted.".to_owned(),
                ],
            )
            .affected([
                property("Template", state.template.clone().unwrap_or_default()),
                property("Storage", state.storage.clone().unwrap_or_else(|| "Local storage".into())),
            ]),
        ],
    };
    let last = step == STEPS.len() - 1;
    let invalid = step == 1 && state.name.trim().is_empty();
    let forward = if last {
        button("lab-create", "Create", move |_| log(&l3, "press:lab-create")).primary()
    } else {
        button("lab-next", "Next", move |_| {
            let mut lab = l3.borrow_mut();
            lab.step += 1;
        })
        .primary()
        .disabled_if(invalid, "Enter a name for the VM.")
    };
    let _ = (l5, l6);
    Surface::wizard(WIZARD, "New VM", STEPS, step)
        .body(body)
        .footer([
            button("lab-cancel", "Cancel", close(lab, WIZARD)),
            button("lab-back", "Back", move |_| {
                let mut lab = l4.borrow_mut();
                lab.step = lab.step.saturating_sub(1);
            })
            .disabled_if(step == 0, "This is the first step."),
            forward,
        ])
        .on_close(close(lab, WIZARD))
}

/// A properties dialog: a page list and the page it shows.
fn properties_dialog(lab: &LabRef) -> Surface {
    let state = lab.borrow();
    let (l1, l2, l3, l4, l5) = (
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
        lab.clone(),
    );
    let body = match state.page.as_str() {
        "cpu" => blocks![
            heading("Processors and memory"),
            choice(
                "lab-view",
                [option("list", "2 vCPUs"), option("tree", "4 vCPUs")],
                Some(state.view.as_str()),
                move |_, id| l1.borrow_mut().view = id.to_owned(),
            )
            .label("Processors")
            .segmented(),
            notice(
                NoticeKind::Info,
                "A change of the processor count takes effect at the next start."
            ),
        ],
        "advanced" => blocks![
            heading("Advanced"),
            properties((0..18).map(|index| property(
                format!("Platform key {index}"),
                format!("value {index}")
            ))),
        ],
        _ => blocks![
            heading("General"),
            input("lab-name", "Name", &state.name, move |_, value| {
                l1.borrow_mut().name = value.to_owned();
            }),
            textarea("lab-notes", "Description", &state.notes, move |_, value| {
                l2.borrow_mut().notes = value.to_owned();
            }),
            checkbox(
                "lab-start",
                "Start with the host",
                state.start,
                move |_| {
                    let mut lab = l3.borrow_mut();
                    lab.start = !lab.start;
                }
            ),
            properties([
                property("UUID", "6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13")
                    .key("lab-uuid")
                    .copyable(),
                property("Power state", "Running").tone(Tone::Success),
            ]),
        ],
    };
    Surface::properties(
        PROPERTIES,
        "Properties",
        [
            option("general", "General").detail("web-01"),
            option("cpu", "Processors and memory").detail("2 vCPUs, 2 GiB"),
            option("advanced", "Advanced"),
            option("ha", "High availability")
                .disabled_if(true, "The pool has no high availability."),
        ],
        state.page.clone(),
        move |_, page| l4.borrow_mut().page = page.to_owned(),
    )
    .subtitle("web-01")
    .body(body)
    .footer([
        button("lab-cancel", "Cancel", close(lab, PROPERTIES)),
        button("lab-apply", "Save", move |_| {
            let mut lab = l5.borrow_mut();
            lab.log.push("press:lab-apply".into());
            if !lab.open.contains(&CONFIRM) {
                lab.open.push(CONFIRM);
            }
        })
        .primary(),
    ])
    .on_close(close(lab, PROPERTIES))
}

/// A small confirmation in front of another surface.
fn confirm(lab: &LabRef) -> Surface {
    let confirmed = lab.clone();
    Surface::dialog(CONFIRM, "Save the changes?")
        .small()
        .body(blocks![
            text("The VM is running. The new name is shown at once; the rest waits for a restart."),
        ])
        .footer([
            button("lab-confirm-no", "Keep editing", close(lab, CONFIRM)),
            button("lab-confirm-yes", "Save", move |_| {
                let mut lab = confirmed.borrow_mut();
                lab.log.push("press:lab-confirm-yes".into());
                lab.open.clear();
            })
            .danger(),
        ])
        .on_close(close(lab, CONFIRM))
}

/// The guests of the model's inventory as rows, in the lab's order.
fn guest_rows(lab: &Lab, m: &Model) -> Vec<Row> {
    if lab.empty {
        return Vec::new();
    }
    let Some(inventory) = m.inventory.as_ref() else {
        return Vec::new();
    };
    let mut guests: Vec<_> = inventory.guests().collect();
    let (by, descending) = (&lab.sort.0, lab.sort.1);
    guests.sort_by(|left, right| {
        let order = match by.as_str() {
            "memory" => left.memory_bytes.cmp(&right.memory_bytes),
            "vcpus" => left.vcpus.cmp(&right.vcpus),
            _ => xenadmin_ui::natural_cmp(&left.name, &right.name),
        };
        let order = order.then_with(|| left.id.to_string().cmp(&right.id.to_string()));
        if descending { order.reverse() } else { order }
    });
    guests
        .into_iter()
        .map(|vm| {
            let state = format!("{:?}", vm.power_state);
            let tone = if state == "Running" {
                Tone::Normal
            } else {
                Tone::Muted
            };
            row(
                vm.id,
                [
                    vm.name.clone(),
                    state,
                    vm.vcpus.to_string(),
                    memory_label(vm.memory_bytes),
                ],
            )
            .tone(tone)
        })
        .collect()
}

fn grid_target(m: &Model, rows: &[String]) -> Target {
    let pool = m.inventory.as_ref().map(|inventory| inventory.pool_id);
    Target::many(
        rows.iter()
            .map(|id| ObjectRef::new(pool, ObjectKind::Vm, id)),
    )
}

fn guests_table(lab: &LabRef, m: &Model, multi: bool) -> Block {
    let state = lab.borrow();
    let (l1, l2, l3) = (lab.clone(), lab.clone(), lab.clone());
    let base = table(
        TABLE,
        [
            column("name", "Name").sortable(),
            column("state", "Power state").width(140),
            column("vcpus", "vCPUs").width(90).right().sortable(),
            column("memory", "Memory").width(120).right().sortable(),
        ],
        guest_rows(&state, m),
    )
    .sorted(state.sort.0.clone(), state.sort.1, move |_, column, descending| {
        let mut lab = l1.borrow_mut();
        lab.sort = (column.to_owned(), descending);
        lab.log.push(format!("sort:{column}:{descending}"));
    })
    .on_activate(move |_, id| log(&l2, format!("activate:{id}")))
    .row_menu([CommandId("object.copy-uuid")])
    .empty("No virtual machines match.");
    let selecting = if multi {
        base.multi(state.marked.clone(), move |m, rows| {
            let mut lab = l3.borrow_mut();
            lab.marked = rows.to_vec();
            lab.log.push(format!("select:{}", rows.len()));
            m.select_in_grid(grid_target(m, rows));
        })
    } else {
        base.single(state.selected.as_deref(), move |m, id| {
            let mut lab = l3.borrow_mut();
            lab.selected = Some(id.to_owned());
            lab.log.push(format!("select:{id}"));
            m.select_in_grid(grid_target(m, &[id.to_owned()]));
        })
    };
    Block::Table(selecting)
}

/// One of every block kind.
fn every_block(lab: &LabRef) -> Vec<Block> {
    let state = lab.borrow();
    let (l1, l2, l3) = (lab.clone(), lab.clone(), lab.clone());
    let outcome_of = |key: &'static str, state: OutcomeState, title: &str| {
        outcome(key, state, title).line("Started 14:02 on xcp-lab-01.")
    };
    blocks![
        heading("Every block"),
        text("A paragraph in the normal tone."),
        caption("A caption in the muted tone.").key("lab-caption"),
        text("A success text.").tone(Tone::Success),
        text("A warning text.").tone(Tone::Warning),
        alert("An alert: the host did not answer.").key("lab-alert"),
        notice(NoticeKind::Info, "An information notice.").key("lab-info"),
        notice(NoticeKind::Warning, "The pool has one host without a current update.")
            .title("Update pending")
            .key("lab-warning"),
        notice(NoticeKind::Danger, "The storage repository is detached.").key("lab-danger"),
        notice(NoticeKind::Preview, "Synthetic data. No host is connected.").key("lab-preview"),
        divider(),
        columns([
            blocks![
                input("lab-name", "Name", &state.name, move |_, value| {
                    l1.borrow_mut().name = value.to_owned();
                }),
                checkbox(
                    "lab-start",
                    "Start with the host",
                    state.start,
                    move |_| {
                        let mut lab = l2.borrow_mut();
                        lab.start = !lab.start;
                    }
                ),
            ],
            blocks![properties([
                property("UUID", "6f1c0a52-8d3e-4b71-9c5a-2e7d0f4b9a13")
                    .key("lab-uuid")
                    .copyable(),
                property("Power state", "Running").tone(Tone::Success),
                property("Tools", "Not installed").tone(Tone::Warning),
            ])],
        ]),
        group(
            "Operations",
            blocks![
                outcome_of("lab-running", OutcomeState::Running(Some(0.4)), "Copying web-01")
                    .action(button("lab-cancel-task", "Cancel", logged(lab, "press:lab-cancel-task"))),
                outcome_of("lab-waiting", OutcomeState::Running(None), "Waiting for the host"),
                outcome_of("lab-succeeded", OutcomeState::Succeeded, "web-01 was copied"),
                outcome_of("lab-failed", OutcomeState::Failed, "The copy failed")
                    .line("SR_FULL: the storage repository has no room."),
                outcome_of("lab-partial", OutcomeState::Partial, "Two of three disks were copied"),
                outcome_of("lab-unknown", OutcomeState::Unknown, "The outcome is unknown")
                    .line("The connection was lost before the host answered.")
                    .action(button("lab-recheck", "Recheck", logged(lab, "press:lab-recheck")).primary())
                    .action(button(
                        "lab-acknowledge",
                        "Acknowledge",
                        logged(lab, "press:lab-acknowledge")
                    )),
                outcome_of("lab-cancelled", OutcomeState::Cancelled, "The copy was cancelled"),
            ]
        )
        .key("lab-operations"),
        group(
            "Folded section",
            blocks![text("Shown when the section is opened.").key("lab-folded-text")]
        )
        .key("lab-folded")
        .collapsed(),
        actions([
            button("lab-primary", "Primary", logged(lab, "press:lab-primary")).primary(),
            button("lab-secondary", "Secondary", logged(lab, "press:lab-secondary")),
            button("lab-danger-button", "Danger", logged(lab, "press:lab-danger-button")).danger(),
            button("lab-ghost", "Ghost", logged(lab, "press:lab-ghost")).ghost(),
            button("lab-disabled", "Disabled", move |_| log(&l3, "press:lab-disabled"))
                .disabled_if(true, "Demo mode cannot change infrastructure."),
        ]),
        custom("lab-missing", json!({ "series": 2 })).key("lab-custom"),
    ]
}

pub fn tab_body(lab: &LabRef, m: &Model) -> Option<TabBody> {
    let body = lab.borrow().body;
    match body {
        Body::None => None,
        Body::Table | Body::MultiTable => Some(TabBody::Blocks(blocks![
            notice(
                NoticeKind::Preview,
                "Synthetic inventory. No host is connected."
            )
            .key("lab-preview"),
            guests_table(lab, m, body == Body::MultiTable),
        ])),
        Body::Blocks => Some(TabBody::Blocks(every_block(lab))),
        Body::Custom => Some(TabBody::Custom(
            custom("lab-chart", json!({ "metric": "cpu" })).key("lab-chart"),
        )),
    }
}

/// The names of the gallery scenes.
pub const SCENES: [&str; 7] = [
    "gallery-dialog",
    "gallery-wizard",
    "gallery-properties",
    "gallery-stacked",
    "gallery-tab",
    "gallery-multi",
    "gallery-blocks",
];

/// Sets the lab up for a gallery scene. False when there is none of that
/// name.
pub fn apply(name: &str, lab: &LabRef) -> bool {
    let mut lab = lab.borrow_mut();
    match name {
        "gallery-dialog" => lab.open = vec![DIALOG],
        "gallery-wizard" => {
            lab.open = vec![WIZARD];
            lab.step = 1;
        }
        "gallery-properties" => lab.open = vec![PROPERTIES],
        "gallery-stacked" => lab.open = vec![PROPERTIES, CONFIRM],
        "gallery-tab" => lab.body = Body::Table,
        "gallery-multi" => lab.body = Body::MultiTable,
        "gallery-blocks" => lab.body = Body::Blocks,
        _ => return false,
    }
    true
}
