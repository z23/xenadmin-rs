use std::{
    collections::HashMap,
    future::Future,
    sync::{Arc, Mutex},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use zbus::{
    Connection, Proxy,
    zvariant::{OwnedObjectPath, OwnedValue, Value},
};
use zeroize::Zeroizing;

use crate::{ConnectionProfile, Credentials, Endpoint, ProfileId};

const SERVICE: &str = "org.freedesktop.secrets";
const PATH: &str = "/org/freedesktop/secrets";
const SERVICE_INTERFACE: &str = "org.freedesktop.Secret.Service";
const ITEM_INTERFACE: &str = "org.freedesktop.Secret.Item";
const COLLECTION_INTERFACE: &str = "org.freedesktop.Secret.Collection";
const PROMPT_INTERFACE: &str = "org.freedesktop.Secret.Prompt";
const TIMEOUT: Duration = Duration::from_secs(12);

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum CredentialMode {
    #[default]
    SessionOnly,
    SecretService,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CredentialKey {
    profile: ProfileId,
    endpoint: Endpoint,
    username: String,
}
impl CredentialKey {
    pub fn for_profile(profile: &ConnectionProfile) -> Self {
        Self {
            profile: profile.id,
            endpoint: profile.endpoint.clone(),
            username: profile.username.clone(),
        }
    }
    fn attributes(&self) -> HashMap<&'static str, String> {
        let mut attributes = profile_attributes(self.profile);
        attributes.insert("endpoint", self.endpoint.to_string());
        attributes.insert("username", self.username.clone());
        attributes
    }
}
fn profile_attributes(profile: ProfileId) -> HashMap<&'static str, String> {
    HashMap::from([
        ("application", "xenadmin-rs".into()),
        ("profile", profile.to_string()),
    ])
}

#[derive(Clone, Debug, PartialEq, Eq, thiserror::Error)]
pub enum CredentialStoreError {
    #[error(
        "The desktop password store is unavailable. Enter a password for this session instead."
    )]
    Unavailable,
    #[error(
        "The desktop password store is locked. Unlock it in your password manager, then retry, or enter a password for this session."
    )]
    Locked,
    #[error("The desktop password store denied access. Enter a password for this session instead.")]
    Denied,
    #[error(
        "The desktop password store did not respond. Enter a password for this session instead."
    )]
    Timeout,
    #[error(
        "More than one saved password matches this connection. Remove duplicates in your password manager or enter a password for this session."
    )]
    Ambiguous,
    #[error(
        "The desktop password store returned an invalid response. Enter a password for this session instead."
    )]
    Invalid,
}
fn classify(error: zbus::Error) -> CredentialStoreError {
    match error {
        zbus::Error::MethodError(name, _, _) if name.as_str().ends_with(".IsLocked") => {
            CredentialStoreError::Locked
        }
        zbus::Error::MethodError(name, _, _)
            if name.as_str().ends_with(".AccessDenied")
                || name.as_str().ends_with(".PermissionDenied") =>
        {
            CredentialStoreError::Denied
        }
        _ => CredentialStoreError::Unavailable,
    }
}

#[derive(Serialize, Deserialize, zbus::zvariant::Type)]
struct SecretValue {
    session: OwnedObjectPath,
    parameters: Vec<u8>,
    value: Vec<u8>,
    content_type: String,
}

