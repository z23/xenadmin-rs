use super::*;
#[path = "external_tool_command.rs"]
pub(super) mod command;
use command::{Catalog, Command, Context, Kind, Tool};
use std::{
    collections::BTreeSet,
    net::IpAddr,
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc,
    },
};

#[derive(Default)]
pub(super) struct Workspace {
    panel: Option<Panel>,
    jobs: Vec<Job>,
    history: Vec<String>,
}
enum Panel {
    Launch(Box<Launch>),
    Configure(Box<Configure>),
}
#[derive(Clone)]
struct Target {
    pool: PoolId,
    vm: VmId,
    generation: u64,
    name: String,
    connection: Option<Connection>,
}
struct Launch {
    target: Target,
    tool: uuid::Uuid,
    address: TextEdit,
    port: TextEdit,
    user: TextEdit,
    reported: bool,
    review: Option<Review>,
    pending: Option<Preparation>,
    error: Option<String>,
}
#[derive(Clone, PartialEq, Eq)]
struct Inputs {
    tool: Tool,
    context: Context,
    reported: bool,
}
struct Preparation {
    inputs: Inputs,
    receiver: mpsc::Receiver<Result<Review, String>>,
}
struct Review {
    inputs: Inputs,
    command: Command,
}
struct Configure {
    catalog: Catalog,
    index: Option<usize>,
    name: TextEdit,
    executable: TextEdit,
    arguments: TextEdit,
    returning: Option<Box<Launch>>,
    error: Option<String>,
}
struct Job {
    label: String,
    receiver: mpsc::Receiver<String>,
}
static RUNNING: AtomicUsize = AtomicUsize::new(0);
struct Permit;
impl Drop for Permit {
    fn drop(&mut self) {
        RUNNING.fetch_sub(1, Ordering::SeqCst);
    }
}
fn permit() -> Result<Permit, String> {
    RUNNING.try_update(Ordering::SeqCst,Ordering::SeqCst,|v|(v<8).then_some(v+1)).map(|_|Permit).map_err(|_|"Eight local tool processes are already being tracked. Close one before launching another.".into())
}
fn addresses(i: &Inventory, vm: VmId) -> BTreeSet<IpAddr> {
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
fn ordinary(i: &Inventory, vm: VmId) -> bool {
    i.vms.get(&vm).is_some_and(|v| {
        !v.is_template && !v.is_control_domain && v.power_state == PowerState::Running
    })
}
impl Target {
    fn verify(&self, ip: IpAddr, reported: bool) -> Result<(), String> {
        let snapshot = self
            .connection
            .as_ref()
            .ok_or("Demo mode cannot launch external programs")?
            .latest();
        self.observed(
            snapshot.is_current(),
            snapshot.generation(),
            snapshot.inventory().map(AsRef::as_ref),
            ip,
            reported,
        )
    }
    fn observed(
        &self,
        current: bool,
        generation: u64,
        inventory: Option<&Inventory>,
        ip: IpAddr,
        reported: bool,
    ) -> Result<(), String> {
        let i = inventory
            .filter(|i| current && generation == self.generation && i.pool_id == self.pool)
            .ok_or("Connection changed or inventory is stale. Close and reopen Guest tools.")?;
        if !ordinary(i, self.vm) {
            return Err("The captured guest is unavailable or no longer running".into());
        }
        if reported && !addresses(i, self.vm).contains(&ip) {
            return Err("The selected address is no longer reported by the guest. Review its current addresses.".into());
        }
        Ok(())
    }
}
pub(super) fn button(s: &State, k: &Kit) -> El {
    let available = s
        .selected()
        .and_then(|v| v.vm)
        .zip(s.inventory.as_ref())
        .is_some_and(|(vm, i)| ordinary(i, vm));
    Button::new("Guest SSH, RDP and tools…")
        .disabled(!available)
        .on_press(open)
        .build(k)
        .key("guest-tools")
}
fn open(s: &mut State) {
    let Some((vm, i)) = s
        .selected()
        .and_then(|r| r.vm)
        .zip(s.inventory.as_ref())
        .filter(|(vm, i)| ordinary(i, *vm))
    else {
        return;
    };
    let Some(tool) = s.preferences.external_tools.tools.first() else {
        configure(s);
        return;
    };
    let ip = addresses(i, vm).into_iter().next();
    let target = Target {
        pool: i.pool_id,
        vm,
        generation: s.snapshot.as_ref().map_or(0, |s| s.generation()),
        name: i.vms[&vm].name.clone(),
        connection: s.connection.clone(),
    };
    s.external_tools.panel = Some(Panel::Launch(Box::new(Launch {
        target,
        tool: tool.id,
        address: TextEdit::new(ip.map(|v| v.to_string()).unwrap_or_default()),
        port: TextEdit::new(if tool.kind == Kind::Rdp { "3389" } else { "22" }),
        user: TextEdit::new(""),
        reported: ip.is_some(),
        review: None,
        pending: None,
        error: None,
    })));
    console_view::release(s);
}
fn launch_mut(s: &mut State) -> &mut Launch {
    match s.external_tools.panel.as_mut().unwrap() {
        Panel::Launch(p) => p,
        _ => unreachable!(),
    }
}
fn config_mut(s: &mut State) -> &mut Configure {
    match s.external_tools.panel.as_mut().unwrap() {
        Panel::Configure(p) => p,
        _ => unreachable!(),
    }
}
fn inputs(s: &State, p: &Launch) -> Result<Inputs, String> {
    let tool = s
        .preferences
        .external_tools
        .tools
        .iter()
        .find(|t| t.id == p.tool)
        .cloned()
        .ok_or("The tool declaration changed. Select it again.")?;
    let ip = p
        .address
        .text
        .trim()
        .parse::<IpAddr>()
        .map_err(|_| "Enter a literal IPv4 or IPv6 guest address")?;
    let port = p
        .port
        .text
        .parse::<u16>()
        .ok()
        .filter(|p| *p > 0)
        .ok_or("Port must be from 1 to 65535")?;
    Ok(Inputs {
        tool,
        context: Context {
            ip,
            port,
            user: p.user.text.clone(),
            vm: p.target.vm,
            pool: p.target.pool,
        },
        reported: p.reported,
    })
}
fn current_workspace(s: &State, p: &Launch) -> bool {
    s.demo
        || s.snapshot.as_ref().is_some_and(|v| {
            v.is_current()
                && v.generation() == p.target.generation
                && v.inventory().is_some_and(|i| i.pool_id == p.target.pool)
        })
}
fn review(s: &mut State) {
    let result = (|| {
        let Some(Panel::Launch(p)) = &s.external_tools.panel else {
            return Err("No guest selected".into());
        };
        if !current_workspace(s, p) {
            return Err("Connection changed. Close and reopen Guest tools.".into());
        }
        let inputs = inputs(s, p)?;
        if !s.demo {
            p.target.verify(inputs.context.ip, inputs.reported)?;
        }
        let token = permit()?;
        let prepared = inputs.clone();
        let (sender, receiver) = mpsc::channel();
        let waker = s.waker.clone();
        std::thread::Builder::new()
            .name("tool-review".into())
            .spawn(move || {
                let _permit = token;
                let result =
                    Command::prepare(&prepared.tool, &prepared.context).map(|command| Review {
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
    let p = launch_mut(s);
    p.review = None;
    match result {
        Ok(pending) => {
            p.pending = Some(pending);
            p.error = Some("Inspecting the configured local executable…".into());
        }
        Err(error) => {
            p.pending = None;
            p.error = Some(error);
        }
    }
}

fn launch(s: &mut State) {
    let result = (|| {
        if s.demo {
            return Err("Demo mode cannot launch external programs".into());
        }
        let Some(Panel::Launch(p)) = &s.external_tools.panel else {
            return Err("No guest selected".into());
        };
        if !current_workspace(s, p) {
            return Err("Connection changed. Close and reopen Guest tools.".into());
        }
        let review = p.review.as_ref().ok_or("Review the command first")?;
        let current = inputs(s, p)?;
        if current != review.inputs {
            return Err("Inputs changed. Review the command again.".into());
        }
        p.target.verify(current.context.ip, current.reported)?;
        let token = permit()?;
        let target = p.target.clone();
        let command = review.command.clone();
        let label = format!(
            "{} · {} · {}",
            current.tool.name, target.name, current.context.ip
        );
        let (sender, receiver) = mpsc::channel();
        let waker = s.waker.clone();
        std::thread::Builder::new().name("guest-tool".into()).spawn(move||{
            let _permit=token;
            let send=|status:String|{let _=sender.send(status);if let Some(w)=&waker{w();}};
            let result=command.spawn(||target.verify(current.context.ip,current.reported));
            match result {
                Ok(mut child)=>{send(format!("Launched process {}. Authentication and connection status are shown by the external client.",child.id()));
                    send(match child.wait(){Ok(status)=>format!("Launcher exited ({status}). A separate client window may still be open."),Err(_)=>"Cannot observe the launched process. Check the external client.".into()});},
                Err(error)=>send(error),
            }
        }).map_err(|_|"Cannot start the local tool worker")?;
        Ok(Job { label, receiver })
    })();
    match result {
        Ok(job) => {
            s.external_tools.jobs.push(job);
            let p = launch_mut(s);
            p.review = None;
            p.error = Some("Launching the reviewed local program…".into());
        }
        Err(e) => launch_mut(s).error = Some(e),
    }
}
pub(super) fn poll(s: &mut State) -> bool {
    let mut changed = false;
    let mut history = vec![];
    s.external_tools.jobs.retain(|job| {
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
    s.external_tools.history.extend(history);
    if s.external_tools.history.len() > 32 {
        s.external_tools
            .history
            .drain(..s.external_tools.history.len() - 32);
    }
    let prepared = if let Some(Panel::Launch(p)) = &s.external_tools.panel {
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
    } else {
        None
    };
    if let Some((captured, result)) = prepared {
        let valid = if let Some(Panel::Launch(p)) = &s.external_tools.panel {
            current_workspace(s, p) && inputs(s, p).ok().as_ref() == Some(&captured)
        } else {
            false
        };
        let p = launch_mut(s);
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
            p.error = Some("Inputs changed while inspecting the executable. Review again.".into());
        }
        changed = true;
    }
    if let Some(Panel::Launch(p)) = &s.external_tools.panel
        && p.review.is_some()
        && (!current_workspace(s, p)
            || inputs(s, p).is_err()
            || inputs(s, p).ok().as_ref() != p.review.as_ref().map(|r| &r.inputs))
    {
        let p = launch_mut(s);
        p.review = None;
        p.error = Some("Selection or command inputs changed. Review again before launch.".into());
        changed = true;
    }
    changed
}
pub(super) fn configure(s: &mut State) {
    let returning = match s.external_tools.panel.take() {
        Some(Panel::Launch(p)) => Some(p),
        _ => None,
    };
    let catalog = s.preferences.external_tools.clone();
    s.external_tools.panel = Some(Panel::Configure(Box::new(Configure {
        catalog,
        index: None,
        name: TextEdit::new(""),
        executable: TextEdit::new(""),
        arguments: TextEdit::new("[]"),
        returning,
        error: None,
    })));
    s.application_panel = None;
    if !config_mut(s).catalog.tools.is_empty() {
        edit(s, 0);
    }
}
fn edit(s: &mut State, index: usize) {
    let p = config_mut(s);
    if let Some(tool) = p.catalog.tools.get(index) {
        p.index = Some(index);
        p.name = TextEdit::new(&tool.name);
        p.executable = TextEdit::new(&tool.executable);
        p.arguments = TextEdit::new(serde_json::to_string_pretty(&tool.arguments).unwrap());
        p.error = None;
    }
}
fn add(s: &mut State, kind: Kind) {
    let p = config_mut(s);
    if p.catalog.tools.len() < command::MAX_TOOLS {
        p.catalog.tools.push(Tool::preset(kind));
        let index = p.catalog.tools.len() - 1;
        edit(s, index);
    }
}
fn save(s: &mut State) {
    let result = (|| {
        let p = config_mut(s);
        if p.index.is_none() {
            return Ok(p.catalog.clone());
        }
        let index = p.index.unwrap();
        let tool = p
            .catalog
            .tools
            .get_mut(index)
            .ok_or("Tool selection changed")?;
        if p.arguments.text.len() > 20_000 {
            return Err("Argument declaration is too large".into());
        }
        let arguments: Vec<String> = serde_json::from_str(&p.arguments.text)
            .map_err(|_| "Arguments must be a JSON array of separate strings")?;
        let updated = Tool {
            id: tool.id,
            kind: tool.kind,
            name: p.name.text.clone(),
            executable: p.executable.text.clone(),
            arguments,
        };
        updated.validate()?;
        *tool = updated;
        if !p.catalog.valid() {
            return Err("Invalid local tool declarations".into());
        }
        Ok(p.catalog.clone())
    })();
    match result {
        Ok(catalog) => {
            if (s._instance.is_none() || s.demo) && !cfg!(test) {
                config_mut(s).error = Some("This preview cannot save tool declarations".into());
                return;
            }
            let old = std::mem::replace(&mut s.preferences.external_tools, catalog);
            let result = if cfg!(test) {
                Ok(())
            } else {
                s.preferences.save()
            };
            if let Err(error) = result {
                s.preferences.external_tools = old;
                config_mut(s).error = Some(error);
            } else {
                config_mut(s).error =
                    Some("Tool declarations saved. Each launch still requires review.".into());
            }
        }
        Err(e) => config_mut(s).error = Some(e),
    }
}
fn back(s: &mut State) {
    let panel = s.external_tools.panel.take();
    if let Some(Panel::Configure(p)) = panel
        && let Some(mut returning) = p.returning
    {
        returning.review = None;
        returning.pending = None;
        s.external_tools.panel = Some(Panel::Launch(returning));
    }
}
pub(super) fn clear(s: &mut State) {
    s.external_tools = Workspace::default();
}
pub(super) fn is_open(s: &State) -> bool {
    s.external_tools.panel.is_some()
}
fn input(label: &str, edit: &TextEdit, accessor: fn(&mut State) -> &mut TextEdit, k: &Kit) -> El {
    col().gap(6.0).child(caption(label, k)).child(
        TextInput::new(edit)
            .label(label)
            .on_edit(accessor)
            .build(k)
            .w_full(),
    )
}
pub(super) fn dialog(s: &State, k: &Kit) -> El {
    let Some(panel) = &s.external_tools.panel else {
        return el();
    };
    let mut body = col().gap(12.0);
    let mut footer = row().gap(8.0).flex_wrap();
    let title = match panel {
        Panel::Launch(p) => {
            body=body.child(caption(format!("{}\nPool {} · VM {}",p.target.name,p.target.pool,p.target.vm),k).wrap_anywhere())
                .child(caption("Connect from this workstation to a guest address. Guest-reported addresses are not proof of server identity; the external client's SSH host-key or RDP certificate checks still apply. Guest passwords are entered in that client.",k).wrap());
            let mut choices = row().gap(6.0).flex_wrap();
            for tool in &s.preferences.external_tools.tools {
                let id = tool.id;
                let kind = tool.kind;
                choices = choices.child(
                    Button::new(&tool.name)
                        .variant(if id == p.tool {
                            ButtonVariant::Neutral
                        } else {
                            ButtonVariant::Outline
                        })
                        .on_press(move |s: &mut State| {
                            let p = launch_mut(s);
                            p.tool = id;
                            p.port = TextEdit::new(if kind == Kind::Rdp { "3389" } else { "22" });
                            p.review = None;
                        })
                        .build(k)
                        .key(format!("guest-tool-{id}")),
                );
            }
            body = body.child(choices).child(
                Button::new("Configure local tools…")
                    .on_press(configure)
                    .build(k)
                    .key("configure-guest-tools"),
            );
            if let Some(i) = s.inventory.as_ref().filter(|i| i.pool_id == p.target.pool) {
                let mut choices = row().gap(6.0).flex_wrap();
                for ip in addresses(i, p.target.vm) {
                    choices = choices.child(
                        Button::new(format!("Use {ip}"))
                            .on_press(move |s: &mut State| {
                                let p = launch_mut(s);
                                p.address = TextEdit::new(ip.to_string());
                                p.reported = true;
                                p.review = None;
                            })
                            .build(k),
                    );
                }
                body = body.child(choices);
            }
            body = body
                .child(
                    input(
                        "Guest IP address",
                        &p.address,
                        |s| &mut launch_mut(s).address,
                        k,
                    )
                    .key("guest-tool-address"),
                )
                .child(
                    caption(
                        if p.reported {
                            "Source: current guest tools report"
                        } else {
                            "Source: manually supplied address; verify it belongs to this guest"
                        },
                        k,
                    )
                    .wrap(),
                )
                .children(p.reported.then(|| {
                    Button::new("Use a manually supplied address")
                        .on_press(|s: &mut State| {
                            let p = launch_mut(s);
                            p.reported = false;
                            p.review = None;
                        })
                        .build(k)
                        .key("guest-tool-manual-address")
                }))
                .child(
                    input("Port", &p.port, |s| &mut launch_mut(s).port, k).key("guest-tool-port"),
                )
                .child(
                    input(
                        "Guest username (separate from host login)",
                        &p.user,
                        |s| &mut launch_mut(s).user,
                        k,
                    )
                    .key("guest-tool-user"),
                );
            if let Some(review) = &p.review {
                body = body.child(heading("Exact local command", k)).child(
                    caption(review.command.executable.display().to_string(), k).wrap_anywhere(),
                );
                for (n, arg) in review.command.arguments.iter().enumerate() {
                    body = body.child(
                        caption(
                            format!(
                                "Argument {}: {}",
                                n + 1,
                                serde_json::to_string(arg).unwrap()
                            ),
                            k,
                        )
                        .wrap_anywhere(),
                    );
                }
                body=body.child(caption("This trusted local executable runs as your Linux user. Arguments are passed separately; no command shell is invoked. Only desktop/locale and SSH-agent environment values are inherited; XAPI credentials and sessions are never supplied. Closing this dialog does not close the external client.",k).wrap());
            }
            if let Some(error) = &p.error {
                body = body.child(caption(error, k).wrap());
            }
            footer = footer
                .child(
                    Button::new("Review command")
                        .disabled(p.pending.is_some())
                        .on_press(review)
                        .build(k)
                        .key("review-guest-tool"),
                )
                .child(
                    Button::new("Launch reviewed tool")
                        .disabled(
                            s.demo
                                || p.review.is_none()
                                || !current_workspace(s, p)
                                || s.external_tools.jobs.len() >= 8,
                        )
                        .on_press(launch)
                        .build(k)
                        .key("launch-guest-tool"),
                );
            "Guest SSH, RDP and local tools"
        }
        Panel::Configure(p) => {
            body=body.child(caption("Trusted local programs, not Windows plugins. Use an absolute executable path and a JSON array with one string per argument. Nothing launches automatically. Never put passwords or tokens in declarations; arguments can be visible to other local programs.",k).wrap());
            let mut choices = row().gap(6.0).flex_wrap();
            for (index, tool) in p.catalog.tools.iter().enumerate() {
                choices = choices.child(
                    Button::new(&tool.name)
                        .on_press(move |s: &mut State| edit(s, index))
                        .build(k),
                );
            }
            body = body.child(choices);
            let mut adders = row().gap(6.0).flex_wrap();
            for (name, kind) in [
                ("Add SSH", Kind::Ssh),
                ("Add RDP", Kind::Rdp),
                ("Add local tool", Kind::Custom),
            ] {
                adders = adders.child(
                    Button::new(name)
                        .disabled(p.catalog.tools.len() >= command::MAX_TOOLS)
                        .on_press(move |s: &mut State| add(s, kind))
                        .build(k),
                );
            }
            body = body.child(adders);
            footer = footer.child(
                Button::new("Save tool declarations")
                    .on_press(save)
                    .build(k)
                    .key("save-guest-tools"),
            );
            if p.index.is_some() {
                body=body.child(input("Tool name",&p.name,|s|&mut config_mut(s).name,k).key("guest-tool-name"))
                .child(input("Executable (absolute path)",&p.executable,|s|&mut config_mut(s).executable,k).key("guest-tool-executable"))
                .child(caption("Argument strings (JSON)",k)).child(rune_kit::TextArea::new(&p.arguments).label("Argument strings (JSON)").on_edit(|s:&mut State|&mut config_mut(s).arguments).build(k).h(180.0).key("guest-tool-arguments"))
                .child(caption("Placeholders: {ip}, {port}, {user}, {vm_uuid}, {pool_uuid}, {rdp_uri}. Substitution never creates extra arguments. SSH preset needs xdg-terminal-exec and OpenSSH; RDP preset needs Remmina. Set the executable and arguments for your installed clients.",k).wrap());
                footer = footer.child(
                    Button::new("Remove tool")
                        .on_press(|s: &mut State| {
                            let p = config_mut(s);
                            if let Some(index) = p.index.take() {
                                p.catalog.tools.remove(index);
                            }
                            if !p.catalog.tools.is_empty() {
                                edit(s, 0);
                            }
                        })
                        .build(k)
                        .key("remove-guest-tool"),
                );
            }
            if let Some(error) = &p.error {
                body = body.child(caption(error, k).wrap());
            }
            "Configure external tools"
        }
    };
    for item in s.external_tools.history.iter().rev().take(8) {
        body = body.child(caption(item, k).wrap_anywhere());
    }
    footer = footer.child(
        Button::new("Close")
            .on_press(back)
            .build(k)
            .key("close-guest-tools"),
    );
    col()
        .absolute(0.0, 0.0)
        .w_full()
        .h_full()
        .fill(rune::Color::BLACK.alpha(0.5))
        .center()
        .role("dialog")
        .label(title)
        .modal(false, |s: &mut State| back(s))
        .child(
            col()
                .w((s.window_width - 32.0).clamp(280.0, 900.0))
                .h((s.window_height - 40.0).clamp(300.0, 850.0))
                .padding(20.0)
                .gap(12.0)
                .fill(k.c.bg_base)
                .radius(10.0)
                .child(heading(title, k))
                .child(body.grow(1.0).min_h(0.0).scroll(rune::ScrollSpec {
                    stick_bottom: false,
                }))
                .child(footer),
        )
}
pub(super) fn scene(s: &mut State, configure_only: bool) {
    demo(s);
    s.preferences.external_tools = Catalog::default();
    if configure_only {
        configure(s);
        return;
    }
    let id = s
        .inventory
        .as_ref()
        .unwrap()
        .guests()
        .find(|v| v.power_state == PowerState::Running)
        .unwrap()
        .id;
    s.table.select(id.to_string());
    open(s);
    if configure_only {
        configure(s);
    } else {
        let p = launch_mut(s);
        p.address = TextEdit::new("192.0.2.23");
        p.user = TextEdit::new("operator");
    }
}

#[cfg(test)]
#[path = "external_tools_tests.rs"]
mod tests;
