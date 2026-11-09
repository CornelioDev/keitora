// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-1.4, rework round 1, finding M-1 of the security
//! review: blocklist validation rejects every site pattern the matcher can't
//! use, and the same check is public so that the service and the add-site
//! form (T-3.7) can reuse it.
//!
//! On 2026-11-09 the maintainer decided to fix every finding of the T-1.4
//! security review within T-1.4, and approved extending its acceptance tests
//! and shared fixture for it. These tests are in a file of their own so that
//! the matcher's tests in `acceptance_t1_4.rs` keep compiling and running
//! while this API doesn't exist yet.
//!
//! # Expected public API
//!
//! - `keitora_core::model::SitePattern::check(&self) -> Result<(), ValidationErrorKind>`:
//!   checks one pattern exactly as `Blocklist::validate` checks each target
//!   pattern and each exception. In this order, it returns
//!   `ValidationErrorKind::Empty` for an empty pattern, `TooLong` above
//!   `MAX_PATTERN_LEN`, `ControlCharacter` for a control character, and the
//!   new `ValidationErrorKind::NotASitePattern` for a pattern the matcher
//!   can't use.
//! - `Blocklist::validate` returns a `ValidationError` whose `field()` names
//!   the pattern (`targets[i].pattern` or `exceptions[i]`) and whose `kind()`
//!   is what `check` returns.
//!
//! What the matcher can't use, and so validation rejects, is listed under
//! `pattern_validation.invalid` in the shared fixture
//! (`tests/acceptance/site_matcher.json`): a scheme, a port, user info, an
//! empty label (a leading dot, `..`, more than one trailing dot), a `*` other
//! than a leading `*.`, a query or a fragment, no host at all (empty, blank,
//! `*`, `*.`, `.`, only a path), an IPv6 address without brackets, or a host
//! IDNA rejects. `pattern_validation.valid` lists forms that pass, among them
//! one trailing dot (`reddit.com.`), which matching already accepts, and IP
//! addresses (`127.0.0.1`, `[::1]`). The matcher itself still takes any
//! blocklist; that side is tested in `acceptance_t1_4.rs`.
//!
//! # Criteria covered
//!
//! - **M-1a — valid patterns pass**: `valid_patterns_pass_the_pattern_check`,
//!   `valid_patterns_pass_blocklist_validation_as_targets_and_exceptions`,
//!   `patterns_that_t1_1_accepts_still_pass`,
//!   `one_trailing_dot_is_accepted_and_more_are_rejected`.
//! - **M-1b — unusable patterns are rejected, naming the field**:
//!   `unusable_patterns_fail_the_pattern_check`,
//!   `unusable_site_target_is_rejected_naming_its_field`,
//!   `unusable_exception_is_rejected_naming_its_field`,
//!   `validation_error_for_an_unusable_pattern_names_its_field_in_the_message`.
//! - **M-1c — one public check, the same as validation**:
//!   `pattern_check_reports_empty_length_and_control_characters_as_validation_does`,
//!   `pattern_check_agrees_with_blocklist_validation`.

#![cfg(test)]

use keitora_core::model::{
    AppIdentifiers, Blocklist, MAX_PATTERN_LEN, SitePattern, Target, ValidationError,
    ValidationErrorKind,
};
use serde::Deserialize;

/// The shared fixture of `acceptance_t1_4.rs`.
const FIXTURE: &str = include_str!("acceptance/site_matcher.json");

