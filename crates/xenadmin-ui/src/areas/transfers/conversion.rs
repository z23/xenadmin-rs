//! Offline conversion of a local disk image (Rune
//! `local_conversion_view.rs`): one background job at a time, a review
//! after inspection, a local history with paging and cleanup, and the
//! hand-over of a completed image to the import wizard. Needs no
//! connection, and no hook closes it.
//!
//! The logic is the Rune module's, word for word. The description is a
//! wizard: "Source and format", "Review" (after the inspection), "Convert"
//! (progress, cancel, result). The progress line, the error, the local
//! history and the cleanup review are under every step, as in Rune. A job
//! keeps running while the wizard is hidden and is shown again when it is
//! reopened.
//!
//! Rune keys kept: `conversion-tool`, `conversion-format-{label}`,
//! `conversion-source`, `conversion-destination`, `conversion-cleanup-{id}`,
//! `conversion-cleanup-confirm`, `conversion-close`, `conversion-cancel`,
//! `conversion-submit`, `conversion-prepare`, `conversion-import`.
//! `conversion-heading` was the Rune heading: the surface's title has its
//! text. New keys: the choices `conversion-format` and `conversion-output`
//! with the options `conversion-output-{raw|vhd}`, the texts
//! `conversion-source-path`, `conversion-destination-path`,
//! `conversion-review`, `conversion-progress`, `conversion-error`,
//! `conversion-preview`, the buttons `conversion-discard`,
//! `conversion-restart`, `conversion-history-previous`,
//! `conversion-history-next`.

use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, mpsc};

use xenadmin_core::{
    ConversionFormat as Format, ConversionRequest, ConversionReview, ConversionState,
    ConversionSummary, ConversionTools, DiskImageFormat, LocalConversions,
};

use crate::prelude::*;

pub const SURFACE: &str = "local-conversion";

const INTRO: &str = "Convert an offline local image. The original is preserved. No host connection is required and no image is imported automatically.";
const FORM_HELP: &str = "Only self-contained images are supported. Inspection first makes a private source copy; allow space for that copy and the converted disk. QEMU and Bubblewrap are optional local dependencies. Missing sandbox support refuses conversion; no software is installed automatically.";
const REVIEW_HELP: &str = "The private staged source is read-only. Conversion and complete content comparison run in a restricted local sandbox. Existing files will not be overwritten. Internal snapshots, backing chains, external extents and encrypted images are refused. This converts disk content only; guest boot and driver compatibility remain unverified.";
pub const PREVIEW: &str = "Synthetic preview · local files and tools will not be accessed";
pub const STOPPED: &str = "Local worker stopped; reopen Convert Disk Image and inspect interrupted files under Local history. Nothing will resume automatically.";
/// New texts: Rune disabled these controls without saying why.
pub const BUSY: &str = "Local work is running.";
pub const REVIEWING: &str = "Discard the review to change the conversion.";
pub const NO_SOURCE: &str = "Choose a source image first.";
pub const NO_OUTPUT: &str = "Choose a new output file first.";
pub const NO_POOL: &str = "Connect to a pool to import the converted disk.";

const FORMATS: [Format; 5] = [
    Format::Qcow2,
    Format::Vmdk,
    Format::Vhdx,
    Format::Vhd,
    Format::Raw,
];

type Wake = Option<Arc<dyn Fn() + Send + Sync>>;

pub(super) enum Event {
    Progress(ConversionSummary),
    Prepared(Box<Result<ConversionReview, String>>),
    Finished(Result<ConversionSummary, String>),
    History(Result<Vec<ConversionSummary>, String>),
    Abandoned(Result<(), String>),
}

pub(super) struct Job {
    pub(super) receiver: mpsc::Receiver<Event>,
    pub(super) cancel: Arc<AtomicBool>,
}

pub struct Workspace {
    pub show: bool,
    pub(super) epoch: u64,
    pub(super) source: Option<PathBuf>,
    pub(super) destination: Option<PathBuf>,
    pub(super) tool: String,
    pub(super) format: Format,
    pub(super) output: DiskImageFormat,
    pub(super) review: Option<ConversionReview>,
    pub(super) job: Option<Job>,
    pub(super) latest: Option<ConversionSummary>,
    pub(super) history: Vec<ConversionSummary>,
    pub(super) history_page: usize,
    pub(super) error: Option<String>,
    pub(super) completed: Option<(PathBuf, DiskImageFormat)>,
    pub(super) preview: bool,
    pub(super) preview_review: bool,
    pub(super) cleanup: Option<uuid::Uuid>,
    /// The reviewed conversion was started: the wizard shows its last
    /// step until "Convert another image".
    pub(super) converting: bool,
}

