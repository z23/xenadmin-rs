use super::*;
use crate::{
    ResourceAction, ResourceTarget, SnapshotPreviewUpload,
    snapshot_capture::{Plan, SnapshotPreviewImage},
    snapshot_preview,
};
use serde_json::Value;
use sha2::{Digest, Sha256};
use xenadmin_xapi::records::{BlobRecord, PoolRecord, VmRecord};

async fn bounded<T>(
    cancel: &CancellationToken,
    future: impl std::future::Future<Output = Result<T, Error>>,
) -> Result<T, Error> {
    tokio::select! {_=cancel.cancelled()=>Err(Error::Closed),r=tokio::time::timeout(Duration::from_secs(45),future)=>r.unwrap_or(Err(Error::Timeout))}
}
fn step(label: &str) -> OperationStep {
    OperationStep {
        label: label.into(),
        state: StepState::Pending,
        detail: String::new(),
    }
}
fn expected(record: &ResourceOperationRecord) -> Result<&SnapshotPreviewUpload, Error> {
    match &record.action {
        ResourceAction::SnapshotPreviewUpload(expected) => Ok(expected),
        _ => Err(Error::Journal),
    }
}
fn source(journal: &Journal, record: &ResourceOperationRecord) -> Result<(), Error> {
    let expected = expected(record)?;
    let ResourceTarget::Vm(vm) = record.target else {
        return Err(Error::Journal);
    };
    let source = journal
        .load(expected.source)?
        .ok_or(Error::Protocol("snapshot creation evidence is unavailable"))?;
    if source.pool != record.pool
        || source.vm != vm
        || !source.verified_snapshot_creation(expected.snapshot)
    {
        return Err(Error::Protocol(
            "preview does not match a verified snapshot creation",
        ));
    }
    Ok(())
}
async fn snapshot(
    session: &Session,
    record: &ResourceOperationRecord,
) -> Result<(String, Value), Error> {
    let pool_ref = session
        .call("pool.get_by_uuid", vec![json!(record.pool.to_string())])
        .await?;
    let pool = session.call("pool.get_record", vec![pool_ref]).await?;
    if PoolRecord::new(&pool).uuid() != record.pool.to_string() {
        return Err(Error::Stale);
    }
    let expected = expected(record)?;
    let ResourceTarget::Vm(vm) = record.target else {
        return Err(Error::Journal);
    };
    let (reference, value) =
        crate::snapshots::snapshot_record(session, vm, expected.snapshot).await?;
    if VmRecord::new(&value)
        .current_operations()
        .as_object()
        .is_none_or(|m| !m.is_empty())
    {
        return Err(Error::Busy);
    }
    Ok((reference, value))
}
async fn blob(session: &Session, record: &ResourceOperationRecord) -> Result<uuid::Uuid, Error> {
    let plan = record
        .snapshot_preview_plan
        .as_ref()
        .ok_or(Error::Journal)?;
    let reference = plan.create_reference.as_ref().ok_or(Error::Protocol(
        "preview creation receipt is missing; creation will not be repeated",
    ))?;
    let (_, snapshot) = snapshot(session, record).await?;
    if VmRecord::new(&snapshot).blobs()[snapshot_preview::KEY] != *reference
        || VmRecord::new(&snapshot)
            .other_config()
            .get(snapshot_preview::KEY)
            .is_some()
    {
        return Err(Error::Stale);
    }
    let fresh = session
        .call("blob.get_record", vec![json!(reference)])
        .await?;
    let id = BlobRecord::new(&fresh)
        .uuid()
        .as_str()
        .and_then(|s| uuid::Uuid::parse_str(s).ok())
        .filter(|id| !id.is_nil())
        .ok_or(Error::Stale)?;
    if plan.baseline.binary_search(&id).is_ok()
        || plan.blob.is_some_and(|old| old != id)
        || BlobRecord::new(&fresh).mime_type() != "image/png"
        || BlobRecord::new(&fresh).public() != false
    {
        return Err(Error::Stale);
    }
    Ok(id)
}
async fn verify_content(session: &Session, record: &ResourceOperationRecord) -> Result<(), Error> {
    let id = blob(session, record).await?;
    let plan = record
        .snapshot_preview_plan
        .as_ref()
        .ok_or(Error::Journal)?;
    if plan.blob != Some(id) || !plan.upload_sent {
        return Err(Error::Journal);
    }
    let ResourceTarget::Vm(vm) = record.target else {
        return Err(Error::Journal);
    };
    let image = snapshot_preview::read(
        session,
        crate::ObservedSnapshot {
            pool: record.pool,
            generation: 0,
            vm,
            id: plan.expected.snapshot,
        },
    )
    .await?
    .ok_or(Error::Protocol("preview content is unavailable"))?;
    if image.mime != "image/png"
        || image.bytes.len() as u64 != plan.expected.bytes
        || <[u8; 32]>::from(Sha256::digest(image.bytes.as_ref())) != plan.sha256
    {
        return Err(Error::Protocol(
            "preview content does not match the captured image",
        ));
    }
    blob(session, record).await?;
    Ok(())
}
async fn upload(
    session: &Session,
    record: &ResourceOperationRecord,
    image: &SnapshotPreviewImage,
    id: uuid::Uuid,
) -> Result<(), Error> {
    if session.options().read_only {
        return Err(Error::ReadOnly);
    }
    let mut url = session.options().endpoint.rpc_url();
    url.set_path("/blob");
    url.query_pairs_mut().append_pair("uuid", &id.to_string());
    let response = session
        .options()
        .client()?
        .put(url)
        .header(
            reqwest::header::COOKIE,
            format!("session_id={}", session.session_cookie()),
        )
        .header(reqwest::header::CONTENT_TYPE, "image/png")
        .body(image.bytes.to_vec())
        .timeout(Duration::from_secs(30))
        .send()
        .await
        .map_err(|e| crate::xapi::transport_error(e.without_url()))?;
    if response.status() != reqwest::StatusCode::OK {
        return Err(Error::Protocol("preview upload was refused"));
    }
    // The target sends HTTP 200 before consuming the request body. Readback, not status, proves content.
    drop(response);
    let mut error = Error::Protocol("preview content is not yet verified");
    for _ in 0..30 {
        match verify_content(session, record).await {
            Ok(()) => return Ok(()),
            Err(e) => error = e,
        }
        tokio::time::sleep(Duration::from_millis(250)).await;
    }
    Err(error)
}
impl ResourceRequest {
    async fn preview_merge(&mut self, journal: &mut Journal) -> Result<bool, Error> {
        self.record = journal.snapshot_preview_evidence(&self.record)?;
        self.announce().await;
        Ok(!self.stopped_after_recovery())
    }
    pub(super) async fn execute_snapshot_preview(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        if session.options().read_only {
            return Err(Error::ReadOnly);
        }
        source(journal, &self.record)?;
        let expected = expected(&self.record)?.clone();
        let image = self.snapshot_preview_image.take().ok_or(Error::Protocol(
            "captured preview image is no longer available",
        ))?;
        if image.descriptor(expected.source, expected.snapshot) != expected {
            return Err(Error::InvalidResourceCommand);
        }
        let (_, fresh) = bounded(cancel, snapshot(session, &self.record)).await?;
        if VmRecord::new(&fresh).blobs().as_object().is_none()
            || VmRecord::new(&fresh)
                .blobs()
                .get(snapshot_preview::KEY)
                .is_some()
            || VmRecord::new(&fresh)
                .other_config()
                .get(snapshot_preview::KEY)
                .is_some()
        {
            return Err(Error::Protocol(
                "snapshot already has a preview or its metadata is unavailable",
            ));
        }
        let raw = bounded(cancel, session.call("blob.get_all_records", vec![])).await?;
        let records = raw
            .as_object()
            .filter(|r| r.len() <= crate::snapshot_capture::MAX_BLOBS)
            .ok_or(Error::Protocol(
                "preview blob baseline is unavailable or too large",
            ))?;
        let mut baseline = records
            .values()
            .map(|v| {
                BlobRecord::new(v)
                    .uuid()
                    .as_str()
                    .and_then(|v| v.parse::<uuid::Uuid>().ok())
                    .filter(|v| !v.is_nil())
                    .ok_or(Error::Protocol(
                        "preview blob baseline has invalid identities",
                    ))
            })
            .collect::<Result<Vec<_>, _>>()?;
        baseline.sort_unstable();
        if baseline.windows(2).any(|v| v[0] == v[1]) {
            return Err(Error::Protocol(
                "preview blob baseline has duplicate identities",
            ));
        }
        self.record.snapshot_preview_plan = Some(Box::new(Plan::new(expected, &image, baseline)));
        self.record.steps = vec![
            step("Create private snapshot preview"),
            step("Upload and verify preview image"),
        ];
        self.save(journal).await?;
        let (reference, fresh) = bounded(cancel, snapshot(session, &self.record)).await?;
        if VmRecord::new(&fresh).blobs().as_object().is_none()
            || VmRecord::new(&fresh)
                .blobs()
                .get(snapshot_preview::KEY)
                .is_some()
            || VmRecord::new(&fresh)
                .other_config()
                .get(snapshot_preview::KEY)
                .is_some()
        {
            return Err(Error::Stale);
        }
        source(journal, &self.record)?;
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        if cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        self.record
            .snapshot_preview_plan
            .as_mut()
            .unwrap()
            .create_sent = true;
        self.record.state = OperationState::Dispatching;
        self.record.steps[0].state = StepState::Dispatching;
        self.record.detail = "Creating a private preview placeholder".into();
        self.save(journal).await?;
        let reply = bounded(
            cancel,
            session.call(
                "VM.create_new_blob",
                vec![
                    json!(reference),
                    json!(snapshot_preview::KEY),
                    json!("image/png"),
                    json!(false),
                ],
            ),
        )
        .await?;
        self.record
            .snapshot_preview_plan
            .as_mut()
            .unwrap()
            .create_reference = Some(crate::snapshots::reference(&reply)?);
        if !self.preview_merge(journal).await? {
            return Ok(());
        }
        let id = bounded(cancel, blob(session, &self.record)).await?;
        self.record.snapshot_preview_plan.as_mut().unwrap().blob = Some(id);
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail =
            "Private blob identity and snapshot association verified".into();
        self.record.state = OperationState::Preparing;
        if !self.preview_merge(journal).await? {
            return Ok(());
        }
        let upload_id = bounded(cancel, blob(session, &self.record)).await?;
        source(journal, &self.record)?;
        self.reload(journal)?;
        if self.stop_before_dispatch() {
            return self.save(journal).await;
        }
        if cancel.is_cancelled() {
            return Err(Error::Closed);
        }
        self.record
            .snapshot_preview_plan
            .as_mut()
            .unwrap()
            .upload_sent = true;
        self.record.state = OperationState::Dispatching;
        self.record.steps[1].state = StepState::Dispatching;
        self.record.detail = "Uploading the captured preview".into();
        self.save(journal).await?;
        bounded(cancel, upload(session, &self.record, &image, upload_id)).await?;
        self.record
            .snapshot_preview_plan
            .as_mut()
            .unwrap()
            .content_verified = true;
        self.record.steps[1].state = StepState::Verified;
        self.record.steps[1].detail = "Downloaded content matches the captured PNG".into();
        self.record.state = OperationState::Succeeded;
        self.record.can_cancel = false;
        self.record.detail =
            "Private preview attached; snapshot creation has its separate operation result".into();
        self.preview_merge(journal).await?;
        Ok(())
    }
    pub(super) async fn recheck_snapshot_preview(
        &mut self,
        journal: &mut Journal,
        session: &Session,
        cancel: &CancellationToken,
    ) -> Result<(), Error> {
        source(journal, &self.record)?;
        if self
            .record
            .snapshot_preview_plan
            .as_ref()
            .is_none_or(|p| !p.create_sent || p.create_reference.is_none())
        {
            return Err(Error::Protocol(
                "preview creation receipt is unavailable; no write will be repeated",
            ));
        }
        self.record.state = OperationState::Verifying;
        self.record.detail = "Checking saved preview evidence without repeating writes".into();
        self.save(journal).await?;
        let id = bounded(cancel, blob(session, &self.record)).await?;
        self.record.snapshot_preview_plan.as_mut().unwrap().blob = Some(id);
        self.record.steps[0].state = StepState::Verified;
        let sent = self
            .record
            .snapshot_preview_plan
            .as_ref()
            .unwrap()
            .upload_sent;
        if sent {
            bounded(cancel, verify_content(session, &self.record)).await?;
            self.record
                .snapshot_preview_plan
                .as_mut()
                .unwrap()
                .content_verified = true;
            self.record.steps[1].state = StepState::Verified;
        }
        self.record = journal.snapshot_preview_evidence(&self.record)?;
        self.reload(journal)?;
        if self.record.state.terminal() {
            return Ok(());
        }
        self.record.steps[0].state = StepState::Verified;
        self.record.steps[0].detail =
            "Private placeholder identity verified from the saved receipt".into();
        if sent {
            self.record.steps[1].state = StepState::Verified;
            self.record.steps[1].detail =
                "Downloaded PNG matches the captured image; no upload repeated".into();
        }
        self.record.state = if sent {
            OperationState::Succeeded
        } else {
            OperationState::PartiallyCompleted
        };
        self.record.detail = if sent {
            "Preview content verified by download; no upload repeated"
        } else {
            "Private placeholder exists; the unsubmitted image upload was not replayed"
        }
        .into();
        self.record.can_cancel = false;
        self.save(journal).await
    }
}
