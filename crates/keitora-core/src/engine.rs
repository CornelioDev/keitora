// SPDX-License-Identifier: MIT OR Apache-2.0

//! The pomodoro state machine: an [`Engine`] runs one session at a time, on
//! the durations and number of cycles of a [`PomodoroPreset`].
//!
//! # The shape of a session
//!
//! A session has [`PomodoroPreset::cycles`] focus phases. Each focus but the
//! last is followed by a short break, and the last by the long break. When the
//! long break ends, the session is over and the engine is idle again. With
//! the default preset (25 / 5 / 15 minutes, 4 cycles):
//!
//! ```text
//! Idle → Focus 1 → Short 1 → Focus 2 → Short 2 → Focus 3 → Short 3 → Focus 4 → Long 4 → Idle
//! ```
//!
//! The number after each phase is what [`Status::cycle`] reports: a break
//! keeps the number of the focus it follows.
//!
//! Every phase starts on its own the instant the previous one runs out;
//! nothing waits for the user. A phase ends exactly when its remaining time
//! reaches zero, so at that instant the next phase is already current, with
//! its full duration.
//!
//! # Scheduling
//!
//! The schedule is fixed when the session starts: the engine doesn't tick or
//! need to be watched. It remembers how much *active* time the session has
//! run (time not spent paused), and [`Engine::status`] works out the phase,
//! cycle and remaining time from that alone, with a few divisions rather than
//! a loop over phases. Observed late, the engine has therefore moved through
//! every phase that elapsed in between, as if it had been watching all along.
//!
//! Only two things change the schedule:
//!
//! - A **pause** ([`Engine::pause`], [`Engine::resume`]) freezes the session;
//!   the rest of the schedule moves later by however long it lasted.
//! - **Skipping a break** ([`Engine::skip_break`]) ends a short break at once
//!   and starts the next focus with its full duration; the schedule continues
//!   from there. Skipping the long break ends the session. A focus can't be
//!   skipped.
//!
//! # Time
//!
//! Every method takes the [`Clock`] last and reads only its
//! [monotonic](Clock::monotonic) reading, never the wall clock: changing the
//! system time, forwards or backwards, neither ends, shortens nor extends a
//! phase (`docs/threat-model.md`, "Change the system clock"). A monotonic
//! reading never goes backwards; if a clock returned an instant earlier than
//! one it gave before, the engine would count that as no time passing, not
//! as negative time. All arithmetic saturates instead of overflowing, so no
//! clock reading can make the engine panic.
//!
//! # Strict sessions
//!
//! A session started with [`LockKind::UntilTimerEnds`] is strict: it is active
//! until its timer ends, in every phase including the breaks. While it is,
//! the engine refuses to stop it, pause it, skip a break or start another
//! session over it ([`EngineError::StrictSession`]), and
//! [`Engine::can_stop`] and [`Engine::can_edit_blocklist`] say no with a
//! [`LockReason`]. The instant its timer ends, all of that is allowed again.
//!
//! [`LockKind::RandomText`] is friction, not a lock: to the engine its session
//! is as free as an unlocked one. Typing the text is checked by the layer
//! above, which owns a random source; core only bounds its length
//! ([`LockKind::validate`]). [`LockKind`] and [`EngineError`] are
//! `#[non_exhaustive]`.
//!
//! # Snapshots
//!
//! [`Engine::snapshot`] writes down a session as an [`EngineState`], which
//! serializes with serde, and [`Engine::restore`] turns one back into an
//! engine, after a reboot for example. The snapshot holds the session's
//! active time, not a clock reading, so the restored session continues with
//! exactly the time it had left; time between the two is never counted.
//! [`Engine::restore`] is the only way from stored data to an engine, and it
//! rejects anything [`Engine::snapshot`] couldn't have written, such as a
//! strict session that is paused. `Engine` itself doesn't implement
//! `Deserialize`.
//!
//! ```
//! use std::time::{Duration, UNIX_EPOCH};
//!
//! use keitora_core::clock::FakeClock;
//! use keitora_core::engine::{Engine, EngineState, LockKind, PomodoroPreset};
//!
//! let clock = FakeClock::new(UNIX_EPOCH);
//! let mut engine = Engine::new();
//! engine.start(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock)?;
//! clock.advance(Duration::from_secs(10 * 60))?;
//! let stored = serde_json::to_string(&engine.snapshot(&clock))?;
//!
//! // After a reboot, on a new clock: 15 minutes of focus are still left.
//! let clock = FakeClock::new(UNIX_EPOCH);
//! let state: EngineState = serde_json::from_str(&stored)?;
//! let engine = Engine::restore(state, &clock)?;
//! assert_eq!(engine.status(&clock).remaining(), Duration::from_secs(15 * 60));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

