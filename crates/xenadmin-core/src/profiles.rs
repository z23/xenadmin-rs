use std::{
    collections::BTreeMap,
    fs,
    io::Write,
    path::{Path, PathBuf},
    time::Duration,
};

use serde::{Deserialize, Serialize};
use uuid::Uuid;

use crate::{CertificateFingerprint, ConnectionOptions, Endpoint, PoolId};

#[derive(Clone, Copy, Debug, PartialEq, Eq, PartialOrd, Ord, Serialize, Deserialize)]
#[serde(transparent)]
pub struct ProfileId(Uuid);
impl ProfileId {
    pub fn new() -> Self {
        Self(Uuid::new_v4())
    }
}
impl Default for ProfileId {
    fn default() -> Self {
        Self::new()
    }
}
impl std::fmt::Display for ProfileId {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ConnectionProfile {
    pub id: ProfileId,
    pub name: String,
    pub endpoint: Endpoint,
    pub username: String,
    #[serde(default)]
    pub credential_mode: crate::CredentialMode,
    #[serde(default)]
    pub auto_connect: bool,
    #[serde(default)]
    pub ca_certificate: Option<PathBuf>,
    #[serde(default)]
    pub host_certificates: BTreeMap<String, CertificateFingerprint>,
    #[serde(default)]
    pub accept_invalid_certificates: bool,
    #[serde(default)]
    pub read_only: bool,
    #[serde(default = "default_timeout")]
    pub request_timeout_seconds: u64,
    #[serde(default)]
    pub pool: Option<PoolId>,
    #[serde(default)]
    pub verified_members: Vec<Endpoint>,
}
fn default_timeout() -> u64 {
    45
}
impl ConnectionProfile {
    pub fn new(name: String, endpoint: Endpoint, username: String) -> Self {
        Self {
            id: ProfileId::new(),
            name,
            endpoint,
            username,
            credential_mode: crate::CredentialMode::SessionOnly,
            auto_connect: false,
            ca_certificate: None,
            host_certificates: BTreeMap::new(),
            accept_invalid_certificates: false,
            read_only: false,
            request_timeout_seconds: default_timeout(),
            pool: None,
            verified_members: Vec::new(),
        }
    }
    pub fn options(&self) -> Result<ConnectionOptions, ProfileError> {
        self.validate()?;
        let mut options = ConnectionOptions::new(self.endpoint.clone());
        options.ca_certificate = self.ca_certificate.clone();
        options.accept_invalid_certificates = self.accept_invalid_certificates;
        options.read_only = self.read_only;
        options.request_timeout = Duration::from_secs(self.request_timeout_seconds);
        options.expected_pool = self.pool;
        if self.pool.is_some() {
            options.fallback_endpoints = self.verified_members.clone();
        }
        for (endpoint, pin) in &self.host_certificates {
            let endpoint = Endpoint::parse(endpoint).map_err(|_| ProfileError::Invalid)?;
            if endpoint == self.endpoint {
                options.certificate_fingerprint = Some(pin.clone());
            }
            options.add_host_certificate(endpoint, pin.clone());
        }
        Ok(options)
    }
    pub fn set_endpoint(&mut self, endpoint: Endpoint) {
        if endpoint != self.endpoint {
            self.endpoint = endpoint;
            self.accept_invalid_certificates = false;
            self.pool = None;
            self.verified_members.clear();
        }
    }
    pub fn remember_member(
        &mut self,
        pool: PoolId,
        endpoint: Endpoint,
    ) -> Result<(), ProfileError> {
        if self.pool.is_some_and(|existing| existing != pool) {
            return Err(ProfileError::PoolChanged);
        }
        self.pool = Some(pool);
        if !self.verified_members.contains(&endpoint) {
            self.verified_members.push(endpoint);
        }
        Ok(())
    }
    fn validate(&self) -> Result<(), ProfileError> {
        if self.name.trim().is_empty()
            || self.name.len() > 255
            || self.name.chars().any(char::is_control)
            || self.username.trim().is_empty()
            || self.username.len() > 255
            || self.username.chars().any(char::is_control)
            || !(5..=300).contains(&self.request_timeout_seconds)
            || (self.auto_connect && self.credential_mode != crate::CredentialMode::SecretService)
            || self.verified_members.len() > 128
            || self.host_certificates.len() > 128
            || self
                .host_certificates
                .keys()
                .any(|endpoint| Endpoint::parse(endpoint).is_err())
        {
            return Err(ProfileError::Invalid);
        }
        Ok(())
    }
}

#[derive(Debug, thiserror::Error)]
pub enum ProfileError {
    #[error("Cannot read or save connection profiles")]
    Storage,
    #[error("Connection profiles use a newer format; use a newer XenAdmin RS version")]
    NewerVersion,
    #[error("Connection profile is invalid; check its name, username, endpoint and timeout")]
    Invalid,
    #[error("The endpoint belongs to a different pool; create a separate connection profile")]
    PoolChanged,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ProfileDocument {
    version: u32,
    profiles: Vec<ConnectionProfile>,
}

pub struct ProfileStore {
    directory: PathBuf,
    profiles: BTreeMap<ProfileId, ConnectionProfile>,
}
impl ProfileStore {
    pub fn open(directory: PathBuf) -> Result<Self, ProfileError> {
        let path = directory.join("profiles.json");
        let mut store = Self {
            directory,
            profiles: BTreeMap::new(),
        };
        match fs::read(&path) {
            Ok(bytes) => {
                if bytes.len() > 2 * 1024 * 1024 {
                    return Err(ProfileError::Invalid);
                }
                let value: serde_json::Value =
                    serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
                if value
                    .get("version")
                    .and_then(serde_json::Value::as_u64)
                    .is_some_and(|version| version > 2)
                {
                    return Err(ProfileError::NewerVersion);
                }
                let document: ProfileDocument =
                    serde_json::from_value(value).map_err(|_| ProfileError::Invalid)?;
                if !(1..=2).contains(&document.version) || document.profiles.len() > 1000 {
                    return Err(ProfileError::Invalid);
                }
                for profile in document.profiles {
                    profile.validate()?;
                    if store.profiles.insert(profile.id, profile).is_some() {
                        return Err(ProfileError::Invalid);
                    }
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
                if let Some(profile) = legacy_profile(&store.directory)? {
                    store.profiles.insert(profile.id, profile);
                    store.persist(&store.profiles)?;
                }
            }
            Err(_) => return Err(ProfileError::Storage),
        }
        Ok(store)
    }
    pub fn profiles(&self) -> impl Iterator<Item = &ConnectionProfile> {
        self.profiles.values()
    }
    pub fn get(&self, id: ProfileId) -> Option<&ConnectionProfile> {
        self.profiles.get(&id)
    }
    pub fn save(&mut self, profile: ConnectionProfile) -> Result<(), ProfileError> {
        profile.validate()?;
        if !self.profiles.contains_key(&profile.id) && self.profiles.len() >= 1000 {
            return Err(ProfileError::Invalid);
        }
        let mut profiles = self.profiles.clone();
        profiles.insert(profile.id, profile);
        self.persist(&profiles)?;
        self.profiles = profiles;
        Ok(())
    }
    pub fn remove(&mut self, id: ProfileId) -> Result<bool, ProfileError> {
        let mut profiles = self.profiles.clone();
        let removed = profiles.remove(&id).is_some();
        if removed {
            self.persist(&profiles)?;
            self.profiles = profiles;
        }
        Ok(removed)
    }
    fn persist(
        &self,
        profiles: &BTreeMap<ProfileId, ConnectionProfile>,
    ) -> Result<(), ProfileError> {
        use std::os::unix::fs::{DirBuilderExt, OpenOptionsExt};
        fs::DirBuilder::new()
            .recursive(true)
            .mode(0o700)
            .create(&self.directory)
            .map_err(|_| ProfileError::Storage)?;
        let bytes = serde_json::to_vec_pretty(&ProfileDocument {
            version: 2,
            profiles: profiles.values().cloned().collect(),
        })
        .map_err(|_| ProfileError::Storage)?;
        let temporary = self
            .directory
            .join(format!("profiles.{}.tmp", Uuid::new_v4()));
        let result = (|| {
            let mut file = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .mode(0o600)
                .open(&temporary)?;
            file.write_all(&bytes)?;
            file.sync_all()?;
            fs::rename(&temporary, self.directory.join("profiles.json"))?;
            fs::File::open(&self.directory)?.sync_all()
        })();
        if result.is_err() {
            let _ = fs::remove_file(&temporary);
        }
        result.map_err(|_| ProfileError::Storage)
    }
}

fn legacy_profile(directory: &Path) -> Result<Option<ConnectionProfile>, ProfileError> {
    #[derive(Deserialize, Default)]
    #[serde(default)]
    struct Legacy {
        endpoint: String,
        username: String,
        fingerprint: String,
        host_certificates: String,
        accept_invalid_certificates: bool,
    }
    let bytes = match fs::read(directory.join("preferences.json")) {
        Ok(bytes) => bytes,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => return Ok(None),
        Err(_) => return Err(ProfileError::Storage),
    };
    let old: Legacy = serde_json::from_slice(&bytes).map_err(|_| ProfileError::Invalid)?;
    if old.endpoint.trim().is_empty() {
        return Ok(None);
    }
    let endpoint = Endpoint::parse(&old.endpoint).map_err(|_| ProfileError::Invalid)?;
    let username = if old.username.trim().is_empty() {
        "root".into()
    } else {
        old.username
    };
    let mut profile = ConnectionProfile::new(endpoint.to_string(), endpoint.clone(), username);
    profile.accept_invalid_certificates = old.accept_invalid_certificates;
    let mut options = ConnectionOptions::new(endpoint);
    options
        .add_host_certificates(&old.host_certificates)
        .map_err(|_| ProfileError::Invalid)?;
    if !old.fingerprint.trim().is_empty() {
        options.certificate_fingerprint =
            Some(old.fingerprint.parse().map_err(|_| ProfileError::Invalid)?);
    }
    profile.host_certificates = options.host_certificates();
    profile.validate()?;
    Ok(Some(profile))
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn legacy_connection_migrates_once_and_credentials_are_not_saved() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(directory.path().join("preferences.json"), serde_json::json!({
            "endpoint":"https://lab.example", "username":"operator", "accept_invalid_certificates":true,
            "password":"never-import-this", "columns":{}
        }).to_string()).unwrap();
        let mut store = ProfileStore::open(directory.path().into()).unwrap();
        let profile = store.profiles().next().unwrap().clone();
        assert!(profile.accept_invalid_certificates);
        assert_eq!(profile.username, "operator");
        assert!(
            !fs::read_to_string(directory.path().join("profiles.json"))
                .unwrap()
                .contains("never-import-this")
        );
        assert!(store.remove(profile.id).unwrap());
        assert_eq!(
            ProfileStore::open(directory.path().into())
                .unwrap()
                .profiles()
                .count(),
            0
        );
    }
    #[test]
    fn editing_endpoint_drops_bypass_and_pool_members() {
        let mut profile = ConnectionProfile::new(
            "Lab".into(),
            Endpoint::parse("old.example").unwrap(),
            "root".into(),
        );
        profile.accept_invalid_certificates = true;
        profile
            .remember_member(
                PoolId::parse("11111111-1111-1111-1111-111111111111").unwrap(),
                Endpoint::parse("member.example").unwrap(),
            )
            .unwrap();
        profile.set_endpoint(Endpoint::parse("new.example").unwrap());
        assert!(!profile.accept_invalid_certificates);
        assert!(profile.pool.is_none());
        assert!(profile.verified_members.is_empty());
    }
    #[test]
    fn future_formats_and_invalid_profiles_are_not_overwritten() {
        let directory = tempfile::tempdir().unwrap();
        fs::write(
            directory.path().join("profiles.json"),
            "{\"version\":3,\"unknown\":true}",
        )
        .unwrap();
        assert!(matches!(
            ProfileStore::open(directory.path().into()),
            Err(ProfileError::NewerVersion)
        ));
        assert_eq!(
            fs::read_to_string(directory.path().join("profiles.json")).unwrap(),
            "{\"version\":3,\"unknown\":true}"
        );
        fs::remove_file(directory.path().join("profiles.json")).unwrap();
        let mut store = ProfileStore::open(directory.path().into()).unwrap();
        let profile = ConnectionProfile::new(
            "".into(),
            Endpoint::parse("lab.example").unwrap(),
            "root".into(),
        );
        assert!(store.save(profile).is_err());
        assert_eq!(store.profiles().count(), 0);
    }

