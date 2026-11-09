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
//! - [`model`]: blocklists, their targets and exceptions, and their validation.
//! - [`site`]: which URLs a blocklist blocks.

#![forbid(unsafe_code)]

pub mod clock;
pub mod model;
pub mod site;
