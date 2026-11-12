// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.4: the named-pipe server `\\.\pipe\keitora`, with
//! an explicit DACL and `PIPE_REJECT_REMOTE_CLIENTS`, many clients at once and
//! event subscriptions.
//!
//! The roadmap gives no acceptance line for this task, so the criteria come
//! from its title and from `docs/architecture.md` ("IPC protocol"),
//! `docs/threat-model.md` and ADR-0014:
//!
//! - C1 The server listens on `\\.\pipe\keitora` (`pipe::PIPE_NAME`).
//! - C2 The pipe has an explicit DACL: only `SYSTEM` and the interactive user
//!   (or the user who runs the service in `--console` mode) may be granted
//!   access; no Everyone, Authenticated Users, Users, Anonymous or Network.
//! - C3 Remote clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`). A real
//!   remote client can't be made in a test, so this is NOT covered here and
//!   is left to the security review (see the report of this task).
//! - C4 Many clients at once, each served independently and in order.
//! - C5 Event subscriptions: `Subscribe` is answered `Subscribed` by the
//!   server, and from then on the connection receives every published event;
//!   connections that didn't subscribe receive none.
//! - C6 Every incoming frame is checked with `keitora-ipc`: bad frames get a
//!   typed error response and never reach the handler, and the connection
//!   stays usable (ADR-0014).
//! - C7 One client can't hurt the others: a flood, a client that never reads
//!   its events, or one that disconnects in the middle of a frame.
//! - C8 The server owns the pipe name (it fails to start if the name is
//!   already served, so nobody can squat on it) and stops cleanly.
//!
//! # The API these tests expect (module `keitora_service::pipe`)
//!
//! ```text
//! pub const PIPE_NAME: &str = r"\\.\pipe\keitora";
//! pub trait RequestHandler: Send + Sync + 'static {
//!     fn handle(&self, request: keitora_ipc::Request) -> keitora_ipc::Response;
//! }
//! impl PipeServer {
//!     /// Creates the pipe `pipe_name` (the first instance, or fails) and
//!     /// serves it on background threads until `shutdown` or drop.
//!     pub fn start(pipe_name: &str, handler: Arc<dyn RequestHandler>)
//!         -> Result<PipeServer, E>;           // E: Debug
//!     /// Sends `event` to every subscribed connection; never blocks on a slow client.
//!     pub fn publish(&self, event: keitora_ipc::Event);
//!     /// Stops serving and closes every connection.
//!     pub fn shutdown(self);
//! }
//! ```
//!
//! The server answers `Subscribe` and every frame that fails to decode by
//! itself; the handler sees only valid requests other than `Subscribe`. The
//! pipe name is a parameter so that tests use a unique one and can run in
//! parallel. The tests need `keitora-ipc` as a dependency of
//! `keitora-service` and `tokio` (features `rt`, `net`, `io-util`, `time`)
//! as a dev-dependency or dependency, to write the test client.
//!
//! Not testable without a numeric limit the docs don't give: the rate limit
//! per connection (architecture.md) and the cap on connections (ADR-0014). The
//! tests only check that a flood doesn't starve other clients and that 32
//! simultaneous clients are all served.
//!
//! Test index:
//! - C1: `default_pipe_name_is_keitora`
//! - C2: `pipe_dacl_grants_only_system_and_the_interactive_user`
//! - C4: `client_gets_the_response_to_its_request_under_the_same_id`,
//!   `handler_receives_the_decoded_request`,
//!   `error_responses_of_the_handler_reach_the_client`,
//!   `pipelined_requests_are_answered_in_order`,
//!   `a_request_split_across_writes_is_reassembled`,
//!   `thirty_two_simultaneous_clients_are_all_served`,
//!   `clients_get_only_their_own_responses`,
//!   `a_client_can_reconnect_after_disconnecting`
//! - C5: `subscribe_is_answered_subscribed_without_calling_the_handler`,
//!   `subscribed_client_receives_published_events`,
//!   `every_subscriber_receives_each_event`,
//!   `unsubscribed_client_receives_no_events`,
//!   `events_published_before_subscribing_are_not_delivered`,
//!   `events_arrive_in_publish_order`,
//!   `events_have_valid_unique_ids`,
//!   `a_subscriber_can_still_send_requests`
//! - C6: `malformed_frame_gets_a_malformed_error_and_the_connection_survives`,
//!   `invalid_request_gets_a_validation_error_and_never_reaches_the_handler`,
//!   `unsupported_version_gets_an_error_response`,
//!   `oversized_frame_gets_frame_too_large_and_the_connection_survives`,
//!   `a_response_sent_as_a_request_is_not_executed`
//! - C7: `request_flood_does_not_starve_other_clients`,
//!   `subscriber_that_never_reads_does_not_block_publish_or_other_clients`,
//!   `client_that_disconnects_mid_frame_does_not_affect_others`,
//!   `client_that_connects_and_leaves_does_not_affect_others`,
//!   `disconnected_subscriber_does_not_break_publishing`
//! - C8: `second_server_on_the_same_pipe_name_fails_to_start`,
//!   `shutdown_closes_connections_and_removes_the_pipe`,
//!   `dropping_the_server_stops_it`

