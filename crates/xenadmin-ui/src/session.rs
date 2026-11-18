//! Sessions: the saved connection profiles and their connections.
//!
//! This is the Rune `profiles_view.rs` without its views, and
//! `profile_credentials.rs`. [`Workspace`] (`m.workspace`) owns the saved
//! profiles, a connection per profile that has been connected in this run
//! (several at once, all polled), the keyring load, save, forget and remove
//! queue, the start-up restore, and the draft of the connect form. The
//! functions keep their Rune names and take the model where the Rune ones
//! took `State`.
//!
//! # The scope session
//!
//! `workspace.selected` is the scope session: the saved connection whose
//! handle is `m.connection`, whose snapshot is `m.snapshot` and whose
//! inventory is `m.inventory`. It is the session that owns the tree
//! selection; [`Model::select`] calls [`adopt`] when the selection moves to
//! an object of another session, and it stays what it was while the
//! selection has no owner (nothing selected, a folder, a tag, a saved
//! search, objects of several pools). [`adopt`] runs [`clear_view`], which
//! drops what belonged to the previous pool and tells every area
//! (`on_session_change`).
//!
//! # What moved elsewhere
//!
//! - The child editors of the Rune workspace (`reconnect`, `join`,
//!   `root_password`) are area state: `connections` and `pool`. Their polls
//!   (`pool_join::poll`, `root_password::poll`) and `trust::poll` are those
//!   areas' `poll` hooks, no longer part of [`refresh`].
//! - [`Trust`] holds the data of the Rune `profile_trust::Panel`, and this
//!   module has the part the connection path needs (`populate`, `form`,
//!   `options`, `remember_proxy`, `cancel`). Inspecting and accepting a
//!   certificate is the `connections` area's.
//! - `show_manager` is gone with the Connections dialog: the connection
//!   list is the Home tab. Where Rune opened the manager, the model shows
//!   Home ([`Model::show_home`]); where Rune closed it, nothing is needed.
//! - A synthetic session (`workspace.synthetic`) is a saved-connection entry
//!   with an inventory and no connection: the demo pool, and the extra pools
//!   of scenes and tests. `m.demo` is true while the scope session is one.

use std::collections::{BTreeMap, VecDeque};
use std::hash::{Hash, Hasher};
use std::sync::Arc;

use xenadmin_core::{
    CertificateDetails, CertificateInspectionTask, CertificateObservation, Connection,
    ConnectionOptions, ConnectionProfile, ConnectionSnapshot, ConnectionState, CredentialKey,
    CredentialMode, CredentialOutcome, CredentialStoreError, Credentials, Endpoint, Inventory,
    OperationControl, OperationId, PoolId, ProfileId, ProfileStore, ProxyConfiguration,
    ProxyCredentials, ProxyEndpoint, VmId,
};

use crate::format::natural_cmp;
use crate::model::{Model, Waker};
use crate::object::{ObjectKind, ObjectRef, Target};
use crate::secret::SecretText;

// -- Keyring jobs (Rune `profile_credentials.rs`) ----------------------------

type Outcome = Result<CredentialOutcome, CredentialStoreError>;

/// One keyring job. In a normal build it is a `xenadmin_core::CredentialTask`.
/// In the tests of this crate no task is created, so no test touches the
/// desktop keyring: a test puts the answer into `result`.
pub struct CredentialJob {
    #[cfg(not(test))]
    task: xenadmin_core::CredentialTask,
    #[cfg(test)]
    pub result: std::cell::RefCell<Option<Outcome>>,
}
impl CredentialJob {
    pub fn load(key: CredentialKey, wake: Option<Waker>) -> Self {
        #[cfg(not(test))]
        {
            Self {
                task: xenadmin_core::CredentialTask::load(key, wake),
            }
        }
        #[cfg(test)]
        {
            let _ = (key, wake);
            Self {
                result: Default::default(),
            }
        }
    }
    pub fn save(key: CredentialKey, credentials: Credentials, wake: Option<Waker>) -> Self {
        #[cfg(not(test))]
        {
            Self {
                task: xenadmin_core::CredentialTask::save(key, credentials, wake),
            }
        }
        #[cfg(test)]
        {
            let _ = (key, credentials, wake);
            Self {
                result: Default::default(),
            }
        }
    }
    pub fn remove(id: ProfileId, wake: Option<Waker>) -> Self {
        #[cfg(not(test))]
        {
            Self {
                task: xenadmin_core::CredentialTask::remove(id, wake),
            }
        }
        #[cfg(test)]
        {
            let _ = (id, wake);
            Self {
                result: Default::default(),
            }
        }
    }
    pub fn take_result(&self) -> Option<Outcome> {
        #[cfg(not(test))]
        {
            self.task.take_result()
        }
        #[cfg(test)]
        {
            self.result.borrow_mut().take()
        }
    }
}

// -- The workspace ------------------------------------------------------------

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum LoadPurpose {
    Editor,
    Connect,
}
#[derive(Clone)]
pub struct CredentialLoad {
    pub id: uuid::Uuid,
    pub profile: ConnectionProfile,
    pub purpose: LoadPurpose,
}

pub enum CredentialWork {
    Load(Box<CredentialLoad>),
    Save(Box<ConnectionProfile>),
    Forget(ProfileId),
    Remove(ProfileId),
}

/// The proxy and certificate sub-editor of the connect form: the data of the
/// Rune `profile_trust::Panel`. The inspection workflow that fills `context`,
/// `job`, `observation` and `status` is the `connections` area's.
pub struct Trust {
    /// 0 Environment, 1 Direct, 2 HTTP proxy.
    pub mode: u8,
    pub proxy: String,
    pub username: String,
    /// The proxy password, for this application session only.
    pub password: SecretText,
    /// The exact endpoint to inspect.
    pub endpoint: String,
    /// The typed endpoint that confirms a pin.
    pub confirmation: String,
    pub verified: bool,
    pub details: BTreeMap<String, CertificateDetails>,
    pub context: Option<TrustContext>,
    pub job: Option<CertificateInspectionTask>,
    pub observation: Option<CertificateObservation>,
    pub status: Option<String>,
    /// The answer a test gives in place of a running inspection.
    #[cfg(test)]
    pub result: Option<Result<CertificateObservation, String>>,
}
impl Default for Trust {
    fn default() -> Self {
        Self {
            mode: 0,
            proxy: String::new(),
            username: String::new(),
            password: SecretText::new(),
            endpoint: String::new(),
            confirmation: String::new(),
            verified: false,
            details: BTreeMap::new(),
            context: None,
            job: None,
            observation: None,
            status: None,
            #[cfg(test)]
            result: None,
        }
    }
}
/// What a running or finished certificate inspection was started for.
pub struct TrustContext {
    pub profile: ConnectionProfile,
    pub epoch: uuid::Uuid,
    pub endpoint: Endpoint,
    pub secret_revision: u64,
}

pub fn inspection_connections(m: &Model) -> impl Iterator<Item = (ProfileId, Connection)> + '_ {
    m.workspace
        .active
        .iter()
        .map(|(id, connection)| (*id, connection.clone()))
}

