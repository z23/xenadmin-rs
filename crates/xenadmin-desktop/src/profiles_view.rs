use super::*;
use xenadmin_core::{
    ConnectionProfile, CredentialKey, CredentialMode, CredentialOutcome, ProfileId, ProfileStore,
};

#[path = "profile_credentials.rs"]
mod credentials;
#[path = "profile_pool_join.rs"]
mod pool_join;
#[path = "profile_reconnect.rs"]
mod reconnect;
#[path = "profile_trust.rs"]
mod trust;
use credentials::CredentialJob;

#[derive(Clone, Copy, PartialEq, Eq)]
enum LoadPurpose {
    Editor,
    Connect,
}
#[derive(Clone)]
struct CredentialLoad {
    id: uuid::Uuid,
    profile: ConnectionProfile,
    purpose: LoadPurpose,
}

enum CredentialWork {
    Load(Box<CredentialLoad>),
    Save(Box<ConnectionProfile>),
    Forget(ProfileId),
    Remove(ProfileId),
}

pub(super) fn inspection_connections(
    s: &State,
) -> impl Iterator<Item = (ProfileId, Connection)> + '_ {
    s.workspace
        .active
        .iter()
        .map(|(id, connection)| (*id, connection.clone()))
}

pub(super) struct Workspace {
    trust: trust::Panel,
    proxy_sessions: BTreeMap<
        ProfileId,
        (
            xenadmin_core::ProxyConfiguration,
            xenadmin_core::ProxyCredentials,
        ),
    >,
    join: Option<pool_join::Editor>,
    reconnect: Option<reconnect::Editor>,
    session_accounts: BTreeMap<ProfileId, (String, bool)>,
    store: Result<ProfileStore, String>,
    writable: bool,
    preview_profiles: Vec<ConnectionProfile>,
    active: BTreeMap<ProfileId, Connection>,
    seen: BTreeMap<ProfileId, Arc<ConnectionSnapshot>>,
    selected: Option<ProfileId>,
    editing: Option<ProfileId>,
    editor_epoch: uuid::Uuid,
    pub(super) show_manager: bool,
    name: TextEdit,
    ca_certificate: TextEdit,
    timeout: TextEdit,
    read_only: bool,
    advanced: bool,
    remember_password: bool,
    auto_connect: bool,
    restore_started: bool,
    loads: BTreeMap<ProfileId, CredentialLoad>,
    load_queue: std::collections::VecDeque<ProfileId>,
    profile_status: BTreeMap<ProfileId, String>,
    credential_tasks: Vec<(CredentialWork, CredentialJob)>,
    cached_credentials: Option<(ConnectionProfile, Credentials)>,
    pending_credentials: BTreeMap<ProfileId, Credentials>,
    credential_status: Option<String>,
}
impl Workspace {
    pub(super) fn load(selected: Option<ProfileId>, writable: bool) -> Self {
        let store = if writable {
            preferences::directory()
                .ok_or_else(|| "No application state directory is available".to_owned())
                .and_then(|directory| {
                    ProfileStore::open(directory).map_err(|error| error.to_string())
                })
        } else {
            Err("Another XenAdmin RS window owns this state directory. Close it before changing connections here.".into())
        };
        Self {
            trust: Default::default(),
            proxy_sessions: BTreeMap::new(),
            reconnect: None,
            join: None,
            session_accounts: BTreeMap::new(),
            store,
            writable,
            preview_profiles: Vec::new(),
            active: BTreeMap::new(),
            seen: BTreeMap::new(),
            selected,
            editing: None,
            editor_epoch: uuid::Uuid::new_v4(),
            show_manager: false,
            name: TextEdit::new(""),
            ca_certificate: TextEdit::new(""),
            timeout: TextEdit::new("45"),
            read_only: false,
            advanced: false,
            remember_password: false,
            auto_connect: false,
            restore_started: false,
            loads: BTreeMap::new(),
            load_queue: std::collections::VecDeque::new(),
            profile_status: BTreeMap::new(),
            credential_tasks: Vec::new(),
            cached_credentials: None,
            pending_credentials: BTreeMap::new(),
            credential_status: None,
        }
    }
    fn profile(&self, id: ProfileId) -> Option<&ConnectionProfile> {
        self.store
            .as_ref()
            .ok()
            .and_then(|store| store.get(id))
            .or_else(|| {
                self.preview_profiles
                    .iter()
                    .find(|profile| profile.id == id)
            })
    }
    fn live(&self, id: ProfileId) -> bool {
        self.active.get(&id).is_some_and(|connection| {
            !matches!(
                connection.latest().state(),
                ConnectionState::Disconnected | ConnectionState::Failed
            )
        })
    }
}

pub(super) fn apply_initial(s: &mut State) {
    if let Some(profile) = s
        .workspace
        .selected
        .and_then(|id| s.workspace.profile(id))
        .cloned()
    {
        populate(s, &profile);
    } else {
        s.workspace.selected = None;
    }
}

fn populate(s: &mut State, profile: &ConnectionProfile) {
    trust::populate(s, profile);
    s.workspace.editor_epoch = uuid::Uuid::new_v4();
    s.workspace.remember_password = profile.credential_mode == CredentialMode::SecretService;
    s.workspace.auto_connect = profile.auto_connect;
    s.workspace.cached_credentials = None;
    s.workspace.credential_status = None;
    s.workspace.editing = Some(profile.id);
    s.workspace.name = TextEdit::new(&profile.name);
    s.workspace.ca_certificate = TextEdit::new(
        profile
            .ca_certificate
            .as_ref()
            .map(|path| path.to_string_lossy().into_owned())
            .unwrap_or_default(),
    );
    s.workspace.timeout = TextEdit::new(profile.request_timeout_seconds.to_string());
    s.workspace.read_only = profile.read_only;
    s.endpoint = TextEdit::new(profile.endpoint.as_str());
    s.username = TextEdit::new(&profile.username);
    s.password = secure_field();
    s.fingerprint = TextEdit::new(
        profile
            .host_certificates
            .get(profile.endpoint.as_str())
            .map(ToString::to_string)
            .unwrap_or_default(),
    );
    s.host_certificates = TextEdit::new(
        profile
            .host_certificates
            .iter()
            .filter(|(endpoint, _)| endpoint.as_str() != profile.endpoint.as_str())
            .map(|(endpoint, pin)| format!("{endpoint}={pin}"))
            .collect::<Vec<_>>()
            .join("\n"),
    );
    s.accept_invalid_certificates = profile.accept_invalid_certificates;
    s.show_certificate_details = false;
}