use std::fmt;
use std::time::Duration;

use serde::de::Error as _;
use serde::{Deserialize, Deserializer, Serialize, Serializer};

use crate::clock::{Clock, MonotonicInstant};

#[cfg(test)]
mod tests;

/// The shortest a focus or a break may be, in minutes.
pub const MIN_DURATION_MIN: u32 = 1;
/// The longest a focus may be, in minutes.
pub const MAX_FOCUS_MIN: u32 = 120;
/// The longest a short break may be, in minutes.
pub const MAX_SHORT_BREAK_MIN: u32 = 30;
/// The longest a long break may be, in minutes.
pub const MAX_LONG_BREAK_MIN: u32 = 60;
/// The fewest focus phases a session may have.
pub const MIN_CYCLES: u32 = 1;
/// The most focus phases a session may have.
pub const MAX_CYCLES: u32 = 8;

/// Seconds in a minute.
const SECS_PER_MIN: u64 = 60;

/// The durations and number of cycles of a session, always within the
/// limits above (#15 in `docs/threat-model.md`).
///
/// The only ways to build one are [`PomodoroPreset::new`], which rejects
/// values out of range, [`PomodoroPreset::default`], and deserializing one,
/// which goes through [`PomodoroPreset::new`]; so an engine never runs a
/// preset it hasn't checked.
///
/// In JSON it is an object with the four values of [`PomodoroPreset::new`],
/// in whole minutes: `{ "focus_min": 25, "short_break_min": 5,
/// "long_break_min": 15, "cycles": 4 }`. All four are required.
#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
pub struct PomodoroPreset {
    /// Length of each focus, in minutes.
    focus_min: u32,
    /// Length of each short break, in minutes.
    short_break_min: u32,
    /// Length of the long break, in minutes.
    long_break_min: u32,
    /// Number of focus phases.
    cycles: u32,
}

impl PomodoroPreset {
    /// A preset with the given durations, in whole minutes, and number of
    /// focus phases.
    ///
    /// # Errors
    ///
    /// Returns a [`PresetError`] naming the first value, in the order of the
    /// arguments, that is outside its range: [`MIN_DURATION_MIN`] to
    /// [`MAX_FOCUS_MIN`], [`MAX_SHORT_BREAK_MIN`] or [`MAX_LONG_BREAK_MIN`]
    /// minutes, and [`MIN_CYCLES`] to [`MAX_CYCLES`] cycles. Values are never
    /// clamped into range.
    pub fn new(
        focus_min: u32,
        short_break_min: u32,
        long_break_min: u32,
        cycles: u32,
    ) -> Result<Self, PresetError> {
        PresetField::Focus.check(focus_min)?;
        PresetField::ShortBreak.check(short_break_min)?;
        PresetField::LongBreak.check(long_break_min)?;
        PresetField::Cycles.check(cycles)?;
        Ok(Self {
            focus_min,
            short_break_min,
            long_break_min,
            cycles,
        })
    }

    /// How long each focus lasts.
    #[must_use]
    pub fn focus(&self) -> Duration {
        minutes(self.focus_min)
    }

    /// How long each short break lasts.
    #[must_use]
    pub fn short_break(&self) -> Duration {
        minutes(self.short_break_min)
    }

    /// How long the long break lasts.
    #[must_use]
    pub fn long_break(&self) -> Duration {
        minutes(self.long_break_min)
    }

    /// How many focus phases a session has.
    #[must_use]
    pub fn cycles(&self) -> u32 {
        self.cycles
    }
}

impl Default for PomodoroPreset {
    /// The classic pomodoro: 25 minutes of focus, 5-minute short breaks, a
    /// 15-minute long break, and 4 cycles.
    fn default() -> Self {
        Self {
            focus_min: 25,
            short_break_min: 5,
            long_break_min: 15,
            cycles: 4,
        }
    }
}

/// The four values of a preset as read from JSON, before they are checked.
#[derive(Deserialize)]
struct RawPreset {
    focus_min: u32,
    short_break_min: u32,
    long_break_min: u32,
    cycles: u32,
}

impl<'de> Deserialize<'de> for PomodoroPreset {
    /// Reads the four values and checks them with [`PomodoroPreset::new`],
    /// so a preset out of range is rejected, never clamped.
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let raw = RawPreset::deserialize(deserializer)?;
        Self::new(
            raw.focus_min,
            raw.short_break_min,
            raw.long_break_min,
            raw.cycles,
        )
        .map_err(D::Error::custom)
    }
}

/// `count` minutes. Never overflows: `u32::MAX` minutes fit in a `u64` of
/// seconds.
fn minutes(count: u32) -> Duration {
    Duration::from_secs(u64::from(count).saturating_mul(SECS_PER_MIN))
}

