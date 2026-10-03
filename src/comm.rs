//! All interprocess transport for Terminal Salem.
//!
//! Unix domain stream sockets carry bytes, not messages. This module gives the
//! byte stream application-level boundaries: a four-byte, big-endian length
//! followed by one UTF-8 JSON document. No socket operations belong in the game
//! rules modules.

use serde::de::DeserializeOwned;
use serde::{Deserialize, Serialize};
use std::collections::{HashMap, HashSet};
use std::fmt;
use std::fs;
use std::io::{self, Read, Write};
use std::os::unix::fs::MetadataExt;
use std::os::unix::fs::PermissionsExt;
use std::os::unix::net::{UnixListener, UnixStream};
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, AtomicU64, AtomicUsize, Ordering};
use std::sync::mpsc::{self, Receiver, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Mutex};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The largest accepted JSON payload. The four-byte header is not included.
pub const MAX_FRAME_BYTES: usize = 64 * 1024;
pub const PROTOCOL_VERSION: u16 = 1;
const ACCEPT_POLL: Duration = Duration::from_millis(20);

#[derive(Clone, Copy, Debug, PartialEq, Eq, Hash, Serialize, Deserialize)]
pub struct PeerId(pub u64);

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct PeerCredentials {
    pub uid: u32,
    pub gid: u32,
    pub pid: i32,
}

#[derive(Clone, Debug)]
pub struct TransportLimits {
    pub outgoing_per_peer: usize,
    pub incoming_global: usize,
    pub max_connections: usize,
    pub max_incomplete_handshakes: usize,
    pub handshake_timeout: Duration,
    pub frame_timeout: Duration,
    pub write_timeout: Duration,
    pub require_unique_uid: bool,
    pub reject_root: bool,
    pub reject_server_uid: bool,
}

impl Default for TransportLimits {
    fn default() -> Self {
        Self {
            outgoing_per_peer: 256,
            incoming_global: 1_024,
            max_connections: 9,
            max_incomplete_handshakes: 4,
            handshake_timeout: Duration::from_secs(5),
            frame_timeout: Duration::from_secs(5),
            write_timeout: Duration::from_secs(5),
            require_unique_uid: true,
            reject_root: true,
            reject_server_uid: true,
        }
    }
}

#[derive(Debug)]
pub enum CommError {
    Io(io::Error),
    EmptyFrame,
    FrameTooLarge { declared: usize, maximum: usize },
    InvalidJson(serde_json::Error),
    Protocol(String),
    QueueFull(PeerId),
    UnknownPeer(PeerId),
    Disconnected,
}

impl fmt::Display for CommError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(error) => write!(f, "socket I/O failed: {error}"),
            Self::EmptyFrame => write!(f, "empty frames are not valid messages"),
            Self::FrameTooLarge { declared, maximum } => {
                write!(f, "frame declares {declared} bytes; maximum is {maximum}")
            }
            Self::InvalidJson(error) => write!(f, "invalid JSON message: {error}"),
            Self::Protocol(text) => write!(f, "protocol error: {text}"),
            Self::QueueFull(peer) => write!(f, "outgoing queue for peer {} is full", peer.0),
            Self::UnknownPeer(peer) => write!(f, "unknown peer {}", peer.0),
            Self::Disconnected => write!(f, "transport disconnected"),
        }
    }
}

impl std::error::Error for CommError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io(error) => Some(error),
            Self::InvalidJson(error) => Some(error),
            Self::EmptyFrame
            | Self::FrameTooLarge { .. }
            | Self::Protocol(_)
            | Self::QueueFull(_)
            | Self::UnknownPeer(_)
            | Self::Disconnected => None,
        }
    }
}

impl From<io::Error> for CommError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serde_json::Error> for CommError {
    fn from(value: serde_json::Error) -> Self {
        Self::InvalidJson(value)
    }
}

/// First-stage protocol messages sent by a client.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ClientMessage {
    Hello {
        version: u16,
        name: String,
    },
    Command {
        request_id: u64,
        phase_id: u64,
        text: String,
    },
    Close,
}

/// First-stage protocol messages sent by the server.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "snake_case")]
pub enum ServerMessage {
    Welcome {
        version: u16,
        peer_id: u64,
        text: String,
    },
    Reply {
        request_id: u64,
        accepted: bool,
        text: String,
    },
    Event {
        sequence: u64,
        phase_id: u64,
        channel: String,
        text: String,
    },
    Error {
        text: String,
    },
    Closing {
        reason: String,
    },
}