#![cfg(test)]

use std::collections::{HashSet, VecDeque};
use std::future::Future;
use std::io;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, AtomicUsize, Ordering};
use std::time::{Duration, Instant};

use keitora_core::engine::LockKind;
use keitora_ipc::{
    Envelope, Event, FrameReader, LockReason, MAX_FRAME_LEN, Message, MessageId, Outcome,
    PageRequest, Phase, Request, Response, SessionState, decode, encode,
};
use keitora_service::pipe::{PIPE_NAME, PipeServer, RequestHandler};
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{ClientOptions, NamedPipeClient};

const WAIT: Duration = Duration::from_secs(10);
const QUIET: Duration = Duration::from_millis(600);

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A pipe name no other test (or run) uses.
fn unique_pipe_name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    format!(r"\\.\pipe\keitora-test-{}-{n}", std::process::id())
}

/// Runs `future` on a fresh runtime, with a hard limit so a hang fails.
fn run<F: Future>(future: F) -> F::Output {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime");
    runtime.block_on(async {
        tokio::time::timeout(Duration::from_secs(90), future)
            .await
            .expect("test took longer than 90 s")
    })
}

fn idle_state() -> SessionState {
    SessionState {
        phase: Phase::Idle,
        cycle: 1,
        remaining_ms: 0,
        paused: false,
        lock: None,
        blocklist_id: None,
    }
}

fn focus_state(remaining_ms: u64) -> SessionState {
    SessionState {
        phase: Phase::Focus,
        cycle: 1,
        remaining_ms,
        paused: false,
        lock: Some(LockKind::UntilTimerEnds),
        blocklist_id: Some("work".to_owned()),
    }
}

/// Records every request and answers by request type:
/// `GetState` -> `State`, `PauseSession` -> strict-lock error,
/// `GetBlocklist` -> `NotFound`, `ListBlocklists` -> an empty page, anything
/// else -> `Ok`.
#[derive(Default)]
struct TestHandler {
    calls: AtomicUsize,
    seen: std::sync::Mutex<Vec<Request>>,
}

impl TestHandler {
    fn calls(&self) -> usize {
        self.calls.load(Ordering::SeqCst)
    }
    fn seen(&self) -> Vec<Request> {
        self.seen.lock().expect("lock").clone()
    }
}

impl RequestHandler for TestHandler {
    fn handle(&self, request: Request) -> Response {
        self.calls.fetch_add(1, Ordering::SeqCst);
        self.seen.lock().expect("lock").push(request.clone());
        match request {
            Request::GetState => Response::State {
                state: idle_state(),
            },
            Request::PauseSession => Response::Error {
                error: keitora_ipc::ProtocolError::StrictSessionLocked {
                    reason: LockReason::UntilTimerEnds,
                },
            },
            Request::GetBlocklist { .. } => Response::Error {
                error: keitora_ipc::ProtocolError::NotFound,
            },
            Request::ListBlocklists { .. } => Response::Blocklists {
                items: Vec::new(),
                next_offset: None,
            },
            _ => Response::Ok,
        }
    }
}

/// A started server plus its handler and pipe name.
struct Fixture {
    name: String,
    handler: Arc<TestHandler>,
    server: PipeServer,
}

