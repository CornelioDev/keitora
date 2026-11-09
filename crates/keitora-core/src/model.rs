// SPDX-License-Identifier: MIT OR Apache-2.0

//! The data model: blocklists, and the websites and applications they block.
//!
//! A [`Blocklist`] holds [`Target`]s, each a website ([`Target::Site`]) or an
//! application ([`Target::App`]), and [`SitePattern`] exceptions: pages that
//! stay allowed even though a site target covers them. An application is a
//! logical entity with one identifier per platform, in [`AppIdentifiers`]
//! (ADR-0005), so the same blocklist can later be used on Android and iOS.
//!
//! In JSON, a blocklist looks like this:
//!
//! ```json
//! {
//!   "id": "3f0c6b1e-8a52-4c1d-9e2b-7d4a1f6c0b9e",
//!   "name": "Deep work",
//!   "color": "#e5484d",
//!   "targets": [
//!     { "kind": "site", "pattern": "youtube.com" },
//!     { "kind": "app", "name": "Steam", "ids": { "windows_exe": "steam.exe" } }
//!   ],
//!   "exceptions": ["youtube.com/feed/subscriptions"],
//!   "block_during_breaks": false
//! }
//! ```
//!
//! # Untrusted input
//!
//! Blocklists come from the desktop app and the browser extension, which the
//! service doesn't trust (`docs/threat-model.md`). Deserializing only checks
//! the shape of the JSON; [`Blocklist::validate`] checks what is in it, and
//! a blocklist is used only after it passes. Validation looks at text alone:
//! it never touches the file system, the network or the clock.
//!
//! ## Limits
//!
//! Every value a client chooses has a maximum, so that a blocklist can't take
//! an unbounded amount of memory or time to handle: [`MAX_TARGETS`],
//! [`MAX_EXCEPTIONS`], [`MAX_ID_LEN`], [`MAX_NAME_LEN`], [`MAX_PATTERN_LEN`]
//! and [`MAX_APP_ID_LEN`]; a color is always 7 bytes (`#rrggbb`).
//!
//! **Lengths count bytes of UTF-8, not characters.** Bytes are what a limit
//! on memory needs, and they are counted in constant time. Text in other
//! scripts takes more bytes per character (up to 3 for Chinese or Japanese,
//! 4 for an emoji), so the maximums leave room for that. A client checking a
//! length before it sends a blocklist has to count UTF-8 bytes as well (in
//! JavaScript, `new TextEncoder().encode(text).length`, not `text.length`).
//!
//! Every text value must also be non-empty and free of control characters,
//! which no name, pattern, path or identifier needs: those of Unicode's
//! category Cc (such as line breaks), the line and paragraph separators
//! (U+2028, U+2029), the byte order mark (U+FEFF), and the bidirectional
//! controls (U+200E, U+200F, U+202A to U+202E, U+2066 to U+2069), which can
//! make text display differently from what it says. Other format characters
//! are allowed: names need the zero-width joiner (U+200D) for emoji such as
//! 👨‍👩‍👧, and the zero-width non-joiner (U+200C) in some scripts. An id is
//! stricter still: ASCII letters, digits and `-` only.
//!
//! ## Site patterns
//!
//! A site pattern must also be one the site rule matcher can use: a host,
//! with an optional `*.` before it and an optional path after it, and nothing
//! else, as [`SitePattern::check`] says. Validation and matching share one
//! parser, so a pattern that passes validation never silently matches
//! nothing.
//!
//! ## Application paths
//!
//! [`AppIdentifiers::windows_path`] is opaque text that is only ever compared
//! with the path of a running process. Nothing opens, reads or resolves it:
//! as the service runs as `SYSTEM`, opening `\\attacker\share\x.exe` would make
//! the machine account sign in to someone else's server. Validation accepts
//! only a fully qualified drive path (`C:\…`), checked as text, and so rejects
//! UNC paths (`\\server\share\…`), device paths (`\\?\…`, `\\.\…`), NT paths
//! (`\??\…`) and relative paths (`Steam\steam.exe`, `C:steam.exe`, `\Games\…`).
//! It doesn't use `std::path`, whose answers depend on the platform the code
//! runs on. The text is kept exactly as given: no change of case, separators,
//! `.` or `..`.

use std::fmt;

use serde::{Deserialize, Serialize};

#[cfg(test)]
mod tests;

/// Most targets, websites and applications together, in one blocklist.
///
/// A personal blocklist rarely has more than a few dozen; 500 leaves room for
/// a pasted list of sites while keeping the work of matching every page load
/// and process against it small.
pub const MAX_TARGETS: usize = 500;