/// Secret Service on the current user's session bus. There is no file-based fallback.
/// Sessions use the standard `plain` algorithm on the local D-Bus connection.
#[derive(Default)]
pub struct CredentialStore {
    #[cfg(test)]
    test_connection: Option<Connection>,
}
/// Asks the store to unlock `objects`. A keyring that has no password of its own stays
/// locked only until something asks, and unlocks here at once. A store that wants its
/// password answers with a prompt of its own; that prompt is dismissed and never started,
/// because nothing on this path may open a dialog the person did not ask for, and the store
/// is reported as locked. The caller reads the store's state again afterwards.
async fn unlock(
    connection: &Connection,
    service: &Proxy<'_>,
    objects: &[OwnedObjectPath],
) -> Result<(), CredentialStoreError> {
    let (_, prompt): (Vec<OwnedObjectPath>, OwnedObjectPath) = service
        .call("Unlock", &(objects,))
        .await
        .map_err(classify)?;
    if prompt.as_str() == "/" {
        return Ok(());
    }
    if let Ok(prompt) = Proxy::new(connection, SERVICE, prompt.as_str(), PROMPT_INTERFACE).await {
        let _: Result<(), zbus::Error> = prompt.call("Dismiss", &()).await;
    }
    Err(CredentialStoreError::Locked)
}
/// The items that match `attributes`, after asking the store to unlock any it holds locked.
async fn search(
    connection: &Connection,
    service: &Proxy<'_>,
    attributes: HashMap<&'static str, String>,
) -> Result<Vec<OwnedObjectPath>, CredentialStoreError> {
    let find = || async {
        service
            .call::<_, _, (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>)>(
                "SearchItems",
                &(&attributes,),
            )
            .await
            .map_err(classify)
    };
    let (items, locked) = find().await?;
    if locked.is_empty() {
        return Ok(items);
    }
    unlock(connection, service, &locked).await?;
    let (items, locked) = find().await?;
    if locked.is_empty() {
        Ok(items)
    } else {
        Err(CredentialStoreError::Locked)
    }
}
/// Whether a collection is locked, read from the store each time and never from a cache.
async fn collection_locked(
    connection: &Connection,
    collection: &OwnedObjectPath,
) -> Result<bool, CredentialStoreError> {
    let reply = connection
        .call_method(
            Some(SERVICE),
            collection.as_str(),
            Some("org.freedesktop.DBus.Properties"),
            "Get",
            &(COLLECTION_INTERFACE, "Locked"),
        )
        .await
        .map_err(classify)?;
    let value: OwnedValue = reply
        .body()
        .deserialize()
        .map_err(|_| CredentialStoreError::Invalid)?;
    bool::try_from(value).map_err(|_| CredentialStoreError::Invalid)
}