fn start() -> Fixture {
    let name = unique_pipe_name();
    let handler = Arc::new(TestHandler::default());
    let server = PipeServer::start(&name, handler.clone()).expect("the server starts");
    Fixture {
        name,
        handler,
        server,
    }
}

/// A test client speaking the protocol over the pipe.
struct Client {
    pipe: NamedPipeClient,
    reader: FrameReader,
    queue: VecDeque<Envelope>,
    closed: bool,
    next_id: u32,
}

impl Client {
    async fn connect(name: &str) -> Client {
        let deadline = Instant::now().checked_add(WAIT).expect("deadline");
        loop {
            match ClientOptions::new().open(name) {
                Ok(pipe) => {
                    return Client {
                        pipe,
                        reader: FrameReader::new(),
                        queue: VecDeque::new(),
                        closed: false,
                        next_id: 0,
                    };
                }
                // ERROR_PIPE_BUSY: every instance is taken for a moment.
                Err(error) if error.raw_os_error() == Some(231) && Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(10)).await;
                }
                Err(error) => panic!("could not connect to {name}: {error}"),
            }
        }
    }

    fn fresh_id(&mut self) -> String {
        self.next_id = self.next_id.checked_add(1).expect("id counter");
        format!("t-{}", self.next_id)
    }

    async fn send_raw(&mut self, bytes: &[u8]) {
        self.pipe.write_all(bytes).await.expect("write to the pipe");
    }

    /// Sends a request and returns its id.
    async fn send(&mut self, request: Request) -> String {
        let id = self.fresh_id();
        let envelope = Envelope::new(
            MessageId::new(&id).expect("valid id"),
            Message::Request(request),
        );
        self.send_raw(&encode(&envelope).expect("encode")).await;
        id
    }

    /// The next message from the service, or `None` after `limit` of silence
    /// or when the connection is closed.
    async fn recv_within(&mut self, limit: Duration) -> Option<Envelope> {
        let deadline = Instant::now().checked_add(limit).expect("deadline");
        loop {
            if let Some(envelope) = self.queue.pop_front() {
                return Some(envelope);
            }
            if self.closed {
                return None;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return None;
            }
            let mut buffer = vec![0_u8; 64 * 1024];
            match tokio::time::timeout(left, self.pipe.read(&mut buffer)).await {
                Err(_) => return None,
                Ok(Ok(0) | Err(_)) => self.closed = true,
                Ok(Ok(count)) => {
                    for frame in self.reader.push(buffer.get(..count).expect("in range")) {
                        let frame = frame.expect("the service sends frames within the limit");
                        let envelope = decode(&frame).expect("the service sends valid messages");
                        self.queue.push_back(envelope);
                    }
                }
            }
        }
    }

    async fn recv(&mut self) -> Envelope {
        self.recv_within(WAIT)
            .await
            .expect("the service should have sent a message")
    }

    /// The response to `id`, skipping nothing: the next message must be it.
    async fn response_to(&mut self, id: &str) -> Response {
        let envelope = self.recv().await;
        assert_eq!(
            envelope.id().as_str(),
            id,
            "response carries the request id"
        );
        match envelope.message() {
            Message::Response(response) => response.clone(),
            other => panic!("expected a response, got {other:?}"),
        }
    }

    /// Sends a request and waits for its response.
    async fn call(&mut self, request: Request) -> Response {
        let id = self.send(request).await;
        self.response_to(&id).await
    }

    async fn next_event(&mut self) -> Event {
        let envelope = self.recv().await;
        match envelope.message() {
            Message::Event(event) => event.clone(),
            other => panic!("expected an event, got {other:?}"),
        }
    }

    async fn subscribe(&mut self) {
        assert_eq!(self.call(Request::Subscribe).await, Response::Subscribed);
    }

    /// Asserts nothing arrives for a while.
    async fn assert_quiet(&mut self) {
        if let Some(envelope) = self.recv_within(QUIET).await {
            panic!("expected silence, got {envelope:?}");
        }
    }

    /// Whether the service closed this connection within `limit`.
    async fn is_closed_within(&mut self, limit: Duration) -> bool {
        let deadline = Instant::now().checked_add(limit).expect("deadline");
        while Instant::now() < deadline {
            if self.closed {
                return true;
            }
            let left = deadline.saturating_duration_since(Instant::now());
            let _ = self.recv_within(left).await;
        }
        self.closed
    }
}