/// Why [`PomodoroPreset::new`] rejected a preset: one value is outside its
/// range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, thiserror::Error)]
#[error("the {field} must be {min} to {max}{unit}, not {value}", min = .field.min(), max = .field.max(), unit = .field.unit())]
pub struct PresetError {
    /// The value that is out of range.
    field: PresetField,
    /// What it was set to.
    value: u32,
}

impl PresetError {
    /// Which value is out of range.
    #[must_use]
    pub fn field(&self) -> PresetField {
        self.field
    }

    /// What the value was set to.
    #[must_use]
    pub fn value(&self) -> u32 {
        self.value
    }
}

/// A value of a [`PomodoroPreset`], as named by a [`PresetError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum PresetField {
    /// The length of each focus, in minutes.
    Focus,
    /// The length of each short break, in minutes.
    ShortBreak,
    /// The length of the long break, in minutes.
    LongBreak,
    /// The number of focus phases.
    Cycles,
}

impl PresetField {
    /// The smallest value allowed.
    fn min(self) -> u32 {
        match self {
            Self::Focus | Self::ShortBreak | Self::LongBreak => MIN_DURATION_MIN,
            Self::Cycles => MIN_CYCLES,
        }
    }

    /// The largest value allowed.
    fn max(self) -> u32 {
        match self {
            Self::Focus => MAX_FOCUS_MIN,
            Self::ShortBreak => MAX_SHORT_BREAK_MIN,
            Self::LongBreak => MAX_LONG_BREAK_MIN,
            Self::Cycles => MAX_CYCLES,
        }
    }

    /// The unit of the value, for error messages.
    fn unit(self) -> &'static str {
        match self {
            Self::Focus | Self::ShortBreak | Self::LongBreak => " minutes",
            Self::Cycles => "",
        }
    }

    /// `Ok` if `value` is within this field's range.
    fn check(self, value: u32) -> Result<(), PresetError> {
        if (self.min()..=self.max()).contains(&value) {
            Ok(())
        } else {
            Err(PresetError { field: self, value })
        }
    }
}

impl fmt::Display for PresetField {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(match self {
            Self::Focus => "focus duration",
            Self::ShortBreak => "short break duration",
            Self::LongBreak => "long break duration",
            Self::Cycles => "number of cycles",
        })
    }
}

/// The fewest characters a [`LockKind::RandomText`] may ask to type.
pub const MIN_RANDOM_TEXT_LEN: u32 = 1;
/// The most characters a [`LockKind::RandomText`] may ask to type (#15 in
/// `docs/threat-model.md`).
pub const MAX_RANDOM_TEXT_LEN: u32 = 256;

/// How firmly a session is locked against being cut short.
///
/// In JSON it is an object tagged by `kind`: `{ "kind": "none" }`,
/// `{ "kind": "until_timer_ends" }` or `{ "kind": "random_text", "len": 8 }`.
/// A `len` is required for `random_text` and rejected for the others (a
/// `null` one counts as absent); other unknown fields are skipped.
/// Deserializing doesn't check the length's range: whoever uses the lock
/// calls [`LockKind::validate`], as [`Engine::start`] and [`Engine::restore`]
/// do.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
#[non_exhaustive]
pub enum LockKind {
    /// No lock: the session can be paused and stopped at any time.
    None,
    /// Strict: the session is active until its timer ends. It can't be
    /// stopped, paused or shortened, and the blocklist can't be edited.
    UntilTimerEnds,
    /// Friction, not strict: to stop early the user types `len` random
    /// characters. The engine treats the session like an unlocked one; the
    /// layer above generates and checks the text.
    RandomText {
        /// Characters to type, from [`MIN_RANDOM_TEXT_LEN`] to
        /// [`MAX_RANDOM_TEXT_LEN`].
        len: u32,
    },
}

impl LockKind {
    /// Checks the lock is well formed: a `RandomText` length within
    /// [`MIN_RANDOM_TEXT_LEN`] to [`MAX_RANDOM_TEXT_LEN`]. [`Engine::start`]
    /// calls it, so a session never runs with an invalid lock.
    ///
    /// # Errors
    ///
    /// A [`LockError`] for a length out of range; it is never clamped.
    pub fn validate(&self) -> Result<(), LockError> {
        match *self {
            Self::RandomText { len }
                if !(MIN_RANDOM_TEXT_LEN..=MAX_RANDOM_TEXT_LEN).contains(&len) =>
            {
                Err(LockError { len })
            }
            _ => Ok(()),
        }
    }

