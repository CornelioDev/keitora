// SPDX-License-Identifier: MIT OR Apache-2.0

//! Security descriptors, built from text in the Security Descriptor
//! Definition Language (SDDL), and the identity of the current process.

use std::ffi::c_void;
use std::io;
use std::os::windows::io::{AsRawHandle, FromRawHandle, OwnedHandle};
use std::ptr;

use windows_sys::Win32::Foundation::{FALSE, HANDLE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertSidToStringSidW, ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{
    GetTokenInformation, PSECURITY_DESCRIPTOR, PSID, SECURITY_ATTRIBUTES, TOKEN_QUERY, TOKEN_USER,
    TokenUser,
};
use windows_sys::Win32::System::Threading::{GetCurrentProcess, OpenProcessToken};

/// A security descriptor that Windows allocated from an SDDL string, freed
/// when it is dropped.
///
/// It is read-only once built, so it can be shared and moved between threads.
#[derive(Debug)]
pub struct SecurityDescriptor {
    /// Memory from `LocalAlloc`, never null.
    raw: PSECURITY_DESCRIPTOR,
}

// SAFETY: the descriptor is never written after `ConvertStringSecurityDescriptor
// ToSecurityDescriptorW` returns, and `LocalFree` may be called from any
// thread, so sharing and moving the owner is sound.
unsafe impl Send for SecurityDescriptor {}
// SAFETY: see the `Send` impl; every method takes `&self` and only reads.
unsafe impl Sync for SecurityDescriptor {}

impl SecurityDescriptor {
    /// Builds a security descriptor from `sddl`, such as
    /// `D:P(A;;FA;;;SY)`.
    ///
    /// # Errors
    ///
    /// Returns [`io::ErrorKind::InvalidInput`] if `sddl` has a NUL, or the
    /// Win32 error if Windows can't parse it.
    pub fn from_sddl(sddl: &str) -> io::Result<Self> {
        if sddl.contains('\0') {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "an SDDL string can't contain NUL",
            ));
        }
        let wide: Vec<u16> = sddl.encode_utf16().chain(std::iter::once(0)).collect();
        let mut raw: PSECURITY_DESCRIPTOR = ptr::null_mut();
        // SAFETY: `wide` is a NUL-terminated UTF-16 string that lives past the
        // call; `raw` is a valid place for the output pointer; the size out
        // parameter is optional and passed as null.
        let converted = unsafe {
            ConvertStringSecurityDescriptorToSecurityDescriptorW(
                wide.as_ptr(),
                SDDL_REVISION_1,
                &raw mut raw,
                ptr::null_mut(),
            )
        };
        if converted == FALSE || raw.is_null() {
            return Err(io::Error::last_os_error());
        }
        Ok(Self { raw })
    }

    /// A `SECURITY_ATTRIBUTES` that points to this descriptor and isn't
    /// inherited by child processes. It borrows the descriptor, so it must
    /// not be used after `self` is dropped.
    pub(crate) fn attributes(&self) -> io::Result<SECURITY_ATTRIBUTES> {
        let length = u32::try_from(size_of::<SECURITY_ATTRIBUTES>())
            .map_err(|_| io::Error::other("SECURITY_ATTRIBUTES is too large"))?;
        Ok(SECURITY_ATTRIBUTES {
            nLength: length,
            lpSecurityDescriptor: self.raw,
            bInheritHandle: FALSE,
        })
    }
}

impl Drop for SecurityDescriptor {
    fn drop(&mut self) {
        // SAFETY: `raw` came from `ConvertStringSecurityDescriptorTo
        // SecurityDescriptorW`, which documents `LocalFree` as the way to free
        // it, and it is freed once, here.
        let _ = unsafe { LocalFree(self.raw) };
    }
}

/// Room for a `TOKEN_USER`: a `SID_AND_ATTRIBUTES` plus the SID it points to,
/// which Windows caps at `SECURITY_MAX_SID_SIZE` (68) bytes. `u64` items give
/// the buffer the alignment a `TOKEN_USER` needs.
const TOKEN_USER_WORDS: usize = 32;

/// An upper bound on the characters of a SID in text form, which is `S-`, a
/// revision, an identifier authority and up to 15 sub-authorities, each at
/// most 10 digits and a separator: 190 characters at the very most.
const MAX_SID_TEXT: usize = 256;

