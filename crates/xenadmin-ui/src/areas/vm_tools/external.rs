//! External tools: the Rune `external_tools.rs`, logic and description.
//!
//! Two dialogs over one workspace, as in Rune:
//!
//! - the launch dialog (`guest-connections`) for one running guest: pick a
//!   declared tool, an address, a port and a user name, have the exact local
//!   command reviewed, then launch it;
//! - the declarations dialog (`external-tools`): the catalog that is stored
//!   in the preferences.
//!
//! No XAPI write is involved and nothing is journaled. What is checked, and
//! when (the Rune order, unchanged):
//!
//! - The guest is captured at open (pool, VM, connection generation, a
//!   clone of the connection) and never retargeted. Any generation change
//!   makes the dialog unusable.
//! - "Review command" inspects the executable on a worker thread
//!   ([`Command::prepare`] touches the file system); a result that arrives
//!   after the inputs changed or the dialog closed is discarded.
//! - A review is dropped as soon as the inputs, the tool declaration or the
//!   workspace differ from what was reviewed ([`drop_stale_review`], run by
//!   `poll` and by every edit).
//! - "Launch reviewed tool": synthetic pool, current workspace, review
//!   present, inputs equal to the reviewed ones, guest verified, a permit;
//!   then the executable identity and the guest once more immediately
//!   before exec ([`Command::spawn`]).
//! - At most eight tracked workers; the log keeps 32 lines and shows 8.
//!
//! The process is started by [`Command::spawn`] of `command.rs` exactly as
//! in Rune (`std::process`, cleared environment, no shell); the Rune module
//! used no `xenadmin-process` call and neither does this one, so no
//! front-end service is needed.
//!
//! Keys and labels are the Rune ones: `guest-tool-<id>` (the options of the
//! tool choice, which were buttons), `configure-guest-tools`,
//! `guest-tool-address`, `guest-tool-manual-address`, `guest-tool-port`,
//! `guest-tool-user`, `review-guest-tool`, `launch-guest-tool`,
//! `close-guest-tools`, `save-guest-tools`, `remove-guest-tool`,
//! `guest-tool-name`, `guest-tool-executable`, `guest-tool-arguments`. New
//! keys, for elements Rune left without one: `guest-tool-choice`,
//! `guest-tool-use-<ip>`, `guest-tool-source`, `guest-tool-command` (the
//! reviewed command), `guest-tool-status` (the error and progress line),
//! `guest-tool-history`, `guest-tool-edit` (the declared tools),
//! `add-guest-tool-ssh`, `add-guest-tool-rdp`, `add-guest-tool-custom`.

