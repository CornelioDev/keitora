// SPDX-License-Identifier: MIT OR Apache-2.0

//! The named-pipe server that the desktop app and the native messaging host
//! talk to ([`PIPE_NAME`], `docs/architecture.md`, "IPC protocol").
//!
//! Clients are untrusted ([`docs/threat-model.md`]), so the server:
//!
//! - creates the pipe with an explicit DACL, [`PIPE_SDDL`]: `SYSTEM` and the
//!   interactive user only;
//! - rejects remote clients (`PIPE_REJECT_REMOTE_CLIENTS`);
//! - creates the first instance with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so it
//!   fails to start if someone else already serves the name;
//! - checks every incoming frame with `keitora-ipc` ([`FrameReader`] and
//!   [`decode`]): a bad frame is answered with a typed error and never reaches
//!   the [`RequestHandler`], and the connection stays usable (ADR-0014);
//! - serves at most [`MAX_CONNECTIONS`] clients at once and slows a client
//!   that sends more than [`REQUESTS_PER_SECOND`] requests a second (after a
//!   burst of [`REQUEST_BURST`]) instead of failing it;
//! - never lets a client that doesn't read hurt the rest: what is waiting to
//!   be written to a connection is capped at [`MAX_PENDING_BYTES`], and a
//!   connection over the cap is closed.
//!
//! The server owns `Subscribe`: it answers [`Response::Subscribed`] itself
//! and sends every event given to [`PipeServer::publish`] to the connections
//! that subscribed. Every other valid request goes to the handler, which
//! decides the response. The handler is where the strict-mode rules live; the
//! server only guarantees that what reaches it is a valid request.
//!
//! The server runs on a thread of its own, with a single-threaded tokio
//! runtime; handler calls run on that runtime's blocking pool, so a slow one
//! doesn't stop the other connections. The pipe is in byte mode, because the
//! frames are lines of text and may be far longer than a pipe message.
//!
//! [`docs/threat-model.md`]: https://github.com/CornelioDev/keitora/blob/main/docs/threat-model.md

use std::collections::HashMap;
use std::io;
use std::sync::atomic::{AtomicU64, AtomicUsize, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError, mpsc};
use std::thread::JoinHandle;
use std::time::Duration;

use keitora_ipc::{
    Envelope, Event, FrameReader, Message, MessageId, ProtocolError, Request, Response, decode,
    encode,
};
use keitora_sys::named_pipe::create_server;
use keitora_sys::security::SecurityDescriptor;
use tokio::io::{AsyncReadExt, AsyncWriteExt, WriteHalf};
use tokio::net::windows::named_pipe::{NamedPipeServer, PipeMode, ServerOptions};
use tokio::runtime::Builder;
use tokio::sync::mpsc::{UnboundedReceiver, UnboundedSender, unbounded_channel};
use tokio::sync::{Notify, OwnedSemaphorePermit, Semaphore, watch};
use tokio::time::Instant;

/// The name of the service's pipe.
pub const PIPE_NAME: &str = r"\\.\pipe\keitora";

/// The pipe's DACL, in SDDL, protected against inheritance (`P`):
///
/// - `SYSTEM` (`SY`): full access;
/// - the interactive user (`IU`): `FRFW`, file read and write. Opening a pipe
///   for reading and writing needs `FILE_APPEND_DATA`, which on a pipe is
///   `FILE_CREATE_PIPE_INSTANCE`, so this right can't be narrowed; the first
///   instance's flag is what keeps the name from being taken over.
///
/// **Accepted risk (M1):** because of that right, the interactive user can
/// still create extra instances of the pipe, and so answer clients in the
/// service's place or use up instances ([`MAX_INSTANCES`] bounds the latter).
/// The fix is a narrower DACL (no `FILE_CREATE_PIPE_INSTANCE`) together with a
/// `keitora-sys` wrapper that opens the pipe with an explicit access mask,
/// because tokio's client always asks for `GENERIC_WRITE`. It is planned as a
/// task of its own before the desktop client (T-3.1) and the native messaging
/// host (T-4.1); see `docs/threat-model.md`.
///
/// No one else is granted anything: not Everyone, Authenticated Users,
/// Users, Anonymous nor Network. Administrators get no ACE of their own; they
/// can still take ownership, as they can of any object.
pub const PIPE_SDDL: &str = "D:P(A;;FA;;;SY)(A;;FRFW;;;IU)";