fn error_code(response: &Response) -> &'static str {
    match response {
        Response::Error { error } => error.code(),
        other => panic!("expected an error response, got {other:?}"),
    }
}

// ---------------------------------------------------------------------------
// C1, C2 - name and DACL
// ---------------------------------------------------------------------------

#[test]
fn default_pipe_name_is_keitora() {
    assert_eq!(PIPE_NAME, r"\\.\pipe\keitora");
}

/// The SDDL of the pipe, read with PowerShell's `Get-Acl`.
fn pipe_sddl(name: &str) -> String {
    let script = format!("(Get-Acl -LiteralPath '{name}').Sddl");
    let output = Command::new("powershell")
        // A PSModulePath inherited from PowerShell 7 breaks module autoload in Windows PowerShell 5.1.
        .env_remove("PSModulePath")
        .args(["-NoProfile", "-NonInteractive", "-Command", &script])
        .output()
        .expect("powershell runs");
    assert!(
        output.status.success(),
        "Get-Acl failed: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    String::from_utf8(output.stdout)
        .expect("utf-8")
        .trim()
        .to_owned()
}

fn current_user_sid() -> String {
    let output = Command::new("whoami")
        .args(["/user", "/fo", "csv", "/nh"])
        .output()
        .expect("whoami runs");
    let text = String::from_utf8_lossy(&output.stdout);
    text.trim()
        .rsplit(',')
        .next()
        .expect("sid column")
        .trim_matches('"')
        .to_owned()
}

#[test]
fn pipe_dacl_grants_only_system_and_the_interactive_user() {
    let fixture = start();
    let sddl = pipe_sddl(&fixture.name);
    let dacl = sddl
        .split_once("D:")
        .map(|(_, rest)| rest.split("S:").next().unwrap_or(rest))
        .unwrap_or_else(|| panic!("the pipe has no DACL (SDDL: {sddl})"));
    let aces: Vec<&str> = dacl
        .split('(')
        .skip(1)
        .map(|ace| ace.trim_end_matches(')'))
        .collect();
    assert!(!aces.is_empty(), "the DACL is empty (SDDL: {sddl})");

    let user = current_user_sid();
    let mut allowed = HashSet::new();
    for ace in &aces {
        let fields: Vec<&str> = ace.split(';').collect();
        let kind = fields.first().copied().unwrap_or_default();
        let trustee = fields.last().copied().unwrap_or_default();
        if kind == "A" || kind == "OA" {
            allowed.insert(trustee.to_owned());
        }
    }
    assert!(
        allowed.contains("SY"),
        "SYSTEM must be granted access (SDDL: {sddl})"
    );
    assert!(
        allowed.contains("IU") || allowed.contains(&user),
        "the interactive user must be granted access (SDDL: {sddl})"
    );
    for trustee in &allowed {
        assert!(
            trustee == "SY" || trustee == "IU" || *trustee == user,
            "trustee {trustee} must not be granted access (SDDL: {sddl})"
        );
    }

    // The ACL query connected to the pipe; the server must still serve.
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        assert_eq!(
            client.call(Request::GetState).await,
            Response::State {
                state: idle_state()
            }
        );
    });
}

// ---------------------------------------------------------------------------
// C4 - requests and responses, many clients
// ---------------------------------------------------------------------------

#[test]
fn client_gets_the_response_to_its_request_under_the_same_id() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        let id = client.send(Request::GetState).await;
        let envelope = client.recv().await;
        assert_eq!(envelope.id().as_str(), id);
        assert_eq!(
            envelope.message(),
            &Message::Response(Response::State {
                state: idle_state()
            })
        );
    });
}

#[test]
fn handler_receives_the_decoded_request() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        let page = PageRequest::new(5, 10).expect("valid page");
        let response = client.call(Request::ListBlocklists { page }).await;
        assert_eq!(
            response,
            Response::Blocklists {
                items: Vec::new(),
                next_offset: None
            }
        );
        assert_eq!(
            client
                .call(Request::GetBlocklist {
                    id: "work".to_owned()
                })
                .await,
            Response::Error {
                error: keitora_ipc::ProtocolError::NotFound
            }
        );
    });
    assert_eq!(
        fixture.handler.seen(),
        vec![
            Request::ListBlocklists {
                page: PageRequest::new(5, 10).expect("valid page")
            },
            Request::GetBlocklist {
                id: "work".to_owned()
            }
        ]
    );
}