use std::{
    collections::BTreeSet,
    net::IpAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

use xenadmin_core::{Connection, Inventory, PoolId, PowerState, VmId};

use super::command::{self, Catalog, Command, Context, Kind, Tool};
use crate::prelude::*;

/// The ids of the two dialogs: the names of their scenes.
pub const LAUNCH: &str = "guest-connections";
pub const CONFIGURE: &str = "external-tools";

pub const NO_LAUNCH_IN_DEMO: &str = "Demo mode cannot launch external programs";
/// (These two name the Rune launcher "Guest tools": decision 10.)
pub const CONNECTION_CHANGED: &str = "Connection changed. Close and reopen this dialog.";
pub const STALE_INVENTORY: &str =
    "Connection changed or inventory is stale. Close and reopen this dialog.";
pub const REVIEW_FIRST: &str = "Review the command first";
pub const INSPECTING: &str = "Inspecting the configured local executable…";
pub const LAUNCHING: &str = "Launching the reviewed local program…";
pub const REVIEW_DROPPED: &str = "Selection or command inputs changed. Review again before launch.";
pub const TOO_MANY: &str = "Eight local tool processes are already being tracked. Close one \
    before launching another.";
pub const PREVIEW: &str = "This preview cannot save tool declarations";
pub const SAVED: &str = "Tool declarations saved. Each launch still requires review.";
/// New text: Rune disabled the three "Add" buttons at the limit silently.
pub const FULL: &str = "Sixteen tools are declared. Remove one before adding another.";

const SECURITY: &str = "Connect from this workstation to a guest address. Guest-reported \
    addresses are not proof of server identity; the external client's SSH host-key or RDP \
    certificate checks still apply. Guest passwords are entered in that client.";
const TRUST: &str = "This trusted local executable runs as your Linux user. Arguments are \
    passed separately; no command shell is invoked. Only desktop/locale and SSH-agent \
    environment values are inherited; XAPI credentials and sessions are never supplied. \
    Closing this dialog does not close the external client.";
const DECLARATIONS: &str = "Trusted local programs, not Windows plugins. Use an absolute \
    executable path and a JSON array with one string per argument. Nothing launches \
    automatically. Never put passwords or tokens in declarations; arguments can be visible to \
    other local programs.";
const PLACEHOLDERS: &str = "Placeholders: {ip}, {port}, {user}, {vm_uuid}, {pool_uuid}, \
    {rdp_uri}. Substitution never creates extra arguments. SSH preset needs xdg-terminal-exec \
    and OpenSSH; RDP preset needs Remmina. Set the executable and arguments for your installed \
    clients.";

#[derive(Default)]
pub struct Workspace {
    pub(super) panel: Option<Panel>,
    pub(super) jobs: Vec<Job>,
    pub(super) history: Vec<String>,
    /// Tests only: what writing the preferences answers, in place of the
    /// preview check and the file. (Rune skipped both under `cfg!(test)`;
    /// here a test opts in, so the preview refusal stays testable and no
    /// test can write the user's preferences.)
    #[cfg(test)]
    pub(super) storage: Option<Result<(), String>>,
}

pub(super) enum Panel {
    Launch(Box<Launch>),
    Configure(Box<Configure>),
}

/// The captured guest (the Rune `Target`).
#[derive(Clone)]
pub(super) struct Guest {
    pub(super) pool: PoolId,
    pub(super) vm: VmId,
    pub(super) generation: u64,
    pub(super) name: String,
    pub(super) connection: Option<Connection>,
}

pub(super) struct Launch {
    pub(super) guest: Guest,
    pub(super) tool: uuid::Uuid,
    pub(super) address: String,
    pub(super) port: String,
    pub(super) user: String,
    pub(super) reported: bool,
    pub(super) review: Option<Reviewed>,
    pub(super) pending: Option<Preparation>,
    pub(super) error: Option<String>,
}

/// The reviewed inputs: a launch needs them unchanged.
#[derive(Clone, PartialEq, Eq)]
pub(super) struct Inputs {
    pub(super) tool: Tool,
    pub(super) context: Context,
    pub(super) reported: bool,
}

pub(super) struct Preparation {
    pub(super) inputs: Inputs,
    pub(super) receiver: mpsc::Receiver<Result<Reviewed, String>>,
}

/// The reviewed command (the Rune `Review`).
pub(super) struct Reviewed {
    pub(super) inputs: Inputs,
    pub(super) command: Command,
}

pub(super) struct Configure {
    pub(super) catalog: Catalog,
    pub(super) index: Option<usize>,
    pub(super) name: String,
    pub(super) executable: String,
    pub(super) arguments: String,
    pub(super) returning: Option<Box<Launch>>,
    pub(super) error: Option<String>,
}

pub(super) struct Job {
    pub(super) label: String,
    pub(super) receiver: mpsc::Receiver<String>,
}

static RUNNING: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}
fn permit() -> Result<Permit, String> {
    RUNNING
        .try_update(Ordering::SeqCst, Ordering::SeqCst, |v| {
            (v < 8).then_some(v + 1)
        })
        .map(|_| Permit)
        .map_err(|_| TOO_MANY.into())
}

/// The usable addresses the guest's tools report now, at most 64.
pub fn addresses(i: &Inventory, vm: VmId) -> BTreeSet<IpAddr> {
    i.details
        .vms
        .get(&vm)
        .and_then(|v| v.guest_metrics)
        .and_then(|id| i.details.guest_metrics.get(&id))
        .filter(|m| m.live == Some(true))
        .map(|m| {
            m.networks
                .values()
                .filter_map(|v| v.parse::<IpAddr>().ok())
                .filter(|ip| command::usable_ip(*ip))
                .take(64)
                .collect()
        })
        .unwrap_or_default()
}