pub struct Workspace {
    pub trust: Trust,
    pub proxy_sessions: BTreeMap<ProfileId, (ProxyConfiguration, ProxyCredentials)>,
    /// Username and read-only flag of a session opened by Reconnect As.
    pub session_accounts: BTreeMap<ProfileId, (String, bool)>,
    pub store: Result<ProfileStore, String>,
    /// This process owns the state directory.
    pub writable: bool,
    /// Synthetic profiles of scenes and tests; when not empty they replace
    /// the store in every list.
    pub preview_profiles: Vec<ConnectionProfile>,
    /// A connection per profile that has been connected in this run. An
    /// entry stays after a disconnect or a failure.
    pub active: BTreeMap<ProfileId, Connection>,
    /// The last snapshot `refresh` processed of each.
    pub seen: BTreeMap<ProfileId, Arc<ConnectionSnapshot>>,
    /// Sessions with an inventory and no connection: the demo pool, and the
    /// extra pools of scenes and tests. The inventory of the scope session
    /// is `m.inventory`; this map holds the others'.
    pub synthetic: BTreeMap<ProfileId, Arc<Inventory>>,
    /// The scope session.
    pub selected: Option<ProfileId>,
    /// The saved profile the connect form edits; `None` for a new one.
    pub editing: Option<ProfileId>,
    pub editor_epoch: uuid::Uuid,
    pub name: String,
    pub ca_certificate: String,
    pub timeout: String,
    pub read_only: bool,
    pub advanced: bool,
    pub remember_password: bool,
    pub auto_connect: bool,
    pub restore_started: bool,
    pub loads: BTreeMap<ProfileId, CredentialLoad>,
    pub load_queue: VecDeque<ProfileId>,
    pub profile_status: BTreeMap<ProfileId, String>,
    pub credential_tasks: Vec<(CredentialWork, CredentialJob)>,
    pub cached_credentials: Option<(ConnectionProfile, Credentials)>,
    pub pending_credentials: BTreeMap<ProfileId, Credentials>,
    pub credential_status: Option<String>,
    // The fields of the connect form that Rune kept directly in `State`.
    pub endpoint: String,
    pub username: String,
    /// The host password. Never persisted.
    pub password: SecretText,
    pub fingerprint: String,
    pub host_certificates: String,
    pub accept_invalid_certificates: bool,
    pub show_certificate_details: bool,
    /// The connect form is open.
    pub show_connect: bool,
}
impl Workspace {
    /// The Rune `Workspace::load`. With `writable` it opens the profile
    /// store of the state directory; without, nothing on disk is touched.
    pub fn load(selected: Option<ProfileId>, writable: bool) -> Self {
        let store = if writable {
            crate::prefs::directory()
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
            session_accounts: BTreeMap::new(),
            store,
            writable,
            preview_profiles: Vec::new(),
            active: BTreeMap::new(),
            seen: BTreeMap::new(),
            synthetic: BTreeMap::new(),
            selected,
            editing: None,
            editor_epoch: uuid::Uuid::new_v4(),
            name: String::new(),
            ca_certificate: String::new(),
            timeout: "45".into(),
            read_only: false,
            advanced: false,
            remember_password: false,
            auto_connect: false,
            restore_started: false,
            loads: BTreeMap::new(),
            load_queue: VecDeque::new(),
            profile_status: BTreeMap::new(),
            credential_tasks: Vec::new(),
            cached_credentials: None,
            pending_credentials: BTreeMap::new(),
            credential_status: None,
            endpoint: String::new(),
            username: "root".into(),
            password: SecretText::new(),
            fingerprint: String::new(),
            host_certificates: String::new(),
            accept_invalid_certificates: false,
            show_certificate_details: false,
            show_connect: false,
        }
    }
    pub fn profile(&self, id: ProfileId) -> Option<&ConnectionProfile> {
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
    /// The saved connections as every list shows them: the synthetic ones
    /// when there are any, otherwise the store's.
    pub fn profiles(&self) -> Vec<&ConnectionProfile> {
        if !self.preview_profiles.is_empty() {
            self.preview_profiles.iter().collect()
        } else {
            self.store
                .as_ref()
                .ok()
                .into_iter()
                .flat_map(|store| store.profiles())
                .collect()
        }
    }
    /// Connected, connecting or reconnecting: in `active` and neither
    /// `Disconnected` nor `Failed`.
    pub fn live(&self, id: ProfileId) -> bool {
        self.active.get(&id).is_some_and(|connection| {
            !matches!(
                connection.latest().state(),
                ConnectionState::Disconnected | ConnectionState::Failed
            )
        })
    }
    /// The latest snapshot of a session: what `refresh` last processed, or
    /// the handle's own when it has not run yet.
    pub fn snapshot(&self, id: ProfileId) -> Option<Arc<ConnectionSnapshot>> {
        self.seen
            .get(&id)
            .cloned()
            .or_else(|| self.active.get(&id).map(Connection::latest))
    }
}

/// The profile id a tree node of kind `Connection` carries as its id.
pub fn profile_id(text: &str) -> Option<ProfileId> {
    serde_json::from_value(serde_json::Value::String(text.to_owned())).ok()
}

pub fn apply_initial(m: &mut Model) {
    if let Some(profile) = m
        .workspace
        .selected
        .and_then(|id| m.workspace.profile(id))
        .cloned()
    {
        populate(m, &profile);
    } else {
        m.workspace.selected = None;
    }
}

pub fn populate(m: &mut Model, profile: &ConnectionProfile) {
    trust_populate(m, profile);
    m.workspace.editor_epoch = uuid::Uuid::new_v4();
    m.workspace.remember_password = profile.credential_mode == CredentialMode::SecretService;
    m.workspace.auto_connect = profile.auto_connect;
    m.workspace.cached_credentials = None;
    m.workspace.credential_status = None;
    m.workspace.editing = Some(profile.id);
    m.workspace.name = profile.name.clone();
    m.workspace.ca_certificate = profile
        .ca_certificate
        .as_ref()
        .map(|path| path.to_string_lossy().into_owned())
        .unwrap_or_default();
    m.workspace.timeout = profile.request_timeout_seconds.to_string();
    m.workspace.read_only = profile.read_only;
    m.workspace.endpoint = profile.endpoint.as_str().to_owned();
    m.workspace.username = profile.username.clone();
    m.workspace.password.clear();
    m.workspace.fingerprint = profile
        .host_certificates
        .get(profile.endpoint.as_str())
        .map(ToString::to_string)
        .unwrap_or_default();
    m.workspace.host_certificates = profile
        .host_certificates
        .iter()
        .filter(|(endpoint, _)| endpoint.as_str() != profile.endpoint.as_str())
        .map(|(endpoint, pin)| format!("{endpoint}={pin}"))
        .collect::<Vec<_>>()
        .join("\n");
    m.workspace.accept_invalid_certificates = profile.accept_invalid_certificates;
    m.workspace.show_certificate_details = false;
}

pub fn begin_new(m: &mut Model) {
    cancel_editor(m);
    m.workspace.trust = Default::default();
    m.workspace.editor_epoch = uuid::Uuid::new_v4();
    m.workspace.remember_password = false;
    m.workspace.auto_connect = false;
    m.workspace.cached_credentials = None;
    m.workspace.credential_status = None;
    m.workspace.editing = None;
    m.workspace.name = String::new();
    m.workspace.ca_certificate = String::new();
    m.workspace.timeout = "45".into();
    m.workspace.read_only = false;
    m.workspace.endpoint = String::new();
    m.workspace.username = "root".into();
    m.workspace.password.clear();
    m.workspace.fingerprint = String::new();
    m.workspace.host_certificates = String::new();
    m.workspace.accept_invalid_certificates = false;
    m.workspace.show_certificate_details = false;
    m.workspace.show_connect = true;
    m.error = None;
}

pub fn edit(m: &mut Model, id: ProfileId) {
    if m.workspace.live(id) {
        m.error = Some("Disconnect this connection before editing its settings.".into());
        return;
    }
    if let Some(profile) = m.workspace.profile(id).cloned() {
        cancel_load(m, id);
        cancel_editor(m);
        populate(m, &profile);
        m.workspace.show_connect = true;
        if profile.credential_mode == CredentialMode::SecretService {
            load_password(m, &profile);
        }
        m.workspace.show_connect = true;
        m.error = None;
    }
}

pub fn form_profile(m: &Model) -> Result<ConnectionProfile, String> {
    let endpoint = Endpoint::parse(&m.workspace.endpoint).map_err(|error| error.to_string())?;
    let mut profile = m
        .workspace
        .editing
        .and_then(|id| m.workspace.profile(id))
        .cloned()
        .unwrap_or_else(|| ConnectionProfile::new(String::new(), endpoint.clone(), String::new()));
    if m.workspace.live(profile.id) {
        return Err("Disconnect this connection before editing its settings.".into());
    }
    let previous_pin = if profile.endpoint != endpoint {
        profile
            .host_certificates
            .get(profile.endpoint.as_str())
            .cloned()
    } else {
        None
    };
    profile.set_endpoint(endpoint);
    profile.name = if m.workspace.name.trim().is_empty() {
        profile.endpoint.to_string()
    } else {
        m.workspace.name.trim().into()
    };
    profile.username = m.workspace.username.trim().into();
    profile.accept_invalid_certificates = m.workspace.accept_invalid_certificates;
    profile.read_only = m.workspace.read_only;
    if profile.credential_mode == CredentialMode::SecretService && !m.workspace.remember_password {
        return Err(
            "Forget the saved password before changing to session-only credentials.".into(),
        );
    }
    profile.credential_mode = if m.workspace.remember_password {
        CredentialMode::SecretService
    } else {
        CredentialMode::SessionOnly
    };
    profile.auto_connect = m.workspace.auto_connect && m.workspace.remember_password;
    profile.ca_certificate = (!m.workspace.ca_certificate.trim().is_empty())
        .then(|| std::path::PathBuf::from(m.workspace.ca_certificate.trim()));
    profile.request_timeout_seconds =
        m.workspace.timeout.trim().parse().map_err(|_| {
            "Request timeout must be a whole number from 5 to 300 seconds".to_owned()
        })?;
    if !profile.accept_invalid_certificates {
        let mut options = ConnectionOptions::new(profile.endpoint.clone());
        if !m.workspace.fingerprint.trim().is_empty() {
            let fingerprint = m
                .workspace
                .fingerprint
                .parse()
                .map_err(|error: xenadmin_core::Error| error.to_string())?;
            if previous_pin.as_ref() != Some(&fingerprint) {
                options.certificate_fingerprint = Some(fingerprint);
            }
        }
        options
            .add_host_certificates(&m.workspace.host_certificates)
            .map_err(|error| error.to_string())?;
        profile.host_certificates = options.host_certificates();
    }
    trust_form(m, &mut profile)?;
    profile.options().map_err(|error| error.to_string())?;
    Ok(profile)
}

pub fn save(m: &mut Model) -> Result<ConnectionProfile, String> {
    if !m.workspace.writable {
        return Err("This window does not own the application state directory. Close the other window before saving connections.".into());
    }
    if m.workspace
        .editing
        .is_some_and(|id| credential_pending(m, id))
    {
        return Err("Wait for this connection's password-store operation to finish before saving or connecting.".into());
    }
    let profile = form_profile(m)?;
    cancel_load(m, profile.id);
    m.workspace
        .store
        .as_mut()
        .map_err(|error| error.clone())?
        .save(profile.clone())
        .map_err(|error| error.to_string())?;
    m.workspace.editing = Some(profile.id);
    Ok(profile)
}

/// "Save without connecting".
pub fn save_only(m: &mut Model) {
    match save(m) {
        Ok(profile) => {
            if profile.credential_mode == CredentialMode::SecretService
                && !m.workspace.password.is_empty()
            {
                let task = CredentialJob::save(
                    CredentialKey::for_profile(&profile),
                    Credentials::new(
                        profile.username.clone(),
                        m.workspace.password.expose().to_owned(),
                    ),
                    m.waker.clone(),
                );
                m.workspace
                    .credential_tasks
                    .push((CredentialWork::Save(Box::new(profile.clone())), task));
            }
            m.workspace.password.clear();
            m.workspace.show_connect = false;
            trust_cancel(m);
            // Rune reopened the Connections dialog here; it is the Home tab now.
            m.show_home();
            m.error = None;
        }
        Err(error) => m.error = Some(error),
    }
}

pub fn connect(m: &mut Model) {
    let result = (|| {
        if m.workspace.username.trim().is_empty() {
            return Err("Enter the host username and password.".into());
        }
        m.backend.as_ref().map_err(Clone::clone)?;
        let profile = save(m)?;
        let credentials = if !m.workspace.password.is_empty() {
            if profile.credential_mode == CredentialMode::SecretService {
                m.workspace.pending_credentials.insert(
                    profile.id,
                    Credentials::new(
                        profile.username.clone(),
                        m.workspace.password.expose().to_owned(),
                    ),
                );
            }
            Credentials::new(
                profile.username.clone(),
                m.workspace.password.expose().to_owned(),
            )
        } else if let Some((loaded_profile, credentials)) = m.workspace.cached_credentials.take() {
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
        start_connection(m, &profile, credentials)?;
        m.workspace.password.clear();
        m.workspace.show_connect = false;
        m.preferences.endpoint = profile.endpoint.to_string();
        m.preferences.username = profile.username.clone();
        m.preferences.fingerprint = m.workspace.fingerprint.trim().into();
        m.preferences.host_certificates = m.workspace.host_certificates.trim().into();
        m.preferences.accept_invalid_certificates = profile.accept_invalid_certificates;
        select(m, profile.id);
        Ok::<(), String>(())
    })();
    m.error = result.err();
}

pub fn start_connection(
    m: &mut Model,
    profile: &ConnectionProfile,
    credentials: Credentials,
) -> Result<(), String> {
    let options = trust_options(m, profile)?;
    trust_remember_proxy(m, profile);
    m.workspace.session_accounts.remove(&profile.id);
    let connection = m
        .backend
        .as_ref()
        .map_err(Clone::clone)?
        .connect(options, credentials);
    if let Some(waker) = &m.waker {
        connection.notify_on_change(waker.clone());
    }
    m.workspace.active.insert(profile.id, connection);
    m.workspace.seen.remove(&profile.id);
    m.workspace.profile_status.remove(&profile.id);
    Ok(())
}

/// Drops everything that belongs to the view of the previous pool: the Rune
/// `clear_view`. Every area drops its part first, in its
/// `on_session_change` hook, while the model still describes the previous
/// session; then the kernel drops its own. It does not clear
/// `operation_rows`, `resource_rows` or `resource_operations`.
pub fn clear_view(m: &mut Model) {
    for area in crate::areas::specs() {
        (area.on_session_change)(m);
    }
    // A synthetic session keeps its inventory while another session is the
    // scope session.
    if m.demo
        && let (Some(id), Some(inventory)) = (m.workspace.selected, m.inventory.clone())
        && m.workspace.synthetic.contains_key(&id)
    {
        m.workspace.synthetic.insert(id, inventory);
    }
    m.diagnostics = None;
    m.diagnostic_results.clear();
    m.operations.clear();
    m.snapshot = None;
    m.inventory = None;
    m.selection.grid = Target::none();
    m.selection.menu = None;
    m.demo = false;
}

/// Makes a session the scope session without moving the tree selection:
/// [`clear_view`], then `m.connection`, `m.snapshot`, `m.inventory` and
/// `m.demo` are that session's. A saved connection that has not been
/// connected leaves all three empty.
pub fn adopt(m: &mut Model, id: ProfileId) {
    clear_view(m);
    m.workspace.selected = Some(id);
    if let Some(inventory) = m.workspace.synthetic.get(&id).cloned() {
        m.connection = None;
        m.snapshot = None;
        m.inventory = Some(inventory);
        m.demo = true;
    } else {
        let connection = m.workspace.active.get(&id).cloned();
        m.snapshot = connection.as_ref().map(Connection::latest);
        m.inventory = m
            .snapshot
            .as_ref()
            .and_then(|snapshot| snapshot.inventory().cloned());
        m.connection = connection;
    }
}

/// The Rune `select`: makes the session the scope session and shows it. The
/// tree selection moves to the session's node (its pool once it has an
/// inventory).
pub fn select(m: &mut Model, id: ProfileId) {
    if !m.workspace.active.contains_key(&id) && !m.workspace.synthetic.contains_key(&id) {
        return;
    }
    adopt(m, id);
    m.preferences.last_profile = Some(id);
    m.error = None;
    let node = session_object(m, id);
    m.select(Target::one(node));
    m.save_preferences();
}

/// Whether the scope session's saved connection has certificate validation
/// disabled (the Rune `active_insecure`).
pub fn active_insecure(m: &Model) -> bool {
    m.workspace
        .selected
        .and_then(|id| m.workspace.profile(id))
        .is_some_and(|profile| profile.accept_invalid_certificates)
}

pub fn disconnect_selected(m: &mut Model) {
    if let Some(id) = m.workspace.selected {
        cancel_load(m, id);
    }
    // Rune dropped the console, the properties editor, placement, the device
    // view, the power confirmation and the snapshot dialog here. Every area
    // drops what belongs to the session.
    for area in crate::areas::specs() {
        (area.on_session_change)(m);
    }
    if let Some(connection) = &m.connection {
        connection.disconnect();
    }
}

/// Disconnects one session: the selected one through
/// [`disconnect_selected`], another directly.
pub fn disconnect(m: &mut Model, id: ProfileId) {
    if m.workspace.selected == Some(id) {
        disconnect_selected(m);
    } else if let Some(connection) = m.workspace.active.get(&id) {
        connection.disconnect();
    }
}

pub fn remove(m: &mut Model, id: ProfileId) {
    if !m.workspace.writable {
        return;
    }
    cancel_load(m, id);
    if m.workspace
        .profile(id)
        .is_some_and(|profile| profile.credential_mode == CredentialMode::SecretService)
    {
        if m.workspace.live(id) {
            m.error = Some("Disconnect this connection before removing its saved settings.".into());
            return;
        }
        if credential_pending(m, id) {
            return;
        }
        m.workspace.credential_tasks.push((
            CredentialWork::Remove(id),
            CredentialJob::remove(id, m.waker.clone()),
        ));
        m.workspace.credential_status = Some("Removing the saved password…".into());
        return;
    }
    remove_record(m, id);
}
pub fn remove_record(m: &mut Model, id: ProfileId) {
    if !m.workspace.writable {
        m.error = Some("This window cannot change saved connections while another window owns the state directory.".into());
        return;
    }
    if m.workspace.live(id) {
        m.error = Some("Disconnect this connection before removing its saved settings.".into());
        return;
    }
    let result = m
        .workspace
        .store
        .as_mut()
        .map_err(|error| error.clone())
        .and_then(|store| store.remove(id).map_err(|error| error.to_string()));
    match result {
        Ok(_) => {
            m.workspace.proxy_sessions.remove(&id);
            m.workspace.active.remove(&id);
            m.workspace.seen.remove(&id);
            m.workspace.profile_status.remove(&id);
            m.workspace.loads.remove(&id);
            if m.workspace.selected == Some(id) {
                clear_view(m);
                m.connection = None;
                m.workspace.selected = None;
                m.preferences.last_profile = None;
                m.reconcile_selection();
                m.save_preferences();
            }
            m.error = None;
        }
        Err(error) => m.error = Some(error),
    }
}

/// Poll every live pool, including those not currently selected in the UI.
pub fn refresh(m: &mut Model) -> bool {
    let mut changed = refresh_credentials(m);
    let ids: Vec<ProfileId> = m.workspace.active.keys().copied().collect();
    for id in ids {
        let Some(snapshot) = m.workspace.active.get(&id).map(Connection::latest) else {
            continue;
        };
        if m.workspace
            .seen
            .get(&id)
            .is_some_and(|old| Arc::ptr_eq(old, &snapshot))
        {
            continue;
        }
        changed = true;
        if snapshot.is_current() {
            if let Some(credentials) = m.workspace.pending_credentials.remove(&id)
                && let Some(profile) = m.workspace.profile(id).cloned()
            {
                let task = CredentialJob::save(
                    CredentialKey::for_profile(&profile),
                    credentials,
                    m.waker.clone(),
                );
                m.workspace
                    .credential_tasks
                    .push((CredentialWork::Save(Box::new(profile)), task));
            }
            if let Some((inventory, profile)) = snapshot.inventory().zip(m.workspace.profile(id)) {
                let mut profile = profile.clone();
                let old_pool = profile.pool;
                let old_members = profile.verified_members.clone();
                let result = snapshot.verified_members().iter().try_for_each(|endpoint| {
                    profile.remember_member(inventory.pool_id, endpoint.clone())
                });
                if let Err(error) = result {
                    m.error = Some(error.to_string());
                } else if (profile.pool != old_pool || profile.verified_members != old_members)
                    && let Ok(store) = &mut m.workspace.store
                    && let Err(error) = store.save(profile)
                {
                    m.error = Some(error.to_string());
                }
            }
        }
        if matches!(
            snapshot.state(),
            ConnectionState::Failed | ConnectionState::Disconnected
        ) {
            m.workspace.pending_credentials.remove(&id);
        }
        m.workspace.seen.insert(id, snapshot);
    }
    changed
}

/// The status word of a connection, as the connection list and the tree
/// show it.
pub fn status(connection: Option<&Connection>) -> &'static str {
    status_word(connection.map(|connection| connection.latest().state()))
}

pub fn status_word(state: Option<ConnectionState>) -> &'static str {
    match state {
        None | Some(ConnectionState::Disconnected) => "Disconnected",
        Some(ConnectionState::Failed) => "Connection failed",
        Some(ConnectionState::Ready) => "Connected",
        Some(ConnectionState::Connecting) => "Connecting…",
        Some(ConnectionState::Synchronizing) => "Loading inventory…",
        Some(ConnectionState::Reconnecting) => "Reconnecting…",
    }
}