/// Most exceptions in one blocklist.
///
/// Exceptions are single pages or sections allowed inside a blocked site, so
/// they are fewer than targets.
pub const MAX_EXCEPTIONS: usize = 200;

/// Longest blocklist id, in bytes of UTF-8.
///
/// Ids are UUIDs in their usual 36-character form; 64 leaves room for another
/// scheme.
pub const MAX_ID_LEN: usize = 64;

/// Longest blocklist name and application name, in bytes of UTF-8.
///
/// 256 bytes fit 85 characters in any script, far more than a name in a
/// sidebar or a list can show.
pub const MAX_NAME_LEN: usize = 256;

/// Longest site pattern, in a target or an exception, in bytes of UTF-8.
///
/// A domain name has at most 253 characters; 512 adds room for a path.
pub const MAX_PATTERN_LEN: usize = 512;

/// Longest value of each [`AppIdentifiers`] field, in bytes of UTF-8.
///
/// Windows paths are usually limited to 260 UTF-16 units (`MAX_PATH`), which
/// take at most 780 bytes of UTF-8 in any script; 1024 covers them all, and
/// Android package names and iOS tokens are shorter.
pub const MAX_APP_ID_LEN: usize = 1024;

/// A named list of websites and applications to block during focus.
#[derive(Debug, Clone, PartialEq, Eq, Serialize, Deserialize)]
pub struct Blocklist {
    /// Unique identifier, usually a UUID: ASCII letters, digits and `-`
    /// only, at most [`MAX_ID_LEN`] bytes.
    pub id: String,
    /// Name shown to the user; at most [`MAX_NAME_LEN`] bytes.
    pub name: String,
    /// Color shown next to the name, as `#rrggbb` (hexadecimal, any case).
    pub color: String,
    /// What the list blocks; at most [`MAX_TARGETS`].
    pub targets: Vec<Target>,
    /// Pages that stay allowed even when a site target covers them; at most
    /// [`MAX_EXCEPTIONS`].
    pub exceptions: Vec<SitePattern>,
    /// Whether the list keeps blocking during breaks, not only during focus.
    pub block_during_breaks: bool,
}

/// Something a blocklist blocks: a website or an application.
///
/// In JSON it is an object tagged by `kind`: `{ "kind": "site", "pattern": … }`
/// or `{ "kind": "app", "name": …, "ids": … }`. A target with a field of the
/// other kind (a site with `ids`, an app with a `pattern`) is rejected; other
/// unknown fields are skipped.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Target {
    /// A website, on any platform.
    Site {
        /// Which pages of the site are blocked.
        pattern: SitePattern,
    },
    /// An application, identified on each platform by its own means.
    App {
        /// Name shown to the user; at most [`MAX_NAME_LEN`] bytes.
        name: String,
        /// How the application is recognized on each platform.
        ids: AppIdentifiers,
    },
}

/// How one application is recognized on each platform (ADR-0005).
///
/// Each field is optional, as an application may exist on some platforms
/// only, but at least one must be set. A field that is not set is left out of
/// the JSON. Each value is at most [`MAX_APP_ID_LEN`] bytes.
#[derive(Debug, Clone, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct AppIdentifiers {
    /// Windows executable file name, such as `steam.exe`. A name, not a path:
    /// it can't contain `\`, `/`, `:` or another character Windows forbids
    /// in file names.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_exe: Option<String>,
    /// Full Windows path of the executable, such as
    /// `C:\Program Files (x86)\Steam\steam.exe`. Opaque text that is only
    /// compared, never opened or resolved; it must start with a drive letter,
    /// a colon and a backslash (see the module documentation).
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub windows_path: Option<String>,
    /// Android package name, such as `com.whatsapp`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub android_package: Option<String>,
    /// iOS application token: opaque, valid on one device only, and never
    /// synced to another.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub ios_token: Option<String>,
}

impl AppIdentifiers {
    /// Returns `true` if no identifier is set, for any platform.
    #[must_use]
    pub fn is_empty(&self) -> bool {
        self.windows_exe.is_none()
            && self.windows_path.is_none()
            && self.android_package.is_none()
            && self.ios_token.is_none()
    }
}

/// A website pattern, such as `youtube.com`, `*.reddit.com` or
/// `youtube.com/shorts`.
///
/// In JSON it is a plain string. This type only holds the text; what a
/// pattern matches is decided by the site rule matcher, in
/// [`crate::site`], and [`SitePattern::check`] tells if it can use it.
#[derive(Debug, Clone, PartialEq, Eq, Hash, Serialize, Deserialize)]
#[serde(transparent)]
pub struct SitePattern(String);

