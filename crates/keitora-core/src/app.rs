// SPDX-License-Identifier: MIT OR Apache-2.0

//! The app rule matcher: whether a blocklist blocks a running program, and
//! which programs are never blocked.
//!
//! An [`AppMatcher`] is built once from a [`Blocklist`] and a
//! [`ProtectedProcesses`] list, and then answers any number of image paths
//! with [`AppMatcher::verdict`]: [`AppVerdict::Protected`] when the path is
//! protected, whatever the blocklist says; otherwise [`AppVerdict::Blocked`]
//! when an app target matches it; otherwise [`AppVerdict::Allowed`].
//!
//! An image path is the Win32 path of a running process's executable, as
//! `QueryFullProcessImageNameW` returns it, such as
//! `C:\Program Files (x86)\Steam\steam.exe`. Only the Windows identifiers of
//! an app target count ([`AppIdentifiers::windows_exe`] and
//! [`AppIdentifiers::windows_path`]); site targets, exceptions and the
//! identifiers of other platforms are ignored. Whether a blocklist applies at
//! all, during focus or a break, is the engine's job, not the matcher's.
//!
//! # How paths are compared
//!
//! Matching works on text alone: it never opens, resolves or looks up a path,
//! so the answer is the same whether or not the file exists. A path is put
//! in a normal form, and two paths match when their normal forms are equal:
//!
//! 1. **Full paths only.** A path is either a drive path (`C:\…`) or a UNC
//!    path (`\\server\share\…`). A leading `\\?\`, `\??\` or `\\.\` before a
//!    drive letter is dropped, and `\\?\UNC\server\share\…` is read as
//!    `\\server\share\…`. Anything else (empty, bare, relative, drive-relative
//!    such as `C:steam.exe`, rooted such as `\Games\steam.exe`, or an NT path
//!    such as `\Device\HarddiskVolume3\…`) is not a full path: it is never
//!    blocked, as the matcher can't tell whether it is a protected program.
//! 2. **Separators.** `/` and `\` are the same separator, and after the root
//!    (`C:\`, or the leading `\\` of a UNC path) a run of separators counts as
//!    one, as in Win32: `C:\Windows\\explorer.exe` is
//!    `C:\Windows\explorer.exe`.
//! 3. **Letter case.** Each character is upper-cased on its own, as NTFS does
//!    with its upcase table: `é` equals `É`, `ж` equals `Ж`. A character whose
//!    upper case is more than one character (`ß`, whose upper case is `SS`)
//!    stays as it is, and so does a non-ASCII character whose upper case is
//!    ASCII (the dotless `ı`, U+0131, and the long `ſ`, U+017F), so that no
//!    lookalike of an ASCII path, such as `C:\Wındows`, equals it. Nothing
//!    else is folded: the Kelvin sign (U+212A) is not `K`, `İ` (U+0130) is
//!    not `I`, full-width letters are not ASCII, and composed and decomposed
//!    accents differ.
//! 4. **Nothing else changes.** No trimming, no removal of trailing dots or
//!    spaces (`C:\Windows.\` is another folder), no `.` or `..` resolution,
//!    and no wildcards.
//!
//! An app target blocks a process when either of its identifiers matches:
//!
//! - **Name** ([`AppIdentifiers::windows_exe`]): equals the image path's file
//!   name, the text after its last separator, ignoring letter case. No
//!   extension is added: `steam` doesn't block `steam.exe`. An empty name
//!   blocks nothing.
//! - **Path** ([`AppIdentifiers::windows_path`]): equals the whole image path,
//!   in normal form. Never a prefix (`C:\` blocks nothing real) and never a
//!   suffix. A path target that isn't a full path blocks nothing.
//!
//! # Protected processes
//!
//! A [`ProtectedProcesses`] list holds full paths that are never blocked:
//! `explorer.exe` in the Windows directory, the processes in its `System32`
//! folder that end the session, the shell or input when killed (listed in
//! [`ProtectedProcesses::new`]), and Keitora's own executables. Protection is
//! by full path only, so a program renamed to a protected name in another
//! folder, or a copy of a protected program, is not protected and can be
//! blocked (#8 in `docs/threat-model.md`).
//!
//! # Accepted limitations
//!
//! - Copying a program under another name and into another folder evades
//!   both name and path targets, as the matcher only sees the path's text
//!   (F0 security review, L3). Matching by hash or signer is post-MVP
//!   (`docs/threat-model.md`).
//! - 8.3 short names (`C:\PROGRA~2\…`) aren't resolved, as that would need
//!   the file system: a program launched through one is only caught by a
//!   name target.
//! - Letter case follows Unicode's case mapping as of the Rust release that
//!   builds Keitora. NTFS writes its upcase table when a volume is formatted,
//!   so a letter that gained a case pair in a later Unicode version, or whose
//!   only upper case is several characters, may compare differently there.
//!   Such letters are rare in paths, and none is ASCII.
//!
//! # Untrusted input
//!
//! Blocklists come from clients the service doesn't trust
//! (`docs/threat-model.md`). The matcher doesn't need a blocklist to have
//! passed [`Blocklist::validate`], and no input makes it panic. Its work
//! grows linearly with the length of the image path, and only
//! logarithmically with the number of targets. Its verdict is only a
//! candidate: the service decides which processes it may end (#10).
//!
//! [`AppIdentifiers::windows_exe`]: crate::model::AppIdentifiers::windows_exe
//! [`AppIdentifiers::windows_path`]: crate::model::AppIdentifiers::windows_path