/// A running guest: not a template, not a control domain.
pub fn ordinary(i: &Inventory, vm: VmId) -> bool {
    i.vms.get(&vm).is_some_and(|v| {
        !v.is_template && !v.is_control_domain && v.power_state == PowerState::Running
    })
}

impl Guest {
    fn verify(&self, ip: IpAddr, reported: bool) -> Result<(), String> {
        let snapshot = self
            .connection
            .as_ref()
            .ok_or(NO_LAUNCH_IN_DEMO)?
            .latest();
        self.observed(
            snapshot.is_current(),
            snapshot.generation(),
            snapshot.inventory().map(AsRef::as_ref),
            ip,
            reported,
        )
    }
    pub(super) fn observed(
        &self,
        current: bool,
        generation: u64,
        inventory: Option<&Inventory>,
        ip: IpAddr,
        reported: bool,
    ) -> Result<(), String> {
        let i = inventory
            .filter(|i| current && generation == self.generation && i.pool_id == self.pool)
            .ok_or(STALE_INVENTORY)?;
        if !ordinary(i, self.vm) {
            return Err("The captured guest is unavailable or no longer running".into());
        }
        if reported && !addresses(i, self.vm).contains(&ip) {
            return Err("The selected address is no longer reported by the guest. Review its \
                        current addresses."
                .into());
        }
        Ok(())
    }
}

fn default_port(kind: Kind) -> &'static str {
    if kind == Kind::Rdp { "3389" } else { "22" }
}

/// Opens the launch dialog for a running guest; with no tool declared, the
/// declarations dialog. Every read is of the guest's pool.
pub fn open(m: &mut Model, object: &ObjectRef) {
    let Some((pool, vm)) = object.pool.zip(object.vm_id()) else {
        return;
    };
    let Some(i) = m.inventory_for(pool).filter(|i| ordinary(i, vm)) else {
        return;
    };
    let Some(tool) = m.preferences.external_tools.tools.first() else {
        configure(m);
        return;
    };
    let ip = addresses(&i, vm).into_iter().next();
    let guest = Guest {
        pool,
        vm,
        generation: m.snapshot_for(pool).map_or(0, |s| s.generation()),
        name: i.vms[&vm].name.clone(),
        connection: m.connection_for(pool),
    };
    m.areas.vm_tools.external.panel = Some(Panel::Launch(Box::new(Launch {
        guest,
        tool: tool.id,
        address: ip.map(|v| v.to_string()).unwrap_or_default(),
        port: default_port(tool.kind).into(),
        user: String::new(),
        reported: ip.is_some(),
        review: None,
        pending: None,
        error: None,
    })));
}

pub(super) fn launch_ref(m: &Model) -> Option<&Launch> {
    match m.areas.vm_tools.external.panel.as_ref()? {
        Panel::Launch(p) => Some(p),
        Panel::Configure(_) => None,
    }
}
pub(super) fn launch_mut(m: &mut Model) -> Option<&mut Launch> {
    match m.areas.vm_tools.external.panel.as_mut()? {
        Panel::Launch(p) => Some(p),
        Panel::Configure(_) => None,
    }
}
pub(super) fn config_mut(m: &mut Model) -> Option<&mut Configure> {
    match m.areas.vm_tools.external.panel.as_mut()? {
        Panel::Configure(p) => Some(p),
        Panel::Launch(_) => None,
    }
}

pub(super) fn inputs(m: &Model, p: &Launch) -> Result<Inputs, String> {
    let tool = m
        .preferences
        .external_tools
        .tools
        .iter()
        .find(|t| t.id == p.tool)
        .cloned()
        .ok_or("The tool declaration changed. Select it again.")?;
    let ip = p
        .address
        .trim()
        .parse::<IpAddr>()
        .map_err(|_| "Enter a literal IPv4 or IPv6 guest address")?;
    let port = p
        .port
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or("Port must be from 1 to 65535")?;
    Ok(Inputs {
        tool,
        context: Context {
            ip,
            port,
            user: p.user.clone(),
            vm: p.guest.vm,
            pool: p.guest.pool,
        },
        reported: p.reported,
    })
}

