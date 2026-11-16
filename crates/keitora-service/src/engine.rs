// SPDX-License-Identifier: MIT OR Apache-2.0

//! The service engine: [`ServiceEngine`] wires `keitora-core`'s pomodoro
//! engine to the store, applies the requests of clients, publishes events
//! and saves a snapshot of the engine on every transition.
//!
//! # One engine, one clock, no thread of its own
//!
//! The service creates one [`ServiceEngine`] with its one
//! [`SystemClock`](crate::clock::SystemClock) and its store, and shares it:
//! the pipe server calls [`RequestHandler::handle`] for each request, from
//! several threads, and the service's main loop calls
//! [`ServiceEngine::tick`] about once a second. Between those calls the
//! engine does nothing. Everything happens under one lock, so requests and
//! ticks apply one at a time, and events are published in the order of the
//! changes they report.
//!
//! The core engine is changed only through its own methods; it is never
//! replaced by a new one, apart from undoing a strict start whose snapshot
//! couldn't be saved, which puts back the engine exactly as it was.
//!
//! # Observing time
//!
//! The core engine's schedule moves with the clock alone, so a phase can
//! change, or a session end, between two calls. Every tick and every request
//! first *observes* the engine: if the phase or cycle changed since the
//! last one published, it publishes [`Event::StateChanged`] and saves a
//! snapshot; if the session ran out, it records it in the history as
//! completed, publishes [`Event::SessionEnded`] and the idle state, and
//! saves the idle snapshot at once. So a session that ended between two
//! ticks is reported and recorded exactly once, before the next request is
//! applied.
//!
//! # Snapshots and checkpoints (ADR-0011, ADR-0013, #48, #49)
//!
//! - Every transition (a start, a pause, a resume, a skipped break, a stop,
//!   a phase change, the end of a session) saves a snapshot before the
//!   request is answered, or during the tick or request that sees it.
//! - While a session runs, a tick saves a checkpoint of its active time
//!   when the last one is [`CHECKPOINT_INTERVAL`] old.
//! - A strict session starts only once its first snapshot is on disk; if it
//!   can't be saved, the start is refused with [`ProtocolError::Internal`]
//!   and the engine is left as it was. Any other failed save is logged,
//!   keeps the session in memory as it is, and is retried by every tick
//!   until it succeeds.
//! - The snapshot holds the id of the session's blocklist, in the same
//!   fixed slot and write ([`Store::save_bound_snapshot`]).
//! - On a store with no snapshot, creating the engine saves the idle one, so
//!   that the slot exists before any strict start; a stored snapshot is
//!   never overwritten by creating the engine (restoring it is T-2.7's).
//!
//! # Strict sessions and `RandomText`
//!
//! The core engine decides what a strict session refuses: stopping,
//! pausing, skipping, starting over it ([`EngineError::StrictSession`]), and
//! editing any blocklist ([`Engine::can_edit_blocklist`], T-1.6). Each refusal
//! is [`ProtocolError::StrictSessionLocked`]. Settings only hold the preset
//! of later sessions, so they can be changed during a strict session.
//!
//! A `RandomText` session gets a text from the operating system's secure
//! random source when it starts; stopping it needs that exact text,
//! compared in constant time, and is otherwise
//! [`ProtocolError::Validation`] on `text`. Pausing and skipping a break
//! need no text. No message carries the text to a client yet: that needs a
//! change to the protocol.
//!
//! # Logs
//!
//! Nothing here logs a blocklist, a setting, a block event's target or a
//! text, and a [`StoreError`] is logged by its own message only, never with
//! its source, which can quote SQLite's text about a planted file. A failing
//! checkpoint is logged once at `WARN` when it starts failing, and again
//! when it works, not on every retry.

use std::fmt;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use keitora_core::clock::{Clock, MonotonicInstant};
use keitora_core::engine::{
    Engine, EngineError, LockKind, LockReason as CoreLockReason, Phase as CorePhase,
    PomodoroPreset, Status,
};
use keitora_ipc::{
    BlockEvent as IpcBlockEvent, BlocklistSummary, Event, HistoryEntry, HistoryRange, Outcome,
    PageRequest, ProtocolError, Request, Response, SessionState, Settings as IpcSettings,
    TargetKind as IpcTargetKind, ValidatedBlocklist, ValidationKind,
};

use crate::pipe::RequestHandler;
use crate::store::{
    BlockEvent, BlocklistSummary as StoredSummary, SessionOutcome, SessionRecord, Settings, Store,
    StoreError, TargetKind,
};

mod stop_text;

use self::stop_text::StopText;

/// Logs a failed save of the snapshot: at `WARN` when saves start failing,
/// then at `DEBUG` while they keep failing, so that a tick retrying every
/// second doesn't fill the logs. `$failure` is a [`SaveFailure`];
/// `$warning` and `$still` are the two messages.
///
/// A macro, so that every place that saves logs from a line of its own: the
/// messages say what was being saved, and each line is its own `tracing`
/// callsite, whose cached interest no other path can change.
macro_rules! report_failed_save {
    ($failure:expr, $warning:literal, $still:literal) => {{
        let failure: &SaveFailure = &$failure;
        let error = describe(&failure.error);
        if failure.first {
            tracing::warn!(%error, $warning);
        } else {
            tracing::debug!(%error, $still);
        }
    }};
}

/// The longest a running session goes without a checkpoint of its active
/// time. After a crash or a power cut, the session resumes from its last
/// checkpoint, so it is longer by up to this much, never shorter.
pub const CHECKPOINT_INTERVAL: Duration = Duration::from_secs(15);

/// Seconds in a day, for the start of today.
const SECS_PER_DAY: i64 = 24 * 60 * 60;

/// How far back [`HistoryRange::Week`] looks.
const WEEK: Duration = Duration::from_secs(7 * 24 * 60 * 60);

/// How far back [`HistoryRange::Month`] looks.
const MONTH: Duration = Duration::from_secs(30 * 24 * 60 * 60);

