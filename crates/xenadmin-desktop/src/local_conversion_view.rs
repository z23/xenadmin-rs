use super::*;
use std::{
    path::PathBuf,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
};
use xenadmin_core::{
    ConversionFormat as Format, ConversionRequest, ConversionReview, ConversionState,
    ConversionSummary, ConversionTools, DiskImageFormat, LocalConversions,
};

enum Event {
    Progress(ConversionSummary),
    Prepared(Box<Result<ConversionReview, String>>),
    Finished(Result<ConversionSummary, String>),
    History(Result<Vec<ConversionSummary>, String>),
    Abandoned(Result<(), String>),
}
struct Job {
    receiver: mpsc::Receiver<Event>,
    cancel: Arc<AtomicBool>,
}
pub(super) struct Workspace {
    pub show: bool,
    epoch: u64,
    source: Option<PathBuf>,
    destination: Option<PathBuf>,
    tool: TextEdit,
    format: Format,
    output: DiskImageFormat,
    review: Option<ConversionReview>,
    job: Option<Job>,
    latest: Option<ConversionSummary>,
    history: Vec<ConversionSummary>,
    history_page: usize,
    error: Option<String>,
    completed: Option<(PathBuf, DiskImageFormat)>,
    preview: bool,
    preview_review: bool,
    cleanup: Option<uuid::Uuid>,
}
impl Default for Workspace {
    fn default() -> Self {
        Self {
            show: false,
            epoch: 0,
            source: None,
            destination: None,
            tool: TextEdit::new("/usr/bin/qemu-img"),
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
fn manager() -> Result<LocalConversions, String> {
    Ok(LocalConversions::new(
        preferences::directory()
            .ok_or("No private state directory is available")?
            .join("conversions"),
    ))
}
fn worker(
    s: &mut State,
    name: &str,
    work: impl FnOnce(mpsc::Sender<Event>, Arc<AtomicBool>, Option<Arc<dyn Fn() + Send + Sync>>)
    + Send
    + 'static,
) {
    if s.local_conversion.job.is_some() {
        return;
    }
    let (sender, receiver) = mpsc::channel();
    let cancel = Arc::new(AtomicBool::new(false));
    let stop = cancel.clone();
    let wake = s.waker.clone();
    match std::thread::Builder::new()
        .name(name.into())
        .spawn(move || work(sender, stop, wake))
    {
        Ok(_) => s.local_conversion.job = Some(Job { receiver, cancel }),
        Err(_) => {
            s.local_conversion.error = Some("Cannot start the local conversion worker".into())
        }
    }
}
fn send(tx: &mpsc::Sender<Event>, wake: &Option<Arc<dyn Fn() + Send + Sync>>, event: Event) {
    let _ = tx.send(event);
    if let Some(wake) = wake {
        wake();
    }
}
pub(super) fn open(s: &mut State) {
    s.local_conversion.show = true;
    s.local_conversion.epoch += 1;
    if s.local_conversion.job.is_none() && s.local_conversion.review.is_none() && !s.demo {
        worker(s, "conversion-history", |tx, _, wake| {
            send(
                &tx,
                &wake,
                Event::History(manager().and_then(|m| m.history())),
            )
        });
    }
}
fn close(s: &mut State) {
    s.local_conversion.show = false;
    s.local_conversion.epoch += 1;
    if let Some(review) = s.local_conversion.review.take() {
        worker(s, "conversion-abandon", move |tx, _, wake| {
            send(&tx, &wake, Event::Abandoned(review.abandon()))
        });
    }
}
fn choose(s: &mut State, tx: &mut Tx, destination: bool) {
    if s.local_conversion.job.is_some() || s.local_conversion.review.is_some() {
        return;
    }
    let epoch = s.local_conversion.epoch;
    let callback =
        move |s: &mut State, result: Result<Option<PathBuf>, rune::FileDialogError>, _: &mut Tx| {
            if !s.local_conversion.show
                || s.local_conversion.epoch != epoch
                || s.local_conversion.job.is_some()
                || s.local_conversion.review.is_some()
            {
                return;
            }
            match result {
                Ok(Some(path)) => {
                    if destination {
                        s.local_conversion.destination = Some(path);
                    } else {
                        s.local_conversion.source = Some(path);
                    }
                    s.local_conversion.completed = None;
                    s.local_conversion.latest = None;
                    s.local_conversion.error = None;
                }
                Ok(None) => {}
                Err(e) => s.local_conversion.error = Some(e.to_string()),
            }
        };
    let options = rune::FileDialogOptions {
        title: if destination {
            "Save a new converted disk image"
        } else {
            "Select an offline self-contained disk image"
        }
        .into(),
        file_name: destination
            .then(|| format!("converted.{}", s.local_conversion.output.extension())),
        ..Default::default()
    };
    if destination {
        tx.save_file(options, callback);
    } else {
        tx.open_file(options, callback);
    }
}
fn prepare(s: &mut State) {
    let p = &s.local_conversion;
    if p.job.is_some() || p.review.is_some() || s.demo || p.preview {
        return;
    }
    let (Some(source), Some(destination)) = (p.source.clone(), p.destination.clone()) else {
        return;
    };
    let format = p.format;
    let output = p.output;
    let tools = ConversionTools {
        qemu: p.tool.text.to_string().into(),
    };
    s.local_conversion.error = None;
    s.local_conversion.completed = None;
    worker(s, "conversion-prepare", move |tx, cancel, wake| {
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
fn convert(s: &mut State) {
    if s.local_conversion.job.is_some() || s.demo || s.local_conversion.preview {
        return;
    }
    let Some(review) = s.local_conversion.review.take() else {
        return;
    };
    worker(s, "conversion-run", move |tx, cancel, wake| {
        let result = review.convert(&cancel, |row| send(&tx, &wake, Event::Progress(row)));
        send(&tx, &wake, Event::Finished(result));
    });
}
fn discard(s: &mut State) {
    if let Some(review) = s.local_conversion.review.take() {
        worker(s, "conversion-discard", move |tx, _, wake| {
            send(&tx, &wake, Event::Abandoned(review.abandon()))
        });
    }
}
fn cancel(s: &mut State) {
    if let Some(job) = &s.local_conversion.job {
        job.cancel.store(true, Ordering::Relaxed);
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let original_job = s.local_conversion.job.as_ref().map(|j| j.cancel.clone());
    let mut disconnected = false;
    let events = if let Some(job) = &s.local_conversion.job {
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
            Event::Progress(row) => s.local_conversion.latest = Some(row),
            Event::Prepared(result) => {
                s.local_conversion.job = None;
                match *result {
                    Ok(review) => {
                        if s.local_conversion.show {
                            s.local_conversion.review = Some(review);
                        } else {
                            worker(s, "conversion-abandon", move |tx, _, wake| {
                                send(&tx, &wake, Event::Abandoned(review.abandon()))
                            });
                        }
                    }
                    Err(e) => s.local_conversion.error = Some(e),
                }
            }
            Event::Finished(result) => {
                s.local_conversion.job = None;
                match result {
                    Ok(row) => {
                        if row.state == ConversionState::Completed {
                            s.local_conversion.completed = s
                                .local_conversion
                                .destination
                                .clone()
                                .map(|p| (p, s.local_conversion.output));
                        }
                        s.local_conversion.history.push(row.clone());
                        s.local_conversion.latest = Some(row);
                    }
                    Err(e) => s.local_conversion.error = Some(e),
                }
            }
            Event::History(result) => {
                s.local_conversion.job = None;
                match result {
                    Ok(rows) => s.local_conversion.history = rows,
                    Err(e) => s.local_conversion.error = Some(e),
                }
            }
            Event::Abandoned(result) => {
                s.local_conversion.job = None;
                if let Err(e) = result {
                    s.local_conversion.error = Some(e);
                }
            }
        }
    }
    if disconnected
        && s.local_conversion.job.as_ref().is_some_and(|j| {
            original_job
                .as_ref()
                .is_some_and(|original| Arc::ptr_eq(original, &j.cancel))
        })
    {
        s.local_conversion.job = None;
        s.local_conversion.error=Some("Local worker stopped; reopen history to inspect interrupted files. Nothing will resume automatically.".into());
        return true;
    }
    changed
}
fn cleanup(s: &mut State) {
    let Some(id) = s.local_conversion.cleanup.take() else {
        return;
    };
    if s.local_conversion.job.is_some()
        || s.local_conversion.review.is_some()
        || s.demo
        || s.local_conversion.preview
    {
        return;
    }
    worker(s, "conversion-cleanup", move |tx, _, wake| {
        let result = manager().and_then(|m| {
            m.discard_temporary_files(id)?;
            m.history()
        });
        send(&tx, &wake, Event::History(result));
    });
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let p = &s.local_conversion;
    let busy = p.job.is_some();
    let editing = !busy && p.review.is_none() && !p.preview_review;
    let mut body=col().gap(10.0).child(caption("Convert an offline local image. The original is preserved. No host connection is required and no image is imported automatically.",k).wrap());
    if p.review.is_some() || p.preview_review {
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
                p.source.as_ref().unwrap().display().to_string(),
                p.destination.as_ref().unwrap().display().to_string(),
            )
        };
        body=body.child(heading("Review local conversion",k)).child(caption(format!("{} → {:?} · {} bytes\n{}",source.label(),output,size,version),k).wrap())
            .child(caption(format!("Original: {}\nNew output: {}",original,destination),k).wrap_anywhere())
            .child(caption("The private staged source is read-only. Conversion and complete content comparison run in a restricted local sandbox. Existing files will not be overwritten. Internal snapshots, backing chains, external extents and encrypted images are refused. This converts disk content only; guest boot and driver compatibility remain unverified.",k).wrap());
    } else {
        body = body
            .child(caption(
                "Trusted local qemu-img executable (QEMU 8 or later)",
                k,
            ))
            .child(
                TextInput::new(&p.tool)
                    .on_edit(|s: &mut State| &mut s.local_conversion.tool)
                    .build(k)
                    .key("conversion-tool"),
            );
        let mut formats = row().gap(6.0).flex_wrap();
        for format in [
            Format::Qcow2,
            Format::Vmdk,
            Format::Vhdx,
            Format::Vhd,
            Format::Raw,
        ] {
            formats = formats.child(
                Button::new(format!(
                    "{}{}",
                    format.label(),
                    if p.format == format {
                        " · selected"
                    } else {
                        ""
                    }
                ))
                .disabled(!editing)
                .on_press(move |s: &mut State| {
                    s.local_conversion.format = format;
                    s.local_conversion.latest = None;
                })
                .build(k)
                .key(format!("conversion-format-{}", format.label())),
            );
        }
        body = body
            .child(caption("Input format (selected explicitly)", k))
            .child(formats)
            .child(
                Button::new("Choose source image…")
                    .disabled(!editing)
                    .on_press_tx(|s: &mut State, tx: &mut Tx| choose(s, tx, false))
                    .build(k)
                    .key("conversion-source"),
            )
            .child(
                caption(
                    p.source
                        .as_ref()
                        .map(|p| p.display().to_string())
                        .unwrap_or_else(|| "No source selected".into()),
                    k,
                )
                .wrap_anywhere(),
            );
        let mut outputs = row().gap(6.0).flex_wrap();
        for output in [DiskImageFormat::Raw, DiskImageFormat::Vhd] {
            outputs = outputs.child(
                Button::new(format!(
                    "{:?}{}",
                    output,
                    if p.output == output {
                        " · selected"
                    } else {
                        ""
                    }
                ))
                .disabled(!editing)
                .on_press(move |s: &mut State| {
                    s.local_conversion.output = output;
                    s.local_conversion.destination = None;
                    s.local_conversion.latest = None;
                })
                .build(k),
            );
        }
        body=body.child(caption("Output format",k)).child(outputs)
            .child(Button::new("Choose new output file…").disabled(!editing).on_press_tx(|s:&mut State,tx:&mut Tx|choose(s,tx,true)).build(k).key("conversion-destination"))
            .child(caption(p.destination.as_ref().map(|p|p.display().to_string()).unwrap_or_else(||"No output selected".into()),k).wrap_anywhere())
            .child(caption("Only self-contained images are supported. Inspection first makes a private source copy; allow space for that copy and the converted disk. QEMU and Bubblewrap are optional local dependencies. Missing sandbox support refuses conversion; no software is installed automatically.",k).wrap());
    }
    if let Some(row) = &p.latest {
        body = body
            .child(heading(format!("{:?}", row.state), k))
            .child(caption(&row.detail, k).wrap());
    }
    if let Some(error) = &p.error {
        body = body.child(
            caption(error, k)
                .color(k.c.danger.icon)
                .wrap()
                .role("alert"),
        );
    }
    if !p.history.is_empty() {
        body = body.child(heading("Local history", k));
        let pages = p.history.len().div_ceil(12);
        let page = p.history_page.min(pages - 1);
        if pages > 1 {
            body = body.child(
                row()
                    .gap(8.0)
                    .flex_wrap()
                    .child(
                        Button::new("Previous history page")
                            .disabled(page == 0)
                            .on_press(|s: &mut State| {
                                s.local_conversion.history_page =
                                    s.local_conversion.history_page.saturating_sub(1)
                            })
                            .build(k),
                    )
                    .child(caption(
                        format!(
                            "Page {} of {pages} · {} recorded jobs",
                            page + 1,
                            p.history.len()
                        ),
                        k,
                    ))
                    .child(
                        Button::new("Next history page")
                            .disabled(page + 1 >= pages)
                            .on_press(move |s: &mut State| {
                                s.local_conversion.history_page = page + 1
                            })
                            .build(k),
                    ),
            );
        }
        for row in p.history.iter().rev().skip(page * 12).take(12) {
            let id = row.id;
            if matches!(
                row.state,
                ConversionState::Failed | ConversionState::Cancelled | ConversionState::Interrupted
            ) {
                body = body.child(
                    Button::new("Review temporary file cleanup")
                        .disabled(!editing)
                        .on_press(move |s: &mut State| s.local_conversion.cleanup = Some(id))
                        .build(k)
                        .key(format!("conversion-cleanup-{id}")),
                );
            }
            body = body.child(
                caption(
                    format!(
                        "{} → {:?} · {:?}\n{}",
                        row.source.label(),
                        row.output,
                        row.state,
                        row.detail
                    ),
                    k,
                )
                .wrap(),
            );
        }
    }
    if let Some(id) = p.cleanup {
        body=body.child(heading("Remove stopped conversion temporary files?",k)).child(caption(format!("Job {id}. Only files with the exact recorded owned identities inside this job's private staging directory are eligible. The original and published output are preserved; unknown files remain."),k).wrap())
            .child(Button::new("Remove recorded temporary files").disabled(!editing||s.demo||p.preview).on_press(cleanup).build(k).key("conversion-cleanup-confirm"));
    }
    if p.preview || s.demo {
        body = body.child(
            caption(
                "Synthetic preview · local files and tools will not be accessed",
                k,
            )
            .wrap(),
        );
    }
    let mut footer = row().gap(8.0).flex_wrap().justify_end().child(
        Button::new(if busy { "Hide" } else { "Close" })
            .on_press(close)
            .build(k)
            .key("conversion-close"),
    );
    if busy {
        footer = footer.child(
            Button::new("Cancel local work")
                .on_press(cancel)
                .build(k)
                .key("conversion-cancel"),
        );
    } else if p.review.is_some() || p.preview_review {
        footer = footer
            .child(Button::new("Discard review").on_press(discard).build(k))
            .child(
                Button::new("Convert reviewed image")
                    .disabled(s.demo || p.preview)
                    .on_press(convert)
                    .build(k)
                    .key("conversion-submit"),
            );
    } else {
        footer = footer.child(
            Button::new("Inspect and review")
                .disabled(p.source.is_none() || p.destination.is_none() || s.demo || p.preview)
                .on_press(prepare)
                .build(k)
                .key("conversion-prepare"),
        );
    }
    if let Some((path, format)) = &p.completed {
        let path = path.clone();
        let format = *format;
        footer = footer.child(
            Button::new("Review disk import…")
                .disabled(s.inventory.is_none() || busy)
                .on_press(move |s: &mut State| {
                    s.local_conversion.show = false;
                    transfers::import_converted(s, path.clone(), format);
                })
                .build(k)
                .key("conversion-import"),
        );
    }
    el().w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .center()
        .fill(rune::Color::BLACK.alpha(0.5))
        .role("dialog")
        .label("Local disk conversion")
        .modal(false, close)
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 780.0))
                .h((s.window_height - 40.0).clamp(280.0, 850.0))
                .padding(18.0)
                .gap(12.0)
                .radius(10.0)
                .fill(k.c.bg_base)
                .child(heading("Local disk conversion", k).key("conversion-heading"))
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}
pub(super) fn scene(s: &mut State) {
    s.local_conversion.show = true;
    s.local_conversion.preview = true;
    s.local_conversion.source = Some("/home/operator/images/offline-guest.qcow2".into());
    s.local_conversion.destination = Some("/home/operator/images/converted-guest.vhd".into());
    s.local_conversion.output = DiskImageFormat::Vhd;
}

pub(super) fn review_scene(s: &mut State) {
    scene(s);
    s.local_conversion.preview_review = true;
}
#[cfg(test)]
#[path = "local_conversion_view_tests.rs"]
mod tests;