pub(super) fn begin_new(s: &mut State) {
    cancel_editor(s);
    s.workspace.trust = Default::default();
    s.workspace.editor_epoch = uuid::Uuid::new_v4();
    s.workspace.remember_password = false;
    s.workspace.auto_connect = false;
    s.workspace.cached_credentials = None;
    s.workspace.credential_status = None;
    s.workspace.editing = None;
    s.workspace.name = TextEdit::new("");
    s.workspace.ca_certificate = TextEdit::new("");
    s.workspace.timeout = TextEdit::new("45");
    s.workspace.read_only = false;
    s.endpoint = TextEdit::new("");
    s.username = TextEdit::new("root");
    s.password = secure_field();
    s.fingerprint = TextEdit::new("");
    s.host_certificates = TextEdit::new("");
    s.accept_invalid_certificates = false;
    s.show_certificate_details = false;
    s.show_connect = true;
    s.workspace.show_manager = false;
    s.error = None;
}

fn edit(s: &mut State, id: ProfileId) {
    if s.workspace.live(id) {
        s.error = Some("Disconnect this connection before editing its settings.".into());
        return;
    }
    if let Some(profile) = s.workspace.profile(id).cloned() {
        cancel_load(s, id);
        cancel_editor(s);
        populate(s, &profile);
        s.show_connect = true;
        if profile.credential_mode == CredentialMode::SecretService {
            load_password(s, &profile);
        }
        s.show_connect = true;
        s.workspace.show_manager = false;
        s.error = None;
    }
}

fn form_profile(s: &State) -> Result<ConnectionProfile, String> {
    let endpoint = Endpoint::parse(&s.endpoint.text).map_err(|error| error.to_string())?;
    let mut profile = s
        .workspace
        .editing
        .and_then(|id| s.workspace.profile(id))
        .cloned()
        .unwrap_or_else(|| ConnectionProfile::new(String::new(), endpoint.clone(), String::new()));
    if s.workspace.live(profile.id) {
        return Err("Disconnect this connection before editing its settings.".into());
    }
    profile.set_endpoint(endpoint);
    profile.name = if s.workspace.name.text.trim().is_empty() {
        profile.endpoint.to_string()
    } else {
        s.workspace.name.text.trim().into()
    };
    profile.username = s.username.text.trim().into();
    profile.accept_invalid_certificates = s.accept_invalid_certificates;
    profile.read_only = s.workspace.read_only;
    if profile.credential_mode == CredentialMode::SecretService && !s.workspace.remember_password {
        return Err(
            "Forget the saved password before changing to session-only credentials.".into(),
        );
    }
    profile.credential_mode = if s.workspace.remember_password {
        CredentialMode::SecretService
    } else {
        CredentialMode::SessionOnly
    };
    profile.auto_connect = s.workspace.auto_connect && s.workspace.remember_password;
    profile.ca_certificate = (!s.workspace.ca_certificate.text.trim().is_empty())
        .then(|| std::path::PathBuf::from(s.workspace.ca_certificate.text.trim()));
    profile.request_timeout_seconds =
        s.workspace.timeout.text.trim().parse().map_err(|_| {
            "Request timeout must be a whole number from 5 to 300 seconds".to_owned()
        })?;
    if !profile.accept_invalid_certificates {
        let mut options = ConnectionOptions::new(profile.endpoint.clone());
        if !s.fingerprint.text.trim().is_empty() {
            options.certificate_fingerprint = Some(
                s.fingerprint
                    .text
                    .parse()
                    .map_err(|error: xenadmin_core::Error| error.to_string())?,
            );
        }
        options
            .add_host_certificates(&s.host_certificates.text)
            .map_err(|error| error.to_string())?;
        profile.host_certificates = options.host_certificates();
    }
    trust::form(s, &mut profile)?;
    profile.options().map_err(|error| error.to_string())?;
    Ok(profile)
}

fn save(s: &mut State) -> Result<ConnectionProfile, String> {
    if !s.workspace.writable {
        return Err("This window does not own the application state directory. Close the other window before saving connections.".into());
    }
    if s.workspace
        .editing
        .is_some_and(|id| credential_pending(s, id))
    {
        return Err("Wait for this connection's password-store operation to finish before saving or connecting.".into());
    }
    let profile = form_profile(s)?;
    cancel_load(s, profile.id);
    s.workspace
        .store
        .as_mut()
        .map_err(|error| error.clone())?
        .save(profile.clone())
        .map_err(|error| error.to_string())?;
    s.workspace.editing = Some(profile.id);
    Ok(profile)
}

fn save_only(s: &mut State) {
    match save(s) {
        Ok(profile) => {
            if profile.credential_mode == CredentialMode::SecretService
                && !s.password.text.is_empty()
            {
                let task = CredentialJob::save(
                    CredentialKey::for_profile(&profile),
                    Credentials::new(profile.username.clone(), s.password.text.clone()),
                    s.waker.clone(),
                );
                s.workspace
                    .credential_tasks
                    .push((CredentialWork::Save(Box::new(profile.clone())), task));
            }
            s.password = secure_field();
            s.show_connect = false;
            trust::cancel(s);
            s.workspace.show_manager = true;
            s.error = None;
        }
        Err(error) => s.error = Some(error),
    }
}

pub(super) fn connect(s: &mut State) {
    let result = (|| {
        if s.username.text.trim().is_empty() {
            return Err("Enter the host username and password.".into());
        }
        s.backend.as_ref().map_err(Clone::clone)?;
        let profile = save(s)?;
        let credentials = if !s.password.text.is_empty() {
            if profile.credential_mode == CredentialMode::SecretService {
                s.workspace.pending_credentials.insert(
                    profile.id,
                    Credentials::new(profile.username.clone(), s.password.text.clone()),
                );
            }
            Credentials::new(profile.username.clone(), s.password.text.clone())
        } else if let Some((loaded_profile, credentials)) = s.workspace.cached_credentials.take() {
            if loaded_profile != profile || profile.credential_mode != CredentialMode::SecretService
            {
                return Err("Enter a password for this connection. Saved passwords are bound to their original connection settings.".into());
            }
            credentials
        } else {
            return Err(
                "Enter the host password, or load its saved password from the desktop keyring."
                    .into(),
            );
        };
        start_connection(s, &profile, credentials)?;
        s.password = secure_field();
        s.show_connect = false;
        s.workspace.show_manager = false;
        s.preferences.endpoint = profile.endpoint.to_string();
        s.preferences.username = profile.username.clone();
        s.preferences.fingerprint = s.fingerprint.text.trim().into();
        s.preferences.host_certificates = s.host_certificates.text.trim().into();
        s.preferences.accept_invalid_certificates = profile.accept_invalid_certificates;
        select(s, profile.id);
        Ok::<(), String>(())
    })();
    s.error = result.err();
}

fn start_connection(
    s: &mut State,
    profile: &ConnectionProfile,
    credentials: Credentials,
) -> Result<(), String> {
    let options = trust::options(s, profile)?;
    trust::remember_proxy(s, profile);
    s.workspace.session_accounts.remove(&profile.id);
    let connection = s
        .backend
        .as_ref()
        .map_err(Clone::clone)?
        .connect(options, credentials);
    if let Some(waker) = &s.waker {
        connection.notify_on_change(waker.clone());
    }
    s.workspace.active.insert(profile.id, connection);
    s.workspace.seen.remove(&profile.id);
    s.workspace.profile_status.remove(&profile.id);
    Ok(())
}