/// Where the engine's events go: the pipe server's subscribers, in the
/// service.
pub trait EventSink: Send + Sync {
    /// Delivers one event to the subscribers. Never blocks on a client.
    fn publish(&self, event: Event);
}

/// Why a [`ServiceEngine`] couldn't be created.
#[derive(Debug, thiserror::Error)]
#[non_exhaustive]
pub enum EngineStartError {
    /// The stored snapshot couldn't be read. The engine isn't created, so
    /// that nothing overwrites a snapshot that may hold a strict session
    /// (T-2.7 decides how the service recovers).
    #[error("couldn't read the stored engine snapshot")]
    LoadSnapshot(#[source] StoreError),
}

/// The service's engine: the pomodoro engine, its store and its clock,
/// shared by the pipe server and the service's main loop.
///
/// See the [module documentation](self) for what it does.
pub struct ServiceEngine {
    /// The service's one clock.
    clock: Arc<dyn Clock + Send + Sync>,
    /// Where events go.
    events: Arc<dyn EventSink>,
    /// Everything that changes, behind one lock.
    state: Mutex<State>,
}

impl fmt::Debug for ServiceEngine {
    /// Shows nothing of the session or the store's data.
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.debug_struct("ServiceEngine").finish_non_exhaustive()
    }
}

/// What the engine's lock guards.
struct State {
    /// The database.
    store: Store,
    /// The pomodoro engine.
    engine: Engine,
    /// What the service knows of the session that the core engine doesn't
    /// tell: `Some` from a start until the end is observed.
    session: Option<Session>,
    /// The phase and cycle last published.
    shown: Shown,
    /// When the snapshot was last saved, and whether a save is owed.
    checkpoint: Checkpoint,
}

/// A started session, as the service sees it.
struct Session {
    /// The blocklist it enforces.
    blocklist_id: String,
    /// Its durations and cycles.
    preset: PomodoroPreset,
    /// How it is locked.
    lock: LockKind,
    /// When it started, by the wall clock, for the history.
    started_at: SystemTime,
    /// The text that stops a `RandomText` session.
    stop_text: Option<StopText>,
}

/// The part of the state whose change a tick publishes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Shown {
    /// The phase.
    phase: CorePhase,
    /// The cycle.
    cycle: u32,
}

impl Shown {
    /// No session.
    const IDLE: Self = Self {
        phase: CorePhase::Idle,
        cycle: 0,
    };

    /// The phase and cycle of `status`.
    fn of(status: &Status) -> Self {
        Self {
            phase: status.phase(),
            cycle: status.cycle(),
        }
    }
}

/// A save of the snapshot that failed, from [`ServiceEngine::save`]. The
/// save is then owed to the next tick.
#[derive(Debug)]
struct SaveFailure {
    /// The store's error.
    error: StoreError,
    /// Whether the save before this one worked: saves start failing here.
    first: bool,
}

/// The state of the checkpoints.
#[derive(Debug, Clone, Copy, Default)]
struct Checkpoint {
    /// When the last save succeeded, by the monotonic clock.
    saved_at: Option<MonotonicInstant>,
    /// Whether the last save failed, so the next tick tries again.
    owed: bool,
}

impl ServiceEngine {
    /// An idle engine on `store`, reading time from `clock` and publishing
    /// to `events`.
    ///
    /// If the store holds no snapshot, the idle one is saved, so that the
    /// snapshot's slot exists before any strict session starts (ADR-0013).
    /// If that save fails, as on a full disk, the engine is created all the
    /// same and later ticks try again. A stored snapshot is left as it is.
    ///
    /// # Errors
    ///
    /// [`EngineStartError::LoadSnapshot`] if the stored snapshot can't be
    /// read.
    pub fn new(
        store: Store,
        clock: Arc<dyn Clock + Send + Sync>,
        events: Arc<dyn EventSink>,
    ) -> Result<Self, EngineStartError> {
        let stored = store
            .load_bound_snapshot()
            .map_err(EngineStartError::LoadSnapshot)?;
        let engine = Self {
            clock,
            events,
            state: Mutex::new(State {
                store,
                engine: Engine::new(),
                session: None,
                shown: Shown::IDLE,
                checkpoint: Checkpoint::default(),
            }),
        };
        if stored.is_none() {
            let mut state = engine.lock();
            // The engine runs all the same; ticks retry the save.
            if let Err(failure) = engine.save(&mut state) {
                report_failed_save!(
                    failure,
                    "couldn't save the first, idle snapshot; ticks retry it",
                    "the first, idle snapshot still can't be saved"
                );
            }
        }
        Ok(engine)
    }

    /// Observes the clock: publishes a phase change or the end of the
    /// session since the last call, records an ended session in the
    /// history, and saves a checkpoint when one is due or owed. When it
    /// returns, all of that is done.
    pub fn tick(&self) {
        let mut state = self.lock();
        let status = self.observe(&mut state);
        let running = status.phase() != CorePhase::Idle && !status.is_paused();
        let due = state.checkpoint.saved_at.is_none_or(|saved_at| {
            self.clock.monotonic().saturating_duration_since(saved_at) >= CHECKPOINT_INTERVAL
        });
        if (state.checkpoint.owed || (running && due))
            && let Err(failure) = self.save(&mut state)
        {
            report_failed_save!(
                failure,
                "couldn't save a checkpoint; the session goes on in memory and ticks retry it",
                "a checkpoint still can't be saved"
            );
        }
    }

    /// Saves a checkpoint now if a session is active, or if a save is owed,
    /// after observing the clock like [`ServiceEngine::tick`]. The service
    /// calls it when it stops, so that a session resumes after a restart
    /// from where it was, not from the last periodic checkpoint.
    ///
    /// An idle engine saves nothing: a snapshot stored before the engine
    /// was created, which T-2.7 restores, is never overwritten by it.
    pub fn checkpoint(&self) {
        let mut state = self.lock();
        let status = self.observe(&mut state);
        if (state.checkpoint.owed || status.phase() != CorePhase::Idle)
            && let Err(failure) = self.save(&mut state)
        {
            report_failed_save!(
                failure,
                "couldn't save the last checkpoint",
                "the last checkpoint still can't be saved"
            );
        }
    }