pub fn credential_pending(m: &Model, id: ProfileId) -> bool {
    m.workspace
        .credential_tasks
        .iter()
        .any(|(work, _)| match work {
            CredentialWork::Load(_) => false,
            CredentialWork::Save(profile) => profile.id == id,
            CredentialWork::Forget(profile) | CredentialWork::Remove(profile) => *profile == id,
        })
}

pub fn set_profile_status(m: &mut Model, id: ProfileId, message: String) {
    if m.workspace.editing == Some(id) {
        m.workspace.credential_status = Some(message.clone());
    }
    m.workspace.profile_status.insert(id, message);
}

/// "Cancel connection attempt".
pub fn cancel_load(m: &mut Model, id: ProfileId) {
    if m.workspace.loads.remove(&id).is_some() {
        set_profile_status(m, id, "Connection attempt cancelled.".into());
    }
    m.workspace.load_queue.retain(|queued| *queued != id);
}

pub fn cancel_editor(m: &mut Model) {
    trust_cancel(m);
    if let Some(id) = m.workspace.editing {
        cancel_load(m, id);
    }
    m.workspace.cached_credentials = None;
    m.workspace.credential_status = None;
    m.workspace.password.clear();
}

pub fn queue_load(m: &mut Model, profile: &ConnectionProfile, purpose: LoadPurpose) {
    if m.workspace.live(profile.id) || m.workspace.loads.contains_key(&profile.id) {
        return;
    }
    if profile.credential_mode != CredentialMode::SecretService {
        set_profile_status(
            m,
            profile.id,
            "Password required. Open Connect / edit to enter a password for this session.".into(),
        );
        return;
    }
    if credential_pending(m, profile.id) {
        set_profile_status(
            m,
            profile.id,
            "Wait for the current password-store operation, then retry.".into(),
        );
        return;
    }
    m.workspace.loads.insert(
        profile.id,
        CredentialLoad {
            id: uuid::Uuid::new_v4(),
            profile: profile.clone(),
            purpose,
        },
    );
    m.workspace.load_queue.push_back(profile.id);
    set_profile_status(m, profile.id, "Waiting to load saved password…".into());
}