/// The most clients served at the same time. A connection beyond this is
/// closed right away. 64 is twice what the desktop app, the native messaging
/// host and a few tools need, and bounds memory: each connection may hold
/// up to a frame being read ([`keitora_ipc::MAX_FRAME_LEN`]) and
/// [`MAX_PENDING_BYTES`] waiting to be written.
pub const MAX_CONNECTIONS: usize = 64;

/// How many requests a connection may send in a burst before it is slowed.
pub const REQUEST_BURST: u32 = 100;

/// The sustained requests per second a connection may send. A client that
/// sends more is not failed: the server reads its next frame later, so the
/// pipe fills and the client waits. The UI polls far less than this.
pub const REQUESTS_PER_SECOND: u32 = 50;

/// The most bytes that may wait to be written to one connection, responses
/// and events together. A client that lets more pile up (by not reading) is
/// disconnected. It holds the largest frame twice.
pub const MAX_PENDING_BYTES: usize = 2 * keitora_ipc::MAX_FRAME_LEN;

/// How many instances of the pipe wait for a client at the same time. A
/// client that finds none gets `ERROR_PIPE_BUSY` until a new one is made, so
/// a few wait, and each is replaced as soon as a client takes it.
const LISTENERS: usize = 4;

/// The most instances of the pipe that may exist at once, set on every
/// instance: the connections being served ([`MAX_CONNECTIONS`]), the
/// [`LISTENERS`] waiting for a client, and up to [`LISTENERS`] more that exist
/// for a moment because the next instance is made before a refused client's
/// is dropped. Anything beyond that is refused by Windows, so the count
/// can't grow without bound.
const MAX_INSTANCES: usize = MAX_CONNECTIONS + 2 * LISTENERS;

/// Bytes read from a pipe at a time.
const READ_CHUNK: usize = 64 * 1024;

/// Size of the pipe's in and out buffers.
const PIPE_BUFFER: u32 = 64 * 1024;

/// Threads for handler calls.
const BLOCKING_THREADS: usize = 16;

/// How long a stopping server waits for handler calls in progress.
const SHUTDOWN_GRACE: Duration = Duration::from_secs(2);

/// How long to wait before trying again to create a pipe instance that failed.
const RETRY_DELAY: Duration = Duration::from_millis(500);

/// Answers the valid requests that reach the server, other than `Subscribe`.
///
/// Calls may come from several connections at once, on the threads of a pool,
/// and may block (for example on the database).
pub trait RequestHandler: Send + Sync + 'static {
    /// Handles one request that already passed `keitora-ipc`'s validation.
    fn handle(&self, request: Request) -> Response;
}

