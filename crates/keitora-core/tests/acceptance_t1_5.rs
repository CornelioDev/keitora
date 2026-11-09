// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-1.5: the app rule matcher, which decides whether a
//! running process is blocked by a blocklist, and which processes are never
//! blocked.
//!
//! # Expected public API
//!
//! Everything lives in `keitora_core::app`, compiled in every build (no
//! `cfg(test)` or feature gate):
//!
//! - `ProtectedProcesses::new(windows_dir: &str, service_exe: &str) ->
//!   ProtectedProcesses`: the processes that are never blocked. The service
//!   passes the Windows directory (from `GetSystemWindowsDirectoryW`, such as
//!   `C:\Windows`, never hard-coded) and the full path of its own executable.
//!   It never fails.
//! - `ProtectedProcesses::is_protected(&self, image_path: &str) -> bool`.
//! - `AppMatcher::new(blocklist: &Blocklist, protected: &ProtectedProcesses)
//!   -> AppMatcher`. It never fails, and doesn't need the blocklist to have
//!   passed `Blocklist::validate`.
//! - `AppMatcher::verdict(&self, image_path: &str) -> AppVerdict`, with
//!   `enum AppVerdict { Allowed, Blocked, Protected }` (`Debug`, `Clone`,
//!   `Copy`, `PartialEq`, `Eq`). `Protected` whenever the path is protected,
//!   whatever the blocklist says; otherwise `Blocked` when an app target
//!   matches; otherwise `Allowed`.
//! - `AppMatcher::is_blocked(&self, image_path: &str) -> bool`, which is
//!   `verdict(..) == AppVerdict::Blocked`.
//! - `AppMatcher` and `ProtectedProcesses` are `Send + Sync`.
//!
//! `image_path` is the Win32 path of a running process's executable, as
//! `QueryFullProcessImageNameW` (flags 0) returns it in T-2.6. Whether the
//! blocklist applies at all (focus or break, an active session) is the
//! engine's job, not the matcher's.
//!
//! # How paths are compared (pinned here)
//!
//! Matching is textual: nothing is opened, resolved or looked up (#11).
//!
//! 1. A leading `\\?\` or `\??\` before a drive letter is ignored, in image
//!    paths and in the two `ProtectedProcesses::new` inputs.
//! 2. `/` and `\` are the same separator.
//! 3. Letter case is ignored one character at a time, as NTFS does with its
//!    upcase table: `É` equals `é`, `Ж` equals `ж`. No other folding: `ß` is
//!    not `SS`, the Kelvin sign `K` (U+212A) is not `K`, `İ` (U+0130) is not
//!    `I`, full-width letters are not ASCII, and composed and decomposed
//!    accents differ. A lookalike folder that a user can create (in `C:\` or
//!    `D:\`, say) must never inherit protection.
//! 4. Nothing else changes: no trimming, no removal of trailing dots or
//!    spaces (`C:\Windows.\` and `C:\Windows \` are other folders), no 8.3
//!    short names (`PROGRA~1`), no `.`/`..`, no wildcards.
//! 5. A name target (`windows_exe`) equals the image path's whole file name,
//!    the text after its last separator. A path target (`windows_path`)
//!    equals the whole image path: never a prefix (`C:\` passes validation
//!    and must match nothing real), never a suffix.
//! 6. An app target with both identifiers blocks a process matching either
//!    ("match by path as well as name", `docs/threat-model.md`).
//! 7. Only full paths are ever blocked: a drive path (with or without
//!    `\\?\`/`\??\`) or a UNC path (`\\server\share\…`, `\\?\UNC\…`), which
//!    name targets still block. An empty, bare, relative, drive-relative or
//!    rooted path, including an NT path such as `\Device\HarddiskVolume3\…`,
//!    is never blocked: the matcher can't tell whether it is a protected
//!    process, so it fails toward not blocking.
//!
//! Left to the developer (not pinned): whether a name target without an
//! extension (`steam`) matches `steam.exe`; whether Keitora's other
//! executables (desktop app, native messaging host) are protected; more
//! protected processes than the minimum listed below; `..` and repeated
//! separators; `\\.\` prefixes; `ı` (U+0131).
//!
//! # Accepted limitation (F0 review, L3)
//!
//! Copying a program under another name and into another folder evades both
//! name and path rules, as the matcher only sees the path's text. Matching
//! by hash or signer is post-MVP (`docs/threat-model.md`). The same goes for
//! a program launched through an 8.3 short path when only a path target
//! names it. `copying_a_program_under_another_name_and_path_evades_name_and_path_rules`
//! and `short_name_form_of_a_target_path_is_not_resolved` record this.
//!
//! # Criteria covered
//!
//! The task: "App rule matcher: executable name (case-insensitive) and path;
//! a protected-process list that is never blocked (`explorer.exe`,
//! `csrss.exe`, `winlogon.exe`, the service itself, …). Security: protected
//! processes match by full path, so an executable renamed to a protected name
//! in another folder isn't protected (#8). The matcher never opens or
//! resolves a path (#11). Its result is only a candidate: the service
//! terminates only the interactive user's own processes, enforced in T-2.6
//! (#10)."
//!
//! - **AC1 — executable name, case-insensitive**:
//!   `name_target_blocks_the_executable_in_any_folder`,
//!   `name_target_ignores_ascii_letter_case`,
//!   `name_target_ignores_unicode_letter_case_one_character_at_a_time`,
//!   `name_target_matches_the_whole_file_name_only`,
//!   `name_target_has_no_wildcards`,
//!   `name_target_reads_both_slash_kinds_as_separators`,
//!   `name_target_blocks_device_prefixed_and_network_image_paths`.
//! - **AC2 — path**: `path_target_blocks_exactly_that_executable`,
//!   `path_target_ignores_letter_case`,
//!   `path_target_reads_both_slash_kinds_as_separators`,
//!   `path_target_blocks_device_prefixed_forms_of_the_same_path`,
//!   `name_and_path_together_block_a_process_matching_either`.
//! - **AC3 — protected processes are never blocked (`explorer.exe`,
//!   `csrss.exe`, `winlogon.exe`, the service itself, …)**:
//!   `every_listed_system_process_is_protected_against_any_target`,
//!   `service_itself_is_protected_against_any_target`,
//!   `protected_path_is_protected_whatever_the_blocklist`,
//!   `protection_ignores_case_slash_kind_and_device_prefix`,
//!   `windows_directory_comes_from_the_caller`,
//!   `service_path_input_is_read_like_an_image_path`,
//!   `protection_is_a_list_not_a_folder`.
//! - **S1 — #8, protection by full path only**:
//!   `executable_with_a_protected_name_in_another_folder_is_not_protected`,
//!   `lookalike_paths_never_inherit_protection`.
//! - **S2 — #11, never opens or resolves a path**:
//!   `paths_are_compared_as_text_whether_or_not_they_exist`,
//!   `short_name_form_of_a_target_path_is_not_resolved`. The static half of
//!   #11 is `crates/keitora-core/clippy.toml`, which bans file access in the
//!   crate (security review to confirm it covers what the code could use).
//! - **S3 — #10, the verdict is only a candidate**: not testable in a pure
//!   crate; the service enforces it in T-2.6. The verdict says nothing about
//!   who owns a process, and these tests don't assume it does.
//! - **C1 — carryover: full paths, never prefixes (`C:\` is valid)**:
//!   `path_target_compares_full_paths_never_prefixes`,
//!   `path_target_is_never_a_suffix_or_a_pattern`.
//! - **C2 — carryover: the textual normalization is pinned** (see above):
//!   the AC1, AC2, AC3 and S1 tests, and
//!   `image_path_that_is_not_a_full_path_is_never_blocked`.
//! - **C3 — carryover: L3 documented**:
//!   `copying_a_program_under_another_name_and_path_evades_name_and_path_rules`.
//! - **R1 — derived: only Windows app identifiers count**:
//!   `site_targets_and_exceptions_never_block_or_unblock_an_app`,
//!   `android_and_ios_identifiers_are_ignored_on_windows`,
//!   `app_without_windows_identifiers_blocks_nothing`,
//!   `empty_blocklist_blocks_nothing`.
//! - **R2 — derived: hostile input never panics, and fails safe**:
//!   `image_path_that_is_not_a_full_path_is_never_blocked`,
//!   `hostile_image_paths_never_make_the_matcher_panic`,
//!   `hostile_constructor_inputs_never_make_it_panic`,
//!   `hostile_blocklist_values_never_make_the_matcher_panic`,
//!   `long_image_path_at_the_windows_limit_is_still_matched`.
//! - **R3 — derived: stateless, order-independent, any size, unvalidated
//!   input, shareable**: `one_matcher_answers_many_image_paths_independently`,
//!   `order_of_targets_does_not_change_the_verdict`,
//!   `matching_target_is_found_in_a_blocklist_at_the_maximum_size`,
//!   `unvalidated_blocklist_still_builds_a_working_matcher`,
//!   `matcher_and_protected_list_can_be_shared_between_threads`,
//!   `verdict_agrees_with_is_blocked_and_is_protected`.