    /// Whether the session is strict: only [`LockKind::UntilTimerEnds`] is.
    fn is_strict(self) -> bool {
        matches!(self, Self::UntilTimerEnds)
    }
}

// `LockKind` goes through a flat struct in JSON instead of serde's
// `#[serde(tag = "kind")]`, like `Target` in `crate::model`: an internally
// tagged enum first copies the whole object, unknown fields included, into a
// buffer, while a plain struct skips unknown fields without keeping them.

/// The `kind` of a lock in JSON.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum LockTag {
    None,
    UntilTimerEnds,
    RandomText,
}

/// Every field a lock can have in JSON; `len` only for `random_text`.
#[derive(Serialize, Deserialize)]
struct LockFields {
    kind: LockTag,
    #[serde(skip_serializing_if = "Option::is_none")]
    len: Option<u32>,
}

impl Serialize for LockKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let (kind, len) = match *self {
            Self::None => (LockTag::None, None),
            Self::UntilTimerEnds => (LockTag::UntilTimerEnds, None),
            Self::RandomText { len } => (LockTag::RandomText, Some(len)),
        };
        LockFields { kind, len }.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for LockKind {
    fn deserialize<D: Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        let LockFields { kind, len } = LockFields::deserialize(deserializer)?;
        match (kind, len) {
            (LockTag::RandomText, Some(len)) => Ok(Self::RandomText { len }),
            (LockTag::RandomText, None) => Err(D::Error::missing_field("len")),
            (LockTag::None, None) => Ok(Self::None),
            (LockTag::UntilTimerEnds, None) => Ok(Self::UntilTimerEnds),
            (LockTag::None | LockTag::UntilTimerEnds, Some(_)) => {
                Err(D::Error::custom("only a `random_text` lock has a `len`"))
            }
        }
    }
}

/// Why [`LockKind::validate`] rejected a lock: the text length is out of range.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[error(
    "the random text must be {MIN_RANDOM_TEXT_LEN} to {MAX_RANDOM_TEXT_LEN} characters, not {len}"
)]
pub struct LockError {
    /// The rejected length.
    len: u32,
}

impl LockError {
    /// The length that was rejected.
    #[must_use]
    pub fn value(&self) -> u32 {
        self.len
    }
}

/// Why a strict session refuses a request, from [`Engine::can_stop`] and
/// [`Engine::can_edit_blocklist`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum LockReason {
    /// A strict session is active until its timer ends.
    #[error("a strict session is active until its timer ends")]
    UntilTimerEnds,
}

/// A phase of a session, or [`Phase::Idle`] when there is none.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum Phase {
    /// No session is running.
    Idle,
    /// Working; the blocklist is enforced.
    Focus,
    /// The break after every focus but the last.
    ShortBreak,
    /// The break after the last focus; the session ends when it does.
    LongBreak,
}

/// What an [`Engine`] is doing at one instant, from [`Engine::status`].
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub struct Status {
    /// The current phase.
    phase: Phase,
    /// The 1-based number of the focus the phase belongs to.
    cycle: u32,
    /// Time left in the phase.
    remaining: Duration,
    /// Whether the session is paused.
    paused: bool,
}

impl Status {
    /// The status of an engine with no session.
    const IDLE: Self = Self {
        phase: Phase::Idle,
        cycle: 0,
        remaining: Duration::ZERO,
        paused: false,
    };

    /// The current phase.
    #[must_use]
    pub fn phase(&self) -> Phase {
        self.phase
    }

    /// The 1-based number of the focus the phase belongs to: during a break,
    /// the number of the focus just finished. Zero when idle.
    #[must_use]
    pub fn cycle(&self) -> u32 {
        self.cycle
    }

    /// The exact time left in the phase, never rounded. Never zero while a
    /// session runs, as a phase ends the instant this would reach zero; zero
    /// when idle.
    #[must_use]
    pub fn remaining(&self) -> Duration {
        self.remaining
    }

    /// Whether the session is paused. Never true when idle.
    #[must_use]
    pub fn is_paused(&self) -> bool {
        self.paused
    }
}

/// Why an [`Engine`] refused a request. The engine is unchanged after any
/// error.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum EngineError {
    /// [`Engine::start`] while a session is running or paused.
    #[error("a session is already running")]
    AlreadyRunning,
    /// A request that needs a session, while there is none (never started,
    /// stopped, or over).
    #[error("no session is running")]
    NoSession,
    /// [`Engine::pause`] while the session is already paused.
    #[error("the session is already paused")]
    AlreadyPaused,
    /// [`Engine::resume`] while the session isn't paused.
    #[error("the session is not paused")]
    NotPaused,
    /// [`Engine::skip_break`] during a focus.
    #[error("only a break can be skipped, and this is a focus")]
    NotABreak,
    /// A request that would stop, pause or shorten a strict session
    /// ([`LockKind::UntilTimerEnds`]), or start another over it.
    #[error("a strict session can't be stopped, paused or shortened")]
    StrictSession,
    /// [`Engine::start`] with a lock that fails [`LockKind::validate`].
    #[error("invalid lock: {0}")]
    InvalidLock(#[from] LockError),
}