#[test]
fn error_responses_of_the_handler_reach_the_client() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        let response = client.call(Request::PauseSession).await;
        assert_eq!(error_code(&response), "strict_session_locked");
    });
}

#[test]
fn pipelined_requests_are_answered_in_order() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        // Five frames in a single write.
        let mut ids = Vec::new();
        let mut bytes = Vec::new();
        for n in 0..5 {
            let id = format!("p-{n}");
            let request = if n % 2 == 0 {
                Request::GetState
            } else {
                Request::PauseSession
            };
            let envelope =
                Envelope::new(MessageId::new(&id).expect("id"), Message::Request(request));
            bytes.extend(encode(&envelope).expect("encode"));
            ids.push(id);
        }
        client.send_raw(&bytes).await;
        for (n, id) in ids.iter().enumerate() {
            let response = client.response_to(id).await;
            if n % 2 == 0 {
                assert!(matches!(response, Response::State { .. }));
            } else {
                assert_eq!(error_code(&response), "strict_session_locked");
            }
        }
    });
}

#[test]
fn a_request_split_across_writes_is_reassembled() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        let envelope = Envelope::new(
            MessageId::new("split-1").expect("id"),
            Message::Request(Request::GetState),
        );
        let bytes = encode(&envelope).expect("encode");
        let (first, second) = bytes.split_at_checked(bytes.len() / 2).expect("split");
        client.send_raw(first).await;
        tokio::time::sleep(Duration::from_millis(150)).await;
        client.send_raw(second).await;
        assert!(matches!(
            client.response_to("split-1").await,
            Response::State { .. }
        ));
    });
}

#[test]
fn thirty_two_simultaneous_clients_are_all_served() {
    let fixture = start();
    run(async {
        let mut clients = Vec::new();
        for _ in 0..32 {
            clients.push(Client::connect(&fixture.name).await);
        }
        // All 32 are connected at the same time; each one gets its answer.
        for client in &mut clients {
            let id = client.send(Request::GetState).await;
            assert!(matches!(
                client.response_to(&id).await,
                Response::State { .. }
            ));
        }
        // And again, in the opposite order, on the same connections.
        for client in clients.iter_mut().rev() {
            assert!(matches!(
                client.call(Request::GetState).await,
                Response::State { .. }
            ));
        }
    });
    assert_eq!(fixture.handler.calls(), 64);
}

#[test]
fn clients_get_only_their_own_responses() {
    let fixture = start();
    run(async {
        let mut a = Client::connect(&fixture.name).await;
        let mut b = Client::connect(&fixture.name).await;
        let id_a = a.send(Request::GetState).await;
        let id_b = b.send(Request::PauseSession).await;
        assert!(matches!(a.response_to(&id_a).await, Response::State { .. }));
        assert_eq!(
            error_code(&b.response_to(&id_b).await),
            "strict_session_locked"
        );
        a.assert_quiet().await;
        b.assert_quiet().await;
    });
}

#[test]
fn a_client_can_reconnect_after_disconnecting() {
    let fixture = start();
    run(async {
        for _ in 0..5 {
            let mut client = Client::connect(&fixture.name).await;
            assert!(matches!(
                client.call(Request::GetState).await,
                Response::State { .. }
            ));
            drop(client);
        }
    });
}

// ---------------------------------------------------------------------------
// C5 - subscriptions
// ---------------------------------------------------------------------------

#[test]
fn subscribe_is_answered_subscribed_without_calling_the_handler() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        client.subscribe().await;
    });
    assert_eq!(fixture.handler.calls(), 0);
}

#[test]
fn subscribed_client_receives_published_events() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        client.subscribe().await;
        let event = Event::StateChanged {
            state: focus_state(1_500_000),
        };
        fixture.server.publish(event.clone());
        assert_eq!(client.next_event().await, event);
        let ended = Event::SessionEnded {
            outcome: Outcome::Completed,
        };
        fixture.server.publish(ended.clone());
        assert_eq!(client.next_event().await, ended);
        let changed = Event::BlocklistChanged {
            id: "work".to_owned(),
        };
        fixture.server.publish(changed.clone());
        assert_eq!(client.next_event().await, changed);
    });
}