/// The SID of the account the current process runs as, in the text form SDDL
/// uses: `S-1-5-18` for the installed service (`SYSTEM`), the developer's own
/// SID when the service runs with `--console`.
///
/// Callers use it to name themselves in a security descriptor, so that a DACL
/// can grant the service what it needs without granting it to everyone who
/// shares its session.
///
/// # Errors
///
/// Returns the Win32 error if the process token can't be opened or read, or if
/// Windows can't turn its user's SID into text.
pub fn current_process_sid() -> io::Result<String> {
    let token = process_token()?;
    let mut buffer = [0_u64; TOKEN_USER_WORDS];
    let length = u32::try_from(size_of_val(&buffer))
        .map_err(|_| io::Error::other("the token information buffer is too large"))?;
    let handle: HANDLE = token.as_raw_handle();
    let mut needed = 0_u32;
    // SAFETY: `handle` is an open token with `TOKEN_QUERY`; `buffer` is
    // writable, aligned for a `TOKEN_USER` and `length` bytes long, which is
    // more than a `TOKEN_USER` and the largest SID take; `needed` is a valid
    // place for the length Windows reports.
    let read = unsafe {
        GetTokenInformation(
            handle,
            TokenUser,
            buffer.as_mut_ptr().cast::<c_void>(),
            length,
            &raw mut needed,
        )
    };
    if read == FALSE {
        return Err(io::Error::last_os_error());
    }
    // SAFETY: the call above succeeded, so it wrote a `TOKEN_USER` at the
    // start of `buffer`, which is aligned for one; the `TOKEN_USER` is `Copy`
    // and its `Sid` points into `buffer`, which outlives the borrow below.
    let user = unsafe { buffer.as_ptr().cast::<TOKEN_USER>().read() };
    sid_to_text(user.User.Sid)
}

/// The current process's access token, open for reading.
fn process_token() -> io::Result<OwnedHandle> {
    // SAFETY: `GetCurrentProcess` takes no argument and returns a pseudo
    // handle to the calling process that needs no closing.
    let process = unsafe { GetCurrentProcess() };
    let mut raw: HANDLE = ptr::null_mut();
    // SAFETY: `process` is the pseudo handle just returned, and `raw` is a
    // valid place for the token handle.
    let opened = unsafe { OpenProcessToken(process, TOKEN_QUERY, &raw mut raw) };
    if opened == FALSE {
        return Err(io::Error::last_os_error());
    }
    if raw.is_null() {
        return Err(io::Error::other("OpenProcessToken returned no handle"));
    }
    // SAFETY: `raw` is an open token handle that only this function owns, so
    // the `OwnedHandle` is its sole owner and closes it exactly once.
    Ok(unsafe { OwnedHandle::from_raw_handle(raw) })
}

/// `sid` in the text form SDDL uses.
///
/// `sid` must point to a valid SID that stays alive for the call.
fn sid_to_text(sid: PSID) -> io::Result<String> {
    let mut text: *mut u16 = ptr::null_mut();
    // SAFETY: `sid` is the SID of the token read above, valid for the call,
    // and `text` is a valid place for the string Windows allocates.
    let converted = unsafe { ConvertSidToStringSidW(sid, &raw mut text) };
    if converted == FALSE || text.is_null() {
        return Err(io::Error::last_os_error());
    }
    let mut units: Vec<u16> = Vec::new();
    let mut terminated = false;
    for offset in 0..MAX_SID_TEXT {
        let location = text.wrapping_add(offset);
        // SAFETY: `text` is a NUL-terminated UTF-16 string that
        // `ConvertSidToStringSidW` allocated, and the loop stops at its NUL,
        // so every read is inside that string.
        let unit = unsafe { location.read() };
        if unit == 0 {
            terminated = true;
            break;
        }
        units.push(unit);
    }
    // SAFETY: `text` came from `ConvertSidToStringSidW`, which documents
    // `LocalFree` as the way to free it, and it is freed once, here.
    let _ = unsafe { LocalFree(text.cast::<c_void>()) };
    if !terminated {
        return Err(io::Error::other(
            "ConvertSidToStringSidW returned an unterminated string",
        ));
    }
    String::from_utf16(&units).map_err(|_| io::Error::other("a SID's text isn't valid UTF-16"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn builds_a_descriptor_from_valid_sddl() {
        let descriptor = SecurityDescriptor::from_sddl("D:P(A;;FA;;;SY)").expect("valid SDDL");
        let attributes = descriptor.attributes().expect("attributes");
        assert!(!attributes.lpSecurityDescriptor.is_null());
        assert_eq!(attributes.bInheritHandle, FALSE);
    }

    #[test]
    fn rejects_invalid_sddl() {
        assert!(SecurityDescriptor::from_sddl("not sddl").is_err());
        assert!(SecurityDescriptor::from_sddl("D:P(A;;FA;;;SY)\0").is_err());
    }

    #[test]
    fn reads_the_sid_of_the_current_process() {
        let sid = current_process_sid().expect("the process has a user SID");
        assert!(sid.starts_with("S-1-"), "{sid} doesn't look like a SID");
        assert!(sid.len() > 8, "{sid} is too short for a user SID");
        // It names a trustee Windows accepts in a DACL.
        SecurityDescriptor::from_sddl(&format!("D:P(A;;FA;;;{sid})"))
            .expect("the SID works as an SDDL trustee");
    }

    #[test]
    fn the_sid_is_the_same_every_time() {
        let first = current_process_sid().expect("a SID");
        let second = current_process_sid().expect("a SID");
        assert_eq!(first, second);
    }
}
