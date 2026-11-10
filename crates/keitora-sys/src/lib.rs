// SPDX-License-Identifier: MIT OR Apache-2.0

//! Keitora's Win32 wrappers: the one crate in the workspace allowed to
//! contain `unsafe` code ([ADR-0012]).
//!
//! Every other Keitora crate has `#![forbid(unsafe_code)]`, so the compiler
//! keeps `unsafe` here. When a crate needs a Win32 function that the standard
//! library or a vetted crate doesn't offer, it gets a wrapper in this crate:
//!
//! - one safe public function per Win32 call, in a module named after the
//!   API family it belongs to;
//! - each `unsafe` block holds that one call and a `// SAFETY:` comment that
//!   says why the call is sound;
//! - the wrapper turns the call's failure into a `Result` or an `Option`, and
//!   never panics.
//!
//! The crate has no policy of its own: what to do when a call fails is the
//! caller's decision.
//!
//! [ADR-0012]: https://github.com/CornelioDev/keitora/blob/main/docs/adr/0012-isolate-unsafe-code-in-keitora-sys.md

// This crate exists to hold the workspace's `unsafe` code, so it is the one
// place where the workspace's `unsafe_code = "deny"` is lifted. The clippy
// lints `undocumented_unsafe_blocks` and `multiple_unsafe_ops_per_block`
// still apply.
#![allow(
    unsafe_code,
    reason = "the only crate allowed to call Win32 directly (ADR-0012)"
)]

pub mod console;
pub mod library_loader;
pub mod time;
