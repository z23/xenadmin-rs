use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use xenadmin_core::{Backend, ConnectionOptions, Credentials, Endpoint};

async fn attempt(pin: Option<bool>) -> (String, bool) {
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["127.0.0.1".into()]).unwrap();
    let fingerprint = format!("{:x}", Sha256::digest(cert.der()));
    let _ = rustls::crypto::ring::default_provider().install_default();
    let configuration = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()).into(),
        )
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = Endpoint::parse(&format!("https://{}", listener.local_addr().unwrap())).unwrap();
    let task = tokio::spawn(async move {
        let (stream, _) = listener.accept().await.unwrap();
        let Ok(mut stream) = tokio_rustls::TlsAcceptor::from(Arc::new(configuration))
            .accept(stream)
            .await
        else {
            return false;
        };
        let mut header = Vec::new();
        while !header.ends_with(b"\r\n\r\n") {
            match stream.read_u8().await {
                Ok(byte) => header.push(byte),
                Err(_) => return false,
            }
            assert!(header.len() < 16384);
        }
        let header = String::from_utf8(header).unwrap();
        let length: usize = header
            .lines()
            .find_map(|line| {
                line.split_once(':')
                    .filter(|(name, _)| name.eq_ignore_ascii_case("content-length"))
                    .map(|(_, value)| value.trim().parse().unwrap())
            })
            .unwrap();
        assert!(length < 16384);
        let mut body = vec![0; length];
        stream.read_exact(&mut body).await.unwrap();
        let request: Value = serde_json::from_slice(&body).unwrap();
        assert_eq!(request["method"], "session.login_with_password");
        let response=serde_json::to_vec(&json!({"jsonrpc":"2.0","id":request["id"],"error":{"code":1,"message":"SESSION_AUTHENTICATION_FAILED","data":[]}})).unwrap();
        let header = format!(
            "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
            response.len()
        );
        stream.write_all(header.as_bytes()).await.unwrap();
        stream.write_all(&response).await.unwrap();
        true
    });
    let mut options = ConnectionOptions::new(endpoint);
    options.certificate_fingerprint = pin
        .map(|correct| {
            if correct {
                fingerprint
            } else {
                "00".repeat(32)
            }
        })
        .map(|pin| pin.parse().unwrap());
    let backend = Backend::start().unwrap();
    let connection = backend.connect(
        options,
        Credentials::new("fixture".into(), "not-a-real-password".into()),
    );
    let error = connection
        .wait_ready(Duration::from_secs(5))
        .await
        .unwrap_err()
        .to_string();
    let reached_api = tokio::time::timeout(Duration::from_secs(5), task)
        .await
        .unwrap()
        .unwrap();
    (error, reached_api)
}

#[tokio::test]
async fn matching_pin_permits_a_real_tls_handshake_and_rpc() {
    let (error, reached_api) = attempt(Some(true)).await;
    assert!(reached_api);
    assert!(error.contains("SESSION_AUTHENTICATION_FAILED"));
}

#[tokio::test]
async fn wrong_pin_and_untrusted_self_signed_certificate_withhold_credentials() {
    for pin in [Some(false), None] {
        let (error, reached_api) = attempt(pin).await;
        assert!(!reached_api);
        assert!(error.contains("TLS verification failed"));
    }
}
