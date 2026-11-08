use crate::{Error, Inventory, xapi::Session};
use serde_json::json;
use std::{collections::BTreeMap, sync::Arc, time::Duration};
use tokio::{
    io::{AsyncRead, AsyncReadExt, AsyncWrite, AsyncWriteExt},
    net::TcpStream,
    sync::{mpsc, watch},
};
use tokio_util::sync::CancellationToken;
use vnc::{VncConnector, VncEncoding, VncEvent, X11Event};

#[path = "console_clipboard.rs"]
mod clipboard;

const MAX_PIXELS: usize = 8_294_400;

#[derive(Clone, Debug, PartialEq, Eq)]
pub enum ConsoleState {
    Connecting,
    Connected,
    Closed,
    Failed(String),
}
#[derive(Clone)]
pub struct ConsoleFrame {
    pub width: u16,
    pub height: u16,
    pub rgba: Arc<[u8]>,
}
#[derive(Clone)]
pub struct ConsoleCursor {
    pub width: u16,
    pub height: u16,
    pub hotspot: [u16; 2],
    pub rgba: Arc<[u8]>,
}
#[derive(Clone)]
pub struct ConsoleSnapshot {
    pub state: ConsoleState,
    pub frame: Option<Arc<ConsoleFrame>>,
    pub cursor: Option<Arc<ConsoleCursor>>,
    pub guest_clipboard: Option<Arc<str>>,
    pub clipboard_utf8: bool,
    pub extended_keys: bool,
}
#[derive(Clone, Debug)]
pub enum ConsoleInput {
    Key {
        keysym: u32,
        down: bool,
    },
    PhysicalKey {
        keysym: u32,
        keycode: u32,
        down: bool,
    },
    KeyChord(Vec<u32>),
    Clipboard(String),
    Pointer {
        x: u16,
        y: u16,
        buttons: u8,
    },
    ReleaseAll,
}

pub struct Console {
    receiver: watch::Receiver<Arc<ConsoleSnapshot>>,
    input: mpsc::Sender<ConsoleInput>,
    cancel: CancellationToken,
    runtime: tokio::runtime::Handle,
}
impl Console {
    pub fn latest(&self) -> Arc<ConsoleSnapshot> {
        self.receiver.borrow().clone()
    }
    pub fn input(&self, event: ConsoleInput) -> Result<(), Error> {
        if let ConsoleInput::Clipboard(text) = &event {
            clipboard::validate(text, self.latest().clipboard_utf8)?;
        }
        if let ConsoleInput::KeyChord(keys) = &event
            && keys.len() > 16
        {
            return Err(Error::ConsoleProtocol);
        }
        self.input.try_send(event).map_err(|_| {
            self.cancel.cancel();
            Error::Busy
        })
    }
    pub fn close(&self) {
        self.cancel.cancel();
    }
    pub fn notify_on_change(&self, callback: Arc<dyn Fn() + Send + Sync>) {
        let mut receiver = self.receiver.clone();
        self.runtime.spawn(async move {
            while receiver.changed().await.is_ok() {
                callback();
            }
        });
    }
}
impl Drop for Console {
    fn drop(&mut self) {
        self.cancel.cancel();
    }
}

pub(crate) struct ConsoleRequest {
    pub vm: crate::VmId,
    pub host: Option<crate::HostId>,
    pub generation: u64,
    sender: watch::Sender<Arc<ConsoleSnapshot>>,
    inputs: mpsc::Receiver<ConsoleInput>,
    cancel: CancellationToken,
}
pub(crate) fn channel(
    vm: crate::VmId,
    generation: u64,
    cancel: CancellationToken,
    runtime: tokio::runtime::Handle,
) -> (Console, ConsoleRequest) {
    let (sender, receiver) = watch::channel(Arc::new(ConsoleSnapshot {
        state: ConsoleState::Connecting,
        frame: None,
        cursor: None,
        guest_clipboard: None,
        clipboard_utf8: false,
        extended_keys: false,
    }));
    let (input, inputs) = mpsc::channel(128);
    (
        Console {
            receiver,
            input,
            cancel: cancel.clone(),
            runtime,
        },
        ConsoleRequest {
            vm,
            host: None,
            generation,
            sender,
            inputs,
            cancel,
        },
    )
}
impl ConsoleRequest {
    pub(crate) fn fail(self, error: Error) {
        self.sender.send_replace(Arc::new(ConsoleSnapshot {
            state: ConsoleState::Failed(error.to_string()),
            frame: None,
            cursor: None,
            guest_clipboard: None,
            clipboard_utf8: false,
            extended_keys: false,
        }));
    }
    pub(crate) async fn run(
        mut self,
        session: Arc<Session>,
        reference: String,
        inventory: Arc<Inventory>,
        generation_cancel: CancellationToken,
    ) {
        let cancel = self.cancel.clone();
        let observer = tokio::spawn(async move {
            generation_cancel.cancelled().await;
            cancel.cancel();
        });
        let result = self.stream(&session, &reference, &inventory).await;
        observer.abort();
        let previous = self.sender.borrow().as_ref().clone();
        self.sender.send_replace(Arc::new(ConsoleSnapshot {
            state: match result {
                Ok(()) => ConsoleState::Closed,
                Err(error) => ConsoleState::Failed(error.to_string()),
            },
            ..previous
        }));
    }

