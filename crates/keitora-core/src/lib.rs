// SPDX-License-Identifier: MIT OR Apache-2.0

//! Pure domain logic for Keitora: the data model, the pomodoro engine and the
//! rules that decide which websites and applications are blocked.
//!
//! This crate has no operating-system dependencies: no Windows APIs, no
//! processes, no network, and no direct reads of the system clock (time is
//! always passed in by the caller). That keeps it easy to test and reusable on
//! other platforms.
//!
//! - [`clock`]: the source of time, and a fake one for tests.
//! - [`engine`]: the pomodoro state machine that runs a session's focus and
//!   break phases.
//! - [`model`]: blocklists, their targets and exceptions, and their validation.
//! - [`site`]: which URLs a blocklist blocks.
//! - [`app`]: which running programs a blocklist blocks, and which are never
//!   blocked.
//!
//! # Example
//!
//! The service puts these pieces together: an [`Engine`](engine::Engine)
//! says which phase a session is in, reading time from a
//! [`Clock`](clock::Clock), and the matchers say what a
//! [`Blocklist`](model::Blocklist) blocks while it applies. Here a
//! [`FakeClock`](clock::FakeClock) stands in for the service's clock, so the
//! example moves time itself.
//!
//! ```
//! use std::time::{Duration, UNIX_EPOCH};
//!
//! use keitora_core::clock::FakeClock;
//! use keitora_core::engine::{Engine, EngineError, LockKind, Phase, PomodoroPreset};
//! use keitora_core::model::{Blocklist, SitePattern, Target};
//! use keitora_core::site::SiteMatcher;
//!
//! // A blocklist with one website, checked before it is used.
//! let blocklist = Blocklist {
//!     id: "work".to_owned(),
//!     name: "Work".to_owned(),
//!     color: "#3366ff".to_owned(),
//!     targets: vec![Target::Site { pattern: SitePattern::new("youtube.com") }],
//!     exceptions: Vec::new(),
//!     block_during_breaks: false,
//! };
//! blocklist.validate()?;
//! let sites = SiteMatcher::new(&blocklist);
//! assert!(sites.is_blocked("https://www.youtube.com/watch?v=abc"));
//! assert!(!sites.is_blocked("https://docs.rs/"));
//!
//! // A strict session with the default preset: 25 minutes of focus first.
//! let clock = FakeClock::new(UNIX_EPOCH);
//! let mut engine = Engine::new();
//! engine.start(PomodoroPreset::default(), LockKind::UntilTimerEnds, &clock)?;
//! assert_eq!(engine.status(&clock).phase(), Phase::Focus);
//!
//! // It can't be stopped early...
//! assert_eq!(engine.stop(&clock), Err(EngineError::StrictSession));
//!
//! // ...and when the focus runs out, the short break starts on its own.
//! clock.advance(Duration::from_secs(25 * 60))?;
//! let status = engine.status(&clock);
//! assert_eq!(status.phase(), Phase::ShortBreak);
//! assert_eq!(status.remaining(), Duration::from_secs(5 * 60));
//! # Ok::<(), Box<dyn std::error::Error>>(())
//! ```

#![forbid(unsafe_code)]

pub mod app;
pub mod clock;
pub mod engine;
pub mod model;
pub mod site;