/// Serialize and write one complete frame.
///
/// `write_all` is essential here: one `write` call is allowed to accept only
/// part of either the header or body on a stream socket.
pub fn write_message<W, T>(writer: &mut W, message: &T) -> Result<(), CommError>
where
    W: Write,
    T: Serialize,
{
    let body = serde_json::to_vec(message)?;
    if body.is_empty() {
        return Err(CommError::EmptyFrame);
    }
    if body.len() > MAX_FRAME_BYTES {
        return Err(CommError::FrameTooLarge {
            declared: body.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }

    let length = u32::try_from(body.len()).expect("64 KiB always fits in u32");
    writer.write_all(&length.to_be_bytes())?;
    writer.write_all(&body)?;
    writer.flush()?;
    Ok(())
}

/// Read and deserialize exactly one frame.
///
/// The declared length is checked before allocating the body buffer. Repeated
/// calls can safely read frames that arrived together because `read_exact`
/// consumes only the current header and body.
pub fn read_message<R, T>(reader: &mut R) -> Result<T, CommError>
where
    R: Read,
    T: DeserializeOwned,
{
    let mut header = [0_u8; 4];
    reader.read_exact(&mut header)?;
    let declared = u32::from_be_bytes(header) as usize;
    if declared == 0 {
        return Err(CommError::EmptyFrame);
    }
    if declared > MAX_FRAME_BYTES {
        return Err(CommError::FrameTooLarge {
            declared,
            maximum: MAX_FRAME_BYTES,
        });
    }

    let mut body = vec![0_u8; declared];
    reader.read_exact(&mut body)?;
    Ok(serde_json::from_slice(&body)?)
}

fn serialized_frame<T: Serialize>(message: &T) -> Result<Vec<u8>, CommError> {
    let body = serde_json::to_vec(message)?;
    if body.is_empty() {
        return Err(CommError::EmptyFrame);
    }
    if body.len() > MAX_FRAME_BYTES {
        return Err(CommError::FrameTooLarge {
            declared: body.len(),
            maximum: MAX_FRAME_BYTES,
        });
    }
    let mut frame = Vec::with_capacity(4 + body.len());
    frame.extend_from_slice(&(body.len() as u32).to_be_bytes());
    frame.extend_from_slice(&body);
    Ok(frame)
}

fn read_exact_until(
    stream: &mut UnixStream,
    buffer: &mut [u8],
    deadline: Instant,
) -> Result<(), CommError> {
    let mut offset = 0;
    while offset < buffer.len() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "frame deadline expired"))?;
        stream.set_read_timeout(Some(remaining.max(Duration::from_millis(1))))?;
        match stream.read(&mut buffer[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::UnexpectedEof,
                    "socket closed during frame",
                )
                .into());
            }
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

fn read_socket_message<T: DeserializeOwned>(
    stream: &mut UnixStream,
    idle_timeout: Option<Duration>,
    frame_timeout: Duration,
) -> Result<T, CommError> {
    stream.set_read_timeout(idle_timeout)?;
    let mut header = [0_u8; 4];
    loop {
        match stream.read(&mut header[..1]) {
            Ok(0) => return Err(CommError::Disconnected),
            Ok(1) => break,
            Ok(_) => unreachable!("one-byte buffer cannot read more than one byte"),
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }

    let deadline = Instant::now() + frame_timeout;
    read_exact_until(stream, &mut header[1..], deadline)?;
    let declared = u32::from_be_bytes(header) as usize;
    if declared == 0 {
        return Err(CommError::EmptyFrame);
    }
    if declared > MAX_FRAME_BYTES {
        return Err(CommError::FrameTooLarge {
            declared,
            maximum: MAX_FRAME_BYTES,
        });
    }
    let mut body = vec![0_u8; declared];
    read_exact_until(stream, &mut body, deadline)?;
    stream.set_read_timeout(idle_timeout)?;
    Ok(serde_json::from_slice(&body)?)
}

fn write_socket_message<T: Serialize>(
    stream: &mut UnixStream,
    message: &T,
    timeout: Duration,
) -> Result<(), CommError> {
    let frame = serialized_frame(message)?;
    let deadline = Instant::now() + timeout;
    let mut offset = 0;
    while offset < frame.len() {
        let remaining = deadline
            .checked_duration_since(Instant::now())
            .ok_or_else(|| io::Error::new(io::ErrorKind::TimedOut, "write deadline expired"))?;
        stream.set_write_timeout(Some(remaining.max(Duration::from_millis(1))))?;
        match stream.write(&frame[offset..]) {
            Ok(0) => {
                return Err(io::Error::new(
                    io::ErrorKind::WriteZero,
                    "socket stopped accepting frame bytes",
                )
                .into());
            }
            Ok(count) => offset += count,
            Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error.into()),
        }
    }
    Ok(())
}

