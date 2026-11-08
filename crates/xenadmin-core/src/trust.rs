use std::{collections::BTreeMap, fmt, path::PathBuf, str::FromStr, sync::Arc, time::Duration};

use rustls::client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier};
use rustls::pki_types::{CertificateDer, ServerName, UnixTime};
use rustls::{DigitallySignedStruct, SignatureScheme};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use url::Url;
use zeroize::Zeroizing;

use crate::Error;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct Endpoint(Url);

impl Endpoint {
    pub fn parse(input: &str) -> Result<Self, Error> {
        let input = input.trim();
        let address = if input.parse::<std::net::Ipv6Addr>().is_ok() {
            format!("https://[{input}]")
        } else if input.contains("://") {
            input.to_owned()
        } else {
            format!("https://{input}")
        };
        let mut url = Url::parse(&address).map_err(|_| Error::InvalidEndpoint)?;
        let loopback = match url.host() {
            Some(url::Host::Ipv4(ip)) => ip.is_loopback(),
            Some(url::Host::Ipv6(ip)) => ip.is_loopback(),
            _ => false,
        };
        if url.host_str().is_none()
            || !(url.scheme() == "https" || (url.scheme() == "http" && loopback))
            || !url.username().is_empty()
            || url.password().is_some()
            || url.query().is_some()
            || url.fragment().is_some()
            || !matches!(url.path(), "" | "/" | "/jsonrpc")
        {
            return Err(Error::InvalidEndpoint);
        }
        url.set_path("/");
        Ok(Self(url))
    }

    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
    pub(crate) fn rpc_url(&self) -> Url {
        self.0.join("jsonrpc").expect("validated base URL")
    }
}

impl TryFrom<String> for Endpoint {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        Self::parse(&value)
    }
}
impl From<Endpoint> for String {
    fn from(value: Endpoint) -> String {
        value.0.into()
    }
}
impl fmt::Display for Endpoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        self.0.fmt(f)
    }
}

#[derive(Clone, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct CertificateFingerprint([u8; 32]);

impl FromStr for CertificateFingerprint {
    type Err = Error;
    fn from_str(value: &str) -> Result<Self, Error> {
        let value: String = value
            .trim()
            .chars()
            .filter(|c| *c != ':' && *c != ' ')
            .collect();
        if value.len() != 64 || !value.is_ascii() {
            return Err(Error::InvalidFingerprint);
        }
        let mut bytes = [0; 32];
        for (i, byte) in bytes.iter_mut().enumerate() {
            *byte = u8::from_str_radix(&value[i * 2..i * 2 + 2], 16)
                .map_err(|_| Error::InvalidFingerprint)?;
        }
        Ok(Self(bytes))
    }
}
impl TryFrom<String> for CertificateFingerprint {
    type Error = Error;
    fn try_from(value: String) -> Result<Self, Error> {
        value.parse()
    }
}
impl From<CertificateFingerprint> for String {
    fn from(value: CertificateFingerprint) -> Self {
        value.to_string()
    }
}
impl fmt::Display for CertificateFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        for byte in self.0 {
            write!(f, "{byte:02x}")?;
        }
        Ok(())
    }
}
impl fmt::Debug for CertificateFingerprint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        fmt::Display::fmt(self, f)
    }
}

pub struct Credentials {
    pub(crate) username: String,
    pub(crate) password: Zeroizing<String>,
}
impl Credentials {
    pub fn new(username: String, password: String) -> Self {
        Self {
            username,
            password: Zeroizing::new(password),
        }
    }
}
impl fmt::Debug for Credentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("Credentials([redacted])")
    }
}

#[derive(Clone, Debug)]
pub struct ConnectionOptions {
    pub endpoint: Endpoint,
    pub ca_certificate: Option<PathBuf>,
    pub certificate_fingerprint: Option<CertificateFingerprint>,
    /// Skip certificate identity/validity checks for this connection and its discovered endpoints.
    /// Overrides pins and custom CA settings; HTTPS and handshake signatures remain enabled.
    pub accept_invalid_certificates: bool,
    pub read_only: bool,
    pub request_timeout: Duration,
    pub expected_pool: Option<crate::PoolId>,
    pub fallback_endpoints: Vec<Endpoint>,
    additional_pins: BTreeMap<String, CertificateFingerprint>,
}
impl ConnectionOptions {
    pub fn new(endpoint: Endpoint) -> Self {
        Self {
            endpoint,
            ca_certificate: None,
            certificate_fingerprint: None,
            accept_invalid_certificates: false,
            read_only: false,
            request_timeout: Duration::from_secs(45),
            expected_pool: None,
            fallback_endpoints: Vec::new(),
            additional_pins: BTreeMap::new(),
        }
    }

