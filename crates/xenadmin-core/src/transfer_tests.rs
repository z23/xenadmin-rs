use super::*;
use crate::{
    ConnectionOptions, Credentials, Endpoint, OperationId, OperationState, ResourceTarget,
    operations::Journal,
    resources::{self, ObservedResource, ResourceRequest},
    xapi::Session,
};
use serde_json::{Value, json};
use std::{
    collections::BTreeMap,
    path::Path,
    sync::{Arc, Mutex},
};
use tokio::{
    io::{AsyncBufReadExt, AsyncReadExt, AsyncWriteExt, BufReader},
    net::TcpListener,
    sync::mpsc,
};
use tokio_util::sync::CancellationToken;
fn uuid(n: u128) -> uuid::Uuid {
    uuid::Uuid::from_u128(n)
}
fn xml(v: &Value) -> String {
    fn escape(s: &str) -> String {
        s.replace('&', "&amp;")
            .replace('<', "&lt;")
            .replace('>', "&gt;")
    }
    let inner = match v {
        Value::String(v) => format!("<string>{}</string>", escape(v)),
        Value::Bool(v) => format!("<boolean>{}</boolean>", u8::from(*v)),
        Value::Number(v) => format!("<i8>{v}</i8>"),
        Value::Array(a) => format!(
            "<array><data>{}</data></array>",
            a.iter().map(xml).collect::<String>()
        ),
        Value::Object(m) => format!(
            "<struct>{}</struct>",
            m.iter()
                .map(|(k, v)| format!("<member><name>{}</name>{}</member>", escape(k), xml(v)))
                .collect::<String>()
        ),
        _ => panic!("fixture XML type"),
    };
    format!("<value>{inner}</value>")
}
fn archive(vm: u128) -> Vec<u8> {
    archive_record(
        json!({"uuid":uuid(vm),"name_label":"Archive guest","is_control_domain":false,"is_a_template":false,"is_a_snapshot":false,"power_state":"Halted","other_config":{}}),
    )
}
fn archive_record(record: Value) -> Vec<u8> {
    let value = json!({"version":{},"objects":[{"class":"VM","id":"Ref:1","snapshot":record},{"class":"VDI","id":"Ref:2","snapshot":{"type":"user","virtual_size":"1073741824"}},{"class":"network","id":"Ref:3","snapshot":{}}]});
    tar_xml(xml(&value).as_bytes())
}
fn tar_xml(xml: &[u8]) -> Vec<u8> {
    let mut h = vec![0u8; 512];
    h[..8].copy_from_slice(b"ova.xml\0");
    h[124..136].copy_from_slice(format!("{:011o}\0", xml.len()).as_bytes());
    h[148..156].fill(b' ');
    h[156] = b'0';
    let sum: usize = h.iter().map(|v| usize::from(*v)).sum();
    h[148..156].copy_from_slice(format!("{sum:06o}\0 ").as_bytes());
    h.extend(xml);
    h.resize(h.len().div_ceil(512) * 512 + 1024, 0);
    h
}
#[derive(Clone, Copy, PartialEq)]
enum Fault {
    None,
    LostImport,
    RedirectImport,
    ExpireExport,
    Truncated,
    WrongArchive,
    TaskGone,
    ExistingGuest,
    Partial,
    Pending,
    Snapshot,
    Untagged,
    LostDiskCreate,
    ExistingDisk,
    WrongDiskResult,
    ChangedDisk,
    DiskFailure,
    DynamicDisk,
    PendingDisk,
    ChangedInput,
    DelayedDiskCreate,
    CloudLostAttach,
    CloudWrongAttach,
    CloudDelayedAttach,
    CloudChangedVmAfterUpload,
    CloudDelayedUpload,
}
struct Fixture {
    fault: Fault,
    options: ConnectionOptions,
    server: tokio::task::JoinHandle<()>,
    calls: Arc<Mutex<Vec<String>>>,
    records: Arc<Mutex<BTreeMap<String, Value>>>,
}
impl Drop for Fixture {
    fn drop(&mut self) {
        self.server.abort();
    }
}
impl Fixture {
    async fn new(fault: Fault) -> Self {
        let listener = TcpListener::bind("127.0.0.1:0").await.unwrap();
        let options = ConnectionOptions::new(
            Endpoint::parse(&format!("http://{}", listener.local_addr().unwrap())).unwrap(),
        );
        let records: Arc<Mutex<BTreeMap<String, Value>>> = Arc::new(Mutex::new(BTreeMap::from([
            (
                "OpaqueRef:pool:p".into(),
                json!({"uuid":uuid(1),"master":"OpaqueRef:host:h"}),
            ),
            (
                "OpaqueRef:host:h".into(),
                json!({"uuid":uuid(3),"address":"127.0.0.1","enabled":true}),
            ),
            (
                "OpaqueRef:SR:s".into(),
                json!({"uuid":uuid(4),"content_type":"user","allowed_operations":["vdi_create"],"current_operations":{},"PBDs":["OpaqueRef:PBD:p"]}),
            ),
            (
                "OpaqueRef:PBD:p".into(),
                json!({"uuid":uuid(5),"SR":"OpaqueRef:SR:s","host":"OpaqueRef:host:h","currently_attached":true}),
            ),
            (
                "OpaqueRef:VM:source".into(),
                json!({"uuid":uuid(2),"is_a_template":false,"is_a_snapshot":false,"is_control_domain":false,"power_state":"Halted","current_operations":{},"blocked_operations":{},"allowed_operations":["export"]}),
            ),
        ])));
        if fault == Fault::Snapshot {
            let mut r = records.lock().unwrap();
            let parent = r["OpaqueRef:VM:source"].clone();
            let v = r.get_mut("OpaqueRef:VM:source").unwrap();
            v["is_a_snapshot"] = json!(true);
            v["is_a_template"] = json!(true);
            v["snapshot_of"] = json!("OpaqueRef:VM:parent");
            v["parent"] = json!("OpaqueRef:VM:parent");
            v["snapshots"] = json!([]);
            v["name_label"] = json!("Disk snapshot");
            v["other_config"] = json!({});
            let mut parent = parent;
            parent["uuid"] = json!(uuid(9));
            r.insert("OpaqueRef:VM:parent".into(), parent);
        }
        let calls = Arc::new(Mutex::new(vec![]));
        let r = records.clone();
        let c = calls.clone();
        let server = tokio::spawn(async move {
            loop {
                let (stream, _) = listener.accept().await.unwrap();
                let r = r.clone();
                let c = c.clone();
                tokio::spawn(async move {
                    let mut reader = BufReader::new(stream);
                    let mut line = String::new();
                    reader.read_line(&mut line).await.unwrap();
                    let request = line.clone();
                    let mut len = 0;
                    let mut cookie = String::new();
                    loop {
                        line.clear();
                        if reader.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((k, v)) = line.split_once(':') {
                            if request.contains("_raw_vdi?") {
                                assert!(!k.eq_ignore_ascii_case("transfer-encoding"));
                            }
                            if k.eq_ignore_ascii_case("content-length") {
                                len = v.trim().parse::<usize>().unwrap();
                            }
                            if k.eq_ignore_ascii_case("cookie") {
                                cookie = v.trim().into();
                            }
                        }
                    }
                    assert!(len < 16_000_000);
                    let mut input = vec![0; len];
                    if reader.read_exact(&mut input).await.is_err() {
                        return;
                    }
                    let mut status = "200 OK";
                    let mut extra = String::new();
                    let mut declared = None;
                    let mut delay_create = false;
                    let body = if request.starts_with("GET /host_backup?")
                        || request.starts_with("GET /system-status?")
                    {
                        assert_eq!(cookie, "session_id=OpaqueRef:synthetic-secret");
                        assert!(!request.contains("synthetic-secret"));
                        let backup = request.starts_with("GET /host_backup?");
                        c.lock().unwrap().push(
                            if backup {
                                "GET /host_backup"
                            } else {
                                "GET /system-status"
                            }
                            .into(),
                        );
                        let u = url::Url::parse(&format!(
                            "http://localhost{}",
                            request.split_whitespace().nth(1).unwrap()
                        ))
                        .unwrap();
                        let q = u.query_pairs().collect::<BTreeMap<_, _>>();
                        assert_eq!(q["task_id"], "OpaqueRef:task:t");
                        if !backup {
                            assert_eq!(q["entries"], "host-crashdump-logs");
                            assert_eq!(q["output"], "tar");
                        }
                        let mut data = r.lock().unwrap();
                        let fault = data
                            .get("fixture:hostfault")
                            .and_then(Value::as_str)
                            .unwrap_or("")
                            .to_owned();
                        data.get_mut("OpaqueRef:task:t").unwrap()["status"] =
                            json!(if fault == "task-failure" {
                                "failure"
                            } else if fault == "pending" {
                                "pending"
                            } else {
                                "success"
                            });
                        if fault == "lost" {
                            return;
                        }
                        if fault == "redirect" {
                            status = "302 Found";
                            extra = "Location: http://127.0.0.1:1/host_backup\r\n".into();
                        }
                        if fault == "expired" {
                            status = "401 Unauthorized";
                        }
                        let bytes = host_support_tests::body(
                            backup,
                            if fault == "wrong-host" { 9 } else { 3 },
                        );
                        if fault == "truncated" {
                            declared = Some(bytes.len() + 100);
                        }
                        bytes
                    } else if request.starts_with("GET /export_raw_vdi?")
                        || request.starts_with("PUT /import_raw_vdi?")
                    {
                        assert_eq!(cookie, "session_id=OpaqueRef:synthetic-secret");
                        assert!(!request.contains("synthetic-secret"));
                        let put = request.starts_with("PUT");
                        let path = if put {
                            "PUT /import_raw_vdi"
                        } else {
                            "GET /export_raw_vdi"
                        };
                        c.lock().unwrap().push(path.into());
                        let url = url::Url::parse(&format!(
                            "http://localhost{}",
                            request.split_whitespace().nth(1).unwrap()
                        ))
                        .unwrap();
                        let q = url.query_pairs().collect::<BTreeMap<_, _>>();
                        let vhd = q["format"] == "vhd";
                        let bytes = if vhd && fault == Fault::DynamicDisk {
                            crate::disk_image::tests::dynamic(true)
                        } else if vhd {
                            crate::disk_image::tests::fixed()
                        } else {
                            vec![7; 4096]
                        };
                        let mut data = r.lock().unwrap();
                        let t = data.get_mut("OpaqueRef:task:t").unwrap();
                        t["status"] = json!(if fault == Fault::DiskFailure {
                            "failure"
                        } else if fault == Fault::PendingDisk {
                            "pending"
                        } else {
                            "success"
                        });
                        t["error_info"] = json!(["IMPORT_ERROR"]);
                        t["result"] = json!(xml(&json!(if fault == Fault::WrongDiskResult {
                            "OpaqueRef:VDI:source"
                        } else {
                            "OpaqueRef:VDI:new"
                        })));
                        if put {
                            delay_create = fault == Fault::CloudDelayedUpload;
                            if data.get("fixture:cloud").is_some() {
                                assert_eq!(input.len(), crate::cloud_init::MEDIA_BYTES);
                                assert_eq!(&input[43..54], b"CIDATA     ");
                                if fault == Fault::CloudChangedVmAfterUpload {
                                    data.get_mut("OpaqueRef:VM:source").unwrap()["power_state"] =
                                        json!("Running");
                                }
                            } else {
                                assert_eq!(input, bytes);
                            }
                            assert_eq!(q["vdi"], "OpaqueRef:VDI:new");
                            if fault == Fault::ChangedInput {
                                let path = data["fixture:sourcepath"].as_str().unwrap();
                                std::fs::write(path, vec![9; 4096]).unwrap();
                                return;
                            }
                            if matches!(fault, Fault::LostImport | Fault::PendingDisk) {
                                return;
                            }
                            Vec::new()
                        } else {
                            assert_eq!(q["vdi"], "OpaqueRef:VDI:source");
                            if fault == Fault::ChangedDisk {
                                data.get_mut("OpaqueRef:VDI:source").unwrap()["virtual_size"] =
                                    json!(8192);
                            }
                            bytes
                        }
                    } else if request.starts_with("GET /export?")
                        || request.starts_with("PUT /import?")
                    {
                        assert_eq!(cookie, "session_id=OpaqueRef:synthetic-secret");
                        assert!(!request.contains("synthetic-secret"));
                        c.lock().unwrap().push(
                            if request.starts_with("PUT") {
                                "PUT /import"
                            } else {
                                "GET /export"
                            }
                            .into(),
                        );
                        let mut data = r.lock().unwrap();
                        if request.starts_with("PUT") {
                            if fault == Fault::RedirectImport {
                                status = "302 Found";
                                extra = "Location: http://127.0.0.1:1/import\r\n".into();
                            } else {
                                assert_eq!(input, archive(2));
                                if fault != Fault::Partial {
                                    data.insert("OpaqueRef:VM:new".into(),json!({"uuid":uuid(if fault==Fault::ExistingGuest{2}else{20}),"name_label":"Archive guest","is_control_domain":false,"is_a_template":false,"is_a_snapshot":false,"power_state":"Halted","current_operations":{},"other_config":{"import_task":"OpaqueRef:task:t"},"VBDs":["OpaqueRef:VBD:new"]}));
                                    data.insert("OpaqueRef:VBD:new".into(),json!({"uuid":uuid(21),"VM":"OpaqueRef:VM:new","VDI":"OpaqueRef:VDI:new","type":"Disk","empty":false}));
                                }
                                data.insert("OpaqueRef:VDI:new".into(),json!({"uuid":uuid(22),"SR":"OpaqueRef:SR:s","type":"user","managed":true,"missing":false,"current_operations":{},"sm_config":{"import_task":"OpaqueRef:task:t"},"VBDs":if fault==Fault::Partial{vec![]}else{vec!["OpaqueRef:VBD:new"]}}));
                                data.insert("OpaqueRef:network:new".into(),json!({"uuid":uuid(23),"other_config":{"import_task":"OpaqueRef:task:t"}}));
                                if fault == Fault::Untagged {
                                    data.get_mut("OpaqueRef:VM:new").unwrap()["other_config"] =
                                        json!({});
                                    data.get_mut("OpaqueRef:network:new").unwrap()["other_config"] =
                                        json!({});
                                    data.get_mut("OpaqueRef:VDI:new").unwrap()["VBDs"] = json!([]);
                                }
                                let task = data.get_mut("OpaqueRef:task:t").unwrap();
                                task["status"] =
                                    json!(if matches!(fault, Fault::Partial | Fault::Untagged) {
                                        "failure"
                                    } else if fault == Fault::Pending {
                                        "pending"
                                    } else {
                                        "success"
                                    });
                                task["result"] = json!(xml(&json!(["OpaqueRef:VM:new"])));
                                task["error_info"] = json!(["IMPORT_ERROR", "private-source-path"]);
                            }
                            if fault == Fault::LostImport {
                                return;
                            }
                            Vec::new()
                        } else {
                            data.get_mut("OpaqueRef:task:t").unwrap()["status"] = json!("success");
                            if fault == Fault::ExpireExport {
                                status = "401 Unauthorized";
                                b"session_id=PRIVATE".to_vec()
                            } else {
                                let mut archive = if fault == Fault::Snapshot {
                                    // XAPI normal export omits snapshot lineage; metadata-export is a distinct endpoint.
                                    let mut v = data["OpaqueRef:VM:source"].clone();
                                    assert_eq!(v["is_a_snapshot"], true);
                                    v["is_a_snapshot"] = json!(false);
                                    v["snapshot_of"] = json!("OpaqueRef:NULL");
                                    v["parent"] = json!("OpaqueRef:NULL");
                                    v["snapshots"] = json!([]);
                                    archive_record(v)
                                } else {
                                    archive(if fault == Fault::WrongArchive { 99 } else { 2 })
                                };
                                if fault == Fault::Truncated {
                                    declared = Some(archive.len());
                                    archive.truncate(800);
                                }
                                archive
                            }
                        }
                    } else {
                        let req: Value = serde_json::from_slice(&input).unwrap();
                        let method = req["method"].as_str().unwrap();
                        c.lock().unwrap().push(method.into());
                        let p = &req["params"];
                        if method == "event.from" {
                            tokio::time::sleep(std::time::Duration::from_millis(30)).await;
                        }
                        let mut data = r.lock().unwrap();
                        let mut response = if method == "session.login_with_password" {
                            json!({"result":"OpaqueRef:synthetic-secret"})
                        } else if method == "session.logout" {
                            json!({"result":null})
                        } else if method == "event.from" {
                            json!({"result":{"token":"transfer-fixture-token","events":if p[2]=="" {data.iter().map(|(reference,snapshot)|json!({"class":reference.split(':').nth(1).unwrap().to_ascii_lowercase(),"operation":"add","ref":reference,"snapshot":snapshot})).collect::<Vec<_>>()}else{vec![]}}})
                        } else if let Some(class) = method.strip_suffix(".get_all_records") {
                            json!({"result":data.iter().filter(|(k,_)|k.starts_with(&format!("OpaqueRef:{class}:"))).collect::<BTreeMap<_,_>>()})
                        } else if let Some(class) = method.strip_suffix(".get_by_uuid") {
                            data.iter()
                                .find(|(k, v)| {
                                    k.starts_with(&format!("OpaqueRef:{class}:"))
                                        && v["uuid"] == p[1]
                                })
                                .map(|(k, _)| json!({"result":k}))
                                .unwrap_or(json!({"error":{"message":"UUID_INVALID"}}))
                        } else if method == "VM.get_allowed_VBD_devices" {
                            json!({"result":["1"]})
                        } else if method == "VBD.create" {
                            let mut v = p[1].clone();
                            v["uuid"] = json!(uuid(31));
                            v["currently_attached"] = json!(false);
                            v["current_operations"] = json!({});
                            if fault == Fault::CloudWrongAttach {
                                v["VM"] = json!("OpaqueRef:VM:wrong");
                            }
                            data.insert("OpaqueRef:VBD:cloud".into(), v);
                            data.get_mut("OpaqueRef:VM:source").unwrap()["VBDs"] =
                                json!(["OpaqueRef:VBD:cloud"]);
                            data.get_mut("OpaqueRef:VDI:new").unwrap()["VBDs"] =
                                json!(["OpaqueRef:VBD:cloud"]);
                            if fault == Fault::CloudLostAttach {
                                return;
                            }
                            delay_create = fault == Fault::CloudDelayedAttach;
                            json!({"result":"OpaqueRef:VBD:cloud"})
                        } else if method == "VDI.create" {
                            delay_create = fault == Fault::DelayedDiskCreate;
                            let mut v = disk_record(
                                if fault == Fault::ExistingDisk { 10 } else { 22 },
                                p[1]["name_label"].as_str().unwrap(),
                            );
                            v["other_config"] = p[1]["other_config"].clone();
                            v["virtual_size"] = p[1]["virtual_size"].clone();
                            data.insert("OpaqueRef:VDI:new".into(), v);
                            if fault == Fault::LostDiskCreate {
                                return;
                            }
                            json!({"result":"OpaqueRef:VDI:new"})
                        } else if method == "host.get_system_status_capabilities" {
                            json!({"result":host_support_tests::CAPS})
                        } else if method == "task.create" {
                            data.insert("OpaqueRef:task:t".into(),json!({"uuid":uuid(100),"name_label":p[1],"status":"pending","progress":0.0,"allowed_operations":["cancel"]}));
                            json!({"result":"OpaqueRef:task:t"})
                        } else if method == "task.cancel" {
                            data.get_mut("OpaqueRef:task:t").unwrap()["status"] =
                                json!("cancelled");
                            json!({"result":null})
                        } else if method == "task.destroy" {
                            data.remove("OpaqueRef:task:t");
                            json!({"result":null})
                        } else if method.ends_with(".get_record") {
                            if fault == Fault::TaskGone
                                && method == "task.get_record"
                                && c.lock()
                                    .unwrap()
                                    .iter()
                                    .any(|v| v == "PUT /import" || v == "GET /export")
                            {
                                json!({"error":{"message":"HANDLE_INVALID"}})
                            } else {
                                data.get(p[1].as_str().unwrap())
                                    .map(|v| json!({"result":v}))
                                    .unwrap_or(json!({"error":{"message":"HANDLE_INVALID"}}))
                            }
                        } else {
                            panic!("unexpected RPC {method}")
                        };
                        response["id"] = req["id"].clone();
                        response["jsonrpc"] = json!("2.0");
                        serde_json::to_vec(&response).unwrap()
                    };
                    if delay_create {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                    let header = format!(
                        "HTTP/1.1 {status}\r\nContent-Length: {}\r\n{extra}Connection: close\r\n\r\n",
                        declared.unwrap_or(body.len())
                    );
                    let mut stream = reader.into_inner();
                    let _ = stream.write_all(header.as_bytes()).await;
                    let _ = stream.write_all(&body).await;
                });
            }
        });
        Self {
            fault,
            options,
            server,
            calls,
            records,
        }
    }
    fn disk(&self) {
        self.records.lock().unwrap().insert(
            "OpaqueRef:VDI:source".into(),
            disk_record(10, "Existing disk"),
        );
    }
    async fn session(&self) -> Session {
        Session::login(
            &self.options,
            &Credentials::new("fixture".into(), "fixture".into()),
        )
        .await
        .unwrap()
    }
    fn count(&self, method: &str) -> usize {
        self.calls
            .lock()
            .unwrap()
            .iter()
            .filter(|m| m.as_str() == method)
            .count()
    }
}
async fn execute(f: &Fixture, dir: &Path, export: bool) -> crate::ResourceOperationRecord {
    let file = if export {
        TransferFile::destination(dir.join("output.xva")).unwrap()
    } else {
        std::fs::write(dir.join("input.xva"), archive(2)).unwrap();
        TransferFile::source(dir.join("input.xva")).unwrap()
    };
    let action = if export {
        TransferAction::ExportXva {
            source: if f.records.lock().unwrap()["OpaqueRef:VM:source"]["is_a_snapshot"] == true {
                ExportSource::Snapshot {
                    snapshot: uuid(2).into(),
                    parent: uuid(9).into(),
                }
            } else {
                ExportSource::Vm {
                    vm: uuid(2).into(),
                    expected_template: false,
                }
            },
        }
    } else {
        TransferAction::ImportXva { sr: uuid(4).into() }
    };
    let (observed, pending) = resources::transfer_channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Pool(uuid(1).into()),
        },
        TransferRequest { action, file },
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(512);
    tokio::time::timeout(
        std::time::Duration::from_secs(5),
        pending.run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        ),
    )
    .await
    .unwrap();
    (*observed.latest()).clone()
}
async fn recheck(
    f: &Fixture,
    dir: &Path,
    r: crate::ResourceOperationRecord,
) -> crate::ResourceOperationRecord {
    let id = r.id;
    let (tx, _rx) = mpsc::channel(512);
    ResourceRequest::tracking(r)
        .run(
            Arc::new(f.session().await),
            dir.into(),
            CancellationToken::new(),
            tx,
        )
        .await;
    Journal::open(dir)
        .unwrap()
        .resource_load(id)
        .unwrap()
        .unwrap()
}
#[tokio::test]
async fn private_export_requires_stream_and_task_then_atomically_publishes() {
    use std::os::unix::fs::PermissionsExt;
    let f = Fixture::new(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), true).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(
        std::fs::read(d.path().join("output.xva")).unwrap(),
        archive(2)
    );
    assert_eq!(
        d.path()
            .join("output.xva")
            .metadata()
            .unwrap()
            .permissions()
            .mode()
            & 0o777,
        0o600
    );
    assert_eq!(f.count("GET /export"), 1);
    assert_eq!(f.count("task.destroy"), 1);
    assert!(
        !serde_json::to_string(&r)
            .unwrap()
            .contains("synthetic-secret")
    );
}
#[tokio::test]
async fn import_reports_new_guest_disk_network_without_starting() {
    let f = Fixture::new(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), false).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(r.transfer_created.vms, vec![uuid(20).into()]);
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert_eq!(r.transfer_created.networks, vec![uuid(23).into()]);
    assert_eq!(f.count("PUT /import"), 1);
}
#[tokio::test]
async fn upload_lost_reply_uses_task_and_provenance_without_replay() {
    let f = Fixture::new(Fault::LostImport).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), false).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let _ = recheck(&f, d.path(), r).await;
    assert_eq!(f.count("PUT /import"), 1);
}
#[tokio::test]
async fn incomplete_expired_or_wrong_export_never_publishes() {
    for fault in [Fault::Truncated, Fault::ExpireExport, Fault::WrongArchive] {
        let f = Fixture::new(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), true).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert!(!d.path().join("output.xva").exists());
        assert_eq!(f.count("GET /export"), 1);
    }
}
#[tokio::test]
async fn redirecting_upload_is_never_replayed() {
    let f = Fixture::new(Fault::RedirectImport).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), false).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(f.count("PUT /import"), 1);
    assert!(r.transfer_created.vms.is_empty());
}
#[tokio::test]
async fn removed_task_and_existing_guest_do_not_claim_success() {
    for fault in [Fault::TaskGone, Fault::ExistingGuest] {
        let f = Fixture::new(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = execute(&f, d.path(), false).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        let r = recheck(&f, d.path(), r).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(f.count("PUT /import"), 1);
    }
}
#[tokio::test]
async fn failed_import_reports_proven_orphan_disk_and_network() {
    let f = Fixture::new(Fault::Partial).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), false).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert_eq!(r.transfer_created.networks, vec![uuid(23).into()]);
    assert!(r.transfer_created.vms.is_empty());
    assert!(!r.detail.contains("private-source-path"));
}
#[tokio::test]
async fn persisted_completion_survives_task_removal_and_file_rename() {
    let f = Fixture::new(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let mut r = execute(&f, d.path(), true).await;
    assert_eq!(r.state, OperationState::Succeeded);
    let original = r.id;
    r.id = OperationId::new();
    r.revision = 0;
    r.state = OperationState::OutcomeUnknown;
    let j = Journal::open(d.path()).unwrap();
    j.resource_save(&mut r).unwrap();
    let result = recheck(&f, d.path(), r).await;
    assert_eq!(result.state, OperationState::Succeeded, "{}", result.detail);
    assert_ne!(result.id, original);
    assert_eq!(f.count("GET /export"), 1);
}
#[test]
fn source_selection_rejects_unsafe_archive_and_file_changes() {
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("input.xva");
    std::fs::write(&p, archive(2)).unwrap();
    let f = TransferFile::source(&p).unwrap();
    assert_eq!(f.archive().unwrap().data_disks, 1);
    std::fs::write(&p, archive(3)).unwrap();
    assert!(files::source(&f).is_err());
    std::fs::write(
        &p,
        tar_xml(
            b"<!DOCTYPE value [<!ENTITY leak SYSTEM 'file:///etc/passwd'>]><value>&leak;</value>",
        ),
    )
    .unwrap();
    assert!(TransferFile::source(&p).is_err());
}
#[test]
fn symlinks_and_preexisting_destinations_are_not_overwritten() {
    use std::os::unix::fs::symlink;
    let d = tempfile::tempdir().unwrap();
    let p = d.path().join("input.xva");
    std::fs::write(&p, archive(2)).unwrap();
    let link = d.path().join("link.xva");
    symlink(&p, &link).unwrap();
    assert!(TransferFile::source(&link).is_err());
    assert!(TransferFile::destination(&p).is_err());
    assert!(TransferFile::destination(&link).is_err());
}

#[tokio::test]
async fn cancellation_requires_server_confirmation_and_retains_created_ids() {
    let f = Fixture::new(Fault::Pending).await;
    let d = tempfile::tempdir().unwrap();
    std::fs::write(d.path().join("input.xva"), archive(2)).unwrap();
    let (op, pending) = resources::transfer_channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Pool(uuid(1).into()),
        },
        TransferRequest {
            action: TransferAction::ImportXva { sr: uuid(4).into() },
            file: TransferFile::source(d.path().join("input.xva")).unwrap(),
        },
        tokio::runtime::Handle::current(),
    );
    let id = op.latest().id;
    let (tx, _rx) = mpsc::channel(512);
    let run = pending.run(
        Arc::new(f.session().await),
        d.path().into(),
        CancellationToken::new(),
        tx,
    );
    let cancel = async {
        for _ in 0..500 {
            if f.count("PUT /import") > 0 {
                Journal::open(d.path())
                    .unwrap()
                    .resource_control(
                        uuid(1).into(),
                        id,
                        crate::OperationControl::RequestCancellation,
                    )
                    .unwrap();
                return;
            }
            tokio::time::sleep(std::time::Duration::from_millis(10)).await;
        }
        panic!("no upload")
    };
    tokio::time::timeout(std::time::Duration::from_secs(8), async {
        tokio::join!(run, cancel);
    })
    .await
    .unwrap();
    let r = op.latest();
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert!(r.cancel_requested);
    assert!(
        Journal::open(d.path())
            .unwrap()
            .resource_pool_busy(r.pool)
            .unwrap()
    );
    // Cancellation may win the journal CAS as the stream ends. A stopped
    // tracker stays unknown; an explicit recheck forwards any unsent request
    // and discovers retained resources without uploading a second time.
    let (tx, _rx) = mpsc::channel(512);
    resources::ResourceRequest::tracking((*r).clone())
        .run(
            Arc::new(f.session().await),
            d.path().into(),
            CancellationToken::new(),
            tx,
        )
        .await;
    let r = Journal::open(d.path())
        .unwrap()
        .resource_load(id)
        .unwrap()
        .unwrap();
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert_eq!(f.count("task.cancel"), 1);
    assert_eq!(f.count("PUT /import"), 1);
    assert_eq!(r.transfer_created.vms, vec![uuid(20).into()]);
    assert!(f.records.lock().unwrap().contains_key("OpaqueRef:task:t"));
}