impl CredentialStore {
    async fn connection(&self) -> Result<Connection, CredentialStoreError> {
        #[cfg(test)]
        if let Some(connection) = &self.test_connection {
            return Ok(connection.clone());
        }
        zbus::connection::Builder::session()
            .map_err(classify)?
            .method_timeout(TIMEOUT)
            .build()
            .await
            .map_err(classify)
    }
    pub async fn load(
        &self,
        key: &CredentialKey,
    ) -> Result<Option<Credentials>, CredentialStoreError> {
        timed(async {
            let connection = self.connection().await?;
            let service = Proxy::new(&connection, SERVICE, PATH, SERVICE_INTERFACE)
                .await
                .map_err(classify)?;
            let items = search(&connection, &service, key.attributes()).await?;
            let item = match items.as_slice() {
                [] => return Ok(None),
                [item] => item,
                _ => return Err(CredentialStoreError::Ambiguous),
            };
            let (_, session): (OwnedValue, OwnedObjectPath) = service
                .call("OpenSession", &("plain", Value::from("")))
                .await
                .map_err(classify)?;
            if session.as_str() == "/" {
                return Err(CredentialStoreError::Invalid);
            }
            let mut secrets: HashMap<OwnedObjectPath, SecretValue> = service
                .call("GetSecrets", &(&items, &session))
                .await
                .map_err(classify)?;
            if secrets.len() != 1 {
                return Err(CredentialStoreError::Invalid);
            }
            let SecretValue {
                session: returned_session,
                parameters,
                value,
                ..
            } = secrets.remove(item).ok_or(CredentialStoreError::Invalid)?;
            let value = Zeroizing::new(value);
            if returned_session != session
                || !parameters.is_empty()
                || value.is_empty()
                || value.len() > 65_536
            {
                return Err(CredentialStoreError::Invalid);
            }
            let password = std::str::from_utf8(&value)
                .map_err(|_| CredentialStoreError::Invalid)?
                .to_owned();
            Ok(Some(Credentials::new(key.username.clone(), password)))
        })
        .await
    }
    pub async fn save(
        &self,
        key: &CredentialKey,
        credentials: Credentials,
    ) -> Result<(), CredentialStoreError> {
        if credentials.username != key.username
            || credentials.password.is_empty()
            || credentials.password.len() > 65_536
        {
            return Err(CredentialStoreError::Invalid);
        }
        timed(async {
            let connection = self.connection().await?;
            let service = Proxy::new(&connection, SERVICE, PATH, SERVICE_INTERFACE)
                .await
                .map_err(classify)?;
            let collection: OwnedObjectPath = service
                .call("ReadAlias", &("default",))
                .await
                .map_err(classify)?;
            if collection.as_str() == "/" {
                return Err(CredentialStoreError::Unavailable);
            }
            if collection_locked(&connection, &collection).await? {
                unlock(&connection, &service, std::slice::from_ref(&collection)).await?;
                if collection_locked(&connection, &collection).await? {
                    return Err(CredentialStoreError::Locked);
                }
            }
            let collection = Proxy::new(
                &connection,
                SERVICE,
                collection.as_str(),
                COLLECTION_INTERFACE,
            )
            .await
            .map_err(classify)?;
            let (_, session): (OwnedValue, OwnedObjectPath) = service
                .call("OpenSession", &("plain", Value::from("")))
                .await
                .map_err(classify)?;
            if session.as_str() == "/" {
                return Err(CredentialStoreError::Invalid);
            }
            let label = format!("XenAdmin RS — {} at {}", key.username, key.endpoint);
            let properties = HashMap::from([
                (
                    "org.freedesktop.Secret.Item.Label",
                    Value::from(label.as_str()),
                ),
                (
                    "org.freedesktop.Secret.Item.Attributes",
                    Value::from(key.attributes()),
                ),
            ]);
            let secret = (
                &session,
                &[] as &[u8],
                credentials.password.as_bytes(),
                "text/plain; charset=utf8",
            );
            let (item, prompt): (OwnedObjectPath, OwnedObjectPath) = collection
                .call("CreateItem", &(properties, secret, true))
                .await
                .map_err(classify)?;
            if prompt.as_str() != "/" || item.as_str() == "/" {
                return Err(CredentialStoreError::Locked);
            }
            Ok(())
        })
        .await
    }
    /// Remove all saved secrets for a profile, including previous endpoint/account bindings.
    pub async fn remove(&self, profile: ProfileId) -> Result<(), CredentialStoreError> {
        timed(async {
            let connection = self.connection().await?;
            let service = Proxy::new(&connection, SERVICE, PATH, SERVICE_INTERFACE)
                .await
                .map_err(classify)?;
            let items = search(&connection, &service, profile_attributes(profile)).await?;
            if items.len() > 128 {
                return Err(CredentialStoreError::Invalid);
            }
            for item in items {
                let item = Proxy::new(&connection, SERVICE, item.as_str(), ITEM_INTERFACE)
                    .await
                    .map_err(classify)?;
                let prompt: OwnedObjectPath = item.call("Delete", &()).await.map_err(classify)?;
                if prompt.as_str() != "/" {
                    return Err(CredentialStoreError::Locked);
                }
            }
            Ok(())
        })
        .await
    }
}
async fn timed<T>(
    future: impl Future<Output = Result<T, CredentialStoreError>>,
) -> Result<T, CredentialStoreError> {
    tokio::time::timeout(TIMEOUT, future)
        .await
        .map_err(|_| CredentialStoreError::Timeout)?
}