    pub fn add_host_certificate(
        &mut self,
        endpoint: Endpoint,
        fingerprint: CertificateFingerprint,
    ) {
        self.additional_pins
            .insert(endpoint.to_string(), fingerprint);
    }
    pub fn add_host_certificates(&mut self, entries: &str) -> Result<(), Error> {
        for entry in entries
            .lines()
            .map(str::trim)
            .filter(|line| !line.is_empty())
        {
            let (endpoint, fingerprint) =
                entry.split_once('=').ok_or(Error::InvalidHostCertificate)?;
            self.add_host_certificate(
                Endpoint::parse(endpoint.trim())?,
                fingerprint.trim().parse()?,
            );
        }
        Ok(())
    }
    pub fn host_certificates(&self) -> BTreeMap<String, CertificateFingerprint> {
        let mut pins = self.additional_pins.clone();
        if let Some(pin) = &self.certificate_fingerprint {
            pins.insert(self.endpoint.to_string(), pin.clone());
        }
        pins
    }

    pub(crate) fn authority_key(&self, credentials: &Credentials, salt: &[u8]) -> [u8; 32] {
        let mut digest = Sha256::new();
        let mut field = |value: &[u8]| {
            digest.update((value.len() as u64).to_be_bytes());
            digest.update(value);
        };
        field(salt);
        field(credentials.username.as_bytes());
        field(credentials.password.as_bytes());
        field(&[
            u8::from(self.read_only),
            u8::from(self.accept_invalid_certificates),
        ]);
        if !self.accept_invalid_certificates {
            if let Some(path) = &self.ca_certificate {
                field(path.as_os_str().as_encoded_bytes());
                if let Ok(pem) = std::fs::read(path) {
                    field(&pem);
                }
            }
            for (endpoint, pin) in self.host_certificates() {
                field(endpoint.as_bytes());
                field(&pin.0);
            }
        }
        digest.finalize().into()
    }

    pub(crate) fn for_endpoint(&self, endpoint: Endpoint) -> Self {
        let mut options = self.clone();
        if let Some(fingerprint) = &self.certificate_fingerprint {
            options
                .additional_pins
                .insert(self.endpoint.to_string(), fingerprint.clone());
        }
        if endpoint.as_str() != self.endpoint.as_str() {
            options.certificate_fingerprint =
                options.additional_pins.get(endpoint.as_str()).cloned();
        }
        options.endpoint = endpoint;
        options
    }
    pub(crate) fn client(&self) -> Result<reqwest::Client, Error> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let mut builder = reqwest::Client::builder()
            .retry(reqwest::retry::never())
            .redirect(reqwest::redirect::Policy::none())
            .connect_timeout(Duration::from_secs(10))
            .timeout(self.request_timeout)
            .user_agent(concat!("xenadmin-rs/", env!("CARGO_PKG_VERSION")));
        if !self.accept_invalid_certificates
            && let Some(path) = &self.ca_certificate
        {
            let pem = std::fs::read(path).map_err(|_| Error::CertificateFile)?;
            builder = builder.add_root_certificate(
                reqwest::Certificate::from_pem(&pem).map_err(|_| Error::CertificateFile)?,
            );
        }
        if self.accept_invalid_certificates || self.certificate_fingerprint.is_some() {
            builder = builder.use_preconfigured_tls(self.tls_config()?);
        }
        builder.build().map_err(|_| Error::Transport)
    }

    pub(crate) fn tls_config(&self) -> Result<rustls::ClientConfig, Error> {
        let _ = rustls::crypto::ring::default_provider().install_default();
        let verifier = if self.accept_invalid_certificates {
            Some(CertificateVerifier::Unverified)
        } else {
            self.certificate_fingerprint
                .clone()
                .map(CertificateVerifier::Pinned)
        };
        if let Some(verifier) = verifier {
            return Ok(rustls::ClientConfig::builder()
                .dangerous()
                .with_custom_certificate_verifier(Arc::new(verifier))
                .with_no_client_auth());
        }
        let mut roots = rustls::RootCertStore::empty();
        for certificate in rustls_native_certs::load_native_certs().certs {
            let _ = roots.add(certificate);
        }
        if let Some(path) = &self.ca_certificate {
            let pem = std::fs::read(path).map_err(|_| Error::CertificateFile)?;
            for certificate in rustls_pemfile::certs(&mut &*pem) {
                roots
                    .add(certificate.map_err(|_| Error::CertificateFile)?)
                    .map_err(|_| Error::CertificateFile)?;
            }
        }
        Ok(rustls::ClientConfig::builder()
            .with_root_certificates(roots)
            .with_no_client_auth())
    }
}