/// The pomodoro state machine. It holds at most one session, and starts idle.
///
/// See the [module documentation](self) for how a session runs.
///
/// ```
/// use std::time::{Duration, UNIX_EPOCH};
///
/// use keitora_core::clock::FakeClock;
/// use keitora_core::engine::{Engine, LockKind, Phase, PomodoroPreset};
///
/// let clock = FakeClock::new(UNIX_EPOCH);
/// let mut engine = Engine::new();
/// engine.start(PomodoroPreset::default(), LockKind::None, &clock)?;
///
/// clock.advance(Duration::from_secs(25 * 60))?;
/// let status = engine.status(&clock);
/// assert_eq!(status.phase(), Phase::ShortBreak);
/// assert_eq!(status.cycle(), 1);
/// assert_eq!(status.remaining(), Duration::from_secs(5 * 60));
/// # Ok::<(), Box<dyn std::error::Error>>(())
/// ```
#[derive(Debug, Clone, Default)]
pub struct Engine {
    /// The session, if one was started and not stopped. It may be over
    /// already: that is only known once the clock is read.
    session: Option<Session>,
}

/// A started session.
#[derive(Debug, Clone)]
struct Session {
    /// Its durations and cycles.
    preset: PomodoroPreset,
    /// How firmly it is locked.
    lock: LockKind,
    /// The cycle whose focus begins when the active time is zero: 1 when the
    /// session starts, and the next cycle after a short break is skipped.
    first_cycle: u32,
    /// Whether it is running or paused, and its active time.
    run: Run,
}

/// Whether a session is running or paused, with enough to know its active
/// time: the time it has run, since `first_cycle` began, outside pauses.
#[derive(Debug, Clone, Copy)]
enum Run {
    /// Running since `since`, with `before` of active time before that.
    Running {
        /// When it last started or resumed.
        since: MonotonicInstant,
        /// Active time before `since`.
        before: Duration,
    },
    /// Paused, with `active` time run so far.
    Paused {
        /// Active time when it was paused.
        active: Duration,
    },
}

impl Run {
    /// Running from now, with `active` time already run.
    fn running(now: MonotonicInstant, active: Duration) -> Self {
        Self::Running {
            since: now,
            before: active,
        }
    }

    /// The active time at `now`. A `now` earlier than `since` counts as no
    /// time passing.
    fn active(self, now: MonotonicInstant) -> Duration {
        match self {
            Self::Running { since, before } => {
                before.saturating_add(now.saturating_duration_since(since))
            }
            Self::Paused { active } => active,
        }
    }

    /// Whether this is a pause.
    fn is_paused(self) -> bool {
        matches!(self, Self::Paused { .. })
    }
}

/// Where a session that isn't over is on its schedule.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
struct Position {
    /// [`Phase::Focus`], [`Phase::ShortBreak`] or [`Phase::LongBreak`].
    phase: Phase,
    /// The 1-based number of the focus the phase belongs to.
    cycle: u32,
    /// Time left in the phase; never zero.
    remaining: Duration,
}

impl Session {
    /// Where the session is at `now`, or `None` if it is over.
    fn position(&self, now: MonotonicInstant) -> Option<Position> {
        locate(&self.preset, self.first_cycle, self.run.active(now))
    }
}

/// Where a session is after `active` time from the start of `first_cycle`'s
/// focus, or `None` if its long break has ended.
///
/// Every cycle before the last is a focus and a short break, so the number of
/// whole such cycles behind is `active` divided by their length; the last
/// cycle is a focus and the long break.
fn locate(preset: &PomodoroPreset, first_cycle: u32, active: Duration) -> Option<Position> {
    let focus = preset.focus();
    let short_break = preset.short_break();
    let cycle_length = focus.saturating_add(short_break);

    // Cycles that end with a short break, from `first_cycle` on.
    let short_cycles = preset.cycles.saturating_sub(first_cycle);
    // The length is at least two minutes, so the division can't fail.
    let cycles_behind = active
        .as_nanos()
        .checked_div(cycle_length.as_nanos())
        .unwrap_or_default();
    let cycles_behind = u32::try_from(cycles_behind)
        .unwrap_or(u32::MAX)
        .min(short_cycles);

    let cycle = first_cycle.saturating_add(cycles_behind);
    let into_cycle = active.saturating_sub(cycle_length.saturating_mul(cycles_behind));
    if into_cycle < focus {
        return Some(Position {
            phase: Phase::Focus,
            cycle,
            remaining: focus.saturating_sub(into_cycle),
        });
    }

    let into_break = into_cycle.saturating_sub(focus);
    let (phase, break_length) = if cycle < preset.cycles {
        (Phase::ShortBreak, short_break)
    } else {
        (Phase::LongBreak, preset.long_break())
    };
    (into_break < break_length).then(|| Position {
        phase,
        cycle,
        remaining: break_length.saturating_sub(into_break),
    })
}

