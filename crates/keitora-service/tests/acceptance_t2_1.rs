// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-2.1: the `keitora-service` binary, its `--console`
//! development mode, its Windows service mode and its log files.
//!
//! The monotonic clock of the same task is tested in
//! `acceptance_t2_1_clock.rs`.
//!
//! These tests run the built binary (`CARGO_BIN_EXE_keitora-service`) as a
//! child process. They never install a service, never need administrator
//! rights and never write to the real `%ProgramData%`: every run gets its own
//! sandbox under Cargo's `CARGO_TARGET_TMPDIR`, a `--data-dir` inside it, and
//! the `ProgramData` and `ALLUSERSPROFILE` environment variables pointed at an
//! empty folder in the sandbox as a safety net.
//!
//! # Command-line contract
//!
//! ```text
//! keitora-service                               service mode (started by the SCM)
//! keitora-service --console [--data-dir <DIR>]  console mode (development builds only)
//! ```
//!
//! - **No arguments: service mode.** The binary hands control to the Service
//!   Control Manager through the `windows-service` crate. Started outside the
//!   SCM (from a terminal, or by these tests), connecting fails at once, and
//!   the binary exits by itself with a non-zero code and a message on stderr.
//!   It writes nothing anywhere before the SCM has connected: no data
//!   directory, no log directory, no log file.
//! - **`--console`: console mode**, for development. It runs the same service
//!   in the foreground until it is told to stop:
//!   - closing its standard input (end of file) stops it cleanly, which is
//!     how these tests and the T-2.9 harness stop it without sending a
//!     console control event (that would need `unsafe` outside
//!     `keitora-sys`). A read error on stdin may stop it too;
//!   - Ctrl+C and Ctrl+Break stop it cleanly too (not tested here, see
//!     below).
//!
//!   A clean stop exits with code 0, after the log has been flushed.
//!   `--console` exists only in builds with debug assertions (the `dev` and
//!   `test` profiles); a release build rejects it like any unknown argument
//!   (F0 security review, L9).
//! - **`--data-dir <DIR>`**: the folder where the service keeps its files,
//!   instead of `%ProgramData%\Keitora`. Accepted only together with
//!   `--console`, in either order, at most once, and always followed by a
//!   value. `<DIR>` and missing parent folders are created if they don't
//!   exist.
//! - **Anything else is rejected**: an unknown option, a positional argument,
//!   `--data-dir` without `--console`, `--data-dir` with no value, or
//!   `--data-dir` given twice. A rejected command line exits by itself with a
//!   non-zero code and a message on stderr, and writes nothing: arguments are
//!   checked before anything is created.
//!
//! # Log files
//!
//! - Logging uses `tracing`, written to `<data dir>\logs`, which is
//!   `%ProgramData%\Keitora\logs` in service mode. The folder is created if
//!   it doesn't exist.
//! - Files rotate daily. A log file's name starts with `keitora`, ends with
//!   `.log` and contains its date as `YYYY-MM-DD` (for example
//!   `keitora.2026-11-10.log`). The date may be UTC or local: the tests accept
//!   a date within one day of the test's UTC date.
//! - The current day's file exists while the service runs (logging is set up
//!   at start) and is not empty after a clean stop (everything buffered has
//!   been written). Starting again on the same day appends to it; an earlier
//!   day's file is never written to again.
//! - If the log folder can't be created or opened, the service keeps running
//!   anyway and reports the problem on stderr. Interpretation chosen for the
//!   maintainer: refusing to run would let anyone who can break the log
//!   folder, such as a standard user who pre-creates `%ProgramData%\Keitora`
//!   before installation (T-5.3), switch off blocking and strict mode.
//!
//! # Criteria covered
//!
//! - **AC1** — `--console` runs the service in the foreground for development
//!   and stops cleanly with exit code 0:
//!   `console_mode_runs_until_stdin_is_closed_and_then_exits_with_code_zero`,
//!   `console_mode_accepts_its_options_in_either_order`.
//! - **AC2** — `--data-dir` moves every file of a console run into a folder
//!   of the developer's choosing, and nothing is written under
//!   `%ProgramData%`:
//!   `console_mode_creates_a_missing_data_directory_and_its_log_folder`,
//!   `console_mode_with_a_data_directory_writes_nothing_under_program_data`.
//! - **AC3** — `tracing` logs go to daily rotating files in
//!   `<data dir>\logs`:
//!   `console_mode_log_file_is_named_after_the_current_day`,
//!   `console_mode_log_file_is_not_empty_after_a_clean_stop`,
//!   `log_from_an_earlier_run_on_the_same_day_is_kept_when_the_service_starts_again`,
//!   `log_file_of_an_earlier_day_is_not_written_to`.
//! - **AC4** — a log folder that can't be used doesn't stop the service:
//!   `console_mode_keeps_running_when_the_log_folder_cannot_be_created`.
//! - **AC5** — invalid command lines are rejected quickly, with a message and
//!   a non-zero exit code, before anything is written:
//!   `unknown_arguments_are_rejected_with_a_message_and_a_non_zero_exit`,
//!   `data_dir_without_console_is_rejected_and_creates_nothing`,
//!   `data_dir_without_a_value_is_rejected`,
//!   `data_dir_given_twice_is_rejected_and_creates_neither_folder`.
//! - **AC6** — with no arguments the binary runs as a Windows service; outside
//!   the SCM it fails quickly with a message instead of hanging, and writes
//!   nothing:
//!   `no_arguments_outside_the_service_manager_fails_quickly_with_a_message`.
//! - **AC7** (F0 security review, L9) — release builds don't have
//!   `--console`:
//!   `console_option_is_rejected_in_release_builds`, compiled only without
//!   debug assertions, so it runs with
//!   `cargo test --release -p keitora-service --test acceptance_t2_1`. The
//!   tests that need `--console` are compiled only with debug assertions.
//!   `cargo test --workspace` (what `just check` and CI run) runs those and
//!   not this one.
//!
//! # Verified by review or other checks, not here
//!
//! - Service mode under the real SCM: start, stop, and a clean exit on the
//!   SCM's stop request (manual test; installing a service needs an
//!   administrator).
//! - Ctrl+C and Ctrl+Break stop console mode cleanly (manual test).
//! - The service calls `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)`
//!   when it starts, before anything can load a DLL (F0 security review, L10).
//! - Issue #1, ADR-0012: every Win32 call is wrapped in `crates/keitora-sys`,
//!   one wrapper per call, each with a `// SAFETY:` comment; it is the only
//!   crate that allows `unsafe`. `keitora-service`, `keitora-nmh` and
//!   `keitora-desktop` have `#![forbid(unsafe_code)]`, and `AGENTS.md`,
//!   `docs/architecture.md`, `docs/threat-model.md` and the workspace lint
//!   comments no longer say `keitora-service/src/sys/`.
//! - `keitora_core::clock::FakeClock` is banned in the production code of
//!   `keitora-service`, `keitora-nmh`, `keitora-desktop` and `keitora-sys`
//!   through `disallowed-types` in each crate's `clippy.toml`, which repeats
//!   the root settings; `scripts/clippy-config.spec.mjs` checks those copies
//!   too. Validation plants a use and checks that clippy fails.
//! - The roadmap's security note: one monotonic clock instance for the whole
//!   service, from a 64-bit source that never wraps or goes backwards
//!   (`Instant`/QPC or `QueryUnbiasedInterruptTime`, never `GetTickCount`),
//!   never from a client; the service changes the engine only through its
//!   methods, never by replacing it with `Engine::default()`. The behaviour
//!   of the clock is tested in `acceptance_t2_1_clock.rs`; its source and its
//!   single instance are checked by review.
//! - It is documented whether time asleep or hibernated counts toward a
//!   session's active time, which follows from the monotonic source chosen.
//! - Logs never contain blocklists or other user data printed with `{:?}`.
//! - Whether old log files are deleted after some days (recommended, so the
//!   folder can't grow without limit) is not a criterion of this task.