#[test]
fn every_subscriber_receives_each_event() {
    let fixture = start();
    run(async {
        let mut clients = Vec::new();
        for _ in 0..8 {
            let mut client = Client::connect(&fixture.name).await;
            client.subscribe().await;
            clients.push(client);
        }
        let event = Event::StateChanged {
            state: focus_state(60_000),
        };
        fixture.server.publish(event.clone());
        for client in &mut clients {
            assert_eq!(client.next_event().await, event);
        }
    });
}

#[test]
fn unsubscribed_client_receives_no_events() {
    let fixture = start();
    run(async {
        let mut subscribed = Client::connect(&fixture.name).await;
        let mut other = Client::connect(&fixture.name).await;
        subscribed.subscribe().await;
        // `other` talks to the service but never subscribes.
        assert!(matches!(
            other.call(Request::GetState).await,
            Response::State { .. }
        ));
        let event = Event::StateChanged {
            state: focus_state(1000),
        };
        fixture.server.publish(event.clone());
        assert_eq!(subscribed.next_event().await, event);
        other.assert_quiet().await;
    });
}

#[test]
fn events_published_before_subscribing_are_not_delivered() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        fixture.server.publish(Event::BlocklistChanged {
            id: "before".to_owned(),
        });
        client.subscribe().await;
        let after = Event::BlocklistChanged {
            id: "after".to_owned(),
        };
        fixture.server.publish(after.clone());
        assert_eq!(client.next_event().await, after);
        client.assert_quiet().await;
    });
}

#[test]
fn events_arrive_in_publish_order() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        client.subscribe().await;
        for n in 0..50 {
            fixture.server.publish(Event::BlocklistChanged {
                id: format!("list-{n}"),
            });
        }
        for n in 0..50 {
            assert_eq!(
                client.next_event().await,
                Event::BlocklistChanged {
                    id: format!("list-{n}")
                }
            );
        }
    });
}

#[test]
fn events_have_valid_unique_ids() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        client.subscribe().await;
        for n in 0..20 {
            fixture.server.publish(Event::BlocklistChanged {
                id: format!("list-{n}"),
            });
        }
        let mut ids = HashSet::new();
        for _ in 0..20 {
            let envelope = client.recv().await;
            assert!(matches!(envelope.message(), Message::Event(_)));
            // `recv` already decoded it, so the id is valid; they differ.
            assert!(
                ids.insert(envelope.id().as_str().to_owned()),
                "duplicate id"
            );
        }
    });
}

#[test]
fn a_subscriber_can_still_send_requests() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        client.subscribe().await;
        assert!(matches!(
            client.call(Request::GetState).await,
            Response::State { .. }
        ));
        let event = Event::SessionEnded {
            outcome: Outcome::Stopped,
        };
        fixture.server.publish(event.clone());
        assert_eq!(client.next_event().await, event);
    });
}

// ---------------------------------------------------------------------------
// C6 - validation of incoming frames
// ---------------------------------------------------------------------------

#[test]
fn malformed_frame_gets_a_malformed_error_and_the_connection_survives() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        client.send_raw(b"this is not json\n").await;
        let envelope = client.recv().await;
        assert_eq!(envelope.id().as_str(), "unknown");
        match envelope.message() {
            Message::Response(response) => assert_eq!(error_code(response), "malformed"),
            other => panic!("expected a response, got {other:?}"),
        }
        assert!(matches!(
            client.call(Request::GetState).await,
            Response::State { .. }
        ));
    });
    assert_eq!(fixture.handler.calls(), 1);
}

#[test]
fn invalid_request_gets_a_validation_error_and_never_reaches_the_handler() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        // `page.limit` is 1 to 100; 101 is out of range.
        client
            .send_raw(
                br#"{"v":1,"id":"bad-1","kind":"request","payload":{"type":"list_blocklists","page":{"offset":0,"limit":101}}}
"#,
            )
            .await;
        let response = client.response_to("bad-1").await;
        assert_eq!(error_code(&response), "validation");
    });
    assert_eq!(fixture.handler.calls(), 0);
}