#[derive(Clone, Debug)]
pub enum ServerEvent {
    Connected {
        peer: PeerId,
        credentials: PeerCredentials,
        name: String,
    },
    Message {
        peer: PeerId,
        message: ClientMessage,
    },
    Disconnected {
        peer: PeerId,
        reason: String,
    },
}

#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub struct DeliveryReport {
    pub queued: Vec<PeerId>,
    pub failed: Vec<PeerId>,
}

#[derive(Debug)]
enum WriterCommand {
    Message(ServerMessage),
    Close(String),
}

#[derive(Debug)]
struct WriterHandle {
    sender: SyncSender<WriterCommand>,
    shutdown: UnixStream,
}

#[derive(Debug)]
struct LifecycleNotice {
    peer: PeerId,
    reason: String,
}

struct IncompleteHandshake(Arc<AtomicUsize>);

impl Drop for IncompleteHandshake {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::AcqRel);
    }
}

#[derive(Clone, Copy, Debug)]
struct SocketIdentity {
    device: u64,
    inode: u64,
}

/// Server-side owner of all interprocess connections.
///
/// The caller owns game state and calls `recv_timeout`; this transport owns only
/// sockets, framing, credentials, and delivery queues.
pub struct ServerTransport {
    path: PathBuf,
    socket_identity: SocketIdentity,
    incoming: Receiver<ServerEvent>,
    lifecycle: Receiver<LifecycleNotice>,
    writers: Arc<Mutex<HashMap<PeerId, WriterHandle>>>,
    reserved_uids: Arc<Mutex<HashSet<u32>>>,
    stop: Arc<AtomicBool>,
    accept_thread: Option<JoinHandle<()>>,
}

impl ServerTransport {
    pub fn bind(path: impl AsRef<Path>, limits: TransportLimits) -> Result<Self, CommError> {
        let path = path.as_ref().to_path_buf();
        if fs::symlink_metadata(&path).is_ok() {
            return Err(CommError::Protocol(format!(
                "{} already exists; refusing to remove it",
                path.display()
            )));
        }
        let listener = UnixListener::bind(&path)?;
        fs::set_permissions(&path, fs::Permissions::from_mode(0o660))?;
        let metadata = fs::symlink_metadata(&path)?;
        let socket_identity = SocketIdentity {
            device: metadata.dev(),
            inode: metadata.ino(),
        };
        listener.set_nonblocking(true)?;

        let (incoming_tx, incoming) = mpsc::sync_channel(limits.incoming_global);
        let (lifecycle_tx, lifecycle) = mpsc::channel();
        let writers = Arc::new(Mutex::new(HashMap::new()));
        let reserved_uids = Arc::new(Mutex::new(HashSet::new()));
        let stop = Arc::new(AtomicBool::new(false));
        let next_peer = Arc::new(AtomicU64::new(1));
        let incomplete = Arc::new(AtomicUsize::new(0));
        let server_uid = nix::unistd::Uid::current().as_raw();

        let accept_thread = {
            let writers = Arc::clone(&writers);
            let reserved_uids = Arc::clone(&reserved_uids);
            let stop = Arc::clone(&stop);
            thread::Builder::new()
                .name("salem-accept".into())
                .spawn(move || {
                    while !stop.load(Ordering::Acquire) {
                        match listener.accept() {
                            Ok((stream, _)) => {
                                if writers.lock().expect("writers poisoned").len()
                                    >= limits.max_connections
                                {
                                    reject_stream(stream, "server is full", limits.write_timeout);
                                    continue;
                                }
                                if incomplete.fetch_add(1, Ordering::AcqRel)
                                    >= limits.max_incomplete_handshakes
                                {
                                    incomplete.fetch_sub(1, Ordering::AcqRel);
                                    reject_stream(
                                        stream,
                                        "too many incomplete handshakes",
                                        limits.write_timeout,
                                    );
                                    continue;
                                }

                                let peer = PeerId(next_peer.fetch_add(1, Ordering::Relaxed));
                                let incoming_tx = incoming_tx.clone();
                                let lifecycle_tx = lifecycle_tx.clone();
                                let writers = Arc::clone(&writers);
                                let reserved_uids = Arc::clone(&reserved_uids);
                                let incomplete = Arc::clone(&incomplete);
                                let limits = limits.clone();
                                let _ = thread::Builder::new()
                                    .name(format!("salem-handshake-{}", peer.0))
                                    .spawn(move || {
                                        handle_connection(
                                            stream,
                                            peer,
                                            server_uid,
                                            limits,
                                            incoming_tx,
                                            lifecycle_tx,
                                            writers,
                                            reserved_uids,
                                            IncompleteHandshake(incomplete),
                                        );
                                    });
                            }
                            Err(error) if error.kind() == io::ErrorKind::WouldBlock => {
                                thread::sleep(ACCEPT_POLL);
                            }
                            Err(_) => break,
                        }
                    }
                })?
        };

        Ok(Self {
            path,
            socket_identity,
            incoming,
            lifecycle,
            writers,
            reserved_uids,
            stop,
            accept_thread: Some(accept_thread),
        })
    }

