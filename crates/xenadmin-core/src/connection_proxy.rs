use crate::{ConnectionOptions, Endpoint, Error};
use base64::Engine;
use hyper_util::client::proxy::matcher::Matcher;
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::{fmt, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpStream,
};
use url::Url;
use zeroize::Zeroizing;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(try_from = "String", into = "String")]
pub struct ProxyEndpoint(Url);
impl ProxyEndpoint {
    pub fn parse(value: &str) -> Result<Self, Error> {
        let url = Url::parse(value.trim()).map_err(|_| Error::ProxyConfiguration)?;
        if url.scheme() != "http" {
            return Err(Error::UnsupportedProxy);
        }
        if url.host_str().is_none()
            || !url.username().is_empty()
            || url.password().is_some()
            || !matches!(url.path(), "" | "/")
            || url.query().is_some()
            || url.fragment().is_some()
        {
            return Err(Error::ProxyConfiguration);
        }
        Ok(Self(url))
    }
    pub fn as_str(&self) -> &str {
        self.0.as_str()
    }
}
impl TryFrom<String> for ProxyEndpoint {
    type Error = Error;
    fn try_from(v: String) -> Result<Self, Error> {
        Self::parse(&v)
    }
}
impl From<ProxyEndpoint> for String {
    fn from(v: ProxyEndpoint) -> Self {
        v.0.into()
    }
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ProxyConfiguration {
    #[default]
    Environment,
    Direct,
    Http {
        endpoint: ProxyEndpoint,
        username: Option<String>,
    },
}
impl ProxyConfiguration {
    pub(crate) fn valid(&self) -> bool {
        match self {
            Self::Http {
                username: Some(u), ..
            } => valid_username(u),
            _ => true,
        }
    }
}
fn valid_username(u: &str) -> bool {
    !u.is_empty() && u.len() <= 255 && !u.contains(':') && !u.chars().any(char::is_control)
}
#[derive(Clone)]
pub struct ProxyCredentials {
    username: String,
    authorization: Zeroizing<String>,
}
impl fmt::Debug for ProxyCredentials {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProxyCredentials([redacted])")
    }
}
impl ProxyCredentials {
    pub fn new(username: String, password: String) -> Result<Self, Error> {
        let password = Zeroizing::new(password);
        if !valid_username(&username) || password.is_empty() || password.len() > 4096 {
            return Err(Error::ProxyConfiguration);
        }
        let combined = Zeroizing::new(format!("{username}:{}", *password));
        let authorization = Zeroizing::new(format!(
            "Basic {}",
            base64::engine::general_purpose::STANDARD.encode(combined.as_bytes())
        ));
        Ok(Self {
            username,
            authorization,
        })
    }
}
#[derive(Clone)]
pub(crate) enum Route {
    Direct,
    Http {
        endpoint: ProxyEndpoint,
        authorization: Option<Arc<Zeroizing<String>>>,
    },
}
impl fmt::Debug for Route {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.description())
    }
}
impl Route {
    pub(crate) fn description(&self) -> String {
        match self {
            Self::Direct => "Direct connection".into(),
            Self::Http {
                endpoint,
                authorization,
            } => format!(
                "HTTP CONNECT via {}{}",
                endpoint.as_str(),
                if authorization.is_some() {
                    " (proxy authentication)"
                } else {
                    ""
                }
            ),
        }
    }
    fn apply(&self, builder: reqwest::ClientBuilder) -> Result<reqwest::ClientBuilder, Error> {
        let builder = builder.no_proxy();
        match self {
            Self::Direct => Ok(builder),
            Self::Http {
                endpoint,
                authorization,
            } => {
                let mut proxy = reqwest::Proxy::all(endpoint.as_str())
                    .map_err(|_| Error::ProxyConfiguration)?;
                if let Some(auth) = authorization {
                    let mut value =
                        http::HeaderValue::from_str(auth).map_err(|_| Error::ProxyConfiguration)?;
                    value.set_sensitive(true);
                    proxy = proxy.custom_http_auth(value);
                }
                Ok(builder.proxy(proxy))
            }
        }
    }
}