fn clear_view(s: &mut State) {
    s.metrics = None;
    s.diagnostics = None;
    s.diagnostic_results.clear();
    console_view::release(s);
    s.console = None;
    s.console_snapshot = None;
    s.console_ui = Default::default();
    s.snapshot_view = None;
    s.snapshot_dialog = None;
    s.snapshot_rows.clear();
    s.properties = None;
    s.placement = None;
    s.devices = None;
    s.force_action = None;
    s.unknown_review = None;
    s.operations.clear();
    s.snapshot = None;
    s.inventory = None;
    s.host_filter = None;
    s.show_details = false;
    s.search = TextEdit::new("");
    s.table = TableState::new().key("inventory-table").multi_select(true);
    s.bulk_power = None;
    s.row_menu = None;
    for (name, width) in &s.preferences.columns {
        s.table.set_column_width(name, *width);
    }
    s.page = Page::Guests;
    s.demo = false;
}

pub(super) fn select(s: &mut State, id: ProfileId) {
    let Some(connection) = s.workspace.active.get(&id).cloned() else {
        return;
    };
    clear_view(s);
    s.workspace.selected = Some(id);
    s.active_insecure = s
        .workspace
        .profile(id)
        .is_some_and(|profile| profile.accept_invalid_certificates);
    s.snapshot = Some(connection.latest());
    s.inventory = s
        .snapshot
        .as_ref()
        .and_then(|snapshot| snapshot.inventory().cloned());
    s.connection = Some(connection);
    s.preferences.last_profile = Some(id);
    s.workspace.show_manager = false;
    s.error = None;
    s.rebuild();
    s.save_preferences();
}

pub(super) fn disconnect_selected(s: &mut State) {
    if let Some(id) = s.workspace.selected {
        cancel_load(s, id);
    }
    console_view::release(s);
    s.console = None;
    s.console_snapshot = None;
    s.console_ui = Default::default();
    s.properties = None;
    s.placement = None;
    s.devices = None;
    s.force_action = None;
    s.snapshot_dialog = None;
    if let Some(connection) = &s.connection {
        connection.disconnect();
    }
}

fn remove(s: &mut State, id: ProfileId) {
    if !s.workspace.writable {
        return;
    }
    cancel_load(s, id);
    if s.workspace
        .profile(id)
        .is_some_and(|profile| profile.credential_mode == CredentialMode::SecretService)
    {
        if s.workspace.live(id) {
            s.error = Some("Disconnect this connection before removing its saved settings.".into());
            return;
        }
        if credential_pending(s, id) {
            return;
        }
        s.workspace.credential_tasks.push((
            CredentialWork::Remove(id),
            CredentialJob::remove(id, s.waker.clone()),
        ));
        s.workspace.credential_status = Some("Removing the saved password…".into());
        return;
    }
    remove_record(s, id);
}
fn remove_record(s: &mut State, id: ProfileId) {
    if !s.workspace.writable {
        s.error = Some("This window cannot change saved connections while another window owns the state directory.".into());
        return;
    }
    if s.workspace.live(id) {
        s.error = Some("Disconnect this connection before removing its saved settings.".into());
        return;
    }
    let result = s
        .workspace
        .store
        .as_mut()
        .map_err(|error| error.clone())
        .and_then(|store| store.remove(id).map_err(|error| error.to_string()));
    match result {
        Ok(_) => {
            s.workspace.proxy_sessions.remove(&id);
            s.workspace.active.remove(&id);
            s.workspace.seen.remove(&id);
            s.workspace.profile_status.remove(&id);
            s.workspace.loads.remove(&id);
            if s.workspace.selected == Some(id) {
                clear_view(s);
                s.connection = None;
                s.workspace.selected = None;
                s.preferences.last_profile = None;
                s.rebuild();
                s.save_preferences();
            }
            s.error = None;
        }
        Err(error) => s.error = Some(error),
    }
}

/// Poll every live pool, including those not currently selected in the UI.
pub(super) fn refresh(s: &mut State) -> bool {
    let mut changed = refresh_credentials(s);
    changed |= trust::poll(s);
    changed |= pool_join::poll(s);
    for (id, connection) in &s.workspace.active {
        let snapshot = connection.latest();
        if s.workspace
            .seen
            .get(id)
            .is_some_and(|old| Arc::ptr_eq(old, &snapshot))
        {
            continue;
        }
        changed = true;
        if snapshot.is_current() {
            if let Some(credentials) = s.workspace.pending_credentials.remove(id)
                && let Some(profile) = s.workspace.profile(*id)
            {
                let task = CredentialJob::save(
                    CredentialKey::for_profile(profile),
                    credentials,
                    s.waker.clone(),
                );
                s.workspace
                    .credential_tasks
                    .push((CredentialWork::Save(Box::new(profile.clone())), task));
            }
            if let Some((inventory, profile)) = snapshot.inventory().zip(s.workspace.profile(*id)) {
                let mut profile = profile.clone();
                let old_pool = profile.pool;
                let old_members = profile.verified_members.clone();
                let result = snapshot.verified_members().iter().try_for_each(|endpoint| {
                    profile.remember_member(inventory.pool_id, endpoint.clone())
                });
                if let Err(error) = result {
                    s.error = Some(error.to_string());
                } else if (profile.pool != old_pool || profile.verified_members != old_members)
                    && let Ok(store) = &mut s.workspace.store
                    && let Err(error) = store.save(profile)
                {
                    s.error = Some(error.to_string());
                }
            }
        }
        if matches!(
            snapshot.state(),
            ConnectionState::Failed | ConnectionState::Disconnected
        ) {
            s.workspace.pending_credentials.remove(id);
        }
        s.workspace.seen.insert(*id, snapshot);
    }
    changed
}

fn status(connection: Option<&Connection>) -> &'static str {
    match connection.map(|connection| connection.latest().state()) {
        None | Some(ConnectionState::Disconnected) => "Disconnected",
        Some(ConnectionState::Failed) => "Connection failed",
        Some(ConnectionState::Ready) => "Connected",
        Some(ConnectionState::Connecting) => "Connecting…",
        Some(ConnectionState::Synchronizing) => "Loading inventory…",
        Some(ConnectionState::Reconnecting) => "Reconnecting…",
    }
}

pub(super) fn sidebar(s: &State, k: &Kit) -> El {
    let mut panel = col().gap(4.0).child(
        Button::new("Connections")
            .variant(ButtonVariant::Outline)
            .on_press(|s: &mut State| {
                s.workspace.show_manager = true;
                s.error = None;
            })
            .build(k)
            .key("manage-connections"),
    );
    for (id, connection) in &s.workspace.active {
        let id = *id;
        let Some(profile) = s.workspace.profile(id) else {
            continue;
        };
        panel = panel.child(
            Button::new(format!(
                "{}{} · {}",
                profile.name,
                s.workspace
                    .session_accounts
                    .get(&id)
                    .map(|(username, _)| format!(" · as {username}"))
                    .unwrap_or_default(),
                status(Some(connection))
            ))
            .variant(if s.workspace.selected == Some(id) && !s.demo {
                ButtonVariant::Neutral
            } else {
                ButtonVariant::Ghost
            })
            .on_press(move |s: &mut State| select(s, id))
            .build(k)
            .key(format!("pool-{id}")),
        );
    }
    panel
}