/// The Rune `s.demo`, for the pool of the captured guest.
fn synthetic(m: &Model, p: &Launch) -> bool {
    m.is_synthetic_pool(p.guest.pool)
}

pub(super) fn current_workspace(m: &Model, p: &Launch) -> bool {
    synthetic(m, p)
        || m.snapshot_for(p.guest.pool).is_some_and(|v| {
            v.is_current()
                && v.generation() == p.guest.generation
                && v.inventory().is_some_and(|i| i.pool_id == p.guest.pool)
        })
}

pub fn review(m: &mut Model) {
    let result: Result<Preparation, String> = (|| {
        let Some(p) = launch_ref(m) else {
            return Err("No guest selected".into());
        };
        if !current_workspace(m, p) {
            return Err(CONNECTION_CHANGED.into());
        }
        let inputs = inputs(m, p)?;
        if !synthetic(m, p) {
            p.guest.verify(inputs.context.ip, inputs.reported)?;
        }
        let token = permit()?;
        let prepared = inputs.clone();
        let (sender, receiver) = mpsc::channel();
        let waker = m.waker.clone();
        std::thread::Builder::new()
            .name("tool-review".into())
            .spawn(move || {
                let _permit = token;
                let result =
                    Command::prepare(&prepared.tool, &prepared.context).map(|command| Reviewed {
                        inputs: prepared,
                        command,
                    });
                let _ = sender.send(result);
                if let Some(w) = waker {
                    w();
                }
            })
            .map_err(|_| "Cannot start the command review worker")?;
        Ok(Preparation { inputs, receiver })
    })();
    let Some(p) = launch_mut(m) else { return };
    p.review = None;
    match result {
        Ok(pending) => {
            p.pending = Some(pending);
            p.error = Some(INSPECTING.into());
        }
        Err(error) => {
            p.pending = None;
            p.error = Some(error);
        }
    }
}

pub fn launch(m: &mut Model) {
    let result: Result<Job, String> = (|| {
        let Some(p) = launch_ref(m) else {
            return Err("No guest selected".into());
        };
        if synthetic(m, p) {
            return Err(NO_LAUNCH_IN_DEMO.into());
        }
        if !current_workspace(m, p) {
            return Err(CONNECTION_CHANGED.into());
        }
        let review = p.review.as_ref().ok_or(REVIEW_FIRST)?;
        let current = inputs(m, p)?;
        if current != review.inputs {
            return Err("Inputs changed. Review the command again.".into());
        }
        p.guest.verify(current.context.ip, current.reported)?;
        let token = permit()?;
        let guest = p.guest.clone();
        let command = review.command.clone();
        let label = format!(
            "{} · {} · {}",
            current.tool.name, guest.name, current.context.ip
        );
        let (sender, receiver) = mpsc::channel();
        let waker = m.waker.clone();
        std::thread::Builder::new()
            .name("guest-tool".into())
            .spawn(move || {
                let _permit = token;
                let send = |status: String| {
                    let _ = sender.send(status);
                    if let Some(w) = &waker {
                        w();
                    }
                };
                let result =
                    command.spawn(|| guest.verify(current.context.ip, current.reported));
                match result {
                    Ok(mut child) => {
                        send(format!(
                            "Launched process {}. Authentication and connection status are \
                             shown by the external client.",
                            child.id()
                        ));
                        send(match child.wait() {
                            Ok(status) => format!(
                                "Launcher exited ({status}). A separate client window may \
                                 still be open."
                            ),
                            Err(_) => "Cannot observe the launched process. Check the external \
                                       client."
                                .into(),
                        });
                    }
                    Err(error) => send(error),
                }
            })
            .map_err(|_| "Cannot start the local tool worker")?;
        Ok(Job { label, receiver })
    })();
    match result {
        Ok(job) => {
            m.areas.vm_tools.external.jobs.push(job);
            if let Some(p) = launch_mut(m) {
                p.review = None;
                p.error = Some(LAUNCHING.into());
            }
        }
        Err(e) => {
            if let Some(p) = launch_mut(m) {
                p.error = Some(e);
            }
        }
    }
}