impl Default for Workspace {
    fn default() -> Self {
        Self {
            show: false,
            epoch: 0,
            source: None,
            destination: None,
            tool: "/usr/bin/qemu-img".into(),
            format: Format::Qcow2,
            output: DiskImageFormat::Raw,
            review: None,
            job: None,
            latest: None,
            history: vec![],
            history_page: 0,
            error: None,
            completed: None,
            preview: false,
            preview_review: false,
            cleanup: None,
            converting: false,
        }
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        if let Some(job) = &self.job {
            job.cancel.store(true, Ordering::Relaxed);
        }
    }
}

fn workspace(m: &mut Model) -> &mut Workspace {
    &mut m.areas.transfers.conversion
}

fn manager() -> Result<LocalConversions, String> {
    Ok(LocalConversions::new(
        crate::prefs::directory()
            .ok_or("No private state directory is available")?
            .join("conversions"),
    ))
}

fn worker(
    m: &mut Model,
    name: &str,
    work: impl FnOnce(mpsc::Sender<Event>, Arc<AtomicBool>, Wake) + Send + 'static,
) {
    if m.areas.transfers.conversion.job.is_some() {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let wake = m.waker.clone();
    match std::thread::Builder::new()
        .name(name.into())
        .spawn(move || work(sender, stop, wake))
    {
        Ok(_) => workspace(m).job = Some(Job { receiver, cancel }),
        Err(_) => workspace(m).error = Some("Cannot start the local conversion worker".into()),
    }
}

fn send(tx: &mpsc::Sender<Event>, wake: &Wake, event: Event) {
    let _ = tx.send(event);
    if let Some(wake) = wake {
        wake();
    }
}

pub fn open(m: &mut Model) {
    let demo = m.demo;
    let p = workspace(m);
    p.show = true;
    p.epoch += 1;
    if p.job.is_none() && p.review.is_none() && !demo && !p.preview {
        worker(m, "conversion-history", |tx, _, wake| {
            send(
                &tx,
                &wake,
                Event::History(manager().and_then(|m| m.history())),
            )
        });
    }
}

pub fn close(m: &mut Model) {
    let p = workspace(m);
    p.show = false;
    p.epoch += 1;
    if let Some(review) = p.review.take() {
        worker(m, "conversion-abandon", move |tx, _, wake| {
            send(&tx, &wake, Event::Abandoned(review.abandon()))
        });
    }
}

pub fn choose(m: &mut Model, destination: bool) {
    let p = workspace(m);
    if p.job.is_some() || p.review.is_some() || p.preview_review {
        return;
    }
    let epoch = p.epoch;
    let extension = p.output.extension();
    let live = move |p: &Workspace| {
        p.show && p.epoch == epoch && p.job.is_none() && p.review.is_none()
    };
    let reply = move |m: &mut Model, path: Option<PathBuf>| {
        let p = workspace(m);
        if !live(p) {
            return;
        }
        let Some(path) = path else { return };
        if destination {
            p.destination = Some(path);
        } else {
            p.source = Some(path);
        }
        p.completed = None;
        p.latest = None;
        p.error = None;
        p.converting = false;
    };
    let failed = move |m: &mut Model, error: &str| {
        let p = workspace(m);
        if live(p) {
            p.error = Some(error.to_owned());
        }
    };
    let request = if destination {
        FileRequest::save(
            "Save a new converted disk image",
            format!("converted.{extension}"),
            reply,
        )
    } else {
        FileRequest::open("Select an offline self-contained disk image", reply)
    };
    m.choose_file(request.on_failure(failed));
}

pub fn prepare(m: &mut Model) {
    let demo = m.demo;
    let p = workspace(m);
    if p.job.is_some() || p.review.is_some() || demo || p.preview {
        return;
    }
    let (Some(source), Some(destination)) = (p.source.clone(), p.destination.clone()) else {
        return;
    };
    let format = p.format;
    let output = p.output;
    let tools = ConversionTools {
        qemu: p.tool.clone().into(),
    };
    p.error = None;
    p.completed = None;
    p.converting = false;
    worker(m, "conversion-prepare", move |tx, cancel, wake| {
        let result = manager().and_then(|m| {
            m.prepare(
                ConversionRequest {
                    tools,
                    source,
                    source_format: format,
                    output,
                    destination,
                },
                &cancel,
                |row| send(&tx, &wake, Event::Progress(row)),
            )
        });
        send(&tx, &wake, Event::Prepared(Box::new(result)));
    });
}

pub fn convert(m: &mut Model) {
    let demo = m.demo;
    let p = workspace(m);
    if p.job.is_some() || demo || p.preview {
        return;
    }
    let Some(review) = p.review.take() else {
        return;
    };
    p.converting = true;
    worker(m, "conversion-run", move |tx, cancel, wake| {
        let result = review.convert(&cancel, |row| send(&tx, &wake, Event::Progress(row)));
        send(&tx, &wake, Event::Finished(result));
    });
}

pub fn discard(m: &mut Model) {
    if let Some(review) = workspace(m).review.take() {
        worker(m, "conversion-discard", move |tx, _, wake| {
            send(&tx, &wake, Event::Abandoned(review.abandon()))
        });
    }
}

pub fn cancel(m: &mut Model) {
    if let Some(job) = &workspace(m).job {
        job.cancel.store(true, Ordering::Relaxed);
    }
}

/// "Convert another image": back to the form, once no job runs.
pub fn restart(m: &mut Model) {
    let p = workspace(m);
    if p.job.is_none() {
        p.converting = false;
    }
}

pub fn poll(m: &mut Model) -> bool {
    let original_job = workspace(m).job.as_ref().map(|j| j.cancel.clone());
    let mut disconnected = false;
    let events = if let Some(job) = &workspace(m).job {
        let mut events = vec![];
        loop {
            match job.receiver.try_recv() {
                Ok(event) => events.push(event),
                Err(mpsc::TryRecvError::Empty) => break,
                Err(mpsc::TryRecvError::Disconnected) => {
                    disconnected = true;
                    break;
                }
            }
        }
        events
    } else {
        return false;
    };
    let changed = !events.is_empty();
    for event in events {
        match event {
            Event::Progress(row) => workspace(m).latest = Some(row),
            Event::Prepared(result) => {
                workspace(m).job = None;
                match *result {
                    Ok(review) => {
                        if workspace(m).show {
                            workspace(m).review = Some(review);
                        } else {
                            worker(m, "conversion-abandon", move |tx, _, wake| {
                                send(&tx, &wake, Event::Abandoned(review.abandon()))
                            });
                        }
                    }
                    Err(e) => workspace(m).error = Some(e),
                }
            }
            Event::Finished(result) => {
                let p = workspace(m);
                p.job = None;
                match result {
                    Ok(row) => {
                        if row.state == ConversionState::Completed {
                            p.completed = p.destination.clone().map(|path| (path, p.output));
                        }
                        p.history.push(row.clone());
                        p.latest = Some(row);
                    }
                    Err(e) => p.error = Some(e),
                }
            }
            Event::History(result) => {
                let p = workspace(m);
                p.job = None;
                match result {
                    Ok(rows) => p.history = rows,
                    Err(e) => p.error = Some(e),
                }
            }
            Event::Abandoned(result) => {
                let p = workspace(m);
                p.job = None;
                if let Err(e) = result {
                    p.error = Some(e);
                }
            }
        }
    }
    let p = workspace(m);
    if disconnected
        && p.job.as_ref().is_some_and(|j| {
            original_job
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, &j.cancel))
        })
    {
        p.job = None;
        p.error = Some(STOPPED.into());
        return true;
    }
    changed
}

