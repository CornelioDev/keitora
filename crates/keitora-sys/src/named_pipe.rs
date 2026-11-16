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
use std::os::windows::io::{AsRawHandle, FromRawHandle, IntoRawHandle, OwnedHandle};
use std::ptr;

use tokio::net::windows::named_pipe::{NamedPipeClient, NamedPipeServer, ServerOptions};
use windows_sys::Win32::Foundation::INVALID_HANDLE_VALUE;
use windows_sys::Win32::Storage::FileSystem::{
    CreateFileW, FILE_FLAG_OVERLAPPED, FILE_TYPE_PIPE, GetFileType, OPEN_EXISTING,
    SECURITY_IDENTIFICATION, SECURITY_SQOS_PRESENT,
};

use crate::security::SecurityDescriptor;

/// The flags [`open_client`] opens a pipe with: overlapped, for tokio, and a
/// security quality of service that lets the server *identify* the client
/// but not impersonate it (`SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`).
///
/// Without the quality of service, Windows lets the server impersonate the
/// client fully. A process that serves `\\.\pipe\keitora` while the service is
/// down, and holds `SeImpersonatePrivilege`, could then act as the user whose
/// desktop app or native messaging host connects. tokio's `ClientOptions`
/// sets the same quality of service by default.
const OPEN_FLAGS: u32 = FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION;

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
/// The pipe is opened at the *identification* impersonation level
/// (`SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION`): the server can learn
/// who the client is, but can't act as the client. That matters when someone
/// other than the service serves the name, for example while the service is
/// stopped.
///
/// # Errors
///
/// Returns [`io::ErrorKind::InvalidInput`] if `name` has a NUL or names
/// something that isn't a pipe, such as a file (which is closed again), and
/// otherwise the Win32 error of `CreateFileW` unchanged, in particular:
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
    let raw = unsafe {
        CreateFileW(
            wide.as_ptr(),
            access,
            0,
            ptr::null(),
            OPEN_EXISTING,
            OPEN_FLAGS,
            ptr::null_mut(),
        )
    };
    if raw == INVALID_HANDLE_VALUE || raw.is_null() {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: `raw` is an open handle that `CreateFileW` just returned and
    // that nothing else owns, so the `OwnedHandle` is its sole owner and
    // closes it exactly once, including on the early return below.
    let handle = unsafe { OwnedHandle::from_raw_handle(raw.cast::<c_void>()) };
    // SAFETY: `handle` is open and stays open for the call, which only reads
    // what kind of object it refers to.
    let kind = unsafe { GetFileType(handle.as_raw_handle()) };
    if kind != FILE_TYPE_PIPE {
        return Err(io::Error::new(
            io::ErrorKind::InvalidInput,
            "the name doesn't refer to a named pipe",
        ));
    }
    // SAFETY: the handle is an open, overlapped named pipe handle (checked
    // just above) that nothing else owns, so ownership passes to tokio, which
    // closes it when the client is dropped and also if this call fails.
    unsafe { NamedPipeClient::from_raw_handle(handle.into_raw_handle()) }
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
    use std::ptr;

    use tokio::io::{AsyncReadExt, AsyncWriteExt};
    use windows_sys::Win32::Foundation::{FALSE, HANDLE, TRUE};
    use windows_sys::Win32::Security::{
        GetTokenInformation, RevertToSelf, SECURITY_IMPERSONATION_LEVEL, SecurityIdentification,
        TOKEN_QUERY, TokenImpersonationLevel,
    };
    use windows_sys::Win32::System::Pipes::ImpersonateNamedPipeClient;
    use windows_sys::Win32::System::Threading::{GetCurrentThread, OpenThreadToken};

    use super::*;
    use crate::security::current_process_sid;

    /// The calling thread impersonating the client of a pipe, until dropped.
    ///
    /// Test-only: the service has no reason to impersonate its clients, so
    /// this isn't part of the crate's API.
    struct Impersonation;

    impl Impersonation {
        /// Starts impersonating the client of `server`, which must already have
        /// read something from it.
        fn of(server: &NamedPipeServer) -> io::Result<Self> {
            // SAFETY: `server` is an open, connected pipe server handle that
            // stays open for the call.
            let impersonated = unsafe { ImpersonateNamedPipeClient(server.as_raw_handle()) };
            if impersonated == FALSE {
                return Err(io::Error::last_os_error());
            }
            Ok(Self)
        }
    }

    impl Drop for Impersonation {
        fn drop(&mut self) {
            // SAFETY: takes no argument; it ends this thread's impersonation.
            let _ = unsafe { RevertToSelf() };
        }
    }

    /// The impersonation level of the token the calling thread impersonates.
    fn thread_impersonation_level() -> io::Result<SECURITY_IMPERSONATION_LEVEL> {
        // SAFETY: takes no argument and returns a pseudo handle that needs no
        // closing.
        let thread = unsafe { GetCurrentThread() };
        let mut raw: HANDLE = ptr::null_mut();
        // SAFETY: `thread` is this thread's pseudo handle and `raw` is a valid
        // place for the token handle; `TRUE` checks access as the process, so
        // an identification-level token can be opened too.
        let opened = unsafe { OpenThreadToken(thread, TOKEN_QUERY, TRUE, &raw mut raw) };
        if opened == FALSE {
            return Err(io::Error::last_os_error());
        }
        // SAFETY: `raw` is an open token handle nothing else owns.
        let token = unsafe { OwnedHandle::from_raw_handle(raw) };
        let mut level: SECURITY_IMPERSONATION_LEVEL = 0;
        let length = u32::try_from(size_of::<SECURITY_IMPERSONATION_LEVEL>()).expect("small");
        let mut needed = 0_u32;
        // SAFETY: `token` is open with `TOKEN_QUERY`; `level` is writable and
        // `length` bytes long, the size of what this class returns.
        let read = unsafe {
            GetTokenInformation(
                token.as_raw_handle(),
                TokenImpersonationLevel,
                (&raw mut level).cast::<c_void>(),
                length,
                &raw mut needed,
            )
        };
        if read == FALSE {
            return Err(io::Error::last_os_error());
        }
        Ok(level)
    }

    #[test]
    fn clients_open_at_the_identification_level_only() {
        assert_eq!(
            OPEN_FLAGS,
            FILE_FLAG_OVERLAPPED | SECURITY_SQOS_PRESENT | SECURITY_IDENTIFICATION
        );
        assert_eq!(OPEN_FLAGS, 0x4011_0000);
    }

    #[test]
    fn the_server_can_identify_the_client_but_not_impersonate_it() {
        let runtime = tokio::runtime::Builder::new_current_thread()
            .enable_io()
            .build()
            .expect("a runtime");
        let level = runtime.block_on(async {
            let name = format!(r"\\.\pipe\keitora-sys-sqos-{}", std::process::id());
            let sid = current_process_sid().expect("the process's SID");
            let security =
                SecurityDescriptor::from_sddl(&format!("D:P(A;;FA;;;{sid})")).expect("valid SDDL");
            let mut options = ServerOptions::new();
            options.first_pipe_instance(true);
            let mut server = create_server(&options, &name, &security).expect("the server");
            let mut client = open_client(&name, CLIENT_ACCESS).expect("the client");
            server
                .connect()
                .await
                .expect("the server accepts the client");
            // A server can only impersonate a client it has read from.
            client.write_all(b"x").await.expect("the client writes");
            let mut byte = [0_u8; 1];
            server
                .read_exact(&mut byte)
                .await
                .expect("the server reads");
            // No `await` while impersonating: it is this thread's state.
            let impersonation = Impersonation::of(&server).expect("identification is allowed");
            let level = thread_impersonation_level();
            drop(impersonation);
            level.expect("the impersonation level")
        });
        // Without the quality of service flags it would be
        // `SecurityImpersonation`, which lets the server act as the client.
        assert_eq!(level, SecurityIdentification);
    }

    #[test]
    fn a_file_is_refused_and_closed_again() {
        let path =
            std::env::temp_dir().join(format!("keitora-sys-not-a-pipe-{}.txt", std::process::id()));
        std::fs::write(&path, b"not a pipe").expect("a temporary file");
        let error = open_client(path.to_str().expect("a UTF-8 path"), CLIENT_ACCESS)
            .expect_err("a file is not a pipe");
        assert_eq!(error.kind(), io::ErrorKind::InvalidInput);
        // `open_client` opens unshared, so the file could not be deleted if
        // the handle were still open.
        std::fs::remove_file(&path).expect("the file's handle was closed");
    }

    // The tests below don't run inside a tokio runtime, so they only use the
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
