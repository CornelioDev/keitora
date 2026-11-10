// SPDX-License-Identifier: MIT OR Apache-2.0

//! The Keitora desktop app. It hosts the Vue user interface in a Tauri window.
//!
//! The app is an untrusted client of the Keitora service
//! (docs/architecture.md): it will forward the interface's requests to the
//! service and the service's events back to the interface, but never decides
//! anything on its own. For now it only opens an empty window.

#![forbid(unsafe_code)]

use std::io;

/// Why [`run`] refuses to start a release build that loads the dev server.
const DEV_SERVER_IN_RELEASE: &str = "this release build would load its interface from the \
     development server instead of the files built into it; build it with \
     `pnpm tauri build`, not `cargo build --release`";

/// Starts the app and blocks until its last window is closed.
///
/// The window, its content and its permissions are configured in
/// `tauri.conf.json` and `capabilities/`.
///
/// # Errors
///
/// Returns an error if this is a release build that would load the interface
/// from the development server (see `check_frontend_source`), or if Tauri
/// fails to start the app or to create its window.
pub fn run() -> tauri::Result<()> {
    check_frontend_source(cfg!(debug_assertions), tauri::is_dev())?;
    tauri::Builder::default().run(tauri::generate_context!())
}

/// Checks that a release build serves the interface built into the binary.
///
/// Tauri only embeds the built interface when its `custom-protocol` feature
/// is on, which `pnpm tauri build` does but a plain `cargo build --release`
/// does not. Without it, the window loads `devUrl` (`http://localhost:1420`)
/// with no Content Security Policy, so any local process listening on that
/// port could serve the interface and use the window's permissions. Debug
/// builds are meant for that (`pnpm tauri dev`) and are allowed.
fn check_frontend_source(debug_build: bool, uses_dev_server: bool) -> tauri::Result<()> {
    if !debug_build && uses_dev_server {
        return Err(tauri::Error::Io(io::Error::new(
            io::ErrorKind::Unsupported,
            DEV_SERVER_IN_RELEASE,
        )));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn release_build_that_uses_the_dev_server_is_refused() {
        let err = check_frontend_source(false, true).unwrap_err();
        assert_eq!(err.to_string(), DEV_SERVER_IN_RELEASE);
    }

    #[test]
    fn release_build_with_the_built_interface_is_allowed() {
        assert!(check_frontend_source(false, false).is_ok());
    }

    #[test]
    fn debug_builds_are_allowed_either_way() {
        assert!(check_frontend_source(true, true).is_ok());
        assert!(check_frontend_source(true, false).is_ok());
    }
}
