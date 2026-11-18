//! The pages, described. Nothing here changes the model.
//!
//! The Rune dialog was one scrolling column: every field, or the review. The
//! fields are on six pages of the plan now and the review is a seventh; the
//! texts are the Rune ones, each on the page of the fields it is about.
//!
//! Keys kept from Rune: `property-name`, `property-secure-boot`,
//! `refresh-boot-readiness`, `review-properties`, `apply-properties`, and,
//! on the options that were buttons with a tick, `property-firmware-bios`,
//! `property-firmware-uefi`, `property-home-host-none` and
//! `property-home-host-<host id>`. The Rune key `properties-dialog` was the
//! dialog itself: that is the kernel's `vm-properties` or
//! `template-properties`.
//!
//! New keys, for what Rune left without one: the inputs
//! `property-description`, `property-max-cpus`, `property-startup-cpus`,
//! `property-live-cpus`, `property-static-min`, `property-dynamic-min`,
//! `property-dynamic-max`, `property-static-max`, `property-boot-order`,
//! `property-cores-per-socket`, `property-cpu-weight`,
//! `property-shadow-multiplier`; the checkbox `property-autostart`; the two
//! choices `property-firmware` and `property-home-host`; the review
//! `properties-review` (so `properties-review-stale` is its stale line); the
//! texts `properties-nothing`, `properties-not-current`, `properties-preview`
//! and `properties-error`; the buttons `properties-cancel` and
//! `properties-back`.

use xenadmin_core::{PermissionAssessment, PowerState};

use super::change::{self, Page};
use super::draft::{Editor, Field as Slot, describe};
use crate::prelude::*;

/// The key of the review.
pub const REVIEW: &str = "properties-review";

/// A page of the plan on which this VM, as it was captured, has nothing to
/// change (Rune showed no field and said nothing).
pub const NOTHING: &str = "Nothing on this page can be changed for this object as it is now.";

const HALTED_ONLY: &str = "Static memory limits, maximum vCPUs and boot order require a halted VM.";
const PREVIEW: &str = "Demo preview • no host changes can be submitted.";
const NO_DEMO_HOST: &str = "Demo preview: there is no host to read certificate readiness from.";

/// The pages this area has for a target: all seven for one VM or one
/// template its pool's inventory has, none for anything else. Several VMs or
/// templates are the `bulk_edit` area's. Every page is outside the supported
/// core, as the Rune dialog was. A page says for itself what cannot be
/// changed, so the list does not move while the dialog is open.
pub fn pages(m: &Model, target: &Target) -> Vec<PropertyPage> {
    if change::known(m, target).is_none() {
        return Vec::new();
    }
    let Some(kind) = PropertiesKind::of(target) else {
        return Vec::new();
    };
    Page::ALL
        .map(|page| {
            match page {
                Page::Review => {
                    PropertyPage::new(page.id(), change::REVIEW_LABEL, change::REVIEW_ORDER)
                }
                _ => PropertyPage::planned(kind, page.id()),
            }
            .experimental(true)
        })
        .to_vec()
}

/// The page that is showing.
pub fn page(m: &Model, _target: &Target, id: &str) -> PageView {
    let (Some(w), Some(page)) = (m.areas.vm_properties.editor.as_ref(), Page::from_id(id)) else {
        return PageView::default();
    };
    let permission = w
        .review
        .as_ref()
        .filter(|_| page == Page::Review)
        .map(|patch| permissions::assess_for(m, w.pool, &patch.permission_requirements()));
    PageView {
        body: blocks![body(m, w, page), tail(m, w, page)],
        footer: footer(m, w, page, permission.as_ref()),
        footer_note: permission.as_ref().map(permissions::footer_note),
    }
}

fn body(m: &Model, w: &Editor, page: Page) -> Vec<Block> {
    let found = match page {
        Page::General => general(w),
        Page::CpuMemory => cpu_memory(w),
        Page::BootOptions => boot_options(w),
        Page::StartOptions => start_options(w),
        Page::HomeServer => home_server(w),
        Page::Advanced => advanced(m, w),
        Page::Review => return reviewing(m, w),
    };
    if found.is_empty() {
        blocks![caption(NOTHING).key("properties-nothing")]
    } else {
        found
    }
}

/// One text of the draft. Rune labelled each input twice (a caption and the
/// input's own label) with the same words; here it is the field's label.
fn field(w: &Editor, slot: Slot, key: &'static str, label: &str) -> Field {
    input(key, label, w.field(slot), move |m, text| {
        if let Some(w) = &mut m.areas.vm_properties.editor {
            *w.field_mut(slot) = text.to_owned();
        }
    })
}

fn halted(w: &Editor) -> bool {
    w.original.power_state == PowerState::Halted
}

fn running(w: &Editor) -> bool {
    w.original.power_state == PowerState::Running
}

fn general(w: &Editor) -> Vec<Block> {
    blocks![
        w.original.is_template.then(|| caption(
            "Custom template properties affect future VMs created from this template. Built-in templates are read-only; copy one first."
        )),
        field(w, Slot::Name, "property-name", "Name"),
        field(w, Slot::Description, "property-description", "Description"),
    ]
}