/// Drains the job reports into the log, takes a finished command review,
/// and drops a review that is no longer the review of what is entered.
pub fn poll(m: &mut Model, _clock: f64) -> bool {
    let mut changed = false;
    let mut history = vec![];
    let workspace = &mut m.areas.vm_tools.external;
    workspace.jobs.retain(|job| {
        loop {
            match job.receiver.try_recv() {
                Ok(v) => {
                    history.push(format!("{}: {v}", job.label));
                    changed = true;
                }
                Err(mpsc::TryRecvError::Empty) => return true,
                Err(mpsc::TryRecvError::Disconnected) => return false,
            }
        }
    });
    workspace.history.extend(history);
    if workspace.history.len() > 32 {
        let excess = workspace.history.len() - 32;
        workspace.history.drain(..excess);
    }
    let prepared = launch_ref(m).and_then(|p| {
        p.pending
            .as_ref()
            .and_then(|pending| match pending.receiver.try_recv() {
                Ok(result) => Some((pending.inputs.clone(), result)),
                Err(mpsc::TryRecvError::Disconnected) => Some((
                    pending.inputs.clone(),
                    Err("The command review worker stopped".into()),
                )),
                Err(mpsc::TryRecvError::Empty) => None,
            })
    });
    if let Some((captured, result)) = prepared {
        let valid = launch_ref(m).is_some_and(|p| {
            current_workspace(m, p) && inputs(m, p).ok().as_ref() == Some(&captured)
        });
        if let Some(p) = launch_mut(m) {
            p.pending = None;
            if valid {
                match result {
                    Ok(review) => {
                        p.review = Some(review);
                        p.error = None;
                    }
                    Err(error) => p.error = Some(error),
                }
            } else {
                p.error =
                    Some("Inputs changed while inspecting the executable. Review again.".into());
            }
        }
        changed = true;
    }
    changed | drop_stale_review(m)
}

/// The staleness rule: a review exists only for the inputs, the tool
/// declaration and the workspace it was made for. Rune ran it every tick;
/// here also every edit runs it, so the description never shows a reviewed
/// command beside other inputs.
pub(super) fn drop_stale_review(m: &mut Model) -> bool {
    let stale = launch_ref(m).is_some_and(|p| {
        p.review.is_some()
            && (!current_workspace(m, p)
                || inputs(m, p).ok().as_ref() != p.review.as_ref().map(|r| &r.inputs))
    });
    if stale && let Some(p) = launch_mut(m) {
        p.review = None;
        p.error = Some(REVIEW_DROPPED.into());
    }
    stale
}

/// Opens the declarations dialog over a draft copy of the stored catalog.
/// An open launch dialog is kept and returned to by Close.
pub fn configure(m: &mut Model) {
    let returning = match m.areas.vm_tools.external.panel.take() {
        Some(Panel::Launch(p)) => Some(p),
        _ => None,
    };
    let catalog = m.preferences.external_tools.clone();
    let first = !catalog.tools.is_empty();
    m.areas.vm_tools.external.panel = Some(Panel::Configure(Box::new(Configure {
        catalog,
        index: None,
        name: String::new(),
        executable: String::new(),
        arguments: "[]".into(),
        returning,
        error: None,
    })));
    if first {
        edit(m, 0);
    }
}

fn edit(m: &mut Model, index: usize) {
    let Some(p) = config_mut(m) else { return };
    if let Some(tool) = p.catalog.tools.get(index) {
        p.index = Some(index);
        p.name = tool.name.clone();
        p.executable = tool.executable.clone();
        p.arguments = serde_json::to_string_pretty(&tool.arguments).unwrap_or_else(|_| "[]".into());
        p.error = None;
    }
}