    pub fn recv_timeout(&mut self, timeout: Duration) -> Result<Option<ServerEvent>, CommError> {
        let deadline = Instant::now() + timeout;
        loop {
            if let Ok(notice) = self.lifecycle.try_recv() {
                self.writers
                    .lock()
                    .expect("writers poisoned")
                    .remove(&notice.peer);
                return Ok(Some(ServerEvent::Disconnected {
                    peer: notice.peer,
                    reason: notice.reason,
                }));
            }

            let remaining = match deadline.checked_duration_since(Instant::now()) {
                Some(remaining) if !remaining.is_zero() => remaining,
                _ => return Ok(None),
            };
            match self.incoming.recv_timeout(remaining.min(ACCEPT_POLL)) {
                Ok(event) => return Ok(Some(event)),
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => return Err(CommError::Disconnected),
            }
        }
    }

    pub fn send_to(&self, peer: PeerId, message: ServerMessage) -> Result<(), CommError> {
        let writers = self.writers.lock().expect("writers poisoned");
        let writer = writers.get(&peer).ok_or(CommError::UnknownPeer(peer))?;
        match writer.sender.try_send(WriterCommand::Message(message)) {
            Ok(()) => Ok(()),
            Err(TrySendError::Full(_)) => {
                let _ = writer.shutdown.shutdown(std::net::Shutdown::Both);
                Err(CommError::QueueFull(peer))
            }
            Err(TrySendError::Disconnected(_)) => Err(CommError::Disconnected),
        }
    }

    pub fn send_group<I>(&self, peers: I, message: ServerMessage) -> DeliveryReport
    where
        I: IntoIterator<Item = PeerId>,
    {
        let mut report = DeliveryReport::default();
        let mut seen = HashSet::new();
        for peer in peers {
            if seen.insert(peer) {
                match self.send_to(peer, message.clone()) {
                    Ok(()) => report.queued.push(peer),
                    Err(_) => report.failed.push(peer),
                }
            }
        }
        report
    }

    pub fn broadcast(&self, message: ServerMessage) -> DeliveryReport {
        let peers: Vec<_> = self
            .writers
            .lock()
            .expect("writers poisoned")
            .keys()
            .copied()
            .collect();
        self.send_group(peers, message)
    }

    pub fn disconnect(&self, peer: PeerId, reason: impl Into<String>) {
        if let Some(writer) = self.writers.lock().expect("writers poisoned").get(&peer) {
            let reason = reason.into();
            if writer
                .sender
                .try_send(WriterCommand::Close(reason))
                .is_err()
            {
                let _ = writer.shutdown.shutdown(std::net::Shutdown::Both);
            }
        }
    }

    /// Lobby code may release a UID after a player departs. In-game code should
    /// retain the reservation so a second process cannot assume that identity.
    pub fn release_uid(&self, uid: u32) {
        self.reserved_uids
            .lock()
            .expect("UID reservations poisoned")
            .remove(&uid);
    }

    pub fn connected_peers(&self) -> Vec<PeerId> {
        self.writers
            .lock()
            .expect("writers poisoned")
            .keys()
            .copied()
            .collect()
    }
}

impl Drop for ServerTransport {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::Release);
        let writers = self.writers.lock().expect("writers poisoned");
        for writer in writers.values() {
            let _ = writer.shutdown.shutdown(std::net::Shutdown::Both);
        }
        drop(writers);
        if let Some(handle) = self.accept_thread.take() {
            let _ = handle.join();
        }
        if let Ok(metadata) = fs::symlink_metadata(&self.path)
            && metadata.dev() == self.socket_identity.device
            && metadata.ino() == self.socket_identity.inode
        {
            let _ = fs::remove_file(&self.path);
        }
    }
}