    /// The engine's lock. A thread that panicked while holding it leaves
    /// it poisoned; the state is still used, as the core engine and the
    /// store change nothing on an error, and refusing every request would
    /// end nothing but the service's answers.
    fn lock(&self) -> MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Publishes and records what time did since the last observation, and
    /// returns the engine's status now.
    fn observe(&self, state: &mut State) -> Status {
        let status = state.engine.status(&*self.clock);
        if status.phase() == CorePhase::Idle {
            if let Some(session) = &state.session {
                let focus = total_focus(&session.preset);
                self.end_session(state, Outcome::Completed, focus);
            }
            return status;
        }
        let shown = Shown::of(&status);
        if shown != state.shown {
            state.shown = shown;
            self.events.publish(Event::StateChanged {
                state: session_state(state, &status),
            });
            if let Err(failure) = self.save(state) {
                report_failed_save!(
                    failure,
                    "couldn't save a phase change; the session goes on in memory and ticks retry it",
                    "a phase change still can't be saved"
                );
            }
        }
        status
    }

    /// Publishes the state after a request changed the session, and saves
    /// it.
    fn changed(&self, state: &mut State) {
        let status = state.engine.status(&*self.clock);
        state.shown = Shown::of(&status);
        self.events.publish(Event::StateChanged {
            state: session_state(state, &status),
        });
        if let Err(failure) = self.save(state) {
            report_failed_save!(
                failure,
                "couldn't save a change to the session; it goes on in memory and ticks retry it",
                "a change to the session still can't be saved"
            );
        }
    }

    /// Ends the service's record of the session: adds it to the history
    /// with `outcome` and `focus`, publishes the end and the idle state, and
    /// saves the idle snapshot. The core engine must be idle already.
    fn end_session(&self, state: &mut State, outcome: Outcome, focus: Duration) {
        let Some(session) = state.session.take() else {
            return;
        };
        let record = SessionRecord {
            started_at: session.started_at,
            blocklist_id: session.blocklist_id.clone(),
            preset: session.preset.clone(),
            lock: session.lock,
            outcome: match outcome {
                Outcome::Completed => SessionOutcome::Completed,
                Outcome::Stopped => SessionOutcome::Stopped,
            },
            focus,
        };
        if let Err(error) = state.store.record_session(&record) {
            tracing::warn!(
                "couldn't add a session to the history: {}",
                describe(&error)
            );
        }
        tracing::info!(outcome = outcome_name(outcome), "a session ended");
        self.events.publish(Event::SessionEnded { outcome });
        state.shown = Shown::IDLE;
        self.events.publish(Event::StateChanged {
            state: idle_state(),
        });
        if let Err(failure) = self.save(state) {
            report_failed_save!(
                failure,
                "couldn't save the end of a session; ticks retry it",
                "the end of a session still can't be saved"
            );
        }
    }

    /// Saves the engine's snapshot, bound to the session's blocklist. The
    /// caller logs a failure, with [`report_failed_save!`].
    ///
    /// # Errors
    ///
    /// A [`SaveFailure`] with the store's error; the save is then owed to
    /// the next tick.
    fn save(&self, state: &mut State) -> Result<(), SaveFailure> {
        let now = self.clock.monotonic();
        let snapshot = state.engine.snapshot(&*self.clock);
        let blocklist_id = match &state.session {
            Some(session) if !snapshot.is_idle() => Some(session.blocklist_id.as_str()),
            _ => None,
        };
        match state.store.save_bound_snapshot(&snapshot, blocklist_id) {
            Ok(()) => {
                if state.checkpoint.owed {
                    tracing::info!("the engine snapshot is saved again");
                }
                state.checkpoint = Checkpoint {
                    saved_at: Some(now),
                    owed: false,
                };
                Ok(())
            }
            Err(error) => {
                let first = !state.checkpoint.owed;
                state.checkpoint.owed = true;
                Err(SaveFailure { error, first })
            }
        }
    }

    // -----------------------------------------------------------------------
    // Session requests
    // -----------------------------------------------------------------------

    /// `StartSession`.
    fn start(
        &self,
        state: &mut State,
        status: &Status,
        blocklist_id: String,
        preset: PomodoroPreset,
        lock: LockKind,
    ) -> Result<Response, ProtocolError> {
        if status.phase() != CorePhase::Idle {
            return Err(match state.engine.can_stop(&*self.clock) {
                Err(reason) => strict_locked(reason),
                Ok(()) => ProtocolError::InvalidState,
            });
        }
        match state.store.blocklist(&blocklist_id) {
            Ok(Some(_)) => {}
            Ok(None) => return Err(ProtocolError::NotFound),
            Err(error) => return Err(internal("couldn't read a blocklist", &error)),
        }
        let stop_text = match lock {
            LockKind::RandomText { len } => Some(StopText::generate(len).map_err(|error| {
                tracing::error!("couldn't generate the text that stops a session: {error}");
                ProtocolError::Internal
            })?),
            _ => None,
        };

        let before = state.engine.clone();
        let checkpoint_before = state.checkpoint;
        state
            .engine
            .start(preset.clone(), lock, &*self.clock)
            .map_err(engine_error)?;
        state.session = Some(Session {
            blocklist_id,
            preset,
            lock,
            started_at: self.clock.wall(),
            stop_text,
        });
        let strict = state.engine.can_stop(&*self.clock).is_err();
        if let Err(failure) = self.save(state) {
            if strict {
                // #48: a strict session runs only once a reboot can restore
                // it. The engine was idle, or its session over, and goes
                // back to exactly that.
                state.engine = before;
                state.session = None;
                // Nothing about the earlier state changed, so nothing is
                // owed for it: a tick must not save the idle engine over a
                // stored snapshot.
                state.checkpoint = checkpoint_before;
                tracing::warn!(
                    error = %describe(&failure.error),
                    "a strict session was refused: its snapshot couldn't be saved"
                );
                return Err(ProtocolError::Internal);
            }
            report_failed_save!(
                failure,
                "couldn't save a new session; it runs in memory and ticks retry the save",
                "a new session still can't be saved"
            );
        }
        tracing::info!(lock = lock_name(lock), "a session started");
        self.changed_without_saving(state);
        Ok(Response::Ok)
    }

