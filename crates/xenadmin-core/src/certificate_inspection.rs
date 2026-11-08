use crate::{CertificateFingerprint, ConnectionOptions, Endpoint, Error};
use rustls::{
    DigitallySignedStruct, SignatureScheme,
    client::danger::{HandshakeSignatureValid, ServerCertVerified, ServerCertVerifier},
    pki_types::{CertificateDer, ServerName, UnixTime},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{
    io::Read,
    sync::{Arc, Mutex},
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};
use tokio_util::sync::CancellationToken;
use x509_parser::{extensions::GeneralName, parse_x509_certificate};

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct CertificateDetails {
    pub fingerprint: CertificateFingerprint,
    pub subject: String,
    pub issuer: String,
    pub names: Vec<String>,
    pub valid_from: String,
    pub valid_until: String,
    pub serial: String,
}
impl CertificateDetails {
    pub(crate) fn valid(&self) -> bool {
        [
            &self.subject,
            &self.issuer,
            &self.valid_from,
            &self.valid_until,
            &self.serial,
        ]
        .into_iter()
        .chain(self.names.iter())
        .all(|s| s.len() <= 1024 && !s.chars().any(char::is_control))
            && self.names.len() <= 64
    }
}
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub enum CertificateValidation {
    Trusted,
    UnknownIssuer,
    Expired,
    NotYetValid,
    HostnameMismatch,
    Rejected,
}
impl CertificateValidation {
    pub fn label(self) -> &'static str {
        match self {
            Self::Trusted => "CA chain, validity and hostname verified",
            Self::UnknownIssuer => "Certificate issuer is not trusted by the configured CA store",
            Self::Expired => "Certificate has expired",
            Self::NotYetValid => "Certificate is not yet valid",
            Self::HostnameMismatch => "Certificate names do not match this endpoint",
            Self::Rejected => "Certificate did not pass CA, validity or hostname verification",
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct CertificateObservation {
    pub endpoint: Endpoint,
    pub details: CertificateDetails,
    pub previous_fingerprint: Option<CertificateFingerprint>,
    pub validation: CertificateValidation,
    pub route: String,
    pub inspected_at_ms: u64,
    pub elapsed_ms: u64,
}
type Outcome = Result<CertificateObservation, Error>;
pub struct CertificateInspectionTask {
    result: Arc<Mutex<Option<Outcome>>>,
    cancel: CancellationToken,
}
impl CertificateInspectionTask {
    /// Performs TLS only. No XAPI credentials, cookies or application requests are sent.
    pub fn start(
        options: ConnectionOptions,
        endpoint: Endpoint,
        wake: Option<Arc<dyn Fn() + Send + Sync>>,
    ) -> Self {
        let result = Arc::new(Mutex::new(None));
        let target = result.clone();
        let error_target = result.clone();
        let error_wake = wake.clone();
        let cancel = CancellationToken::new();
        let stopped = cancel.clone();
        if std::thread::Builder::new().name("xenadmin-certificate".into()).spawn(move || {
            let outcome = match tokio::runtime::Builder::new_current_thread().enable_all().build() {
                Ok(runtime) => runtime.block_on(async {
                    tokio::select! { _ = stopped.cancelled() => Err(Error::Closed), value = inspect(&options, endpoint) => value }
                }),
                Err(_) => Err(Error::Runtime),
            };
            *target.lock().unwrap_or_else(|p| p.into_inner()) = Some(outcome);
            if let Some(wake) = wake { wake(); }
        }).is_err() {
            *error_target.lock().unwrap_or_else(|p| p.into_inner()) = Some(Err(Error::Runtime));
            if let Some(wake) = error_wake { wake(); }
        }
        Self { result, cancel }
    }
    pub fn take_result(&self) -> Option<Outcome> {
        self.result.lock().unwrap_or_else(|p| p.into_inner()).take()
    }
}
impl Drop for CertificateInspectionTask {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub(crate) fn ca_verifier(
    options: &ConnectionOptions,
) -> Result<Arc<dyn ServerCertVerifier>, Error> {
    let provider = Arc::new(rustls::crypto::ring::default_provider());
    let verifier = if let Some(path) = &options.ca_certificate {
        let mut pem = Vec::new();
        std::fs::File::open(path)
            .map_err(|_| Error::CertificateFile)?
            .take(1024 * 1024 + 1)
            .read_to_end(&mut pem)
            .map_err(|_| Error::CertificateFile)?;
        if pem.len() > 1024 * 1024 {
            return Err(Error::CertificateFile);
        }
        let certs = rustls_pemfile::certs(&mut &*pem)
            .collect::<Result<Vec<_>, _>>()
            .map_err(|_| Error::CertificateFile)?;
        if certs.is_empty() || certs.len() > 128 {
            return Err(Error::CertificateFile);
        }
        rustls_platform_verifier::Verifier::new_with_extra_roots(certs, provider)
    } else {
        rustls_platform_verifier::Verifier::new(provider)
    }
    .map_err(|_| Error::CertificateFile)?;
    Ok(Arc::new(verifier))
}
#[derive(Debug)]
struct Capture {
    standard: Arc<dyn ServerCertVerifier>,
    observation: Mutex<Option<(CertificateDetails, CertificateValidation)>>,
}
fn bounded(s: String) -> String {
    s.chars().filter(|c| !c.is_control()).take(256).collect()
}
fn details(der: &[u8]) -> Result<CertificateDetails, rustls::Error> {
    let bad = || rustls::Error::InvalidCertificate(rustls::CertificateError::BadEncoding);
    if der.len() > 128 * 1024 {
        return Err(bad());
    }
    let (rest, cert) = parse_x509_certificate(der).map_err(|_| bad())?;
    if !rest.is_empty() {
        return Err(bad());
    }
    let mut names = Vec::new();
    if let Some(extension) = cert.subject_alternative_name().map_err(|_| bad())? {
        if extension.value.general_names.len() > 64 {
            return Err(bad());
        }
        for name in &extension.value.general_names {
            match name {
                GeneralName::DNSName(name) => names.push(bounded((*name).into())),
                GeneralName::IPAddress(bytes) => {
                    let ip = match bytes.len() {
                        4 => std::net::IpAddr::V4(std::net::Ipv4Addr::from(
                            <[u8; 4]>::try_from(*bytes).map_err(|_| bad())?,
                        )),
                        16 => std::net::IpAddr::V6(std::net::Ipv6Addr::from(
                            <[u8; 16]>::try_from(*bytes).map_err(|_| bad())?,
                        )),
                        _ => return Err(bad()),
                    };
                    names.push(ip.to_string());
                }
                _ => {}
            }
        }
    }
    Ok(CertificateDetails {
        fingerprint: format!("{:x}", Sha256::digest(der))
            .parse()
            .map_err(|_| bad())?,
        subject: bounded(cert.subject().to_string()),
        issuer: bounded(cert.issuer().to_string()),
        names,
        valid_from: bounded(cert.validity().not_before.to_string()),
        valid_until: bounded(cert.validity().not_after.to_string()),
        serial: bounded(cert.raw_serial_as_string()),
    })
}
impl ServerCertVerifier for Capture {
    fn verify_server_cert(
        &self,
        cert: &CertificateDer<'_>,
        chain: &[CertificateDer<'_>],
        name: &ServerName<'_>,
        ocsp: &[u8],
        now: UnixTime,
    ) -> Result<ServerCertVerified, rustls::Error> {
        if chain.len() > 8 || chain.iter().map(|c| c.len()).sum::<usize>() > 1024 * 1024 {
            return Err(rustls::Error::General(
                "Certificate chain exceeds inspection bounds".into(),
            ));
        }
        let validation = match self
            .standard
            .verify_server_cert(cert, chain, name, ocsp, now)
        {
            Ok(_) => CertificateValidation::Trusted,
            Err(rustls::Error::InvalidCertificate(reason)) => match reason {
                rustls::CertificateError::UnknownIssuer => CertificateValidation::UnknownIssuer,
                rustls::CertificateError::Expired
                | rustls::CertificateError::ExpiredContext { .. } => CertificateValidation::Expired,
                rustls::CertificateError::NotValidYet
                | rustls::CertificateError::NotValidYetContext { .. } => {
                    CertificateValidation::NotYetValid
                }
                rustls::CertificateError::NotValidForName
                | rustls::CertificateError::NotValidForNameContext { .. } => {
                    CertificateValidation::HostnameMismatch
                }
                _ => CertificateValidation::Rejected,
            },
            Err(_) => CertificateValidation::Rejected,
        };
        *self.observation.lock().unwrap_or_else(|p| p.into_inner()) =
            Some((details(cert.as_ref())?, validation));
        Ok(ServerCertVerified::assertion())
    }
    fn verify_tls12_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.standard.verify_tls12_signature(message, cert, dss)
    }
    fn verify_tls13_signature(
        &self,
        message: &[u8],
        cert: &CertificateDer<'_>,
        dss: &DigitallySignedStruct,
    ) -> Result<HandshakeSignatureValid, rustls::Error> {
        self.standard.verify_tls13_signature(message, cert, dss)
    }
    fn supported_verify_schemes(&self) -> Vec<SignatureScheme> {
        self.standard.supported_verify_schemes()
    }
}
pub(crate) async fn inspect(options: &ConnectionOptions, endpoint: Endpoint) -> Outcome {
    let started = Instant::now();
    let url = url::Url::parse(endpoint.as_str()).map_err(|_| Error::InvalidEndpoint)?;
    if url.scheme() != "https" {
        return Err(Error::CertificateInspection);
    }
    let route = options.connection_route(&endpoint)?;
    let capture = Arc::new(Capture {
        standard: ca_verifier(options)?,
        observation: Mutex::new(None),
    });
    let config = rustls::ClientConfig::builder_with_provider(Arc::new(
        rustls::crypto::ring::default_provider(),
    ))
    .with_safe_default_protocol_versions()
    .map_err(|_| Error::CertificateInspection)?
    .dangerous()
    .with_custom_certificate_verifier(capture.clone())
    .with_no_client_auth();
    let name = ServerName::try_from(crate::connection_proxy::host_name(&url)?)
        .map_err(|_| Error::InvalidEndpoint)?;
    tokio::time::timeout(Duration::from_secs(15), async {
        let socket = options.endpoint_socket(&endpoint).await?;
        let stream = tokio_rustls::TlsConnector::from(Arc::new(config))
            .connect(name, socket)
            .await
            .map_err(|_| Error::CertificateInspection)?;
        drop(stream);
        Ok::<_, Error>(())
    })
    .await
    .map_err(|_| Error::Timeout)??;
    let (details, validation) = capture
        .observation
        .lock()
        .unwrap_or_else(|p| p.into_inner())
        .take()
        .ok_or(Error::CertificateInspection)?;
    Ok(CertificateObservation {
        previous_fingerprint: options.host_certificates().get(endpoint.as_str()).cloned(),
        endpoint,
        details,
        validation,
        route,
        inspected_at_ms: SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap_or_default()
            .as_millis() as u64,
        elapsed_ms: started.elapsed().as_millis() as u64,
    })
}