pub(super) fn form_fields(s: &State, k: &Kit) -> El {
    let mut fields = col()
        .gap(8.0)
        .child(
            TextInput::new(&s.workspace.name)
                .label("Connection name")
                .placeholder("Connection name (optional)")
                .on_edit(|s: &mut State| &mut s.workspace.name)
                .build(k)
                .key("profile-name"),
        )
        .child(
            Button::new("Saved connection options")
                .variant(ButtonVariant::Ghost)
                .on_press(|s: &mut State| s.workspace.advanced = !s.workspace.advanced)
                .build(k)
                .key("profile-options"),
        );
    if s.workspace.advanced {
        fields = fields
            .child(trust::proxy_fields(s, k))
            .child(
                TextInput::new(&s.workspace.ca_certificate)
                    .label("CA certificate file")
                    .placeholder("CA certificate file (optional)")
                    .on_edit(|s: &mut State| &mut s.workspace.ca_certificate)
                    .build(k)
                    .key("profile-ca"),
            )
            .child(
                Button::new("Choose CA certificate…")
                    .variant(ButtonVariant::Outline)
                    .disabled(s.accept_invalid_certificates)
                    .on_press_tx(choose_ca)
                    .build(k)
                    .key("choose-ca-file"),
            )
            .child(
                row()
                    .gap(12.0)
                    .center_y()
                    .child(caption("Request timeout (seconds)", k))
                    .child(
                        TextInput::new(&s.workspace.timeout)
                            .label("Request timeout in seconds")
                            .on_edit(|s: &mut State| &mut s.workspace.timeout)
                            .build(k)
                            .w(100.0)
                            .key("profile-timeout"),
                    ),
            )
            .child(
                Checkbox::new(s.workspace.read_only)
                    .label("Read-only connection")
                    .on_toggle(|s: &mut State| s.workspace.read_only = !s.workspace.read_only)
                    .build(k)
                    .key("profile-read-only"),
            )
            .child(
                Button::new("Save without connecting")
                    .variant(ButtonVariant::Ghost)
                    .on_press(save_only)
                    .build(k)
                    .key("save-profile"),
            );
    }
    fields = fields.child(
        Checkbox::new(s.workspace.remember_password)
            .label("Remember password in desktop keyring")
            .on_toggle(|s: &mut State| {
                if s.workspace.remember_password
                    && s.workspace
                        .editing
                        .and_then(|id| s.workspace.profile(id))
                        .is_some_and(|profile| {
                            profile.credential_mode == CredentialMode::SecretService
                        })
                {
                    forget_password(s);
                } else {
                    s.workspace.remember_password = !s.workspace.remember_password;
                    if !s.workspace.remember_password {
                        s.workspace.auto_connect = false;
                        s.workspace.cached_credentials = None;
                    }
                }
            })
            .build(k)
            .key("remember-password"),
    );
    fields = fields.child(
        Checkbox::new(s.workspace.auto_connect)
            .label("Connect automatically when XenAdmin RS starts")
            .disabled(!s.workspace.remember_password)
            .on_toggle(|s: &mut State| {
                if s.workspace.remember_password { s.workspace.auto_connect = !s.workspace.auto_connect; }
            })
            .build(k)
            .key("profile-auto-connect"),
    ).child(caption("Automatic connection requires a password saved in the desktop keyring. A locked or unavailable keyring leaves the connection disconnected.", k).wrap());
    if let Some(status) = &s.workspace.credential_status {
        fields = fields.child(caption(status, k).wrap());
    }
    if s.workspace
        .editing
        .and_then(|id| s.workspace.profile(id))
        .is_some_and(|profile| profile.credential_mode == CredentialMode::SecretService)
    {
        fields = fields.child(
            row()
                .gap(8.0)
                .child(
                    Button::new("Load saved password")
                        .variant(ButtonVariant::Ghost)
                        .on_press(|s: &mut State| {
                            if let Some(profile) = s
                                .workspace
                                .editing
                                .and_then(|id| s.workspace.profile(id))
                                .cloned()
                            {
                                load_password(s, &profile);
                            }
                        })
                        .build(k)
                        .key("load-password"),
                )
                .child(
                    Button::new("Forget saved password")
                        .variant(ButtonVariant::Ghost)
                        .on_press(forget_password)
                        .build(k)
                        .key("forget-password"),
                ),
        );
    }
    fields
}

fn choose_ca(s: &mut State, tx: &mut Tx) {
    let epoch = s.workspace.editor_epoch;
    tx.open_file(
        rune::FileDialogOptions {
            title: "Choose CA certificate".into(),
            ..Default::default()
        }
        .filter("PEM certificates", &["pem", "crt", "cer"]),
        move |s: &mut State, result, _| {
            if !s.show_connect || s.workspace.editor_epoch != epoch {
                return;
            }
            match result {
                Ok(Some(path)) => match path.to_str() {
                    Some(path) => {
                        s.workspace.ca_certificate = TextEdit::new(path);
                        s.workspace.cached_credentials = None;
                        s.error = None;
                    }
                    None => {
                        s.error =
                            Some("Choose a certificate path with valid Unicode characters.".into())
                    }
                },
                Ok(None) => {}
                Err(error) => {
                    s.error = Some(format!(
                        "Cannot open the file chooser: {error}. Enter the CA file path instead."
                    ))
                }
            }
        },
    );
}