#![cfg(test)]

use std::ffi::OsString;
use std::fs;
use std::io::Read;
use std::path::{Path, PathBuf};
use std::process::{Child, ChildStdin, Command, ExitStatus, Stdio};
use std::thread::{self, JoinHandle};
use std::time::{Duration, Instant};

/// The binary under test, built by Cargo for this test.
const SERVICE_EXE: &str = env!("CARGO_BIN_EXE_keitora-service");

/// How long a process may take to exit once it has been told to stop, or to
/// fail on its own. Generous, for slow CI runners.
const EXIT_TIMEOUT: Duration = Duration::from_secs(30);

/// How long console mode may take to create its log file after starting.
#[cfg(debug_assertions)]
const START_TIMEOUT: Duration = Duration::from_secs(30);

/// How long a process must stay alive to count as "still running".
#[cfg(debug_assertions)]
const STAY_ALIVE: Duration = Duration::from_millis(1500);

/// How often the tests look at a process or a folder while waiting.
const POLL_INTERVAL: Duration = Duration::from_millis(50);

// ---------------------------------------------------------------------------
// Helpers: sandboxes
// ---------------------------------------------------------------------------

/// A fresh, empty folder for one test, named after it, under Cargo's
/// temporary folder for integration tests. It contains an empty
/// `programdata` folder that stands in for `%ProgramData%`.
fn sandbox(test_name: &str) -> PathBuf {
    let root = Path::new(env!("CARGO_TARGET_TMPDIR"))
        .join("acceptance_t2_1")
        .join(test_name);
    match fs::remove_dir_all(&root) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => panic!("couldn't clear the sandbox {}: {error}", root.display()),
    }
    fs::create_dir_all(fake_program_data(&root)).expect("couldn't create the sandbox");
    root
}

