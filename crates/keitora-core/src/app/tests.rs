// SPDX-License-Identifier: MIT OR Apache-2.0

use super::*;
use crate::model::{AppIdentifiers, SitePattern};

fn parsed(path: &str) -> Option<(String, String)> {
    FullPath::parse(path).map(|path| (path.text.clone(), path.file_name().to_owned()))
}

fn full(text: &str, file_name: &str) -> Option<(String, String)> {
    Some((text.to_owned(), file_name.to_owned()))
}

fn blocklist(targets: Vec<Target>) -> Blocklist {
    Blocklist {
        id: "list".to_owned(),
        name: "List".to_owned(),
        color: "#000000".to_owned(),
        targets,
        exceptions: Vec::new(),
        block_during_breaks: false,
    }
}

fn app(windows_exe: Option<&str>, windows_path: Option<&str>) -> Target {
    Target::App {
        name: "App".to_owned(),
        ids: AppIdentifiers {
            windows_exe: windows_exe.map(str::to_owned),
            windows_path: windows_path.map(str::to_owned),
            ..AppIdentifiers::default()
        },
    }
}

#[test]
fn fold_upper_cases_one_character_at_a_time() {
    assert_eq!(fold('a'), 'A');
    assert_eq!(fold('A'), 'A');
    assert_eq!(fold('1'), '1');
    assert_eq!(fold('\u{e9}'), '\u{c9}');
    assert_eq!(fold('\u{436}'), '\u{416}');
    assert_eq!(fold('\u{3c9}'), '\u{3a9}');
}

#[test]
fn fold_keeps_a_character_whose_upper_case_is_several() {
    // ß is SS, and the ligature ﬀ is FF.
    assert_eq!(fold('\u{df}'), '\u{df}');
    assert_eq!(fold('\u{fb00}'), '\u{fb00}');
}

#[test]
fn fold_never_turns_a_non_ascii_character_into_ascii() {
    // The dotless i and the long s upper-case to I and S.
    assert_eq!(fold('\u{131}'), '\u{131}');
    assert_eq!(fold('\u{17f}'), '\u{17f}');
    // The Kelvin sign and the capital I with a dot are upper case already.
    assert_eq!(fold('\u{212a}'), '\u{212a}');
    assert_eq!(fold('\u{130}'), '\u{130}');
}