pub(super) fn dialog(s: &State, k: &Kit) -> El {
    if s.workspace.join.is_some() {
        return pool_join::dialog(s, k);
    }
    if s.workspace.reconnect.is_some() {
        return reconnect::dialog(s, k);
    }
    let mut list = col()
        .gap(12.0)
        .min_h(0.0)
        .grow(1.0)
        .scroll(rune::ScrollSpec {
            stick_bottom: false,
        });
    let profiles: Vec<_> = if !s.workspace.preview_profiles.is_empty() {
        s.workspace.preview_profiles.iter().collect()
    } else {
        match &s.workspace.store {
            Err(error) => {
                list = list.child(caption(error, k).wrap());
                Vec::new()
            }
            Ok(store) => store.profiles().collect(),
        }
    };
    if profiles.is_empty() && s.workspace.store.is_ok() {
        list = list.child(caption("Save a host address to reconnect to it later. Passwords are requested each session.", k).wrap());
    }
    let has_profiles = !profiles.is_empty();
    for profile in profiles {
        let id = profile.id;
        let active = s.workspace.live(id);
        let connection = s.workspace.active.get(&id);
        let mut entry = col()
            .gap(8.0)
            .padding(12.0)
            .border(1.0, k.c.border_weak)
            .radius(6.0)
            .child(heading(&profile.name, k))
            .child(
                caption(
                    format!(
                        "{} · {} · {}{}",
                        profile.endpoint,
                        profile.username,
                        status(connection),
                        if profile.read_only {
                            " · Read-only"
                        } else {
                            ""
                        }
                    ),
                    k,
                )
                .wrap(),
            );
        if profile.auto_connect {
            entry = entry.child(caption("Connects automatically at startup", k));
        }
        if let Some((username, read_only)) = s.workspace.session_accounts.get(&id) {
            entry = entry.child(
                caption(
                    format!(
                        "Current session account: {username}{} · saved account unchanged",
                        if *read_only { " · Read-only" } else { "" }
                    ),
                    k,
                )
                .wrap(),
            );
        }

        if let Some(message) = s.workspace.profile_status.get(&id) {
            entry = entry.child(
                caption(message, k)
                    .wrap()
                    .key(format!("profile-status-{id}")),
            );
        }
        if let Some(error) =
            connection.and_then(|connection| connection.latest().error().map(str::to_owned))
        {
            entry = entry.child(
                text(error)
                    .font(k.ty.text_small)
                    .color(k.c.danger.icon)
                    .wrap(),
            );
        }
        if let Some(route) =
            connection.and_then(|c| c.latest().connection_route().map(str::to_owned))
        {
            entry = entry.child(caption(format!("Last connection route: {route}"), k).wrap());
        }
        if let Some(failure) = connection.and_then(|c| c.latest().failure().cloned()) {
            entry = entry
                .child(caption(format!("Last failure stage: {}", failure.stage.label()), k).wrap());
            if let Some(endpoint) = failure.certificate_endpoint
                && !active
            {
                entry = entry.child(
                    Button::new("Review this endpoint's certificate…")
                        .variant(ButtonVariant::Outline)
                        .on_press(move |s: &mut State| {
                            trust::inspect_failed(s, id, endpoint.clone())
                        })
                        .build(k)
                        .key(format!("inspect-failed-{id}")),
                );
            }
        }
        let mut actions = row().gap(8.0);
        if active {
            actions = actions
                .child(
                    Button::new("View pool")
                        .on_press(move |s: &mut State| select(s, id))
                        .build(k)
                        .key(format!("view-profile-{id}")),
                )
                .child(
                    Button::new("Disconnect")
                        .variant(ButtonVariant::Ghost)
                        .on_press(move |s: &mut State| {
                            if s.workspace.selected == Some(id) {
                                disconnect_selected(s);
                            } else if let Some(connection) = s.workspace.active.get(&id) {
                                connection.disconnect();
                            }
                        })
                        .build(k)
                        .key(format!("disconnect-profile-{id}")),
                );
        } else if s.workspace.loads.contains_key(&id) {
            actions = actions.child(
                Button::new("Cancel connection attempt")
                    .variant(ButtonVariant::Ghost)
                    .on_press(move |s: &mut State| cancel_load(s, id))
                    .build(k)
                    .key(format!("cancel-profile-{id}")),
            );
        } else {
            actions = actions
                .child(
                    Button::new("Connect / edit")
                        .on_press(move |s: &mut State| edit(s, id))
                        .build(k)
                        .key(format!("edit-profile-{id}")),
                )
                .child(
                    Button::new("Remove")
                        .variant(ButtonVariant::Ghost)
                        .on_press(move |s: &mut State| remove(s, id))
                        .build(k)
                        .key(format!("remove-profile-{id}")),
                );
        }
        if active {
            entry = entry.child(
                Button::new("Reconnect as another account…")
                    .on_press(move |s: &mut State| reconnect::open(s, id))
                    .build(k)
                    .key(format!("reconnect-profile-{id}")),
            );
        }
        list = list.child(entry.child(actions));
    }
    let dialog = col().w((s.window_width - 32.0).clamp(320.0, 620.0)).h(560.0).padding(24.0).gap(16.0).radius(12.0)
        .fill(k.c.bg_base).border(1.0, k.c.border_weak)
        .child(heading("Connections", k))
        .child(caption("Keep several pools connected and switch between them. Same-pool entries share one session; disconnecting either disconnects that pool.", k).wrap())
        .child(row().gap(8.0)
            .child(Button::new("Connect all").disabled(!s.workspace.writable || !has_profiles).on_press(connect_all).build(k).key("connect-all"))
            .child(Button::new("Disconnect all").variant(ButtonVariant::Ghost).disabled(s.workspace.active.is_empty() && s.workspace.loads.is_empty()).on_press(disconnect_all).build(k).key("disconnect-all")))
        .child(list)
        .child(if let Some(status) = &s.workspace.credential_status { caption(status, k).wrap() } else {el()})
        .child(if let Some(error) = &s.error { text(error.clone()).font(k.ty.text_small).color(k.c.danger.icon).wrap().role("alert") } else { el() })
        .child(row().justify_end().gap(8.0)
            .child(Button::new("Close").variant(ButtonVariant::Ghost).on_press(|s: &mut State| s.workspace.show_manager = false).build(k).key("close-connections"))
            .child(Button::new("Add connection").on_press(begin_new).build(k).key("new-profile")));
    el().key("profiles-dialog")
        .role("dialog")
        .label("Connections")
        .w_full()
        .h_full()
        .absolute(0.0, 0.0)
        .fill(rune::Color::BLACK.alpha(0.5))
        .center()
        .modal(false, |s: &mut State| s.workspace.show_manager = false)
        .child(dialog)
}

fn credential_pending(s: &State, id: ProfileId) -> bool {
    s.workspace
        .credential_tasks
        .iter()
        .any(|(work, _)| match work {
            CredentialWork::Load(_) => false,
            CredentialWork::Save(profile) => profile.id == id,
            CredentialWork::Forget(profile) | CredentialWork::Remove(profile) => *profile == id,
        })
}

fn set_profile_status(s: &mut State, id: ProfileId, message: String) {
    if s.workspace.editing == Some(id) {
        s.workspace.credential_status = Some(message.clone());
    }
    s.workspace.profile_status.insert(id, message);
}

fn cancel_load(s: &mut State, id: ProfileId) {
    if s.workspace.loads.remove(&id).is_some() {
        set_profile_status(s, id, "Connection attempt cancelled.".into());
    }
    s.workspace.load_queue.retain(|queued| *queued != id);
}

pub(super) fn cancel_editor(s: &mut State) {
    trust::cancel(s);
    if let Some(id) = s.workspace.editing {
        cancel_load(s, id);
    }
    s.workspace.cached_credentials = None;
    s.workspace.credential_status = None;
    s.password = secure_field();
}
pub(super) fn certificate_fields(s: &State, k: &Kit) -> El {
    trust::certificate_fields(s, k)
}
pub(super) fn trust_scene(s: &mut State, review: bool) {
    trust::scene(s, review);
}
pub(super) fn certificate_review_controls(s: &State, k: &Kit) -> Option<El> {
    trust::review_controls(s, k)
}

