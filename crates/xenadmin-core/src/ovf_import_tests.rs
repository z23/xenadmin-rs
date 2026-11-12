use super::*;
#[derive(Clone, Copy, PartialEq, Eq)]
enum OvfFault {
    None,
    LostGuest,
    LostDisk,
    LostAttachment,
    LostNic,
    LostUpload,
    FailedUpload,
    PendingUpload,
    WrongDisk,
    ExistingGuest,
    WrongOwner,
    NoHosts,
    ChangedFile,
    ChangedExport,
    TruncatedExport,
    DelayedGuest,
    DestinationRace,
}
struct OvfFixture {
    options: ConnectionOptions,
    server: tokio::task::JoinHandle<()>,
    records: Arc<Mutex<BTreeMap<String, Value>>>,
    calls: Arc<Mutex<Vec<String>>>,
}
impl Drop for OvfFixture {
    fn drop(&mut self) {
        self.server.abort()
    }
}
impl OvfFixture {
    async fn new(fault: OvfFault) -> Self {
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
                "OpaqueRef:network:n".into(),
                json!({"uuid":uuid(6),"MTU":"1500"}),
            ),
            (
                "OpaqueRef:VM:old".into(),
                json!({"uuid":uuid(9),"other_config":{}}),
            ),
        ])));
        let calls = Arc::new(Mutex::new(vec![]));
        let r = records.clone();
        let c = calls.clone();
        let server = tokio::spawn(async move {
            loop {
                let (socket, _) = listener.accept().await.unwrap();
                let r = r.clone();
                let c = c.clone();
                tokio::spawn(async move {
                    let mut socket = BufReader::new(socket);
                    let mut line = String::new();
                    socket.read_line(&mut line).await.unwrap();
                    let request = line.clone();
                    let mut length = 0;
                    let mut cookie = String::new();
                    loop {
                        line.clear();
                        if socket.read_line(&mut line).await.unwrap_or(0) == 0 {
                            return;
                        }
                        if line == "\r\n" {
                            break;
                        }
                        if let Some((k, v)) = line.split_once(':') {
                            if k.eq_ignore_ascii_case("content-length") {
                                length = v.trim().parse::<usize>().unwrap()
                            }
                            if k.eq_ignore_ascii_case("cookie") {
                                cookie = v.trim().into()
                            }
                            assert!(!k.eq_ignore_ascii_case("transfer-encoding"));
                        }
                    }
                    assert!(length < 1000000);
                    let mut input = vec![0; length];
                    if socket.read_exact(&mut input).await.is_err() {
                        return;
                    }
                    let mut delay_create = false;
                    let body = if request.starts_with("PUT /import_raw_vdi?")
                        || request.starts_with("GET /export_raw_vdi?")
                    {
                        assert_eq!(cookie, "session_id=OpaqueRef:test-secret");
                        assert!(!request.contains("test-secret"));
                        let put = request.starts_with("PUT");
                        c.lock()
                            .unwrap()
                            .push(if put { "PUT" } else { "GET" }.into());
                        let url = url::Url::parse(&format!(
                            "http://localhost{}",
                            request.split_whitespace().nth(1).unwrap()
                        ))
                        .unwrap();
                        let q = url.query_pairs().collect::<BTreeMap<_, _>>();
                        let mut data = r.lock().unwrap();
                        let task = data.get_mut(q["task_id"].as_ref()).unwrap();
                        task["status"] = json!(match fault {
                            OvfFault::FailedUpload => "failure",
                            OvfFault::PendingUpload => "pending",
                            _ => "success",
                        });
                        task["result"] = json!(xml(&json!(if fault == OvfFault::WrongDisk {
                            "OpaqueRef:VDI:wrong"
                        } else {
                            q["vdi"].as_ref()
                        })));
                        if fault == OvfFault::ChangedFile {
                            let path = data["sourcepath"].as_str().unwrap();
                            std::fs::write(path, vec![9; 4096]).unwrap();
                        }
                        if matches!(fault, OvfFault::LostUpload | OvfFault::PendingUpload) {
                            return;
                        }
                        if put {
                            assert_eq!(
                                input,
                                if q["format"] == "vhd" {
                                    crate::disk_image::tests::fixed()
                                } else {
                                    vec![7; 4096]
                                }
                            );
                            Vec::new()
                        } else {
                            if fault == OvfFault::ChangedExport {
                                data.get_mut(q["vdi"].as_ref()).unwrap()["virtual_size"] =
                                    json!(8192);
                            }
                            if fault == OvfFault::DestinationRace {
                                let path = data["destination"].as_str().unwrap();
                                std::fs::write(path, b"keep this existing file").unwrap();
                            }
                            let mut bytes = crate::disk_image::tests::fixed();
                            if fault == OvfFault::TruncatedExport {
                                bytes.truncate(800);
                            }
                            bytes
                        }
                    } else {
                        let req: Value = serde_json::from_slice(&input).unwrap();
                        let method = req["method"].as_str().unwrap();
                        c.lock().unwrap().push(method.into());
                        let p = &req["params"];
                        let mut data = r.lock().unwrap();
                        let result = if method == "session.login_with_password" {
                            json!({"result":"OpaqueRef:test-secret"})
                        } else if method == "session.logout" {
                            json!({"result":null})
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
                        } else if method.ends_with(".get_record") {
                            data.get(p[1].as_str().unwrap())
                                .map(|v| json!({"result":v}))
                                .unwrap_or(json!({"error":{"message":"HANDLE_INVALID"}}))
                        } else if method == "VM.get_possible_hosts" {
                            json!({"result":if fault==OvfFault::NoHosts{vec![]}else{vec!["OpaqueRef:host:h"]}})
                        } else if method == "VM.get_allowed_VBD_devices"
                            || method == "VM.get_allowed_VIF_devices"
                        {
                            json!({"result":["0","1","2"]})
                        } else if method == "task.cancel" {
                            data.get_mut(p[1].as_str().unwrap()).unwrap()["status"] =
                                json!("cancelled");
                            json!({"result":null})
                        } else if method == "task.destroy" {
                            data.remove(p[1].as_str().unwrap());
                            json!({"result":null})
                        } else if let Some(class) = method.strip_suffix(".create") {
                            delay_create = class == "VM" && fault == OvfFault::DelayedGuest;
                            let count = data
                                .keys()
                                .filter(|k| k.starts_with(&format!("OpaqueRef:{class}:")))
                                .count();
                            let counter = format!("counter:{class}");
                            let count = data
                                .get(&counter)
                                .and_then(Value::as_u64)
                                .map_or(count, |n| n as usize);
                            data.insert(counter, json!(count + 1));
                            let reference = format!("OpaqueRef:{class}:new{count}");
                            let mut v = p[1].clone();
                            let id =
                                100 + (match class {
                                    "VM" => 0,
                                    "VDI" => 100,
                                    "VBD" => 200,
                                    "VIF" => 300,
                                    "task" => 400,
                                    _ => panic!("{class}"),
                                }) + count as u128;
                            if class == "task" {
                                v = json!({"name_label":p[1],"status":"pending","allowed_operations":["cancel"],"progress":0.0});
                            }
                            v["uuid"] =
                                json!(uuid(if fault == OvfFault::ExistingGuest && class == "VM" {
                                    9
                                } else {
                                    id
                                }));
                            v["current_operations"] = json!({});
                            match class {
                                "VM" => {
                                    v["power_state"] = json!("Halted");
                                    v["is_a_snapshot"] = json!(false);
                                    v["is_control_domain"] = json!(false);
                                    v["VBDs"] = json!([]);
                                    v["VIFs"] = json!([]);
                                }
                                "VDI" => {
                                    v["managed"] = json!(true);
                                    v["missing"] = json!(false);
                                    v["cbt_enabled"] = json!(false);
                                    v["is_a_snapshot"] = json!(false);
                                    v["VBDs"] = json!([]);
                                }
                                "VBD" => {
                                    v["currently_attached"] = json!(false);
                                    if fault == OvfFault::WrongOwner {
                                        v["other_config"]["owner"] = json!(false);
                                    }
                                    for (field, owner) in [
                                        ("VBDs", v["VM"].as_str().unwrap()),
                                        ("VBDs", v["VDI"].as_str().unwrap()),
                                    ] {
                                        data.get_mut(owner).unwrap()[field]
                                            .as_array_mut()
                                            .unwrap()
                                            .push(json!(reference));
                                    }
                                }
                                "VIF" => {
                                    v["currently_attached"] = json!(false);
                                    v["MAC"] = json!("02:00:00:00:00:01");
                                    data.get_mut(v["VM"].as_str().unwrap()).unwrap()["VIFs"]
                                        .as_array_mut()
                                        .unwrap()
                                        .push(json!(reference));
                                }
                                _ => {}
                            }
                            data.insert(reference.clone(), v);
                            if (class == "VM" && fault == OvfFault::LostGuest)
                                || (class == "VDI" && fault == OvfFault::LostDisk)
                                || (class == "VBD" && fault == OvfFault::LostAttachment)
                                || (class == "VIF" && fault == OvfFault::LostNic)
                            {
                                return;
                            }
                            json!({"result":reference})
                        } else {
                            panic!("unexpected {method}")
                        };
                        serde_json::to_vec(&json!({"jsonrpc":"2.0","id":req["id"],"result":result["result"],"error":result["error"]})).unwrap()
                    };
                    if delay_create {
                        tokio::time::sleep(std::time::Duration::from_millis(200)).await;
                    }
                    let header = format!(
                        "HTTP/1.1 200 OK\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                        body.len()
                    );
                    let _ = socket.write_all(header.as_bytes()).await;
                    let _ = socket.write_all(&body).await;
                });
            }
        });
        Self {
            options,
            server,
            records,
            calls,
        }
    }
    async fn session(&self) -> Session {
        Session::login(
            &self.options,
            &Credentials::new("root".into(), "password".into()),
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
    async fn execute(&self, dir: &Path, ova: bool) -> crate::ResourceOperationRecord {
        let descriptor = crate::ovf::tests::descriptor("disk.raw", "http://raw");
        let path = dir.join(if ova { "guest.ova" } else { "guest.ovf" });
        if ova {
            std::fs::write(
                &path,
                crate::ovf::tests::tar(&[
                    ("guest.ovf", descriptor.into_bytes()),
                    ("disk.raw", vec![7; 4096]),
                ]),
            )
            .unwrap();
        } else {
            std::fs::write(&path, descriptor).unwrap();
            std::fs::write(dir.join("disk.raw"), vec![7; 4096]).unwrap();
        }
        self.records
            .lock()
            .unwrap()
            .insert("sourcepath".into(), json!(dir.join("disk.raw")));
        let file = TransferFile::ovf_source(path).unwrap();
        let (observed, pending) = resources::transfer_channel(
            OperationId::new(),
            ObservedResource {
                pool: uuid(1).into(),
                generation: 1,
                target: ResourceTarget::Pool(uuid(1).into()),
            },
            TransferRequest {
                action: TransferAction::ImportOvf {
                    sr: uuid(4).into(),
                    networks: BTreeMap::from([("Guest network".into(), uuid(6).into())]),
                },
                file,
            },
            tokio::runtime::Handle::current(),
        );
        let (tx, _rx) = mpsc::channel(1024);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            pending.run(
                Arc::new(self.session().await),
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
        &self,
        dir: &Path,
        r: crate::ResourceOperationRecord,
    ) -> crate::ResourceOperationRecord {
        let id = r.id;
        let (tx, _rx) = mpsc::channel(1024);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            ResourceRequest::tracking(r).run(
                Arc::new(self.session().await),
                dir.into(),
                CancellationToken::new(),
                tx,
            ),
        )
        .await
        .unwrap();
        Journal::open(dir)
            .unwrap()
            .resource_load(id)
            .unwrap()
            .unwrap()
    }
}
#[tokio::test]
async fn ovf_and_ova_import_create_new_verified_halted_guests_with_mapped_devices() {
    for ova in [false, true] {
        let f = OvfFixture::new(OvfFault::None).await;
        let d = tempfile::tempdir().unwrap();
        let r = f.execute(d.path(), ova).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(r.transfer_created.vms.len(), 1);
        assert_eq!(r.transfer_created.disks.len(), 1);
        assert_eq!(r.transfer_created.vbds.len(), 1);
        assert_eq!(r.transfer_created.vifs.len(), 1);
        for m in [
            "VM.create",
            "VDI.create",
            "VBD.create",
            "VIF.create",
            "PUT",
            "task.destroy",
        ] {
            assert_eq!(f.count(m), 1, "{m}")
        }
        assert_eq!(f.count("VM.start"), 0);
        let public = serde_json::to_string(&r).unwrap();
        assert!(!public.contains("OpaqueRef:"));
        assert!(!public.contains("transfer_plan"));
        assert!(!public.contains(d.path().to_str().unwrap()));
    }
}
#[tokio::test]
async fn lost_ovf_create_replies_are_identified_without_replaying_or_continuing() {
    for fault in [
        OvfFault::LostGuest,
        OvfFault::LostDisk,
        OvfFault::LostAttachment,
        OvfFault::LostNic,
    ] {
        let f = OvfFixture::new(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = f.execute(d.path(), false).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        let r = f.recheck(d.path(), r).await;
        assert!(
            matches!(
                r.state,
                OperationState::PartiallyCompleted | OperationState::Succeeded
            ),
            "{:?}: {}",
            r.state,
            r.detail
        );
        assert_eq!(f.count("VM.create"), 1);
        assert_eq!(
            f.count("VDI.create"),
            usize::from(fault != OvfFault::LostGuest)
        );
        assert_eq!(
            f.count("PUT"),
            usize::from(matches!(
                fault,
                OvfFault::LostAttachment | OvfFault::LostNic
            ))
        );
        assert_eq!(r.transfer_created.vms.len(), 1);
    }
}
#[tokio::test]
async fn ovf_existing_ids_wrong_ownership_and_wrong_upload_results_never_succeed() {
    for fault in [
        OvfFault::ExistingGuest,
        OvfFault::WrongOwner,
        OvfFault::WrongDisk,
    ] {
        let f = OvfFixture::new(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = f.execute(d.path(), false).await;
        assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
        let r = f.recheck(d.path(), r).await;
        assert_ne!(r.state, OperationState::Succeeded);
        if fault == OvfFault::WrongOwner {
            assert_eq!(
                r.transfer_created.vbds.len(),
                1,
                "proven new identity remains inspectable despite wrong configuration"
            );
        }
        if fault == OvfFault::ExistingGuest {
            assert!(r.transfer_created.vms.is_empty());
        }
        assert_eq!(f.count("VM.create"), 1);
    }
}
#[tokio::test]
async fn ovf_lost_upload_reply_uses_task_proof_but_failed_upload_retains_partial_resources() {
    for fault in [
        OvfFault::LostUpload,
        OvfFault::FailedUpload,
        OvfFault::NoHosts,
        OvfFault::ChangedFile,
    ] {
        let f = OvfFixture::new(fault).await;
        let d = tempfile::tempdir().unwrap();
        let r = f.execute(d.path(), false).await;
        assert_eq!(
            r.state,
            if fault == OvfFault::LostUpload {
                OperationState::Succeeded
            } else {
                OperationState::PartiallyCompleted
            },
            "{}",
            r.detail
        );
        assert_eq!(f.count("PUT"), 1);
        assert_eq!(f.count("VDI.destroy"), 0);
        assert_eq!(r.transfer_created.disks.len(), 1);
    }
}
#[tokio::test]
async fn ovf_pending_upload_cancellation_is_distinct_from_stopping_stream_and_holds_pool_guard() {
    let f = OvfFixture::new(OvfFault::PendingUpload).await;
    let d = tempfile::tempdir().unwrap();
    let r = f.execute(d.path(), false).await;
    assert_eq!(r.state, OperationState::OutcomeUnknown, "{}", r.detail);
    let j = Journal::open(d.path()).unwrap();
    assert!(j.resource_pool_busy(uuid(1).into()).unwrap());
    let r = j
        .resource_control(
            uuid(1).into(),
            r.id,
            crate::OperationControl::RequestCancellation,
        )
        .unwrap();
    let r = f.recheck(d.path(), r).await;
    let r = f.recheck(d.path(), r).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(f.count("task.cancel"), 1);
    assert_eq!(f.count("PUT"), 1);
    assert_eq!(f.count("VBD.create"), 0);
}
#[tokio::test]
async fn readonly_ovf_import_refuses_before_any_creation() {
    let mut f = OvfFixture::new(OvfFault::None).await;
    f.options.read_only = true;
    let d = tempfile::tempdir().unwrap();
    let r = f.execute(d.path(), false).await;
    assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
    assert_eq!(f.count("VM.create"), 0);
}

impl OvfFixture {
    fn export_source(&self, snapshot: bool) -> ExportSource {
        let mut r = self.records.lock().unwrap();
        r.insert("OpaqueRef:VM:old".into(), json!({"uuid":uuid(9),"name_label":"Exported guest & test","name_description":"A synthetic guest","is_a_template":snapshot,"is_a_snapshot":snapshot,"snapshot_of":"OpaqueRef:VM:parent","is_control_domain":false,"power_state":"Halted","allowed_operations":["export"],"blocked_operations":{},"current_operations":{},"VCPUs_at_startup":"2","memory_static_max":"536870912","HVM_boot_policy":"BIOS order","HVM_boot_params":{"order":"dc","firmware":"uefi"},"platform":{"secureboot":"false"},"VBDs":["OpaqueRef:VBD:source"],"VIFs":["OpaqueRef:VIF:source"]}));
        r.insert("OpaqueRef:VM:parent".into(), json!({"uuid":uuid(8)}));
        r.insert("OpaqueRef:VDI:source".into(), json!({"uuid":uuid(90),"SR":"OpaqueRef:SR:s","type":"user","managed":true,"missing":false,"sharable":false,"read_only":false,"cbt_enabled":false,"virtual_size":"4096","current_operations":{},"VBDs":["OpaqueRef:VBD:source"]}));
        r.insert("OpaqueRef:VBD:source".into(), json!({"uuid":uuid(91),"VM":"OpaqueRef:VM:old","VDI":"OpaqueRef:VDI:source","userdevice":"0","type":"Disk","mode":"RW","empty":false,"currently_attached":false,"current_operations":{}}));
        r.insert("OpaqueRef:VIF:source".into(), json!({"uuid":uuid(92),"VM":"OpaqueRef:VM:old","network":"OpaqueRef:network:n","device":"2","currently_attached":false,"current_operations":{}}));
        if snapshot {
            ExportSource::Snapshot {
                snapshot: uuid(9).into(),
                parent: uuid(8).into(),
            }
        } else {
            ExportSource::Vm {
                vm: uuid(9).into(),
                expected_template: false,
            }
        }
    }
    async fn export(
        &self,
        dir: &Path,
        format: crate::OvfPackageFormat,
        source: ExportSource,
    ) -> crate::ResourceOperationRecord {
        self.records.lock().unwrap().insert(
            "destination".into(),
            json!(dir.join(if format == crate::OvfPackageFormat::Ova {
                "guest.ova"
            } else {
                "package"
            })),
        );
        let file = TransferFile::destination(dir.join(if format == crate::OvfPackageFormat::Ova {
            "guest.ova"
        } else {
            "package"
        }))
        .unwrap();
        let (observed, pending) = resources::transfer_channel(
            OperationId::new(),
            ObservedResource {
                pool: uuid(1).into(),
                generation: 1,
                target: ResourceTarget::Pool(uuid(1).into()),
            },
            TransferRequest {
                action: TransferAction::ExportOvf { source, format },
                file,
            },
            tokio::runtime::Handle::current(),
        );
        let (tx, _rx) = mpsc::channel(1024);
        tokio::time::timeout(
            std::time::Duration::from_secs(5),
            pending.run(
                Arc::new(self.session().await),
                dir.into(),
                CancellationToken::new(),
                tx,
            ),
        )
        .await
        .unwrap();
        (*observed.latest()).clone()
    }
}
#[tokio::test]
async fn ovf_directory_and_ova_export_are_private_and_manifest_validated() {
    use std::os::unix::fs::PermissionsExt;
    for format in [crate::OvfPackageFormat::Ova, crate::OvfPackageFormat::Ovf] {
        for snapshot in [false, true] {
            let mut f = OvfFixture::new(OvfFault::None).await;
            f.options.read_only = true;
            let source = f.export_source(snapshot);
            let d = tempfile::tempdir().unwrap();
            let r = f.export(d.path(), format, source).await;
            assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
            let path = d.path().join(if format == crate::OvfPackageFormat::Ova {
                "guest.ova"
            } else {
                "package/guest.ovf"
            });
            assert_eq!(path.metadata().unwrap().permissions().mode() & 0o777, 0o600);
            if format == crate::OvfPackageFormat::Ovf {
                assert_eq!(
                    d.path()
                        .join("package")
                        .metadata()
                        .unwrap()
                        .permissions()
                        .mode()
                        & 0o777,
                    0o700
                );
            }
            let file = TransferFile::ovf_source(&path).unwrap();
            let summary = file.ovf().unwrap();
            assert!(summary.manifest_verified);
            assert_eq!(summary.guests[0].name, "Exported guest & test");
            assert_eq!(summary.guests[0].firmware, crate::GuestFirmware::Uefi);
            assert_eq!(summary.guests[0].nics[0].device, 2);
            assert_eq!(summary.disks[0].virtual_size, 4096);
            assert_eq!(f.count("GET"), 1);
            assert_eq!(f.count("task.destroy"), 1);
            assert_eq!(f.count("VM.create"), 0);
            if format == crate::OvfPackageFormat::Ova {
                assert!(
                    !std::fs::read_dir(d.path())
                        .unwrap()
                        .filter_map(Result::ok)
                        .any(|e| e
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".xenadmin-ovf-"))
                );
            }
        }
    }
}
#[tokio::test]
async fn changed_or_truncated_ovf_exports_never_publish_and_recheck_never_downloads_again() {
    for fault in [OvfFault::ChangedExport, OvfFault::TruncatedExport] {
        let f = OvfFixture::new(fault).await;
        let source = f.export_source(false);
        let d = tempfile::tempdir().unwrap();
        let r = f
            .export(d.path(), crate::OvfPackageFormat::Ova, source)
            .await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert!(!d.path().join("guest.ova").exists());
        let r = f.recheck(d.path(), r).await;
        assert_ne!(r.state, OperationState::Succeeded);
        assert_eq!(f.count("GET"), 1);
        assert!(!d.path().join("guest.ova").exists());
    }
}
#[tokio::test]
async fn unsupported_or_running_ovf_guests_fail_before_export_task_or_download() {
    for mode in 0..3 {
        let f = OvfFixture::new(OvfFault::None).await;
        let source = f.export_source(false);
        {
            let mut r = f.records.lock().unwrap();
            let vm = r.get_mut("OpaqueRef:VM:old").unwrap();
            match mode {
                0 => vm["power_state"] = json!("Running"),
                1 => vm["platform"]["secureboot"] = json!("true"),
                _ => vm["HVM_boot_policy"] = json!(""),
            }
        }
        let d = tempfile::tempdir().unwrap();
        let r = f
            .export(d.path(), crate::OvfPackageFormat::Ova, source)
            .await;
        assert_eq!(r.state, OperationState::Rejected, "{}", r.detail);
        assert_eq!(f.count("GET"), 0);
        assert_eq!(f.count("task.create"), 0);
    }
}

#[tokio::test]
async fn ovf_late_guest_identity_survives_recovery_and_acknowledgement_without_continuation() {
    for acknowledged in [false, true] {
        let f = OvfFixture::new(OvfFault::DelayedGuest).await;
        let d = tempfile::tempdir().unwrap();
        let run = f.execute(d.path(), false);
        let interrupt = async {
            for _ in 0..200 {
                if f.count("VM.create") > 0 {
                    let j = Journal::open(d.path()).unwrap();
                    let rows = j.resource_recover(uuid(1).into()).unwrap();
                    assert_eq!(rows.len(), 1);
                    if acknowledged {
                        j.resource_control(
                            uuid(1).into(),
                            rows[0].id,
                            crate::OperationControl::AcknowledgeUnknown,
                        )
                        .unwrap();
                    }
                    return;
                }
                tokio::time::sleep(std::time::Duration::from_millis(2)).await;
            }
            panic!("no guest create")
        };
        let (r, _) = tokio::join!(run, interrupt);
        assert_eq!(
            r.state,
            if acknowledged {
                OperationState::AcknowledgedUnknown
            } else {
                OperationState::OutcomeUnknown
            },
            "{}",
            r.detail
        );
        assert_eq!(r.transfer_created.vms.len(), 1);
        assert_eq!(f.count("VDI.create"), 0);
        assert_eq!(f.count("VM.create"), 1);
    }
}
#[tokio::test]
async fn ovf_task_success_evidence_survives_restart_and_removed_task_without_upload_replay() {
    let f = OvfFixture::new(OvfFault::PendingUpload).await;
    let d = tempfile::tempdir().unwrap();
    let mut r = f.execute(d.path(), false).await;
    let p = r
        .transfer_plan
        .as_mut()
        .unwrap()
        .ovf_import
        .as_mut()
        .unwrap();
    let disk = p
        .reference(crate::ovf::import::Kind::Disk { guest: 0, disk: 0 })
        .unwrap()
        .to_owned();
    let step = p
        .steps
        .iter_mut()
        .find(|s| matches!(s.kind, crate::ovf::import::Kind::Upload { .. }))
        .unwrap();
    step.stream.as_mut().unwrap().task_success = true;
    step.stream.as_mut().unwrap().result = Some(vec![disk]);
    let task = step.task.clone().unwrap();
    Journal::open(d.path())
        .unwrap()
        .resource_save(&mut r)
        .unwrap();
    f.records.lock().unwrap().remove(&task);
    let r = f.recheck(d.path(), r).await;
    assert_eq!(r.state, OperationState::PartiallyCompleted, "{}", r.detail);
    assert_eq!(f.count("PUT"), 1);
    assert_eq!(f.count("VBD.create"), 0);
}
#[tokio::test]
async fn ovf_output_destination_race_does_not_overwrite_existing_data() {
    let f = OvfFixture::new(OvfFault::DestinationRace).await;
    let source = f.export_source(false);
    let d = tempfile::tempdir().unwrap();
    let r = f
        .export(d.path(), crate::OvfPackageFormat::Ova, source)
        .await;
    assert_ne!(r.state, OperationState::Succeeded);
    assert_eq!(
        std::fs::read(d.path().join("guest.ova")).unwrap(),
        b"keep this existing file"
    );
    assert_eq!(f.count("GET"), 1);
}
#[tokio::test]
async fn ovf_persisted_export_proof_reconciles_after_task_removal_and_publication() {
    for format in [crate::OvfPackageFormat::Ova, crate::OvfPackageFormat::Ovf] {
        let f = OvfFixture::new(OvfFault::None).await;
        let source = f.export_source(false);
        let d = tempfile::tempdir().unwrap();
        let mut r = f.export(d.path(), format, source).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        // Model the journal image just before the final terminal save, after atomic publication.
        r.id = OperationId::new();
        r.revision = 0;
        r.state = OperationState::OutcomeUnknown;
        r.transfer_plan
            .as_mut()
            .unwrap()
            .ovf_export
            .as_mut()
            .unwrap()
            .published = false;
        Journal::open(d.path())
            .unwrap()
            .resource_save(&mut r)
            .unwrap();
        let r = f.recheck(d.path(), r).await;
        assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
        assert_eq!(f.count("GET"), 1);
        assert_eq!(f.count("task.create"), 1);
    }
}

#[tokio::test]
async fn ovf_collection_creates_private_disk_copies_for_each_guest() {
    let f = OvfFixture::new(OvfFault::None).await;
    let d = tempfile::tempdir().unwrap();
    let xml = crate::ovf::tests::descriptor("disk.raw", "http://raw");
    let start = xml.find("<VirtualSystem ovf:id").unwrap();
    let guest = &xml[start..xml.find("</Envelope>").unwrap()];
    let package = format!(
        "{}<VirtualSystemCollection ovf:id=\"collection\"><Name>Collection</Name>{}{}</VirtualSystemCollection></Envelope>",
        &xml[..start],
        guest,
        guest
            .replace("vm1", "vm2")
            .replace("Imported guest", "Second guest")
    );
    std::fs::write(d.path().join("collection.ovf"), package).unwrap();
    std::fs::write(d.path().join("disk.raw"), vec![7; 4096]).unwrap();
    let file = TransferFile::ovf_source(d.path().join("collection.ovf")).unwrap();
    assert_eq!(file.ovf().unwrap().guests.len(), 2);
    let (observed, pending) = resources::transfer_channel(
        OperationId::new(),
        ObservedResource {
            pool: uuid(1).into(),
            generation: 1,
            target: ResourceTarget::Pool(uuid(1).into()),
        },
        TransferRequest {
            action: TransferAction::ImportOvf {
                sr: uuid(4).into(),
                networks: BTreeMap::from([("Guest network".into(), uuid(6).into())]),
            },
            file,
        },
        tokio::runtime::Handle::current(),
    );
    let (tx, _rx) = mpsc::channel(1024);
    pending
        .run(
            Arc::new(f.session().await),
            d.path().into(),
            CancellationToken::new(),
            tx,
        )
        .await;
    let r = observed.latest();
    assert_eq!(r.state, OperationState::Succeeded, "{}", r.detail);
    assert_eq!(r.transfer_created.vms.len(), 2);
    assert_eq!(r.transfer_created.disks.len(), 2);
    assert_ne!(r.transfer_created.disks[0], r.transfer_created.disks[1]);
    assert_eq!(f.count("PUT"), 2);
    assert_eq!(f.count("task.create"), 2);
    assert_eq!(f.count("task.destroy"), 2);
}

#[tokio::test]
async fn batch_ovf_and_ova_exports_keep_each_manifest_and_child_identity() {
    for format in [crate::OvfPackageFormat::Ovf, crate::OvfPackageFormat::Ova] {
        let mut fixture = OvfFixture::new(OvfFault::None).await;
        fixture.options.read_only = true;
        fixture.export_source(false);
        {
            let mut records = fixture.records.lock().unwrap();
            let mut guest = records["OpaqueRef:VM:old"].clone();
            guest["uuid"] = json!(uuid(19));
            guest["name_label"] = json!("Second captured guest");
            guest["VBDs"] = json!(["OpaqueRef:VBD:second"]);
            guest["VIFs"] = json!([]);
            records.insert("OpaqueRef:VM:second".into(), guest);
            let mut disk = records["OpaqueRef:VDI:source"].clone();
            disk["uuid"] = json!(uuid(70));
            disk["VBDs"] = json!(["OpaqueRef:VBD:second"]);
            records.insert("OpaqueRef:VDI:second".into(), disk);
            let mut attachment = records["OpaqueRef:VBD:source"].clone();
            attachment["uuid"] = json!(uuid(71));
            attachment["VM"] = json!("OpaqueRef:VM:second");
            attachment["VDI"] = json!("OpaqueRef:VDI:second");
            records.insert("OpaqueRef:VBD:second".into(), attachment);
        }
        let dir = tempfile::tempdir().unwrap();
        let selection = crate::VmWorkflowBatch {
            items: [9, 19]
                .into_iter()
                .map(|id| crate::BatchVmWorkflow {
                    vm: uuid(id).into(),
                    expected_state: crate::PowerState::Halted,
                    workflow: crate::VmWorkflow::Export {
                        expected_template: false,
                        format: crate::VmExportFormat::Ovf { format },
                    },
                })
                .collect(),
        };
        let outputs = [9, 19]
            .into_iter()
            .map(|id| {
                (
                    uuid(id),
                    TransferFile::destination(dir.path().join(format!(
                        "guest-{id}.{}",
                        if format == crate::OvfPackageFormat::Ova {
                            "ova"
                        } else {
                            "ovf-package"
                        }
                    )))
                    .unwrap(),
                )
            })
            .collect();
        let files = crate::vm_workflow_export::bind(&selection, outputs).unwrap();
        let (handle, mut request) = resources::channel(
            OperationId::new(),
            ObservedResource {
                pool: uuid(1).into(),
                target: ResourceTarget::Pool(uuid(1).into()),
                generation: 1,
            },
            crate::ResourceAction::VmWorkflowBatch(selection),
            tokio::runtime::Handle::current(),
        );
        request.record.workflow_batch_plan.as_mut().unwrap().outputs = files;
        let (tx, mut rx) = mpsc::channel(128);
        let drain = tokio::spawn(async move { while rx.recv().await.is_some() {} });
        tokio::time::timeout(
            std::time::Duration::from_secs(20),
            request.run(
                Arc::new(fixture.session().await),
                dir.path().into(),
                CancellationToken::new(),
                tx,
            ),
        )
        .await
        .unwrap();
        drain.abort();
        assert_eq!(
            handle.latest().state,
            OperationState::Succeeded,
            "{}",
            handle.latest().detail
        );
        for (id, name) in [(9, "Exported guest & test"), (19, "Second captured guest")] {
            let output = dir.path().join(format!(
                "guest-{id}.{}",
                if format == crate::OvfPackageFormat::Ova {
                    "ova"
                } else {
                    "ovf-package"
                }
            ));
            let source = TransferFile::ovf_source(if format == crate::OvfPackageFormat::Ovf {
                output.join("guest.ovf")
            } else {
                output
            })
            .unwrap();
            assert!(source.ovf().unwrap().manifest_verified);
            assert_eq!(source.ovf().unwrap().guests[0].name, name);
        }
        assert_eq!(fixture.count("GET"), 2);
        assert_eq!(fixture.count("task.destroy"), 2);
        assert_eq!(fixture.count("VM.create"), 0);
    }
}