    /// Publishes the state after a change whose snapshot is already saved.
    fn changed_without_saving(&self, state: &mut State) {
        let status = state.engine.status(&*self.clock);
        state.shown = Shown::of(&status);
        self.events.publish(Event::StateChanged {
            state: session_state(state, &status),
        });
    }

    /// `StopSession`.
    fn stop(
        &self,
        state: &mut State,
        status: &Status,
        text: Option<&str>,
    ) -> Result<Response, ProtocolError> {
        let Some(session) = state.session.as_ref() else {
            return Err(ProtocolError::InvalidState);
        };
        if let Err(reason) = state.engine.can_stop(&*self.clock) {
            return Err(strict_locked(reason));
        }
        if let Some(expected) = &session.stop_text {
            let kind = match text {
                None | Some("") => Some(ValidationKind::Empty),
                Some(typed) if !expected.matches(typed) => Some(ValidationKind::Invalid),
                Some(_) => None,
            };
            if let Some(kind) = kind {
                return Err(ProtocolError::Validation {
                    field: "text".to_owned(),
                    kind,
                });
            }
        }
        let focus = focus_run(&session.preset, status);
        state.engine.stop(&*self.clock).map_err(engine_error)?;
        self.end_session(state, Outcome::Stopped, focus);
        Ok(Response::Ok)
    }

    /// `SkipBreak`. Skipping the long break ends the session, every focus
    /// having run: it is recorded as completed.
    fn skip_break(&self, state: &mut State) -> Result<Response, ProtocolError> {
        state
            .engine
            .skip_break(&*self.clock)
            .map_err(engine_error)?;
        let ended = state.engine.status(&*self.clock).phase() == CorePhase::Idle;
        match state.session.as_ref() {
            Some(session) if ended => {
                let focus = total_focus(&session.preset);
                self.end_session(state, Outcome::Completed, focus);
            }
            _ => self.changed(state),
        }
        Ok(Response::Ok)
    }

    // -----------------------------------------------------------------------
    // Blocklists, settings, history and block events
    // -----------------------------------------------------------------------

    /// `UpsertBlocklist`.
    fn upsert_blocklist(
        &self,
        state: &mut State,
        blocklist: ValidatedBlocklist,
    ) -> Result<Response, ProtocolError> {
        state
            .engine
            .can_edit_blocklist(&*self.clock)
            .map_err(strict_locked)?;
        let blocklist = blocklist.into_blocklist();
        state
            .store
            .put_blocklist(&blocklist)
            .map_err(upsert_error)?;
        self.events
            .publish(Event::BlocklistChanged { id: blocklist.id });
        Ok(Response::Ok)
    }

    /// `DeleteBlocklist`.
    fn delete_blocklist(&self, state: &mut State, id: String) -> Result<Response, ProtocolError> {
        state
            .engine
            .can_edit_blocklist(&*self.clock)
            .map_err(strict_locked)?;
        match state.store.delete_blocklist(&id) {
            Ok(true) => {
                self.events.publish(Event::BlocklistChanged { id });
                Ok(Response::Ok)
            }
            Ok(false) => Err(ProtocolError::NotFound),
            Err(error) => Err(internal("couldn't delete a blocklist", &error)),
        }
    }

    /// `GetHistory`.
    fn history(
        &self,
        state: &State,
        range: HistoryRange,
        page: PageRequest,
    ) -> Result<Response, ProtocolError> {
        let since = since(range, self.clock.wall());
        let found = state
            .store
            .history_page(since, page.offset(), page.limit())
            .map_err(|error| internal("couldn't read the history", &error))?;
        let next_offset = next_offset(page.offset(), found.items.len(), found.more);
        Ok(Response::History {
            items: found.items.into_iter().map(history_entry).collect(),
            next_offset,
        })
    }
}

impl RequestHandler for ServiceEngine {
    fn handle(&self, request: Request) -> Response {
        let mut guard = self.lock();
        let state = &mut *guard;
        let status = self.observe(state);
        let answer =
            match request {
                Request::GetState => Ok(Response::State {
                    state: session_state(state, &status),
                }),
                Request::StartSession {
                    blocklist_id,
                    preset,
                    lock,
                } => self.start(state, &status, blocklist_id, preset, lock),
                Request::StopSession { text } => self.stop(state, &status, text.as_deref()),
                Request::PauseSession => state
                    .engine
                    .pause(&*self.clock)
                    .map_err(engine_error)
                    .map(|()| {
                        self.changed(state);
                        Response::Ok
                    }),
                Request::ResumeSession => state
                    .engine
                    .resume(&*self.clock)
                    .map_err(engine_error)
                    .map(|()| {
                        self.changed(state);
                        Response::Ok
                    }),
                Request::SkipBreak => self.skip_break(state),
                Request::ListBlocklists { page } => list_blocklists(state, page),
                Request::GetBlocklist { id } => get_blocklist(state, &id),
                Request::UpsertBlocklist { blocklist } => self.upsert_blocklist(state, blocklist),
                Request::DeleteBlocklist { id } => self.delete_blocklist(state, id),
                Request::GetSettings => state
                    .store
                    .settings()
                    .map(|settings| Response::Settings {
                        settings: IpcSettings {
                            default_preset: settings.default_preset,
                        },
                    })
                    .map_err(|error| internal("couldn't read the settings", &error)),
                Request::UpdateSettings { settings } => state
                    .store
                    .put_settings(&Settings {
                        default_preset: settings.default_preset,
                    })
                    .map(|()| Response::Ok)
                    .map_err(|error| internal("couldn't store the settings", &error)),
                // T-2.8 lists the installed applications; until then there are
                // none to offer.
                Request::ListInstalledApps { .. } => Ok(Response::InstalledApps {
                    items: Vec::new(),
                    next_offset: None,
                }),
                Request::GetHistory { range, page } => self.history(state, range, page),
                Request::GetBlockEvents { page } => block_events(state, page),
                // The pipe server answers `Subscribe` itself, and a request this
                // version doesn't know can't be served.
                _ => {
                    tracing::warn!("a request the engine doesn't serve reached it");
                    Err(ProtocolError::Internal)
                }
            };
        answer.unwrap_or_else(|error| Response::Error { error })
    }
}

