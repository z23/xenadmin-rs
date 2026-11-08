use crate::{certificate_inspection::inspect, *};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::sync::Arc;
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};

fn certificate(name: &str) -> rcgen::CertifiedKey {
    rcgen::generate_simple_self_signed(vec![name.into()]).unwrap()
}
fn config(key: rcgen::CertifiedKey) -> tokio_rustls::TlsAcceptor {
    let _ = rustls::crypto::ring::default_provider().install_default();
    tokio_rustls::TlsAcceptor::from(Arc::new(
        rustls::ServerConfig::builder()
            .with_no_client_auth()
            .with_single_cert(
                vec![key.cert.der().clone()],
                rustls::pki_types::PrivatePkcs8KeyDer::from(key.key_pair.serialize_der()).into(),
            )
            .unwrap(),
    ))
}
async fn header(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> Vec<u8> {
    let mut bytes = Vec::new();
    while !bytes.ends_with(b"\r\n\r\n") {
        match stream.read_u8().await {
            Ok(b) => bytes.push(b),
            Err(_) => break,
        }
        assert!(bytes.len() < 16384);
    }
    bytes
}
async fn fixture(
    proxy: bool,
    certs: Vec<rcgen::CertifiedKey>,
) -> (
    Endpoint,
    ProxyEndpoint,
    tokio::task::JoinHandle<Vec<Vec<u8>>>,
) {
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let endpoint = Endpoint::parse(&if proxy {
        "https://target.example:443".into()
    } else {
        format!("https://{address}")
    })
    .unwrap();
    let proxy_endpoint = ProxyEndpoint::parse(&format!("http://{address}")).unwrap();
    let task = tokio::spawn(async move {
        let mut requests = Vec::new();
        for cert in certs {
            let (mut socket, _) = listener.accept().await.unwrap();
            if proxy {
                let request = String::from_utf8(header(&mut socket).await).unwrap();
                assert!(
                    request.starts_with("CONNECT target.example:443 HTTP/1.1\r\n"),
                    "{request}"
                );
                assert!(!request.contains("Cookie:") && !request.contains("host-password"));
                assert!(
                    request
                        .to_ascii_lowercase()
                        .contains("proxy-authorization: basic ")
                );
                socket
                    .write_all(b"HTTP/1.1 200 Connection established\r\n\r\n")
                    .await
                    .unwrap();
            }
            let Ok(mut stream) = config(cert).accept(socket).await else {
                requests.push(Vec::new());
                continue;
            };
            let mut request = header(&mut stream).await;
            if !request.is_empty() {
                let headers = String::from_utf8(request.clone()).unwrap();
                assert!(!headers.to_ascii_lowercase().contains("proxy-authorization"));
                let length = headers
                    .lines()
                    .find_map(|l| {
                        l.split_once(':')
                            .filter(|(k, _)| k.eq_ignore_ascii_case("content-length"))
                            .map(|(_, v)| v.trim().parse::<usize>().unwrap())
                    })
                    .unwrap_or(0);
                let mut body = vec![0; length];
                stream.read_exact(&mut body).await.unwrap();
                request.extend(&body);
                if let Ok(rpc) = serde_json::from_slice::<Value>(&body) {
                    let bytes=serde_json::to_vec(&json!({"jsonrpc":"2.0","id":rpc["id"],"error":{"code":1,"message":"SESSION_AUTHENTICATION_FAILED","data":[]}})).unwrap();
                    stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).as_bytes()).await.unwrap();
                    stream.write_all(&bytes).await.unwrap();
                }
            }
            requests.push(request);
        }
        requests
    });
    (endpoint, proxy_endpoint, task)
}
#[tokio::test]
async fn certificate_inspection_is_credential_free_and_ca_details_are_consistent() {
    for trusted in [false, true] {
        let cert = certificate("127.0.0.1");
        let fingerprint = format!("{:x}", Sha256::digest(cert.cert.der()));
        let pem = cert.cert.pem();
        let (endpoint, _, server) = fixture(false, vec![cert]).await;
        let mut options = ConnectionOptions::new(endpoint.clone());
        options.proxy = ProxyConfiguration::Direct;
        let dir = tempfile::tempdir().unwrap();
        if trusted {
            let path = dir.path().join("ca.pem");
            std::fs::write(&path, pem).unwrap();
            options.ca_certificate = Some(path);
        }
        let observed = inspect(&options, endpoint.clone()).await.unwrap();
        assert_eq!(observed.endpoint, endpoint);
        assert_eq!(observed.details.fingerprint.to_string(), fingerprint);
        assert_eq!(
            observed.validation,
            if trusted {
                CertificateValidation::Trusted
            } else {
                CertificateValidation::UnknownIssuer
            }
        );
        assert!(observed.details.names.contains(&"127.0.0.1".into()));
        assert!(
            server.await.unwrap()[0].is_empty(),
            "TLS inspection sent application bytes"
        );
    }
}
#[tokio::test]
async fn inspected_pin_does_not_accept_a_certificate_rotated_before_login() {
    let first = certificate("127.0.0.1");
    let second = certificate("127.0.0.1");
    let (endpoint, _, server) = fixture(false, vec![first, second]).await;
    let mut options = ConnectionOptions::new(endpoint.clone());
    options.proxy = ProxyConfiguration::Direct;
    let observed = inspect(&options, endpoint.clone()).await.unwrap();
    let mut profile = ConnectionProfile::new("Fixture".into(), endpoint, "host-user".into());
    profile.proxy = ProxyConfiguration::Direct;
    profile.accept_certificate(&observed).unwrap();
    assert!(matches!(
        crate::xapi::Session::login(
            &profile.options().unwrap(),
            &Credentials::new("host-user".into(), "host-password".into())
        )
        .await,
        Err(Error::EndpointCertificate(_))
    ));
    assert!(server.await.unwrap().iter().all(Vec::is_empty));
}
#[tokio::test]
async fn http_proxy_routes_inspection_rpc_and_console_socket_without_credential_crossing() {
    let first = certificate("target.example");
    let second = certificate("target.example");
    let second_pin = format!("{:x}", Sha256::digest(second.cert.der()));
    let third = certificate("target.example");
    let (endpoint, proxy, server) = fixture(true, vec![first, second, third]).await;
    let mut options = ConnectionOptions::new(endpoint.clone());
    options.proxy = ProxyConfiguration::Http {
        endpoint: proxy,
        username: Some("proxy-user".into()),
    };
    options
        .set_proxy_credentials(
            ProxyCredentials::new("proxy-user".into(), "proxy-password".into()).unwrap(),
        )
        .unwrap();
    let observed = inspect(&options, endpoint.clone()).await.unwrap();
    assert!(observed.route.contains("HTTP CONNECT"));
    options.certificate_fingerprint = Some(second_pin.parse().unwrap());
    assert!(
        matches!(crate::xapi::Session::login(&options,&Credentials::new("host-user".into(),"host-password".into())).await,Err(Error::Api(code)) if code=="SESSION_AUTHENTICATION_FAILED")
    );
    options.accept_invalid_certificates = true;
    let socket = options.endpoint_socket(&endpoint).await.unwrap();
    let mut stream = tokio_rustls::TlsConnector::from(Arc::new(options.tls_config().unwrap()))
        .connect(
            rustls::pki_types::ServerName::try_from("target.example").unwrap(),
            socket,
        )
        .await
        .unwrap();
    stream.write_all(b"CONNECT /console?ref=fixture HTTP/1.0\r\nHost: target.example\r\nCookie: session_id=synthetic-console-session\r\n\r\n").await.unwrap();
    drop(stream);
    let requests = server.await.unwrap();
    assert!(requests[0].is_empty());
    assert!(String::from_utf8_lossy(&requests[1]).contains("session.login_with_password"));
    assert!(String::from_utf8_lossy(&requests[2]).contains("synthetic-console-session"));
}
#[tokio::test]
async fn certificate_replacement_requires_the_observed_old_pin_and_persists_no_secrets() {
    let cert = certificate("127.0.0.1");
    let (endpoint, _, server) = fixture(false, vec![cert]).await;
    let mut options = ConnectionOptions::new(endpoint.clone());
    options.proxy = ProxyConfiguration::Direct;
    options.certificate_fingerprint = Some("aa".repeat(32).parse().unwrap());
    let observed = inspect(&options, endpoint.clone()).await.unwrap();
    server.await.unwrap();
    let mut profile =
        ConnectionProfile::new("Fixture".into(), endpoint.clone(), "host-user".into());
    profile
        .host_certificates
        .insert(endpoint.to_string(), "bb".repeat(32).parse().unwrap());
    assert!(profile.accept_certificate(&observed).is_err());
    profile
        .host_certificates
        .insert(endpoint.to_string(), "aa".repeat(32).parse().unwrap());
    profile.accept_certificate(&observed).unwrap();
    let directory = tempfile::tempdir().unwrap();
    let mut store = ProfileStore::open(directory.path().into()).unwrap();
    store.save(profile.clone()).unwrap();
    let reopened = ProfileStore::open(directory.path().into()).unwrap();
    assert_eq!(reopened.get(profile.id), Some(&profile));
    assert_eq!(
        profile.certificate_details[endpoint.as_str()],
        observed.details
    );
}
#[tokio::test]
async fn proxy_refusal_and_oversized_headers_never_fall_back_direct() {
    for status in [407, 302, 500, 999] {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let address = listener.local_addr().unwrap();
        let proxy = tokio::spawn(async move {
            let (mut stream, _) = listener.accept().await.unwrap();
            header(&mut stream).await;
            stream
                .write_all(format!("HTTP/1.1 {status} Refused\r\n\r\n").as_bytes())
                .await
                .unwrap();
        });
        let mut options =
            ConnectionOptions::new(Endpoint::parse("https://target.example").unwrap());
        options.proxy = ProxyConfiguration::Http {
            endpoint: ProxyEndpoint::parse(&format!("http://{address}")).unwrap(),
            username: None,
        };
        assert!(
            matches!(options.endpoint_socket(&options.endpoint).await,Err(Error::ProxyRefused(code)) if code==status)
        );
        proxy.await.unwrap();
    }
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let address = listener.local_addr().unwrap();
    let proxy = tokio::spawn(async move {
        let (mut stream, _) = listener.accept().await.unwrap();
        header(&mut stream).await;
        let _ = stream
            .write_all(
                format!("HTTP/1.1 200 OK\r\nX-Large: {}\r\n\r\n", "a".repeat(17000)).as_bytes(),
            )
            .await;
    });
    let mut options = ConnectionOptions::new(Endpoint::parse("target.example").unwrap());
    options.proxy = ProxyConfiguration::Http {
        endpoint: ProxyEndpoint::parse(&format!("http://{address}")).unwrap(),
        username: None,
    };
    assert!(matches!(
        options.endpoint_socket(&options.endpoint).await,
        Err(Error::ProxyProtocol)
    ));
    proxy.await.unwrap();
}