fn add(m: &mut Model, kind: Kind) {
    let Some(p) = config_mut(m) else { return };
    if p.catalog.tools.len() < command::MAX_TOOLS {
        p.catalog.tools.push(Tool::preset(kind));
        let index = p.catalog.tools.len() - 1;
        edit(m, index);
    }
}

/// "Remove tool" changes the draft only.
fn remove(m: &mut Model) {
    let Some(p) = config_mut(m) else { return };
    if let Some(index) = p.index.take()
        && index < p.catalog.tools.len()
    {
        p.catalog.tools.remove(index);
    }
    if !p.catalog.tools.is_empty() {
        edit(m, 0);
    }
}

/// Writes the preferences that now hold the new catalog. A preview (this
/// process does not own the state directory, or the demo) never writes.
fn write(m: &Model) -> Result<(), String> {
    #[cfg(test)]
    if let Some(answer) = &m.areas.vm_tools.external.storage {
        return answer.clone();
    }
    if !m.owns_state() || m.demo {
        return Err(PREVIEW.into());
    }
    m.preferences.save()
}

/// Validates the tool being edited into the draft and stores the draft.
/// With no tool selected the draft is stored as it is. A refused or failed
/// write leaves the stored catalog as it was.
pub fn save(m: &mut Model) {
    let result: Result<Catalog, String> = (|| {
        let p = config_mut(m).ok_or("Tool selection changed")?;
        let Some(index) = p.index else {
            return Ok(p.catalog.clone());
        };
        let tool = p
            .catalog
            .tools
            .get_mut(index)
            .ok_or("Tool selection changed")?;
        if p.arguments.len() > 20_000 {
            return Err("Argument declaration is too large".into());
        }
        let arguments: Vec<String> = serde_json::from_str(&p.arguments)
            .map_err(|_| "Arguments must be a JSON array of separate strings")?;
        let updated = Tool {
            id: tool.id,
            kind: tool.kind,
            name: p.name.clone(),
            executable: p.executable.clone(),
            arguments,
        };
        updated.validate()?;
        *tool = updated;
        if !p.catalog.valid() {
            return Err("Invalid local tool declarations".into());
        }
        Ok(p.catalog.clone())
    })();
    let message = match result {
        Ok(catalog) => {
            let old = std::mem::replace(&mut m.preferences.external_tools, catalog);
            match write(m) {
                Ok(()) => SAVED.to_owned(),
                Err(error) => {
                    m.preferences.external_tools = old;
                    error
                }
            }
        }
        Err(e) => e,
    };
    if let Some(p) = config_mut(m) {
        p.error = Some(message);
    }
}

/// Close and Escape. From the declarations dialog with a launch dialog
/// under it: back to that one, its review and pending inspection dropped
/// (a dropped `Preparation` drops its receiver, so a late review cannot
/// apply). Otherwise the dialog closes.
pub fn back(m: &mut Model) {
    let panel = m.areas.vm_tools.external.panel.take();
    if let Some(Panel::Configure(p)) = panel
        && let Some(mut returning) = p.returning
    {
        returning.review = None;
        returning.pending = None;
        m.areas.vm_tools.external.panel = Some(Panel::Launch(returning));
    }
}

/// Another object is the scope, or another session: the launch dialog
/// belongs to the guest it captured and closes; the declarations dialog is
/// not about any object and stays, without a launch dialog to return to.
/// Running jobs and the log are kept: they are local processes.
pub fn leave(m: &mut Model) {
    match &mut m.areas.vm_tools.external.panel {
        Some(Panel::Launch(_)) => m.areas.vm_tools.external.panel = None,
        Some(Panel::Configure(p)) => p.returning = None,
        None => {}
    }
}

pub fn is_open(m: &Model) -> bool {
    m.areas.vm_tools.external.panel.is_some()
}

// -- Description ----------------------------------------------------------------

