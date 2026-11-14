// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for issue #133 (T-2.4 security findings M1 and M1b): the
//! pipe's DACL no longer grants the interactive user
//! `FILE_CREATE_PIPE_INSTANCE`, and clients open the pipe through a
//! `keitora-sys` wrapper that asks for an explicit, narrow access mask.
//!
//! The rest of the pipe server is tested in `acceptance_t2_4.rs`. The criteria
//! below come from issue #133, `docs/threat-model.md` (accepted risk M1, which
//! this issue closes) and `docs/architecture.md` ("IPC protocol").
//!
//! - C1 The interactive user's ACE on the pipe grants **exactly**
//!   `keitora_sys::named_pipe::CLIENT_ACCESS` (`0x12019b`): generic read plus
//!   `FILE_WRITE_DATA`, `FILE_WRITE_EA` and `FILE_WRITE_ATTRIBUTES`, and in
//!   particular not `FILE_APPEND_DATA` (`0x4`), which on a pipe is
//!   `FILE_CREATE_PIPE_INSTANCE`.
//! - C2 Nobody else is granted anything: no Everyone (`WD`), Authenticated
//!   Users (`AU`), Users (`BU`), Anonymous (`AN`) or Network (`NU`) ACE. Only
//!   `SYSTEM`, the interactive user and the SID the service itself runs under
//!   may appear.
//! - C3 (M1) With the DACL of a service running as `SYSTEM`, a process of the
//!   interactive user can **not** create another instance of the pipe name, so
//!   it can't answer clients in the service's place.
//! - C4 (M1b) Nor can it use up the pipe's instances: every attempt is
//!   refused, and the instance the server is waiting on is still free for a
//!   legitimate client afterwards.
//! - C5 The service can still create its own instances when it runs as the
//!   interactive user (`--console`, development): `PipeServer::start`
//!   succeeds, it keeps making new instances as clients come and go, and the
//!   SID passed to `pipe_sddl` is the one allowed to create instances.
//! - C6 The `keitora-sys` client wrapper opens the service's pipe with the
//!   narrow mask and round-trips a request, both directions included.
//! - C7 The wrapper really uses the mask it is given, so a client asks for no
//!   more than it needs: on a pipe with the `SYSTEM` DACL, `CLIENT_ACCESS`
//!   opens, while a mask beyond what the DACL grants (`FILE_ALL_ACCESS`) is
//!   denied. `acceptance_t2_4.rs` opens all of its clients through the
//!   wrapper, so the whole T-2.4 suite runs over it (change approved by the
//!   maintainer on 2026-11-13).
//! - C8 Error path of the wrapper: a pipe name that nobody serves is
//!   [`io::ErrorKind::NotFound`], not a panic.
//!
//! **Measured while these tests were written (Windows 11 26200)**
//!
//! Issue #133 says the narrow DACL needs the wrapper because tokio's client
//! always asks for `GENERIC_WRITE`, which contains `FILE_CREATE_PIPE_INSTANCE`.
//! That turns out not to be how Windows checks a *client* open: with the DACL
//! of C1 in place, `CreateFileW` on the pipe succeeds with `GENERIC_WRITE`,
//! with `GENERIC_READ | GENERIC_WRITE` (tokio's `ClientOptions`) and even with
//! `FILE_APPEND_DATA` alone, while `CreateNamedPipeW` for a second instance of
//! the same name is denied. The instance right is only checked on the
//! server-end create. So the DACL alone closes M1 and M1b, and the wrapper is
//! defense in depth plus a guard against that behavior differing on another
//! Windows build; there is therefore no test here asserting that tokio's
//! client is refused, because today it is not.
//!
//! **Not covered here**
//!
//! - A *second user account*: these tests run as one user, and creating
//!   another account and a logon session for it isn't something a test may do
//!   on the maintainer's machine. C3 and C4 instead use the DACL the service
//!   has in production (`pipe_sddl("S-1-5-18")`, service as `SYSTEM`) and
//!   attack it from the interactive user, which is exactly the M1 attacker;
//!   whether a different standard user, or a service account, is equally shut
//!   out follows from the DACL having no ACE for them (C2) and is left to the
//!   security review, as T-2.4's C3 leaves remote clients.
//! - An **elevated administrator**: Administrators get no ACE, but an
//!   administrator can take ownership of any object, which the threat model
//!   puts out of scope. If these tests are run from an elevated prompt, C3,
//!   C4 and C7 still expect the attempt to be denied, because taking
//!   ownership is a deliberate act and not what `CreateNamedPipeW` or
//!   `CreateFileW` do.
//! - The DACL of the pipe of a **really installed** service (owner `SYSTEM`):
//!   the tests check `pipe_sddl` for that SID and the live pipe of a server
//!   run by the interactive user, which is what a test host can start.
//!
//! # The API these tests expect
//!
//! In `keitora_sys::named_pipe`:
//!
//! ```text
//! /// Everything a Keitora pipe client needs and nothing more: generic read
//! /// (without `FILE_APPEND_DATA`) plus `FILE_WRITE_DATA`, `FILE_WRITE_EA`
//! /// and `FILE_WRITE_ATTRIBUTES`. `FILE_APPEND_DATA` is left out because on
//! /// a pipe it is `FILE_CREATE_PIPE_INSTANCE`.
//! pub const CLIENT_ACCESS: u32 = 0x0012_019b;
//!
//! /// Opens the client end of the existing pipe `name` with the access mask
//! /// `access`, overlapped, and hands it to tokio
//! /// (`NamedPipeClient::from_raw_handle`, with a `// SAFETY:` comment).
//! /// Must be called inside a tokio runtime. Returns the Win32 error:
//! /// `NotFound` if nobody serves `name`, `PermissionDenied` (5) if the DACL
//! /// doesn't grant `access`, and `ERROR_PIPE_BUSY` (231) if every instance
//! /// is taken.
//! pub fn open_client(name: &str, access: u32) -> io::Result<NamedPipeClient>;
//! ```
//!
//! In `keitora_service::pipe`, replacing the `PIPE_SDDL` constant, which these
//! tests expect to be gone (the DACL now depends on who runs the service):
//!
//! ```text
//! /// The pipe's DACL in SDDL, protected against inheritance (`P`):
//! /// `SYSTEM` full access, the interactive user `CLIENT_ACCESS` only, and,
//! /// when `service_sid` is not `SYSTEM` (that is, in `--console` mode), an
//! /// ACE for `service_sid` so the service can create its own instances.
//! pub fn pipe_sddl(service_sid: &str) -> String;
//! ```
//!
//! The service builds it with the SID of its own token, which needs a
//! `keitora-sys` wrapper of its own (for example
//! `keitora_sys::security::current_process_sid() -> io::Result<String>`); the
//! tests don't use that wrapper, they pass the SID they want. A failure to
//! read the SID belongs in `PipeError::Security`.
//!
//! Test index:
//! - C1: `client_access_is_read_and_write_without_create_pipe_instance`,
//!   `pipe_sddl_grants_the_interactive_user_exactly_the_client_mask`,
//!   `the_live_pipe_grants_the_interactive_user_exactly_the_client_mask`
//! - C2: `pipe_sddl_grants_nobody_but_system_the_user_and_the_service`,
//!   `the_live_pipe_grants_nobody_but_system_the_user_and_the_service`
//! - C3: `the_interactive_user_cannot_create_another_instance_of_a_system_pipe`
//! - C4: `refused_instance_attempts_leave_the_waiting_instance_free`
//! - C5: `the_service_sid_may_create_further_instances_of_its_own_pipe`,
//!   `a_server_run_by_the_interactive_user_keeps_serving_new_clients`
//! - C6: `the_client_wrapper_round_trips_a_request_over_the_service_pipe`
//! - C7: `the_client_wrapper_honors_the_access_mask_it_is_given`
//! - C8: `the_client_wrapper_reports_an_unserved_pipe_as_not_found`