pub fn pump_loads(m: &mut Model) {
    let mut running = m
        .workspace
        .credential_tasks
        .iter()
        .filter(|(work, _)| matches!(work, CredentialWork::Load(_)))
        .count();
    while running < 4 {
        let Some(id) = m.workspace.load_queue.pop_front() else {
            break;
        };
        let Some(load) = m.workspace.loads.get(&id).cloned() else {
            continue;
        };
        if m.workspace.profile(id) != Some(&load.profile) {
            cancel_load(m, id);
            continue;
        }
        let task = CredentialJob::load(CredentialKey::for_profile(&load.profile), m.waker.clone());
        m.workspace
            .credential_tasks
            .push((CredentialWork::Load(Box::new(load)), task));
        set_profile_status(m, id, "Loading saved password…".into());
        running += 1;
    }
}

/// "Load saved password".
pub fn load_password(m: &mut Model, profile: &ConnectionProfile) {
    queue_load(m, profile, LoadPurpose::Editor);
    pump_loads(m);
}

pub fn restore_sessions(m: &mut Model) {
    if m.workspace.restore_started {
        return;
    }
    m.workspace.restore_started = true;
    if !m.workspace.writable || m.demo || crate::launch::preview_arguments(std::env::args().skip(1))
    {
        return;
    }
    let profiles = m
        .workspace
        .store
        .as_ref()
        .ok()
        .into_iter()
        .flat_map(|store| store.profiles())
        .filter(|profile| profile.auto_connect)
        .cloned()
        .collect::<Vec<_>>();
    for profile in profiles {
        queue_load(m, &profile, LoadPurpose::Connect);
    }
    pump_loads(m);
}

/// "Connect all".
pub fn connect_all(m: &mut Model) {
    if !m.workspace.writable {
        return;
    }
    let profiles = m
        .workspace
        .store
        .as_ref()
        .ok()
        .into_iter()
        .flat_map(|store| store.profiles())
        .cloned()
        .collect::<Vec<_>>();
    for profile in profiles {
        queue_load(m, &profile, LoadPurpose::Connect);
    }
    pump_loads(m);
}

/// "Forget saved password" of the connect form: the password of the saved
/// connection being edited.
pub fn forget_password(m: &mut Model) {
    let Some(id) = m.workspace.editing else {
        return;
    };
    forget_password_of(m, id);
}

