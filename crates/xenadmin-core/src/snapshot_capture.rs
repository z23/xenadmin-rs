use crate::{
    Error, OperationId, OperationState, ResourceAction, ResourceOperationRecord, ResourceTarget,
    SnapshotId, StepState,
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::Cursor;
use zeroize::Zeroizing;

pub(crate) const MAX_BLOBS: usize = 16_384;
pub(crate) const MAX_BYTES: usize = 8 * 1024 * 1024;
pub struct SnapshotPreviewImage {
    pub(crate) bytes: Zeroizing<Vec<u8>>,
    pub(crate) width: u32,
    pub(crate) height: u32,
}
impl std::fmt::Debug for SnapshotPreviewImage {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.debug_struct("SnapshotPreviewImage")
            .field("width", &self.width)
            .field("height", &self.height)
            .field("bytes", &self.bytes.len())
            .finish()
    }
}
impl SnapshotPreviewImage {
    /// Decodes bounded PNG data for validation. Call off the UI thread.
    pub fn from_png(bytes: Vec<u8>) -> Result<Self, Error> {
        let bytes = Zeroizing::new(bytes);
        if bytes.is_empty() || bytes.len() > MAX_BYTES {
            return Err(Error::Protocol("preview image exceeds byte limit"));
        }
        let mut decoder = png::Decoder::new(Cursor::new(bytes.as_slice()));
        decoder.set_limits(png::Limits {
            bytes: 32 * 1024 * 1024,
        });
        let mut reader = decoder
            .read_info()
            .map_err(|_| Error::Protocol("preview PNG is invalid"))?;
        let info = reader.info();
        let (width, height) = (info.width, info.height);
        if width == 0
            || height == 0
            || width > 4096
            || height > 4096
            || u64::from(width) * u64::from(height) > 4_194_304
            || info.animation_control.is_some()
        {
            return Err(Error::Protocol("preview PNG exceeds image limits"));
        }
        let size = reader
            .output_buffer_size()
            .filter(|n| *n <= 32 * 1024 * 1024)
            .ok_or(Error::Protocol("preview PNG exceeds decode limits"))?;
        let mut pixels = Zeroizing::new(vec![0; size]);
        reader
            .next_frame(&mut pixels)
            .map_err(|_| Error::Protocol("preview PNG is truncated or invalid"))?;
        reader
            .finish()
            .map_err(|_| Error::Protocol("preview PNG end is invalid"))?;
        Ok(Self {
            bytes,
            width,
            height,
        })
    }
    pub fn dimensions(&self) -> (u32, u32) {
        (self.width, self.height)
    }
    pub fn byte_len(&self) -> usize {
        self.bytes.len()
    }
    pub(crate) fn descriptor(
        &self,
        source: OperationId,
        snapshot: SnapshotId,
    ) -> SnapshotPreviewUpload {
        SnapshotPreviewUpload {
            source,
            snapshot,
            width: self.width,
            height: self.height,
            bytes: self.bytes.len() as u64,
        }
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct SnapshotPreviewUpload {
    pub source: OperationId,
    pub snapshot: SnapshotId,
    pub width: u32,
    pub height: u32,
    pub bytes: u64,
}
impl SnapshotPreviewUpload {
    pub(crate) fn valid(&self, target: ResourceTarget) -> bool {
        matches!(target, ResourceTarget::Vm(_))
            && !self.snapshot.uuid().is_nil()
            && self.width > 0
            && self.height > 0
            && self.width <= 4096
            && self.height <= 4096
            && u64::from(self.width) * u64::from(self.height) <= 4_194_304
            && (1..=MAX_BYTES as u64).contains(&self.bytes)
    }
}
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct Plan {
    pub expected: SnapshotPreviewUpload,
    pub sha256: [u8; 32],
    pub baseline: Vec<uuid::Uuid>,
    pub create_sent: bool,
    pub create_reference: Option<String>,
    pub blob: Option<uuid::Uuid>,
    pub upload_sent: bool,
    pub content_verified: bool,
}
impl Plan {
    pub fn new(
        expected: SnapshotPreviewUpload,
        image: &SnapshotPreviewImage,
        baseline: Vec<uuid::Uuid>,
    ) -> Self {
        Self {
            expected,
            baseline,
            sha256: Sha256::digest(image.bytes.as_slice()).into(),
            create_sent: false,
            create_reference: None,
            blob: None,
            upload_sent: false,
            content_verified: false,
        }
    }
    pub fn extends(&self, old: &Self) -> bool {
        self.expected == old.expected
            && self.sha256 == old.sha256
            && self.baseline == old.baseline
            && (!old.create_sent || self.create_sent)
            && (old.create_reference.is_none() || self.create_reference == old.create_reference)
            && (old.blob.is_none() || self.blob == old.blob)
            && (!old.upload_sent || self.upload_sent)
            && (!old.content_verified || self.content_verified)
    }
}
pub(crate) fn validate_record(record: &ResourceOperationRecord) -> Result<(), Error> {
    let action = matches!(record.action, ResourceAction::SnapshotPreviewUpload(_));
    if let Some(plan) = &record.snapshot_preview_plan {
        if record.action != ResourceAction::SnapshotPreviewUpload(plan.expected.clone())
            || !plan.expected.valid(record.target)
            || plan.baseline.len() > MAX_BLOBS
            || plan.baseline.iter().any(uuid::Uuid::is_nil)
            || plan.baseline.windows(2).any(|v| v[0] >= v[1])
            || plan
                .blob
                .is_some_and(|id| plan.baseline.binary_search(&id).is_ok())
            || record.steps.len() != 2
            || record.steps[0].label != "Create private snapshot preview"
            || record.steps[1].label != "Upload and verify preview image"
            || plan.create_reference.as_ref().is_some_and(|r| {
                crate::snapshots::reference(&serde_json::Value::String(r.clone())).is_err()
            })
            || plan.create_reference.is_some() && !plan.create_sent
            || plan.blob.is_some() && plan.create_reference.is_none()
            || plan.blob.is_some_and(|id| id.is_nil())
            || plan.upload_sent && plan.blob.is_none()
            || plan.content_verified && !plan.upload_sent
            || record.steps[0].state == StepState::Verified && plan.blob.is_none()
            || record.steps[1].state == StepState::Verified && !plan.content_verified
            || plan.create_sent && record.steps[0].state == StepState::Pending
            || plan.upload_sent && record.steps[1].state == StepState::Pending
            || record.state == OperationState::Succeeded && !plan.content_verified
        {
            return Err(Error::Journal);
        }
    } else if action
        && !matches!(
            record.state,
            OperationState::Preparing
                | OperationState::Rejected
                | OperationState::Cancelled
                | OperationState::Failed
        )
    {
        return Err(Error::Journal);
    }
    Ok(())
}