fn cpu_memory(w: &Editor) -> Vec<Block> {
    let cpus = w.details.vcpus_max.is_some() && w.details.vcpus_at_startup.is_some();
    let memory = w.memory().is_some();
    let hvm = match w.details.domain_type.as_deref() {
        Some("hvm") => true,
        None | Some("unspecified") => w.details.boot_policy.as_ref().is_some_and(|p| !p.is_empty()),
        _ => false,
    };
    blocks![
        (halted(w) && cpus).then(|| blocks![
            field(w, Slot::MaxCpus, "property-max-cpus", "Maximum vCPUs"),
            field(w, Slot::StartupCpus, "property-startup-cpus", "Startup vCPUs"),
        ]),
        (halted(w) && w.details.topology_known).then(|| field(
            w,
            Slot::CpuTopology,
            "property-cores-per-socket",
            "Cores per socket"
        )),
        (halted(w) && memory).then(|| blocks![
            caption("Memory accepts whole bytes, KiB, MiB or GiB. Minimum ≤ dynamic range ≤ maximum."),
            field(w, Slot::StaticMin, "property-static-min", "Static minimum"),
            field(w, Slot::DynamicMin, "property-dynamic-min", "Dynamic minimum"),
            field(w, Slot::DynamicMax, "property-dynamic-max", "Dynamic maximum"),
            field(w, Slot::StaticMax, "property-static-max", "Static maximum"),
        ]),
        running(w).then(|| blocks![
            caption(
                "Live changes apply to a running guest. vCPUs can only be increased within its current maximum. Ballooning requires compatible guest tools and may reduce the memory available to applications."
            ),
            // The field Rune reused: the startup count of a running guest is
            // its live count.
            cpus.then(|| field(w, Slot::StartupCpus, "property-live-cpus", "Live vCPUs")),
            memory.then(|| blocks![
                field(w, Slot::DynamicMin, "property-dynamic-min", "Dynamic memory minimum"),
                field(w, Slot::DynamicMax, "property-dynamic-max", "Dynamic memory maximum"),
            ]),
            caption(HALTED_ONLY),
        ]),
        (halted(w) || running(w)).then(|| blocks![
            w.details.cpu_weight_known.then(|| field(
                w,
                Slot::CpuWeight,
                "property-cpu-weight",
                "CPU scheduling weight (1–65535)"
            )),
            (hvm && w.details.shadow_multiplier.is_some()).then(|| field(
                w,
                Slot::ShadowMultiplier,
                "property-shadow-multiplier",
                "Shadow memory multiplier (1–1000)"
            )),
        ]),
    ]
}

fn boot_options(w: &Editor) -> Vec<Block> {
    blocks![
        (halted(w) && w.details.boot_order.is_some()).then(|| field(
            w,
            Slot::BootOrder,
            "property-boot-order",
            "Boot order (c=disk, d=DVD, n=network)"
        )),
        running(w).then(|| caption(HALTED_ONLY)),
    ]
}

fn start_options(w: &Editor) -> Vec<Block> {
    blocks![w.details.auto_power_on.is_some().then(|| checkbox(
        "property-autostart",
        "Start automatically with the pool",
        w.autostart,
        |m| {
            if let Some(w) = &mut m.areas.vm_properties.editor {
                w.autostart = !w.autostart;
            }
        }
    ))]
}

fn home_server(w: &Editor) -> Vec<Block> {
    if !w.details.affinity_known {
        return blocks![
            heading("Home host"),
            caption(
                "The current home-host preference was not reported. Refresh inventory before editing it."
            ),
        ];
    }
    let none = option("none", "No preference").key("property-home-host-none");
    let hosts = w.hosts.iter().map(|(id, name)| {
        option(id.to_string(), format!("{name} · {id}")).key(format!("property-home-host-{id}"))
    });
    let selected = w
        .home_host
        .map_or_else(|| "none".to_owned(), |id| id.to_string());
    blocks![
        heading("Home host"),
        caption(
            "Preferred host for future starts. The pool can use another host if resources require it; changing this does not migrate the VM."
        ),
        choice(
            "property-home-host",
            std::iter::once(none).chain(hosts),
            Some(&selected),
            |m, id| {
                if let Some(w) = &mut m.areas.vm_properties.editor {
                    // An id that is neither "none" nor a listed host changes
                    // nothing.
                    if id == "none" {
                        w.home_host = None;
                    } else if let Some((host, _)) =
                        w.hosts.iter().find(|(host, _)| host.to_string() == id)
                    {
                        w.home_host = Some(*host);
                    }
                }
            }
        ),
    ]
}