/// The folder that stands in for `%ProgramData%` in `sandbox`.
fn fake_program_data(sandbox: &Path) -> PathBuf {
    sandbox.join("programdata")
}

/// Every entry under `dir`, recursively, as paths relative to it.
fn tree(dir: &Path) -> Vec<PathBuf> {
    let mut found = Vec::new();
    let mut pending = vec![dir.to_path_buf()];
    while let Some(current) = pending.pop() {
        for entry in fs::read_dir(&current).expect("couldn't list a folder") {
            let path = entry.expect("couldn't read a folder entry").path();
            if path.is_dir() {
                pending.push(path.clone());
            }
            found.push(
                path.strip_prefix(dir)
                    .expect("an entry is inside its folder")
                    .to_path_buf(),
            );
        }
    }
    found.sort();
    found
}

/// Fails the test if anything was written under the sandbox's stand-in for
/// `%ProgramData%`.
fn assert_fake_program_data_is_empty(sandbox: &Path) {
    let written = tree(&fake_program_data(sandbox));
    assert!(
        written.is_empty(),
        "the service wrote under %ProgramData% (the sandbox's stand-in): {written:?}"
    );
}

/// The real `%ProgramData%\Keitora`, if the test's own environment names a
/// `%ProgramData%`.
fn real_keitora_program_data() -> Option<PathBuf> {
    std::env::var_os("ProgramData").map(|dir| PathBuf::from(dir).join("Keitora"))
}

/// Remembers whether the real `%ProgramData%\Keitora` existed, so a test can
/// check that a run didn't create it.
struct RealProgramDataGuard {
    path: Option<PathBuf>,
    existed: bool,
}

impl RealProgramDataGuard {
    fn new() -> Self {
        let path = real_keitora_program_data();
        let existed = path.as_deref().is_some_and(Path::exists);
        Self { path, existed }
    }

    /// Fails the test if the run created the real `%ProgramData%\Keitora`.
    fn assert_not_created(&self) {
        if let Some(path) = &self.path
            && !self.existed
        {
            assert!(
                !path.exists(),
                "the service created the real {}; delete it by hand",
                path.display()
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Helpers: running the binary
// ---------------------------------------------------------------------------

/// A command for the service binary with `args`, run from `sandbox`, with
/// `%ProgramData%` pointed at the sandbox's stand-in and every standard
/// stream piped.
fn service_command<I, S>(sandbox: &Path, args: I) -> Command
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut command = Command::new(SERVICE_EXE);
    command
        .args(args.into_iter().map(Into::into))
        .current_dir(sandbox)
        .env("ProgramData", fake_program_data(sandbox))
        .env("ALLUSERSPROFILE", fake_program_data(sandbox))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    command
}

/// Reads all of `stream` on its own thread, so a full pipe never blocks the
/// child.
fn read_in_background<R: Read + Send + 'static>(mut stream: R) -> JoinHandle<String> {
    thread::spawn(move || {
        let mut bytes = Vec::new();
        // A read error only cuts the captured output short.
        let _ = stream.read_to_end(&mut bytes);
        String::from_utf8_lossy(&bytes).into_owned()
    })
}

/// How a process ended.
struct Finished {
    status: ExitStatus,
    stdout: String,
    stderr: String,
}

impl std::fmt::Display for Finished {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "{}
--- stdout ---
{}
--- stderr ---
{}",
            self.status, self.stdout, self.stderr
        )
    }
}

/// A running service process. Its stdin stays open until
/// [`Running::close_stdin`]; dropping it kills the process if it is still
/// running, so a failed test leaves nothing behind.
struct Running {
    child: Child,
    stdin: Option<ChildStdin>,
    stdout: Option<JoinHandle<String>>,
    stderr: Option<JoinHandle<String>>,
}