// ---------------------------------------------------------------------------
// Reads that need no clock
// ---------------------------------------------------------------------------

/// `ListBlocklists`.
fn list_blocklists(state: &State, page: PageRequest) -> Result<Response, ProtocolError> {
    let found = state
        .store
        .blocklist_summaries_page(page.offset(), page.limit())
        .map_err(|error| internal("couldn't read the blocklists", &error))?;
    let next_offset = next_offset(page.offset(), found.items.len(), found.more);
    Ok(Response::Blocklists {
        items: found.items.into_iter().map(summary).collect(),
        next_offset,
    })
}

/// `GetBlocklist`.
fn get_blocklist(state: &State, id: &str) -> Result<Response, ProtocolError> {
    let blocklist = state
        .store
        .blocklist(id)
        .map_err(|error| internal("couldn't read a blocklist", &error))?
        .ok_or(ProtocolError::NotFound)?;
    // The store checked it already; this is the type that says so.
    let blocklist = ValidatedBlocklist::new(blocklist).map_err(|_| {
        tracing::error!("a stored blocklist failed validation");
        ProtocolError::Internal
    })?;
    Ok(Response::Blocklist { blocklist })
}

/// `GetBlockEvents`.
fn block_events(state: &State, page: PageRequest) -> Result<Response, ProtocolError> {
    let found = state
        .store
        .block_events_page(page.offset(), page.limit())
        .map_err(|error| internal("couldn't read the block events", &error))?;
    let next_offset = next_offset(page.offset(), found.items.len(), found.more);
    Ok(Response::BlockEvents {
        items: found.items.into_iter().map(block_event).collect(),
        next_offset,
    })
}

// ---------------------------------------------------------------------------
// Conversions and small rules
// ---------------------------------------------------------------------------

/// What `GetState` reports for `status`.
fn session_state(state: &State, status: &Status) -> SessionState {
    match &state.session {
        Some(session) if status.phase() != CorePhase::Idle => SessionState {
            phase: status.phase().into(),
            cycle: status.cycle(),
            remaining_ms: millis(status.remaining()),
            paused: status.is_paused(),
            lock: Some(session.lock),
            blocklist_id: Some(session.blocklist_id.clone()),
        },
        _ => idle_state(),
    }
}

/// The state of an engine with no session.
fn idle_state() -> SessionState {
    SessionState {
        phase: CorePhase::Idle.into(),
        cycle: 0,
        remaining_ms: 0,
        paused: false,
        lock: None,
        blocklist_id: None,
    }
}

/// The protocol's error for a request the core engine refused.
fn engine_error(error: EngineError) -> ProtocolError {
    match error {
        EngineError::StrictSession => strict_locked(CoreLockReason::UntilTimerEnds),
        // `keitora-ipc` validates the lock before a request gets here.
        EngineError::InvalidLock(_) => ProtocolError::Validation {
            field: "lock".to_owned(),
            kind: ValidationKind::OutOfRange,
        },
        // No session, one already running, already paused, not paused, or
        // not a break; and any refusal the core engine adds later.
        _ => ProtocolError::InvalidState,
    }
}

/// The protocol's error for a blocklist the store didn't store: a new one
/// beyond [`MAX_BLOCKLISTS`](crate::store::MAX_BLOCKLISTS) is the client's
/// to fix; anything else is the service's.
fn upsert_error(error: StoreError) -> ProtocolError {
    match error {
        StoreError::TooManyBlocklists { .. } => ProtocolError::Validation {
            field: "blocklist".to_owned(),
            kind: ValidationKind::TooMany,
        },
        error => internal("couldn't store a blocklist", &error),
    }
}

/// The refusal of a strict session.
fn strict_locked(reason: CoreLockReason) -> ProtocolError {
    ProtocolError::StrictSessionLocked {
        reason: reason.into(),
    }
}

/// Logs a store failure with `what` failed, and gives the client
/// [`ProtocolError::Internal`].
fn internal(what: &str, error: &StoreError) -> ProtocolError {
    tracing::error!("{what}: {}", describe(error));
    ProtocolError::Internal
}

/// A store error for the logs: its own message, which holds no stored value,
/// and for an SQLite failure its numeric code, but never SQLite's text,
/// which can quote a name from a planted file.
pub(crate) fn describe(error: &StoreError) -> String {
    match error {
        StoreError::Database {
            source: rusqlite::Error::SqliteFailure(failure, _),
        } => format!("{error} (SQLite code {})", failure.extended_code),
        _ => error.to_string(),
    }
}

/// The focus time a session has run, given its status now: every focus
/// before the current cycle's, which can't be skipped, and the part of the
/// current one that ran.
fn focus_run(preset: &PomodoroPreset, status: &Status) -> Duration {
    let focus = preset.focus();
    let earlier = focus.saturating_mul(status.cycle().saturating_sub(1));
    let current = match status.phase() {
        CorePhase::Focus => focus.saturating_sub(status.remaining()),
        CorePhase::ShortBreak | CorePhase::LongBreak => focus,
        CorePhase::Idle => Duration::ZERO,
    };
    earlier.saturating_add(current)
}