impl SitePattern {
    /// Wraps a pattern's text. [`Blocklist::validate`] checks it.
    #[must_use]
    pub fn new(pattern: impl Into<String>) -> Self {
        Self(pattern.into())
    }

    /// Returns the pattern's text, exactly as given.
    #[must_use]
    pub fn as_str(&self) -> &str {
        &self.0
    }

    /// Checks the pattern exactly as [`Blocklist::validate`] checks each
    /// site target and exception: that its text is valid (see the module
    /// documentation) and that the site rule matcher can use it.
    ///
    /// # Errors
    ///
    /// In this order: [`ValidationErrorKind::Empty`] for an empty pattern,
    /// [`ValidationErrorKind::TooLong`] above [`MAX_PATTERN_LEN`],
    /// [`ValidationErrorKind::ControlCharacter`] for a control character,
    /// and [`ValidationErrorKind::NotASitePattern`] for a pattern the matcher
    /// can't use, such as one with a scheme, a port or no host (the
    /// [`crate::site`] module documentation lists them all).
    pub fn check(&self) -> Result<(), ValidationErrorKind> {
        check_text(&self.0, MAX_PATTERN_LEN)?;
        if crate::site::is_usable(self) {
            Ok(())
        } else {
            Err(ValidationErrorKind::NotASitePattern)
        }
    }
}

// `Target` goes through a flat struct in JSON instead of serde's
// `#[serde(tag = "kind")]`. For an internally tagged enum, serde first copies
// the whole object, unknown fields included, into a buffer and only then picks
// the variant, so a client could make the service hold many times the size of
// a message in memory. A plain struct skips unknown fields without keeping
// them.

/// The `kind` of a target in JSON.
#[derive(Debug, Clone, Copy, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
enum TargetKind {
    Site,
    App,
}

/// Every field a target can have in JSON, as read.
#[derive(Deserialize)]
struct RawTarget {
    kind: TargetKind,
    pattern: Option<SitePattern>,
    name: Option<String>,
    ids: Option<AppIdentifiers>,
}

/// A target as written to JSON; fields of the other kind are left out.
#[derive(Serialize)]
struct TargetRef<'a> {
    kind: TargetKind,
    #[serde(skip_serializing_if = "Option::is_none")]
    pattern: Option<&'a SitePattern>,
    #[serde(skip_serializing_if = "Option::is_none")]
    name: Option<&'a str>,
    #[serde(skip_serializing_if = "Option::is_none")]
    ids: Option<&'a AppIdentifiers>,
}

impl Serialize for Target {
    fn serialize<S: serde::Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        let flat = match self {
            Self::Site { pattern } => TargetRef {
                kind: TargetKind::Site,
                pattern: Some(pattern),
                name: None,
                ids: None,
            },
            Self::App { name, ids } => TargetRef {
                kind: TargetKind::App,
                pattern: None,
                name: Some(name),
                ids: Some(ids),
            },
        };
        flat.serialize(serializer)
    }
}

impl<'de> Deserialize<'de> for Target {
    fn deserialize<D: serde::Deserializer<'de>>(deserializer: D) -> Result<Self, D::Error> {
        use serde::de::Error;

        let raw = RawTarget::deserialize(deserializer)?;
        match raw.kind {
            TargetKind::Site => {
                if raw.name.is_some() || raw.ids.is_some() {
                    return Err(D::Error::custom(
                        "a site target has a `pattern`, not a `name` or `ids`",
                    ));
                }
                let pattern = raw
                    .pattern
                    .ok_or_else(|| D::Error::missing_field("pattern"))?;
                Ok(Self::Site { pattern })
            }
            TargetKind::App => {
                if raw.pattern.is_some() {
                    return Err(D::Error::custom(
                        "an app target has a `name` and `ids`, not a `pattern`",
                    ));
                }
                let name = raw.name.ok_or_else(|| D::Error::missing_field("name"))?;
                let ids = raw.ids.ok_or_else(|| D::Error::missing_field("ids"))?;
                Ok(Self::App { name, ids })
            }
        }
    }
}