pub fn review_cleanup(m: &mut Model, id: uuid::Uuid) {
    let p = workspace(m);
    if p.job.is_none() && p.review.is_none() && !p.preview_review {
        p.cleanup = Some(id);
    }
}

pub fn cleanup(m: &mut Model) {
    let demo = m.demo;
    let p = workspace(m);
    let Some(id) = p.cleanup.take() else {
        return;
    };
    if p.job.is_some() || p.review.is_some() || demo || p.preview {
        return;
    }
    worker(m, "conversion-cleanup", move |tx, _, wake| {
        let result = manager().and_then(|m| {
            m.discard_temporary_files(id)?;
            m.history()
        });
        send(&tx, &wake, Event::History(result));
    });
}

fn pick_format(m: &mut Model, label: &str) {
    let p = workspace(m);
    if p.job.is_some() || p.review.is_some() || p.preview_review {
        return;
    }
    if let Some(format) = FORMATS.into_iter().find(|format| format.label() == label) {
        p.format = format;
        p.latest = None;
    }
}

fn pick_output(m: &mut Model, extension: &str) {
    let p = workspace(m);
    if p.job.is_some() || p.review.is_some() || p.preview_review {
        return;
    }
    if let Some(output) = [DiskImageFormat::Raw, DiskImageFormat::Vhd]
        .into_iter()
        .find(|output| output.extension() == extension)
    {
        p.output = output;
        p.destination = None;
        p.latest = None;
    }
}