#[derive(Debug)]
enum CertificateVerifier {
    Pinned(CertificateFingerprint),
    Unverified,
}
impl ServerCertVerifier for CertificateVerifier {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        _: &[CertificateDer<'_>],
        _: &ServerName<'_>,
        _: &[u8],
        _: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if match self {
            Self::Unverified => true,
            Self::Pinned(fingerprint) => Sha256::digest(cert.as_ref()).as_slice() == fingerprint.0,
        } {
            Ok(ServerCertVerified::assertion())
        } else {
            Err(rustls::Error::InvalidCertificate(
                rustls::CertificateError::ApplicationVerificationFailure,
            ))
        }
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls12_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        rustls::crypto::verify_tls13_signature(
            message,
            cert,
            dss,
            &rustls::crypto::ring::default_provider().signature_verification_algorithms,
        )
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        rustls::crypto::ring::default_provider()
            .signature_verification_algorithms
            .supported_schemes()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn certificate_pins_are_scoped_to_the_exact_endpoint() {
        let mut options = ConnectionOptions::new(Endpoint::parse("https://first.example").unwrap());
        options.certificate_fingerprint = Some("aa".repeat(32).parse().unwrap());
        options
            .add_host_certificates(&format!("https://second.example={}\n", "bb".repeat(32)))
            .unwrap();
        assert!(
            options
                .for_endpoint(Endpoint::parse("https://third.example").unwrap())
                .certificate_fingerprint
                .is_none()
        );
        assert!(
            options
                .for_endpoint(Endpoint::parse("https://first.example:444").unwrap())
                .certificate_fingerprint
                .is_none()
        );
        assert_eq!(
            options
                .for_endpoint(Endpoint::parse("https://second.example").unwrap())
                .certificate_fingerprint
                .unwrap()
                .to_string(),
            "bb".repeat(32)
        );
    }
    #[test]
    fn original_member_pin_survives_coordinator_redirect_and_console_return() {
        let member = Endpoint::parse("https://member.example").unwrap();
        let coordinator = Endpoint::parse("https://coordinator.example").unwrap();
        let mut options = ConnectionOptions::new(member.clone());
        options.certificate_fingerprint = Some("aa".repeat(32).parse().unwrap());
        options.add_host_certificate(coordinator.clone(), "bb".repeat(32).parse().unwrap());
        let redirected = options.for_endpoint(coordinator);
        assert_eq!(
            redirected.certificate_fingerprint.unwrap(),
            "bb".repeat(32).parse().unwrap()
        );
        let returned = options
            .for_endpoint(Endpoint::parse("https://coordinator.example").unwrap())
            .for_endpoint(member);
        assert_eq!(
            returned.certificate_fingerprint.unwrap(),
            "aa".repeat(32).parse().unwrap()
        );
        assert!(!returned.accept_invalid_certificates);
    }
    #[test]
    fn endpoints_reject_embedded_secrets_and_remote_plaintext() {
        for value in [
            "https://root:secret@host",
            "http://host",
            "https://host/?session_id=secret",
            "https://host/#fragment",
            "https://host/other",
        ] {
            assert!(Endpoint::parse(value).is_err(), "{value}");
        }
        assert_eq!(Endpoint::parse("host").unwrap().as_str(), "https://host/");
        assert!(Endpoint::parse("http://127.0.0.1:1234").is_ok());
    }
    #[test]
    fn credentials_debug_never_exposes_passwords() {
        let value = Credentials::new("operator".into(), "do-not-print".into());
        assert_eq!(format!("{value:?}"), "Credentials([redacted])");
    }
}
