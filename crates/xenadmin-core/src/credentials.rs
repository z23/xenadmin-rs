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
            let (items, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service
                .call("SearchItems", &(key.attributes(),))
                .await
                .map_err(classify)?;
            if !locked.is_empty() {
                return Err(CredentialStoreError::Locked);
            }
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
            let collection = Proxy::new(
                &connection,
                SERVICE,
                collection.as_str(),
                COLLECTION_INTERFACE,
            )
            .await
            .map_err(classify)?;
            if collection
                .get_property::<bool>("Locked")
                .await
                .map_err(classify)?
            {
                return Err(CredentialStoreError::Locked);
            }
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
            let (items, locked): (Vec<OwnedObjectPath>, Vec<OwnedObjectPath>) = service
                .call("SearchItems", &(profile_attributes(profile),))
                .await
                .map_err(classify)?;
            if !locked.is_empty() {
                return Err(CredentialStoreError::Locked);
            }
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
    fn path(value: &str) -> OwnedObjectPath {
        OwnedObjectPath::try_from(value).unwrap()
    }
    #[derive(Default)]
    struct Saved {
        attributes: HashMap<String, String>,
        secret: Vec<u8>,
        locked: bool,
        omit_secret: bool,
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
        state.lock().unwrap().locked = true;
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
        state.lock().unwrap().locked = false;
        state.lock().unwrap().omit_secret = true;
        assert_eq!(
            store.load(&key).await.unwrap_err(),
            CredentialStoreError::Invalid
        );
    }
}
