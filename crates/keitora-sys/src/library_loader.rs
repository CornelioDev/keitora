// SPDX-License-Identifier: MIT OR Apache-2.0

//! Where Windows looks for the DLLs a process loads.

use std::io;

use windows_sys::Win32::System::LibraryLoader::{
    LOAD_LIBRARY_SEARCH_SYSTEM32, SetDefaultDllDirectories,
};

/// Makes every later DLL load of this process that names a DLL without a
/// full path look in `System32` only, with
/// `SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)`.
///
/// By default Windows also searches the application's folder, the current
/// folder and `PATH`, where a user could plant a DLL that a process running
/// as `SYSTEM` would then load (DLL planting). Call this first thing in
/// `main`, before anything can load a DLL. It doesn't cover the DLLs the
/// loader resolved before `main` started, which come from the executable's
/// import table.
///
/// The setting is for the whole process and can't be undone, which is what
/// is wanted; calling this more than once is harmless.
///
/// # Errors
///
/// Returns the Win32 error if Windows rejects the call. On the Windows
/// versions Keitora supports it doesn't fail; what to do if it does is the
/// caller's decision.
pub fn restrict_dll_search_to_system32() -> io::Result<()> {
    // SAFETY: `SetDefaultDllDirectories` takes a flags value by value and
    // touches no memory of ours; `LOAD_LIBRARY_SEARCH_SYSTEM32` is one of the
    // flags it documents as valid on its own.
    let succeeded = unsafe { SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32) };
    if succeeded == 0 {
        Err(io::Error::last_os_error())
    } else {
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn restricting_the_dll_search_succeeds_and_can_be_repeated() {
        // Changes the search order of the test process too, which only
        // stops it from loading DLLs from outside System32 by name.
        restrict_dll_search_to_system32().expect("the first call succeeds");
        restrict_dll_search_to_system32().expect("a second call succeeds too");
    }
}