fn queue_load(s: &mut State, profile: &ConnectionProfile, purpose: LoadPurpose) {
    if s.workspace.live(profile.id) || s.workspace.loads.contains_key(&profile.id) {
        return;
    }
    if profile.credential_mode != CredentialMode::SecretService {
        set_profile_status(
            s,
            profile.id,
            "Password required. Open Connect / edit to enter a password for this session.".into(),
        );
        return;
    }
    if credential_pending(s, profile.id) {
        set_profile_status(
            s,
            profile.id,
            "Wait for the current password-store operation, then retry.".into(),
        );
        return;
    }
    s.workspace.loads.insert(
        profile.id,
        CredentialLoad {
            id: uuid::Uuid::new_v4(),
            profile: profile.clone(),
            purpose,
        },
    );
    s.workspace.load_queue.push_back(profile.id);
    set_profile_status(s, profile.id, "Waiting to load saved password…".into());
}

fn pump_loads(s: &mut State) {
    let mut running = s
        .workspace
        .credential_tasks
        .iter()
        .filter(|(work, _)| matches!(work, CredentialWork::Load(_)))
        .count();
    while running < 4 {
        let Some(id) = s.workspace.load_queue.pop_front() else {
            break;
        };
        let Some(load) = s.workspace.loads.get(&id).cloned() else {
            continue;
        };
        if s.workspace.profile(id) != Some(&load.profile) {
            cancel_load(s, id);
            continue;
        }
        let task = CredentialJob::load(CredentialKey::for_profile(&load.profile), s.waker.clone());
        s.workspace
            .credential_tasks
            .push((CredentialWork::Load(Box::new(load)), task));
        set_profile_status(s, id, "Loading saved password…".into());
        running += 1;
    }
}

fn load_password(s: &mut State, profile: &ConnectionProfile) {
    queue_load(s, profile, LoadPurpose::Editor);
    pump_loads(s);
}

pub(super) fn restore_sessions(s: &mut State) {
    if s.workspace.restore_started {
        return;
    }
    s.workspace.restore_started = true;
    if !s.workspace.writable || s.demo || preview_arguments(std::env::args().skip(1)) {
        return;
    }
    let profiles = s
        .workspace
        .store
        .as_ref()
        .ok()
        .into_iter()
        .flat_map(|store| store.profiles())
        .filter(|profile| profile.auto_connect)
        .cloned()
        .collect::<Vec<_>>();
    if !profiles.is_empty() {
        s.workspace.show_manager = true;
    }
    for profile in profiles {
        queue_load(s, &profile, LoadPurpose::Connect);
    }
    pump_loads(s);
}

fn preview_arguments(arguments: impl Iterator<Item = String>) -> bool {
    arguments.into_iter().any(|argument| {
        ["--demo", "--scene", "--snap", "--tree"]
            .into_iter()
            .any(|flag| {
                argument == flag
                    || argument
                        .strip_prefix(flag)
                        .is_some_and(|tail| tail.starts_with('='))
            })
    })
}

fn connect_all(s: &mut State) {
    if !s.workspace.writable {
        return;
    }
    let profiles = s
        .workspace
        .store
        .as_ref()
        .ok()
        .into_iter()
        .flat_map(|store| store.profiles())
        .cloned()
        .collect::<Vec<_>>();
    for profile in profiles {
        queue_load(s, &profile, LoadPurpose::Connect);
    }
    pump_loads(s);
}

fn forget_password(s: &mut State) {
    let Some(id) = s.workspace.editing else {
        return;
    };
    if !s.workspace.writable {
        return;
    }
    cancel_load(s, id);
    if credential_pending(s, id) {
        return;
    }
    s.workspace.credential_tasks.push((
        CredentialWork::Forget(id),
        CredentialJob::remove(id, s.waker.clone()),
    ));
    s.workspace.credential_status = Some("Removing the saved password…".into());
}
fn refresh_credentials(s: &mut State) -> bool {
    let mut changed = false;
    let tasks = std::mem::take(&mut s.workspace.credential_tasks);
    for (work, task) in tasks {
        let Some(result) = task.take_result() else {
            s.workspace.credential_tasks.push((work, task));
            continue;
        };
        changed = true;
        apply_credential_result(s, work, result);
    }
    pump_loads(s);
    changed
}

fn apply_credential_result(
    s: &mut State,
    work: CredentialWork,
    result: Result<CredentialOutcome, xenadmin_core::CredentialStoreError>,
) {
    match (work, result) {
        (CredentialWork::Load(load), result) => {
            let id = load.profile.id;
            if !s
                .workspace
                .loads
                .get(&id)
                .is_some_and(|current| current.id == load.id)
            {
                return;
            }
            s.workspace.loads.remove(&id);
            if s.workspace.profile(id) != Some(&load.profile) {
                if s.workspace.profile(id).is_some() {
                    set_profile_status(
                        s,
                        id,
                        "Connection settings changed. Retry to load the matching password.".into(),
                    );
                }
                return;
            }
            if load.purpose == LoadPurpose::Editor
                && (!s.show_connect
                    || s.workspace.editing != Some(id)
                    || !form_profile(s).is_ok_and(|current| current == load.profile))
            {
                set_profile_status(
                    s,
                    id,
                    "Connection settings changed. Retry to load the matching password.".into(),
                );
                return;
            }
            match result {
                Ok(CredentialOutcome::Loaded(Some(credentials)))
                    if load.purpose == LoadPurpose::Editor =>
                {
                    s.workspace.cached_credentials = Some((load.profile, credentials));
                    set_profile_status(
                        s,
                        id,
                        "Saved password loaded. Leave the password field empty to use it.".into(),
                    );
                }
                Ok(CredentialOutcome::Loaded(Some(credentials))) => {
                    if s.workspace.live(id) {
                        return;
                    }
                    if let Err(error) = start_connection(s, &load.profile, credentials) {
                        set_profile_status(s, id, error);
                    } else if s.workspace.selected.is_none() || s.workspace.selected == Some(id) {
                        let show_manager = s.workspace.show_manager;
                        select(s, id);
                        s.workspace.show_manager = show_manager;
                    }
                }
                Ok(CredentialOutcome::Loaded(None)) => set_profile_status(
                    s,
                    id,
                    "No saved password was found. Open Connect / edit to enter one.".into(),
                ),
                Err(error) => set_profile_status(s, id, error.to_string()),
                _ => set_profile_status(
                    s,
                    id,
                    "The password store returned an unexpected response.".into(),
                ),
            }
        }
        (CredentialWork::Save(profile), Ok(CredentialOutcome::Saved)) => {
            if s.workspace.profile(profile.id).is_some_and(|current| {
                CredentialKey::for_profile(current) == CredentialKey::for_profile(&profile)
            }) {
                set_profile_status(
                    s,
                    profile.id,
                    "Password saved in the desktop keyring.".into(),
                );
            }
        }
        (CredentialWork::Forget(id), Ok(CredentialOutcome::Removed)) => {
            if let Some(mut profile) = s.workspace.profile(id).cloned() {
                profile.credential_mode = CredentialMode::SessionOnly;
                profile.auto_connect = false;
                if let Ok(store) = &mut s.workspace.store
                    && let Err(error) = store.save(profile)
                {
                    s.error = Some(error.to_string());
                }
            }
            if s.workspace.editing == Some(id) {
                s.workspace.remember_password = false;
                s.workspace.auto_connect = false;
                s.workspace.cached_credentials = None;
            }
            s.workspace.credential_status = Some(
                "Saved password removed. This connection now asks for a password each session."
                    .into(),
            );
        }
        (CredentialWork::Remove(id), Ok(CredentialOutcome::Removed)) => {
            remove_record(s, id);
            s.workspace.credential_status = Some("Connection and saved password removed.".into());
        }
        (work, Err(error)) => {
            let id = match work {
                CredentialWork::Save(profile) => profile.id,
                CredentialWork::Forget(id) | CredentialWork::Remove(id) => id,
                CredentialWork::Load(_) => unreachable!("load results handled above"),
            };
            set_profile_status(s, id, error.to_string());
            if s.workspace.editing == Some(id) {
                s.error = Some(error.to_string());
            }
        }
        _ => {
            s.error = Some("The password store returned an unexpected response.".into());
        }
    }
}