    #[test]
    fn startup_restore_is_opt_in_and_requires_secret_service() {
        let directory = tempfile::tempdir().unwrap();
        let mut profile = ConnectionProfile::new(
            "Fixture".into(),
            Endpoint::parse("fixture.example").unwrap(),
            "operator".into(),
        );
        assert!(!profile.auto_connect);
        let mut legacy = serde_json::to_value(&profile).unwrap();
        legacy.as_object_mut().unwrap().remove("auto_connect");
        fs::write(
            directory.path().join("profiles.json"),
            serde_json::json!({"version":1,"profiles":[legacy]}).to_string(),
        )
        .unwrap();
        let mut store = ProfileStore::open(directory.path().into()).unwrap();
        assert!(!store.get(profile.id).unwrap().auto_connect);
        profile.auto_connect = true;
        assert!(store.save(profile.clone()).is_err());
        profile.credential_mode = crate::CredentialMode::SecretService;
        store.save(profile.clone()).unwrap();
        let loaded = ProfileStore::open(directory.path().into()).unwrap();
        assert!(loaded.get(profile.id).unwrap().auto_connect);
        let document: serde_json::Value =
            serde_json::from_slice(&fs::read(directory.path().join("profiles.json")).unwrap())
                .unwrap();
        assert_eq!(document["version"], 2);
        assert!(document["profiles"][0].get("password").is_none());
    }
}