#![cfg(test)]

use std::collections::HashSet;
use std::future::Future;
use std::io;
use std::process::Command;
use std::sync::Arc;
use std::sync::atomic::{AtomicU32, Ordering};
use std::time::Duration;

use keitora_ipc::{
    Envelope, FrameReader, Message, MessageId, Phase, Request, Response, SessionState, decode,
    encode,
};
use keitora_service::pipe::{PipeServer, RequestHandler, pipe_sddl};
use keitora_sys::named_pipe::{CLIENT_ACCESS, create_server, open_client};
use keitora_sys::security::SecurityDescriptor;
use tokio::io::{AsyncReadExt, AsyncWriteExt};
use tokio::net::windows::named_pipe::{NamedPipeClient, PipeMode, ServerOptions};

/// `S-1-5-18`, the SID the service runs under once it is installed.
const SYSTEM_SID: &str = "S-1-5-18";

/// `ERROR_ACCESS_DENIED`.
const ACCESS_DENIED: i32 = 5;

/// `FILE_APPEND_DATA`, which on a pipe is `FILE_CREATE_PIPE_INSTANCE`.
const FILE_CREATE_PIPE_INSTANCE: u32 = 0x0000_0004;

/// The mask the DACL must grant the interactive user, written out right by
/// right so that the number in the issue is checked against its meaning:
/// `READ_CONTROL`, `SYNCHRONIZE`, `FILE_READ_DATA`, `FILE_WRITE_DATA`,
/// `FILE_READ_EA`, `FILE_WRITE_EA`, `FILE_READ_ATTRIBUTES` and
/// `FILE_WRITE_ATTRIBUTES`.
const EXPECTED_CLIENT_ACCESS: u32 =
    0x0002_0000 | 0x0010_0000 | 0x0001 | 0x0002 | 0x0008 | 0x0010 | 0x0080 | 0x0100;