/// The focus time of a session that ran to its end.
fn total_focus(preset: &PomodoroPreset) -> Duration {
    preset.focus().saturating_mul(preset.cycles())
}

/// Where a history range starts, given the wall time `now`; `None` for no
/// start.
fn since(range: HistoryRange, now: SystemTime) -> Option<SystemTime> {
    match range {
        HistoryRange::Today => {
            let offset = keitora_sys::time::local_utc_offset_minutes().unwrap_or_else(|| {
                tracing::warn!("couldn't read the time zone; today starts at midnight UTC");
                0
            });
            start_of_day(now, offset)
        }
        HistoryRange::Week => now.checked_sub(WEEK),
        HistoryRange::Month => now.checked_sub(MONTH),
        HistoryRange::All => None,
    }
}

/// The last midnight at or before `now`, in a time zone `offset_minutes`
/// ahead of UTC; `None` if it can't be represented.
fn start_of_day(now: SystemTime, offset_minutes: i32) -> Option<SystemTime> {
    let secs = match now.duration_since(UNIX_EPOCH) {
        Ok(after) => i64::try_from(after.as_secs()).ok()?,
        // Before 1970, whole seconds rounded towards the past.
        Err(before) => {
            let before = before.duration();
            let whole = i64::try_from(before.as_secs()).ok()?;
            let rounded = if before.subsec_nanos() > 0 {
                whole.checked_add(1)?
            } else {
                whole
            };
            rounded.checked_neg()?
        }
    };
    let offset = i64::from(offset_minutes).checked_mul(60)?;
    let local = secs.checked_add(offset)?;
    let local_midnight = local
        .checked_div_euclid(SECS_PER_DAY)?
        .checked_mul(SECS_PER_DAY)?;
    let midnight = local_midnight.checked_sub(offset)?;
    let distance = Duration::from_secs(midnight.unsigned_abs());
    if midnight >= 0 {
        UNIX_EPOCH.checked_add(distance)
    } else {
        UNIX_EPOCH.checked_sub(distance)
    }
}

/// The offset of the page after one of `count` items at `offset`: `None`
/// when no item follows it, or when its offset wouldn't fit in a `u32`.
fn next_offset(offset: u32, count: usize, more: bool) -> Option<u32> {
    if !more {
        return None;
    }
    u32::try_from(count)
        .ok()
        .and_then(|count| offset.checked_add(count))
}

/// A stored blocklist's summary as `ListBlocklists` lists it.
fn summary(stored: StoredSummary) -> BlocklistSummary {
    BlocklistSummary {
        id: stored.id,
        name: stored.name,
        color: stored.color,
    }
}

/// A stored session as `GetHistory` reports it.
fn history_entry(record: SessionRecord) -> HistoryEntry {
    HistoryEntry {
        started_at_unix_ms: unix_millis(record.started_at),
        blocklist_id: record.blocklist_id,
        preset: record.preset,
        lock: record.lock,
        outcome: match record.outcome {
            SessionOutcome::Completed => Outcome::Completed,
            SessionOutcome::Stopped => Outcome::Stopped,
        },
        focus_ms: millis(record.focus),
    }
}

/// A stored block event as `GetBlockEvents` reports it.
fn block_event(event: BlockEvent) -> IpcBlockEvent {
    IpcBlockEvent {
        at_unix_ms: unix_millis(event.at),
        kind: match event.kind {
            TargetKind::Site => IpcTargetKind::Site,
            TargetKind::App => IpcTargetKind::App,
        },
        target: event.target,
    }
}

/// `duration` in whole milliseconds, at most `u64::MAX`.
fn millis(duration: Duration) -> u64 {
    u64::try_from(duration.as_millis()).unwrap_or(u64::MAX)
}

/// `time` in milliseconds since the Unix epoch; 0 for a time before it,
/// which only a wall clock set before 1970 gives.
fn unix_millis(time: SystemTime) -> u64 {
    time.duration_since(UNIX_EPOCH).map_or(0, millis)
}

/// A lock's kind, for the logs; never its text.
fn lock_name(lock: LockKind) -> &'static str {
    match lock {
        LockKind::None => "none",
        LockKind::UntilTimerEnds => "until_timer_ends",
        LockKind::RandomText { .. } => "random_text",
        // `LockKind` is `#[non_exhaustive]`.
        _ => "other",
    }
}

/// An outcome, for the logs.
fn outcome_name(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Completed => "completed",
        Outcome::Stopped => "stopped",
    }
}

#[cfg(test)]
mod tests {
    use std::path::PathBuf;

    use keitora_core::engine::LockKind;
    use keitora_core::model::{Blocklist, SitePattern, Target};

    use super::*;

    /// Every event published, in order.
    #[derive(Debug, Default)]
    struct Recorder(Mutex<Vec<Event>>);

    impl Recorder {
        fn take(&self) -> Vec<Event> {
            std::mem::take(&mut *self.0.lock().expect("not poisoned"))
        }
    }

    impl EventSink for Recorder {
        fn publish(&self, event: Event) {
            self.0.lock().expect("not poisoned").push(event);
        }
    }

    /// An engine on a fresh store in a folder of its own, which holds the
    /// blocklist `work`; the folder is deleted when dropped.
    struct Harness {
        dir: PathBuf,
        clock: Arc<HandClock>,
        events: Arc<Recorder>,
        engine: ServiceEngine,
    }

    impl Drop for Harness {
        fn drop(&mut self) {
            let _ = std::fs::remove_dir_all(&self.dir);
        }
    }