fn peer_credentials(stream: &UnixStream) -> Result<PeerCredentials, CommError> {
    let credentials =
        nix::sys::socket::getsockopt(stream, nix::sys::socket::sockopt::PeerCredentials)
            .map_err(|error| io::Error::from_raw_os_error(error as i32))?;
    Ok(PeerCredentials {
        uid: credentials.uid(),
        gid: credentials.gid(),
        pid: credentials.pid(),
    })
}

fn reject_stream(mut stream: UnixStream, reason: &str, write_timeout: Duration) {
    let _ = write_socket_message(
        &mut stream,
        &ServerMessage::Error {
            text: reason.into(),
        },
        write_timeout,
    );
    let _ = stream.shutdown(std::net::Shutdown::Both);
}

#[allow(clippy::too_many_arguments)]
fn handle_connection(
    mut stream: UnixStream,
    peer: PeerId,
    server_uid: u32,
    limits: TransportLimits,
    incoming: SyncSender<ServerEvent>,
    lifecycle: mpsc::Sender<LifecycleNotice>,
    writers: Arc<Mutex<HashMap<PeerId, WriterHandle>>>,
    reserved_uids: Arc<Mutex<HashSet<u32>>>,
    handshake: IncompleteHandshake,
) {
    let credentials = match peer_credentials(&stream) {
        Ok(credentials) => credentials,
        Err(error) => {
            reject_stream(stream, &error.to_string(), limits.write_timeout);
            return;
        }
    };
    if limits.reject_root && credentials.uid == 0 {
        reject_stream(stream, "root clients are not allowed", limits.write_timeout);
        return;
    }
    if limits.reject_server_uid && credentials.uid == server_uid {
        reject_stream(
            stream,
            "the server account cannot join as a player",
            limits.write_timeout,
        );
        return;
    }

    let hello: ClientMessage = match read_socket_message(
        &mut stream,
        Some(limits.handshake_timeout),
        limits.frame_timeout,
    ) {
        Ok(message) => message,
        Err(error) => {
            reject_stream(stream, &error.to_string(), limits.write_timeout);
            return;
        }
    };
    let name = match hello {
        ClientMessage::Hello { version, name } if version == PROTOCOL_VERSION => name,
        ClientMessage::Hello { version, .. } => {
            reject_stream(
                stream,
                &format!("protocol version {version} is incompatible with {PROTOCOL_VERSION}"),
                limits.write_timeout,
            );
            return;
        }
        _ => {
            reject_stream(
                stream,
                "the first message must be hello",
                limits.write_timeout,
            );
            return;
        }
    };

    if limits.require_unique_uid {
        let mut uids = reserved_uids.lock().expect("UID reservations poisoned");
        if !uids.insert(credentials.uid) {
            drop(uids);
            reject_stream(
                stream,
                "this Linux UID is already reserved by a player",
                limits.write_timeout,
            );
            return;
        }
    }
    drop(handshake);

    let mut writer_stream = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    let shutdown = match stream.try_clone() {
        Ok(stream) => stream,
        Err(_) => return,
    };
    let (writer_tx, writer_rx) = mpsc::sync_channel(limits.outgoing_per_peer);
    let reported = Arc::new(AtomicBool::new(false));
    writers.lock().expect("writers poisoned").insert(
        peer,
        WriterHandle {
            sender: writer_tx.clone(),
            shutdown,
        },
    );

    let writer_lifecycle = lifecycle.clone();
    let writer_reported = Arc::clone(&reported);
    let write_timeout = limits.write_timeout;
    let _ = thread::Builder::new()
        .name(format!("salem-write-{}", peer.0))
        .spawn(move || {
            while let Ok(command) = writer_rx.recv() {
                let result = match command {
                    WriterCommand::Message(message) => {
                        write_socket_message(&mut writer_stream, &message, write_timeout)
                    }
                    WriterCommand::Close(reason) => {
                        let _ = write_socket_message(
                            &mut writer_stream,
                            &ServerMessage::Closing {
                                reason: reason.clone(),
                            },
                            write_timeout,
                        );
                        let _ = writer_stream.shutdown(std::net::Shutdown::Both);
                        if !writer_reported.swap(true, Ordering::AcqRel) {
                            let _ = writer_lifecycle.send(LifecycleNotice { peer, reason });
                        }
                        return;
                    }
                };
                if let Err(error) = result {
                    let _ = writer_stream.shutdown(std::net::Shutdown::Both);
                    if !writer_reported.swap(true, Ordering::AcqRel) {
                        let _ = writer_lifecycle.send(LifecycleNotice {
                            peer,
                            reason: error.to_string(),
                        });
                    }
                    return;
                }
            }
        });

    if writer_tx
        .send(WriterCommand::Message(ServerMessage::Welcome {
            version: PROTOCOL_VERSION,
            peer_id: peer.0,
            text: format!("connected as IPC peer {}", peer.0),
        }))
        .is_err()
    {
        return;
    }
    if incoming
        .try_send(ServerEvent::Connected {
            peer,
            credentials,
            name,
        })
        .is_err()
    {
        let _ = stream.shutdown(std::net::Shutdown::Both);
        return;
    }

    loop {
        match read_socket_message(&mut stream, None, limits.frame_timeout) {
            Ok(message @ ClientMessage::Command { .. }) => {
                if incoming
                    .try_send(ServerEvent::Message { peer, message })
                    .is_err()
                {
                    let _ = stream.shutdown(std::net::Shutdown::Both);
                    report_disconnect(&lifecycle, &reported, peer, "incoming queue is full");
                    return;
                }
            }
            Ok(ClientMessage::Close) => {
                report_disconnect(&lifecycle, &reported, peer, "client closed normally");
                return;
            }
            Ok(ClientMessage::Hello { .. }) => {
                let _ = stream.shutdown(std::net::Shutdown::Both);
                report_disconnect(&lifecycle, &reported, peer, "unexpected message type");
                return;
            }
            Err(error) => {
                report_disconnect(&lifecycle, &reported, peer, &error.to_string());
                return;
            }
        }
    }
}

