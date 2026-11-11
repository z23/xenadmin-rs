use crate::{Error, ObservedSnapshot, xapi::Session};
use base64::Engine;
use serde_json::{Value, json};
use std::{sync::Arc, time::Duration};
use tokio::sync::watch;
use tokio_util::sync::CancellationToken;
use xenadmin_xapi::records::{BlobRecord, PoolRecord, VmRecord};

pub(crate) const KEY: &str = "XenCenter.VNCSnapshot";
const MAX_BYTES: usize = 8 * 1024 * 1024;
#[derive(Clone)]
pub struct SnapshotPreview {
    pub mime: &'static str,
    pub bytes: Arc<[u8]>,
}
impl std::fmt::Debug for SnapshotPreview {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotPreview")
            .field("mime", &self.mime)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
pub struct SnapshotPreviewQuery {
    receiver: watch::Receiver<Option<Result<Option<SnapshotPreview>, String>>>,
    runtime: tokio::runtime::Handle,
    cancel: CancellationToken,
}
impl SnapshotPreviewQuery {
    pub fn latest(&self) -> Option<Result<Option<SnapshotPreview>, String>> {
        self.receiver.borrow().clone().or_else(|| {
            self.receiver
                .has_changed()
                .is_err()
                .then(|| Err(Error::Closed.to_string()))
        })
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            if receiver.changed().await.is_ok() {
                callback();
            }
        });
    }
}
impl Drop for SnapshotPreviewQuery {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}
pub(crate) struct Request {
    pub observed: ObservedSnapshot,
    sender: watch::Sender<Option<Result<Option<SnapshotPreview>, String>>>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    observed: ObservedSnapshot,
    runtime: tokio::runtime::Handle,
) -> (SnapshotPreviewQuery, Request) {
    let (sender, receiver) = watch::channel(None);
    let cancel = CancellationToken::new();
    (
        SnapshotPreviewQuery {
            receiver,
            runtime,
            cancel: cancel.clone(),
        },
        Request {
            observed,
            sender,
            cancel,
        },
    )
}
impl Request {
    pub(crate) fn fail(self, error: Error) {
        self.sender.send_replace(Some(Err(error.to_string())));
    }
    pub(crate) async fn run(
        self,
        session: Arc<Session>,
        receiver: watch::Receiver<Arc<crate::ConnectionSnapshot>>,
        generation: CancellationToken,
    ) {
        let result = tokio::select! {
            _=self.cancel.cancelled()=>Err(Error::Closed),
            _=generation.cancelled()=>Err(Error::Stale),
            r=tokio::time::timeout(Duration::from_secs(30),read(&session,self.observed))=>r.unwrap_or(Err(Error::Timeout))
        };
        let current = receiver
            .borrow()
            .observe_snapshot(self.observed.vm, self.observed.id)
            .is_ok_and(|v| v == self.observed);
        self.sender.send_replace(Some(if current {
            result.map_err(|e| e.to_string())
        } else {
            Err(Error::Stale.to_string())
        }));
    }
}
fn image(bytes: Vec<u8>) -> Result<SnapshotPreview, Error> {
    if bytes.is_empty() || bytes.len() > MAX_BYTES {
        return Err(Error::Protocol("snapshot preview exceeds image bounds"));
    }
    let mime = if bytes.starts_with(b"\x89PNG\r\n\x1a\n") {
        "image/png"
    } else if bytes.starts_with(b"\xff\xd8\xff") {
        "image/jpeg"
    } else {
        return Err(Error::Protocol(
            "snapshot preview is not a PNG or JPEG image",
        ));
    };
    Ok(SnapshotPreview {
        mime,
        bytes: bytes.into(),
    })
}
fn blob_identity(record: &Value) -> Result<(uuid::Uuid, u64, String, String), Error> {
    let id = BlobRecord::new(record)
        .uuid()
        .as_str()
        .and_then(|s| s.parse().ok())
        .ok_or(Error::Protocol("invalid snapshot preview identity"))?;
    let size = BlobRecord::new(record)
        .size()
        .as_u64()
        .or_else(|| BlobRecord::new(record).size().as_str()?.parse().ok())
        .filter(|size| *size > 0 && *size <= MAX_BYTES as u64)
        .ok_or(Error::Protocol("snapshot preview exceeds image bounds"))?;
    let mime = BlobRecord::new(record)
        .mime_type()
        .as_str()
        .filter(|s| matches!(*s, "image/png" | "image/jpeg"))
        .ok_or(Error::Protocol("unsupported snapshot preview type"))?
        .to_owned();
    let updated = BlobRecord::new(record)
        .last_updated()
        .as_str()
        .filter(|s| s.len() <= 64)
        .ok_or(Error::Protocol("snapshot preview metadata unavailable"))?
        .to_owned();
    Ok((id, size, mime, updated))
}
pub(crate) async fn read(
    session: &Session,
    observed: ObservedSnapshot,
) -> Result<Option<SnapshotPreview>, Error> {
    let pool = session
        .call("pool.get_by_uuid", vec![json!(observed.pool.to_string())])
        .await?;
    let pool_record = session.call("pool.get_record", vec![pool]).await?;
    if PoolRecord::new(&pool_record).uuid() != observed.pool.to_string() {
        return Err(Error::Stale);
    }
    let (snapshot_ref, before) =
        crate::snapshots::snapshot_record(session, observed.vm, observed.id).await?;
    let blob = VmRecord::new(&before).blobs().value().get(KEY);
    let result = if let Some(blob) = blob {
        let reference = crate::snapshots::reference(blob)?;
        let metadata = session
            .call("blob.get_record", vec![json!(&reference)])
            .await?;
        let identity = blob_identity(&metadata)?;
        let mut url = session.options().endpoint.rpc_url();
        url.set_path("/blob");
        url.query_pairs_mut()
            .append_pair("uuid", &identity.0.to_string());
        let mut response = session
            .options()
            .client()?
            .get(url)
            .header(
                reqwest::header::COOKIE,
                format!("session_id={}", session.session_cookie()),
            )
            .timeout(Duration::from_secs(20))
            .send()
            .await
            .map_err(|e| crate::xapi::transport_error(e.without_url()))?;
        if response.status() != reqwest::StatusCode::OK {
            return Err(Error::Protocol("snapshot preview download was refused"));
        }
        if response.content_length().is_some_and(|n| n != identity.1) {
            return Err(Error::Protocol("snapshot preview size changed"));
        }
        let mut bytes = Vec::new();
        while let Some(chunk) = response
            .chunk()
            .await
            .map_err(|e| crate::xapi::transport_error(e.without_url()))?
        {
            if chunk.len() > MAX_BYTES.saturating_sub(bytes.len()) {
                return Err(Error::Protocol("snapshot preview exceeds image bounds"));
            }
            bytes.extend_from_slice(&chunk);
        }
        if bytes.len() as u64 != identity.1 {
            return Err(Error::Protocol("snapshot preview size changed"));
        }
        let image = image(bytes)?;
        if image.mime != identity.2 {
            return Err(Error::Protocol("snapshot preview type changed"));
        }
        let after = session
            .call("blob.get_record", vec![json!(reference)])
            .await?;
        if blob_identity(&after)? != identity {
            return Err(Error::Stale);
        }
        Some(image)
    } else if let Some(encoded) = VmRecord::new(&before).other_config().get(KEY) {
        let encoded = encoded
            .as_str()
            .filter(|s| s.len() <= MAX_BYTES.div_ceil(3) * 4)
            .ok_or(Error::Protocol("snapshot preview exceeds image bounds"))?;
        let bytes = base64::engine::general_purpose::STANDARD
            .decode(encoded)
            .map_err(|_| Error::Protocol("invalid snapshot preview encoding"))?;
        Some(image(bytes)?)
    } else {
        None
    };
    let (after_ref, after) =
        crate::snapshots::snapshot_record(session, observed.vm, observed.id).await?;
    if after_ref != snapshot_ref
        || VmRecord::new(&after).blobs().get(KEY) != VmRecord::new(&before).blobs().get(KEY)
        || VmRecord::new(&after).other_config().get(KEY)
            != VmRecord::new(&before).other_config().get(KEY)
    {
        return Err(Error::Stale);
    }
    Ok(result)
}

#[cfg(test)]
#[path = "snapshot_preview_tests.rs"]
mod tests;