impl Blocklist {
    /// Checks that every value in the blocklist is within the limits and in
    /// the expected form (see the module documentation).
    ///
    /// # Errors
    ///
    /// Returns the first problem found, with the JSON path of the value that
    /// has it, such as `targets[2].ids.windows_path`.
    pub fn validate(&self) -> Result<(), ValidationError> {
        check_text(&self.id, MAX_ID_LEN)
            .and_then(|()| check_id(&self.id))
            .map_err(|kind| ValidationError::new("id", kind))?;
        check_text(&self.name, MAX_NAME_LEN).map_err(|kind| ValidationError::new("name", kind))?;
        if !is_hex_color(&self.color) {
            return Err(ValidationError::new(
                "color",
                ValidationErrorKind::NotAColor,
            ));
        }
        check_count(self.targets.len(), MAX_TARGETS)
            .map_err(|kind| ValidationError::new("targets", kind))?;
        check_count(self.exceptions.len(), MAX_EXCEPTIONS)
            .map_err(|kind| ValidationError::new("exceptions", kind))?;
        for (index, target) in self.targets.iter().enumerate() {
            validate_target(target).map_err(|(field, kind)| {
                ValidationError::new(format!("targets[{index}].{field}"), kind)
            })?;
        }
        for (index, exception) in self.exceptions.iter().enumerate() {
            exception
                .check()
                .map_err(|kind| ValidationError::new(format!("exceptions[{index}]"), kind))?;
        }
        Ok(())
    }
}

/// Why [`Blocklist::validate`] rejected a blocklist.
#[derive(Debug, Clone, PartialEq, Eq, thiserror::Error)]
#[error("{field} {kind}")]
pub struct ValidationError {
    field: String,
    kind: ValidationErrorKind,
}

impl ValidationError {
    fn new(field: impl Into<String>, kind: ValidationErrorKind) -> Self {
        Self {
            field: field.into(),
            kind,
        }
    }

    /// The JSON path of the value with the problem, such as `name`,
    /// `targets[2].pattern` or `exceptions[0]`.
    #[must_use]
    pub fn field(&self) -> &str {
        &self.field
    }

    /// What is wrong with the value.
    #[must_use]
    pub fn kind(&self) -> ValidationErrorKind {
        self.kind
    }
}

/// What is wrong with a value in a [`ValidationError`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub enum ValidationErrorKind {
    /// The text is empty.
    Empty,
    /// The text is longer than its maximum, in bytes of UTF-8.
    TooLong {
        /// Length of the text, in bytes.
        len: usize,
        /// Maximum length, in bytes.
        max: usize,
    },
    /// The list has more items than its maximum.
    TooMany {
        /// Number of items in the list.
        count: usize,
        /// Maximum number of items.
        max: usize,
    },
    /// The text contains a control character, such as a line break or a
    /// bidirectional control (see the module documentation).
    ControlCharacter,
    /// The id contains a character other than an ASCII letter, a digit or
    /// `-`.
    NotAnId,
    /// The color is not in the form `#rrggbb`.
    NotAColor,
    /// The Windows executable name contains a character Windows forbids in
    /// file names, such as a path separator.
    NotAFileName,
    /// The Windows path is not a fully qualified drive path such as
    /// `C:\Games\game.exe`: it is a UNC, device or relative path.
    NotADrivePath,
    /// The application sets no identifier for any platform.
    NoIdentifier,
    /// The site pattern is not one the site rule matcher can use: it has no
    /// host, or a scheme, a port, user info, an empty label, a misplaced `*`,
    /// a query or a fragment (see [`SitePattern::check`]).
    NotASitePattern,
}

impl fmt::Display for ValidationErrorKind {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Empty => f.write_str("is empty"),
            Self::TooLong { len, max } => {
                write!(f, "is {len} bytes long, more than the maximum of {max}")
            }
            Self::TooMany { count, max } => {
                write!(f, "has {count} items, more than the maximum of {max}")
            }
            Self::ControlCharacter => f.write_str("contains a control character"),
            Self::NotAnId => f.write_str("may only contain ASCII letters, digits and -"),
            Self::NotAColor => f.write_str("is not a color in the form #rrggbb"),
            Self::NotAFileName => f.write_str(
                "is not a file name: it contains one of the characters < > : \" / \\ | ? *",
            ),
            Self::NotADrivePath => {
                f.write_str("is not a full path that starts with a drive, such as C:\\")
            }
            Self::NoIdentifier => f.write_str("sets no identifier for any platform"),
            Self::NotASitePattern => f.write_str(
                "is not a site pattern such as example.com, *.example.com or example.com/path",
            ),
        }
    }
}