#[tokio::test]
async fn disk_snapshot_export_validates_parent_and_normalized_archive_identity() {
    let f = Fixture::new(Fault::Snapshot).await;
    let d = tempfile::tempdir().unwrap();
    let raw = f.records.lock().unwrap()["OpaqueRef:VM:source"].clone();
    std::fs::write(d.path().join("metadata-only.xva"), archive_record(raw)).unwrap();
    assert!(TransferFile::source(d.path().join("metadata-only.xva")).is_err());
    let r = execute(&f, d.path(), true).await;
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    let file = TransferFile::source(d.path().join("output.xva")).unwrap();
    let guest = &file.archive().unwrap().guests[0];
    assert_eq!(guest.uuid, uuid(2));
    assert!(guest.template);
    assert_eq!(f.count("GET /export"), 1);
}
#[tokio::test]
async fn failed_import_with_untagged_resources_keeps_guard_and_only_reports_proven_ids() {
    let f = Fixture::new(Fault::Untagged).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), false).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    assert!(r.transfer_created.vms.is_empty());
    assert!(r.transfer_created.networks.is_empty());
    assert_eq!(r.transfer_created.disks, vec![uuid(22).into()]);
    assert!(
        Journal::open(d.path())
            .unwrap()
            .resource_pool_busy(uuid(1).into())
            .unwrap()
    );
    assert_eq!(f.count("task.destroy"), 0);
    let r = recheck(&f, d.path(), r).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown);
    assert_eq!(f.count("PUT /import"), 1);
}
#[test]
fn gzip_metadata_is_bounded_and_supported() {
    use std::io::Write;
    let d = tempfile::tempdir().unwrap();
    let mut zip = flate2::write::GzEncoder::new(vec![], flate2::Compression::default());
    zip.write_all(&archive(2)).unwrap();
    let path = d.path().join("compressed.xva");
    std::fs::write(&path, zip.finish().unwrap()).unwrap();
    assert_eq!(
        TransferFile::source(path)
            .unwrap()
            .archive()
            .unwrap()
            .guests
            .len(),
        1
    );
}

