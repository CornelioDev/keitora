// SPDX-License-Identifier: MIT OR Apache-2.0

//! Named pipe servers with an explicit security descriptor.
//!
//! tokio can create a pipe server with a `SECURITY_ATTRIBUTES`, but only
//! through an `unsafe` method. This is the safe wrapper around it, so that
//! `keitora-service`, which forbids `unsafe`, can still protect its pipe with
//! a DACL (ADR-0012).

use std::ffi::c_void;
use std::io;

use tokio::net::windows::named_pipe::{NamedPipeServer, ServerOptions};

use crate::security::SecurityDescriptor;

/// Creates one instance of the byte-mode pipe `name` for `options`, protected
/// by `security`.
///
/// It must be called inside a tokio runtime, as the pipe is registered with
/// its reactor. `options` carries the other settings, such as
/// `first_pipe_instance` and `reject_remote_clients`.
///
/// # Errors
///
/// Returns the Win32 error if `CreateNamedPipeW` fails; with
/// `first_pipe_instance(true)`, that includes an access-denied error when
/// another instance of `name` already exists.
pub fn create_server(
    options: &ServerOptions,
    name: &str,
    security: &SecurityDescriptor,
) -> io::Result<NamedPipeServer> {
    let mut attributes = security.attributes()?;
    // SAFETY: `attributes` is a valid `SECURITY_ATTRIBUTES` that lives past
    // the call, and its descriptor is owned by `security`, which the caller
    // keeps borrowed for the call; `CreateNamedPipeW` copies the descriptor
    // and doesn't keep the pointer.
    unsafe {
        options.create_with_security_attributes_raw(name, (&raw mut attributes).cast::<c_void>())
    }
}
