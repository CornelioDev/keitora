// SPDX-License-Identifier: MIT OR Apache-2.0

//! Named pipes: servers with an explicit security descriptor, and clients
//! with an explicit access mask.
//!
//! tokio can create a pipe server with a `SECURITY_ATTRIBUTES`, but only
//! through an `unsafe` method, and its client always asks for `GENERIC_READ`
//! and `GENERIC_WRITE`. These are the safe wrappers around both, so that
//! `keitora-service`, which forbids `unsafe`, can protect its pipe with a DACL
//! and every client can ask for no more access than it needs (ADR-0012).

use std::ffi::c_void;
use std::io;
use std::ptr;

use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{CreateFileW, FILE_FLAG_OVERLAPPED, OPEN_EXISTING};

use crate::security::SecurityDescriptor;

/// Everything a Keitora pipe client needs and nothing more: generic read
/// (`FILE_GENERIC_READ`) plus `FILE_WRITE_DATA`, `FILE_WRITE_EA` and
/// `FILE_WRITE_ATTRIBUTES`, which is `0x12019b`.
///
/// `FILE_APPEND_DATA` (`0x4`) is deliberately left out: on a pipe it is
/// `FILE_CREATE_PIPE_INSTANCE`, the right to serve the pipe's name. Clients
/// never need it, and the service's DACL doesn't grant it to the interactive
/// user (`keitora_service::pipe::pipe_sddl`).
///
/// Windows checks `FILE_CREATE_PIPE_INSTANCE` in `CreateNamedPipeW` only, not
/// when a client opens a pipe's other end: measured on Windows 11 26200, a
/// client that asks for `GENERIC_READ | GENERIC_WRITE` still opens a pipe whose
/// DACL grants it `0x12019b` alone. So the narrow DACL is what closes the
/// finding, and asking for this mask is defense in depth: every client asks
/// for only what it needs, and the open stops working if a future Windows
/// build does check the right here.
pub const CLIENT_ACCESS: u32 = 0x0012_019b;

/// Opens the client end of the existing pipe `name` with the access mask
/// `access`, for overlapped I/O, and hands it to tokio.
///
/// Use [`CLIENT_ACCESS`] unless there is a reason not to. It must be called
/// inside a tokio runtime, as the pipe is registered with its reactor.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] if `name` has a NUL, and otherwise
/// the Win32 error of `CreateFileW` unchanged, in particular:
///
/// - [`io::ErrorKind::NotFound`] (`ERROR_FILE_NOT_FOUND`) when nobody serves
///   `name`;
/// - `ERROR_ACCESS_DENIED` (5) when the pipe's DACL doesn't grant `access` to
///   the caller;
/// - `ERROR_PIPE_BUSY` (231) when the pipe exists but has no free instance,
///   which a caller is expected to retry.
pub fn open_client(name: &str, access: u32) -> io::Result<NamedPipeClient> {
    if name.contains('\0') {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "a pipe name can't contain NUL",
        ));
    }
    let wide: Vec<u16> = name.encode_utf16().chain(std::iter::once(0)).collect();
    // SAFETY: `wide` is a NUL-terminated UTF-16 string that lives past the
    // call; the pipe is opened unshared (share mode 0) with no security
    // attributes and no template file, both passed as null, as `OPEN_EXISTING`
    // allows.
    let handle = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            0,
            ptr::null(),
            OPEN_EXISTING,
            FILE_FLAG_OVERLAPPED,
            ptr::null_mut(),
        )
    };
    if handle == INVALID_HANDLE_VALUE || handle.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `handle` is an open, overlapped pipe handle that `CreateFileW`
    // just returned and that nothing else owns, so ownership passes to tokio,
    // which closes it when the client is dropped and also if this call fails.
    unsafe { NamedPipeClient::from_raw_handle(handle.cast::<c_void>()) }
}

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

#[cfg(test)]
mod tests {
    use super::*;

    // These tests don't run inside a tokio runtime, so they only use the
    // paths that return before a handle reaches tokio.

    #[test]
    fn the_client_mask_is_read_and_write_without_create_pipe_instance() {
        const FILE_CREATE_PIPE_INSTANCE: u32 = 0x0000_0004;
        // READ_CONTROL, SYNCHRONIZE, read and write data, read and write EA,
        // read and write attributes.
        let expected: u32 =
            0x0002_0000 | 0x0010_0000 | 0x0001 | 0x0002 | 0x0008 | 0x0010 | 0x0080 | 0x0100;
        assert_eq!(CLIENT_ACCESS, expected);
        assert_eq!(CLIENT_ACCESS & FILE_CREATE_PIPE_INSTANCE, 0);
    }

    #[test]
    fn a_name_with_nul_is_invalid_input() {
        let error = open_client("\\\\.\\pipe\\keitora\0evil", CLIENT_ACCESS)
            .expect_err("a NUL in a pipe name is refused");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
    }

    #[test]
    fn a_pipe_nobody_serves_is_not_found() {
        let name = format!(r"\\.\pipe\keitora-sys-absent-{}", std::process::id());
        let error = open_client(&name, CLIENT_ACCESS).expect_err("nobody serves that name");
        assert_eq!(error.kind(), io::ErrorKind::NotFound);
    }
}