#[test]
fn drive_paths_are_normalized() {
    assert_eq!(
        parsed(r"c:\Games\steam.exe"),
        full(r"C:\GAMES\STEAM.EXE", "STEAM.EXE")
    );
    assert_eq!(
        parsed(r"C://Games\/\steam.exe"),
        full(r"C:\GAMES\STEAM.EXE", "STEAM.EXE")
    );
    assert_eq!(parsed(r"C:\"), full(r"C:\", ""));
    assert_eq!(parsed(r"C:\\\"), full(r"C:\", ""));
    assert_eq!(parsed(r"C:\Games\"), full(r"C:\GAMES\", ""));
}

#[test]
fn device_prefixes_before_a_drive_are_dropped() {
    for path in [
        r"\\?\C:\Games\steam.exe",
        r"\??\C:\Games\steam.exe",
        r"\\.\C:\Games\steam.exe",
        r"//?/C:/Games/steam.exe",
    ] {
        assert_eq!(
            parsed(path),
            full(r"C:\GAMES\STEAM.EXE", "STEAM.EXE"),
            "{path}"
        );
    }
}

#[test]
fn unc_paths_keep_their_server_and_share() {
    for path in [
        r"\\server\share\Games\steam.exe",
        r"\\?\UNC\server\share\Games\steam.exe",
        r"\\?\unc\server\share\Games\steam.exe",
        r"//server//share/Games\\steam.exe",
    ] {
        assert_eq!(
            parsed(path),
            full(r"\\SERVER\SHARE\GAMES\STEAM.EXE", "STEAM.EXE"),
            "{path}"
        );
    }
    // The root of a share has no file name.
    assert_eq!(parsed(r"\\server\share"), full(r"\\SERVER\SHARE", ""));
    assert_eq!(parsed(r"\\server\share\"), full(r"\\SERVER\SHARE\", ""));
}

#[test]
fn paths_that_are_not_full_are_rejected() {
    for path in [
        "",
        " ",
        "steam.exe",
        r"Games\steam.exe",
        r"C:",
        r"C:steam.exe",
        r"C::\steam.exe",
        r"1:\steam.exe",
        "\u{e9}:\\steam.exe",
        r"\Games\steam.exe",
        r"/Games/steam.exe",
        r"\Device\HarddiskVolume3\steam.exe",
        r"\\?\",
        r"\\?\C:",
        r"\\?\C:steam.exe",
        r"\\?\Volume{0}\steam.exe",
        r"\\.\pipe\steam",
        r"\??\UNC\server\share\steam.exe",
        r"\\",
        r"\\\server\share\steam.exe",
        r"\\server",
        r"\\server\",
        r"\\server\\",
        r"\\?\UNC\",
        r"\\?\UNC\\server\share\steam.exe",
        r"\\?\UNC",
    ] {
        assert_eq!(parsed(path), None, "{path:?}");
    }
}

#[test]
fn protected_list_holds_the_system_programs_and_the_executables_given() {
    let protected = ProtectedProcesses::new(r"C:\Windows", &[r"D:\Keitora\keitora-service.exe"]);
    assert_eq!(
        protected.paths.len(),
        IN_WINDOWS_DIR.len() + IN_SYSTEM32.len() + 1
    );
    assert!(protected.paths.contains(r"C:\WINDOWS\EXPLORER.EXE"));
    assert!(protected.paths.contains(r"C:\WINDOWS\SYSTEM32\LOGONUI.EXE"));
    assert!(protected.paths.contains(r"D:\KEITORA\KEITORA-SERVICE.EXE"));
}

#[test]
fn windows_directory_that_is_not_a_full_path_protects_no_system_program() {
    for windows_dir in ["", "Windows", "C:", r"\Windows"] {
        let protected = ProtectedProcesses::new(windows_dir, &[r"D:\Keitora\keitora-service.exe"]);
        assert_eq!(protected.paths.len(), 1, "{windows_dir:?}");
        assert!(
            !protected.is_protected(r"C:\explorer.exe"),
            "{windows_dir:?}"
        );
    }
}

#[test]
fn matcher_keeps_only_usable_windows_identifiers() {
    let matcher = AppMatcher::new(
        &blocklist(vec![
            app(Some("Steam.exe"), Some(r"c:/games//steam.exe")),
            app(Some(""), Some("relative.exe")),
            app(None, None),
            Target::Site {
                pattern: SitePattern::new("reddit.com"),
            },
        ]),
        &ProtectedProcesses::new(r"C:\Windows", &[]),
    );
    assert_eq!(matcher.names.iter().collect::<Vec<_>>(), ["STEAM.EXE"]);
    assert_eq!(
        matcher.paths.iter().collect::<Vec<_>>(),
        [r"C:\GAMES\STEAM.EXE"]
    );
}

#[test]
fn verdict_checks_protection_first() {
    let protected = ProtectedProcesses::new(r"C:\Windows", &[]);
    let matcher = AppMatcher::new(
        &blocklist(vec![app(Some("explorer.exe"), None)]),
        &protected,
    );
    assert_eq!(
        matcher.verdict(r"C:\Windows\explorer.exe"),
        AppVerdict::Protected
    );
    assert_eq!(
        matcher.verdict(r"C:\Temp\explorer.exe"),
        AppVerdict::Blocked
    );
    assert!(matcher.is_blocked(r"C:\Temp\explorer.exe"));
    assert_eq!(matcher.verdict("explorer.exe"), AppVerdict::Allowed);
    assert_eq!(matcher.verdict(r"C:\Temp\other.exe"), AppVerdict::Allowed);
}

#[test]
fn image_path_ending_in_a_separator_matches_no_name() {
    let matcher = AppMatcher::new(
        &blocklist(vec![app(Some("Games"), None)]),
        &ProtectedProcesses::new(r"C:\Windows", &[]),
    );
    assert_eq!(matcher.verdict(r"C:\Games\"), AppVerdict::Allowed);
    assert_eq!(matcher.verdict(r"C:\Games"), AppVerdict::Blocked);
}