/// Why a [`PipeServer`] couldn't start.
#[derive(Debug, thiserror::Error)]
pub enum PipeError {
    /// The server's thread or runtime couldn't be created.
    #[error("couldn't start the pipe server's runtime")]
    Runtime(#[source] io::Error),
    /// The pipe's security descriptor couldn't be built.
    #[error("couldn't build the pipe's security descriptor")]
    Security(#[source] io::Error),
    /// The pipe couldn't be created, for instance because another process
    /// already serves its name.
    #[error("couldn't create the pipe {name}")]
    Create {
        /// The name of the pipe.
        name: String,
        /// Why.
        #[source]
        source: io::Error,
    },
}

/// A running pipe server. Dropping it stops it, like [`shutdown`](Self::shutdown).
#[derive(Debug)]
pub struct PipeServer {
    shared: Arc<Shared>,
    stop: watch::Sender<bool>,
    thread: Option<JoinHandle<()>>,
}

impl PipeServer {
    /// Creates the pipe `pipe_name` and serves it on a background thread
    /// until [`shutdown`](Self::shutdown) or drop. Production code passes
    /// [`PIPE_NAME`]; tests pass a name of their own.
    ///
    /// # Errors
    ///
    /// Returns [`PipeError`] if the runtime or the security descriptor can't
    /// be made, or if the pipe can't be created as its first instance, which
    /// is the case when its name is already served.
    pub fn start(pipe_name: &str, handler: Arc<dyn RequestHandler>) -> Result<Self, PipeError> {
        let shared = Arc::new(Shared::default());
        let (stop, stop_rx) = watch::channel(false);
        let (ready_tx, ready_rx) = mpsc::channel();
        let name = pipe_name.to_owned();
        let thread_shared = Arc::clone(&shared);
        let thread = std::thread::Builder::new()
            .name("keitora-pipe".to_owned())
            .spawn(move || run(&name, handler, thread_shared, stop_rx, &ready_tx))
            .map_err(PipeError::Runtime)?;
        match ready_rx.recv() {
            Ok(Ok(())) => Ok(Self {
                shared,
                stop,
                thread: Some(thread),
            }),
            Ok(Err(error)) => {
                let _ = thread.join();
                Err(error)
            }
            Err(_) => {
                let _ = thread.join();
                Err(PipeError::Runtime(io::Error::other(
                    "the pipe server's thread ended before it was ready",
                )))
            }
        }
    }

    /// Sends `event` to every subscribed connection, in the order of the
    /// calls. It never waits for a client: a connection with too much
    /// waiting to be written ([`MAX_PENDING_BYTES`]) is closed instead.
    pub fn publish(&self, event: Event) {
        let sequence = self.shared.next_event.fetch_add(1, Ordering::Relaxed);
        let Ok(id) = MessageId::new(&format!("ev-{sequence}")) else {
            return;
        };
        let envelope = Envelope::new(id, Message::Event(event));
        let bytes: Arc<[u8]> = match encode(&envelope) {
            Ok(bytes) => bytes.into(),
            Err(error) => {
                tracing::error!(%error, "couldn't encode an event");
                return;
            }
        };
        let mut subscribers = self.shared.subscribers();
        subscribers.retain(|_, outbox| outbox.push(Arc::clone(&bytes)));
    }

    /// Stops serving and closes every connection and the pipe.
    pub fn shutdown(mut self) {
        self.stop();
    }

    fn stop(&mut self) {
        let _ = self.stop.send(true);
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

impl Drop for PipeServer {
    fn drop(&mut self) {
        self.stop();
    }
}

/// State shared by the server's handle and its connections.
#[derive(Debug, Default)]
struct Shared {
    /// The connections that subscribed, by connection number.
    subscribers: Mutex<HashMap<u64, Arc<Outbox>>>,
    /// Numbers the events' ids.
    next_event: AtomicU64,
}

impl Shared {
    fn subscribers(&self) -> MutexGuard<'_, HashMap<u64, Arc<Outbox>>> {
        // A poisoned lock only means a thread panicked while holding it; the
        // map is still a valid map.
        self.subscribers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
    }
}

/// What waits to be written to one connection.
#[derive(Debug)]
struct Outbox {
    /// Frames for the connection's writer, in order.
    frames: UnboundedSender<Arc<[u8]>>,
    /// Bytes in `frames` that the writer hasn't finished writing.
    pending: AtomicUsize,
    /// Wakes the connection to close it.
    close: Notify,
}

impl Outbox {
    fn new() -> (Arc<Self>, UnboundedReceiver<Arc<[u8]>>) {
        let (frames, receiver) = unbounded_channel();
        let outbox = Arc::new(Self {
            frames,
            pending: AtomicUsize::new(0),
            close: Notify::new(),
        });
        (outbox, receiver)
    }

    /// Queues `frame`. Returns `false`, and closes the connection, if that
    /// would leave more than [`MAX_PENDING_BYTES`] waiting.
    fn push(&self, frame: Arc<[u8]>) -> bool {
        let length = frame.len();
        let before = self.pending.fetch_add(length, Ordering::AcqRel);
        if before.saturating_add(length) > MAX_PENDING_BYTES || self.frames.send(frame).is_err() {
            self.pending.fetch_sub(length, Ordering::AcqRel);
            self.close.notify_one();
            return false;
        }
        true
    }
}

/// A token bucket for the requests of one connection. It never refuses: it
/// says how long to wait before the next request is read.
#[derive(Debug)]
struct RateLimiter {
    /// Millionths of a request available now.
    tokens: u64,
    /// When `tokens` was last brought up to date.
    updated: Instant,
}

/// One request, in millionths.
const REQUEST_COST: u64 = 1_000_000;

impl RateLimiter {
    fn new(now: Instant) -> Self {
        Self {
            tokens: u64::from(REQUEST_BURST).saturating_mul(REQUEST_COST),
            updated: now,
        }
    }

    /// Takes one request's worth at `now`. Returns how long the caller must
    /// wait before using it: zero while the burst lasts.
    fn take(&mut self, now: Instant) -> Duration {
        let capacity = u64::from(REQUEST_BURST).saturating_mul(REQUEST_COST);
        let rate = u64::from(REQUESTS_PER_SECOND);
        let elapsed = u64::try_from(now.saturating_duration_since(self.updated).as_micros())
            .unwrap_or(u64::MAX);
        // `rate` requests a second is `rate` millionths of a request per
        // microsecond.
        self.tokens = self
            .tokens
            .saturating_add(elapsed.saturating_mul(rate))
            .min(capacity);
        self.updated = self.updated.max(now);
        if let Some(left) = self.tokens.checked_sub(REQUEST_COST) {
            self.tokens = left;
            return Duration::ZERO;
        }
        let missing = REQUEST_COST.saturating_sub(self.tokens);
        let wait = Duration::from_micros(missing.div_ceil(rate.max(1)));
        // After the wait the token is there and used; account for it now.
        self.tokens = 0;
        self.updated = self.updated.max(now).checked_add(wait).unwrap_or(now);
        wait
    }
}

/// The body of the server's thread.
fn run(
    name: &str,
    handler: Arc<dyn RequestHandler>,
    shared: Arc<Shared>,
    stop: watch::Receiver<bool>,
    ready: &mpsc::Sender<Result<(), PipeError>>,
) {
    let runtime = match Builder::new_current_thread()
        .enable_io()
        .enable_time()
        .max_blocking_threads(BLOCKING_THREADS)
        .build()
    {
        Ok(runtime) => runtime,
        Err(error) => {
            let _ = ready.send(Err(PipeError::Runtime(error)));
            return;
        }
    };
    runtime.block_on(async {
        let security = match SecurityDescriptor::from_sddl(PIPE_SDDL) {
            Ok(security) => security,
            Err(error) => {
                let _ = ready.send(Err(PipeError::Security(error)));
                return;
            }
        };
        // The first instance claims the name; the others only have to exist.
        let mut waiting = Vec::new();
        for index in 0..LISTENERS {
            match create_instance(name, &security, index == 0) {
                Ok(pipe) => waiting.push(pipe),
                Err(source) => {
                    let _ = ready.send(Err(PipeError::Create {
                        name: name.to_owned(),
                        source,
                    }));
                    return;
                }
            }
        }
        let _ = ready.send(Ok(()));
        let acceptor = Arc::new(Acceptor {
            name: name.to_owned(),
            security,
            handler,
            shared,
            slots: Arc::new(Semaphore::new(MAX_CONNECTIONS)),
            connections: AtomicU64::new(0),
            stop: stop.clone(),
        });
        for pipe in waiting {
            tokio::spawn(Arc::clone(&acceptor).accept_loop(pipe));
        }
        let mut stop = stop;
        stopped(&mut stop).await;
    });
    // Dropping the runtime drops every connection, which closes the pipes.
    runtime.shutdown_timeout(SHUTDOWN_GRACE);
}

/// One instance of the pipe, with the DACL and without remote clients.
fn create_instance(
    name: &str,
    security: &SecurityDescriptor,
    first: bool,
) -> io::Result<NamedPipeServer> {
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .pipe_mode(PipeMode::Byte)
        .access_inbound(true)
        .access_outbound(true)
        .reject_remote_clients(true)
        .max_instances(MAX_INSTANCES)
        .in_buffer_size(PIPE_BUFFER)
        .out_buffer_size(PIPE_BUFFER);
    create_server(&options, name, security)
}

/// What the tasks that wait for clients share.
struct Acceptor {
    name: String,
    security: SecurityDescriptor,
    handler: Arc<dyn RequestHandler>,
    shared: Arc<Shared>,
    /// One permit per connection being served.
    slots: Arc<Semaphore>,
    /// Numbers the connections.
    connections: AtomicU64,
    stop: watch::Receiver<bool>,
}

impl Acceptor {
    /// Waits for clients on `first` and on the instances it makes after it,
    /// and starts a task for each client, until the server stops.
    async fn accept_loop(self: Arc<Self>, first: NamedPipeServer) {
        let mut listener = Some(first);
        loop {
            let Some(waiting) = listener.take() else {
                // The last attempt to create an instance failed; try again
                // soon.
                let mut stop = self.stop.clone();
                tokio::select! {
                    () = stopped(&mut stop) => return,
                    () = tokio::time::sleep(RETRY_DELAY) => {}
                }
                listener = self.new_instance();
                continue;
            };
            let mut stop = self.stop.clone();
            let connected = tokio::select! {
                () = stopped(&mut stop) => return,
                result = waiting.connect() => result,
            };
            // The next instance is made before the client is handed over, so
            // the pipe is not left without this one waiting.
            listener = self.new_instance();
            if let Err(error) = connected {
                tracing::debug!(%error, "a client failed to connect");
                continue;
            }
            let Ok(permit) = Arc::clone(&self.slots).try_acquire_owned() else {
                tracing::warn!("too many connections, one was refused");
                continue;
            };
            let number = self.connections.fetch_add(1, Ordering::Relaxed);
            tokio::spawn(serve(
                number,
                waiting,
                permit,
                Arc::clone(&self.handler),
                Arc::clone(&self.shared),
                stop,
            ));
        }
    }

    fn new_instance(&self) -> Option<NamedPipeServer> {
        match create_instance(&self.name, &self.security, false) {
            Ok(pipe) => Some(pipe),
            Err(error) => {
                tracing::error!(%error, "couldn't create a pipe instance");
                None
            }
        }
    }
}

/// Completes when the server is told to stop (or is gone).
async fn stopped(stop: &mut watch::Receiver<bool>) {
    let _ = stop.wait_for(|stopping| *stopping).await;
}

/// Serves one client until it leaves, is closed, or the server stops.
async fn serve(
    number: u64,
    pipe: NamedPipeServer,
    permit: OwnedSemaphorePermit,
    handler: Arc<dyn RequestHandler>,
    shared: Arc<Shared>,
    mut stop: watch::Receiver<bool>,
) {
    let (mut reader, writer) = tokio::io::split(pipe);
    let (outbox, frames) = Outbox::new();
    let writer = tokio::spawn(write_frames(writer, frames, Arc::clone(&outbox)));

    let mut connection = Connection {
        number,
        outbox: Arc::clone(&outbox),
        handler,
        shared: Arc::clone(&shared),
    };
    let mut frames = FrameReader::new();
    let mut limiter = RateLimiter::new(Instant::now());
    let mut chunk = vec![0_u8; READ_CHUNK];
    'connection: loop {
        let count = tokio::select! {
            () = stopped(&mut stop) => break,
            () = outbox.close.notified() => break,
            read = reader.read(&mut chunk) => match read {
                Ok(0) | Err(_) => break,
                Ok(count) => count,
            },
        };
        let Some(bytes) = chunk.get(..count) else {
            break;
        };
        for item in frames.push(bytes) {
            let wait = limiter.take(Instant::now());
            if !wait.is_zero() {
                tokio::select! {
                    () = stopped(&mut stop) => break 'connection,
                    () = outbox.close.notified() => break 'connection,
                    () = tokio::time::sleep(wait) => {}
                }
            }
            if !connection.handle(item).await {
                break 'connection;
            }
        }
    }
    shared.subscribers().remove(&number);
    writer.abort();
    drop(permit);
}

/// What the reading side of one connection needs to answer frames.
struct Connection {
    number: u64,
    outbox: Arc<Outbox>,
    handler: Arc<dyn RequestHandler>,
    shared: Arc<Shared>,
}

impl Connection {
    /// Answers one frame. Returns `false` if the connection must close.
    async fn handle(&mut self, item: Result<Vec<u8>, ProtocolError>) -> bool {
        let (reply, subscribe) = match item {
            Err(error) => (error_envelope(error), false),
            Ok(frame) => match decode(&frame) {
                Err(rejected) => (Some(rejected.into_response()), false),
                Ok(envelope) => self.answer(envelope).await,
            },
        };
        let Some(reply) = reply else { return false };
        let sent = match encode(&reply) {
            Ok(bytes) => self.outbox.push(bytes.into()),
            Err(error) => {
                tracing::error!(%error, "couldn't encode a response");
                true
            }
        };
        // Subscribed only after the answer is queued, so the first event the
        // client gets comes after `Subscribed`.
        if sent && subscribe {
            self.shared
                .subscribers()
                .insert(self.number, Arc::clone(&self.outbox));
        }
        sent
    }

    /// The response to a decoded message, and whether it subscribes.
    async fn answer(&self, envelope: Envelope) -> (Option<Envelope>, bool) {
        let id = envelope.id().clone();
        let (response, subscribe) = match envelope.message() {
            Message::Request(Request::Subscribe) => (Response::Subscribed, true),
            Message::Request(request) => {
                let handler = Arc::clone(&self.handler);
                let request = request.clone();
                match tokio::task::spawn_blocking(move || handler.handle(request)).await {
                    Ok(response) => (response, false),
                    Err(_) => {
                        tracing::error!("the request handler panicked");
                        (
                            Response::Error {
                                error: ProtocolError::Internal,
                            },
                            false,
                        )
                    }
                }
            }
            // A client may only send requests.
            Message::Response(_) | Message::Event(_) => (
                Response::Error {
                    error: ProtocolError::Malformed,
                },
                false,
            ),
        };
        (
            Some(Envelope::new(id, Message::Response(response))),
            subscribe,
        )
    }
}

/// An error response for a frame that has no id to answer under.
fn error_envelope(error: ProtocolError) -> Option<Envelope> {
    let id = MessageId::new("unknown").ok()?;
    Some(Envelope::new(
        id,
        Message::Response(Response::Error { error }),
    ))
}

/// Writes queued frames to the client, in order.
async fn write_frames(
    mut writer: WriteHalf<NamedPipeServer>,
    mut frames: UnboundedReceiver<Arc<[u8]>>,
    outbox: Arc<Outbox>,
) {
    while let Some(frame) = frames.recv().await {
        let written = writer.write_all(&frame).await;
        outbox.pending.fetch_sub(frame.len(), Ordering::AcqRel);
        if written.is_err() {
            outbox.close.notify_one();
            return;
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    fn frame(length: usize) -> Arc<[u8]> {
        vec![0_u8; length].into()
    }

    #[test]
    fn the_burst_is_free_then_requests_wait() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(start);
        for _ in 0..REQUEST_BURST {
            assert_eq!(limiter.take(start), Duration::ZERO);
        }
        let wait = limiter.take(start);
        assert_eq!(
            wait,
            Duration::from_micros(1_000_000 / u64::from(REQUESTS_PER_SECOND))
        );
    }

    #[test]
    fn waits_add_up_at_the_sustained_rate() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(start);
        let mut total = Duration::ZERO;
        for _ in 0..REQUEST_BURST {
            limiter.take(start);
        }
        // The caller honors each wait before asking again.
        let mut now = start;
        for _ in 0..REQUESTS_PER_SECOND {
            let wait = limiter.take(now);
            total += wait;
            now += wait;
        }
        assert_eq!(total, Duration::from_secs(1));
    }

    #[test]
    fn tokens_come_back_with_time_up_to_the_burst() {
        let start = Instant::now();
        let mut limiter = RateLimiter::new(start);
        for _ in 0..REQUEST_BURST {
            limiter.take(start);
        }
        let later = start + Duration::from_secs(3600);
        for _ in 0..REQUEST_BURST {
            assert_eq!(limiter.take(later), Duration::ZERO);
        }
        assert!(!limiter.take(later).is_zero());
    }

    #[test]
    fn an_outbox_over_its_cap_refuses_and_closes() {
        let (outbox, mut frames) = Outbox::new();
        assert!(outbox.push(frame(MAX_PENDING_BYTES)));
        assert!(!outbox.push(frame(1)));
        assert_eq!(outbox.pending.load(Ordering::SeqCst), MAX_PENDING_BYTES);
        assert_eq!(
            frames.try_recv().expect("the first frame").len(),
            MAX_PENDING_BYTES
        );
        assert!(frames.try_recv().is_err());
    }

    #[test]
    fn an_outbox_whose_writer_is_gone_refuses() {
        let (outbox, frames) = Outbox::new();
        drop(frames);
        assert!(!outbox.push(frame(10)));
        assert_eq!(outbox.pending.load(Ordering::SeqCst), 0);
    }

    #[test]
    fn the_instance_limit_covers_connections_and_listeners() {
        assert_eq!(MAX_INSTANCES, 72);
    }

    #[test]
    fn the_security_descriptor_text_is_valid() {
        SecurityDescriptor::from_sddl(PIPE_SDDL).expect("PIPE_SDDL parses");
    }
}