#[test]
fn unsupported_version_gets_an_error_response() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        client
            .send_raw(
                br#"{"v":99,"id":"ver-1","kind":"request","payload":{"type":"get_state"}}
"#,
            )
            .await;
        let response = client.response_to("ver-1").await;
        assert_eq!(error_code(&response), "unsupported_version");
    });
    assert_eq!(fixture.handler.calls(), 0);
}

#[test]
fn oversized_frame_gets_frame_too_large_and_the_connection_survives() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        // One line a byte longer than the limit, then a normal request. The
        // client writes in a task of its own: the server must keep reading
        // (and dropping) the line, not stall the writer.
        let long = vec![b'a'; MAX_FRAME_LEN + 1];
        client.send_raw(&long).await;
        client.send_raw(b"\n").await;
        let envelope = client.recv().await;
        match envelope.message() {
            Message::Response(response) => assert_eq!(error_code(response), "frame_too_large"),
            other => panic!("expected a response, got {other:?}"),
        }
        assert!(matches!(
            client.call(Request::GetState).await,
            Response::State { .. }
        ));
    });
    assert_eq!(fixture.handler.calls(), 1);
}

#[test]
fn a_response_sent_as_a_request_is_not_executed() {
    let fixture = start();
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        let fake = Envelope::new(
            MessageId::new("fake-1").expect("id"),
            Message::Response(Response::Ok),
        );
        client.send_raw(&encode(&fake).expect("encode")).await;
        let envelope = client.recv().await;
        match envelope.message() {
            Message::Response(response) => assert!(
                matches!(response, Response::Error { .. }),
                "a client may only send requests, got {response:?}"
            ),
            other => panic!("expected an error response, got {other:?}"),
        }
        let event = Envelope::new(
            MessageId::new("fake-2").expect("id"),
            Message::Event(Event::SessionEnded {
                outcome: Outcome::Stopped,
            }),
        );
        client.send_raw(&encode(&event).expect("encode")).await;
        let envelope = client.recv().await;
        assert!(matches!(
            envelope.message(),
            Message::Response(Response::Error { .. })
        ));
    });
    assert_eq!(fixture.handler.calls(), 0);
}

// ---------------------------------------------------------------------------
// C7 - one client can't hurt the others
// ---------------------------------------------------------------------------

#[test]
fn request_flood_does_not_starve_other_clients() {
    let fixture = start();
    run(async {
        let mut flooder = Client::connect(&fixture.name).await;
        let mut polite = Client::connect(&fixture.name).await;
        let envelope = Envelope::new(
            MessageId::new("flood").expect("id"),
            Message::Request(Request::GetState),
        );
        let frame = encode(&envelope).expect("encode");
        let mut burst = Vec::new();
        for _ in 0..5_000 {
            burst.extend_from_slice(&frame);
        }
        // The flooder writes (and never reads) in the background.
        let writer = tokio::spawn(async move {
            let _ = flooder.pipe.write_all(&burst).await;
            flooder
        });
        let started = Instant::now();
        for _ in 0..5 {
            assert!(matches!(
                polite.call(Request::GetState).await,
                Response::State { .. }
            ));
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "other clients were starved by the flood"
        );
        writer.abort();
    });
}

#[test]
fn subscriber_that_never_reads_does_not_block_publish_or_other_clients() {
    let fixture = start();
    run(async {
        let mut stuck = Client::connect(&fixture.name).await;
        stuck.subscribe().await;
        let mut healthy = Client::connect(&fixture.name).await;
        healthy.subscribe().await;
        // `stuck` never reads again. Enough events to fill any pipe buffer.
        let started = Instant::now();
        for n in 0..3_000 {
            fixture.server.publish(Event::BlocklistChanged {
                id: format!("{n:0>40}"),
            });
        }
        assert!(
            started.elapsed() < Duration::from_secs(5),
            "publish blocked on a client that doesn't read"
        );
        // The healthy subscriber still gets events, and requests still work.
        let marker = Event::BlocklistChanged {
            id: "marker".to_owned(),
        };
        fixture.server.publish(marker.clone());
        let deadline = Instant::now().checked_add(WAIT).expect("deadline");
        let mut got_marker = false;
        while Instant::now() < deadline {
            match healthy.recv_within(Duration::from_secs(1)).await {
                Some(envelope) if envelope.message() == &Message::Event(marker.clone()) => {
                    got_marker = true;
                    break;
                }
                Some(_) => {}
                None => break,
            }
        }
        // A slow client may be dropped by the server, but a healthy one that
        // reads must not lose the newest event.
        assert!(
            got_marker,
            "the healthy subscriber never got the last event"
        );
        let mut other = Client::connect(&fixture.name).await;
        assert!(matches!(
            other.call(Request::GetState).await,
            Response::State { .. }
        ));
    });
}