    async fn stream(
        &mut self,
        session: &Session,
        reference: &str,
        inventory: &Inventory,
    ) -> Result<(), Error> {
        let cancel = self.cancel.clone();
        let opening = async {
            let reference = if let Some(host) = self.host {
                host_reference(session, host, self.vm).await?
            } else {
                reference.to_owned()
            };
            let references = session
                .call("VM.get_consoles", vec![json!(reference)])
                .await?;
            let mut location = None;
            for console_ref in references
                .as_array()
                .filter(|v| v.len() <= 32)
                .ok_or(Error::ConsoleProtocol)?
            {
                let console_ref = crate::xapi::opaque_reference(console_ref)?;
                let console = session
                    .call("console.get_record", vec![json!(console_ref)])
                    .await?;
                if console["protocol"].as_str() == Some("rfb") {
                    if console["VM"] != reference {
                        return Err(Error::Stale);
                    }
                    location = console["location"].as_str().map(str::to_owned);
                    break;
                }
            }
            let location = location.ok_or(Error::NoConsole)?;
            let stream = tokio::time::timeout(
                Duration::from_secs(15),
                tunnel(session, &location, inventory),
            )
            .await
            .map_err(|_| Error::Timeout)??;
            let client = tokio::time::timeout(Duration::from_secs(15), async {
                VncConnector::new(stream)
                    .set_auth_method(async { Err(vnc::VncError::ConnectError) })
                    .set_version(vnc::VncVersion::RFB33)
                    .allow_shared(true)
                    .set_pixel_format(vnc::PixelFormat::rgba())
                    .add_encoding(VncEncoding::Zrle)
                    .add_encoding(VncEncoding::CopyRect)
                    .add_encoding(VncEncoding::Raw)
                    .add_encoding(VncEncoding::DesktopSizePseudo)
                    .add_encoding(VncEncoding::ExtendedDesktopSizePseudo)
                    .add_encoding(VncEncoding::CursorPseudo)
                    .add_encoding(VncEncoding::QemuExtendedKeyEvent)
                    .add_encoding(VncEncoding::ExtendedClipboard)
                    .build()?
                    .try_start()
                    .await?
                    .finish()
            })
            .await
            .map_err(|_| Error::Timeout)?
            .map_err(|_| Error::ConsoleProtocol)?;
            Ok::<_, Error>(client)
        };
        let client = tokio::select! {
            biased;
            _ = cancel.cancelled() => return Ok(()),
            client = opening => client?,
        };
        let mut buffer = Framebuffer::default();
        let mut held = BTreeMap::new();
        let mut clipboard = clipboard::Clipboard::default();
        let mut snapshot = self.sender.borrow().as_ref().clone();
        let mut refresh = tokio::time::interval(Duration::from_millis(33));
        refresh.set_missed_tick_behavior(tokio::time::MissedTickBehavior::Skip);
        let result = 'running: loop {
            tokio::select! {
                _ = self.cancel.cancelled() => break Ok(()),
                input = self.inputs.recv() => {
                    let Some(input) = input else { break Ok(()); };
                    let result = match input {
                        ConsoleInput::Key {keysym, down} => {
                            if down { held.insert(u64::from(keysym), (keysym, None)); } else { held.remove(&u64::from(keysym)); }
                            client.input(key_event(keysym, None, down)).await
                        }
                        ConsoleInput::PhysicalKey {keysym, keycode, down} => {
                            let code = if snapshot.extended_keys { Some(keycode) } else { None };
                            let identity = (1u64 << 32) | u64::from(keycode);
                            let key = held.get(&identity).copied().unwrap_or((keysym, code));
                            if down { held.insert(identity, key); } else { held.remove(&identity); }
                            client.input(key_event(key.0, key.1, down)).await
                        }
                        ConsoleInput::KeyChord(keys) => {
                            for (_, (symbol, code)) in std::mem::take(&mut held) { let _ = client.input(key_event(symbol, code, false)).await; }
                            let mut result = Ok(());
                            for symbol in &keys { if let Err(error) = client.input(key_event(*symbol, None, true)).await { result = Err(error); break; } held.insert(u64::from(*symbol), (*symbol, None)); }
                            for symbol in keys.iter().rev() { let _ = client.input(key_event(*symbol, None, false)).await; held.remove(&u64::from(*symbol)); }
                            result
                        }
                        ConsoleInput::Clipboard(text) => clipboard.send(&client, text).await,
                        ConsoleInput::Pointer {x,y,buttons} => client.input(X11Event::PointerEvent((x,y,buttons).into())).await,
                        ConsoleInput::ReleaseAll => {
                            for (_, (symbol, code)) in std::mem::take(&mut held) { let _ = client.input(key_event(symbol, code, false)).await; }
                            client.input(X11Event::PointerEvent((0,0,0).into())).await
                        }
                    };
                    if result.is_err() { break Err(Error::ConsoleProtocol); }
                }
                _ = refresh.tick() => {
                    let mut changed = false;
                    let mut failed = false;
                    let mut metadata_changed = false;
                    for _ in 0..256 {
                        match client.poll_event().await {
                            Ok(Some(VncEvent::SetResolution(screen))) => { if let Err(error)=buffer.resize(screen.width, screen.height){break 'running Err(error);} changed = true; }
                            Ok(Some(VncEvent::RawImage(rect, data))) => { if let Err(error)=buffer.draw(rect, &data){break 'running Err(error);} changed = true; }
                            Ok(Some(VncEvent::Copy(dst, src))) => { if let Err(error)=buffer.copy(dst, src){break 'running Err(error);} changed = true; }
                            Ok(Some(VncEvent::DesktopUpdate(update))) => {
                                if let Some(layout) = update.layout && (buffer.width, buffer.height) != (layout.width, layout.height) { if let Err(error) = buffer.resize(layout.width, layout.height) { break 'running Err(error); } changed = true; }
                            }
                            Ok(Some(VncEvent::QemuExtendedKeyEvent)) => { snapshot.extended_keys = true; metadata_changed = true; }
                            Ok(Some(VncEvent::SetCursor(rect, rgba))) => {
                                if rgba.len() != usize::from(rect.width) * usize::from(rect.height) * 4 || rect.width > 256 || rect.height > 256 { break 'running Err(Error::ConsoleProtocol); }
                                snapshot.cursor = Some(Arc::new(ConsoleCursor { width: rect.width, height: rect.height, hotspot: [rect.x.min(rect.width.saturating_sub(1)), rect.y.min(rect.height.saturating_sub(1))], rgba: rgba.into() })); metadata_changed = true;
                            }
                            Ok(Some(VncEvent::Text(text))) => { if text.len() > clipboard::MAX_TEXT { break 'running Err(Error::ConsoleProtocol); } clipboard.guest = Some(text); metadata_changed = true; }
                            Ok(Some(VncEvent::Clipboard(message))) => { if clipboard.receive(&client, message).await.is_err() { break 'running Err(Error::ConsoleProtocol); } metadata_changed = true; }
                            Ok(Some(VncEvent::Error(_))) | Err(_) => { failed = true; break; }
                            Ok(None) => break,
                            _ => {},
                        }
                    }
                    if failed { break Err(Error::ConsoleProtocol); }
                    if changed && buffer.width > 0 {
                        let frame = Arc::new(ConsoleFrame { width: buffer.width, height: buffer.height, rgba: buffer.rgba.clone().into() });
                        snapshot.state = ConsoleState::Connected; snapshot.frame = Some(frame);
                    }
                    if changed || metadata_changed {
                        snapshot.guest_clipboard = clipboard.guest.as_deref().map(Arc::from);
                        snapshot.clipboard_utf8 = clipboard.utf8();
                        self.sender.send_replace(Arc::new(snapshot.clone()));
                    }
                    if client.input(X11Event::Refresh).await.is_err() { break Err(Error::ConsoleProtocol); }
                }
            }
        };
        let _ = tokio::time::timeout(Duration::from_secs(1), async {
            for (_, (symbol, code)) in held {
                let _ = client.input(key_event(symbol, code, false)).await;
            }
            let _ = client.input(X11Event::PointerEvent((0, 0, 0).into())).await;
            let _ = client.close().await;
        })
        .await;
        result
    }
}

async fn host_reference(
    session: &Session,
    host: crate::HostId,
    vm: crate::VmId,
) -> Result<String, Error> {
    let host_ref = crate::xapi::opaque_reference(
        &session
            .call("host.get_by_uuid", vec![json!(host.to_string())])
            .await?,
    )?;
    let host_record = session
        .call("host.get_record", vec![json!(host_ref)])
        .await?;
    if host_record["uuid"] != host.to_string() {
        return Err(Error::Stale);
    }
    let vm_ref = crate::xapi::opaque_reference(
        &session
            .call("host.get_control_domain", vec![json!(host_ref)])
            .await?,
    )?;
    let vm_record = session.call("VM.get_record", vec![json!(vm_ref)]).await?;
    if vm_record["uuid"] != vm.to_string()
        || vm_record["is_control_domain"] != true
        || vm_record["is_a_template"] != false
        || vm_record["is_a_snapshot"] != false
        || vm_record["resident_on"] != host_ref
        || vm_record["power_state"] != "Running"
    {
        return Err(Error::Stale);
    }
    Ok(vm_ref)
}

fn key_event(keysym: u32, keycode: Option<u32>, down: bool) -> X11Event {
    match keycode {
        Some(keycode) => X11Event::ExtendedKeyEvent {
            keysym,
            keycode,
            down,
        },
        None => X11Event::KeyEvent((keysym, down).into()),
    }
}

trait Stream: AsyncRead + AsyncWrite + Unpin + Send + Sync {}
impl<T: AsyncRead + AsyncWrite + Unpin + Send + Sync> Stream for T {}
async fn tunnel(
    session: &Session,
    location: &str,
    inventory: &Inventory,
) -> Result<Box<dyn Stream>, Error> {
    let mut url = url::Url::parse(location).map_err(|_| Error::ConsoleEndpoint)?;
    let original =
        url::Url::parse(session.options().endpoint.as_str()).map_err(|_| Error::ConsoleEndpoint)?;
    let host = url.host_str().ok_or(Error::ConsoleEndpoint)?.to_owned();
    if !url.username().is_empty()
        || url.password().is_some()
        || url.fragment().is_some()
        || !["http", "https"].contains(&url.scheme())
        || !(original.host_str() == Some(host.as_str())
            || inventory.hosts.values().any(|h| h.address == host))
    {
        return Err(Error::ConsoleEndpoint);
    }
    let same_host = original.host_str() == Some(host.as_str());
    let loopback_http = original.scheme() == "http" && same_host;
    if !loopback_http {
        if url.port() == Some(80) {
            url.set_port(Some(443))
                .map_err(|_| Error::ConsoleEndpoint)?;
        }
        url.set_scheme("https")
            .map_err(|_| Error::ConsoleEndpoint)?;
    }
    let port = url.port_or_known_default().ok_or(Error::ConsoleEndpoint)?;
    let endpoint = crate::Endpoint::parse(&url[..url::Position::BeforePath])?;
    let options = session.options().for_endpoint(endpoint.clone());
    let socket = TcpStream::connect((host.as_str(), port))
        .await
        .map_err(|_| Error::Transport)?;
    socket.set_nodelay(true).map_err(|_| Error::Transport)?;
    let mut stream: Box<dyn Stream> = if url.scheme() == "https" {
        let tls = tokio_rustls::TlsConnector::from(Arc::new(options.tls_config()?));
        let name = rustls::pki_types::ServerName::try_from(host.clone())
            .map_err(|_| Error::ConsoleEndpoint)?;
        Box::new(
            tls.connect(name, socket)
                .await
                .map_err(|_| Error::EndpointCertificate(endpoint))?,
        )
    } else {
        Box::new(socket)
    };
    let path = &url[url::Position::BeforePath..url::Position::AfterQuery];
    let cookie = session.session_cookie();
    if cookie.bytes().any(|b| !b.is_ascii_graphic() || b == b';') {
        return Err(Error::ConsoleProtocol);
    }
    let authority = &url[url::Position::BeforeHost..url::Position::AfterPort];
    stream.write_all(format!("CONNECT {path} HTTP/1.0\r\nHost: {authority}\r\nCookie: session_id={cookie}\r\n\r\n").as_bytes()).await.map_err(|_| Error::Transport)?;
    let mut header = Vec::new();
    while !header.ends_with(b"\r\n\r\n") {
        if header.len() >= 16384 {
            return Err(Error::ConsoleProtocol);
        }
        header.push(stream.read_u8().await.map_err(|_| Error::ConsoleProtocol)?);
    }
    let status = header
        .split(|b| *b == b'\n')
        .next()
        .and_then(|line| std::str::from_utf8(line).ok())
        .and_then(|line| line.split_whitespace().nth(1));
    if status != Some("200") {
        return Err(Error::ConsoleProtocol);
    }
    Ok(stream)
}

#[derive(Default)]
struct Framebuffer {
    width: u16,
    height: u16,
    rgba: Vec<u8>,
}
impl Framebuffer {
    fn resize(&mut self, width: u16, height: u16) -> Result<(), Error> {
        let pixels = usize::from(width) * usize::from(height);
        if pixels == 0 || pixels > MAX_PIXELS || width > 8192 || height > 8192 {
            return Err(Error::ConsoleProtocol);
        }
        self.width = width;
        self.height = height;
        self.rgba = vec![0; pixels * 4];
        Ok(())
    }
    fn valid(&self, r: vnc::Rect) -> bool {
        u32::from(r.x) + u32::from(r.width) <= u32::from(self.width)
            && u32::from(r.y) + u32::from(r.height) <= u32::from(self.height)
    }
    fn draw(&mut self, rect: vnc::Rect, data: &[u8]) -> Result<(), Error> {
        if !self.valid(rect) || data.len() != usize::from(rect.width) * usize::from(rect.height) * 4
        {
            return Err(Error::ConsoleProtocol);
        }
        for y in 0..usize::from(rect.height) {
            let dst =
                ((usize::from(rect.y) + y) * usize::from(self.width) + usize::from(rect.x)) * 4;
            let src = y * usize::from(rect.width) * 4;
            let size = usize::from(rect.width) * 4;
            self.rgba[dst..dst + size].copy_from_slice(&data[src..src + size]);
            for pixel in self.rgba[dst..dst + size].as_chunks_mut::<4>().0 {
                pixel[3] = 255;
            }
        }
        Ok(())
    }
    fn copy(&mut self, dst: vnc::Rect, src: vnc::Rect) -> Result<(), Error> {
        if !self.valid(src)
            || !self.valid(dst)
            || src.width != dst.width
            || src.height != dst.height
        {
            return Err(Error::ConsoleProtocol);
        }
        let mut pixels = Vec::with_capacity(usize::from(src.width) * usize::from(src.height) * 4);
        for y in 0..usize::from(src.height) {
            let offset =
                ((usize::from(src.y) + y) * usize::from(self.width) + usize::from(src.x)) * 4;
            pixels.extend_from_slice(&self.rgba[offset..offset + usize::from(src.width) * 4]);
        }
        self.draw(dst, &pixels)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn overlapping_copy_uses_original_pixels_and_rejects_out_of_bounds() {
        let mut buffer = Framebuffer::default();
        buffer.resize(3, 1).unwrap();
        buffer
            .draw(
                vnc::Rect {
                    x: 0,
                    y: 0,
                    width: 3,
                    height: 1,
                },
                &[1, 0, 0, 0, 2, 0, 0, 0, 3, 0, 0, 0],
            )
            .unwrap();
        buffer
            .copy(
                vnc::Rect {
                    x: 1,
                    y: 0,
                    width: 2,
                    height: 1,
                },
                vnc::Rect {
                    x: 0,
                    y: 0,
                    width: 2,
                    height: 1,
                },
            )
            .unwrap();
        assert_eq!(buffer.rgba, [1, 0, 0, 255, 1, 0, 0, 255, 2, 0, 0, 255]);
        assert!(
            buffer
                .draw(
                    vnc::Rect {
                        x: 2,
                        y: 0,
                        width: 2,
                        height: 1
                    },
                    &[0; 8]
                )
                .is_err()
        );
    }
}