#![cfg(test)]

use keitora_core::app::{AppMatcher, AppVerdict, ProtectedProcesses};
use keitora_core::model::{AppIdentifiers, Blocklist, MAX_TARGETS, SitePattern, Target};

/// The Windows directory most tests pass, as `GetSystemWindowsDirectoryW`
/// often reports it.
const WINDOWS: &str = r"C:\Windows";

/// The service's own executable in most tests.
const SERVICE: &str = r"C:\Program Files\Keitora\keitora-service.exe";

/// Processes in the Windows directory that are always protected.
const PROTECTED_IN_WINDOWS_DIR: &[&str] = &["explorer.exe"];

/// Processes in `System32` that are always protected: the minimum list. The
/// task names `csrss.exe` and `winlogon.exe`; the rest end the session, the
/// shell or input if killed. The developer may protect more.
const PROTECTED_IN_SYSTEM32: &[&str] = &[
    "csrss.exe",
    "winlogon.exe",
    "wininit.exe",
    "smss.exe",
    "services.exe",
    "lsass.exe",
    "svchost.exe",
    "dwm.exe",
    "sihost.exe",
    "ctfmon.exe",
    "fontdrvhost.exe",
    "LogonUI.exe",
    "userinit.exe",
    "taskhostw.exe",
    "conhost.exe",
    "RuntimeBroker.exe",
];

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn blocklist(targets: Vec<Target>) -> Blocklist {
    Blocklist {
        id: "3f0c6b1e-8a52-4c1d-9e2b-7d4a1f6c0b9e".to_owned(),
        name: "Deep work".to_owned(),
        color: "#e5484d".to_owned(),
        targets,
        exceptions: Vec::new(),
        block_during_breaks: false,
    }
}

fn app(ids: AppIdentifiers) -> Target {
    Target::App {
        name: "Some app".to_owned(),
        ids,
    }
}

fn by_name(exe: &str) -> Target {
    app(AppIdentifiers {
        windows_exe: Some(exe.to_owned()),
        ..AppIdentifiers::default()
    })
}

fn by_path(path: &str) -> Target {
    app(AppIdentifiers {
        windows_path: Some(path.to_owned()),
        ..AppIdentifiers::default()
    })
}

fn by_both(exe: &str, path: &str) -> Target {
    app(AppIdentifiers {
        windows_exe: Some(exe.to_owned()),
        windows_path: Some(path.to_owned()),
        ..AppIdentifiers::default()
    })
}

fn site(pattern: &str) -> Target {
    Target::Site {
        pattern: SitePattern::new(pattern),
    }
}

fn protected() -> ProtectedProcesses {
    ProtectedProcesses::new(WINDOWS, SERVICE)
}

fn matcher(targets: &[Target]) -> AppMatcher {
    AppMatcher::new(&blocklist(targets.to_vec()), &protected())
}

fn verdict(targets: &[Target], image_path: &str) -> AppVerdict {
    matcher(targets).verdict(image_path)
}

/// Every pinned protected process, at its full path under `windows_dir`.
fn protected_system_paths(windows_dir: &str) -> Vec<String> {
    let mut paths = Vec::new();
    for name in PROTECTED_IN_WINDOWS_DIR {
        paths.push(format!(r"{windows_dir}\{name}"));
    }
    for name in PROTECTED_IN_SYSTEM32 {
        paths.push(format!(r"{windows_dir}\System32\{name}"));
    }
    paths
}

/// Every pinned protected process's file name.
fn protected_names() -> Vec<&'static str> {
    PROTECTED_IN_WINDOWS_DIR
        .iter()
        .chain(PROTECTED_IN_SYSTEM32.iter())
        .copied()
        .collect()
}

fn assert_send_sync<T: Send + Sync>() {}

// ---------------------------------------------------------------------------
// AC1: executable name, case-insensitive
// ---------------------------------------------------------------------------

