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

#[cfg(test)]
mod tests {
    use std::fs;
    use std::path::{Path, PathBuf};

    /// Every `.rs` file under `dir`, recursively.
    fn rust_files(dir: &Path) -> Vec<PathBuf> {
        let mut found = Vec::new();
        for entry in fs::read_dir(dir).expect("couldn't list a folder") {
            let path = entry.expect("couldn't read a folder entry").path();
            if path.is_dir() {
                found.extend(rust_files(&path));
            } else if path.extension().is_some_and(|extension| extension == "rs") {
                found.push(path);
            }
        }
        found
    }

    /// The `unsafe_code` lint doesn't apply inside a macro expanded in
    /// another crate, so a macro exported from here could carry `unsafe` into
    /// crates that forbid it (ADR-0012). This crate exports none, and isn't a
    /// procedural macro crate either.
    #[test]
    fn the_crate_exports_no_macros() {
        let root = Path::new(env!("CARGO_MANIFEST_DIR"));
        // Built from pieces, so that this file doesn't match itself.
        let banned = [concat!("macro", "_export"), concat!("proc", "_macro")];
        for file in rust_files(&root.join("src")) {
            let text = fs::read_to_string(&file).expect("couldn't read a source file");
            for word in banned {
                assert!(!text.contains(word), "{} contains `{word}`", file.display());
            }
        }
        let manifest =
            fs::read_to_string(root.join("Cargo.toml")).expect("couldn't read Cargo.toml");
        assert!(
            !manifest.contains(concat!("proc", "-macro")),
            "keitora-sys must not be a procedural macro crate"
        );
    }
}