/// Forgets the saved password of one saved connection, which need not be
/// open in the connect form (Server > Password > Forget Password). The
/// keyring entry goes first; the profile becomes session-only when the
/// keyring answers (`apply_credential_result`).
pub fn forget_password_of(m: &mut Model, id: ProfileId) {
    if !m.workspace.writable {
        return;
    }
    cancel_load(m, id);
    if credential_pending(m, id) {
        return;
    }
    m.workspace.credential_tasks.push((
        CredentialWork::Forget(id),
        CredentialJob::remove(id, m.waker.clone()),
    ));
    m.workspace.credential_status = Some("Removing the saved password…".into());
}
pub fn refresh_credentials(m: &mut Model) -> bool {
    let mut changed = false;
    let tasks = std::mem::take(&mut m.workspace.credential_tasks);
    for (work, task) in tasks {
        let Some(result) = task.take_result() else {
            m.workspace.credential_tasks.push((work, task));
            continue;
        };
        changed = true;
        apply_credential_result(m, work, result);
    }
    pump_loads(m);
    changed
}

pub fn apply_credential_result(
    m: &mut Model,
    work: CredentialWork,
    result: Result<CredentialOutcome, CredentialStoreError>,
) {
    match (work, result) {
        (CredentialWork::Load(load), result) => {
            let id = load.profile.id;
            if !m
                .workspace
                .loads
                .get(&id)
                .is_some_and(|current| current.id == load.id)
            {
                return;
            }
            m.workspace.loads.remove(&id);
            if m.workspace.profile(id) != Some(&load.profile) {
                if m.workspace.profile(id).is_some() {
                    set_profile_status(
                        m,
                        id,
                        "Connection settings changed. Retry to load the matching password.".into(),
                    );
                }
                return;
            }
            if load.purpose == LoadPurpose::Editor
                && (!m.workspace.show_connect
                    || m.workspace.editing != Some(id)
                    || !form_profile(m).is_ok_and(|current| current == load.profile))
            {
                set_profile_status(
                    m,
                    id,
                    "Connection settings changed. Retry to load the matching password.".into(),
                );
                return;
            }
            match result {
                Ok(CredentialOutcome::Loaded(Some(credentials)))
                    if load.purpose == LoadPurpose::Editor =>
                {
                    m.workspace.cached_credentials = Some((load.profile, credentials));
                    set_profile_status(
                        m,
                        id,
                        "Saved password loaded. Leave the password field empty to use it.".into(),
                    );
                }
                Ok(CredentialOutcome::Loaded(Some(credentials))) => {
                    if m.workspace.live(id) {
                        return;
                    }
                    if let Err(error) = start_connection(m, &load.profile, credentials) {
                        set_profile_status(m, id, error);
                    } else if m.workspace.selected.is_none() || m.workspace.selected == Some(id) {
                        // Rune selected the profile and left the Connections
                        // dialog as it was: the session becomes the scope
                        // session and the tree selection stays.
                        adopt(m, id);
                        m.preferences.last_profile = Some(id);
                        m.error = None;
                        m.save_preferences();
                    }
                }
                Ok(CredentialOutcome::Loaded(None)) => set_profile_status(
                    m,
                    id,
                    "No saved password was found. Open Connect / edit to enter one.".into(),
                ),
                Err(error) => set_profile_status(m, id, error.to_string()),
                _ => set_profile_status(
                    m,
                    id,
                    "The password store returned an unexpected response.".into(),
                ),
            }
        }
        (CredentialWork::Save(profile), Ok(CredentialOutcome::Saved)) => {
            if m.workspace.profile(profile.id).is_some_and(|current| {
                CredentialKey::for_profile(current) == CredentialKey::for_profile(&profile)
            }) {
                set_profile_status(
                    m,
                    profile.id,
                    "Password saved in the desktop keyring.".into(),
                );
            }
        }
        (CredentialWork::Forget(id), Ok(CredentialOutcome::Removed)) => {
            if let Some(mut profile) = m.workspace.profile(id).cloned() {
                profile.credential_mode = CredentialMode::SessionOnly;
                profile.auto_connect = false;
                if let Ok(store) = &mut m.workspace.store
                    && let Err(error) = store.save(profile)
                {
                    m.error = Some(error.to_string());
                }
            }
            if m.workspace.editing == Some(id) {
                m.workspace.remember_password = false;
                m.workspace.auto_connect = false;
                m.workspace.cached_credentials = None;
            }
            m.workspace.credential_status = Some(
                "Saved password removed. This connection now asks for a password each session."
                    .into(),
            );
        }
        (CredentialWork::Remove(id), Ok(CredentialOutcome::Removed)) => {
            remove_record(m, id);
            m.workspace.credential_status = Some("Connection and saved password removed.".into());
        }
        (work, Err(error)) => {
            let id = match work {
                CredentialWork::Save(profile) => profile.id,
                CredentialWork::Forget(id) | CredentialWork::Remove(id) => id,
                CredentialWork::Load(_) => unreachable!("load results handled above"),
            };
            set_profile_status(m, id, error.to_string());
            if m.workspace.editing == Some(id) {
                m.error = Some(error.to_string());
            }
        }
        _ => {
            m.error = Some("The password store returned an unexpected response.".into());
        }
    }
}

/// "Disconnect all", and what the close of the main window runs.
pub fn disconnect_all(m: &mut Model) {
    m.workspace.restore_started = true;
    let pending = m.workspace.loads.keys().copied().collect::<Vec<_>>();
    for id in pending {
        cancel_load(m, id);
    }
    disconnect_selected(m);
    for connection in m.workspace.active.values() {
        connection.disconnect();
    }
    m.workspace.pending_credentials.clear();
    m.workspace.cached_credentials = None;
}

/// The saved connection through which Reconnect As reaches a pool: the
/// scope session when its inventory is that pool, else the first active one
/// that is (the lookup of the Rune `reconnect_pool`).
pub fn profile_for_pool(m: &Model, pool: PoolId) -> Option<ProfileId> {
    let on_pool = |connection: &Connection| {
        connection
            .latest()
            .inventory()
            .is_some_and(|i| i.pool_id == pool)
    };
    m.workspace
        .selected
        .filter(|id| m.workspace.active.get(id).is_some_and(on_pool))
        .or_else(|| {
            m.workspace
                .active
                .iter()
                .find(|(_, c)| on_pool(c))
                .map(|(id, _)| *id)
        })
}

/// Whether a transfer between two pools can be rechecked now: both of its
/// pools connected for writing. The `cross_pool` area gives the rule; until
/// it lands this is the Rune `cross_pool_vm::can_recheck`.
fn cross_pool_can_recheck(m: &Model, id: OperationId) -> bool {
    let writable = |pool: PoolId| {
        m.workspace.active.values().any(|c| {
            !c.is_read_only()
                && c.latest().is_current()
                && c.latest().inventory().is_some_and(|i| i.pool_id == pool)
        })
    };
    m.resource_rows
        .iter()
        .find(|r| r.id == id)
        .is_some_and(|r| {
            r.action.spans_pools()
                && r.batch_parent.is_none()
                && writable(r.pool)
                && r.peer_pool().is_some_and(writable)
        })
}

/// The connection that can recheck, acknowledge or cancel a resource
/// operation, which may belong to another pool than the scope session's.
pub fn resource_control_connection(
    m: &Model,
    id: OperationId,
    control: OperationControl,
) -> Option<Connection> {
    let record = m.resource_rows.iter().find(|r| r.id == id)?;
    if record.batch_parent.is_some() {
        return None;
    }
    if matches!(control, OperationControl::Recheck)
        && record.action.spans_pools()
        && !cross_pool_can_recheck(m, id)
    {
        return None;
    }
    let applies = |pool| {
        if matches!(control, OperationControl::Recheck) {
            pool == record.recovery_pool()
        } else {
            record.applies_to_pool(pool)
        }
    };
    m.workspace
        .active
        .values()
        .chain(m.connection.iter())
        .find(|c| {
            (!c.is_read_only() || record.action.read_only_export())
                && c.latest().is_current()
                && c.latest().inventory().is_some_and(|i| applies(i.pool_id))
        })
        .cloned()
}

/// Shows the VM a cross-pool transfer created, in its destination pool.
pub fn select_transfer_destination(
    m: &mut Model,
    profile: ProfileId,
    pool: PoolId,
    vm: VmId,
) -> bool {
    let ready = m.workspace.active.get(&profile).is_some_and(|c| {
        let snapshot = c.latest();
        !c.is_read_only()
            && snapshot.inventory().is_some_and(|i| i.pool_id == pool)
            && snapshot.observe_vm(vm).is_ok()
    });
    if !ready {
        m.error = Some("The destination VM is no longer available through this connection".into());
        return false;
    }
    select(m, profile);
    m.nav.filter.clear();
    m.select(Target::one(ObjectRef::vm(pool, vm)));
    true
}