impl Running {
    fn start(mut command: Command) -> Self {
        let mut child = command.spawn().expect("couldn't start the service binary");
        let stdin = child.stdin.take();
        let stdout = child.stdout.take().map(read_in_background);
        let stderr = child.stderr.take().map(read_in_background);
        Self {
            child,
            stdin,
            stdout,
            stderr,
        }
    }

    #[cfg(debug_assertions)]
    fn is_running(&mut self) -> bool {
        self.child
            .try_wait()
            .expect("couldn't check the service process")
            .is_none()
    }

    /// Closes the child's stdin: end of file, which stops console mode.
    #[cfg(debug_assertions)]
    fn close_stdin(&mut self) {
        drop(self.stdin.take());
    }

    fn take_output(&mut self) -> (String, String) {
        let join = |handle: Option<JoinHandle<String>>| {
            handle
                .map(|handle| handle.join().expect("an output reader panicked"))
                .unwrap_or_default()
        };
        (join(self.stdout.take()), join(self.stderr.take()))
    }

    /// Waits for the process to exit on its own, for up to `timeout`. If it
    /// doesn't, kills it and fails the test with what it printed.
    fn wait(&mut self, timeout: Duration, what: &str) -> Finished {
        let deadline = Instant::now()
            .checked_add(timeout)
            .expect("the deadline fits in an Instant");
        loop {
            if let Some(status) = self
                .child
                .try_wait()
                .expect("couldn't check the service process")
            {
                let (stdout, stderr) = self.take_output();
                return Finished {
                    status,
                    stdout,
                    stderr,
                };
            }
            if Instant::now() >= deadline {
                // Errors only mean that it has just exited.
                let _ = self.child.kill();
                let _ = self.child.wait();
                let (stdout, stderr) = self.take_output();
                panic!(
                    "{what}: the service didn't exit within {timeout:?}\n\
                     stdout:\n{stdout}\nstderr:\n{stderr}"
                );
            }
            thread::sleep(POLL_INTERVAL);
        }
    }

    /// Fails the test if the process exits within [`STAY_ALIVE`].
    #[cfg(debug_assertions)]
    fn assert_stays_running(&mut self, what: &str) {
        let deadline = Instant::now()
            .checked_add(STAY_ALIVE)
            .expect("the deadline fits in an Instant");
        while Instant::now() < deadline {
            if !self.is_running() {
                let finished = self.wait(EXIT_TIMEOUT, what);
                panic!("{what}: the service exited on its own: {finished}");
            }
            thread::sleep(POLL_INTERVAL);
        }
    }
}