#[test]
fn name_target_blocks_the_executable_in_any_folder() {
    let targets = [by_name("steam.exe")];
    for image in [
        r"C:\Program Files (x86)\Steam\steam.exe",
        r"D:\Games\steam.exe",
        r"C:\Users\Ana\AppData\Local\Temp\steam.exe",
        r"C:\steam.exe",
        r"Z:\a\b\c\d\e\f\steam.exe",
    ] {
        assert_eq!(
            verdict(&targets, image),
            AppVerdict::Blocked,
            "steam.exe should block {image}"
        );
    }
    assert_eq!(
        verdict(&targets, r"C:\Program Files\Mozilla Firefox\firefox.exe"),
        AppVerdict::Allowed
    );
}

#[test]
fn name_target_ignores_ascii_letter_case() {
    for (exe, image) in [
        ("steam.exe", r"C:\Games\STEAM.EXE"),
        ("STEAM.EXE", r"C:\Games\steam.exe"),
        ("Steam.Exe", r"C:\Games\sTEAM.eXE"),
        (
            "Discord.exe",
            r"C:\Users\Ana\AppData\Local\Discord\app-1.0\discord.exe",
        ),
    ] {
        assert_eq!(
            verdict(&[by_name(exe)], image),
            AppVerdict::Blocked,
            "{exe} should block {image}"
        );
    }
}

#[test]
fn name_target_ignores_unicode_letter_case_one_character_at_a_time() {
    for (exe, image) in [
        ("\u{c9}DITEUR.EXE", "C:\\Apps\\\u{e9}diteur.exe"),
        ("\u{e9}diteur.exe", "C:\\Apps\\\u{c9}DITEUR.EXE"),
        (
            "\u{416}\u{423}\u{420}\u{41d}\u{410}\u{41b}.exe",
            "C:\\Apps\\\u{436}\u{443}\u{440}\u{43d}\u{430}\u{43b}.EXE",
        ),
        ("\u{3a9}mega.exe", "C:\\Apps\\\u{3c9}MEGA.exe"),
        ("\u{f1}and\u{fa}.exe", "C:\\Apps\\\u{d1}AND\u{da}.EXE"),
    ] {
        assert_eq!(
            verdict(&[by_name(exe)], image),
            AppVerdict::Blocked,
            "{exe:?} should block {image:?}"
        );
    }
}

#[test]
fn name_target_matches_the_whole_file_name_only() {
    let targets = [by_name("steam.exe")];
    for image in [
        r"C:\Games\notsteam.exe",
        r"C:\Games\steam.exe.bak",
        r"C:\Games\steam.exe2",
        r"C:\Games\steam.ex",
        r"C:\Games\steam",
        r"C:\Games\steam.exe.lnk",
        r"C:\Games\my steam.exe",
        r"C:\steam.exe\other.exe",
        r"C:\Games\steam.exe\",
        r"C:\Games\steam.exe:hidden.exe",
    ] {
        assert_ne!(
            verdict(&targets, image),
            AppVerdict::Blocked,
            "steam.exe must not block {image}"
        );
    }
    assert_ne!(
        verdict(&[by_name("team.exe")], r"C:\Games\steam.exe"),
        AppVerdict::Blocked,
        "team.exe must not block steam.exe"
    );
    assert_ne!(
        verdict(&[by_name("explorer.exe")], r"C:\Tools\myexplorer.exe"),
        AppVerdict::Blocked,
        "explorer.exe must not block myexplorer.exe"
    );
}

#[test]
fn name_target_has_no_wildcards() {
    // Validation rejects `*` and `?` in a file name; an unvalidated
    // blocklist still never treats them as wildcards.
    for exe in ["*", "*.exe", "?team.exe", "steam*", "s*.exe", "*.*"] {
        assert_ne!(
            verdict(&[by_name(exe)], r"C:\Games\steam.exe"),
            AppVerdict::Blocked,
            "{exe} must not act as a wildcard"
        );
    }
}

#[test]
fn name_target_reads_both_slash_kinds_as_separators() {
    let targets = [by_name("steam.exe")];
    for image in [
        r"C:/Games/steam.exe",
        r"C:\Games/steam.exe",
        r"C:/Games\steam.exe",
        r"C:\Games\Steam/steam.exe",
    ] {
        assert_eq!(
            verdict(&targets, image),
            AppVerdict::Blocked,
            "steam.exe should block {image}"
        );
    }
}