// -- Sessions as the tree, the Home tab and the status bar see them ----------

/// One saved connection with what is known of its session.
#[derive(Clone, Debug)]
pub struct SessionView {
    pub profile: ProfileId,
    /// The connection's name.
    pub name: String,
    /// `None`: not connected in this run.
    pub state: Option<ConnectionState>,
    /// The status word: "Connected", "Connecting…", "Connection failed",
    /// "Disconnected", ..., or "Demo" for a synthetic session.
    pub status: &'static str,
    /// The connection generation of the latest snapshot: it changes on
    /// every reconnect, and with it every captured observation goes stale.
    pub generation: Option<u64>,
    /// The latest snapshot is current (`Ready`). Never for a synthetic
    /// session.
    pub current: bool,
    /// The inventory, when the session has one (also a stale one).
    pub inventory: Option<Arc<Inventory>>,
    pub synthetic: bool,
    /// The scope session.
    pub selected: bool,
    /// A keyring load or a connection attempt is wanted.
    pub loading: bool,
}

/// The inventory of one session. For the scope session it is `m.inventory`.
pub fn inventory_of(m: &Model, id: ProfileId) -> Option<Arc<Inventory>> {
    if m.workspace.selected == Some(id) {
        return m.inventory.clone();
    }
    if let Some(inventory) = m.workspace.synthetic.get(&id) {
        return Some(inventory.clone());
    }
    m.workspace
        .snapshot(id)
        .and_then(|snapshot| snapshot.inventory().cloned())
}

/// Every saved connection, by name. A session without a saved profile (one
/// removed from the store while connected cannot exist; a test's handle
/// may) is left out.
pub fn sessions(m: &Model) -> Vec<SessionView> {
    let mut sessions: Vec<SessionView> = m
        .workspace
        .profiles()
        .into_iter()
        .map(|profile| {
            let id = profile.id;
            let synthetic = m.workspace.synthetic.contains_key(&id);
            let snapshot = m.workspace.snapshot(id);
            let state = snapshot.as_ref().map(|snapshot| snapshot.state());
            SessionView {
                profile: id,
                name: profile.name.clone(),
                state,
                status: if synthetic {
                    "Demo"
                } else {
                    status_word(state)
                },
                generation: snapshot.as_ref().map(|snapshot| snapshot.generation()),
                current: snapshot
                    .as_ref()
                    .is_some_and(|snapshot| snapshot.is_current()),
                inventory: inventory_of(m, id),
                synthetic,
                selected: m.workspace.selected == Some(id),
                loading: m.workspace.loads.contains_key(&id),
            }
        })
        .collect();
    sessions.sort_by(|a, b| natural_cmp(&a.name, &b.name).then_with(|| a.profile.cmp(&b.profile)));
    sessions
}

/// The session through which a pool is shown and acted on. Two saved
/// connections may reach one pool: a current one beats a stale one, then the
/// scope session wins, then the first by id (the rule of the Rune
/// `search_view::add_source`; inventory revisions are not compared).
pub fn session_for_pool(m: &Model, pool: PoolId) -> Option<ProfileId> {
    let mut best: Option<((bool, bool), ProfileId)> = None;
    for profile in m.workspace.profiles() {
        let id = profile.id;
        if !inventory_of(m, id).is_some_and(|inventory| inventory.pool_id == pool) {
            continue;
        }
        let selected = m.workspace.selected == Some(id);
        let current = if selected {
            m.snapshot.as_ref().is_some_and(|s| s.is_current())
        } else {
            m.workspace.snapshot(id).is_some_and(|s| s.is_current())
        };
        let rank = (current, selected);
        if best.as_ref().is_none_or(|(old, _)| rank > *old) {
            best = Some((rank, id));
        }
    }
    best.map(|(_, id)| id)
}

/// The tree node of a session: its pool when it is the session through
/// which that pool is shown, otherwise the saved connection itself.
pub fn session_object(m: &Model, id: ProfileId) -> ObjectRef {
    match inventory_of(m, id) {
        Some(inventory) if session_for_pool(m, inventory.pool_id) == Some(id) => {
            ObjectRef::pool(inventory.pool_id)
        }
        _ => ObjectRef::new(None, ObjectKind::Connection, id),
    }
}

/// The session that owns what is selected: the saved connection of a
/// `Connection` node, or the session of the pool every object shares.
/// `None` when the selection has no single owner.
pub fn owner(m: &Model, scope: &Target) -> Option<ProfileId> {
    if let Some(node) = scope.single_of(ObjectKind::Connection) {
        return profile_id(&node.id).filter(|id| m.workspace.profile(*id).is_some());
    }
    session_for_pool(m, scope.pool()?)
}

// -- Trust: the part of the Rune `profile_trust.rs` the connection path needs --