/// `FILE_ALL_ACCESS`, which SDDL writes `FA`.
const FILE_ALL_ACCESS: u32 = 0x001f_01ff;

/// Instances of a hand-made pipe; the value doesn't matter as long as every
/// instance of the same name is created with the same one.
const TEST_MAX_INSTANCES: usize = 72;

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

/// A pipe name no other test (or run) uses.
fn unique_pipe_name() -> String {
    static NEXT: AtomicU32 = AtomicU32::new(0);
    let n = NEXT.fetch_add(1, Ordering::SeqCst);
    format!(r"\\.\pipe\keitora-dacl-{}-{n}", std::process::id())
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

/// Answers `GetState` with the idle state and anything else with `Ok`.
struct TestHandler;

impl RequestHandler for TestHandler {
    fn handle(&self, request: Request) -> Response {
        match request {
            Request::GetState => Response::State {
                state: idle_state(),
            },
            _ => Response::Ok,
        }
    }
}

/// Starts a server on a name of its own, as the user running the tests, which
/// is what `--console` mode does.
fn start() -> (String, PipeServer) {
    let name = unique_pipe_name();
    let server = PipeServer::start(&name, Arc::new(TestHandler)).expect("the server starts");
    (name, server)
}

/// The settings every instance of a hand-made pipe is created with; they match
/// across instances, so a refused second instance is refused for its access
/// and not for a mismatch.
fn server_options(first: bool) -> ServerOptions {
    let mut options = ServerOptions::new();
    options
        .first_pipe_instance(first)
        .pipe_mode(PipeMode::Byte)
        .access_inbound(true)
        .access_outbound(true)
        .reject_remote_clients(true)
        .max_instances(TEST_MAX_INSTANCES)
        .in_buffer_size(64 * 1024)
        .out_buffer_size(64 * 1024);
    options
}

/// The SID of the user running the tests.
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

/// The SDDL of a live pipe, read with PowerShell's `Get-Acl`.
fn live_sddl(name: &str) -> String {
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

/// One access-allowed ACE: its rights and its trustee.
#[derive(Debug)]
struct Ace {
    rights: u32,
    trustee: String,
}

/// The bits an SDDL rights field stands for, whether it is hexadecimal
/// (`0x12019b`) or a string of two-letter codes (`FRFW`).
fn rights_mask(field: &str) -> u32 {
    if let Some(hex) = field
        .strip_prefix("0x")
        .or_else(|| field.strip_prefix("0X"))
    {
        return u32::from_str_radix(hex, 16).expect("a hexadecimal rights field");
    }
    const CODES: [(&str, u32); 16] = [
        ("GA", 0x1000_0000),
        ("GR", 0x8000_0000),
        ("GW", 0x4000_0000),
        ("GX", 0x2000_0000),
        ("FA", FILE_ALL_ACCESS),
        ("FR", 0x0012_0089),
        ("FW", 0x0012_0116),
        ("FX", 0x0012_00a0),
        ("SD", 0x0001_0000),
        ("RC", 0x0002_0000),
        ("WD", 0x0004_0000),
        ("WO", 0x0008_0000),
        ("CC", 0x0000_0001),
        ("DC", 0x0000_0002),
        ("LC", 0x0000_0004),
        ("SW", 0x0000_0008),
    ];
    let mut mask = 0_u32;
    for pair in field.as_bytes().chunks(2) {
        let code = std::str::from_utf8(pair).expect("an ASCII rights code");
        let bits = CODES
            .iter()
            .find(|(name, _)| *name == code)
            .map(|(_, bits)| *bits)
            .unwrap_or_else(|| panic!("unknown SDDL rights code {code} in {field}"));
        mask |= bits;
    }
    mask
}

/// The access-allowed ACEs of the DACL in `sddl`. Panics if it has no DACL or
/// if the DACL isn't protected against inheritance (`D:P`).
fn allowed_aces(sddl: &str) -> Vec<Ace> {
    let dacl = sddl
        .split_once("D:")
        .map(|(_, rest)| rest.split("S:").next().unwrap_or(rest))
        .unwrap_or_else(|| panic!("the DACL is missing (SDDL: {sddl})"));
    assert!(
        dacl.starts_with('P'),
        "the DACL must be protected against inheritance (SDDL: {sddl})"
    );
    let mut aces = Vec::new();
    for ace in dacl.split('(').skip(1) {
        let ace = ace.trim_end_matches(')');
        let fields: Vec<&str> = ace.split(';').collect();
        let kind = fields.first().copied().unwrap_or_default();
        if kind != "A" && kind != "OA" {
            continue;
        }
        let rights = fields.get(2).copied().unwrap_or_default();
        let trustee = fields.last().copied().unwrap_or_default();
        aces.push(Ace {
            rights: rights_mask(rights),
            trustee: trustee.to_owned(),
        });
    }
    assert!(!aces.is_empty(), "the DACL is empty (SDDL: {sddl})");
    aces
}

/// Checks C1 on `sddl`: the interactive user is granted exactly
/// `CLIENT_ACCESS` and nothing beyond it.
fn assert_interactive_mask(sddl: &str) {
    let aces = allowed_aces(sddl);
    let interactive: Vec<&Ace> = aces.iter().filter(|ace| ace.trustee == "IU").collect();
    assert_eq!(
        interactive.len(),
        1,
        "the interactive user must have exactly one ACE (SDDL: {sddl})"
    );
    for ace in interactive {
        assert_eq!(
            ace.rights, CLIENT_ACCESS,
            "the interactive user's ACE must grant exactly 0x{CLIENT_ACCESS:x} (SDDL: {sddl})"
        );
        assert_eq!(
            ace.rights & FILE_CREATE_PIPE_INSTANCE,
            0,
            "the interactive user must not be granted FILE_CREATE_PIPE_INSTANCE (SDDL: {sddl})"
        );
    }
}

/// Checks C2 on `sddl`: only `SYSTEM`, the interactive user and the SID the
/// service runs under are granted anything.
fn assert_no_other_trustee(sddl: &str, service_sid: &str) {
    let trustees: HashSet<String> = allowed_aces(sddl)
        .into_iter()
        .map(|ace| ace.trustee)
        .collect();
    let allowed = ["SY", "IU", SYSTEM_SID, service_sid];
    for trustee in &trustees {
        assert!(
            allowed.contains(&trustee.as_str()),
            "trustee {trustee} must not be granted access (SDDL: {sddl})"
        );
    }
    assert!(
        trustees.contains("SY") || trustees.contains(SYSTEM_SID),
        "SYSTEM must be granted access (SDDL: {sddl})"
    );
    assert!(
        trustees.contains("IU"),
        "the interactive user must be granted access (SDDL: {sddl})"
    );
}

/// Sends `request` over `pipe` and returns the response to it.
async fn round_trip(pipe: &mut NamedPipeClient, id: &str, request: Request) -> Response {
    let envelope = Envelope::new(
        MessageId::new(id).expect("a valid id"),
        Message::Request(request),
    );
    pipe.write_all(&encode(&envelope).expect("encode"))
        .await
        .expect("write to the pipe");
    let mut reader = FrameReader::new();
    let mut buffer = vec![0_u8; 64 * 1024];
    loop {
        let count = pipe.read(&mut buffer).await.expect("read from the pipe");
        assert!(count > 0, "the service closed the connection");
        // One request at a time, so the first frame is the answer to it.
        let frames = reader.push(buffer.get(..count).expect("in range"));
        let Some(frame) = frames.into_iter().next() else {
            continue;
        };
        let frame = frame.expect("a frame within the limit");
        let decoded = decode(&frame).expect("a valid message");
        assert_eq!(decoded.id().as_str(), id, "the response carries the id");
        match decoded.message() {
            Message::Response(response) => return response.clone(),
            other => panic!("expected a response, got {other:?}"),
        }
    }
}

// ---------------------------------------------------------------------------
// C1, C2 - the mask the DACL grants
// ---------------------------------------------------------------------------

#[test]
fn client_access_is_read_and_write_without_create_pipe_instance() {
    assert_eq!(
        CLIENT_ACCESS, 0x0012_019b,
        "the mask issue #133 asks for is 0x12019b"
    );
    assert_eq!(
        CLIENT_ACCESS, EXPECTED_CLIENT_ACCESS,
        "0x12019b is generic read plus write data, write EA and write attributes"
    );
    assert_eq!(
        CLIENT_ACCESS & FILE_CREATE_PIPE_INSTANCE,
        0,
        "the client mask must not contain FILE_CREATE_PIPE_INSTANCE"
    );
}

#[test]
fn pipe_sddl_grants_the_interactive_user_exactly_the_client_mask() {
    // As the installed service runs, and as it runs with `--console`.
    let user = current_user_sid();
    for sid in [SYSTEM_SID, user.as_str()] {
        let sddl = pipe_sddl(sid);
        assert_interactive_mask(&sddl);
        SecurityDescriptor::from_sddl(&sddl).expect("Windows parses the pipe's SDDL");
    }
}

#[test]
fn pipe_sddl_grants_nobody_but_system_the_user_and_the_service() {
    let user = current_user_sid();
    for sid in [SYSTEM_SID, user.as_str()] {
        let sddl = pipe_sddl(sid);
        assert_no_other_trustee(&sddl, sid);
        for ace in allowed_aces(&sddl) {
            if ace.trustee == "SY" || ace.trustee == SYSTEM_SID {
                assert_eq!(
                    ace.rights & FILE_ALL_ACCESS,
                    FILE_ALL_ACCESS,
                    "SYSTEM keeps full access (SDDL: {sddl})"
                );
            }
        }
    }
}

#[test]
fn the_live_pipe_grants_the_interactive_user_exactly_the_client_mask() {
    let (name, server) = start();
    let sddl = live_sddl(&name);
    assert_interactive_mask(&sddl);
    drop(server);
}

#[test]
fn the_live_pipe_grants_nobody_but_system_the_user_and_the_service() {
    let (name, server) = start();
    let sddl = live_sddl(&name);
    assert_no_other_trustee(&sddl, &current_user_sid());
    for denied in ["WD", "AU", "BU", "AN", "NU", "BA", "WR"] {
        assert!(
            !allowed_aces(&sddl).iter().any(|ace| ace.trustee == denied),
            "{denied} must not be granted access (SDDL: {sddl})"
        );
    }
    drop(server);
}

// ---------------------------------------------------------------------------
// C3, C4 - M1 and M1b: no extra instances for the interactive user
// ---------------------------------------------------------------------------

#[test]
fn the_interactive_user_cannot_create_another_instance_of_a_system_pipe() {
    let name = unique_pipe_name();
    run(async {
        // The DACL of the service as it is installed: owner `SYSTEM`.
        let security =
            SecurityDescriptor::from_sddl(&pipe_sddl(SYSTEM_SID)).expect("valid pipe SDDL");
        let first =
            create_server(&server_options(true), &name, &security).expect("the first instance");
        // A process of the interactive user tries to serve the same name.
        match create_server(&server_options(false), &name, &security) {
            Ok(_) => panic!(
                "the interactive user created a second instance of {name}: \
                 it can impersonate the service (M1)"
            ),
            Err(error) => assert_eq!(
                error.raw_os_error(),
                Some(ACCESS_DENIED),
                "creating an instance must be denied, got {error}"
            ),
        }
        drop(first);
    });
}

#[test]
fn refused_instance_attempts_leave_the_waiting_instance_free() {
    let name = unique_pipe_name();
    run(async {
        let security =
            SecurityDescriptor::from_sddl(&pipe_sddl(SYSTEM_SID)).expect("valid pipe SDDL");
        let first =
            create_server(&server_options(true), &name, &security).expect("the first instance");
        // M1b: filling the instances is refused, attempt after attempt.
        for attempt in 0..8 {
            match create_server(&server_options(false), &name, &security) {
                Ok(_) => panic!("attempt {attempt} created an instance of {name} (M1b)"),
                Err(error) => assert_eq!(
                    error.raw_os_error(),
                    Some(ACCESS_DENIED),
                    "attempt {attempt} must be denied, got {error}"
                ),
            }
        }
        // The instance the server waits on is untouched: a client still gets it.
        let client = open_client(&name, CLIENT_ACCESS).expect("a narrow client still connects");
        first.connect().await.expect("the server accepts it");
        drop(client);
    });
}

// ---------------------------------------------------------------------------
// C5 - the service still creates its own instances
// ---------------------------------------------------------------------------

#[test]
fn the_service_sid_may_create_further_instances_of_its_own_pipe() {
    let name = unique_pipe_name();
    let user = current_user_sid();
    run(async {
        // `--console` mode: the service's SID is the interactive user's.
        let security = SecurityDescriptor::from_sddl(&pipe_sddl(&user)).expect("valid pipe SDDL");
        let first =
            create_server(&server_options(true), &name, &security).expect("the first instance");
        let second = create_server(&server_options(false), &name, &security)
            .expect("the service's own SID may create more instances");
        drop(second);
        drop(first);
    });
}

#[test]
fn a_server_run_by_the_interactive_user_keeps_serving_new_clients() {
    // `PipeServer::start` makes several instances at once, and one more for
    // every client it takes, all as the interactive user.
    let (name, server) = start();
    run(async {
        for n in 0..12 {
            let mut client =
                open_client(&name, CLIENT_ACCESS).expect("the server has an instance waiting");
            assert_eq!(
                round_trip(&mut client, &format!("c-{n}"), Request::GetState).await,
                Response::State {
                    state: idle_state()
                }
            );
            drop(client);
        }
    });
    drop(server);
}

// ---------------------------------------------------------------------------
// C6, C7, C8 - the client wrapper
// ---------------------------------------------------------------------------

#[test]
fn the_client_wrapper_round_trips_a_request_over_the_service_pipe() {
    let (name, server) = start();
    run(async {
        let mut client = open_client(&name, CLIENT_ACCESS).expect("the narrow mask is enough");
        assert_eq!(
            round_trip(&mut client, "rt-1", Request::GetState).await,
            Response::State {
                state: idle_state()
            }
        );
        // Both directions keep working on the same connection.
        assert_eq!(
            round_trip(&mut client, "rt-2", Request::Subscribe).await,
            Response::Subscribed
        );
    });
    drop(server);
}

#[test]
fn the_client_wrapper_honors_the_access_mask_it_is_given() {
    let name = unique_pipe_name();
    run(async {
        let security =
            SecurityDescriptor::from_sddl(&pipe_sddl(SYSTEM_SID)).expect("valid pipe SDDL");
        let first =
            create_server(&server_options(true), &name, &security).expect("the first instance");
        // The mask reaches `CreateFileW`: the DACL grants the interactive
        // user `CLIENT_ACCESS` and no more, so a client that asks for
        // `FILE_ALL_ACCESS` (which adds DELETE and WRITE_OWNER) is refused.
        // A refused open must not take the instance the server waits on.
        match open_client(&name, FILE_ALL_ACCESS) {
            Ok(_) => panic!(
                "a client asking for FILE_ALL_ACCESS must be denied; \
                 the wrapper has to pass the mask it is given"
            ),
            Err(error) => assert_eq!(
                error.raw_os_error(),
                Some(ACCESS_DENIED),
                "a mask beyond the DACL must be denied, got {error}"
            ),
        }
        // The same wrapper, with the narrow mask, is let in and connects.
        let client = open_client(&name, CLIENT_ACCESS).expect("the narrow mask is granted");
        first.connect().await.expect("the server accepts it");
        drop(client);
    });
}

#[test]
fn the_client_wrapper_reports_an_unserved_pipe_as_not_found() {
    run(async {
        let name = unique_pipe_name();
        match open_client(&name, CLIENT_ACCESS) {
            Ok(_) => panic!("{name} is served by nobody"),
            Err(error) => assert_eq!(
                error.kind(),
                io::ErrorKind::NotFound,
                "an unserved pipe name is NotFound, got {error}"
            ),
        }
    });
}