/// "Review disk import…": the conversion hides and the import wizard opens
/// on the converted file, for the pool of the scope session.
fn import(m: &mut Model) {
    let p = workspace(m);
    if p.job.is_some() {
        return;
    }
    let Some((path, format)) = p.completed.clone() else {
        return;
    };
    let Some(pool) = m.inventory.as_ref().map(|i| i.pool_id) else {
        return;
    };
    workspace(m).show = false;
    super::panel::import_converted(m, Some(pool), path, format);
}

fn path_text(path: &Option<PathBuf>, none: &str) -> String {
    path.as_ref()
        .map(|p| p.display().to_string())
        .unwrap_or_else(|| none.into())
}

fn progress_state(state: &ConversionState, busy: bool) -> OutcomeState {
    match state {
        ConversionState::Completed => OutcomeState::Succeeded,
        ConversionState::Failed => OutcomeState::Failed,
        ConversionState::Cancelled => OutcomeState::Cancelled,
        ConversionState::Interrupted => OutcomeState::Unknown,
        _ if busy => OutcomeState::Running(None),
        // A row that stopped before a final state and has no worker: what
        // is known of it is partial.
        _ => OutcomeState::Partial,
    }
}

pub fn surface(m: &Model) -> Surface {
    let p = &m.areas.transfers.conversion;
    let busy = p.job.is_some();
    let reviewing = p.review.is_some() || p.preview_review;
    let editing = !busy && !reviewing;
    let synthetic = p.preview || m.demo;
    let locked = (!editing).then_some(if busy { BUSY } else { REVIEWING });
    let stage = if reviewing {
        1
    } else if p.converting {
        2
    } else {
        0
    };
    let mut body = blocks![caption(INTRO)];
    if reviewing {
        let (source, output, size, version, original, destination) = if let Some(review) = &p.review
        {
            (
                review.summary().source,
                review.summary().output,
                review.summary().virtual_size.unwrap_or(0),
                review.qemu_version().to_string(),
                review.source_path().display().to_string(),
                review.destination().display().to_string(),
            )
        } else {
            (
                p.format,
                p.output,
                512 * 1024 * 1024,
                "qemu-img version 10.1.2 · synthetic".into(),
                path_text(&p.source, "No source selected"),
                path_text(&p.destination, "No output selected"),
            )
        };
        body.push(heading("Review local conversion"));
        body.extend(blocks![
            review(
                "conversion-review",
                [
                    format!("{} → {:?} · {} bytes", source.label(), output, size),
                    version,
                    format!("Original: {original}"),
                    format!("New output: {destination}"),
                ],
            )
            .warning(REVIEW_HELP)
        ]);
    } else if stage == 2 {
        body.extend(blocks![caption(format!(
            "Original: {}\nNew output: {}",
            path_text(&p.source, "No source selected"),
            path_text(&p.destination, "No output selected")
        ))]);
    } else {
        body.extend(blocks![
            input(
                "conversion-tool",
                "Trusted local qemu-img executable (QEMU 8 or later)",
                &p.tool,
                |m, text| workspace(m).tool = text.to_owned(),
            ),
            choice(
                "conversion-format",
                FORMATS.into_iter().map(|format| {
                    option(format.label(), format.label())
                        .key(format!("conversion-format-{}", format.label()))
                }),
                Some(p.format.label()),
                pick_format,
            )
            .label("Input format (selected explicitly)")
            .dropdown()
            .disabled_if(!editing, locked.unwrap_or(BUSY)),
            actions([
                button("conversion-source", "Choose source image…", |m| choose(
                    m, false
                ))
                .disabled_opt(locked)
            ]),
            caption(path_text(&p.source, "No source selected")).key("conversion-source-path"),
            choice(
                "conversion-output",
                [DiskImageFormat::Raw, DiskImageFormat::Vhd]
                    .into_iter()
                    .map(|output| {
                        option(output.extension(), format!("{output:?}"))
                            .key(format!("conversion-output-{}", output.extension()))
                    }),
                Some(p.output.extension()),
                pick_output,
            )
            .label("Output format")
            .segmented()
            .disabled_if(!editing, locked.unwrap_or(BUSY)),
            actions([button(
                "conversion-destination",
                "Choose new output file…",
                |m| choose(m, true)
            )
            .disabled_opt(locked)]),
            caption(path_text(&p.destination, "No output selected"))
                .key("conversion-destination-path"),
            caption(FORM_HELP),
        ]);
    }
    if let Some(row) = &p.latest {
        body.extend(blocks![
            outcome(
                "conversion-progress",
                progress_state(&row.state, busy),
                format!("{:?}", row.state),
            )
            .line(row.detail.clone())
        ]);
    }
    if let Some(error) = &p.error {
        body.extend(blocks![alert(error.clone()).key("conversion-error")]);
    }
    if !p.history.is_empty() {
        body.extend(blocks![
            group("Local history", history(p, locked)).key("conversion-history")
        ]);
    }
    if let Some(id) = p.cleanup {
        body.push(heading("Remove stopped conversion temporary files?"));
        body.extend(blocks![
            caption(format!("Job {id}. Only files with the exact recorded owned identities inside this job's private staging directory are eligible. The original and published output are preserved; unknown files remain.")),
            actions([button(
                "conversion-cleanup-confirm",
                "Remove recorded temporary files",
                cleanup,
            )
            .danger()
            .disabled_opt(locked)
            .disabled_if(synthetic, PREVIEW)]),
        ]);
    }
    if synthetic {
        body.extend(blocks![caption(PREVIEW).key("conversion-preview")]);
    }
    let mut footer = vec![button(
        "conversion-close",
        if busy { "Hide" } else { "Close" },
        close,
    )];
    if busy {
        footer.push(button("conversion-cancel", "Cancel local work", cancel));
    } else if reviewing {
        footer.push(button("conversion-discard", "Discard review", discard));
        footer.push(
            button("conversion-submit", "Convert reviewed image", convert)
                .primary()
                .disabled_if(synthetic, PREVIEW),
        );
    } else if stage == 2 {
        footer.push(button(
            "conversion-restart",
            "Convert another image",
            restart,
        ));
    } else {
        footer.push(
            button("conversion-prepare", "Inspect and review", prepare)
                .primary()
                .disabled_if(p.source.is_none(), NO_SOURCE)
                .disabled_if(p.destination.is_none(), NO_OUTPUT)
                .disabled_if(synthetic, PREVIEW),
        );
    }
    if p.completed.is_some() {
        footer.push(
            button("conversion-import", "Review disk import…", import)
                .primary()
                .disabled_if(m.inventory.is_none(), NO_POOL)
                .disabled_if(busy, BUSY),
        );
    }
    Surface::wizard(
        SURFACE,
        "Local disk conversion",
        ["Source and format", "Review", "Convert"],
        stage,
    )
    .experimental(true)
    .body(body)
    .footer(footer)
    .on_close(close)
}