pub fn trust_secret_revision(m: &Model) -> u64 {
    let mut hash = std::collections::hash_map::DefaultHasher::new();
    m.workspace.trust.password.expose().hash(&mut hash);
    hash.finish()
}
pub fn trust_populate(m: &mut Model, profile: &ConnectionProfile) {
    let mut p = Trust {
        details: profile.certificate_details.clone(),
        endpoint: profile.endpoint.as_str().to_owned(),
        ..Default::default()
    };
    match &profile.proxy {
        ProxyConfiguration::Environment => {}
        ProxyConfiguration::Direct => p.mode = 1,
        ProxyConfiguration::Http { endpoint, username } => {
            p.mode = 2;
            p.proxy = endpoint.as_str().to_owned();
            p.username = username.as_deref().unwrap_or("").to_owned();
        }
    }
    m.workspace.trust = p;
}
pub fn trust_form(m: &Model, profile: &mut ConnectionProfile) -> Result<(), String> {
    let p = &m.workspace.trust;
    profile.proxy = match p.mode {
        0 => ProxyConfiguration::Environment,
        1 => ProxyConfiguration::Direct,
        _ => ProxyConfiguration::Http {
            endpoint: ProxyEndpoint::parse(&p.proxy).map_err(|e| e.to_string())?,
            username: (!p.username.trim().is_empty()).then(|| p.username.trim().into()),
        },
    };
    profile.certificate_details = p
        .details
        .iter()
        .filter(|(endpoint, d)| profile.host_certificates.get(*endpoint) == Some(&d.fingerprint))
        .map(|(e, d)| (e.clone(), d.clone()))
        .collect();
    Ok(())
}
pub fn trust_options(m: &Model, profile: &ConnectionProfile) -> Result<ConnectionOptions, String> {
    let mut options = profile.options().map_err(|e| e.to_string())?;
    if let ProxyConfiguration::Http {
        username: Some(username),
        ..
    } = &profile.proxy
    {
        let credentials =
            if trust_draft_proxy_matches(m, profile) && !m.workspace.trust.password.is_empty() {
                ProxyCredentials::new(
                    username.clone(),
                    m.workspace.trust.password.expose().to_owned(),
                )
                .map_err(|e| e.to_string())?
            } else {
                m.workspace
                    .proxy_sessions
                    .get(&profile.id)
                    .filter(|(p, _)| p == &profile.proxy)
                    .map(|(_, c)| c.clone())
                    .ok_or_else(|| xenadmin_core::Error::ProxyAuthenticationRequired.to_string())?
            };
        options
            .set_proxy_credentials(credentials)
            .map_err(|e| e.to_string())?;
    }
    options
        .connection_route(&profile.endpoint)
        .map_err(|e| e.to_string())?;
    Ok(options)
}
pub fn trust_remember_proxy(m: &mut Model, profile: &ConnectionProfile) {
    if let ProxyConfiguration::Http {
        username: Some(username),
        ..
    } = &profile.proxy
        && trust_draft_proxy_matches(m, profile)
        && !m.workspace.trust.password.is_empty()
        && let Ok(credentials) = ProxyCredentials::new(
            username.clone(),
            m.workspace.trust.password.expose().to_owned(),
        )
    {
        m.workspace
            .proxy_sessions
            .insert(profile.id, (profile.proxy.clone(), credentials));
    }
    m.workspace.trust.password.clear();
}
pub fn trust_cancel(m: &mut Model) {
    m.workspace.trust.job = None;
    m.workspace.trust.context = None;
    m.workspace.trust.observation = None;
    m.workspace.trust.password.clear();
}
pub fn trust_draft_proxy_matches(m: &Model, profile: &ConnectionProfile) -> bool {
    m.workspace.show_connect
        && m.workspace.editing.is_none_or(|id| id == profile.id)
        && m.workspace.trust.mode == 2
        && matches!(&profile.proxy, ProxyConfiguration::Http { endpoint, username }
            if ProxyEndpoint::parse(&m.workspace.trust.proxy).ok().as_ref() == Some(endpoint)
            && username.as_deref() == Some(m.workspace.trust.username.trim()))
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// The model with a private, empty state directory; shared with every
    /// area through `crate::testing`.
    pub(crate) use crate::testing::Fixture;

    pub(crate) fn save_profile(
        m: &mut Model,
        name: &str,
        saved: bool,
        restore: bool,
    ) -> ConnectionProfile {
        let mut profile = ConnectionProfile::new(
            name.into(),
            Endpoint::parse("https://fixture.example").unwrap(),
            "operator".into(),
        );
        profile.credential_mode = if saved {
            CredentialMode::SecretService
        } else {
            CredentialMode::SessionOnly
        };
        profile.auto_connect = restore;
        m.workspace
            .store
            .as_mut()
            .unwrap()
            .save(profile.clone())
            .unwrap();
        profile
    }

    /// Gives the running keyring load of a profile its answer.
    pub(crate) fn finish(m: &mut Model, id: ProfileId, result: Outcome) {
        let (_, job) = m
            .workspace
            .credential_tasks
            .iter()
            .find(|(work, _)| matches!(work, CredentialWork::Load(load) if load.profile.id == id))
            .unwrap();
        *job.result.borrow_mut() = Some(result);
        refresh_credentials(m);
    }

    pub(crate) fn loaded() -> Outcome {
        Ok(CredentialOutcome::Loaded(Some(Credentials::new(
            "operator".into(),
            "synthetic-secret".into(),
        ))))
    }

    // Rune: profiles_view.rs, the same name.
    #[test]
    fn saving_and_editing_a_profile_needs_no_connection_and_does_not_save_passwords() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        m.workspace.endpoint = "host.example".into();
        m.workspace.username = "operator".into();
        m.workspace
            .password
            .set("session-secret-that-must-not-be-saved");
        m.workspace.name = "Recovery pool".into();
        m.workspace.read_only = true;
        m.workspace.timeout = "60".into();
        save_only(m);
        assert!(m.error.is_none());
        assert!(m.connection.is_none());
        assert!(m.workspace.password.is_empty());
        let profile = m
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
        edit(m, profile.id);
        assert_eq!(m.workspace.endpoint, "https://host.example/");
        assert_eq!(m.workspace.username, "operator");
        assert_eq!(m.workspace.timeout, "60");
        m.workspace.name = "Recovery renamed".into();
        save_only(m);
        assert_eq!(m.workspace.store.as_ref().unwrap().profiles().count(), 1);
        assert_eq!(
            m.workspace.profile(profile.id).unwrap().name,
            "Recovery renamed"
        );
        remove(m, profile.id);
        assert_eq!(
            ProfileStore::open(fixture.directory.clone())
                .unwrap()
                .profiles()
                .count(),
            0
        );
    }

    // Rune: profiles_view.rs, the same name.
    #[test]
    fn invalid_profile_settings_do_not_replace_saved_settings() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        m.workspace.endpoint = "host.example".into();
        save_only(m);
        let id = m.workspace.editing.unwrap();
        edit(m, id);
        m.workspace.timeout = "0".into();
        save_only(m);
        assert!(m.error.is_some());
        assert_eq!(m.workspace.profile(id).unwrap().request_timeout_seconds, 45);
        m.workspace.timeout = "45".into();
        m.workspace.endpoint = "https://user:secret@host.example/".into();
        save_only(m);
        assert!(m.error.is_some());
        assert_eq!(
            m.workspace.profile(id).unwrap().endpoint.as_str(),
            "https://host.example/"
        );
    }

    // Rune: profiles_view.rs, the same name.
    #[test]
    fn second_window_cannot_write_profiles_or_migrate_legacy_settings() {
        let mut fixture = Fixture::new();
        fixture.model.workspace.writable = false;
        fixture.model.workspace.endpoint = "host.example".into();
        save_only(&mut fixture.model);
        assert!(fixture.model.error.is_some());
        assert!(!fixture.directory.join("profiles.json").exists());
        let workspace = Workspace::load(None, false);
        assert!(workspace.store.is_err());
        assert!(!workspace.writable);
    }

    // Rune: profiles_view.rs, the same name.
    #[test]
    fn saved_password_results_are_bound_to_the_open_profile_and_failures_allow_manual_input() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        m.workspace.endpoint = "host.example".into();
        save_only(m);
        let id = m.workspace.editing.unwrap();
        let mut profile = m.workspace.profile(id).unwrap().clone();
        profile.credential_mode = CredentialMode::SecretService;
        m.workspace
            .store
            .as_mut()
            .unwrap()
            .save(profile.clone())
            .unwrap();
        populate(m, &profile);
        m.workspace.show_connect = true;
        queue_load(m, &profile, LoadPurpose::Editor);
        let load = m.workspace.loads[&id].clone();
        apply_credential_result(
            m,
            CredentialWork::Load(Box::new(load.clone())),
            Ok(CredentialOutcome::Loaded(Some(Credentials::new(
                "root".into(),
                "fixture-secret".into(),
            )))),
        );
        assert!(m.workspace.cached_credentials.is_some());
        assert!(m.workspace.password.is_empty());
        assert!(
            m.workspace
                .credential_status
                .as_deref()
                .unwrap()
                .contains("Saved password loaded")
        );
        begin_new(m);
        apply_credential_result(
            m,
            CredentialWork::Load(Box::new(load)),
            Ok(CredentialOutcome::Loaded(Some(Credentials::new(
                "root".into(),
                "late-response".into(),
            )))),
        );
        assert!(
            m.workspace.cached_credentials.is_none(),
            "a late response must not populate another connection"
        );
        populate(m, &profile);
        apply_credential_result(
            m,
            CredentialWork::Save(Box::new(profile.clone())),
            Err(CredentialStoreError::Locked),
        );
        assert!(m.error.as_deref().unwrap().contains("for this session"));
        m.workspace.password.set("manual-fixture");
        assert_eq!(m.workspace.password.expose(), "manual-fixture");
        populate(m, &profile);
        apply_credential_result(
            m,
            CredentialWork::Forget(id),
            Ok(CredentialOutcome::Removed),
        );
        assert_eq!(
            m.workspace.profile(id).unwrap().credential_mode,
            CredentialMode::SessionOnly
        );
        assert!(!m.workspace.remember_password);
        let stored = std::fs::read_to_string(fixture.directory.join("profiles.json")).unwrap();
        assert!(!stored.contains("fixture-secret"));
        assert!(!stored.contains("manual-fixture"));
    }

    // Rune: profile_restore_tests.rs
    // `startup_attempts_only_opted_in_profiles_once_and_explains_locked_keyring`.
    #[test]
    fn startup_attempts_only_opted_in_profiles_once_and_explains_locked_keyring() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        let opted = save_profile(m, "Restore", true, true);
        let manual = save_profile(m, "Manual", true, false);
        let session = save_profile(m, "Session only", false, false);
        restore_sessions(m);
        assert!(m.workspace.loads.contains_key(&opted.id));
        assert!(!m.workspace.loads.contains_key(&manual.id));
        assert!(!m.workspace.loads.contains_key(&session.id));
        finish(m, opted.id, Err(CredentialStoreError::Locked));
        assert!(m.workspace.profile_status[&opted.id].contains("locked"));
        assert!(m.workspace.active.is_empty());
        restore_sessions(m);
        assert!(m.workspace.loads.is_empty());
        assert!(m.workspace.credential_tasks.is_empty());
        assert!(
            !std::fs::read_to_string(fixture.directory.join("profiles.json"))
                .unwrap()
                .contains("synthetic-secret")
        );
    }

    // Rune: profile_restore_tests.rs, the same name.
    #[test]
    fn bulk_loading_is_bounded_and_disconnect_all_invalidates_pending_credentials() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        let mut profiles = Vec::new();
        for n in 0..6 {
            profiles.push(save_profile(m, &format!("Saved {n}"), true, false));
        }
        let session = save_profile(m, "Session only", false, false);
        connect_all(m);
        assert_eq!(m.workspace.loads.len(), 6);
        assert_eq!(m.workspace.credential_tasks.len(), 4);
        assert_eq!(m.workspace.load_queue.len(), 2);
        assert!(m.workspace.profile_status[&session.id].contains("Password required"));
        connect_all(m);
        assert_eq!(m.workspace.credential_tasks.len(), 4);
        let running = match &m.workspace.credential_tasks[0].0 {
            CredentialWork::Load(load) => load.profile.id,
            _ => unreachable!(),
        };
        finish(m, running, Ok(CredentialOutcome::Loaded(None)));
        assert_eq!(m.workspace.credential_tasks.len(), 4);
        assert_eq!(m.workspace.loads.len(), 5);
        assert!(m.workspace.profile_status[&running].contains("No saved password"));
        disconnect_all(m);
        assert!(m.workspace.loads.is_empty());
        assert!(m.workspace.load_queue.is_empty());
        for (_, job) in &m.workspace.credential_tasks {
            *job.result.borrow_mut() = Some(loaded());
        }
        refresh_credentials(m);
        assert!(m.workspace.active.is_empty());
        assert!(m.workspace.cached_credentials.is_none());
    }

    // Rune: profile_restore_tests.rs, the same name.
    #[test]
    fn edited_removed_and_cancelled_profiles_discard_late_results() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        for change in [
            "endpoint",
            "username",
            "trust",
            "read_only",
            "remove",
            "cancel",
        ] {
            let profile = save_profile(m, change, true, false);
            queue_load(m, &profile, LoadPurpose::Connect);
            pump_loads(m);
            let mut modified = profile.clone();
            match change {
                "endpoint" => modified.set_endpoint(Endpoint::parse("different.example").unwrap()),
                "username" => modified.username = "another-operator".into(),
                "trust" => modified.accept_invalid_certificates = true,
                "read_only" => modified.read_only = true,
                "remove" => {
                    m.workspace
                        .store
                        .as_mut()
                        .unwrap()
                        .remove(profile.id)
                        .unwrap();
                }
                "cancel" => cancel_load(m, profile.id),
                _ => unreachable!(),
            }
            if !matches!(change, "remove" | "cancel") {
                m.workspace.store.as_mut().unwrap().save(modified).unwrap();
            }
            finish(m, profile.id, loaded());
            assert!(
                !m.workspace.active.contains_key(&profile.id),
                "late result used after {change}"
            );
            assert!(m.workspace.cached_credentials.is_none());
        }
    }

    // Rune: profile_restore_tests.rs, the same name.
    #[test]
    fn retry_does_not_accept_an_old_attempt_and_editor_trust_edits_discard_loaded_passwords() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        let profile = save_profile(m, "Retry", true, false);
        queue_load(m, &profile, LoadPurpose::Connect);
        let first = m.workspace.loads[&profile.id].clone();
        cancel_load(m, profile.id);
        queue_load(m, &profile, LoadPurpose::Connect);
        let second = m.workspace.loads[&profile.id].id;
        apply_credential_result(m, CredentialWork::Load(Box::new(first)), loaded());
        assert_eq!(m.workspace.loads[&profile.id].id, second);
        assert!(m.workspace.active.is_empty());
        cancel_load(m, profile.id);
        populate(m, &profile);
        m.workspace.show_connect = true;
        load_password(m, &profile);
        m.workspace.accept_invalid_certificates = true;
        finish(m, profile.id, loaded());
        assert!(m.workspace.cached_credentials.is_none());
        assert!(
            m.workspace
                .credential_status
                .as_ref()
                .unwrap()
                .contains("settings changed")
        );
    }

    // Rune: profile_restore_tests.rs, the same name. With several sessions in
    // one tree, the first to connect becomes the scope session and the second
    // does not take it over.
    #[test]
    fn concurrent_successes_connect_their_own_profiles_without_replacing_the_open_editor() {
        let mut fixture = Fixture::new();
        let listener = std::net::TcpListener::bind("127.0.0.1:0").unwrap();
        let endpoint =
            Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap();
        let backend =
            xenadmin_core::Backend::with_state_directory(fixture.directory.join("backend"))
                .unwrap();
        let m = &mut fixture.model;
        m.backend = Ok(backend);
        let mut a = save_profile(m, "First saved connection", true, false);
        let mut b = save_profile(m, "Second saved connection", true, false);
        for profile in [&mut a, &mut b] {
            profile.set_endpoint(endpoint.clone());
            m.workspace
                .store
                .as_mut()
                .unwrap()
                .save(profile.clone())
                .unwrap();
        }
        let editor = save_profile(m, "Unrelated editor", false, false);
        populate(m, &editor);
        m.workspace.show_connect = true;
        connect_all(m);
        finish(m, b.id, loaded());
        finish(m, a.id, loaded());
        assert!(m.workspace.active.contains_key(&a.id));
        assert!(m.workspace.active.contains_key(&b.id));
        assert_eq!(m.workspace.editing, Some(editor.id));
        assert_eq!(m.workspace.endpoint, editor.endpoint.as_str());
        assert!(m.workspace.password.is_empty());
        assert_eq!(m.workspace.selected, Some(b.id));
        assert!(m.connection.is_some());
        assert!(m.selection.scope.is_empty(), "the tree selection stays");
        // Both are listed, each with its own status word.
        let listed = sessions(m);
        assert_eq!(listed.len(), 3);
        assert!(listed.iter().filter(|s| s.state.is_some()).count() == 2);
        connect_all(m);
        assert!(!m.workspace.loads.contains_key(&a.id));
        assert!(!m.workspace.loads.contains_key(&b.id));
        disconnect_all(m);
    }

    // Rune: profile_restore_tests.rs, the same name.
    #[test]
    fn forgetting_saved_credentials_also_disables_startup_restore() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        let profile = save_profile(m, "Forget", true, true);
        populate(m, &profile);
        apply_credential_result(
            m,
            CredentialWork::Forget(profile.id),
            Ok(CredentialOutcome::Removed),
        );
        let stored = m.workspace.profile(profile.id).unwrap();
        assert_eq!(stored.credential_mode, CredentialMode::SessionOnly);
        assert!(!stored.auto_connect);
        assert!(!m.workspace.auto_connect);
    }

    // Rune: profile_restore_tests.rs, the same name.
    #[test]
    fn previews_do_not_restore_profiles_and_keyring_mutations_cannot_race_profile_saves() {
        let mut fixture = Fixture::new();
        let m = &mut fixture.model;
        let profile = save_profile(m, "Saved", true, true);
        m.demo = true;
        restore_sessions(m);
        assert!(m.workspace.loads.is_empty());
        m.demo = false;
        populate(m, &profile);
        m.workspace.credential_tasks.push((
            CredentialWork::Save(Box::new(profile.clone())),
            CredentialJob::save(
                CredentialKey::for_profile(&profile),
                Credentials::new("operator".into(), "synthetic-secret".into()),
                None,
            ),
        ));
        m.workspace.endpoint = "https://changed.example".into();
        save_only(m);
        assert!(
            m.error
                .as_ref()
                .unwrap()
                .contains("password-store operation")
        );
        assert_eq!(
            m.workspace.profile(profile.id).unwrap().endpoint,
            profile.endpoint
        );
        assert_eq!(m.workspace.credential_tasks.len(), 1);
    }

    #[test]
    fn a_fixture_workspace_touches_no_disk_and_cannot_connect() {
        let mut m = Model::fixture();
        assert!(!m.workspace.writable);
        assert!(m.workspace.store.is_err());
        assert!(inspection_connections(&m).next().is_none());
        begin_new(&mut m);
        m.workspace.endpoint = "host.example".into();
        m.workspace.password.set("secret");
        connect(&mut m);
        assert_eq!(
            m.error.as_deref(),
            Some("UI fixtures cannot connect to hosts")
        );
        assert!(m.workspace.active.is_empty());
        // Nothing is queued for the keyring either.
        connect_all(&mut m);
        restore_sessions(&mut m);
        forget_password(&mut m);
        assert!(m.workspace.credential_tasks.is_empty());
    }

    #[test]
    fn the_status_words_are_those_of_the_connection_list() {
        assert_eq!(status(None), "Disconnected");
        assert_eq!(status_word(Some(ConnectionState::Ready)), "Connected");
        assert_eq!(
            status_word(Some(ConnectionState::Connecting)),
            "Connecting…"
        );
        assert_eq!(
            status_word(Some(ConnectionState::Synchronizing)),
            "Loading inventory…"
        );
        assert_eq!(
            status_word(Some(ConnectionState::Reconnecting)),
            "Reconnecting…"
        );
        assert_eq!(
            status_word(Some(ConnectionState::Failed)),
            "Connection failed"
        );
        assert_eq!(
            status_word(Some(ConnectionState::Disconnected)),
            "Disconnected"
        );
    }
}