fn report_disconnect(
    lifecycle: &mpsc::Sender<LifecycleNotice>,
    reported: &AtomicBool,
    peer: PeerId,
    reason: &str,
) {
    if !reported.swap(true, Ordering::AcqRel) {
        let _ = lifecycle.send(LifecycleNotice {
            peer,
            reason: reason.into(),
        });
    }
}

/// Client-side connection with independent reader and writer threads.
pub struct ClientTransport {
    welcome: ServerMessage,
    outgoing: SyncSender<ClientMessage>,
    incoming: Receiver<Result<ServerMessage, String>>,
    shutdown: UnixStream,
}

impl ClientTransport {
    pub fn connect(
        path: impl AsRef<Path>,
        name: impl Into<String>,
        limits: TransportLimits,
    ) -> Result<Self, CommError> {
        let mut stream = UnixStream::connect(path)?;
        write_socket_message(
            &mut stream,
            &ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                name: name.into(),
            },
            limits.write_timeout,
        )?;
        let welcome: ServerMessage = read_socket_message(
            &mut stream,
            Some(limits.handshake_timeout),
            limits.frame_timeout,
        )?;
        if let ServerMessage::Error { text } = &welcome {
            return Err(CommError::Protocol(text.clone()));
        }
        if !matches!(welcome, ServerMessage::Welcome { .. }) {
            return Err(CommError::Protocol(
                "server did not answer hello with welcome".into(),
            ));
        }

        let mut writer_stream = stream.try_clone()?;
        let mut reader_stream = stream.try_clone()?;
        let shutdown = stream;
        let (outgoing, writer_rx) = mpsc::sync_channel(limits.outgoing_per_peer);
        let (reader_tx, incoming) = mpsc::sync_channel(limits.outgoing_per_peer);
        let write_timeout = limits.write_timeout;
        let frame_timeout = limits.frame_timeout;
        let _ = thread::Builder::new()
            .name("salem-client-write".into())
            .spawn(move || {
                while let Ok(message) = writer_rx.recv() {
                    let closing = matches!(message, ClientMessage::Close);
                    if write_socket_message(&mut writer_stream, &message, write_timeout).is_err()
                        || closing
                    {
                        let _ = writer_stream.shutdown(std::net::Shutdown::Both);
                        return;
                    }
                }
            });
        let _ = thread::Builder::new()
            .name("salem-client-read".into())
            .spawn(move || {
                loop {
                    match read_socket_message(&mut reader_stream, None, frame_timeout) {
                        Ok(message) => {
                            let closing = matches!(message, ServerMessage::Closing { .. });
                            if reader_tx.send(Ok(message)).is_err() || closing {
                                return;
                            }
                        }
                        Err(error) => {
                            let _ = reader_tx.send(Err(error.to_string()));
                            return;
                        }
                    }
                }
            });
        Ok(Self {
            welcome,
            outgoing,
            incoming,
            shutdown,
        })
    }

    pub fn welcome(&self) -> &ServerMessage {
        &self.welcome
    }

    pub fn send(&self, message: ClientMessage) -> Result<(), CommError> {
        self.outgoing
            .try_send(message)
            .map_err(|error| match error {
                TrySendError::Full(_) => CommError::QueueFull(PeerId(0)),
                TrySendError::Disconnected(_) => CommError::Disconnected,
            })
    }

    pub fn recv_timeout(&self, timeout: Duration) -> Result<Option<ServerMessage>, CommError> {
        match self.incoming.recv_timeout(timeout) {
            Ok(Ok(message)) => Ok(Some(message)),
            Ok(Err(error)) => Err(CommError::Protocol(error)),
            Err(RecvTimeoutError::Timeout) => Ok(None),
            Err(RecvTimeoutError::Disconnected) => Err(CommError::Disconnected),
        }
    }

    pub fn close(&self) {
        if self.outgoing.try_send(ClientMessage::Close).is_err() {
            let _ = self.shutdown.shutdown(std::net::Shutdown::Both);
        }
    }
}