pub enum CredentialOutcome {
    Loaded(Option<Credentials>),
    Saved,
    Removed,
}
impl std::fmt::Debug for CredentialOutcome {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_str(match self {
            Self::Loaded(_) => "Loaded([redacted])",
            Self::Saved => "Saved",
            Self::Removed => "Removed",
        })
    }
}
type TaskResult = Result<CredentialOutcome, CredentialStoreError>;
pub struct CredentialTask {
    result: Arc<Mutex<Option<TaskResult>>>,
}
impl CredentialTask {
    pub fn load(key: CredentialKey, wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        Self::spawn(
            async move {
                CredentialStore::default()
                    .load(&key)
                    .await
                    .map(CredentialOutcome::Loaded)
            },
            wake,
        )
    }
    pub fn save(
        key: CredentialKey,
        credentials: Credentials,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Self {
        Self::spawn(
            async move {
                CredentialStore::default()
                    .save(&key, credentials)
                    .await
                    .map(|()| CredentialOutcome::Saved)
            },
            wake,
        )
    }
    pub fn remove(profile: ProfileId, wake: Option<Arc<dyn Fn() + Send + Sync>>) -> Self {
        Self::spawn(
            async move {
                CredentialStore::default()
                    .remove(profile)
                    .await
                    .map(|()| CredentialOutcome::Removed)
            },
            wake,
        )
    }
    fn spawn(
        future: impl Future<Output = TaskResult> + Send + 'static,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Self {
        let result = Arc::new(Mutex::new(None));
        let target = result.clone();
        let error_target = result.clone();
        let error_wake = wake.clone();
        if std::thread::Builder::new()
            .name("xenadmin-credentials".into())
            .spawn(move || {
                let value = match tokio::runtime::Builder::new_current_thread()
                    .enable_all()
                    .build()
                {
                    Ok(runtime) => runtime.block_on(future),
                    Err(_) => Err(CredentialStoreError::Unavailable),
                };
                *target.lock().unwrap_or_else(|poison| poison.into_inner()) = Some(value);
                if let Some(wake) = wake {
                    wake();
                }
            })
            .is_err()
        {
            *error_target
                .lock()
                .unwrap_or_else(|poison| poison.into_inner()) =
                Some(Err(CredentialStoreError::Unavailable));
            if let Some(wake) = error_wake {
                wake();
            }
        }
        Self { result }
    }
    pub fn take_result(&self) -> Option<TaskResult> {
        self.result
            .lock()
            .unwrap_or_else(|poison| poison.into_inner())
            .take()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const COLLECTION: &str = "/org/freedesktop/secrets/collection/default";
    const ITEM: &str = "/org/freedesktop/secrets/collection/default/item";
    const SESSION: &str = "/org/freedesktop/secrets/session/fixture";
    const PROMPT: &str = "/org/freedesktop/secrets/prompt/fixture";
    fn path(value: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(value).unwrap()
    }
    #[derive(Default)]
    struct Saved {
        attributes: HashMap<String, String>,
        secret: Vec<u8>,
        locked: bool,
        omit_secret: bool,
        /// Whether unlocking needs the store's own password prompt.
        needs_password: bool,
        unlock_requests: Vec<Vec<String>>,
        prompts_started: usize,
        prompts_dismissed: usize,
    }
    struct Service(Arc<Mutex<Saved>>);
    #[zbus::interface(name = "org.freedesktop.Secret.Service")]
    impl Service {
        #[zbus(out_args("output", "result"))]
        fn open_session(
            &self,
            algorithm: &str,
            _input: Value<'_>,
        ) -> (OwnedValue, OwnedObjectPath) {
            assert_eq!(algorithm, "plain");
            (Value::from("").try_to_owned().unwrap(), path(SESSION))
        }
        fn read_alias(&self, alias: &str) -> OwnedObjectPath {
            assert_eq!(alias, "default");
            path(COLLECTION)
        }
        #[zbus(out_args("unlocked", "locked"))]
        fn search_items(
            &self,
            attributes: HashMap<String, String>,
        ) -> (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) {
            let state = self.0.lock().unwrap();
            if state.secret.is_empty()
                || !attributes
                    .iter()
                    .all(|(key, value)| state.attributes.get(key) == Some(value))
            {
                return (vec![], vec![]);
            }
            if state.locked {
                (vec![], vec![path(ITEM)])
            } else {
                (vec![path(ITEM)], vec![])
            }
        }
        #[zbus(out_args("unlocked", "prompt"))]
        fn unlock(&self, objects: Vec<OwnedObjectPath>) -> (Vec<OwnedObjectPath>, OwnedObjectPath) {
            let mut state = self.0.lock().unwrap();
            state
                .unlock_requests
                .push(objects.iter().map(|o| o.as_str().to_owned()).collect());
            if state.needs_password {
                (vec![], path(PROMPT))
            } else {
                state.locked = false;
                (objects, path("/"))
            }
        }
        fn get_secrets(
            &self,
            items: Vec<OwnedObjectPath>,
            session: OwnedObjectPath,
        ) -> HashMap<OwnedObjectPath, SecretValue> {
            assert_eq!(items, vec![path(ITEM)]);
            let state = self.0.lock().unwrap();
            if state.omit_secret {
                return HashMap::new();
            }
            HashMap::from([(
                path(ITEM),
                SecretValue {
                    session,
                    parameters: vec![],
                    value: state.secret.clone(),
                    content_type: "text/plain; charset=utf8".into(),
                },
            )])
        }
    }
    struct Collection(Arc<Mutex<Saved>>);
    #[zbus::interface(name = "org.freedesktop.Secret.Collection")]
    impl Collection {
        #[zbus(property)]
        fn locked(&self) -> bool {
            self.0.lock().unwrap().locked
        }
        #[zbus(out_args("item", "prompt"))]
        fn create_item(
            &self,
            mut properties: HashMap<String, OwnedValue>,
            secret: SecretValue,
            replace: bool,
        ) -> (OwnedObjectPath, OwnedObjectPath) {
            assert!(replace);
            assert_eq!(secret.session.as_str(), SESSION);
            assert!(secret.parameters.is_empty());
            let attributes = HashMap::<String, String>::try_from(
                properties
                    .remove("org.freedesktop.Secret.Item.Attributes")
                    .unwrap(),
            )
            .unwrap();
            let mut state = self.0.lock().unwrap();
            state.attributes = attributes;
            state.secret = secret.value;
            (path(ITEM), path("/"))
        }
    }
    struct Prompt(Arc<Mutex<Saved>>);
    #[zbus::interface(name = "org.freedesktop.Secret.Prompt")]
    impl Prompt {
        fn prompt(&self, _window: &str) {
            self.0.lock().unwrap().prompts_started += 1;
        }
        fn dismiss(&self) {
            self.0.lock().unwrap().prompts_dismissed += 1;
        }
    }
    struct Item(Arc<Mutex<Saved>>);
    #[zbus::interface(name = "org.freedesktop.Secret.Item")]
    impl Item {
        fn delete(&self) -> OwnedObjectPath {
            let mut state = self.0.lock().unwrap();
            state.attributes.clear();
            state.secret.clear();
            path("/")
        }
    }
    async fn fixture() -> (CredentialStore, Connection, Arc<Mutex<Saved>>) {
        let (client, server) = tokio::net::UnixStream::pair().unwrap();
        let state = Arc::new(Mutex::new(Saved::default()));
        let server = zbus::connection::Builder::unix_stream(server)
            .server(zbus::Guid::generate())
            .unwrap()
            .p2p()
            .serve_at(PATH, Service(state.clone()))
            .unwrap()
            .serve_at(COLLECTION, Collection(state.clone()))
            .unwrap()
            .serve_at(ITEM, Item(state.clone()))
            .unwrap()
            .serve_at(PROMPT, Prompt(state.clone()))
            .unwrap()
            .build();
        let client = zbus::connection::Builder::unix_stream(client).p2p().build();
        let (server, client) = tokio::join!(server, client);
        (
            CredentialStore {
                test_connection: Some(client.unwrap()),
            },
            server.unwrap(),
            state,
        )
    }
    fn profile() -> ConnectionProfile {
        ConnectionProfile::new(
            "Fixture".into(),
            Endpoint::parse("host.example").unwrap(),
            "operator".into(),
        )
    }

    #[tokio::test]
    async fn private_secret_service_round_trip_is_bound_to_profile_endpoint_and_account() {
        let (store, _server, state) = fixture().await;
        let mut profile = profile();
        let key = CredentialKey::for_profile(&profile);
        assert!(store.load(&key).await.unwrap().is_none());
        store
            .save(
                &key,
                Credentials::new("operator".into(), "fixture-password".into()),
            )
            .await
            .unwrap();
        let credentials = store.load(&key).await.unwrap().unwrap();
        assert_eq!(credentials.username, "operator");
        assert_eq!(credentials.password.as_str(), "fixture-password");
        assert!(!format!("{credentials:?}").contains("fixture-password"));
        profile.username = "other".into();
        assert!(
            store
                .load(&CredentialKey::for_profile(&profile))
                .await
                .unwrap()
                .is_none()
        );
        profile.username = "operator".into();
        profile.set_endpoint(Endpoint::parse("different.example").unwrap());
        assert!(
            store
                .load(&CredentialKey::for_profile(&profile))
                .await
                .unwrap()
                .is_none()
        );
        store.remove(profile.id).await.unwrap();
        assert!(state.lock().unwrap().secret.is_empty());
        assert!(store.load(&key).await.unwrap().is_none());
    }

    #[tokio::test]
    async fn locked_and_malformed_stores_fail_without_falling_back_to_files() {
        let (store, _server, state) = fixture().await;
        let key = CredentialKey::for_profile(&profile());
        store
            .save(
                &key,
                Credentials::new("operator".into(), "fixture-password".into()),
            )
            .await
            .unwrap();
        // A store that wants its password stays locked: its prompt is dismissed, never
        // started, and nothing is read or written.
        state.lock().unwrap().locked = true;
        state.lock().unwrap().needs_password = true;
        assert_eq!(
            store.load(&key).await.unwrap_err(),
            CredentialStoreError::Locked
        );
        assert_eq!(
            store
                .save(
                    &key,
                    Credentials::new("operator".into(), "replacement".into())
                )
                .await
                .unwrap_err(),
            CredentialStoreError::Locked
        );
        assert_eq!(
            store.remove(key.profile).await.unwrap_err(),
            CredentialStoreError::Locked
        );
        assert_eq!(state.lock().unwrap().secret, b"fixture-password");
        {
            let state = state.lock().unwrap();
            assert_eq!(state.unlock_requests.len(), 3);
            assert_eq!(state.prompts_dismissed, 3);
            assert_eq!(state.prompts_started, 0);
        }
        state.lock().unwrap().locked = false;
        state.lock().unwrap().omit_secret = true;
        assert_eq!(
            store.load(&key).await.unwrap_err(),
            CredentialStoreError::Invalid
        );
    }

    #[tokio::test]
    async fn a_store_that_needs_no_password_is_unlocked_on_demand() {
        let (store, _server, state) = fixture().await;
        let key = CredentialKey::for_profile(&profile());
        // Saving into a locked collection asks for the collection.
        state.lock().unwrap().locked = true;
        store
            .save(
                &key,
                Credentials::new("operator".into(), "fixture-password".into()),
            )
            .await
            .unwrap();
        assert_eq!(
            state.lock().unwrap().unlock_requests,
            [vec![COLLECTION.to_owned()]]
        );
        // Reading and removing a locked item ask for the item.
        state.lock().unwrap().locked = true;
        let credentials = store.load(&key).await.unwrap().unwrap();
        assert_eq!(credentials.password.as_str(), "fixture-password");
        assert!(!state.lock().unwrap().locked);
        state.lock().unwrap().locked = true;
        store.remove(key.profile).await.unwrap();
        let state = state.lock().unwrap();
        assert!(state.secret.is_empty());
        assert_eq!(
            state.unlock_requests[1..],
            [vec![ITEM.to_owned()], vec![ITEM.to_owned()]]
        );
        assert_eq!((state.prompts_started, state.prompts_dismissed), (0, 0));
    }

    /// Against a real Secret Service. It writes and removes one throwaway item, so it runs
    /// only when `XENADMIN_TEST_PRIVATE_SECRET_SERVICE` repeats the session bus address it
    /// may use, which must be a private bus and not the person's own. With no stage it
    /// saves, reads and removes the item. `XENADMIN_TEST_SECRET_STAGE` splits that up for a
    /// store that is locked in between: `seed` saves, `locked` expects every call refused
    /// as locked, `remove` cleans up.
    #[tokio::test]
    #[ignore = "needs a private session bus with a Secret Service"]
    async fn a_real_store_round_trip_on_a_private_bus() {
        let Ok(address) = std::env::var("XENADMIN_TEST_PRIVATE_SECRET_SERVICE") else {
            eprintln!("XENADMIN_TEST_PRIVATE_SECRET_SERVICE not set; skipping");
            return;
        };
        assert_eq!(
            std::env::var("DBUS_SESSION_BUS_ADDRESS").as_deref(),
            Ok(address.as_str()),
            "the session bus must be the private one"
        );
        let store = CredentialStore::default();
        let mut profile = profile();
        // A fixed profile, so that the stages of one run find each other's item.
        profile.id = serde_json::from_str("\"99999999-9999-4999-8999-999999999999\"").unwrap();
        let key = CredentialKey::for_profile(&profile);
        let secret = || Credentials::new("operator".into(), "fixture-password".into());
        match std::env::var("XENADMIN_TEST_SECRET_STAGE").as_deref() {
            Ok("seed") => store.save(&key, secret()).await.unwrap(),
            Ok("locked") => {
                assert_eq!(
                    store.load(&key).await.unwrap_err(),
                    CredentialStoreError::Locked
                );
                assert_eq!(
                    store.save(&key, secret()).await.unwrap_err(),
                    CredentialStoreError::Locked
                );
                assert_eq!(
                    store.remove(key.profile).await.unwrap_err(),
                    CredentialStoreError::Locked
                );
            }
            Ok("remove") => store.remove(key.profile).await.unwrap(),
            _ => {
                assert!(store.load(&key).await.unwrap().is_none());
                store.save(&key, secret()).await.unwrap();
                let credentials = store.load(&key).await.unwrap().unwrap();
                assert_eq!(credentials.password.as_str(), "fixture-password");
                store.remove(key.profile).await.unwrap();
                assert!(store.load(&key).await.unwrap().is_none());
            }
        }
    }
}