use std::collections::BTreeSet;

use crate::model::{Blocklist, Target};

#[cfg(test)]
mod tests;

/// Programs in the Windows directory that are never blocked.
const IN_WINDOWS_DIR: &[&str] = &["explorer.exe"];

/// Programs in the Windows directory's `System32` folder that are never
/// blocked: killing one ends the session, the shell or input.
const IN_SYSTEM32: &[&str] = &[
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

/// What the matcher decides about a running program.
#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum AppVerdict {
    /// No app target matches the program, or its path isn't a full path.
    Allowed,
    /// An app target matches the program, and it isn't protected.
    Blocked,
    /// The program is protected and is never blocked, whatever the
    /// blocklist says.
    Protected,
}

/// The full paths of the programs that are never blocked.
///
/// Built once by the service with [`ProtectedProcesses::new`]; see the
/// [module documentation](self) for how paths are compared.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ProtectedProcesses {
    /// Each protected path, in normal form.
    paths: BTreeSet<String>,
}

impl ProtectedProcesses {
    /// Builds the list of protected programs.
    ///
    /// `windows_dir` is the Windows directory, such as `C:\Windows`, as
    /// `GetSystemWindowsDirectoryW` reports it. Protected in it are
    /// `explorer.exe` and, in its `System32` folder, `csrss.exe`,
    /// `winlogon.exe`, `wininit.exe`, `smss.exe`, `services.exe`,
    /// `lsass.exe`, `svchost.exe`, `dwm.exe`, `sihost.exe`, `ctfmon.exe`,
    /// `fontdrvhost.exe`, `LogonUI.exe`, `userinit.exe`, `taskhostw.exe`,
    /// `conhost.exe` and `RuntimeBroker.exe`. Other programs in those
    /// folders can be blocked.
    ///
    /// `keitora_exes` are the full paths of Keitora's own executables: the
    /// service, the desktop app and the native messaging host. Only the paths
    /// given are protected, not copies of them elsewhere.
    ///
    /// It never fails: a `windows_dir` that isn't a full path protects no
    /// system program, and an entry of `keitora_exes` that isn't one
    /// protects nothing.
    #[must_use]
    pub fn new(windows_dir: &str, keitora_exes: &[&str]) -> Self {
        let mut paths = BTreeSet::new();
        if let Some(dir) = FullPath::parse(windows_dir) {
            let in_windows_dir = IN_WINDOWS_DIR
                .iter()
                .map(|name| format!(r"{}\{name}", dir.text));
            let in_system32 = IN_SYSTEM32
                .iter()
                .map(|name| format!(r"{}\System32\{name}", dir.text));
            for path in in_windows_dir.chain(in_system32) {
                if let Some(path) = FullPath::parse(&path) {
                    paths.insert(path.text);
                }
            }
        }
        for exe in keitora_exes {
            if let Some(path) = FullPath::parse(exe) {
                paths.insert(path.text);
            }
        }
        Self { paths }
    }