impl Drop for ClientTransport {
    fn drop(&mut self) {
        self.close();
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::io::Cursor;
    use std::os::unix::net::UnixStream;
    use std::thread;
    use std::time::{SystemTime, UNIX_EPOCH};

    fn socket_path(label: &str) -> PathBuf {
        let nonce = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .unwrap()
            .as_nanos();
        std::env::temp_dir().join(format!(
            "terminal-salem-{label}-{}-{nonce}.sock",
            std::process::id()
        ))
    }

    fn local_test_limits() -> TransportLimits {
        TransportLimits {
            reject_root: false,
            reject_server_uid: false,
            require_unique_uid: false,
            handshake_timeout: Duration::from_secs(1),
            frame_timeout: Duration::from_secs(1),
            write_timeout: Duration::from_secs(1),
            ..TransportLimits::default()
        }
    }

    fn connected_peer(server: &mut ServerTransport) -> (PeerId, PeerCredentials) {
        match server.recv_timeout(Duration::from_secs(2)).unwrap() {
            Some(ServerEvent::Connected {
                peer, credentials, ..
            }) => (peer, credentials),
            event => panic!("expected a connection event, got {event:?}"),
        }
    }

    struct SmallChunks<R> {
        inner: R,
        limit: usize,
    }

    impl<R: Read> Read for SmallChunks<R> {
        fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
            let count = buffer.len().min(self.limit);
            self.inner.read(&mut buffer[..count])
        }
    }

    #[test]
    fn frame_round_trip_survives_split_header_and_body_reads() {
        let original = ClientMessage::Command {
            request_id: 19,
            phase_id: 4,
            text: "/investigate 3".into(),
        };
        let mut bytes = Vec::new();
        write_message(&mut bytes, &original).unwrap();

        let mut split = SmallChunks {
            inner: Cursor::new(bytes),
            limit: 1,
        };
        let decoded: ClientMessage = read_message(&mut split).unwrap();
        assert_eq!(decoded, original);
    }

    #[test]
    fn multiple_frames_can_arrive_in_one_stream_read() {
        let mut bytes = Vec::new();
        write_message(&mut bytes, &ClientMessage::Close).unwrap();
        write_message(
            &mut bytes,
            &ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                name: "Mira".into(),
            },
        )
        .unwrap();