#[tokio::test]
async fn read_only_client_exports_but_cannot_import_and_recovers_only_exports() {
    let f = Fixture::new(Fault::None).await;
    let d = tempfile::tempdir().unwrap();
    let backend = crate::Backend::with_state_directory(d.path().into()).unwrap();
    let mut options = f.options.clone();
    options.read_only = true;
    let connection = backend.connect(
        options.clone(),
        Credentials::new("fixture".into(), "fixture".into()),
    );
    connection
        .wait_ready(std::time::Duration::from_secs(3))
        .await
        .unwrap();
    assert!(!d.path().join("operations.sqlite3").exists());
    let pool = connection
        .latest()
        .observe_resource(ResourceTarget::Pool(uuid(1).into()))
        .unwrap();
    std::fs::write(d.path().join("input.xva"), archive(2)).unwrap();
    let denied = connection.transfer(
        OperationId::new(),
        pool,
        TransferRequest {
            action: TransferAction::ImportXva { sr: uuid(4).into() },
            file: TransferFile::source(d.path().join("input.xva")).unwrap(),
        },
    );
    assert!(matches!(denied, Err(Error::ReadOnly)));
    let op = connection
        .transfer(
            OperationId::new(),
            pool,
            TransferRequest {
                action: TransferAction::ExportXva {
                    source: ExportSource::Vm {
                        vm: uuid(2).into(),
                        expected_template: false,
                    },
                },
                file: TransferFile::destination(d.path().join("output.xva")).unwrap(),
            },
        )
        .unwrap();
    for _ in 0..500 {
        if op.latest().state.terminal() {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert_eq!(
        op.latest().state,
        OperationState::Succeeded,
        "{}",
        op.latest().detail
    );
    let id = op.latest().id;
    connection.close().await;
    let connection = backend.connect(
        options,
        Credentials::new("fixture".into(), "fixture".into()),
    );
    connection
        .wait_ready(std::time::Duration::from_secs(3))
        .await
        .unwrap();
    for _ in 0..100 {
        if connection.resource_history().iter().any(|r| r.id == id) {
            break;
        }
        tokio::time::sleep(std::time::Duration::from_millis(10)).await;
    }
    assert!(
        connection
            .resource_history()
            .iter()
            .any(|r| r.id == id && r.state == OperationState::Succeeded)
    );
    connection.close().await;
    assert_eq!(f.count("PUT /import"), 0);
    assert_eq!(f.count("GET /export"), 1);
}

#[tokio::test]
async fn operation_identity_includes_selected_file_and_private_receipts_cannot_regress() {
    let d = tempfile::tempdir().unwrap();
    let request = |path: &str| TransferRequest {
        action: TransferAction::ExportXva {
            source: ExportSource::Vm {
                vm: uuid(2).into(),
                expected_template: false,
            },
        },
        file: TransferFile::destination(d.path().join(path)).unwrap(),
    };
    let target = ObservedResource {
        pool: uuid(1).into(),
        generation: 1,
        target: ResourceTarget::Pool(uuid(1).into()),
    };
    let id = OperationId::new();
    let (_, first) = resources::transfer_channel(
        id,
        target,
        request("first.xva"),
        tokio::runtime::Handle::current(),
    );
    let (_, second) = resources::transfer_channel(
        id,
        target,
        request("second.xva"),
        tokio::runtime::Handle::current(),
    );
    let j = Journal::open(d.path()).unwrap();
    let mut a = first.record;
    let mut b = second.record;
    assert!(j.resource_admit(&mut a).unwrap());
    assert!(matches!(
        j.resource_admit(&mut b),
        Err(Error::OperationConflict)
    ));
    a.transfer_plan.as_mut().unwrap().dispatched = true;
    j.resource_save(&mut a).unwrap();
    a.transfer_plan.as_mut().unwrap().dispatched = false;
    assert!(matches!(
        j.resource_save(&mut a),
        Err(Error::JournalChanged)
    ));
}

#[tokio::test]
async fn acknowledging_failed_import_releases_only_local_guard_without_cleanup_or_replay() {
    let f = Fixture::new(Fault::Untagged).await;
    let d = tempfile::tempdir().unwrap();
    let r = execute(&f, d.path(), false).await;
    let j = Journal::open(d.path()).unwrap();
    assert!(j.resource_pool_busy(r.pool).unwrap());
    let acknowledged = j
        .resource_control(r.pool, r.id, crate::OperationControl::AcknowledgeUnknown)
        .unwrap();
    assert_eq!(acknowledged.state, OperationState::AcknowledgedUnknown);
    assert!(!j.resource_pool_busy(r.pool).unwrap());
    let result = recheck(&f, d.path(), acknowledged).await;
    assert_eq!(result.state, OperationState::AcknowledgedUnknown);
    assert_eq!(f.count("PUT /import"), 1);
    assert_eq!(f.count("task.destroy"), 0);
    assert!(f.records.lock().unwrap().contains_key("OpaqueRef:VM:new"));
}

fn disk_record(n: u128, name: &str) -> Value {
    json!({"uuid":uuid(n),"name_label":name,"SR":"OpaqueRef:SR:s","type":"user","managed":true,"missing":false,"sharable":false,"read_only":false,"is_a_snapshot":false,"cbt_enabled":false,"virtual_size":"4096","current_operations":{},"other_config":{},"VBDs":[]})
}
#[path = "transfer_disk_tests.rs"]
mod disk_tests;

#[path = "ovf_import_tests.rs"]
mod ovf_import_tests;

#[path = "cloud_init_protocol_tests.rs"]
mod cloud_init_protocol_tests;

#[path = "host_support_tests.rs"]
mod host_support_tests;