#[test]
fn name_target_blocks_device_prefixed_and_network_image_paths() {
    let targets = [by_name("steam.exe")];
    for image in [
        r"\\?\C:\Games\steam.exe",
        r"\??\C:\Games\steam.exe",
        r"\\server\share\Games\steam.exe",
        r"\\?\UNC\server\share\Games\steam.exe",
    ] {
        assert_eq!(
            verdict(&targets, image),
            AppVerdict::Blocked,
            "steam.exe should block {image}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC2: path
// ---------------------------------------------------------------------------

#[test]
fn path_target_blocks_exactly_that_executable() {
    let path = r"C:\Program Files (x86)\Steam\steam.exe";
    assert_eq!(verdict(&[by_path(path)], path), AppVerdict::Blocked);
    assert_eq!(
        verdict(&[by_path(path)], r"D:\Other\steam.exe"),
        AppVerdict::Allowed,
        "a path target doesn't block the same file name elsewhere"
    );
}

#[test]
fn path_target_ignores_letter_case() {
    for (path, image) in [
        (r"C:\Games\Steam\steam.exe", r"c:\games\steam\STEAM.EXE"),
        (r"c:\games\steam\steam.exe", r"C:\GAMES\STEAM\STEAM.EXE"),
        (
            "C:\\Jeux\\\u{c9}diteur\\app.exe",
            "c:\\JEUX\\\u{e9}DITEUR\\APP.EXE",
        ),
    ] {
        assert_eq!(
            verdict(&[by_path(path)], image),
            AppVerdict::Blocked,
            "{path:?} should block {image:?}"
        );
    }
}

#[test]
fn path_target_reads_both_slash_kinds_as_separators() {
    assert_eq!(
        verdict(
            &[by_path(r"C:\Games/Steam/steam.exe")],
            r"C:\Games\Steam\steam.exe"
        ),
        AppVerdict::Blocked
    );
    assert_eq!(
        verdict(
            &[by_path(r"C:\Games\Steam\steam.exe")],
            r"C:/Games/Steam/steam.exe"
        ),
        AppVerdict::Blocked
    );
}

#[test]
fn path_target_blocks_device_prefixed_forms_of_the_same_path() {
    let targets = [by_path(r"C:\Games\Steam\steam.exe")];
    for image in [
        r"\\?\C:\Games\Steam\steam.exe",
        r"\??\C:\Games\Steam\steam.exe",
    ] {
        assert_eq!(
            verdict(&targets, image),
            AppVerdict::Blocked,
            "the path target should block {image}"
        );
    }
}

#[test]
fn name_and_path_together_block_a_process_matching_either() {
    let targets = [by_both("steam.exe", r"C:\Games\Steam\steam.exe")];
    // Both match.
    assert_eq!(
        verdict(&targets, r"C:\Games\Steam\steam.exe"),
        AppVerdict::Blocked
    );
    // Only the name matches: copied to another folder.
    assert_eq!(verdict(&targets, r"D:\Copy\steam.exe"), AppVerdict::Blocked);
    // Only the path matches: the name field names another file.
    let mismatched = [by_both("launcher.exe", r"C:\Games\Steam\steam.exe")];
    assert_eq!(
        verdict(&mismatched, r"C:\Games\Steam\steam.exe"),
        AppVerdict::Blocked
    );
    // Neither matches.
    assert_eq!(
        verdict(&targets, r"C:\Games\Steam\renamed.exe"),
        AppVerdict::Allowed
    );
}

// ---------------------------------------------------------------------------
// C1: full paths, never prefixes or suffixes
// ---------------------------------------------------------------------------

#[test]
fn path_target_compares_full_paths_never_prefixes() {
    let image = r"C:\Games\Steam\steam.exe";
    // `C:\` passes validation; it must match nothing real.
    for path in [
        r"C:\",
        r"C:",
        r"C:\Games",
        r"C:\Games\",
        r"C:\Games\Steam",
        r"C:\Games\Steam\",
        r"C:\Games\Steam\steam",
        r"C:\Games\Steam\steam.ex",
    ] {
        assert_eq!(
            verdict(&[by_path(path)], image),
            AppVerdict::Allowed,
            "{path} must not block {image}"
        );
    }
    let target = [by_path(image)];
    for other in [
        r"C:\Games\Steam\steam.exe.bak",
        r"C:\Games\Steam\steam.exe\child.exe",
        r"C:\Games\Steam\steam.exe:stream",
        r"C:\Games\Steam\bin\steam.exe",
        r"C:\Games\Old\Steam\steam.exe",
        r"D:\Games\Steam\steam.exe",
        r"D:\C\Games\Steam\steam.exe",
    ] {
        assert_eq!(
            verdict(&target, other),
            AppVerdict::Allowed,
            "{image} must not block {other}"
        );
    }
    // `C:\` doesn't block any of a range of processes either.
    let root = [by_path(r"C:\")];
    for other in [
        r"C:\steam.exe",
        r"C:\Program Files\Mozilla Firefox\firefox.exe",
        r"C:\Users\Ana\Downloads\game.exe",
    ] {
        assert_eq!(
            verdict(&root, other),
            AppVerdict::Allowed,
            "C:\\ blocked {other}"
        );
    }
}

#[test]
fn path_target_is_never_a_suffix_or_a_pattern() {
    // An unvalidated blocklist can hold any text in `windows_path`; none of
    // these is the full path of `C:\Games\steam.exe`.
    for path in [
        "",
        "steam.exe",
        r"Games\steam.exe",
        r"\Games\steam.exe",
        r"C:steam.exe",
        r"C:Games\steam.exe",
        "*",
        r"C:\Games\*",
        r"C:\Games\*.exe",
        r"C:\Games\?team.exe",
        r"C:\*\steam.exe",
    ] {
        assert_ne!(
            verdict(&[by_path(path)], r"C:\Games\steam.exe"),
            AppVerdict::Blocked,
            "{path:?} must not block C:\\Games\\steam.exe"
        );
    }
}

// ---------------------------------------------------------------------------
// AC3: the protected-process list
// ---------------------------------------------------------------------------

#[test]
fn every_listed_system_process_is_protected_against_any_target() {
    let protected = protected();
    for name in PROTECTED_IN_WINDOWS_DIR {
        assert!(
            protected.is_protected(&format!(r"C:\Windows\{name}")),
            "{name} in the Windows directory must be protected"
        );
    }
    for name in PROTECTED_IN_SYSTEM32 {
        assert!(
            protected.is_protected(&format!(r"C:\Windows\System32\{name}")),
            "{name} in System32 must be protected"
        );
    }

    for path in protected_system_paths(WINDOWS) {
        let name = path
            .rsplit('\\')
            .next()
            .expect("a protected path has a file name");
        let every_target = [by_name(name), by_path(&path), by_both(name, &path)];
        for target in &every_target {
            assert_eq!(
                verdict(std::slice::from_ref(target), &path),
                AppVerdict::Protected,
                "{path} must be protected against {target:?}"
            );
        }
        assert_eq!(
            verdict(&every_target, &path),
            AppVerdict::Protected,
            "{path} must be protected against all its targets at once"
        );
        assert!(!matcher(&every_target).is_blocked(&path));
    }
}

#[test]
fn service_itself_is_protected_against_any_target() {
    assert!(protected().is_protected(SERVICE));
    for targets in [
        vec![by_name("keitora-service.exe")],
        vec![by_path(SERVICE)],
        vec![by_both("keitora-service.exe", SERVICE)],
    ] {
        assert_eq!(
            verdict(&targets, SERVICE),
            AppVerdict::Protected,
            "the service must be protected against {targets:?}"
        );
    }
}

#[test]
fn protected_path_is_protected_whatever_the_blocklist() {
    let mut images = protected_system_paths(WINDOWS);
    images.push(SERVICE.to_owned());
    let blocklists = [
        Vec::new(),
        vec![by_name("steam.exe")],
        vec![site("reddit.com")],
    ];
    for targets in &blocklists {
        for image in &images {
            assert_eq!(
                verdict(targets, image),
                AppVerdict::Protected,
                "{image} with targets {targets:?}"
            );
        }
    }
}

#[test]
fn protection_ignores_case_slash_kind_and_device_prefix() {
    let protected = protected();
    for image in [
        r"C:\Windows\explorer.exe",
        r"c:\windows\explorer.exe",
        r"C:\WINDOWS\EXPLORER.EXE",
        r"C:/Windows/explorer.exe",
        r"C:\Windows/explorer.exe",
        r"\\?\C:\Windows\explorer.exe",
        r"\??\C:\Windows\explorer.exe",
        r"\\?\c:\WINDOWS\Explorer.EXE",
        r"C:\WINDOWS\system32\CSRSS.EXE",
        r"C:\Windows\SYSTEM32\winlogon.exe",
        r"C:/Windows/System32/svchost.exe",
        r"\\?\C:\Windows\System32\lsass.exe",
        r"c:\program files\keitora\KEITORA-SERVICE.EXE",
        r"C:/Program Files/Keitora/keitora-service.exe",
        r"\\?\C:\Program Files\Keitora\keitora-service.exe",
        r"\??\C:\Program Files\Keitora\keitora-service.exe",
    ] {
        assert!(protected.is_protected(image), "{image} must be protected");
        assert_eq!(
            verdict(&[by_name("explorer.exe"), by_name("csrss.exe")], image),
            AppVerdict::Protected,
            "{image}"
        );
    }
}

#[test]
fn windows_directory_comes_from_the_caller() {
    // The same directory written in other ways protects the same processes.
    for windows_dir in [
        r"C:\Windows",
        r"C:\Windows\",
        r"C:\WINDOWS",
        r"c:\windows",
        r"C:/Windows",
        r"\\?\C:\Windows",
    ] {
        let protected = ProtectedProcesses::new(windows_dir, SERVICE);
        for path in protected_system_paths(r"C:\Windows") {
            assert!(
                protected.is_protected(&path),
                "with the Windows directory {windows_dir:?}, {path} must be protected"
            );
        }
    }

    // Windows installed elsewhere: its processes are protected, and the
    // usual `C:\Windows` is just a folder (a user may be able to create it).
    let elsewhere = ProtectedProcesses::new(r"D:\WINNT", SERVICE);
    for path in protected_system_paths(r"D:\WINNT") {
        assert!(elsewhere.is_protected(&path), "{path} must be protected");
    }
    for path in protected_system_paths(r"C:\Windows") {
        assert!(
            !elsewhere.is_protected(&path),
            "{path} must not be protected when Windows is in D:\\WINNT"
        );
        let name = path
            .rsplit('\\')
            .next()
            .expect("a protected path has a file name");
        let matcher = AppMatcher::new(&blocklist(vec![by_name(name)]), &elsewhere);
        assert_eq!(matcher.verdict(&path), AppVerdict::Blocked, "{path}");
    }
}

#[test]
fn service_path_input_is_read_like_an_image_path() {
    for service in [
        r"\\?\C:\Program Files\Keitora\keitora-service.exe",
        r"C:/Program Files/Keitora/keitora-service.exe",
        r"c:\PROGRAM FILES\keitora\Keitora-Service.exe",
    ] {
        let protected = ProtectedProcesses::new(WINDOWS, service);
        assert!(
            protected.is_protected(SERVICE),
            "service given as {service:?} must protect {SERVICE}"
        );
    }

    // A service installed somewhere else is protected there, and only there.
    let custom = ProtectedProcesses::new(WINDOWS, r"D:\Tools\Keitora\keitora-service.exe");
    assert!(custom.is_protected(r"D:\Tools\Keitora\keitora-service.exe"));
    assert!(!custom.is_protected(SERVICE));
}

#[test]
fn protection_is_a_list_not_a_folder() {
    // Other programs in the Windows directory can be blocked, by name or
    // path (T-5.6 even plans to block Task Manager).
    for image in [
        r"C:\Windows\System32\notepad.exe",
        r"C:\Windows\notepad.exe",
        r"C:\Windows\System32\mspaint.exe",
    ] {
        assert!(!protected().is_protected(image), "{image}");
        let name = image
            .rsplit('\\')
            .next()
            .expect("the image path has a file name");
        assert_eq!(
            verdict(&[by_name(name)], image),
            AppVerdict::Blocked,
            "{image}"
        );
        assert_eq!(
            verdict(&[by_path(image)], image),
            AppVerdict::Blocked,
            "{image}"
        );
    }
}

// ---------------------------------------------------------------------------
// S1 (#8): protection by full path only
// ---------------------------------------------------------------------------

#[test]
fn executable_with_a_protected_name_in_another_folder_is_not_protected() {
    let protected = protected();

    // Every protected name, copied to a folder the user owns.
    for name in protected_names() {
        let image = format!(r"C:\Users\Ana\Downloads\{name}");
        assert!(!protected.is_protected(&image), "{image}");
        assert_eq!(
            verdict(&[by_name(name)], &image),
            AppVerdict::Blocked,
            "a copy of {name} in Downloads must be blockable by name"
        );
    }

    // Folders a standard user can write to, inside or next to protected
    // ones, and other drives and shares.
    for (image, name) in [
        (r"C:\Windows\Temp\explorer.exe", "explorer.exe"),
        (r"C:\Windows\Tasks\csrss.exe", "csrss.exe"),
        (r"C:\Windows\Tracing\winlogon.exe", "winlogon.exe"),
        (
            r"C:\Windows\System32\spool\drivers\color\svchost.exe",
            "svchost.exe",
        ),
        (r"C:\Windows\System32\Tasks\lsass.exe", "lsass.exe"),
        (r"C:\Windows\explorer\explorer.exe", "explorer.exe"),
        (r"C:\Windows2\explorer.exe", "explorer.exe"),
        (r"C:\WindowsApps\explorer.exe", "explorer.exe"),
        (r"C:\Windows.\explorer.exe", "explorer.exe"),
        (r"C:\Windows..\explorer.exe", "explorer.exe"),
        (r"C:\Windows \explorer.exe", "explorer.exe"),
        (r"C:\Windows\System32 \csrss.exe", "csrss.exe"),
        (r"C:\Windows\System32.\csrss.exe", "csrss.exe"),
        (r"D:\Windows\explorer.exe", "explorer.exe"),
        (r"D:\Windows\System32\csrss.exe", "csrss.exe"),
        (r"\\server\share\Windows\explorer.exe", "explorer.exe"),
        (
            r"\\?\UNC\server\share\Windows\System32\csrss.exe",
            "csrss.exe",
        ),
        (r"C:\Users\Ana\keitora-service.exe", "keitora-service.exe"),
        (
            r"C:\Program Files\Keitora\old\keitora-service.exe",
            "keitora-service.exe",
        ),
        (
            r"D:\Program Files\Keitora\keitora-service.exe",
            "keitora-service.exe",
        ),
    ] {
        assert!(
            !protected.is_protected(image),
            "{image} must not be protected"
        );
        assert_eq!(
            verdict(&[by_name(name)], image),
            AppVerdict::Blocked,
            "{image} must be blockable by the name {name}"
        );
    }

    // An alternate data stream on a writable folder: not protected, and its
    // path text can still be targeted.
    let stream = r"C:\Windows\Temp:explorer.exe";
    assert!(!protected.is_protected(stream));
    assert_eq!(verdict(&[by_path(stream)], stream), AppVerdict::Blocked);
}

#[test]
fn lookalike_paths_never_inherit_protection() {
    // Paths that look like protected ones but are other files to NTFS. Each
    // is blockable by its own exact path.
    let lookalikes = [
        // Capital I with a dot (U+0130) is not I.
        "C:\\W\u{130}NDOWS\\explorer.exe",
        // Full-width letters are not ASCII.
        "C:\\\u{ff37}\u{ff49}\u{ff4e}\u{ff44}\u{ff4f}\u{ff57}\u{ff53}\\explorer.exe",
        // Cyrillic e (U+0435) and dze (U+0455) are not e and s.
        "C:\\Windows\\\u{435}xplorer.exe",
        "C:\\Windows\\System32\\c\u{455}rss.exe",
        // A zero-width space inside a name.
        "C:\\Windows\\explorer\u{200b}.exe",
        // A no-break space instead of a space.
        "C:\\Program\u{a0}Files\\Keitora\\keitora-service.exe",
    ];
    let protected = protected();
    for image in lookalikes {
        assert!(
            !protected.is_protected(image),
            "{image:?} must not be protected"
        );
        assert_eq!(
            verdict(&[by_path(image)], image),
            AppVerdict::Blocked,
            "{image:?} must be blockable by its own path"
        );
    }

    // A service installed in folders with non-ASCII names: letter case is
    // ignored one character at a time, nothing more.
    let strasse = ProtectedProcesses::new(WINDOWS, "D:\\Stra\u{df}e\\keitora-service.exe");
    assert!(strasse.is_protected("D:\\STRA\u{df}E\\KEITORA-SERVICE.EXE"));
    assert!(
        !strasse.is_protected(r"D:\STRASSE\keitora-service.exe"),
        "sharp s is not SS"
    );
    assert!(
        !strasse.is_protected(r"D:\Strasse\keitora-service.exe"),
        "sharp s is not ss"
    );

    let cafe = ProtectedProcesses::new(WINDOWS, "D:\\Caf\u{e9}\\keitora-service.exe");
    assert!(cafe.is_protected("D:\\CAF\u{c9}\\keitora-service.exe"));
    assert!(
        !cafe.is_protected("D:\\Cafe\u{301}\\keitora-service.exe"),
        "a decomposed accent is another name to NTFS"
    );
    assert!(
        !cafe.is_protected(r"D:\Cafe\keitora-service.exe"),
        "accents are not ignored"
    );

    let kelvin = ProtectedProcesses::new(WINDOWS, r"D:\Keitora\keitora-service.exe");
    assert!(kelvin.is_protected(r"d:\KEITORA\keitora-service.exe"));
    assert!(
        !kelvin.is_protected("D:\\\u{212a}eitora\\keitora-service.exe"),
        "the Kelvin sign is not K"
    );
}

// ---------------------------------------------------------------------------
// S2 (#11): text only, never resolved
// ---------------------------------------------------------------------------

#[test]
fn paths_are_compared_as_text_whether_or_not_they_exist() {
    // None of these exists on a test machine; matching doesn't look.
    for path in [
        r"Q:\Nowhere\never-installed.exe",
        r"Z:\mapped\share\game.exe",
        r"C:\Keitora acceptance\does not exist\app.exe",
    ] {
        assert_eq!(
            verdict(&[by_path(path)], path),
            AppVerdict::Blocked,
            "{path}"
        );
    }
    let nowhere = ProtectedProcesses::new(r"Q:\NoWindows", r"Q:\Nowhere\keitora-service.exe");
    assert!(nowhere.is_protected(r"Q:\NoWindows\explorer.exe"));
    assert!(nowhere.is_protected(r"Q:\NoWindows\System32\csrss.exe"));
    assert!(nowhere.is_protected(r"Q:\Nowhere\keitora-service.exe"));
}

#[test]
fn short_name_form_of_a_target_path_is_not_resolved() {
    // Accepted limitation: `PROGRA~2` would need the file system to resolve.
    // A name target still catches it.
    let short = r"C:\PROGRA~2\Steam\steam.exe";
    assert_eq!(
        verdict(&[by_path(r"C:\Program Files (x86)\Steam\steam.exe")], short),
        AppVerdict::Allowed
    );
    assert_eq!(verdict(&[by_name("steam.exe")], short), AppVerdict::Blocked);
}

// ---------------------------------------------------------------------------
// C3: accepted limitation (F0 review, L3)
// ---------------------------------------------------------------------------

#[test]
fn copying_a_program_under_another_name_and_path_evades_name_and_path_rules() {
    // Accepted for the MVP: the matcher only sees the image path's text, so
    // a copy with another name in another folder matches no rule. Hash or
    // signer matching is post-MVP (docs/threat-model.md).
    let targets = [by_both("steam.exe", r"C:\Games\Steam\steam.exe")];
    assert_eq!(
        verdict(&targets, r"C:\Users\Ana\Desktop\notes.exe"),
        AppVerdict::Allowed
    );
    // Renamed to a protected name, it is still only Allowed, never Protected
    // (#8): adding that name as a target blocks it.
    let disguised = r"C:\Users\Ana\Desktop\explorer.exe";
    assert_eq!(verdict(&targets, disguised), AppVerdict::Allowed);
    assert!(!protected().is_protected(disguised));
}

// ---------------------------------------------------------------------------
// R1: only Windows app identifiers count
// ---------------------------------------------------------------------------

#[test]
fn site_targets_and_exceptions_never_block_or_unblock_an_app() {
    let image = r"C:\Games\steam.exe";
    for pattern in ["steam.exe", "games", "c", "*.exe"] {
        assert_eq!(
            verdict(&[site(pattern)], image),
            AppVerdict::Allowed,
            "site target {pattern} blocked an app"
        );
    }

    let mut list = blocklist(vec![by_name("steam.exe"), site("reddit.com")]);
    list.exceptions = vec![
        SitePattern::new("steam.exe"),
        SitePattern::new("reddit.com"),
    ];
    let matcher = AppMatcher::new(&list, &protected());
    assert_eq!(
        matcher.verdict(image),
        AppVerdict::Blocked,
        "site exceptions must not unblock an app"
    );
}

#[test]
fn android_and_ios_identifiers_are_ignored_on_windows() {
    let image = r"C:\Games\steam.exe";
    let mobile_only = app(AppIdentifiers {
        android_package: Some("steam.exe".to_owned()),
        ios_token: Some(image.to_owned()),
        ..AppIdentifiers::default()
    });
    assert_eq!(verdict(&[mobile_only], image), AppVerdict::Allowed);

    let everywhere = app(AppIdentifiers {
        windows_exe: Some("steam.exe".to_owned()),
        windows_path: None,
        android_package: Some("com.valvesoftware.android.steam.community".to_owned()),
        ios_token: Some("opaque-device-token".to_owned()),
    });
    assert_eq!(verdict(&[everywhere], image), AppVerdict::Blocked);
}

#[test]
fn app_without_windows_identifiers_blocks_nothing() {
    let targets = [
        app(AppIdentifiers::default()),
        app(AppIdentifiers {
            windows_exe: Some(String::new()),
            windows_path: Some(String::new()),
            ..AppIdentifiers::default()
        }),
    ];
    for image in [
        r"C:\Games\steam.exe",
        r"C:\Games\",
        r"C:\",
        r"\\server\share\",
        r"\\?\C:\",
    ] {
        assert_ne!(
            verdict(&targets, image),
            AppVerdict::Blocked,
            "empty identifiers blocked {image}"
        );
    }
}

#[test]
fn empty_blocklist_blocks_nothing() {
    for image in [
        r"C:\Games\steam.exe",
        r"C:\Users\Ana\Downloads\explorer.exe",
        r"\\server\share\game.exe",
    ] {
        assert_eq!(verdict(&[], image), AppVerdict::Allowed, "{image}");
    }
}

// ---------------------------------------------------------------------------
// R2: hostile input never panics, and fails safe
// ---------------------------------------------------------------------------

#[test]
fn image_path_that_is_not_a_full_path_is_never_blocked() {
    let mut names = vec!["steam.exe"];
    names.extend(protected_names());
    names.push("keitora-service.exe");
    let targets: Vec<Target> = names.iter().map(|name| by_name(name)).collect();
    let matcher = matcher(&targets);

    for image in [
        "",
        " ",
        "steam.exe",
        "explorer.exe",
        "csrss.exe",
        "keitora-service.exe",
        r"Games\steam.exe",
        r".\steam.exe",
        r"..\steam.exe",
        r"C:steam.exe",
        r"C:Games\steam.exe",
        r"\Games\steam.exe",
        r"\steam.exe",
        r"/Games/steam.exe",
        r"\Device\HarddiskVolume3\Games\steam.exe",
        r"\Device\HarddiskVolume3\Windows\explorer.exe",
        r"\Device\HarddiskVolume3\Windows\System32\csrss.exe",
        r"\SystemRoot\System32\smss.exe",
    ] {
        assert_ne!(
            matcher.verdict(image),
            AppVerdict::Blocked,
            "{image:?} is not a full path and must never be blocked"
        );
        assert!(!matcher.is_blocked(image), "{image:?}");
    }
}

#[test]
fn hostile_image_paths_never_make_the_matcher_panic() {
    let targets = [
        by_name("steam.exe"),
        by_path(r"C:\Games\steam.exe"),
        by_both("explorer.exe", r"C:\Windows\explorer.exe"),
    ];
    let matcher = matcher(&targets);
    let protected = protected();

    let mut images: Vec<String> = [
        "",
        "\\",
        "/",
        "\\\\",
        "\\\\?",
        "\\\\?\\",
        "\\??\\",
        "\\\\.\\",
        "\\\\?\\\\",
        "\\\\?\\C:",
        "\\\\?\\C",
        "\\\\?\\UNC",
        "\\\\?\\UNC\\",
        "\\\\?\\UNC\\\\",
        ":",
        "C:",
        "C:\\",
        "C:/",
        "C::\\",
        "1:\\a.exe",
        "\u{e9}:\\a.exe",
        "\\\\server",
        "\\\\server\\",
        "C:\\Games\\steam.exe\u{0}",
        "C:\\Games\\\u{0}steam.exe",
        "C:\\Games\\steam\n.exe",
        "C:\\Games\\\u{202e}exe.maets",
        "C:\\Games\\\u{feff}steam.exe",
        "C:\\Games\\\u{1f600}.exe",
        "\u{130}\u{131}\u{df}\u{212a}:\\\u{301}\u{301}",
        "C:\\\u{10ffff}\\steam.exe",
    ]
    .iter()
    .map(|s| (*s).to_owned())
    .collect();
    images.push("\\".repeat(1 << 20));
    images.push("/".repeat(1 << 20));
    images.push(format!("C:\\{}", "a".repeat(1 << 20)));
    images.push(format!("C:\\{}steam.exe", "x\\".repeat(1 << 18)));
    images.push(format!("\\\\?\\{}", "\\\\?\\".repeat(1 << 16)));
    images.push("\u{301}".repeat(1 << 16));
    images.push("\u{df}".repeat(1 << 16));

    for image in &images {
        let verdict = matcher.verdict(image);
        assert_eq!(
            matcher.is_blocked(image),
            verdict == AppVerdict::Blocked,
            "is_blocked disagrees with verdict for a hostile path"
        );
        let _ = protected.is_protected(image);
    }
}

#[test]
fn hostile_constructor_inputs_never_make_it_panic() {
    let hostile = [
        String::new(),
        "\\".to_owned(),
        "C:".to_owned(),
        "C:\\".to_owned(),
        "\\\\?\\".to_owned(),
        "\\??\\".to_owned(),
        "\\\\server\\share".to_owned(),
        "relative\\dir".to_owned(),
        "C:\\Windows\u{0}".to_owned(),
        "\u{202e}C:\\Windows".to_owned(),
        "\\".repeat(1 << 14),
        format!("C:\\{}", "w".repeat(1 << 14)),
    ];
    for windows_dir in &hostile {
        for service in &hostile {
            let protected = ProtectedProcesses::new(windows_dir, service);
            let matcher = AppMatcher::new(&blocklist(vec![by_name("steam.exe")]), &protected);
            for image in [
                r"C:\Games\steam.exe",
                r"C:\Windows\explorer.exe",
                "",
                windows_dir.as_str(),
                service.as_str(),
            ] {
                let _ = protected.is_protected(image);
                let _ = matcher.verdict(image);
            }
        }
    }
}

#[test]
fn hostile_blocklist_values_never_make_the_matcher_panic() {
    let hostile = [
        String::new(),
        "\u{0}".to_owned(),
        "\\".to_owned(),
        "\\\\?\\".to_owned(),
        "C:".to_owned(),
        "*".to_owned(),
        "\u{202e}exe.maets".to_owned(),
        "\u{130}\u{df}\u{212a}\u{301}".to_owned(),
        "\\".repeat(1 << 18),
        format!("C:\\{}", "a".repeat(1 << 18)),
    ];
    let mut targets = Vec::new();
    for value in &hostile {
        targets.push(by_name(value));
        targets.push(by_path(value));
        targets.push(app(AppIdentifiers {
            windows_exe: Some(value.clone()),
            windows_path: Some(value.clone()),
            android_package: Some(value.clone()),
            ios_token: Some(value.clone()),
        }));
        targets.push(site(value));
    }
    let matcher = matcher(&targets);
    for image in [
        r"C:\Games\steam.exe",
        r"C:\Windows\explorer.exe",
        "",
        "\\",
        "C:",
    ] {
        let _ = matcher.verdict(image);
    }
    assert_eq!(
        matcher.verdict(r"C:\Games\steam.exe"),
        AppVerdict::Allowed,
        "no hostile value names C:\\Games\\steam.exe"
    );
}

#[test]
fn long_image_path_at_the_windows_limit_is_still_matched() {
    // Win32 paths reach 32,767 UTF-16 units with long paths enabled.
    let mut image = String::from(r"C:\");
    while image.len() < 32_000 {
        image.push_str(r"deep\");
    }
    image.push_str("steam.exe");
    assert!(image.len() < 32_767);
    assert_eq!(
        verdict(&[by_name("steam.exe")], &image),
        AppVerdict::Blocked
    );
    assert_eq!(verdict(&[by_path(&image)], &image), AppVerdict::Blocked);
    assert_eq!(
        verdict(&[by_name("steam.exe")], &format!(r"\\?\{image}")),
        AppVerdict::Blocked
    );
}

// ---------------------------------------------------------------------------
// R3: stateless, order-independent, any size, unvalidated, shareable
// ---------------------------------------------------------------------------

#[test]
fn one_matcher_answers_many_image_paths_independently() {
    let matcher = matcher(&[
        by_name("steam.exe"),
        by_path(r"C:\Program Files\Discord\Discord.exe"),
        by_name("explorer.exe"),
    ]);
    let expected = [
        (r"C:\Games\steam.exe", AppVerdict::Blocked),
        (
            r"C:\Program Files\Mozilla Firefox\firefox.exe",
            AppVerdict::Allowed,
        ),
        (r"C:\Windows\explorer.exe", AppVerdict::Protected),
        (r"C:\Program Files\Discord\Discord.exe", AppVerdict::Blocked),
        (r"D:\Discord\Discord.exe", AppVerdict::Allowed),
        (r"C:\Users\Ana\explorer.exe", AppVerdict::Blocked),
        (SERVICE, AppVerdict::Protected),
        ("", AppVerdict::Allowed),
        (r"C:\Windows\System32\csrss.exe", AppVerdict::Protected),
    ];
    // Twice, forwards and backwards: earlier questions must not change later
    // answers.
    for (image, want) in expected.iter().chain(expected.iter().rev()) {
        if image.is_empty() {
            assert_ne!(matcher.verdict(image), AppVerdict::Blocked);
        } else {
            assert_eq!(matcher.verdict(image), *want, "{image}");
        }
    }
}

#[test]
fn order_of_targets_does_not_change_the_verdict() {
    let targets = vec![
        by_name("steam.exe"),
        site("reddit.com"),
        by_path(r"C:\Program Files\Discord\Discord.exe"),
        by_both("explorer.exe", r"C:\Windows\explorer.exe"),
        app(AppIdentifiers::default()),
    ];
    let mut reversed = targets.clone();
    reversed.reverse();
    let forwards = matcher(&targets);
    let backwards = matcher(&reversed);
    for image in [
        r"C:\Games\steam.exe",
        r"C:\Program Files\Discord\Discord.exe",
        r"C:\Windows\explorer.exe",
        r"C:\Users\Ana\explorer.exe",
        r"C:\Program Files\Mozilla Firefox\firefox.exe",
    ] {
        assert_eq!(forwards.verdict(image), backwards.verdict(image), "{image}");
    }
}

#[test]
fn matching_target_is_found_in_a_blocklist_at_the_maximum_size() {
    let mut targets: Vec<Target> = (0..MAX_TARGETS - 1)
        .map(|i| {
            by_both(
                &format!("app{i}.exe"),
                &format!(r"C:\Apps\App{i}\app{i}.exe"),
            )
        })
        .collect();
    targets.push(by_path(r"C:\Games\Last\last.exe"));
    assert_eq!(targets.len(), MAX_TARGETS);
    let matcher = matcher(&targets);
    assert_eq!(
        matcher.verdict(r"C:\Games\Last\last.exe"),
        AppVerdict::Blocked
    );
    assert_eq!(
        matcher.verdict(r"D:\Anywhere\app0.exe"),
        AppVerdict::Blocked
    );
    assert_eq!(
        matcher.verdict(&format!(r"C:\Apps\App{0}\app{0}.exe", MAX_TARGETS - 2)),
        AppVerdict::Blocked
    );
    assert_eq!(
        matcher.verdict(r"C:\Games\Last\first.exe"),
        AppVerdict::Allowed
    );
    assert_eq!(
        matcher.verdict(r"C:\Windows\explorer.exe"),
        AppVerdict::Protected
    );
}

#[test]
fn unvalidated_blocklist_still_builds_a_working_matcher() {
    let mut list = blocklist(Vec::new());
    list.id = "not a valid id!".to_owned();
    list.color = "red".to_owned();
    list.name = String::new();
    for i in 0..MAX_TARGETS + 100 {
        list.targets.push(by_name(&format!("filler{i}.exe")));
    }
    list.targets.push(by_name("steam.exe"));
    assert!(
        list.validate().is_err(),
        "the blocklist is meant to be invalid"
    );

    let matcher = AppMatcher::new(&list, &protected());
    assert_eq!(matcher.verdict(r"C:\Games\steam.exe"), AppVerdict::Blocked);
    assert_eq!(matcher.verdict(r"C:\Games\other.exe"), AppVerdict::Allowed);
}

#[test]
fn matcher_and_protected_list_can_be_shared_between_threads() {
    assert_send_sync::<AppMatcher>();
    assert_send_sync::<ProtectedProcesses>();
    assert_send_sync::<AppVerdict>();
}

#[test]
fn verdict_agrees_with_is_blocked_and_is_protected() {
    let protected = protected();
    let targets = [
        by_name("steam.exe"),
        by_name("explorer.exe"),
        by_path(r"C:\Windows\System32\csrss.exe"),
        by_path(r"C:\Games\Other\game.exe"),
    ];
    let matcher = AppMatcher::new(&blocklist(targets.to_vec()), &protected);
    let mut images = protected_system_paths(WINDOWS);
    images.extend(
        [
            SERVICE,
            r"C:\Games\steam.exe",
            r"C:\Users\Ana\explorer.exe",
            r"C:\Games\Other\game.exe",
            r"C:\Games\Other\other.exe",
            r"\\server\share\steam.exe",
            "steam.exe",
            "",
        ]
        .iter()
        .map(|s| (*s).to_owned()),
    );
    for image in &images {
        let verdict = matcher.verdict(image);
        assert_eq!(
            matcher.is_blocked(image),
            verdict == AppVerdict::Blocked,
            "{image}"
        );
        assert_eq!(
            protected.is_protected(image),
            verdict == AppVerdict::Protected,
            "{image}"
        );
    }
}