impl Engine {
    /// An idle engine.
    #[must_use]
    pub const fn new() -> Self {
        Self { session: None }
    }

    /// Starts a session with `preset` and `lock`: its first focus begins now,
    /// with its full duration.
    ///
    /// # Errors
    ///
    /// [`EngineError::InvalidLock`] if `lock` is invalid,
    /// [`EngineError::StrictSession`] if a strict session is active, and
    /// [`EngineError::AlreadyRunning`] if another session is running or
    /// paused. A session that is over doesn't count: a new one replaces it.
    pub fn start<C>(
        &mut self,
        preset: PomodoroPreset,
        lock: LockKind,
        clock: &C,
    ) -> Result<(), EngineError>
    where
        C: Clock + ?Sized,
    {
        lock.validate()?;
        let now = clock.monotonic();
        if let Some((session, _)) = self.current(now) {
            return Err(if session.lock.is_strict() {
                EngineError::StrictSession
            } else {
                EngineError::AlreadyRunning
            });
        }
        self.session = Some(Session {
            preset,
            lock,
            first_cycle: 1,
            run: Run::running(now, Duration::ZERO),
        });
        Ok(())
    }

    /// Pauses the session: its phase and remaining time stay as they are
    /// until [`Engine::resume`].
    ///
    /// # Errors
    ///
    /// [`EngineError::NoSession`] if there is no session,
    /// [`EngineError::StrictSession`] if it is strict, and
    /// [`EngineError::AlreadyPaused`] if it is paused already.
    pub fn pause<C>(&mut self, clock: &C) -> Result<(), EngineError>
    where
        C: Clock + ?Sized,
    {
        let now = clock.monotonic();
        let (session, _) = self.current_mut(now).ok_or(EngineError::NoSession)?;
        if session.lock.is_strict() {
            return Err(EngineError::StrictSession);
        }
        if session.run.is_paused() {
            return Err(EngineError::AlreadyPaused);
        }
        session.run = Run::Paused {
            active: session.run.active(now),
        };
        Ok(())
    }

    /// Resumes a paused session from where it was paused. The rest of the
    /// schedule moves later by the length of the pause.
    ///
    /// # Errors
    ///
    /// [`EngineError::NoSession`] if there is no session, and
    /// [`EngineError::NotPaused`] if it isn't paused.
    pub fn resume<C>(&mut self, clock: &C) -> Result<(), EngineError>
    where
        C: Clock + ?Sized,
    {
        let now = clock.monotonic();
        let (session, _) = self.current_mut(now).ok_or(EngineError::NoSession)?;
        match session.run {
            Run::Paused { active } => {
                session.run = Run::running(now, active);
                Ok(())
            }
            Run::Running { .. } => Err(EngineError::NotPaused),
        }
    }

    /// Ends the current break now. After a short break, the next focus starts
    /// with its full duration and the schedule continues from it; after the
    /// long break, the session ends. Skipping doesn't pause or resume: a
    /// paused session stays paused, at the start of its next focus.
    ///
    /// # Errors
    ///
    /// [`EngineError::NoSession`] if there is no session,
    /// [`EngineError::StrictSession`] if it is strict (any skip would shorten
    /// it), and [`EngineError::NotABreak`] during a focus.
    pub fn skip_break<C>(&mut self, clock: &C) -> Result<(), EngineError>
    where
        C: Clock + ?Sized,
    {
        let now = clock.monotonic();
        let (session, position) = self.current_mut(now).ok_or(EngineError::NoSession)?;
        if session.lock.is_strict() {
            return Err(EngineError::StrictSession);
        }
        match position.phase {
            Phase::ShortBreak => {
                session.first_cycle = position.cycle.saturating_add(1);
                session.run = if session.run.is_paused() {
                    Run::Paused {
                        active: Duration::ZERO,
                    }
                } else {
                    Run::running(now, Duration::ZERO)
                };
            }
            Phase::LongBreak => self.session = None,
            Phase::Focus | Phase::Idle => return Err(EngineError::NotABreak),
        }
        Ok(())
    }