pub(super) fn disconnect_all(s: &mut State) {
    s.workspace.restore_started = true;
    let pending = s.workspace.loads.keys().copied().collect::<Vec<_>>();
    for id in pending {
        cancel_load(s, id);
    }
    disconnect_selected(s);
    for connection in s.workspace.active.values() {
        connection.disconnect();
    }
    s.workspace.pending_credentials.clear();
    s.workspace.cached_credentials = None;
}

pub(super) fn scene(s: &mut State, configure: bool) {
    // Scenes are synthetic and cannot save, connect, or touch the desktop password store.
    s.workspace = Workspace::load(None, false);
    s.backend = Err("Preview only".into());
    s.error = None;
    if configure {
        begin_new(s);
        s.endpoint = TextEdit::new("https://xcp-lab.example");
        s.workspace.name = TextEdit::new("Lab recovery");
        s.workspace.advanced = true;
        s.workspace.read_only = true;
        s.workspace.timeout = TextEdit::new("60");
    } else {
        s.workspace.show_manager = true;
        let first = ConnectionProfile::new(
            "Lab pool".into(),
            Endpoint::parse("https://xcp-lab.example").unwrap(),
            "root".into(),
        );
        let mut second = ConnectionProfile::new(
            "Recovery pool".into(),
            Endpoint::parse("https://recovery.example").unwrap(),
            "operator".into(),
        );
        second.read_only = true;
        s.workspace.preview_profiles = vec![first, second];
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn ca_picker_applies_to_its_editor_only_and_cancel_preserves_the_path() {
        let mut state = init();
        begin_new(&mut state);
        let mut runtime = rune::Runtime::new(
            state,
            |_: &State, cx: &mut rune::Cx<State>| {
                Button::new("Choose CA")
                    .on_press_tx(choose_ca)
                    .build(&Kit::neutral(cx.system_dark()))
                    .key("pick")
            },
            (480.0, 640.0),
            rune::Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        let pick = |runtime: &mut rune::Runtime<State>| {
            runtime.frame(0.0);
            let (x, y) = runtime.find("pick").unwrap();
            runtime.pointer_down(x, y);
            runtime.pointer_up(x, y);
            runtime.frame(0.0);
            assert_eq!(runtime.file_dialog().unwrap().0, rune::FileDialogKind::Open);
        };
        pick(&mut runtime);
        runtime
            .resolve_file_dialog(Some("/tmp/synthetic-ca.pem".into()))
            .unwrap();
        runtime.frame(0.0);
        assert_eq!(
            runtime.state.workspace.ca_certificate.text,
            "/tmp/synthetic-ca.pem"
        );
        pick(&mut runtime);
        runtime.resolve_file_dialog(None).unwrap();
        runtime.frame(0.0);
        assert_eq!(
            runtime.state.workspace.ca_certificate.text,
            "/tmp/synthetic-ca.pem"
        );
        pick(&mut runtime);
        begin_new(&mut runtime.state);
        runtime
            .resolve_file_dialog(Some("/tmp/old-editor.pem".into()))
            .unwrap();
        runtime.frame(0.0);
        assert!(runtime.state.workspace.ca_certificate.text.is_empty());
    }

    pub(super) struct Fixture {
        pub(super) state: State,
        pub(super) directory: std::path::PathBuf,
    }
    impl Fixture {
        pub(super) fn new() -> Self {
            let directory =
                std::env::temp_dir().join(format!("xenadmin-profiles-ui-{}", uuid::Uuid::new_v4()));
            let mut state = init();
            state.workspace = Workspace::load(None, false);
            state.workspace.writable = true;
            state.workspace.store = Ok(ProfileStore::open(directory.clone()).unwrap());
            begin_new(&mut state);
            Self { state, directory }
        }
    }
    impl Drop for Fixture {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.directory);
        }
    }

    #[test]
    fn saving_and_editing_a_profile_needs_no_connection_and_does_not_save_passwords() {
        let mut fixture = Fixture::new();
        let s = &mut fixture.state;
        s.endpoint.text = "host.example".into();
        s.username.text = "operator".into();
        s.password.text = "session-secret-that-must-not-be-saved".into();
        s.workspace.name.text = "Recovery pool".into();
        s.workspace.read_only = true;
        s.workspace.timeout.text = "60".into();
        save_only(s);
        assert!(s.error.is_none());
        assert!(s.connection.is_none());
        assert!(s.password.text.is_empty());
        let profile = s
            .workspace
            .store
            .as_ref()
            .unwrap()
            .profiles()
            .next()
            .unwrap()
            .clone();
        assert!(profile.read_only);
        assert_eq!(profile.request_timeout_seconds, 60);
        assert!(
            !std::fs::read_to_string(fixture.directory.join("profiles.json"))
                .unwrap()
                .contains("session-secret")
        );
        edit(s, profile.id);
        assert_eq!(s.endpoint.text, "https://host.example/");
        assert_eq!(s.username.text, "operator");
        assert_eq!(s.workspace.timeout.text, "60");
        s.workspace.name.text = "Recovery renamed".into();
        save_only(s);
        assert_eq!(s.workspace.store.as_ref().unwrap().profiles().count(), 1);
        assert_eq!(
            s.workspace.profile(profile.id).unwrap().name,
            "Recovery renamed"
        );
        remove(s, profile.id);
        assert_eq!(
            ProfileStore::open(fixture.directory.clone())
                .unwrap()
                .profiles()
                .count(),
            0
        );
    }

    #[test]
    fn invalid_profile_settings_do_not_replace_saved_settings() {
        let mut fixture = Fixture::new();
        let s = &mut fixture.state;
        s.endpoint.text = "host.example".into();
        save_only(s);
        let id = s.workspace.editing.unwrap();
        edit(s, id);
        s.workspace.timeout.text = "0".into();
        save_only(s);
        assert!(s.error.is_some());
        assert_eq!(s.workspace.profile(id).unwrap().request_timeout_seconds, 45);
        s.workspace.timeout.text = "45".into();
        s.endpoint.text = "https://user:secret@host.example/".into();
        save_only(s);
        assert!(s.error.is_some());
        assert_eq!(
            s.workspace.profile(id).unwrap().endpoint.as_str(),
            "https://host.example/"
        );
    }

    #[test]
    fn second_window_cannot_write_profiles_or_migrate_legacy_settings() {
        let mut fixture = Fixture::new();
        fixture.state.workspace.writable = false;
        fixture.state.endpoint.text = "host.example".into();
        save_only(&mut fixture.state);
        assert!(fixture.state.error.is_some());
        assert!(!fixture.directory.join("profiles.json").exists());
        let workspace = Workspace::load(None, false);
        assert!(workspace.store.is_err());
        assert!(!workspace.writable);
    }

    #[test]
    fn connections_dialog_exposes_saved_profiles_and_opens_an_editor() {
        use rune::{Env, Runtime};
        let mut fixture = Fixture::new();
        fixture.state.endpoint.text = "host.example".into();
        fixture.state.workspace.name.text = "Fixture pool".into();
        save_only(&mut fixture.state);
        let id = fixture.state.workspace.editing.unwrap();
        let mut state = init();
        std::mem::swap(&mut state, &mut fixture.state);
        let mut runtime = Runtime::new(
            state,
            view,
            (1440.0, 900.0),
            Env {
                system_dark: true,
                scale: 1.0,
            },
        );
        for _ in 0..5 {
            runtime.frame(1.0 / 60.0);
        }
        assert!(
            runtime
                .accessibility_json()
                .to_string()
                .contains("Fixture pool")
        );
        let (x, y) = runtime.find(&format!("edit-profile-{id}")).unwrap();
        runtime.pointer_down(x, y);
        runtime.pointer_up(x, y);
        runtime.frame(0.0);
        assert!(runtime.state.show_connect);
        assert!(!runtime.state.workspace.show_manager);
        assert_eq!(runtime.state.endpoint.text, "https://host.example/");
        assert!(runtime.state.password.text.is_empty());
    }
    #[test]
    fn saved_password_results_are_bound_to_the_open_profile_and_failures_allow_manual_input() {
        let mut fixture = Fixture::new();
        let s = &mut fixture.state;
        s.endpoint.text = "host.example".into();
        save_only(s);
        let id = s.workspace.editing.unwrap();
        let mut profile = s.workspace.profile(id).unwrap().clone();
        profile.credential_mode = CredentialMode::SecretService;
        s.workspace
            .store
            .as_mut()
            .unwrap()
            .save(profile.clone())
            .unwrap();
        populate(s, &profile);
        s.show_connect = true;
        queue_load(s, &profile, LoadPurpose::Editor);
        let load = s.workspace.loads[&id].clone();
        apply_credential_result(
            s,
            CredentialWork::Load(Box::new(load.clone())),
            Ok(CredentialOutcome::Loaded(Some(Credentials::new(
                "root".into(),
                "fixture-secret".into(),
            )))),
        );
        assert!(s.workspace.cached_credentials.is_some());
        assert!(s.password.text.is_empty());
        assert!(
            s.workspace
                .credential_status
                .as_deref()
                .unwrap()
                .contains("Saved password loaded")
        );
        begin_new(s);
        apply_credential_result(
            s,
            CredentialWork::Load(Box::new(load)),
            Ok(CredentialOutcome::Loaded(Some(Credentials::new(
                "root".into(),
                "late-response".into(),
            )))),
        );
        assert!(
            s.workspace.cached_credentials.is_none(),
            "a late response must not populate another connection"
        );
        populate(s, &profile);
        apply_credential_result(
            s,
            CredentialWork::Save(Box::new(profile.clone())),
            Err(xenadmin_core::CredentialStoreError::Locked),
        );
        assert!(s.error.as_deref().unwrap().contains("for this session"));
        s.password.text = "manual-fixture".into();
        assert_eq!(s.password.text, "manual-fixture");
        populate(s, &profile);
        apply_credential_result(
            s,
            CredentialWork::Forget(id),
            Ok(CredentialOutcome::Removed),
        );
        assert_eq!(
            s.workspace.profile(id).unwrap().credential_mode,
            CredentialMode::SessionOnly
        );
        assert!(!s.workspace.remember_password);
        let stored = std::fs::read_to_string(fixture.directory.join("profiles.json")).unwrap();
        assert!(!stored.contains("fixture-secret"));
        assert!(!stored.contains("manual-fixture"));
    }
}