    fn harness(test_name: &str) -> Harness {
        let dir = std::env::temp_dir()
            .join("keitora-engine-unit-tests")
            .join(format!("{test_name}-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let mut store = Store::open(&dir).expect("opens");
        store
            .put_blocklist(&Blocklist {
                id: "work".to_owned(),
                name: "Work".to_owned(),
                color: "#112233".to_owned(),
                targets: vec![Target::Site {
                    pattern: SitePattern::new("example.com"),
                }],
                exceptions: Vec::new(),
                block_during_breaks: false,
            })
            .expect("stores");
        let clock = Arc::new(HandClock::default());
        let events = Arc::new(Recorder::default());
        let engine = ServiceEngine::new(
            store,
            Arc::<HandClock>::clone(&clock),
            Arc::<Recorder>::clone(&events),
        )
        .expect("starts");
        Harness {
            dir,
            clock,
            events,
            engine,
        }
    }

    impl Harness {
        fn start(&self, preset: PomodoroPreset, lock: LockKind) {
            assert_eq!(
                self.engine.handle(Request::StartSession {
                    blocklist_id: "work".to_owned(),
                    preset,
                    lock,
                }),
                Response::Ok
            );
        }

        fn history(&self) -> Vec<SessionRecord> {
            self.engine
                .lock()
                .store
                .history_page(None, 0, 100)
                .expect("reads")
                .items
        }

        fn stored(&self) -> Option<crate::store::BoundSnapshot> {
            Store::open(&self.dir)
                .expect("opens")
                .load_bound_snapshot()
                .expect("reads")
        }
    }

    #[test]
    fn a_random_text_session_stops_with_its_exact_text_only() {
        let h = harness("random-text-stop");
        h.start(PomodoroPreset::default(), LockKind::RandomText { len: 16 });
        let text = h
            .engine
            .lock()
            .session
            .as_ref()
            .and_then(|session| session.stop_text.as_ref())
            .map(|text| text.as_str().to_owned())
            .expect("a random text session has a text");
        assert_eq!(text.len(), 16);

        let wrong = text.to_uppercase();
        assert!(matches!(
            h.engine.handle(Request::StopSession { text: Some(wrong) }),
            Response::Error {
                error: ProtocolError::Validation { .. }
            }
        ));
        h.clock.advance(Duration::from_secs(60));
        h.events.take();
        assert_eq!(
            h.engine.handle(Request::StopSession { text: Some(text) }),
            Response::Ok
        );
        assert_eq!(
            h.events.take(),
            vec![
                Event::SessionEnded {
                    outcome: Outcome::Stopped
                },
                Event::StateChanged {
                    state: idle_state()
                },
            ]
        );
        let history = h.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].outcome, SessionOutcome::Stopped);
        assert_eq!(history[0].focus, Duration::from_secs(60));
    }

    #[test]
    fn skipping_the_long_break_completes_the_session() {
        let h = harness("skip-long-break");
        let preset = PomodoroPreset::new(1, 1, 5, 2).expect("valid");
        h.start(preset, LockKind::None);
        // Focus, short break, focus: into the long break by a minute.
        h.clock.advance(Duration::from_secs(4 * 60));
        h.engine.tick();
        h.events.take();
        assert_eq!(h.engine.handle(Request::SkipBreak), Response::Ok);
        assert_eq!(
            h.events.take(),
            vec![
                Event::SessionEnded {
                    outcome: Outcome::Completed
                },
                Event::StateChanged {
                    state: idle_state()
                },
            ]
        );
        let history = h.history();
        assert_eq!(history.len(), 1);
        assert_eq!(history[0].outcome, SessionOutcome::Completed);
        assert_eq!(history[0].focus, Duration::from_secs(2 * 60));
        assert_eq!(
            h.stored().map(|bound| bound.state.is_idle()),
            Some(true),
            "the end is saved at once"
        );
    }

    #[test]
    fn checkpoint_saves_a_running_session_with_its_blocklist() {
        let h = harness("checkpoint");
        h.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
        h.clock.advance(Duration::from_secs(5));
        h.engine.checkpoint();
        let stored = h.stored().expect("a snapshot is stored");
        assert_eq!(stored.blocklist_id.as_deref(), Some("work"));
        let expected = {
            let clock = HandClock::default();
            let mut engine = Engine::new();
            engine
                .start(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock)
                .expect("starts");
            clock.advance(Duration::from_secs(5));
            engine.snapshot(&clock)
        };
        assert_eq!(stored.state, expected);
    }

    #[test]
    fn checkpoint_of_an_idle_engine_keeps_a_stored_session() {
        let h = harness("checkpoint-idle");
        h.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
        let stored = h.stored();
        // A new engine on the same store is idle until T-2.7 restores it.
        let second = ServiceEngine::new(
            Store::open(&h.dir).expect("opens"),
            Arc::new(HandClock::default()),
            Arc::new(Recorder::default()),
        )
        .expect("starts");
        second.tick();
        second.checkpoint();
        assert_eq!(h.stored(), stored);
    }

    #[test]
    fn subscribe_never_reaches_a_session() {
        let h = harness("subscribe");
        assert_eq!(
            h.engine.handle(Request::Subscribe),
            Response::Error {
                error: ProtocolError::Internal
            }
        );
        assert_eq!(h.events.take(), Vec::new());
    }

    fn at(secs: i64) -> SystemTime {
        let distance = Duration::from_secs(secs.unsigned_abs());
        if secs >= 0 {
            UNIX_EPOCH.checked_add(distance)
        } else {
            UNIX_EPOCH.checked_sub(distance)
        }
        .expect("a valid wall time")
    }

    #[test]
    fn today_starts_at_local_midnight() {
        // 2027-01-15T08:00:00Z.
        let now = at(1_800_000_000);
        let utc_midnight = 1_800_000_000 - 8 * 3600;
        assert_eq!(start_of_day(now, 0), Some(at(utc_midnight)));
        // UTC+2: 10:00 local, so midnight was 08:00 earlier, at 22:00Z.
        assert_eq!(start_of_day(now, 120), Some(at(utc_midnight - 2 * 3600)));
        // UTC-10: 22:00 the day before, so midnight was at 10:00Z that day.
        assert_eq!(
            start_of_day(now, -600),
            Some(at(utc_midnight - 24 * 3600 + 10 * 3600))
        );
        // Exactly midnight is the start of its own day.
        assert_eq!(start_of_day(at(utc_midnight), 0), Some(at(utc_midnight)));
    }