    /// Returns `true` if `image_path` is one of the protected programs.
    ///
    /// A path that isn't a full path is never protected (nor blocked).
    #[must_use]
    pub fn is_protected(&self, image_path: &str) -> bool {
        FullPath::parse(image_path).is_some_and(|path| self.contains(&path))
    }

    /// Whether `path` is one of the protected programs.
    fn contains(&self, path: &FullPath) -> bool {
        self.paths.contains(&path.text)
    }
}

/// Decides which running programs a blocklist blocks.
///
/// Built from a blocklist's app targets and a protected list with
/// [`AppMatcher::new`]; neither is kept, so a matcher must be rebuilt when
/// its blocklist changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct AppMatcher {
    /// The programs that are never blocked.
    protected: ProtectedProcesses,
    /// The file names of the name targets, upper-cased by [`fold`]; never
    /// empty.
    names: BTreeSet<String>,
    /// The full paths of the path targets, in normal form.
    paths: BTreeSet<String>,
}

impl AppMatcher {
    /// Builds a matcher from the app targets of `blocklist` and the
    /// `protected` programs.
    ///
    /// It never fails, and doesn't need the blocklist to have passed
    /// [`Blocklist::validate`]: an empty name, or a path that isn't a full
    /// path, blocks nothing.
    #[must_use]
    pub fn new(blocklist: &Blocklist, protected: &ProtectedProcesses) -> Self {
        let mut names = BTreeSet::new();
        let mut paths = BTreeSet::new();
        for target in &blocklist.targets {
            let Target::App { ids, .. } = target else {
                continue;
            };
            if let Some(name) = &ids.windows_exe
                && !name.is_empty()
            {
                names.insert(name.chars().map(fold).collect());
            }
            if let Some(path) = ids.windows_path.as_deref().and_then(FullPath::parse) {
                paths.insert(path.text);
            }
        }
        Self {
            protected: protected.clone(),
            names,
            paths,
        }
    }

    /// Decides about the program at `image_path`: protected, blocked or
    /// allowed (see the [module documentation](self)).
    ///
    /// A path that isn't a full path is [`AppVerdict::Allowed`]. The answer
    /// depends only on `image_path`, the blocklist and the protected list,
    /// not on earlier calls.
    #[must_use]
    pub fn verdict(&self, image_path: &str) -> AppVerdict {
        let Some(path) = FullPath::parse(image_path) else {
            return AppVerdict::Allowed;
        };
        if self.protected.contains(&path) {
            AppVerdict::Protected
        } else if self.paths.contains(&path.text) || self.names.contains(path.file_name()) {
            AppVerdict::Blocked
        } else {
            AppVerdict::Allowed
        }
    }

    /// Returns `true` if the program at `image_path` is blocked: the same as
    /// `self.verdict(image_path) == AppVerdict::Blocked`.
    #[must_use]
    pub fn is_blocked(&self, image_path: &str) -> bool {
        self.verdict(image_path) == AppVerdict::Blocked
    }
}

/// A full path in normal form (see the module documentation).
#[derive(Debug, Clone, PartialEq, Eq)]
struct FullPath {
    /// The path with no device prefix, every separator written `\`, runs of
    /// separators after the root collapsed, and each character upper-cased
    /// by [`fold`]: `C:\GAMES\STEAM.EXE` or `\\SERVER\SHARE\STEAM.EXE`.
    text: String,
    /// Length in bytes of the root at the start of `text`: `C:\` for a drive
    /// path, `\\SERVER\SHARE` for a UNC path.
    root_len: usize,
}

