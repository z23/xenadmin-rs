use super::*;
use crate::{Backend, Connection, ConnectionOptions, Credentials, Endpoint};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    sync::{
        Mutex,
        atomic::{AtomicBool, AtomicUsize, Ordering},
    },
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
};
const POOL: &str = "11111111-1111-1111-1111-111111111111";
const HOST: &str = "22222222-2222-2222-2222-222222222222";
const HOST2: &str = "44444444-4444-4444-4444-444444444444";
const VM: &str = "33333333-3333-3333-3333-333333333333";
fn target() -> MetricTarget {
    MetricTarget::Vm(VmId::parse(VM).unwrap())
}
fn data(target: MetricTarget) -> Value {
    json!({"meta":{"start":1000,"end":1005,"step":5,"rows":2,"columns":3,
        "legend":[format!("AVERAGE:{}:{}:cpu0",target.class().to_ascii_lowercase(),target.uuid()),format!("AVERAGE:vm:{VM}:memory"),"AVERAGE:vm:55555555-5555-5555-5555-555555555555:cpu0"]},
        "data":[{"t":1005,"values":["NaN","2048","99"]},{"t":1000,"values":["0.5","1024","99"]}]})
}
fn metadata() -> Value {
    json!([
        {"name_label":"cpu0","name_description":"CPU usage","units":"fraction","enabled":true},
        {"name_label":"memory","name_description":"Memory usage","units":"bytes","enabled":true},
        {"name_label":"vif_0_rx","name_description":"Network receive","units":"bytes/s","enabled":false},
        {"name_label":"vbd_xvda_read","name_description":"Disk read","units":"bytes/s","enabled":true}
    ])
}
#[test]
fn rrd_json_projection_preserves_units_gaps_order_and_target_scope() {
    let sources = decode::sources(&metadata()).unwrap();
    let selected = vec!["cpu0".into(), "memory".into()];
    let batch = decode::decode(
        &serde_json::to_vec(&data(target())).unwrap(),
        target(),
        &sources,
        &selected,
    )
    .unwrap();
    assert_eq!(
        batch.series[0].samples,
        vec![
            MetricSample {
                timestamp: 1000,
                value: Some(0.5)
            },
            MetricSample {
                timestamp: 1005,
                value: None
            }
        ]
    );
    assert_eq!(batch.series[1].samples[1].value, Some(2048.0));
    assert_eq!(
        sources.iter().find(|s| s.name == "cpu0").unwrap().units,
        "fraction"
    );
}
#[test]
fn disabled_missing_and_nonfinite_values_are_unknown_not_zero() {
    let mut sources = decode::sources(&metadata()).unwrap();
    let selected = decode::select(
        &mut sources,
        &["vif_0_rx".into(), "missing".into(), "cpu0".into()],
    );
    for invalid in [
        Value::Null,
        json!("Infinity"),
        json!("-Inf"),
        json!("1e999"),
    ] {
        let mut value = data(target());
        value["data"][0]["values"][0] = invalid;
        let batch = decode::decode(
            &serde_json::to_vec(&value).unwrap(),
            target(),
            &sources,
            &selected,
        )
        .unwrap();
        assert!(batch.series[0].samples.iter().all(|p| p.value.is_none()));
        assert!(batch.series[1].samples.iter().all(|p| p.value.is_none()));
        assert!(batch.series[2].samples[1].value.is_none());
    }
}
#[test]
fn malformed_dimensions_duplicate_times_and_oversized_inputs_fail_atomically() {
    let sources = decode::sources(&metadata()).unwrap();
    let selected = vec!["cpu0".into()];
    let base = data(target());
    let mut malformed = Vec::new();
    let mut value = base.clone();
    value["meta"]["rows"] = json!(3);
    malformed.push(value);
    let mut value = base.clone();
    value["data"][0]["values"] = json!([1]);
    malformed.push(value);
    let mut value = base.clone();
    value["data"][0]["t"] = json!(1000);
    malformed.push(value);
    let mut value = base.clone();
    value["data"][0]["values"][0] = json!("not a number");
    malformed.push(value);
    let mut value = base.clone();
    value["meta"]["step"] = json!(0);
    malformed.push(value);
    let mut value = base.clone();
    value["meta"]["legend"] = json!(vec!["metric"; 2049]);
    malformed.push(value);
    for value in malformed {
        assert!(
            decode::decode(
                &serde_json::to_vec(&value).unwrap(),
                target(),
                &sources,
                &selected
            )
            .is_err()
        );
    }
    assert!(decode::sources(&json!(vec![metadata()[0].clone(); 513])).is_err());
    assert!(
        MetricsOptions {
            window: MetricsWindow::TenMinutes,
            sources: vec!["cpu0".into(); 33]
        }
        .validate()
        .is_err()
    );
    assert!(
        MetricsOptions {
            window: MetricsWindow::TenMinutes,
            sources: vec!["cpu0&session_id=secret".into()]
        }
        .validate()
        .is_err()
    );
}
#[test]
fn merging_windows_retains_real_boundaries_without_inventing_missing_rows() {
    let observed = ObservedMetrics {
        pool: PoolId::parse(POOL).unwrap(),
        generation: 1,
        target: target(),
    };
    let runtime = tokio::runtime::Runtime::new().unwrap();
    let (handle, _) = channel(
        observed,
        MetricsOptions::default(),
        runtime.handle().clone(),
    );
    let mut snapshot = (*handle.latest()).clone();
    let sources = decode::sources(&metadata()).unwrap();
    let selected = vec!["cpu0".into()];
    decode::merge(
        &mut snapshot,
        decode::decode(
            &serde_json::to_vec(&data(target())).unwrap(),
            target(),
            &sources,
            &selected,
        )
        .unwrap(),
        MetricsWindow::TenMinutes,
        true,
    );
    let mut next = data(target());
    next["meta"]["start"] = json!(1015);
    next["meta"]["end"] = json!(1020);
    next["data"][0]["t"] = json!(1020);
    next["data"][1]["t"] = json!(1015);
    decode::merge(
        &mut snapshot,
        decode::decode(
            &serde_json::to_vec(&next).unwrap(),
            target(),
            &sources,
            &selected,
        )
        .unwrap(),
        MetricsWindow::TenMinutes,
        false,
    );
    assert_eq!(snapshot.range_start, Some(1000));
    assert_eq!(snapshot.range_end, Some(1020));
    assert_eq!(snapshot.sample_step_seconds, Some(5));
    assert_eq!(snapshot.series[0].samples.len(), 4);
    assert!(
        !snapshot.series[0]
            .samples
            .iter()
            .any(|p| p.timestamp == 1010)
    );
}

