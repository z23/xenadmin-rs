use serde_json::{Value, json};
use sha2::{Digest, Sha256};
use std::{sync::Arc, time::Duration};
use tokio::{
    io::{AsyncReadExt, AsyncWriteExt},
    net::TcpListener,
};
use xenadmin_core::{Backend, ConnectionOptions, Credentials, Endpoint};

async fn attempt(pin: Option<bool>, insecure: bool) -> (String, bool) {
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
    options.accept_invalid_certificates = insecure;
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
    let (error, reached_api) = attempt(Some(true), false).await;
    assert!(reached_api);
    assert!(error.contains("SESSION_AUTHENTICATION_FAILED"));
}

#[tokio::test]
async fn wrong_pin_and_untrusted_self_signed_certificate_withhold_credentials() {
    for pin in [Some(false), None] {
        let (error, reached_api) = attempt(pin, false).await;
        assert!(!reached_api);
        assert!(error.contains("TLS verification failed"));
    }
}

#[tokio::test]
async fn explicit_opt_out_permits_untrusted_certificates_and_overrides_pins() {
    for pin in [None, Some(false)] {
        let (error, reached_api) = attempt(pin, true).await;
        assert!(reached_api);
        assert!(error.contains("SESSION_AUTHENTICATION_FAILED"));
    }
}

async fn untrusted_server() -> (TcpListener, tokio_rustls::TlsAcceptor, String) {
    let _ = rustls::crypto::ring::default_provider().install_default();
    let rcgen::CertifiedKey { cert, key_pair } =
        rcgen::generate_simple_self_signed(vec!["different-host.example".into()]).unwrap();
    let config = rustls::ServerConfig::builder()
        .with_no_client_auth()
        .with_single_cert(
            vec![cert.der().clone()],
            rustls::pki_types::PrivatePkcs8KeyDer::from(key_pair.serialize_der()).into(),
        )
        .unwrap();
    let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
    let endpoint = format!("https://{}", listener.local_addr().unwrap());
    (
        listener,
        tokio_rustls::TlsAcceptor::from(Arc::new(config)),
        endpoint,
    )
}

async fn request(stream: &mut (impl tokio::io::AsyncRead + Unpin)) -> (String, Value) {
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        header.push(stream.read_u8().await.unwrap());
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
        .unwrap_or(0);
    assert!(length < 16384);
    let mut body = vec![0; length];
    stream.read_exact(&mut body).await.unwrap();
    (
        header,
        if body.is_empty() {
            Value::Null
        } else {
            serde_json::from_slice(&body).unwrap()
        },
    )
}

async fn reply(stream: &mut (impl tokio::io::AsyncWrite + Unpin), id: &Value, result: Value) {
    let mut response = json!({"jsonrpc":"2.0", "id":id});
    response
        .as_object_mut()
        .unwrap()
        .extend(result.as_object().unwrap().clone());
    let bytes = serde_json::to_vec(&response).unwrap();
    stream
        .write_all(
            format!(
                "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                bytes.len()
            )
            .as_bytes(),
        )
        .await
        .unwrap();
    stream.write_all(&bytes).await.unwrap();
}

#[tokio::test]
async fn opt_out_survives_member_redirect_and_a_separate_console_tls_endpoint() {
    let (member, member_tls, member_endpoint) = untrusted_server().await;
    let (coordinator, coordinator_tls, coordinator_endpoint) = untrusted_server().await;
    let (console_listener, console_tls, console_endpoint) = untrusted_server().await;
    let member_task = tokio::spawn(async move {
        let (stream, _) = member.accept().await.unwrap();
        let mut stream = member_tls.accept(stream).await.unwrap();
        let (_, call) = request(&mut stream).await;
        assert_eq!(call["method"], "session.login_with_password");
        reply(
            &mut stream,
            &call["id"],
            json!({"error":{"code":1,"message":"HOST_IS_SLAVE","data":[coordinator_endpoint]}}),
        )
        .await;
    });
    let console_task = tokio::spawn(async move {
        let (stream, _) = console_listener.accept().await.unwrap();
        let mut stream = console_tls.accept(stream).await.unwrap();
        let (header, _) = request(&mut stream).await;
        assert!(header.starts_with("CONNECT /console?ref=OpaqueRef:console HTTP/1.0"));
        assert!(header.contains("Cookie: session_id=OpaqueRef:tls-fixture"));
        stream
            .write_all(b"HTTP/1.0 503 Fixture tunnel reached\r\n\r\n")
            .await
            .unwrap();
    });
    let coordinator_task = tokio::spawn(async move {
        loop {
            let (stream, _) = coordinator.accept().await.unwrap();
            let tls = coordinator_tls.clone();
            let console_endpoint = console_endpoint.clone();
            tokio::spawn(async move {
                let mut stream = tls.accept(stream).await.unwrap();
                let (_, call) = request(&mut stream).await;
                let response = match call["method"].as_str().unwrap() {
                    "session.login_with_password" => json!({"result":"OpaqueRef:tls-fixture"}),
                    "event.from" if call["params"][2] == "" => {
                        json!({"result":{"token":"initial", "events":[
                            {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":"11111111-1111-1111-1111-111111111111","name_label":"TLS fixture"}},
                            {"class":"host","operation":"add","ref":"OpaqueRef:host","snapshot":{"uuid":"22222222-2222-2222-2222-222222222222","address":"127.0.0.1"}},
                            {"class":"VM","operation":"add","ref":"OpaqueRef:vm","snapshot":{"uuid":"33333333-3333-3333-3333-333333333333","power_state":"Running"}},
                            {"class":"SR","operation":"add","ref":"OpaqueRef:sr","snapshot":{"uuid":"44444444-4444-4444-4444-444444444444","physical_size":-1,"physical_utilisation":"-1"}}
                        ]}})
                    }
                    "event.from" => {
                        tokio::time::sleep(Duration::from_secs(60)).await;
                        return;
                    }
                    "VM.get_consoles" => json!({"result":["OpaqueRef:console"]}),
                    "console.get_record" => {
                        json!({"result":{"protocol":"rfb", "VM":"OpaqueRef:vm", "location":format!("{console_endpoint}/console?ref=OpaqueRef:console")}})
                    }
                    "session.logout" => json!({"result":null}),
                    other => panic!("unexpected fixture call: {other}"),
                };
                reply(&mut stream, &call["id"], response).await;
            });
        }
    });
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let mut options = ConnectionOptions::new(Endpoint::parse(&member_endpoint).unwrap());
    assert!(!options.accept_invalid_certificates);
    options.accept_invalid_certificates = true;
    let connection = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture".into()),
    );
    let inventory = connection.wait_ready(Duration::from_secs(5)).await.unwrap();
    assert_eq!(inventory.storage[0].capacity_bytes, None);
    let observed = connection
        .latest()
        .observe_vm(inventory.guests().next().unwrap().id)
        .unwrap();
    let _console = connection.open_console(observed).unwrap();
    tokio::time::timeout(Duration::from_secs(5), console_task)
        .await
        .unwrap()
        .unwrap();
    member_task.await.unwrap();
    connection.close().await;
    coordinator_task.abort();
}