#[derive(Clone, Copy, Debug)]
enum EnvironmentError {
    Invalid,
    Unsupported,
}
type EnvironmentRoute = Result<Option<Route>, EnvironmentError>;
pub(crate) fn host_name(url: &Url) -> Result<String, Error> {
    match url.host().ok_or(Error::InvalidEndpoint)? {
        url::Host::Domain(name) => Ok(name.into()),
        url::Host::Ipv4(ip) => Ok(ip.to_string()),
        url::Host::Ipv6(ip) => Ok(ip.to_string()),
    }
}
#[derive(Clone)]
pub(crate) struct Environment {
    https: EnvironmentRoute,
    http: EnvironmentRoute,
    bypass: Arc<Matcher>,
    digest: [u8; 32],
}
impl fmt::Debug for Environment {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str("ProxyEnvironment([captured])")
    }
}
impl Environment {
    pub(crate) fn capture() -> Self {
        let get = |upper: &str, lower: &str| {
            Zeroizing::new(
                std::env::var(upper)
                    .or_else(|_| std::env::var(lower))
                    .unwrap_or_default(),
            )
        };
        Self::from_values(
            &get("ALL_PROXY", "all_proxy"),
            &get("HTTP_PROXY", "http_proxy"),
            &get("HTTPS_PROXY", "https_proxy"),
            &get("NO_PROXY", "no_proxy"),
            std::env::var_os("REQUEST_METHOD").is_some(),
        )
    }
    fn from_values(all: &str, http: &str, https: &str, no: &str, cgi: bool) -> Self {
        let mut hash = Sha256::new();
        for value in [all, http, https, no] {
            hash.update((value.len() as u64).to_be_bytes());
            hash.update(value.as_bytes());
        }
        hash.update([u8::from(cgi)]);
        if [all, http, https, no].into_iter().any(|v| v.len() > 8192) {
            return Self {
                https: Err(EnvironmentError::Invalid),
                http: Err(EnvironmentError::Invalid),
                bypass: Arc::new(Matcher::builder().all("http://bypass.invalid").build()),
                digest: hash.finalize().into(),
            };
        }
        let route = |specific: &str| {
            if cgi {
                return Ok(None);
            }
            let raw = if specific.trim().is_empty() {
                all
            } else {
                specific
            };
            if raw.is_empty() {
                return Ok(None);
            }
            if raw.len() > 8192 || no.len() > 8192 {
                return Err(EnvironmentError::Invalid);
            }
            let address = Zeroizing::new(if raw.contains("://") {
                raw.to_owned()
            } else {
                format!("http://{raw}")
            });
            let mut url = Url::parse(&address).map_err(|_| EnvironmentError::Invalid)?;
            if url.scheme() != "http" {
                return Err(EnvironmentError::Unsupported);
            }
            let matcher = Matcher::builder().all(address.as_str()).build();
            let auth = matcher
                .intercept(&http::Uri::from_static("https://destination.invalid/"))
                .and_then(|route| route.basic_auth().cloned())
                .map(|auth| {
                    auth.to_str()
                        .map(|v| Arc::new(Zeroizing::new(v.to_owned())))
                        .map_err(|_| EnvironmentError::Invalid)
                })
                .transpose()?;
            url.set_username("")
                .map_err(|_| EnvironmentError::Invalid)?;
            url.set_password(None)
                .map_err(|_| EnvironmentError::Invalid)?;
            let endpoint =
                ProxyEndpoint::parse(url.as_str()).map_err(|_| EnvironmentError::Invalid)?;
            Ok(Some(Route::Http {
                endpoint,
                authorization: auth,
            }))
        };
        Self {
            https: route(https),
            http: route(http),
            bypass: Arc::new(
                Matcher::builder()
                    .all("http://bypass.invalid")
                    .no(no)
                    .build(),
            ),
            digest: hash.finalize().into(),
        }
    }
    fn route(&self, endpoint: &Endpoint) -> Result<Route, Error> {
        let uri = endpoint
            .as_str()
            .parse::<http::Uri>()
            .map_err(|_| Error::InvalidEndpoint)?;
        if self.bypass.intercept(&uri).is_none() {
            return Ok(Route::Direct);
        }
        match if uri.scheme_str() == Some("http") {
            &self.http
        } else {
            &self.https
        } {
            Ok(Some(route)) => Ok(route.clone()),
            Ok(None) => Ok(Route::Direct),
            Err(EnvironmentError::Invalid) => Err(Error::ProxyConfiguration),
            Err(EnvironmentError::Unsupported) => Err(Error::UnsupportedProxy),
        }
    }
}