#[cfg(test)]
#[path = "profile_restore_tests.rs"]
mod restore_tests;

pub(super) fn reconnect_pool(s: &mut State, pool: PoolId) {
    let id = s
        .workspace
        .selected
        .filter(|id| {
            s.workspace
                .active
                .get(id)
                .is_some_and(|c| c.latest().inventory().is_some_and(|i| i.pool_id == pool))
        })
        .or_else(|| {
            s.workspace
                .active
                .iter()
                .find(|(_, c)| c.latest().inventory().is_some_and(|i| i.pool_id == pool))
                .map(|(id, _)| *id)
        });
    if let Some(id) = id {
        reconnect::open(s, id);
    }
}

pub(super) fn join_pool(s: &mut State) {
    pool_join::open(s);
}
#[cfg(debug_assertions)]
pub(super) fn join_scene(s: &mut State) {
    pool_join::scene(s);
}
pub(super) fn resource_control_connection(
    s: &State,
    id: xenadmin_core::OperationId,
    control: xenadmin_core::OperationControl,
) -> Option<Connection> {
    let record = s.resource_rows.iter().find(|r| r.id == id)?;
    if matches!(control, xenadmin_core::OperationControl::Recheck)
        && matches!(record.action, xenadmin_core::ResourceAction::CrossPoolVm(_))
        && !cross_pool_vm::can_recheck(s, id)
    {
        return None;
    }
    let applies = |pool| {
        if matches!(control, xenadmin_core::OperationControl::Recheck) {
            pool == record.recovery_pool()
        } else {
            record.applies_to_pool(pool)
        }
    };
    s.workspace
        .active
        .values()
        .chain(s.connection.iter())
        .find(|c| {
            !c.is_read_only()
                && c.latest().is_current()
                && c.latest().inventory().is_some_and(|i| applies(i.pool_id))
        })
        .cloned()
}

pub(super) fn select_transfer_destination(
    s: &mut State,
    profile: ProfileId,
    pool: PoolId,
    vm: VmId,
) -> bool {
    let ready = s.workspace.active.get(&profile).is_some_and(|c| {
        let snapshot = c.latest();
        !c.is_read_only()
            && snapshot.inventory().is_some_and(|i| i.pool_id == pool)
            && snapshot.observe_vm(vm).is_ok()
    });
    if !ready {
        s.error = Some("The destination VM is no longer available through this connection".into());
        return false;
    }
    select(s, profile);
    s.page = Page::Guests;
    s.host_filter = None;
    s.search = TextEdit::new("");
    s.rebuild();
    s.table.select(vm.to_string());
    true
}