/// "Firmware and Secure Boot": what the host reports, the readiness of its
/// certificates, and the two settings where they can be changed.
fn advanced(m: &Model, w: &Editor) -> Vec<Block> {
    let current = change::current(m, w);
    let readiness = current.then(|| {
        match w.boot_query.as_ref().and_then(|q| q.latest()) {
            Some(Ok(status)) => format!(
                "Guest readiness: {}\nPool defaults: {}",
                status.guest.explanation(),
                status.pool.explanation()
            ),
            Some(Err(error)) => error,
            None if w.boot_query.is_some() => "Reading certificate readiness…".into(),
            None => "Certificate readiness has not been queried.".into(),
        }
    });
    let firmware_choice = w.empty_template
        && (w.details.domain_type.as_deref() == Some("hvm")
            || w.details.boot_policy.as_deref() == Some("BIOS order"))
        && w.details.secure_boot == Some(false);
    let secure_boot =
        halted(w) && w.firmware.as_deref() == Some("uefi") && w.details.secure_boot.is_some();
    blocks![
        heading("Firmware and Secure Boot"),
        caption(format!(
            "Firmware: {} · Secure Boot policy: {}",
            w.details.firmware.as_deref().unwrap_or("unavailable"),
            w.details
                .secure_boot
                .map_or("unavailable", |v| if v { "on" } else { "off" })
        )),
        readiness.map(caption),
        w.boot_error.clone().map(caption),
        caption(
            "Readiness is sampled at the last check and checks certificate presence. Verify Secure Boot enforcement inside the guest. This editor does not replace certificates or reset guest NVRAM. Shut down the guest before changing its Secure Boot policy."
        ),
        actions([
            button("refresh-boot-readiness", "Refresh boot readiness", change::refresh_boot)
                .disabled_if(!current, change::NOT_CURRENT)
                .disabled_if(change::synthetic(m, w), NO_DEMO_HOST)
        ]),
        firmware_choice.then(|| blocks![
            choice(
                "property-firmware",
                ["bios", "uefi"].map(|value| {
                    option(value, format!("{} firmware", value.to_uppercase()))
                        .key(format!("property-firmware-{value}"))
                }),
                w.firmware.as_deref(),
                |m, value| {
                    if let Some(e) = &mut m.areas.vm_properties.editor
                        && matches!(value, "bios" | "uefi")
                    {
                        e.firmware = Some(value.into());
                        if value == "bios" {
                            e.secure_boot = false;
                        }
                    }
                }
            )
            .segmented(),
            caption(
                "Only empty custom templates can change BIOS/UEFI firmware. Attach no installation disks until the choice is saved."
            ),
        ]),
        secure_boot.then(|| checkbox(
            "property-secure-boot",
            "Request Secure Boot policy",
            w.secure_boot,
            |m| {
                if let Some(e) = &mut m.areas.vm_properties.editor {
                    e.secure_boot = !e.secure_boot;
                }
            }
        )),
    ]
}

/// The review: one line for each edit of the one patch, in the order they
/// are applied. Without a review (the page was chosen from the list and the
/// draft cannot be reviewed) the page has only the reason, in the tail.
fn reviewing(m: &Model, w: &Editor) -> Vec<Block> {
    let Some(patch) = &w.review else {
        return Vec::new();
    };
    blocks![
        heading(if w.original.is_template {
            "Review template property changes"
        } else {
            "Review VM property changes"
        }),
        review(
            REVIEW,
            patch.edits().iter().map(|edit| describe(edit, &w.hosts))
        )
        .stale(change::stale(m, w)),
        caption(
            "Changes apply in steps. A failure stops remaining steps; already saved values remain applied. Changed values are checked on the host before and after each step."
        ),
    ]
}

/// What Rune put under the fields and under the review alike.
fn tail(m: &Model, w: &Editor, page: Page) -> Vec<Block> {
    let reviewed = page == Page::Review && w.review.is_some();
    blocks![
        // On the review the stale line of the review says it.
        (!reviewed && !change::current(m, w)).then(|| caption(change::NOT_CURRENT)
            .tone(Tone::Warning)
            .key("properties-not-current")),
        change::synthetic(m, w)
            .then(|| notice(NoticeKind::Preview, PREVIEW).key("properties-preview")),
        w.error
            .clone()
            .map(|error| alert(error).key("properties-error")),
    ]
}

fn footer(
    m: &Model,
    w: &Editor,
    page: Page,
    permission: Option<&PermissionAssessment>,
) -> Vec<Button> {
    let cancel = button("properties-cancel", "Cancel", change::cancel);
    if page != Page::Review {
        return vec![
            cancel,
            button("review-properties", "Review changes", change::review)
                .primary()
                .disabled_opt(change::review_blocked(m, w)),
        ];
    }
    let apply = button("apply-properties", "Apply changes", change::submit)
        .primary()
        .disabled_opt(
            w.review
                .is_none()
                .then(|| w.error.clone().unwrap_or_else(|| "No properties have changed".into())),
        )
        // The first reason wins: stale, not writable, a known denial, the demo.
        .disabled_opt(change::apply_blocked(m, w))
        .disabled_opt(permission.and_then(permissions::denial))
        .disabled_if(change::synthetic(m, w), change::DEMO);
    vec![
        cancel,
        button("properties-back", "Back", change::back),
        apply,
    ]
}