pub fn surface(m: &Model) -> Option<Surface> {
    let surface = match m.areas.vm_tools.external.panel.as_ref()? {
        Panel::Launch(p) => Surface::dialog(LAUNCH, "Guest SSH, RDP and local tools")
            .subtitle(p.guest.name.clone())
            .body(launch_body(m, p))
            .footer(launch_footer(m, p)),
        Panel::Configure(p) => Surface::dialog(CONFIGURE, "Configure external tools")
            .body(configure_body(m, p))
            .footer(configure_footer(p)),
    };
    Some(surface.experimental(true).large().on_close(back))
}

/// The last eight lines of the job log, newest first.
fn history(m: &Model) -> Option<Text> {
    let lines: Vec<&str> = m
        .areas
        .vm_tools
        .external
        .history
        .iter()
        .rev()
        .take(8)
        .map(String::as_str)
        .collect();
    (!lines.is_empty()).then(|| preformatted(lines.join("\n")).key("guest-tool-history"))
}

fn edited(change: impl Fn(&mut Launch, &str) + 'static) -> impl Fn(&mut Model, &str) + 'static {
    move |m, text| {
        if let Some(p) = launch_mut(m) {
            change(p, text);
        }
        drop_stale_review(m);
    }
}

fn pick_tool(m: &mut Model, id: &str) {
    let tools = &m.preferences.external_tools.tools;
    let Some((id, kind)) = tools
        .iter()
        .find(|t| t.id.to_string() == id)
        .map(|t| (t.id, t.kind))
    else {
        return;
    };
    if let Some(p) = launch_mut(m) {
        p.tool = id;
        p.port = default_port(kind).into();
        p.review = None;
    }
}

fn launch_body(m: &Model, p: &Launch) -> Vec<Block> {
    let tools = &m.preferences.external_tools.tools;
    let options: Vec<OptionItem> = tools
        .iter()
        .map(|tool| option(tool.id.to_string(), &tool.name).key(format!("guest-tool-{}", tool.id)))
        .collect();
    let selected = p.tool.to_string();
    let tool = choice("guest-tool-choice", options, Some(selected.as_str()), pick_tool).label("Local tool");
    let tool = if tools.len() <= 4 {
        tool.segmented()
    } else {
        tool.dropdown()
    };
    let reported: Vec<Button> = m
        .inventory_for(p.guest.pool)
        .map(|i| addresses(&i, p.guest.vm))
        .unwrap_or_default()
        .into_iter()
        .map(|ip| {
            button(
                format!("guest-tool-use-{ip}"),
                format!("Use {ip}"),
                move |m| {
                    if let Some(p) = launch_mut(m) {
                        p.address = ip.to_string();
                        p.reported = true;
                        p.review = None;
                    }
                },
            )
        })
        .collect();
    let command = p.review.as_ref().map(|review| {
        let lines = std::iter::once(review.command.executable.display().to_string()).chain(
            review.command.arguments.iter().enumerate().map(|(n, arg)| {
                format!(
                    "Argument {}: {}",
                    n + 1,
                    serde_json::to_string(arg).unwrap_or_default()
                )
            }),
        );
        blocks![
            heading("Exact local command"),
            crate::surface::review("guest-tool-command", lines),
            caption(TRUST),
        ]
    });
    blocks![
        caption(format!("Pool {} · VM {}", p.guest.pool, p.guest.vm)),
        caption(SECURITY),
        tool,
        actions([button(
            "configure-guest-tools",
            "Configure local tools…",
            configure
        )]),
        (!reported.is_empty()).then(|| actions(reported)),
        input(
            "guest-tool-address",
            "Guest IP address",
            &p.address,
            edited(|p, text| p.address = text.to_owned()),
        )
        .on_submit(review),
        caption(if p.reported {
            "Source: current guest tools report"
        } else {
            "Source: manually supplied address; verify it belongs to this guest"
        })
        .key("guest-tool-source"),
        p.reported.then(|| actions([button(
            "guest-tool-manual-address",
            "Use a manually supplied address",
            |m| {
                if let Some(p) = launch_mut(m) {
                    p.reported = false;
                    p.review = None;
                }
            },
        )])),
        input(
            "guest-tool-port",
            "Port",
            &p.port,
            edited(|p, text| p.port = text.to_owned()),
        )
        .on_submit(review),
        input(
            "guest-tool-user",
            "Guest username (separate from host login)",
            &p.user,
            edited(|p, text| p.user = text.to_owned()),
        )
        .on_submit(review),
        command,
        p.error
            .clone()
            .map(|error| text(error).key("guest-tool-status")),
        history(m),
    ]
}

/// Why "Launch reviewed tool" is disabled: the refusals of [`launch`] that
/// are known without doing anything, in its order.
fn launch_refusal(m: &Model, p: &Launch) -> Option<&'static str> {
    if synthetic(m, p) {
        Some(NO_LAUNCH_IN_DEMO)
    } else if !current_workspace(m, p) {
        Some(CONNECTION_CHANGED)
    } else if p.review.is_none() {
        Some(REVIEW_FIRST)
    } else if m.areas.vm_tools.external.jobs.len() >= 8 {
        Some(TOO_MANY)
    } else {
        None
    }
}