/// Characters Windows doesn't allow in a file name, apart from control
/// characters, which [`check_text`] already rejects.
const NOT_IN_FILE_NAMES: &[char] = &['<', '>', ':', '"', '/', '\\', '|', '?', '*'];

/// Checks a target; on error, also returns the field's path inside it.
fn validate_target(target: &Target) -> Result<(), (&'static str, ValidationErrorKind)> {
    match target {
        Target::Site { pattern } => pattern.check().map_err(|kind| ("pattern", kind)),
        Target::App { name, ids } => {
            check_text(name, MAX_NAME_LEN).map_err(|kind| ("name", kind))?;
            validate_ids(ids)
        }
    }
}

/// Checks an application's identifiers; on error, also returns the field's
/// path inside the target.
fn validate_ids(ids: &AppIdentifiers) -> Result<(), (&'static str, ValidationErrorKind)> {
    if ids.is_empty() {
        return Err(("ids", ValidationErrorKind::NoIdentifier));
    }
    if let Some(exe) = &ids.windows_exe {
        check_text(exe, MAX_APP_ID_LEN)
            .and_then(|()| check_file_name(exe))
            .map_err(|kind| ("ids.windows_exe", kind))?;
    }
    if let Some(path) = &ids.windows_path {
        check_text(path, MAX_APP_ID_LEN)
            .and_then(|()| check_drive_path(path))
            .map_err(|kind| ("ids.windows_path", kind))?;
    }
    if let Some(package) = &ids.android_package {
        check_text(package, MAX_APP_ID_LEN).map_err(|kind| ("ids.android_package", kind))?;
    }
    if let Some(token) = &ids.ios_token {
        check_text(token, MAX_APP_ID_LEN).map_err(|kind| ("ids.ios_token", kind))?;
    }
    Ok(())
}

/// Checks that text is non-empty, at most `max` bytes long and free of
/// control characters (see [`is_forbidden`]).
fn check_text(text: &str, max: usize) -> Result<(), ValidationErrorKind> {
    if text.is_empty() {
        Err(ValidationErrorKind::Empty)
    } else if text.len() > max {
        Err(ValidationErrorKind::TooLong {
            len: text.len(),
            max,
        })
    } else if text.chars().any(is_forbidden) {
        Err(ValidationErrorKind::ControlCharacter)
    } else {
        Ok(())
    }
}

/// Tells if a character is a control character no text value may hold: one
/// of category Cc, a bidirectional control, a line or paragraph separator, or
/// the byte order mark. The zero-width joiner and non-joiner are allowed.
fn is_forbidden(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200E}'
                | '\u{200F}'
                | '\u{202A}'..='\u{202E}'
                | '\u{2066}'..='\u{2069}'
                | '\u{2028}'
                | '\u{2029}'
                | '\u{FEFF}'
        )
}

/// Checks that an id holds only ASCII letters, digits and `-`, so it is safe
/// wherever it ends up, such as in a file name or a log line.
fn check_id(id: &str) -> Result<(), ValidationErrorKind> {
    if id.bytes().all(|b| b.is_ascii_alphanumeric() || b == b'-') {
        Ok(())
    } else {
        Err(ValidationErrorKind::NotAnId)
    }
}

/// Checks that a list has at most `max` items.
fn check_count(count: usize, max: usize) -> Result<(), ValidationErrorKind> {
    if count > max {
        Err(ValidationErrorKind::TooMany { count, max })
    } else {
        Ok(())
    }
}

/// Checks that a Windows executable name is a file name, not a path.
fn check_file_name(name: &str) -> Result<(), ValidationErrorKind> {
    if name.contains(NOT_IN_FILE_NAMES) {
        Err(ValidationErrorKind::NotAFileName)
    } else {
        Ok(())
    }
}

/// Checks, as text only, that a Windows path starts with a drive letter, a
/// colon and a backslash (`C:\`). That one form is fully qualified and local;
/// every other start is a UNC, device, NT or relative path.
fn check_drive_path(path: &str) -> Result<(), ValidationErrorKind> {
    let mut bytes = path.bytes();
    match (bytes.next(), bytes.next(), bytes.next()) {
        (Some(drive), Some(b':'), Some(b'\\')) if drive.is_ascii_alphabetic() => Ok(()),
        _ => Err(ValidationErrorKind::NotADrivePath),
    }
}

/// Tells if text is a color in the form `#rrggbb`, in hexadecimal of any case.
fn is_hex_color(color: &str) -> bool {
    let mut chars = color.chars();
    color.len() == 7 && chars.next() == Some('#') && chars.all(|c| c.is_ascii_hexdigit())
}
