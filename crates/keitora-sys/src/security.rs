// SPDX-License-Identifier: MIT OR Apache-2.0

//! Security descriptors, built from text in the Security Descriptor
//! Definition Language (SDDL).

use std::io;
use std::ptr;

use windows_sys::Win32::Foundation::{FALSE, LocalFree};
use windows_sys::Win32::Security::Authorization::{
    ConvertStringSecurityDescriptorToSecurityDescriptorW, SDDL_REVISION_1,
};
use windows_sys::Win32::Security::{PSECURITY_DESCRIPTOR, SECURITY_ATTRIBUTES};

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
}