#[test]
fn client_that_disconnects_mid_frame_does_not_affect_others() {
    let fixture = start();
    run(async {
        let mut rude = Client::connect(&fixture.name).await;
        rude.send_raw(br#"{"v":1,"id":"half","kind":"requ"#).await;
        drop(rude);
        let mut client = Client::connect(&fixture.name).await;
        assert!(matches!(
            client.call(Request::GetState).await,
            Response::State { .. }
        ));
    });
    assert_eq!(fixture.handler.calls(), 1);
}

#[test]
fn client_that_connects_and_leaves_does_not_affect_others() {
    let fixture = start();
    run(async {
        for _ in 0..20 {
            drop(Client::connect(&fixture.name).await);
        }
        let mut client = Client::connect(&fixture.name).await;
        assert!(matches!(
            client.call(Request::GetState).await,
            Response::State { .. }
        ));
    });
}

#[test]
fn disconnected_subscriber_does_not_break_publishing() {
    let fixture = start();
    run(async {
        let mut gone = Client::connect(&fixture.name).await;
        gone.subscribe().await;
        let mut stays = Client::connect(&fixture.name).await;
        stays.subscribe().await;
        drop(gone);
        tokio::time::sleep(Duration::from_millis(200)).await;
        for n in 0..5 {
            let event = Event::BlocklistChanged {
                id: format!("list-{n}"),
            };
            fixture.server.publish(event.clone());
            assert_eq!(stays.next_event().await, event);
        }
    });
}

// ---------------------------------------------------------------------------
// C8 - ownership of the name and shutdown
// ---------------------------------------------------------------------------

#[test]
fn second_server_on_the_same_pipe_name_fails_to_start() {
    let fixture = start();
    let second = PipeServer::start(&fixture.name, Arc::new(TestHandler::default()));
    assert!(
        second.is_err(),
        "a second server must not share or take over a served pipe name"
    );
    // The first one is unharmed.
    run(async {
        let mut client = Client::connect(&fixture.name).await;
        assert!(matches!(
            client.call(Request::GetState).await,
            Response::State { .. }
        ));
    });
}

#[test]
fn shutdown_closes_connections_and_removes_the_pipe() {
    let fixture = start();
    let Fixture { name, server, .. } = fixture;
    run(async {
        let mut client = Client::connect(&name).await;
        client.subscribe().await;
        server.shutdown();
        assert!(
            client.is_closed_within(WAIT).await,
            "shutdown must close open connections"
        );
        let deadline = Instant::now().checked_add(WAIT).expect("deadline");
        loop {
            match ClientOptions::new().open(&name) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Ok(_) | Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                other => panic!("the pipe should be gone after shutdown: {other:?}"),
            }
        }
    });
}

#[test]
fn dropping_the_server_stops_it() {
    let name = unique_pipe_name();
    let server = PipeServer::start(&name, Arc::new(TestHandler::default())).expect("starts");
    drop(server);
    run(async {
        let deadline = Instant::now().checked_add(WAIT).expect("deadline");
        loop {
            match ClientOptions::new().open(&name) {
                Err(error) if error.kind() == io::ErrorKind::NotFound => break,
                Ok(_) | Err(_) if Instant::now() < deadline => {
                    tokio::time::sleep(Duration::from_millis(50)).await;
                }
                other => panic!("the pipe should be gone after drop: {other:?}"),
            }
        }
    });
    // The name is free again.
    let again = PipeServer::start(&name, Arc::new(TestHandler::default()));
    assert!(again.is_ok(), "the name should be reusable after a stop");
}