    /// Ends the session now, running or paused, and goes back to idle.
    ///
    /// # Errors
    ///
    /// [`EngineError::NoSession`] if there is no session, and
    /// [`EngineError::StrictSession`] if it is strict.
    pub fn stop<C>(&mut self, clock: &C) -> Result<(), EngineError>
    where
        C: Clock + ?Sized,
    {
        let (session, _) = self
            .current(clock.monotonic())
            .ok_or(EngineError::NoSession)?;
        if session.lock.is_strict() {
            return Err(EngineError::StrictSession);
        }
        self.session = None;
        Ok(())
    }

    /// Whether stopping the session now would be allowed.
    ///
    /// # Errors
    ///
    /// [`LockReason::UntilTimerEnds`] while a strict session is active.
    /// With no session, or one that is over or not strict, it is `Ok`.
    pub fn can_stop<C>(&self, clock: &C) -> Result<(), LockReason>
    where
        C: Clock + ?Sized,
    {
        self.lock_reason(clock)
    }

    /// Whether the blocklist may be edited now (#18 in
    /// `docs/threat-model.md`): not while a strict session is active, in any
    /// of its phases.
    ///
    /// # Errors
    ///
    /// [`LockReason::UntilTimerEnds`] while a strict session is active.
    pub fn can_edit_blocklist<C>(&self, clock: &C) -> Result<(), LockReason>
    where
        C: Clock + ?Sized,
    {
        self.lock_reason(clock)
    }

    /// The reason a strict session is active, if one is.
    fn lock_reason<C>(&self, clock: &C) -> Result<(), LockReason>
    where
        C: Clock + ?Sized,
    {
        match self.current(clock.monotonic()) {
            Some((session, _)) if session.lock.is_strict() => Err(LockReason::UntilTimerEnds),
            _ => Ok(()),
        }
    }

    /// What the engine is doing now. Observing never changes the session.
    #[must_use]
    pub fn status<C>(&self, clock: &C) -> Status
    where
        C: Clock + ?Sized,
    {
        match self.current(clock.monotonic()) {
            Some((session, position)) => Status {
                phase: position.phase,
                cycle: position.cycle,
                remaining: position.remaining,
                paused: session.run.is_paused(),
            },
            None => Status::IDLE,
        }
    }

    /// The session and where it is at `now`, unless there is none or it is
    /// over.
    fn current(&self, now: MonotonicInstant) -> Option<(&Session, Position)> {
        let session = self.session.as_ref()?;
        Some((session, session.position(now)?))
    }

    /// Like [`Engine::current`], with the session mutable.
    fn current_mut(&mut self, now: MonotonicInstant) -> Option<(&mut Session, Position)> {
        let session = self.session.as_mut()?;
        let position = session.position(now)?;
        Some((session, position))
    }
}

/// A snapshot of an [`Engine`], to be stored and restored after the service
/// restarts, from [`Engine::snapshot`]; [`Engine::restore`] turns it back into
/// an engine.
///
/// It holds the session's preset, lock, the cycle its schedule counts from,
/// its active time and whether it is paused (ADR-0010). It never holds a
/// [`MonotonicInstant`], whose origin changes every time the service starts,
/// nor a wall-clock time.
///
/// # JSON format
///
/// A snapshot is written to disk and read back by later versions of the
/// service, so this format is a contract. With a session:
///
/// ```json
/// {
///   "session": {
///     "preset": { "focus_min": 25, "short_break_min": 5, "long_break_min": 15, "cycles": 4 },
///     "lock": { "kind": "until_timer_ends" },
///     "first_cycle": 1,
///     "active": { "secs": 600, "nanos": 0 },
///     "paused": false
///   }
/// }
/// ```
///
/// - `preset`: a [`PomodoroPreset`], checked like [`PomodoroPreset::new`].
/// - `lock`: a [`LockKind`].
/// - `first_cycle`: the cycle whose focus began when the active time was
///   zero: 1, or the cycle after a short break that was skipped.
/// - `active`: the time the session has run since then, outside pauses, as
///   serde writes a [`Duration`].
/// - `paused`: whether the session is paused.
///
/// An engine with no session, or whose session is over, writes
/// `{ "session": null }`; a missing `session` reads as `null` too. Every field
/// of a session is required: none has a default, so that a missing `lock`
/// can't turn a strict session into an unlocked one. Unknown fields are
/// skipped, so a later version may add optional ones.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct EngineState {
    /// The session, unless the engine was idle.
    session: Option<SessionState>,
}