/// The part of the fixture these tests read; the rest is skipped.
#[derive(Debug, Deserialize)]
struct Fixture {
    pattern_validation: PatternValidation,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatternValidation {
    valid: Vec<String>,
    invalid: Vec<InvalidPattern>,
}

#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvalidPattern {
    pattern: String,
    reason: String,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn lists() -> PatternValidation {
    serde_json::from_str::<Fixture>(FIXTURE)
        .expect("the shared fixture is valid JSON in the expected shape")
        .pattern_validation
}

fn site(pattern: &str) -> Target {
    Target::Site {
        pattern: SitePattern::new(pattern),
    }
}

fn steam() -> Target {
    Target::App {
        name: "Steam".to_owned(),
        ids: AppIdentifiers {
            windows_exe: Some("steam.exe".to_owned()),
            ..AppIdentifiers::default()
        },
    }
}

fn blocklist(targets: Vec<Target>, exceptions: Vec<SitePattern>) -> Blocklist {
    Blocklist {
        id: "3f0c6b1e-8a52-4c1d-9e2b-7d4a1f6c0b9e".to_owned(),
        name: "Deep work".to_owned(),
        color: "#e5484d".to_owned(),
        targets,
        exceptions,
        block_during_breaks: false,
    }
}

/// Validates a blocklist whose third target is `pattern`.
fn validate_as_target(pattern: &str) -> Result<(), ValidationError> {
    blocklist(
        vec![site("youtube.com"), steam(), site(pattern)],
        vec![SitePattern::new("youtube.com/feed")],
    )
    .validate()
}

/// Validates a blocklist whose second exception is `pattern`.
fn validate_as_exception(pattern: &str) -> Result<(), ValidationError> {
    blocklist(
        vec![site("youtube.com")],
        vec![
            SitePattern::new("youtube.com/feed"),
            SitePattern::new(pattern),
        ],
    )
    .validate()
}

/// The kind validation reports for an invalid pattern of the fixture: an
/// empty one is `Empty`, as before T-1.4; every other is `NotASitePattern`.
fn expected_kind(pattern: &str) -> ValidationErrorKind {
    if pattern.is_empty() {
        ValidationErrorKind::Empty
    } else {
        ValidationErrorKind::NotASitePattern
    }
}

// ---------------------------------------------------------------------------
// M-1a: valid patterns pass
// ---------------------------------------------------------------------------

#[test]
fn valid_patterns_pass_the_pattern_check() {
    for pattern in lists().valid {
        assert_eq!(
            SitePattern::new(pattern.as_str()).check(),
            Ok(()),
            "{pattern:?} should pass the check"
        );
    }
}

#[test]
fn valid_patterns_pass_blocklist_validation_as_targets_and_exceptions() {
    for pattern in lists().valid {
        if let Err(error) = validate_as_target(&pattern) {
            panic!("target {pattern:?} should pass validation, got: {error}");
        }
        if let Err(error) = validate_as_exception(&pattern) {
            panic!("exception {pattern:?} should pass validation, got: {error}");
        }
    }
}

#[test]
fn patterns_that_t1_1_accepts_still_pass() {
    let longest = format!("example.com/{}", "a".repeat(MAX_PATTERN_LEN - 12));
    assert_eq!(longest.len(), MAX_PATTERN_LEN);
    for pattern in [
        "youtube.com",
        "*.reddit.com",
        "youtube.com/shorts",
        "youtube.com/feed/subscriptions",
        "old.reddit.com/r/rust",
        "x.com",
        "example.com",
        "site0.example",
        "site499.example",
        "example.com/allowed/0",
        longest.as_str(),
    ] {
        assert_eq!(SitePattern::new(pattern).check(), Ok(()), "{pattern:?}");
        assert!(validate_as_target(pattern).is_ok(), "target {pattern:?}");
        assert!(
            validate_as_exception(pattern).is_ok(),
            "exception {pattern:?}"
        );
    }
}

#[test]
fn one_trailing_dot_is_accepted_and_more_are_rejected() {
    for pattern in [
        "reddit.com.",
        "*.reddit.com.",
        "reddit.com./r/rust",
        "bücher.de.",
    ] {
        assert_eq!(SitePattern::new(pattern).check(), Ok(()), "{pattern:?}");
    }
    for pattern in [
        "reddit.com..",
        "*.reddit.com..",
        "reddit.com.../r/rust",
        "reddit..com",
    ] {
        assert_eq!(
            SitePattern::new(pattern).check(),
            Err(ValidationErrorKind::NotASitePattern),
            "{pattern:?}"
        );
    }
}

// ---------------------------------------------------------------------------
// M-1b: unusable patterns are rejected, naming the field
// ---------------------------------------------------------------------------

#[test]
fn unusable_patterns_fail_the_pattern_check() {
    for invalid in lists().invalid {
        assert_eq!(
            SitePattern::new(invalid.pattern.as_str()).check(),
            Err(expected_kind(&invalid.pattern)),
            "{:?} ({}) should fail the check",
            invalid.pattern,
            invalid.reason
        );
    }
}

#[test]
fn unusable_site_target_is_rejected_naming_its_field() {
    for invalid in lists().invalid {
        let Err(error) = validate_as_target(&invalid.pattern) else {
            panic!(
                "target {:?} ({}) should be rejected",
                invalid.pattern, invalid.reason
            );
        };
        assert_eq!(error.field(), "targets[2].pattern", "{:?}", invalid.pattern);
        assert_eq!(
            error.kind(),
            expected_kind(&invalid.pattern),
            "{:?}",
            invalid.pattern
        );
    }
}

#[test]
fn unusable_exception_is_rejected_naming_its_field() {
    for invalid in lists().invalid {
        let Err(error) = validate_as_exception(&invalid.pattern) else {
            panic!(
                "exception {:?} ({}) should be rejected",
                invalid.pattern, invalid.reason
            );
        };
        assert_eq!(error.field(), "exceptions[1]", "{:?}", invalid.pattern);
        assert_eq!(
            error.kind(),
            expected_kind(&invalid.pattern),
            "{:?}",
            invalid.pattern
        );
    }
}

#[test]
fn validation_error_for_an_unusable_pattern_names_its_field_in_the_message() {
    let error = validate_as_target("https://www.youtube.com/").expect_err("a scheme is rejected");
    let message = error.to_string();
    assert!(
        message.starts_with("targets[2].pattern "),
        "message {message:?} should start with the field"
    );
    let kind = ValidationErrorKind::NotASitePattern.to_string();
    assert!(!kind.trim().is_empty(), "NotASitePattern has a description");
    assert_ne!(kind, ValidationErrorKind::Empty.to_string());
}

// ---------------------------------------------------------------------------
// M-1c: one public check, the same as validation
// ---------------------------------------------------------------------------

#[test]
fn pattern_check_reports_empty_length_and_control_characters_as_validation_does() {
    assert_eq!(
        SitePattern::new("").check(),
        Err(ValidationErrorKind::Empty)
    );

    let too_long = format!("example.com/{}", "a".repeat(MAX_PATTERN_LEN - 11));
    assert_eq!(too_long.len(), MAX_PATTERN_LEN + 1);
    assert_eq!(
        SitePattern::new(too_long.as_str()).check(),
        Err(ValidationErrorKind::TooLong {
            len: MAX_PATTERN_LEN + 1,
            max: MAX_PATTERN_LEN,
        })
    );

    for pattern in ["reddit.com\n", "reddit.com/\t", "\u{202E}reddit.com"] {
        assert_eq!(
            SitePattern::new(pattern).check(),
            Err(ValidationErrorKind::ControlCharacter),
            "{pattern:?}"
        );
    }
}

#[test]
fn pattern_check_agrees_with_blocklist_validation() {
    let lists = lists();
    let mut patterns: Vec<String> = lists.valid;
    patterns.extend(lists.invalid.into_iter().map(|invalid| invalid.pattern));
    patterns.extend(
        [
            "reddit.com\n",
            "\u{202E}reddit.com",
            "reddit.com..",
            "reddit.com.",
        ]
        .map(str::to_owned),
    );
    for pattern in patterns {
        let check = SitePattern::new(pattern.as_str()).check();
        let as_target = validate_as_target(&pattern).map_err(|error| error.kind());
        let as_exception = validate_as_exception(&pattern).map_err(|error| error.kind());
        assert_eq!(as_target, check, "target {pattern:?}");
        assert_eq!(as_exception, check, "exception {pattern:?}");
    }
}