fn launch_footer(m: &Model, p: &Launch) -> Vec<Button> {
    let review = button("review-guest-tool", "Review command", review)
        .disabled_if(p.pending.is_some(), INSPECTING);
    let launch = button("launch-guest-tool", "Launch reviewed tool", launch)
        .disabled_opt(launch_refusal(m, p));
    let (review, launch) = if p.review.is_some() {
        (review, launch.primary())
    } else {
        (review.primary(), launch)
    };
    vec![
        button("close-guest-tools", "Close", back),
        review,
        launch,
    ]
}

fn configure_body(m: &Model, p: &Configure) -> Vec<Block> {
    let tools = &p.catalog.tools;
    let options: Vec<OptionItem> = tools
        .iter()
        .enumerate()
        .map(|(index, tool)| option(index.to_string(), &tool.name))
        .collect();
    let selected = p.index.map(|index| index.to_string());
    let declared = (!tools.is_empty()).then(|| {
        let declared = choice("guest-tool-edit", options, selected.as_deref(), |m, id| {
            if let Ok(index) = id.parse() {
                edit(m, index);
            }
        })
        .label("Declared tools");
        if tools.len() <= 4 {
            declared.segmented()
        } else {
            declared.dropdown()
        }
    });
    let full = tools.len() >= command::MAX_TOOLS;
    let adder = |key: &str, label: &str, kind: Kind| {
        button(key, label, move |m| add(m, kind)).disabled_if(full, FULL)
    };
    let fields = p.index.is_some().then(|| {
        blocks![
            input("guest-tool-name", "Tool name", &p.name, |m, text| {
                if let Some(p) = config_mut(m) {
                    p.name = text.to_owned();
                }
            }),
            input(
                "guest-tool-executable",
                "Executable (absolute path)",
                &p.executable,
                |m, text| {
                    if let Some(p) = config_mut(m) {
                        p.executable = text.to_owned();
                    }
                }
            ),
            textarea(
                "guest-tool-arguments",
                "Argument strings (JSON)",
                &p.arguments,
                |m, text| {
                    if let Some(p) = config_mut(m) {
                        p.arguments = text.to_owned();
                    }
                }
            ),
            caption(PLACEHOLDERS),
        ]
    });
    blocks![
        caption(DECLARATIONS),
        declared,
        actions([
            adder("add-guest-tool-ssh", "Add SSH", Kind::Ssh),
            adder("add-guest-tool-rdp", "Add RDP", Kind::Rdp),
            adder("add-guest-tool-custom", "Add local tool", Kind::Custom),
        ]),
        fields,
        p.error
            .clone()
            .map(|error| text(error).key("guest-tool-status")),
        history(m),
    ]
}

fn configure_footer(p: &Configure) -> Vec<Button> {
    blocks_of([
        Some(button("close-guest-tools", "Close", back)),
        p.index
            .is_some()
            .then(|| button("remove-guest-tool", "Remove tool", remove)),
        Some(button("save-guest-tools", "Save tool declarations", save).primary()),
    ])
}

fn blocks_of<const N: usize>(buttons: [Option<Button>; N]) -> Vec<Button> {
    buttons.into_iter().flatten().collect()
}