        let mut cursor = Cursor::new(bytes);
        let first: ClientMessage = read_message(&mut cursor).unwrap();
        assert_eq!(first, ClientMessage::Close);
        let second: ClientMessage = read_message(&mut cursor).unwrap();
        assert_eq!(
            second,
            ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                name: "Mira".into()
            }
        );
    }

    #[test]
    fn oversized_length_is_rejected_before_body_read() {
        let declared = (MAX_FRAME_BYTES as u32 + 1).to_be_bytes();
        let error = read_message::<_, ClientMessage>(&mut Cursor::new(declared)).unwrap_err();
        assert!(matches!(error, CommError::FrameTooLarge { .. }));
    }

    #[test]
    fn invalid_json_is_rejected() {
        let mut bytes = Vec::from(8_u32.to_be_bytes());
        bytes.extend_from_slice(b"not-json");
        let error = read_message::<_, ClientMessage>(&mut Cursor::new(bytes)).unwrap_err();
        assert!(matches!(error, CommError::InvalidJson(_)));
    }

    #[test]
    fn eof_mid_frame_is_reported() {
        let mut bytes = Vec::from(20_u32.to_be_bytes());
        bytes.extend_from_slice(b"short");
        let error = read_message::<_, ClientMessage>(&mut Cursor::new(bytes)).unwrap_err();
        assert!(
            matches!(error, CommError::Io(ref io) if io.kind() == io::ErrorKind::UnexpectedEof)
        );
    }

    #[test]
    fn real_unix_stream_pair_exchanges_a_framed_request_and_response() {
        let (mut client, mut server) = UnixStream::pair().unwrap();
        let server_thread = thread::spawn(move || {
            let request: ClientMessage = read_message(&mut server).unwrap();
            assert!(matches!(request, ClientMessage::Hello { .. }));
            write_message(
                &mut server,
                &ServerMessage::Welcome {
                    version: PROTOCOL_VERSION,
                    peer_id: 1,
                    text: "connected over AF_UNIX/SOCK_STREAM".into(),
                },
            )
            .unwrap();
        });

        write_message(
            &mut client,
            &ClientMessage::Hello {
                version: PROTOCOL_VERSION,
                name: "Rowan".into(),
            },
        )
        .unwrap();
        let reply: ServerMessage = read_message(&mut client).unwrap();
        assert!(matches!(reply, ServerMessage::Welcome { peer_id: 1, .. }));
        server_thread.join().unwrap();
    }

    #[test]
    fn server_transport_routes_private_group_and_broadcast_messages() {
        let path = socket_path("routing");
        let limits = local_test_limits();
        let mut server = ServerTransport::bind(&path, limits.clone()).unwrap();
        let alice = ClientTransport::connect(&path, "Alice", limits.clone()).unwrap();
        let bob = ClientTransport::connect(&path, "Bob", limits.clone()).unwrap();
        let cara = ClientTransport::connect(&path, "Cara", limits).unwrap();
        let (alice_peer, credentials) = connected_peer(&mut server);
        let (bob_peer, _) = connected_peer(&mut server);
        let (cara_peer, _) = connected_peer(&mut server);
        assert_eq!(credentials.uid, nix::unistd::Uid::current().as_raw());

        let broadcast = ServerMessage::Event {
            sequence: 1,
            phase_id: 0,
            channel: "public".into(),
            text: "hello everyone".into(),
        };
        let report = server.broadcast(broadcast.clone());
        assert_eq!(report.queued.len(), 3);
        assert_eq!(
            alice.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(broadcast.clone())
        );
        assert_eq!(
            bob.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(broadcast.clone())
        );
        assert_eq!(
            cara.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(broadcast)
        );

        let group = ServerMessage::Event {
            sequence: 2,
            phase_id: 0,
            channel: "group".into(),
            text: "Alice and Bob only".into(),
        };
        let report = server.send_group([alice_peer, bob_peer, alice_peer], group.clone());
        assert_eq!(report.queued.len(), 2, "duplicate recipients are removed");
        assert_eq!(
            alice.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(group.clone())
        );
        assert_eq!(
            bob.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(group)
        );
        assert_eq!(cara.recv_timeout(Duration::from_millis(50)).unwrap(), None);

        let private = ServerMessage::Event {
            sequence: 3,
            phase_id: 0,
            channel: "private".into(),
            text: "for Cara".into(),
        };
        server.send_to(cara_peer, private.clone()).unwrap();
        assert_eq!(
            cara.recv_timeout(Duration::from_secs(1)).unwrap(),
            Some(private)
        );
    }

    #[test]
    fn duplicate_linux_uid_is_rejected() {
        let path = socket_path("duplicate-uid");
        let limits = TransportLimits {
            reject_root: false,
            reject_server_uid: false,
            require_unique_uid: true,
            handshake_timeout: Duration::from_secs(1),
            frame_timeout: Duration::from_secs(1),
            write_timeout: Duration::from_secs(1),
            ..TransportLimits::default()
        };
        let mut server = ServerTransport::bind(&path, limits.clone()).unwrap();
        let _first = ClientTransport::connect(&path, "First", limits.clone()).unwrap();
        let _ = connected_peer(&mut server);
        let second = ClientTransport::connect(&path, "Second", limits);
        assert!(matches!(second, Err(CommError::Protocol(message)) if message.contains("UID")));
    }

    #[test]
    fn client_disconnect_is_reported_on_reserved_lifecycle_channel() {
        let path = socket_path("disconnect");
        let limits = local_test_limits();
        let mut server = ServerTransport::bind(&path, limits.clone()).unwrap();
        let client = ClientTransport::connect(&path, "Alice", limits).unwrap();
        let (peer, _) = connected_peer(&mut server);
        client.close();
        loop {
            match server.recv_timeout(Duration::from_secs(2)).unwrap() {
                Some(ServerEvent::Disconnected { peer: actual, .. }) => {
                    assert_eq!(actual, peer);
                    break;
                }
                Some(_) => {}
                None => panic!("disconnect was not reported"),
            }
        }
    }
}