impl FullPath {
    /// Puts `path` in normal form, or returns `None` if it isn't a full path.
    fn parse(path: &str) -> Option<Self> {
        if let Some(body) = strip_prefix(path, r"\\?\UNC\") {
            return Self::unc(body);
        }
        for prefix in [r"\\?\", r"\??\", r"\\.\"] {
            if let Some(rest) = strip_prefix(path, prefix) {
                return Self::drive(rest);
            }
        }
        match strip_prefix(path, r"\\") {
            Some(body) => Self::unc(body),
            None => Self::drive(path),
        }
    }

    /// Reads a drive path: an ASCII letter, a colon, a separator, and the
    /// rest of the path.
    fn drive(path: &str) -> Option<Self> {
        let mut chars = path.chars();
        let letter = chars.next().filter(char::is_ascii_alphabetic)?;
        if chars.next() != Some(':') || !chars.next().is_some_and(is_separator) {
            return None;
        }
        let mut text = String::with_capacity(path.len());
        text.push(letter.to_ascii_uppercase());
        text.push_str(r":\");
        let root_len = text.len();
        push_normalized(&mut text, chars.as_str());
        Some(Self { text, root_len })
    }

    /// Reads what follows the leading `\\` of a UNC path: a server name, a
    /// separator, a share name, and the rest of the path. Both names must be
    /// there.
    fn unc(body: &str) -> Option<Self> {
        if body.starts_with(is_separator) {
            return None;
        }
        let mut normalized = String::with_capacity(body.len());
        push_normalized(&mut normalized, body);
        let mut parts = normalized.splitn(3, '\\');
        let server = parts.next().filter(|server| !server.is_empty())?;
        let share = parts.next().filter(|share| !share.is_empty())?;
        let mut text = format!(r"\\{server}\{share}");
        let root_len = text.len();
        if let Some(rest) = parts.next() {
            text.push('\\');
            text.push_str(rest);
        }
        Some(Self { text, root_len })
    }

    /// The text after the last separator, which is empty when the path ends
    /// in one or is only a root.
    fn file_name(&self) -> &str {
        self.text
            .get(self.root_len..)
            .and_then(|rest| rest.rsplit('\\').next())
            .unwrap_or_default()
    }
}

/// Returns what follows `prefix` at the start of `path`, or `None` if `path`
/// doesn't start with it. A `\` in `prefix` stands for either separator, and
/// letters match in either ASCII case.
fn strip_prefix<'a>(path: &'a str, prefix: &str) -> Option<&'a str> {
    let mut chars = path.chars();
    for expected in prefix.chars() {
        let actual = chars.next()?;
        let same = if expected == '\\' {
            is_separator(actual)
        } else {
            actual.eq_ignore_ascii_case(&expected)
        };
        if !same {
            return None;
        }
    }
    Some(chars.as_str())
}

/// Appends `rest` to `text`, writing each separator as `\`, dropping one
/// that follows another (or the `\` that `text` ends with), and upper-casing
/// every other character with [`fold`].
fn push_normalized(text: &mut String, rest: &str) {
    for c in rest.chars() {
        if is_separator(c) {
            if !text.ends_with('\\') {
                text.push('\\');
            }
        } else {
            text.push(fold(c));
        }
    }
}

/// Whether `c` separates the parts of a path: `\` or `/`.
fn is_separator(c: char) -> bool {
    c == '\\' || c == '/'
}

/// Upper-cases one character for comparison (see the module documentation):
/// its upper case if that is a single character, and not ASCII unless `c`
/// is; otherwise `c` itself.
fn fold(c: char) -> char {
    let mut upper = c.to_uppercase();
    match (upper.next(), upper.next()) {
        (Some(single), None) if c.is_ascii() || !single.is_ascii() => single,
        _ => c,
    }
}