/// A session in an [`EngineState`]. It is checked only by
/// [`Engine::restore`], apart from its preset, which is always valid.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
struct SessionState {
    /// Its durations and cycles.
    preset: PomodoroPreset,
    /// How firmly it is locked.
    lock: LockKind,
    /// The cycle whose focus began when the active time was zero.
    first_cycle: u32,
    /// Active time since `first_cycle`'s focus began.
    active: Duration,
    /// Whether it is paused.
    paused: bool,
}

/// Why [`Engine::restore`] rejected a snapshot. Data the engine wrote never
/// fails; these mean the stored snapshot was changed or damaged.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash, thiserror::Error)]
#[non_exhaustive]
pub enum RestoreError {
    /// The lock fails [`LockKind::validate`].
    #[error("invalid lock: {0}")]
    InvalidLock(#[from] LockError),
    /// The first cycle isn't between 1 and the preset's number of cycles.
    #[error("the first cycle must be 1 to {cycles}, not {first_cycle}")]
    FirstCycleOutOfRange {
        /// The stored first cycle.
        first_cycle: u32,
        /// The preset's number of cycles.
        cycles: u32,
    },
    /// A strict session that is paused: a strict session can't be paused.
    #[error("a strict session can't be paused")]
    PausedStrictSession,
    /// A strict session counting from a cycle after the first, which would
    /// cut cycles from it: a strict session can't skip a break.
    #[error("a strict session counts from cycle 1, not {first_cycle}")]
    StrictSessionSkippedCycles {
        /// The stored first cycle.
        first_cycle: u32,
    },
    /// The session's active time reaches the end of its schedule: it had
    /// ended already, and an ended session is never restored.
    #[error("the session had already ended")]
    SessionEnded,
}

impl Engine {
    /// A snapshot of the engine now, to store and later give to
    /// [`Engine::restore`]. It records the session's active time at this
    /// instant, so the service can take one every few seconds as a
    /// checkpoint, not only when something changes. Taking it never changes
    /// the engine.
    ///
    /// With no session, or one that is over, the snapshot has no session.
    #[must_use]
    pub fn snapshot<C>(&self, clock: &C) -> EngineState
    where
        C: Clock + ?Sized,
    {
        let now = clock.monotonic();
        let session = self.current(now).map(|(session, _)| SessionState {
            preset: session.preset.clone(),
            lock: session.lock,
            first_cycle: session.first_cycle,
            active: session.run.active(now),
            paused: session.run.is_paused(),
        });
        EngineState { session }
    }

    /// An engine from a snapshot taken by [`Engine::snapshot`], possibly in an
    /// earlier run of the service. A snapshot with no session gives an idle
    /// engine.
    ///
    /// The session continues on `clock` from exactly the active time stored,
    /// running or paused as it was: the time between the snapshot and the
    /// restore, such as while the machine was off, is never counted, and the
    /// wall clock is never read (ADR-0010). Changing the date, in Windows or in
    /// the firmware, therefore can't end a strict session.
    ///
    /// # Errors
    ///
    /// A [`RestoreError`] if the session couldn't have come from
    /// [`Engine::snapshot`], checked in this order:
    /// [`RestoreError::InvalidLock`] if the lock fails [`LockKind::validate`],
    /// [`RestoreError::FirstCycleOutOfRange`] for a first cycle outside 1 to
    /// the preset's cycles, [`RestoreError::PausedStrictSession`] and
    /// [`RestoreError::StrictSessionSkippedCycles`] for a strict session
    /// that is paused or counts from a cycle after 1, and
    /// [`RestoreError::SessionEnded`] if the session is already over. The
    /// preset was checked when the snapshot was deserialized.
    pub fn restore<C>(state: EngineState, clock: &C) -> Result<Self, RestoreError>
    where
        C: Clock + ?Sized,
    {
        let Some(saved) = state.session else {
            return Ok(Self::new());
        };
        saved.lock.validate()?;
        let cycles = saved.preset.cycles();
        if !(1..=cycles).contains(&saved.first_cycle) {
            return Err(RestoreError::FirstCycleOutOfRange {
                first_cycle: saved.first_cycle,
                cycles,
            });
        }
        if saved.lock.is_strict() {
            if saved.paused {
                return Err(RestoreError::PausedStrictSession);
            }
            if saved.first_cycle != 1 {
                return Err(RestoreError::StrictSessionSkippedCycles {
                    first_cycle: saved.first_cycle,
                });
            }
        }

        let now = clock.monotonic();
        let session = Session {
            preset: saved.preset,
            lock: saved.lock,
            first_cycle: saved.first_cycle,
            run: if saved.paused {
                Run::Paused {
                    active: saved.active,
                }
            } else {
                Run::running(now, saved.active)
            },
        };
        if session.position(now).is_none() {
            return Err(RestoreError::SessionEnded);
        }
        Ok(Self {
            session: Some(session),
        })
    }
}