#[derive(Clone, Copy, PartialEq)]
enum Mode {
    Normal,
    Unauthorized,
    Forbidden,
    Oversized,
    Malformed,
    UntrustedRedirect,
    TrustedRedirect,
    Slow,
    Migrate,
    Expire,
}
type HttpRequests = Arc<Mutex<Vec<(String, BTreeMap<String, String>)>>>;
struct Fixture {
    options: ConnectionOptions,
    task: tokio::task::JoinHandle<()>,
    http: HttpRequests,
    methods: Arc<Mutex<Vec<String>>>,
    resident: Arc<AtomicUsize>,
    gets: Arc<AtomicUsize>,
    mode: Arc<Mutex<Mode>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.task.abort();
    }
}
impl Fixture {
    async fn new(mode: Mode) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let port = listener.local_addr().unwrap().port();
        let member = TcpListener::bind(("127.0.0.2", port)).await.unwrap();
        let options =
            ConnectionOptions::new(Endpoint::parse(&format!("http://127.0.0.1:{port}")).unwrap());
        let http = Arc::new(Mutex::new(Vec::new()));
        let methods = Arc::new(Mutex::new(Vec::new()));
        let resident = Arc::new(AtomicUsize::new(1));
        let gets = Arc::new(AtomicUsize::new(0));
        let mode = Arc::new(Mutex::new(mode));
        let expired = Arc::new(AtomicBool::new(false));
        let task = tokio::spawn({
            let http = http.clone();
            let methods = methods.clone();
            let resident = resident.clone();
            let gets = gets.clone();
            let mode = mode.clone();
            async move {
                loop {
                    let (stream, _) = tokio::select! {result=listener.accept()=>result.unwrap(),result=member.accept()=>result.unwrap()};
                    let (http, methods, resident, gets, mode, expired) = (
                        http.clone(),
                        methods.clone(),
                        resident.clone(),
                        gets.clone(),
                        mode.clone(),
                        expired.clone(),
                    );
                    tokio::spawn(async move {
                        let mut reader = BufReader::new(stream);
                        let mut line = String::new();
                        reader.read_line(&mut line).await.unwrap();
                        let parts = line
                            .split_whitespace()
                            .map(str::to_owned)
                            .collect::<Vec<_>>();
                        let mut headers = BTreeMap::new();
                        loop {
                            line.clear();
                            if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                                return;
                            }
                            if line == "\r\n" {
                                break;
                            }
                            if let Some((name, value)) = line.split_once(':') {
                                headers.insert(name.to_ascii_lowercase(), value.trim().to_owned());
                            }
                        }
                        let mut status = 200;
                        let mut extra = String::new();
                        let body = if parts[0] == "GET" {
                            if !parts[1].starts_with("/rrd_updates?") {
                                return;
                            }
                            gets.fetch_add(1, Ordering::SeqCst);
                            http.lock()
                                .unwrap()
                                .push((parts[1].clone(), headers.clone()));
                            let active = *mode.lock().unwrap();
                            if active == Mode::Slow {
                                tokio::time::sleep(Duration::from_secs(2)).await;
                            }
                            if active == Mode::Migrate {
                                resident.store(2, Ordering::SeqCst);
                            }
                            if active == Mode::Expire {
                                expired.store(true, Ordering::SeqCst);
                            }
                            let path =
                                url::Url::parse(&format!("http://fixture{}", parts[1])).unwrap();
                            let query = path.query_pairs().collect::<BTreeMap<_, _>>();
                            let selected = if query["host"] == "true" {
                                MetricTarget::Host(
                                    HostId::parse(if headers["host"].starts_with("127.0.0.2:") {
                                        HOST2
                                    } else {
                                        HOST
                                    })
                                    .unwrap(),
                                )
                            } else {
                                MetricTarget::Vm(VmId::parse(&query["vm_uuid"]).unwrap())
                            };
                            match active {
                                Mode::Unauthorized => {
                                    status = 401;
                                    b"private-auth-body".to_vec()
                                }
                                Mode::Forbidden => {
                                    status = 403;
                                    b"private-denial-body".to_vec()
                                }
                                Mode::Oversized => {
                                    extra = "Content-Length: 9000000\r\n".into();
                                    Vec::new()
                                }
                                Mode::Malformed => b"not-json OpaqueRef:private-session".to_vec(),
                                Mode::UntrustedRedirect => {
                                    status = 302;
                                    extra = format!(
                                        "Location: http://127.0.0.3:{port}/rrd_updates?session_id=evil\r\n"
                                    );
                                    Vec::new()
                                }
                                Mode::TrustedRedirect
                                    if headers["host"].starts_with("127.0.0.1:") =>
                                {
                                    status = 302;
                                    extra = format!(
                                        "Location: http://127.0.0.2:{port}/rrd_updates?uuid=wrong&session_id=evil\r\n"
                                    );
                                    Vec::new()
                                }
                                _ => serde_json::to_vec(&data(selected)).unwrap(),
                            }
                        } else {
                            let length: usize =
                                headers.get("content-length").unwrap().parse().unwrap();
                            assert!(length < 16384);
                            let mut bytes = vec![0; length];
                            reader.read_exact(&mut bytes).await.unwrap();
                            let request: Value = serde_json::from_slice(&bytes).unwrap();
                            let method = request["method"].as_str().unwrap();
                            methods.lock().unwrap().push(method.into());
                            let host = |second: bool| json!({"uuid":if second{HOST2}else{HOST},"name_label":"Fixture host","address":if second{"127.0.0.2"}else{"127.0.0.1"},"enabled":true});
                            let vm = || json!({"uuid":VM,"name_label":"Fixture VM","power_state":"Running","resident_on":if resident.load(Ordering::SeqCst)==1{"OpaqueRef:host"}else{"OpaqueRef:host2"},"VCPUs_at_startup":"2","memory_static_max":"2048"});
                            let result = match method {
                                "session.login_with_password" => json!("OpaqueRef:fixture-session"),
                                "session.logout" => Value::Null,
                                "event.from" => {
                                    if request["params"][2] != "" {
                                        tokio::time::sleep(Duration::from_millis(50)).await;
                                    }
                                    if expired.swap(false, Ordering::SeqCst) {
                                        let response = json!({"jsonrpc":"2.0","id":request["id"],"error":{"message":"SESSION_INVALID","data":[]}});
                                        let bytes = serde_json::to_vec(&response).unwrap();
                                        let mut stream = reader.into_inner();
                                        let _=stream.write_all(format!("HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",bytes.len()).as_bytes()).await;
                                        let _ = stream.write_all(&bytes).await;
                                        return;
                                    }
                                    if request["params"][2] == "" {
                                        json!({"token":"current","events":[
                                    {"class":"pool","operation":"add","ref":"OpaqueRef:pool","snapshot":{"uuid":POOL,"name_label":"Fixture pool","master":"OpaqueRef:host"}},
                                    {"class":"host","operation":"add","ref":"OpaqueRef:host","snapshot":host(false)},
                                    {"class":"host","operation":"add","ref":"OpaqueRef:host2","snapshot":host(true)},
                                    {"class":"VM","operation":"add","ref":"OpaqueRef:vm","snapshot":vm()}]})
                                    } else {
                                        json!({"token":"current","events":[]})
                                    }
                                }
                                "VM.get_by_uuid" => json!("OpaqueRef:vm"),
                                "VM.get_record" => vm(),
                                "host.get_by_uuid" => json!(if request["params"][1] == HOST2 {
                                    "OpaqueRef:host2"
                                } else {
                                    "OpaqueRef:host"
                                }),
                                "host.get_record" => {
                                    host(request["params"][1] == "OpaqueRef:host2")
                                }
                                "VM.get_data_sources" | "host.get_data_sources" => metadata(),
                                other => panic!("Unexpected metric RPC {other}"),
                            };
                            serde_json::to_vec(
                                &json!({"jsonrpc":"2.0","id":request["id"],"result":result}),
                            )
                            .unwrap()
                        };
                        if !extra.contains("Content-Length:") {
                            extra.push_str(&format!("Content-Length: {}\r\n", body.len()));
                        }
                        let mut stream = reader.into_inner();
                        let _ = stream
                            .write_all(
                                format!(
                                    "HTTP/1.1 {status} Fixture\r\n{extra}Connection: close\r\n\r\n"
                                )
                                .as_bytes(),
                            )
                            .await;
                        let _ = stream.write_all(&body).await;
                    });
                }
            }
        });
        Self {
            options,
            task,
            http,
            methods,
            resident,
            gets,
            mode,
        }
    }
    async fn connect(&self, backend: &Backend) -> Connection {
        let mut options = self.options.clone();
        options.read_only = true;
        let connection = backend.connect(
            options,
            Credentials::new("fixture-user".into(), "fixture-password".into()),
        );
        connection.wait_ready(Duration::from_secs(3)).await.unwrap();
        connection
    }
}
async fn settled(metrics: &Metrics) -> Arc<MetricsSnapshot> {
    tokio::time::timeout(Duration::from_secs(4), async {
        loop {
            let snapshot = metrics.latest();
            if snapshot.state != MetricsState::Loading {
                return snapshot;
            }
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap()
}
fn subscribe(connection: &Connection, target: MetricTarget) -> Metrics {
    connection
        .subscribe_metrics(
            connection.latest().observe_metrics(target).unwrap(),
            MetricsOptions {
                window: MetricsWindow::TenMinutes,
                sources: vec!["cpu0".into(), "memory".into()],
            },
        )
        .unwrap()
}
#[tokio::test]
async fn read_only_subscriptions_reuse_session_scope_queries_and_do_not_touch_the_journal() {
    let fixture = Fixture::new(Mode::Normal).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = fixture.connect(&backend).await;
    for target in [target(), MetricTarget::Host(HostId::parse(HOST).unwrap())] {
        let metrics = subscribe(&connection, target);
        let snapshot = settled(&metrics).await;
        assert_eq!(snapshot.state, MetricsState::Ready);
        assert_eq!(snapshot.target, target);
        assert_eq!(snapshot.series[0].samples[0].value, Some(0.5));
        assert_eq!(snapshot.sample_step_seconds, Some(5));
        let (path, headers) = fixture.http.lock().unwrap().last().unwrap().clone();
        let url = url::Url::parse(&format!("http://fixture{path}")).unwrap();
        let query = url.query_pairs().collect::<BTreeMap<_, _>>();
        assert_eq!(
            query["vm_uuid"],
            match target {
                MetricTarget::Vm(id) => id.to_string(),
                MetricTarget::Host(_) => "none".into(),
            }
        );
        assert!(!query.contains_key("uuid"));
        assert_eq!(query["start"], "-600");
        assert_eq!(query["cf"], "AVERAGE");
        assert_eq!(query["json"], "true");
        assert_eq!(headers["accept"], "application/json");
        assert!(
            !serde_json::to_string(&*snapshot)
                .unwrap()
                .contains("OpaqueRef:")
        );
        drop(metrics);
    }
    assert!(!directory.path().join("operations.sqlite3").exists());
    assert_eq!(
        fixture
            .methods
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == "session.login_with_password")
            .count(),
        1
    );
    connection.close().await;
}
#[tokio::test]
async fn unsafe_redirects_authentication_errors_oversize_and_malformed_data_are_sanitized() {
    for mode in [
        Mode::Unauthorized,
        Mode::Forbidden,
        Mode::Oversized,
        Mode::Malformed,
        Mode::UntrustedRedirect,
    ] {
        let fixture = Fixture::new(mode).await;
        let directory = tempfile::tempdir().unwrap();
        let backend = Backend::with_state_directory(directory.path().into()).unwrap();
        let connection = fixture.connect(&backend).await;
        let metrics = subscribe(&connection, target());
        let snapshot = settled(&metrics).await;
        assert!(matches!(
            snapshot.state,
            MetricsState::Failed | MetricsState::Stale
        ));
        assert!(snapshot.series.is_empty());
        let encoded = serde_json::to_string(&*snapshot).unwrap();
        for forbidden in [
            "OpaqueRef:",
            "fixture-password",
            "private-auth-body",
            "private-denial-body",
            "127.0.0.3",
            "evil",
        ] {
            assert!(!encoded.contains(forbidden));
        }
        assert_eq!(fixture.gets.load(Ordering::SeqCst), 1);
        connection.close().await;
    }
}
#[tokio::test]
async fn known_member_redirect_rebuilds_only_the_original_target_query() {
    let fixture = Fixture::new(Mode::TrustedRedirect).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = fixture.connect(&backend).await;
    let metrics = subscribe(&connection, target());
    assert_eq!(settled(&metrics).await.state, MetricsState::Ready);
    let requests = fixture.http.lock().unwrap().clone();
    assert_eq!(requests.len(), 2);
    assert!(requests[1].1["host"].starts_with("127.0.0.2:"));
    assert!(!requests[1].0.contains("evil") && !requests[1].0.contains("wrong"));
    connection.close().await;
}
#[tokio::test]
async fn dropping_a_subscription_cancels_its_pending_fetch_and_stops_polling() {
    let fixture = Fixture::new(Mode::Slow).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = fixture.connect(&backend).await;
    let metrics = subscribe(&connection, target());
    tokio::time::timeout(Duration::from_secs(2), async {
        while fixture.gets.load(Ordering::SeqCst) == 0 {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    drop(metrics);
    tokio::time::sleep(Duration::from_millis(30)).await;
    *fixture.mode.lock().unwrap() = Mode::Normal;
    let replacement = subscribe(&connection, target());
    assert_eq!(settled(&replacement).await.state, MetricsState::Ready);
    let count = fixture.gets.load(Ordering::SeqCst);
    drop(replacement);
    tokio::time::sleep(Duration::from_millis(5100)).await;
    assert_eq!(fixture.gets.load(Ordering::SeqCst), count);
    connection.close().await;
}
#[tokio::test]
async fn migration_during_fetch_is_not_published_under_the_previous_resident_host() {
    let fixture = Fixture::new(Mode::Migrate).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = fixture.connect(&backend).await;
    let metrics = subscribe(&connection, target());
    let snapshot = settled(&metrics).await;
    assert_eq!(snapshot.state, MetricsState::Stale);
    assert!(snapshot.series.is_empty());
    assert_eq!(fixture.resident.load(Ordering::SeqCst), 2);
    *fixture.mode.lock().unwrap() = Mode::Normal;
    tokio::time::timeout(Duration::from_secs(7), async {
        while metrics.latest().state != MetricsState::Ready {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    })
    .await
    .unwrap();
    assert!(fixture.http.lock().unwrap().last().unwrap().1["host"].starts_with("127.0.0.2:"));
    connection.close().await;
}
#[tokio::test]
async fn generation_change_stales_old_metrics_and_rejects_old_observations() {
    let fixture = Fixture::new(Mode::Expire).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = fixture.connect(&backend).await;
    let observed = connection.latest().observe_metrics(target()).unwrap();
    let metrics = subscribe(&connection, target());
    tokio::time::timeout(Duration::from_secs(3), async {
        while metrics.latest().state != MetricsState::Stale {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await
    .unwrap();
    assert!(
        connection
            .subscribe_metrics(observed, MetricsOptions::default())
            .is_err()
    );
    connection.close().await;
}

#[tokio::test]
async fn subscriptions_are_bounded_and_capacity_returns_when_views_close() {
    let fixture = Fixture::new(Mode::Normal).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = fixture.connect(&backend).await;
    let mut handles = Vec::new();
    for _ in 0..4 {
        handles.push(subscribe(&connection, target()));
    }
    let excess = subscribe(&connection, target());
    assert_eq!(settled(&excess).await.state, MetricsState::Failed);
    assert_eq!(
        excess.latest().error.as_deref(),
        Some(Error::Busy.to_string().as_str())
    );
    handles.clear();
    tokio::time::sleep(Duration::from_millis(30)).await;
    let replacement = subscribe(&connection, target());
    assert_eq!(settled(&replacement).await.state, MetricsState::Ready);
    connection.close().await;
}
#[tokio::test]
async fn redirect_authority_validation_rejects_ports_paths_userinfo_and_scheme_changes() {
    let fixture = Fixture::new(Mode::Normal).await;
    let directory = tempfile::tempdir().unwrap();
    let backend = Backend::with_state_directory(directory.path().into()).unwrap();
    let connection = fixture.connect(&backend).await;
    let session = crate::xapi::Session::login(
        &fixture.options,
        &Credentials::new("fixture-user".into(), "fixture-password".into()),
    )
    .await
    .unwrap();
    let mut current = url::Url::parse(fixture.options.endpoint.as_str()).unwrap();
    current.set_path("/rrd_updates");
    let inventory = connection.latest().inventory().unwrap().clone();
    let port = current.port().unwrap();
    for bad in [
        format!("https://127.0.0.1:{port}/rrd_updates"),
        format!("http://127.0.0.1:{}/rrd_updates", port.saturating_add(1)),
        format!("http://user:password@127.0.0.1:{port}/rrd_updates"),
        format!("http://127.0.0.1:{port}/rrd_updates#fragment"),
        format!("http://127.0.0.1:{port}/jsonrpc"),
        format!("http://127.0.0.3:{port}/rrd_updates"),
    ] {
        assert!(transport::redirect(&session, &current, &bad, &inventory).is_err());
    }
    assert!(
        transport::redirect(
            &session,
            &current,
            &format!("http://127.0.0.2:{port}/rrd_updates?ignored=true"),
            &inventory
        )
        .is_ok()
    );
    session.logout().await;
    connection.close().await;
}