    #[test]
    fn today_before_1970_is_still_the_day_before_now() {
        // 1969-12-31T23:59:59.5Z.
        let now = UNIX_EPOCH - Duration::from_millis(500);
        assert_eq!(start_of_day(now, 0), Some(at(-SECS_PER_DAY)));
    }

    #[test]
    fn focus_counts_whole_earlier_focuses_and_the_current_one() {
        let preset = PomodoroPreset::default();
        let clock = HandClock::default();
        let mut engine = Engine::new();
        engine
            .start(preset.clone(), LockKind::None, &clock)
            .expect("starts");
        let status = engine.status(&clock);
        assert_eq!(focus_run(&preset, &status), Duration::ZERO);
        clock.advance(Duration::from_secs(10 * 60));
        assert_eq!(
            focus_run(&preset, &engine.status(&clock)),
            Duration::from_secs(10 * 60)
        );
        // Into the second focus by 3 minutes, past the first short break.
        clock.advance(Duration::from_secs(15 * 60 + 5 * 60 + 3 * 60));
        assert_eq!(
            focus_run(&preset, &engine.status(&clock)),
            Duration::from_secs(28 * 60)
        );
        assert_eq!(total_focus(&preset), Duration::from_secs(100 * 60));
    }

    #[test]
    fn a_next_offset_past_u32_is_the_end() {
        assert_eq!(next_offset(u32::MAX - 1, 1, true), Some(u32::MAX));
        assert_eq!(next_offset(u32::MAX, 1, true), None);
        assert_eq!(next_offset(5, 3, false), None);
    }

    #[test]
    fn engine_refusals_map_to_protocol_errors() {
        assert_eq!(
            engine_error(EngineError::StrictSession),
            ProtocolError::StrictSessionLocked {
                reason: keitora_ipc::LockReason::UntilTimerEnds,
            }
        );
        for error in [
            EngineError::NoSession,
            EngineError::AlreadyRunning,
            EngineError::AlreadyPaused,
            EngineError::NotPaused,
            EngineError::NotABreak,
        ] {
            assert_eq!(engine_error(error), ProtocolError::InvalidState);
        }
    }

    #[test]
    fn a_store_error_is_described_without_sqlite_text() {
        let error = StoreError::Database {
            source: rusqlite::Error::SqliteFailure(
                rusqlite::ffi::Error::new(1),
                Some("no such table: zq_planted_name".to_owned()),
            ),
        };
        let described = describe(&error);
        assert!(!described.contains("zq_planted_name"), "{described}");
        assert!(described.contains("SQLite code 1"), "{described}");
    }

    #[test]
    fn times_before_1970_and_huge_durations_saturate() {
        assert_eq!(unix_millis(UNIX_EPOCH - Duration::from_secs(1)), 0);
        assert_eq!(unix_millis(at(2)), 2000);
        assert_eq!(millis(Duration::MAX), u64::MAX);
    }

    /// A clock moved by hand. The crate's `clippy.toml` bans `FakeClock`,
    /// and these tests need nothing more than this.
    #[derive(Debug, Default)]
    struct HandClock {
        nanos: std::sync::atomic::AtomicU64,
    }

    impl HandClock {
        fn advance(&self, by: Duration) {
            let by = u64::try_from(by.as_nanos()).expect("small");
            self.nanos
                .fetch_add(by, std::sync::atomic::Ordering::SeqCst);
        }
    }

    impl Clock for HandClock {
        fn monotonic(&self) -> MonotonicInstant {
            let nanos = self.nanos.load(std::sync::atomic::Ordering::SeqCst);
            MonotonicInstant::from_duration_since_origin(Duration::from_nanos(nanos))
        }

        fn wall(&self) -> SystemTime {
            UNIX_EPOCH
        }
    }

    #[test]
    fn a_refused_strict_start_owes_no_save_over_the_stored_snapshot() {
        let h = harness("refused-strict-start");
        h.start(PomodoroPreset::default(), LockKind::UntilTimerEnds);
        let stored = h.stored();
        assert!(stored.as_ref().is_some_and(|bound| !bound.state.is_idle()));

        // The service starts again; its engine is idle until T-2.7 restores
        // the stored session.
        let second = ServiceEngine::new(
            Store::open(&h.dir).expect("opens"),
            Arc::new(HandClock::default()),
            Arc::new(Recorder::default()),
        )
        .expect("starts");
        {
            // Another connection holds the write lock: the start's save
            // fails once SQLite's busy timeout runs out.
            let lock = rusqlite::Connection::open(h.dir.join(crate::store::DB_FILE_NAME))
                .expect("opens the file directly");
            lock.execute_batch("BEGIN IMMEDIATE")
                .expect("takes the lock");
            assert_eq!(
                second.handle(Request::StartSession {
                    blocklist_id: "work".to_owned(),
                    preset: PomodoroPreset::default(),
                    lock: LockKind::UntilTimerEnds,
                }),
                Response::Error {
                    error: ProtocolError::Internal
                }
            );
            lock.execute_batch("ROLLBACK").expect("releases the lock");
        }
        assert!(!second.lock().checkpoint.owed);
        second.tick();
        assert_eq!(h.stored(), stored, "the idle engine saved nothing over it");
    }

    #[test]
    fn a_new_blocklist_beyond_the_cap_is_a_validation_error() {
        assert_eq!(
            upsert_error(StoreError::TooManyBlocklists {
                max: crate::store::MAX_BLOCKLISTS,
            }),
            ProtocolError::Validation {
                field: "blocklist".to_owned(),
                kind: ValidationKind::TooMany,
            }
        );
        assert_eq!(
            upsert_error(StoreError::WriteNotApplied { what: "blocklist" }),
            ProtocolError::Internal
        );
    }
}