impl ConnectionOptions {
    pub fn set_proxy_credentials(&mut self, credentials: ProxyCredentials) -> Result<(), Error> {
        if !matches!(&self.proxy, ProxyConfiguration::Http { username: Some(u), .. } if u == &credentials.username)
        {
            return Err(Error::ProxyConfiguration);
        }
        let ProxyConfiguration::Http { endpoint, .. } = &self.proxy else {
            return Err(Error::ProxyConfiguration);
        };
        self.proxy_credentials = Some((endpoint.clone(), Arc::new(credentials)));
        Ok(())
    }
    pub fn connection_route(&self, endpoint: &Endpoint) -> Result<String, Error> {
        Ok(self.proxy_route(endpoint)?.description())
    }
    pub(crate) fn proxy_route(&self, endpoint: &Endpoint) -> Result<Route, Error> {
        if !self.proxy.valid() {
            return Err(Error::ProxyConfiguration);
        }
        match &self.proxy {
            ProxyConfiguration::Direct => Ok(Route::Direct),
            ProxyConfiguration::Environment => self.proxy_environment.route(endpoint),
            ProxyConfiguration::Http { endpoint, username } => {
                let authorization = username
                    .as_ref()
                    .map(|u| {
                        self.proxy_credentials
                            .as_ref()
                            .filter(|(bound, c)| bound == endpoint && c.username == *u)
                            .map(|(_, c)| Arc::new(c.authorization.clone()))
                            .ok_or(Error::ProxyAuthenticationRequired)
                    })
                    .transpose()?;
                Ok(Route::Http {
                    endpoint: endpoint.clone(),
                    authorization,
                })
            }
        }
    }
    pub(crate) fn proxy_authority(&self) -> Vec<u8> {
        let mut value = serde_json::to_vec(&self.proxy).expect("proxy configuration serializes");
        if self.proxy == ProxyConfiguration::Environment {
            value.extend(self.proxy_environment.digest);
        }
        if let Some((_, credentials)) = &self.proxy_credentials {
            value.extend(Sha256::digest(credentials.authorization.as_bytes()));
        }
        value
    }
    pub(crate) fn apply_proxy(
        &self,
        builder: reqwest::ClientBuilder,
    ) -> Result<reqwest::ClientBuilder, Error> {
        self.proxy_route(&self.endpoint)?.apply(builder)
    }
    pub(crate) async fn endpoint_socket(&self, endpoint: &Endpoint) -> Result<TcpStream, Error> {
        let url = Url::parse(endpoint.as_str()).map_err(|_| Error::InvalidEndpoint)?;
        let host = host_name(&url)?;
        let port = url.port_or_known_default().ok_or(Error::InvalidEndpoint)?;
        let route = self.proxy_route(endpoint)?;
        tokio::time::timeout(Duration::from_secs(10), async {
            let (dial_host, dial_port) = match &route {
                Route::Direct => (host.clone(), port),
                Route::Http { endpoint, .. } => (
                    host_name(&endpoint.0)?,
                    endpoint
                        .0
                        .port_or_known_default()
                        .ok_or(Error::ProxyConfiguration)?,
                ),
            };
            let mut socket = TcpStream::connect((dial_host.as_str(), dial_port))
                .await
                .map_err(|_| {
                    if matches!(route, Route::Direct) {
                        Error::Transport
                    } else {
                        Error::ProxyConnect
                    }
                })?;
            socket.set_nodelay(true).map_err(|_| Error::Transport)?;
            if let Route::Http { authorization, .. } = route {
                let authority = match url.host().ok_or(Error::InvalidEndpoint)? {
                    url::Host::Ipv6(ip) => format!("[{ip}]:{port}"),
                    _ => format!("{host}:{port}"),
                };
                let auth = Zeroizing::new(
                    authorization
                        .map(|a| format!("Proxy-Authorization: {}\r\n", **a))
                        .unwrap_or_default(),
                );
                let request = Zeroizing::new(format!(
                    "CONNECT {authority} HTTP/1.1\r\nHost: {authority}\r\n{}\r\n",
                    *auth
                ));
                socket
                    .write_all(request.as_bytes())
                    .await
                    .map_err(|_| Error::ProxyConnect)?;
                let mut header = Vec::new();
                while !header.ends_with(b"\r\n\r\n") {
                    if header.len() >= 16 * 1024 {
                        return Err(Error::ProxyProtocol);
                    }
                    header.push(socket.read_u8().await.map_err(|_| Error::ProxyConnect)?);
                }
                let line = std::str::from_utf8(&header)
                    .map_err(|_| Error::ProxyProtocol)?
                    .split("\r\n")
                    .next()
                    .ok_or(Error::ProxyProtocol)?;
                let mut fields = line.split(' ');
                if !matches!(fields.next(), Some("HTTP/1.0" | "HTTP/1.1")) {
                    return Err(Error::ProxyProtocol);
                }
                let status = fields
                    .next()
                    .and_then(|s| s.parse::<u16>().ok())
                    .ok_or(Error::ProxyProtocol)?;
                if !(200..300).contains(&status) {
                    return Err(Error::ProxyRefused(status));
                }
            }
            Ok(socket)
        })
        .await
        .map_err(|_| Error::Timeout)?
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn captured_environment_applies_bypass_and_refuses_unsupported_routes() {
        let environment = Environment::from_values(
            "http://user:private-proxy-password@proxy.example:3128",
            "",
            "",
            "localhost, 10.0.0.0/8, ::1, .direct.example",
            false,
        );
        for host in ["localhost", "10.1.2.3", "[::1]", "host.direct.example"] {
            assert!(
                matches!(
                    environment.route(&Endpoint::parse(host).unwrap()).unwrap(),
                    Route::Direct
                ),
                "{host}"
            );
        }
        let route = environment
            .route(&Endpoint::parse("remote.example").unwrap())
            .unwrap();
        assert!(matches!(
            route,
            Route::Http {
                authorization: Some(_),
                ..
            }
        ));
        assert!(!format!("{environment:?} {route:?}").contains("private-proxy-password"));
        for proxy in [
            "https://proxy.example",
            "socks5://proxy.example",
            "file:///proxy.pac",
        ] {
            let environment = Environment::from_values(proxy, "", "", "local.example", false);
            assert!(matches!(
                environment.route(&Endpoint::parse("remote.example").unwrap()),
                Err(Error::UnsupportedProxy)
            ));
            assert!(matches!(
                environment.route(&Endpoint::parse("local.example").unwrap()),
                Ok(Route::Direct)
            ));
        }
        let malformed = Environment::from_values("http://[", "", "", "", false);
        assert!(matches!(
            malformed.route(&Endpoint::parse("remote.example").unwrap()),
            Err(Error::ProxyConfiguration)
        ));
    }
    #[test]
    fn proxy_policy_and_credentials_cannot_borrow_another_owner_authority() {
        let mut options = ConnectionOptions::new(Endpoint::parse("host.example").unwrap());
        options.proxy = ProxyConfiguration::Direct;
        let credentials = crate::Credentials::new("host-user".into(), "host-password".into());
        let direct = options.authority_key(&credentials, b"test-salt");
        options.proxy = ProxyConfiguration::Http {
            endpoint: ProxyEndpoint::parse("http://proxy.example:3128").unwrap(),
            username: Some("proxy-user".into()),
        };
        assert!(matches!(
            options.connection_route(&options.endpoint),
            Err(Error::ProxyAuthenticationRequired)
        ));
        options
            .set_proxy_credentials(
                ProxyCredentials::new("proxy-user".into(), "one".into()).unwrap(),
            )
            .unwrap();
        let first = options.authority_key(&credentials, b"test-salt");
        assert_ne!(first, direct);
        options
            .set_proxy_credentials(
                ProxyCredentials::new("proxy-user".into(), "two".into()).unwrap(),
            )
            .unwrap();
        assert_ne!(first, options.authority_key(&credentials, b"test-salt"));
        let redirected = options.for_endpoint(Endpoint::parse("member.example").unwrap());
        assert_eq!(
            options.connection_route(&options.endpoint).unwrap(),
            redirected.connection_route(&redirected.endpoint).unwrap()
        );
        assert!(ProxyEndpoint::parse("http://user:secret@proxy.example").is_err());
        assert!(ProxyEndpoint::parse("http://proxy.example/a").is_err());
        options.proxy = ProxyConfiguration::Http {
            endpoint: ProxyEndpoint::parse("http://different-proxy.example:3128").unwrap(),
            username: Some("proxy-user".into()),
        };
        assert!(matches!(
            options.connection_route(&options.endpoint),
            Err(Error::ProxyAuthenticationRequired)
        ));
    }
}
