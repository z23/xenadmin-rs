use super::*;
use crate::{ConnectionOptions, Endpoint, xapi::Session};
use sha2::{Digest, Sha256};
use std::{
    collections::BTreeSet,
    pin::Pin,
    sync::{
        Arc,
        atomic::{AtomicU64, Ordering},
    },
    task::{Context, Poll},
    time::Duration,
};
use tokio::io::{AsyncRead, AsyncReadExt, AsyncWriteExt, ReadBuf};
use tokio_util::sync::CancellationToken;

#[derive(Clone, Copy)]
pub(crate) enum Kind {
    Archive,
    Disk,
}
impl Kind {
    fn path(self, upload: bool) -> &'static str {
        match (self, upload) {
            (Self::Archive, true) => "/import",
            (Self::Archive, false) => "/export",
            (Self::Disk, true) => "/import_raw_vdi",
            (Self::Disk, false) => "/export_raw_vdi",
        }
    }
}
pub(crate) struct Prepared {
    pub kind: Kind,
    pub endpoint: Endpoint,
    pub known_hosts: BTreeSet<String>,
    pub query: Vec<(String, String)>,
    pub upload: bool,
}
#[derive(Default)]
pub(crate) struct Meter {
    pub bytes: AtomicU64,
    pub total: AtomicU64,
    pub cancel: CancellationToken,
}
impl Meter {
    pub fn progress(&self) -> TransferProgress {
        let total = self.total.load(Ordering::Relaxed);
        TransferProgress {
            bytes: self.bytes.load(Ordering::Relaxed),
            total: (total > 0).then_some(total),
            stream_complete: false,
            phase: "Streaming".into(),
        }
    }
}
struct Reader {
    file: tokio::io::Take<tokio::fs::File>,
    meter: Arc<Meter>,
}
impl AsyncRead for Reader {
    fn poll_read(
        self: Pin<&mut Self>,
        cx: &mut Context<'_>,
        buffer: &mut ReadBuf<'_>,
    ) -> Poll<std::io::Result<()>> {
        let this = self.get_mut();
        if this.meter.cancel.is_cancelled() {
            return Poll::Ready(Err(std::io::ErrorKind::Interrupted.into()));
        }
        let before = buffer.filled().len();
        let result = Pin::new(&mut this.file).poll_read(cx, buffer);
        if let Poll::Ready(Ok(())) = result {
            this.meter
                .bytes
                .fetch_add((buffer.filled().len() - before) as u64, Ordering::Relaxed);
        }
        result
    }
}
fn url(
    endpoint: &Endpoint,
    upload: bool,
    kind: Kind,
    query: &[(String, String)],
) -> Result<url::Url, Error> {
    let mut u = url::Url::parse(endpoint.as_str()).map_err(|_| Error::TransferEndpoint)?;
    u.set_path(kind.path(upload));
    u.query_pairs_mut().extend_pairs(query.iter());
    Ok(u)
}
fn redirected(
    current: &url::Url,
    location: &str,
    prepared: &Prepared,
    options: &ConnectionOptions,
) -> Result<Endpoint, Error> {
    let target = current
        .join(location)
        .map_err(|_| Error::TransferEndpoint)?;
    let original =
        url::Url::parse(options.endpoint.as_str()).map_err(|_| Error::TransferEndpoint)?;
    if target.username() != ""
        || target.password().is_some()
        || target.fragment().is_some()
        || target.path() != current.path()
    {
        return Err(Error::TransferEndpoint);
    }
    let host = target.host_str().ok_or(Error::TransferEndpoint)?;
    let same_authority = target.scheme() == original.scheme()
        && target.host_str() == original.host_str()
        && target.port_or_known_default() == original.port_or_known_default();
    if !same_authority
        && (!prepared.known_hosts.contains(host)
            || target.scheme() != "https"
            || target.port_or_known_default() != Some(443))
    {
        return Err(Error::TransferEndpoint);
    }
    let expected = prepared
        .query
        .iter()
        .cloned()
        .collect::<std::collections::BTreeMap<_, _>>();
    let mut seen = BTreeSet::new();
    for (key, value) in target.query_pairs() {
        if !seen.insert(key.to_string()) {
            return Err(Error::TransferEndpoint);
        }
        if key == "session_id" {
            return Err(Error::TransferEndpoint);
        }
        if expected.get(key.as_ref()).map(String::as_str) != Some(value.as_ref()) {
            return Err(Error::TransferEndpoint);
        }
    }
    // Some hosts retain no query in Location. Reconstruct our original fixed query below.
    Endpoint::parse(&target[..url::Position::BeforePath]).map_err(|_| Error::TransferEndpoint)
}
fn client(session: &Session, endpoint: Endpoint) -> Result<reqwest::Client, Error> {
    let mut options = session.options().for_endpoint(endpoint);
    options.request_timeout = Duration::from_secs(24 * 60 * 60);
    options.client()
}
fn error(error: reqwest::Error) -> Error {
    crate::xapi::transport_error(error.without_url())
}
async fn status(response: reqwest::Response) -> Result<reqwest::Response, Error> {
    if response.status() == reqwest::StatusCode::UNAUTHORIZED {
        return Err(Error::TransferExpired);
    }
    if !response.status().is_success() {
        return Err(Error::Http(response.status().as_u16()));
    }
    Ok(response)
}
async fn export_response(
    session: &Session,
    prepared: &Prepared,
    meter: &Meter,
) -> Result<reqwest::Response, Error> {
    let mut endpoint = prepared.endpoint.clone();
    let mut visited = BTreeSet::new();
    for _ in 0..5 {
        if !visited.insert(endpoint.to_string()) {
            return Err(Error::TransferEndpoint);
        }
        let target = url(&endpoint, false, prepared.kind, &prepared.query)?;
        let response = tokio::select! {_=meter.cancel.cancelled()=>return Err(Error::Closed),result=client(session,endpoint.clone())?.get(target.clone()).header(reqwest::header::COOKIE,format!("session_id={}",session.session_cookie())).header(reqwest::header::ACCEPT,"application/octet-stream").send()=>result.map_err(error)?};
        if response.status().is_redirection() {
            let location = response
                .headers()
                .get(reqwest::header::LOCATION)
                .and_then(|v| v.to_str().ok())
                .ok_or(Error::TransferEndpoint)?;
            endpoint = redirected(&target, location, prepared, session.options())?;
            continue;
        }
        return status(response).await;
    }
    Err(Error::TransferEndpoint)
}
pub(crate) async fn run(
    session: &Session,
    prepared: Prepared,
    plan: Plan,
    meter: Arc<Meter>,
) -> Result<files::Receipt, Error> {
    if prepared.upload {
        let input = files::source(&plan.file)?;
        let verifier = input.try_clone().map_err(|_| Error::TransferFile)?;
        let bytes = plan.file.size().ok_or(Error::TransferFile)?;
        meter.total.store(bytes, Ordering::Relaxed);
        let reader = Reader {
            file: tokio::fs::File::from_std(input).take(bytes),
            meter: meter.clone(),
        };
        let response = tokio::select! {_=meter.cancel.cancelled()=>return Err(Error::Closed),result=client(session,prepared.endpoint.clone())?.put(url(&prepared.endpoint,true,prepared.kind,&prepared.query)?).header(reqwest::header::COOKIE,format!("session_id={}",session.session_cookie())).header(reqwest::header::CONTENT_TYPE,"application/octet-stream").header(reqwest::header::CONTENT_LENGTH,bytes).body(reqwest::Body::wrap_stream(tokio_util::io::ReaderStream::with_capacity(reader,65536))).send()=>result.map_err(error)?};
        if response.status().is_redirection() {
            return Err(Error::TransferEndpoint);
        }
        let mut response = status(response).await?;
        let mut reply = 0usize;
        loop {
            let chunk = tokio::select! {_=meter.cancel.cancelled()=>return Err(Error::Closed),result=tokio::time::timeout(Duration::from_secs(60),response.chunk())=>result.map_err(|_|Error::Timeout)?.map_err(error)?};
            let Some(chunk) = chunk else { break };
            reply = reply
                .checked_add(chunk.len())
                .filter(|n| *n <= 1024 * 1024)
                .ok_or(Error::TransferLimit)?;
        }
        if meter.bytes.load(Ordering::Relaxed) != bytes {
            return Err(Error::TransferConflict);
        }
        files::source_unchanged(&plan.file, &verifier)?;
        // This receipt records selected file identity and sent byte count; it does not claim a server-side disk digest.
        files::receipt(&verifier, bytes, [0; 32])
    } else {
        let output = files::output(&plan)?;
        let verifier = output.try_clone().map_err(|_| Error::TransferFile)?;
        let mut output = tokio::fs::File::from_std(output);
        let mut response = export_response(session, &prepared, &meter).await?;
        let expected = response.content_length();
        if expected.is_some_and(|n| n == 0 || n > MAX_TRANSFER_BYTES) {
            return Err(Error::TransferLimit);
        }
        meter.total.store(expected.unwrap_or(0), Ordering::Relaxed);
        let mut bytes = 0u64;
        let mut digest = Sha256::new();
        loop {
            let next = tokio::select! {_=meter.cancel.cancelled()=>return Err(Error::Closed),result=tokio::time::timeout(Duration::from_secs(60),response.chunk())=>result.map_err(|_|Error::Timeout)?.map_err(error)?};
            let Some(chunk) = next else { break };
            bytes = bytes
                .checked_add(chunk.len() as u64)
                .filter(|n| *n <= MAX_TRANSFER_BYTES)
                .ok_or(Error::TransferLimit)?;
            for piece in chunk.chunks(65536) {
                output
                    .write_all(piece)
                    .await
                    .map_err(|_| Error::TransferFile)?;
                digest.update(piece);
                meter.bytes.fetch_add(piece.len() as u64, Ordering::Relaxed);
            }
        }
        if bytes < 512 || expected.is_some_and(|n| n != bytes) {
            return Err(Error::TransferConflict);
        }
        output.flush().await.map_err(|_| Error::TransferFile)?;
        output.sync_all().await.map_err(|_| Error::TransferFile)?;
        files::receipt(&verifier, bytes, digest.finalize().into())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn redirects_reject_credentials_unknown_hosts_ports_paths_and_query_changes() {
        let options =
            ConnectionOptions::new(Endpoint::parse("https://coordinator.example").unwrap());
        let prepared = Prepared {
            kind: Kind::Archive,
            endpoint: options.endpoint.clone(),
            known_hosts: BTreeSet::from(["member.example".into()]),
            query: vec![
                ("uuid".into(), "selected-vm".into()),
                ("task_id".into(), "OpaqueRef:owned".into()),
            ],
            upload: false,
        };
        let current = url(&prepared.endpoint, false, prepared.kind, &prepared.query).unwrap();
        assert!(
            redirected(
                &current,
                "https://member.example/export",
                &prepared,
                &options
            )
            .is_ok()
        );
        for location in [
            "https://evil.example/export",
            "http://member.example/export",
            "https://member.example:444/export",
            "https://member.example/import",
            "https://user:password@member.example/export",
            "https://member.example/export?session_id=SECRET",
            "https://member.example/export?uuid=wrong",
            "https://member.example/export?uuid=selected-vm&uuid=selected-vm",
            "https://member.example/export#fragment",
        ] {
            assert!(
                redirected(&current, location, &prepared, &options).is_err(),
                "{location}"
            );
        }
    }
}