impl Drop for Running {
    fn drop(&mut self) {
        drop(self.stdin.take());
        if let Ok(None) = self.child.try_wait() {
            // Errors only mean that it has just exited.
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }
}

/// Runs the binary with `args`, keeping its stdin open so that only the
/// binary itself can decide to exit, and checks that it rejects the command
/// line: it exits by itself, quickly, with a non-zero code and a message on
/// stderr.
fn assert_rejected<I, S>(sandbox: &Path, args: I, what: &str) -> Finished
where
    I: IntoIterator<Item = S>,
    S: Into<OsString>,
{
    let mut running = Running::start(service_command(sandbox, args));
    let finished = running.wait(EXIT_TIMEOUT, what);
    assert!(
        !finished.status.success(),
        "{what}: expected a non-zero exit code, got {finished}"
    );
    assert!(
        !finished.stderr.trim().is_empty(),
        "{what}: expected a message on stderr, got {finished}"
    );
    finished
}

// ---------------------------------------------------------------------------
// Helpers: console mode and log files
// ---------------------------------------------------------------------------

/// The arguments of a console run whose data folder is `data_dir`.
#[cfg(debug_assertions)]
fn console_args(data_dir: &Path) -> Vec<OsString> {
    vec![
        OsString::from("--console"),
        OsString::from("--data-dir"),
        data_dir.as_os_str().to_owned(),
    ]
}

/// Whether `name` looks like a Keitora log file: `keitora…<date>….log`.
#[cfg(debug_assertions)]
fn is_log_file_name(name: &str) -> bool {
    name.starts_with("keitora") && name.ends_with(".log") && date_in_name(name).is_some()
}

/// The Keitora log files directly inside `log_dir`, sorted by name; empty if
/// the folder doesn't exist.
#[cfg(debug_assertions)]
fn log_files(log_dir: &Path) -> Vec<PathBuf> {
    let Ok(entries) = fs::read_dir(log_dir) else {
        return Vec::new();
    };
    let mut files: Vec<PathBuf> = entries
        .map(|entry| entry.expect("couldn't read a folder entry").path())
        .filter(|path| {
            path.is_file()
                && path
                    .file_name()
                    .and_then(|name| name.to_str())
                    .is_some_and(is_log_file_name)
        })
        .collect();
    files.sort();
    files
}

/// Starts a console run on `data_dir` and waits until its log file exists,
/// failing the test if the service exits or no log file appears in time.
#[cfg(debug_assertions)]
fn start_console(sandbox: &Path, data_dir: &Path) -> Running {
    let mut running = Running::start(service_command(sandbox, console_args(data_dir)));
    let log_dir = data_dir.join("logs");
    let deadline = Instant::now()
        .checked_add(START_TIMEOUT)
        .expect("the deadline fits in an Instant");
    loop {
        if !log_files(&log_dir).is_empty() {
            return running;
        }
        if !running.is_running() {
            let finished = running.wait(EXIT_TIMEOUT, "console start");
            panic!("console mode exited before creating its log file: {finished}");
        }
        if Instant::now() >= deadline {
            running.close_stdin();
            let finished = running.wait(EXIT_TIMEOUT, "console start");
            panic!(
                "console mode created no log file named keitora…YYYY-MM-DD….log in {} \
                 within {START_TIMEOUT:?}; the data folder holds {:?}; {finished}",
                log_dir.display(),
                tree_or_empty(data_dir),
            );
        }
        thread::sleep(POLL_INTERVAL);
    }
}

/// Like [`tree`], but an empty list if `dir` doesn't exist.
#[cfg(debug_assertions)]
fn tree_or_empty(dir: &Path) -> Vec<PathBuf> {
    if dir.is_dir() { tree(dir) } else { Vec::new() }
}

/// Closes the run's stdin and checks that it stops cleanly: exit code 0.
#[cfg(debug_assertions)]
fn stop_cleanly(running: &mut Running, what: &str) -> Finished {
    running.close_stdin();
    let finished = running.wait(EXIT_TIMEOUT, what);
    assert!(
        finished.status.success(),
        "{what}: console mode didn't exit with code 0 after stdin was closed: {finished}"
    );
    finished
}

/// One whole console run on `data_dir`: start, wait for the log file, stop.
#[cfg(debug_assertions)]
fn run_console_once(sandbox: &Path, data_dir: &Path) -> Finished {
    let mut running = start_console(sandbox, data_dir);
    stop_cleanly(&mut running, "console run")
}

/// Days since 1970-01-01 of a proleptic Gregorian date, or `None` if the
/// date doesn't exist.
#[cfg(debug_assertions)]
fn days_from_civil(year: i64, month: i64, day: i64) -> Option<i64> {
    if !(1..=12).contains(&month) || !(1..=31).contains(&day) || year < 1970 {
        return None;
    }
    // Howard Hinnant's `days_from_civil`, for years from 1970 on.
    let year = if month <= 2 {
        year.checked_sub(1)?
    } else {
        year
    };
    let era = year.checked_div(400)?;
    let year_of_era = year.checked_sub(era.checked_mul(400)?)?;
    let shifted_month = if month > 2 {
        month.checked_sub(3)?
    } else {
        month.checked_add(9)?
    };
    let day_of_year = shifted_month
        .checked_mul(153)?
        .checked_add(2)?
        .checked_div(5)?
        .checked_add(day.checked_sub(1)?)?;
    let day_of_era = year_of_era
        .checked_mul(365)?
        .checked_add(year_of_era.checked_div(4)?)?
        .checked_sub(year_of_era.checked_div(100)?)?
        .checked_add(day_of_year)?;
    era.checked_mul(146_097)?
        .checked_add(day_of_era)?
        .checked_sub(719_468)
}

/// The first `YYYY-MM-DD` date in `name`, as days since 1970-01-01.
#[cfg(debug_assertions)]
fn date_in_name(name: &str) -> Option<i64> {
    let bytes = name.as_bytes();
    let mut start = 0_usize;
    while let Some(end) = start.checked_add(10) {
        let window = bytes.get(start..end)?;
        let shape_ok = window.iter().enumerate().all(|(index, byte)| {
            if index == 4 || index == 7 {
                *byte == b'-'
            } else {
                byte.is_ascii_digit()
            }
        });
        if shape_ok {
            let text = name.get(start..end).expect("ASCII digits and dashes");
            let number =
                |range: std::ops::Range<usize>| -> Option<i64> { text.get(range)?.parse().ok() };
            if let Some(days) = days_from_civil(number(0..4)?, number(5..7)?, number(8..10)?) {
                return Some(days);
            }
        }
        start = start.checked_add(1)?;
    }
    None
}

/// Days since 1970-01-01 of the current UTC date.
#[cfg(debug_assertions)]
fn utc_today() -> i64 {
    let seconds = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .expect("the system time is after 1970")
        .as_secs();
    i64::try_from(seconds.checked_div(86_400).expect("a day has seconds"))
        .expect("today fits in an i64")
}

/// The file name of `path` as text.
#[cfg(debug_assertions)]
fn file_name(path: &Path) -> &str {
    path.file_name()
        .and_then(|name| name.to_str())
        .expect("a log file has a UTF-8 name")
}

// ---------------------------------------------------------------------------
// AC1 — console mode runs in the foreground and stops cleanly
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
#[test]
fn console_mode_runs_until_stdin_is_closed_and_then_exits_with_code_zero() {
    let sandbox = sandbox("console_runs_until_stdin_closed");
    let data_dir = sandbox.join("data");

    let mut running = start_console(&sandbox, &data_dir);
    running.assert_stays_running("console mode with stdin open");

    stop_cleanly(&mut running, "console mode, stdin closed");
}

#[cfg(debug_assertions)]
#[test]
fn console_mode_accepts_its_options_in_either_order() {
    let sandbox = sandbox("console_options_either_order");
    let data_dir = sandbox.join("data");

    let mut running = Running::start(service_command(
        &sandbox,
        [
            OsString::from("--data-dir"),
            data_dir.as_os_str().to_owned(),
            OsString::from("--console"),
        ],
    ));
    running.assert_stays_running("--data-dir <DIR> --console");
    stop_cleanly(&mut running, "--data-dir <DIR> --console");

    assert!(
        !log_files(&data_dir.join("logs")).is_empty(),
        "--data-dir before --console must be honoured too; the data folder holds {:?}",
        tree_or_empty(&data_dir)
    );
}

// ---------------------------------------------------------------------------
// AC2 — the data folder of a console run
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
#[test]
fn console_mode_creates_a_missing_data_directory_and_its_log_folder() {
    let sandbox = sandbox("console_creates_missing_data_dir");
    // Neither folder exists yet; the path also has non-ASCII characters and a
    // space, as a Windows user name can.
    let data_dir = sandbox.join("not yet").join("Keitora ñ");
    assert!(!data_dir.exists());

    run_console_once(&sandbox, &data_dir);

    let log_dir = data_dir.join("logs");
    assert!(log_dir.is_dir(), "{} wasn't created", log_dir.display());
    assert!(
        !log_files(&log_dir).is_empty(),
        "no log file in {}",
        log_dir.display()
    );
}

#[cfg(debug_assertions)]
#[test]
fn console_mode_with_a_data_directory_writes_nothing_under_program_data() {
    let sandbox = sandbox("console_writes_nothing_under_program_data");
    let data_dir = sandbox.join("data");
    let real = RealProgramDataGuard::new();

    run_console_once(&sandbox, &data_dir);

    assert_fake_program_data_is_empty(&sandbox);
    real.assert_not_created();
    // Nothing is written next to the data folder or in the working folder
    // either: the sandbox holds only the two folders the test made.
    let mut top_level: Vec<PathBuf> = fs::read_dir(&sandbox)
        .expect("couldn't list the sandbox")
        .map(|entry| PathBuf::from(entry.expect("couldn't read an entry").file_name()))
        .collect();
    top_level.sort();
    assert_eq!(
        top_level,
        vec![PathBuf::from("data"), PathBuf::from("programdata")],
        "a console run with --data-dir wrote outside its data folder"
    );
}

// ---------------------------------------------------------------------------
// AC3 — daily rotating log files
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
#[test]
fn console_mode_log_file_is_named_after_the_current_day() {
    let sandbox = sandbox("log_file_named_after_the_day");
    let data_dir = sandbox.join("data");
    let first_day = utc_today();

    run_console_once(&sandbox, &data_dir);

    let last_day = utc_today();
    let files = log_files(&data_dir.join("logs"));
    assert!(!files.is_empty(), "no log file was written");
    for file in &files {
        let name = file_name(file);
        let day = date_in_name(name).expect("a log file name holds a date");
        // UTC or local date, and the run may cross midnight.
        assert!(
            (first_day - 1..=last_day + 1).contains(&day),
            "{name} isn't named after the current day"
        );
    }
}

#[cfg(debug_assertions)]
#[test]
fn console_mode_log_file_is_not_empty_after_a_clean_stop() {
    let sandbox = sandbox("log_file_not_empty_after_stop");
    let data_dir = sandbox.join("data");

    run_console_once(&sandbox, &data_dir);

    let files = log_files(&data_dir.join("logs"));
    assert!(!files.is_empty(), "no log file was written");
    let mut total = 0_u64;
    for file in &files {
        total += fs::metadata(file).expect("couldn't read a log file").len();
    }
    assert!(
        total > 0,
        "the log is empty after a clean stop; buffered records must be written \
         before the process exits"
    );
}

#[cfg(debug_assertions)]
#[test]
fn log_from_an_earlier_run_on_the_same_day_is_kept_when_the_service_starts_again() {
    let sandbox = sandbox("log_kept_across_runs");
    let data_dir = sandbox.join("data");
    let log_dir = data_dir.join("logs");

    run_console_once(&sandbox, &data_dir);
    let first_run: Vec<(PathBuf, Vec<u8>)> = log_files(&log_dir)
        .into_iter()
        .map(|file| {
            let bytes = fs::read(&file).expect("couldn't read a log file");
            (file, bytes)
        })
        .collect();
    assert!(!first_run.is_empty(), "the first run wrote no log file");

    run_console_once(&sandbox, &data_dir);

    for (file, before) in &first_run {
        let after = fs::read(file).unwrap_or_else(|error| {
            panic!("{} disappeared after a restart: {error}", file.display())
        });
        assert!(
            after.starts_with(before),
            "{} lost what the first run wrote; a restart must append",
            file.display()
        );
    }
    let mut grew = false;
    for (file, before) in &first_run {
        let after = fs::read(file).expect("couldn't read a log file");
        grew |= after.len() > before.len();
    }
    let new_file = log_files(&log_dir).len() > first_run.len();
    assert!(grew || new_file, "the second run logged nothing");
}

#[cfg(debug_assertions)]
#[test]
fn log_file_of_an_earlier_day_is_not_written_to() {
    let sandbox = sandbox("old_log_file_not_written");
    let data_dir = sandbox.join("data");
    let log_dir = data_dir.join("logs");
    fs::create_dir_all(&log_dir).expect("couldn't create the log folder");
    let old_name = "keitora.2020-01-01.log";
    let old_file = log_dir.join(old_name);
    let old_content = b"a line from 2020\n";
    fs::write(&old_file, old_content).expect("couldn't write the old log file");

    run_console_once(&sandbox, &data_dir);

    // The old file may have been deleted by a retention rule, but never
    // written to.
    if old_file.exists() {
        assert_eq!(
            fs::read(&old_file).expect("couldn't read the old log file"),
            old_content,
            "the service wrote into the log file of an earlier day"
        );
    }
    let today = utc_today();
    let current: Vec<PathBuf> = log_files(&log_dir)
        .into_iter()
        .filter(|file| {
            date_in_name(file_name(file)).is_some_and(|day| (today - 1..=today + 1).contains(&day))
        })
        .collect();
    assert!(
        !current.is_empty(),
        "no log file for the current day next to {old_name}: {:?}",
        log_files(&log_dir)
    );
}

// ---------------------------------------------------------------------------
// AC4 — a broken log folder doesn't stop the service
// ---------------------------------------------------------------------------

#[cfg(debug_assertions)]
#[test]
fn console_mode_keeps_running_when_the_log_folder_cannot_be_created() {
    let sandbox = sandbox("console_runs_without_log_folder");
    let data_dir = sandbox.join("data");
    fs::create_dir_all(&data_dir).expect("couldn't create the data folder");
    // A file where the log folder should be: neither it nor a file inside it
    // can be created.
    fs::write(data_dir.join("logs"), b"not a folder").expect("couldn't plant the file");

    let mut running = Running::start(service_command(&sandbox, console_args(&data_dir)));
    running.assert_stays_running("console mode with an unusable log folder");
    let finished = stop_cleanly(&mut running, "console mode with an unusable log folder");

    assert!(
        !finished.stderr.trim().is_empty(),
        "the logging problem must be reported on stderr: {finished}"
    );
    assert_eq!(
        fs::read(data_dir.join("logs")).expect("the planted file is gone"),
        b"not a folder",
        "the service changed the file that blocked its log folder"
    );
}

// ---------------------------------------------------------------------------
// AC5 — invalid command lines
// ---------------------------------------------------------------------------

#[test]
fn unknown_arguments_are_rejected_with_a_message_and_a_non_zero_exit() {
    let sandbox = sandbox("unknown_arguments_rejected");
    let data_dir = sandbox.join("data");
    let real = RealProgramDataGuard::new();

    let cases: Vec<(&str, Vec<OsString>)> = vec![
        ("an unknown option", vec!["--bogus".into()]),
        ("a positional argument", vec!["run".into()]),
        (
            "an unknown option after valid ones",
            vec![
                "--console".into(),
                "--data-dir".into(),
                data_dir.as_os_str().to_owned(),
                "--bogus".into(),
            ],
        ),
        (
            "a positional argument after valid ones",
            vec![
                "--console".into(),
                "--data-dir".into(),
                data_dir.as_os_str().to_owned(),
                "extra".into(),
            ],
        ),
    ];
    for (what, args) in cases {
        assert_rejected(&sandbox, args, what);
        assert!(
            !data_dir.exists(),
            "{what}: the data folder was created before the command line was checked"
        );
    }

    assert_fake_program_data_is_empty(&sandbox);
    real.assert_not_created();
}

#[test]
fn data_dir_without_console_is_rejected_and_creates_nothing() {
    let sandbox = sandbox("data_dir_without_console_rejected");
    let data_dir = sandbox.join("data");
    let real = RealProgramDataGuard::new();

    assert_rejected(
        &sandbox,
        [
            OsString::from("--data-dir"),
            data_dir.as_os_str().to_owned(),
        ],
        "--data-dir without --console",
    );

    assert!(
        !data_dir.exists(),
        "--data-dir without --console created {}",
        data_dir.display()
    );
    assert_fake_program_data_is_empty(&sandbox);
    real.assert_not_created();
}

#[test]
fn data_dir_without_a_value_is_rejected() {
    let sandbox = sandbox("data_dir_without_value_rejected");
    let real = RealProgramDataGuard::new();

    assert_rejected(
        &sandbox,
        ["--console", "--data-dir"],
        "--data-dir with no value",
    );

    assert_fake_program_data_is_empty(&sandbox);
    real.assert_not_created();
}

#[test]
fn data_dir_given_twice_is_rejected_and_creates_neither_folder() {
    let sandbox = sandbox("data_dir_twice_rejected");
    let first = sandbox.join("first");
    let second = sandbox.join("second");
    let real = RealProgramDataGuard::new();

    assert_rejected(
        &sandbox,
        [
            OsString::from("--console"),
            OsString::from("--data-dir"),
            first.as_os_str().to_owned(),
            OsString::from("--data-dir"),
            second.as_os_str().to_owned(),
        ],
        "--data-dir given twice",
    );

    assert!(!first.exists(), "{} was created", first.display());
    assert!(!second.exists(), "{} was created", second.display());
    assert_fake_program_data_is_empty(&sandbox);
    real.assert_not_created();
}

// ---------------------------------------------------------------------------
// AC6 — service mode outside the SCM
// ---------------------------------------------------------------------------

#[test]
fn no_arguments_outside_the_service_manager_fails_quickly_with_a_message() {
    let sandbox = sandbox("no_arguments_outside_scm");
    let real = RealProgramDataGuard::new();
    let no_args: [&str; 0] = [];

    // stdin stays open, so the process can't mistake this for a console
    // stop: it must give up on its own when the SCM doesn't answer.
    assert_rejected(&sandbox, no_args, "no arguments, outside the SCM");

    assert_fake_program_data_is_empty(&sandbox);
    real.assert_not_created();
    let written: Vec<PathBuf> = tree(&sandbox)
        .into_iter()
        .filter(|path| path != Path::new("programdata"))
        .collect();
    assert!(
        written.is_empty(),
        "service mode wrote files before the SCM connected: {written:?}"
    );
}

// ---------------------------------------------------------------------------
// AC7 — no console mode in release builds
// ---------------------------------------------------------------------------

#[cfg(not(debug_assertions))]
#[test]
fn console_option_is_rejected_in_release_builds() {
    let sandbox = sandbox("console_rejected_in_release");
    let data_dir = sandbox.join("data");
    let real = RealProgramDataGuard::new();

    assert_rejected(&sandbox, ["--console"], "--console in a release build");
    assert_rejected(
        &sandbox,
        [
            OsString::from("--console"),
            OsString::from("--data-dir"),
            data_dir.as_os_str().to_owned(),
        ],
        "--console --data-dir in a release build",
    );

    assert!(
        !data_dir.exists(),
        "a release build created {} for --console",
        data_dir.display()
    );
    assert_fake_program_data_is_empty(&sandbox);
    real.assert_not_created();
}