fn history(p: &Workspace, locked: Option<&'static str>) -> Vec<Block> {
    let mut body = Vec::new();
    let pages = p.history.len().div_ceil(12);
    let page = p.history_page.min(pages - 1);
    if pages > 1 {
        body.extend(blocks![
            caption(format!(
                "Page {} of {pages} · {} recorded jobs",
                page + 1,
                p.history.len()
            )),
            actions([
                button(
                    "conversion-history-previous",
                    "Previous history page",
                    |m| {
                        let p = workspace(m);
                        p.history_page = p.history_page.saturating_sub(1);
                    }
                )
                .disabled_if(page == 0, "This is the first page."),
                button(
                    "conversion-history-next",
                    "Next history page",
                    move |m| workspace(m).history_page = page + 1
                )
                .disabled_if(page + 1 >= pages, "This is the last page."),
            ]),
        ]);
    }
    for row in p.history.iter().rev().skip(page * 12).take(12) {
        let id = row.id;
        body.extend(blocks![caption(format!(
            "{} → {:?} · {:?}\n{}",
            row.source.label(),
            row.output,
            row.state,
            row.detail
        ))]);
        if matches!(
            row.state,
            ConversionState::Failed | ConversionState::Cancelled | ConversionState::Interrupted
        ) {
            body.extend(blocks![actions([button(
                format!("conversion-cleanup-{id}"),
                "Review temporary file cleanup",
                move |m| review_cleanup(m, id),
            )
            .disabled_opt(locked)]),]);
        }
    }
    body
}
