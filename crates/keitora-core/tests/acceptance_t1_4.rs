// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-1.4: the site rule matcher, which decides whether a
//! blocklist blocks a URL.
//!
//! # Expected public API
//!
//! Everything lives in `keitora_core::site`, compiled in every build (no
//! `cfg(test)` or feature gate):
//!
//! - `SiteMatcher`, built from a blocklist with
//!   `SiteMatcher::new(blocklist: &Blocklist) -> SiteMatcher`. It never fails:
//!   a pattern it can't use matches nothing. It must not require the
//!   blocklist to have passed `Blocklist::validate`.
//! - `SiteMatcher::is_blocked(&self, url: &str) -> bool`: whether the URL is
//!   blocked by the blocklist's site targets and exceptions. It takes `&self`,
//!   so one matcher answers any number of URLs.
//!
//! Whether the blocklist applies at all (focus or break,
//! `block_during_breaks`, an active session) is the engine's job, not the
//! matcher's.
//!
//! # The shared fixture
//!
//! The table of cases is `tests/acceptance/site_matcher.json` in this crate
//! (`crates/keitora-core/tests/acceptance/site_matcher.json` in the
//! repository), the contract the browser extension checks too (T-4.3). Its
//! `semantics` list states the rules in prose; each case gives the targets
//! and exceptions of a blocklist, in the JSON shape of `keitora_core::model`,
//! a URL, and whether it is blocked. It sits under `tests/acceptance*`, so
//! only `test(` commits may change it. It is read with `include_str!`, because
//! `crates/keitora-core/clippy.toml` bans reading files, in tests as well.
//!
//! # Criteria covered
//!
//! The task: "Site rule matcher: domain, subdomain wildcard (`*.reddit.com`),
//! path (`youtube.com/shorts`), exceptions taking precedence; IDN and case
//! normalization. Acceptance: table-driven tests, at least 25 cases, exported
//! as a shared JSON fixture for the extension."
//!
//! Every fixture case runs in `every_shared_fixture_case_gives_the_expected_decision`;
//! the case names below are fixture cases.
//!
//! - **AC1 — domain**: `domain_blocks_*`, `subdomain_does_not_block_*`,
//!   `www_subdomain_*`, the lookalikes `domain_does_not_block_*`,
//!   `domain_in_the_path_is_not_the_host`, `domain_in_the_query_is_not_the_host`
//!   and `homograph_with_a_cyrillic_letter_is_another_domain`.
//! - **AC1 — subdomain wildcard**: `wildcard_*`.
//! - **AC1 — path**: `path_*`, `multi_segment_path_*`,
//!   `trailing_slash_in_a_pattern_is_ignored`,
//!   `pattern_with_only_a_slash_blocks_the_whole_site`, `*dot_segments*`,
//!   `percent_encoded_letters_in_the_*_path_are_decoded`.
//! - **AC1 — exceptions taking precedence**: `exception_*`,
//!   `*_exception_*`, `broader_exception_beats_a_narrower_target`; and, over
//!   every fixture case, `exceptions_alone_never_block_anything`,
//!   `removing_exceptions_never_unblocks_a_url`.
//! - **AC1 — IDN and case normalization**: `host_case_is_ignored_*`,
//!   `path_case_is_ignored_*`, `unicode_*`, `punycode_*`, `full_width_*`,
//!   `ideographic_full_stop_in_the_url_host_is_a_dot`,
//!   `percent_encoded_url_host_is_decoded`, `trailing_dot_*`.
//! - **AC2 — table-driven, at least 25 cases, shared JSON fixture**:
//!   `shared_fixture_is_well_formed_with_at_least_25_uniquely_named_cases`,
//!   `every_shared_fixture_case_gives_the_expected_decision`.
//! - **D1 — a domain covers its subdomains** (maintainer, 2026-11-03):
//!   `domain_target_blocks_its_subdomains`, fixture cases
//!   `domain_blocks_its_subdomains`, `domain_blocks_nested_subdomains`;
//!   pinned in the fixture by `shared_fixture_pins_the_maintainer_decisions`.
//! - **D2 — a wildcard also covers the root domain** (maintainer, 2026-11-03):
//!   `wildcard_target_also_blocks_its_root_domain`, fixture case
//!   `wildcard_blocks_the_root_domain`; pinned by
//!   `shared_fixture_pins_the_maintainer_decisions`.
//! - **R1 — review carryover: a blank or hostless exception never exempts
//!   anything** (a whitespace-only pattern passes T-1.1 validation):
//!   `blank_or_hostless_exception_never_exempts_anything`, fixture cases
//!   `*_exception_exempts_nothing`, `whitespace_exceptions_exempt_nothing`,
//!   `wildcard_without_a_domain_exempts_nothing`, `blank_target_blocks_nothing`,
//!   `empty_target_blocks_nothing`.
//! - **R2 — derived: other URL parts can't fool the matcher** (scheme, port,
//!   user info, backslashes, query, fragment): fixture cases `port_is_ignored`,
//!   `http_is_blocked_like_https`, `scheme_case_is_ignored`, `user_info_*`,
//!   `blocked_name_as_user_*`, `backslash*`, `spaces_around_the_url_are_ignored`,
//!   `tabs_and_newlines_inside_the_url_are_ignored`.
//! - **R3 — derived: input that isn't a web URL is never blocked, and no input
//!   makes the matcher panic** (fail safe): fixture cases `*_url_is_not_blocked`,
//!   `browser_page_is_not_blocked`, `extension_page_is_not_blocked`,
//!   `url_with*_is_not_blocked`, `url_without_a_*`; and
//!   `hostile_input_never_makes_the_matcher_panic`.
//! - **R4 — derived: only site targets block**: `app_targets_never_block_sites`,
//!   fixture cases `app_target_does_not_block_a_site`,
//!   `site_target_next_to_an_app_target_still_blocks`,
//!   `empty_blocklist_blocks_nothing`, `any_matching_target_blocks`,
//!   `no_matching_target_blocks_nothing`.
//! - **R5 — derived: the decision depends on the rules, not on their order or
//!   on earlier queries, at any blocklist size**:
//!   `order_of_targets_and_exceptions_does_not_change_the_decision`,
//!   `one_matcher_answers_many_urls_independently`,
//!   `matching_target_is_found_in_a_blocklist_at_the_maximum_size`.
//!
//! # Rework round 1: findings of the security review
//!
//! On 2026-11-09 the maintainer decided to fix every finding of the T-1.4
//! security review within T-1.4, and approved extending these tests and the
//! fixture for it. Pattern validation (M-1) is tested in
//! `acceptance_t1_4_validation.rs`; the matcher's side of it is here.
//!
//! - **M-1 — unusable patterns match nothing, also unvalidated**:
//!   `patterns_validation_rejects_match_nothing_in_an_unvalidated_blocklist`,
//!   `shared_fixture_lists_valid_and_invalid_patterns`, fixture cases
//!   `exception_with_a_*_exempts_nothing`, `exception_with_two_trailing_dots_exempts_nothing`,
//!   `target_with_a_scheme_blocks_nothing`.
//! - **M-2 — a target with a path matches any lenient reading of the URL's
//!   path** (`//`, `%2F`/`%5C`, `;` parameters, combined, then dot segments):
//!   `target_path_matches_any_lenient_reading_of_the_url_path`,
//!   `lenient_readings_never_widen_a_target_path`, fixture cases
//!   `target_path_matches_*`, `lenient_readings_still_match_*`,
//!   `parameter_named_like_the_target_path_does_not_block`,
//!   `encoded_slash*_do*_not_block`,
//!   `domain_exception_still_beats_a_lenient_target_match`.
//! - **M-2 — an exception with a path doesn't apply to a URL path with `//`,
//!   `%2F`, `%5C` or `;`; one without a path is unaffected**:
//!   `exception_path_never_applies_to_an_ambiguous_url_path`,
//!   `exception_without_a_path_ignores_path_ambiguity`, fixture cases
//!   `exception_path_*`, `domain_exception_ignores_path_ambiguity`.
//! - **L-1 — every trailing dot of the URL's host is ignored**:
//!   `every_trailing_dot_of_the_url_host_is_ignored`, fixture cases
//!   `*_trailing_dot*_in_the_url_host_*`,
//!   `exception_applies_to_a_url_host_with_trailing_dots`.
//! - **L-2 — an IPv4-mapped IPv6 address is the same host as its IPv4
//!   address; an IP covers only itself**:
//!   `ipv4_mapped_ipv6_address_is_the_same_host_as_ipv4`, fixture cases
//!   `ip_*`, `ipv4_mapped_*`, `hexadecimal_ipv4_mapped_*`, `ipv6_*`.
//! - **R3, extended**: `hostile_paths_and_hosts_never_make_the_matcher_panic`.
//!
//! # Rework round 2: finding of the security re-review
//!
//! On 2026-11-09 the maintainer decided to fix finding N-1 within T-1.4 and
//! approved extending these tests and the fixture for it.
//!
//! - **N-1 — an encoded semicolon (`%3B`, any case) counts like `;`**: an
//!   exception with a path doesn't apply to a URL path that contains one, and
//!   lenient readings remove `%3B…` parameters like `;…` ones:
//!   `encoded_semicolon_counts_like_a_semicolon`, fixture cases
//!   `*encoded_parameter*`, `*encoded_segment_parameters`,
//!   `domain_exception_ignores_encoded_parameters`.

#![cfg(test)]

use std::collections::HashSet;

use keitora_core::model::{
    AppIdentifiers, Blocklist, MAX_EXCEPTIONS, MAX_PATTERN_LEN, MAX_TARGETS, SitePattern, Target,
};
use keitora_core::site::SiteMatcher;
use serde::Deserialize;

/// The shared fixture, also read by the extension's tests (T-4.3).
const FIXTURE: &str = include_str!("acceptance/site_matcher.json");

/// The whole fixture file. Unknown fields are rejected so that a misspelt
/// key can't silently drop part of a case.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Fixture {
    description: String,
    semantics: Vec<String>,
    cases: Vec<Case>,
    pattern_validation: PatternValidation,
}

/// Patterns that blocklist validation accepts and rejects (security review
/// M-1); `acceptance_t1_4_validation.rs` checks validation against them.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct PatternValidation {
    valid: Vec<String>,
    invalid: Vec<InvalidPattern>,
}

/// A pattern that validation rejects, and why.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct InvalidPattern {
    pattern: String,
    reason: String,
}

/// One row of the table.
#[derive(Debug, Clone, Deserialize)]
#[serde(deny_unknown_fields)]
struct Case {
    name: String,
    #[serde(default)]
    note: Option<String>,
    targets: Vec<Target>,
    exceptions: Vec<SitePattern>,
    url: String,
    blocked: bool,
}

// ---------------------------------------------------------------------------
// Helpers
// ---------------------------------------------------------------------------

fn fixture() -> Fixture {
    serde_json::from_str(FIXTURE).expect("the shared fixture is valid JSON in the expected shape")
}

fn cases() -> Vec<Case> {
    fixture().cases
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

fn sites(patterns: &[&str]) -> Vec<Target> {
    patterns
        .iter()
        .map(|pattern| Target::Site {
            pattern: SitePattern::new(*pattern),
        })
        .collect()
}

fn patterns(patterns: &[&str]) -> Vec<SitePattern> {
    patterns
        .iter()
        .map(|pattern| SitePattern::new(*pattern))
        .collect()
}

fn app(name: &str, exe: &str) -> Target {
    Target::App {
        name: name.to_owned(),
        ids: AppIdentifiers {
            windows_exe: Some(exe.to_owned()),
            ..AppIdentifiers::default()
        },
    }
}

/// Builds a matcher from targets and exceptions and asks it about `url`.
fn decide(targets: &[Target], exceptions: &[SitePattern], url: &str) -> bool {
    SiteMatcher::new(&blocklist(targets.to_vec(), exceptions.to_vec())).is_blocked(url)
}

fn decide_case(case: &Case) -> bool {
    decide(&case.targets, &case.exceptions, &case.url)
}

fn find_case<'a>(cases: &'a [Case], name: &str) -> &'a Case {
    cases
        .iter()
        .find(|case| case.name == name)
        .unwrap_or_else(|| panic!("the shared fixture has no case named {name}"))
}

// ---------------------------------------------------------------------------
// AC2: the shared fixture
// ---------------------------------------------------------------------------

#[test]
fn shared_fixture_is_well_formed_with_at_least_25_uniquely_named_cases() {
    let fixture = fixture();
    assert!(
        !fixture.description.trim().is_empty(),
        "the fixture describes itself"
    );
    assert!(
        fixture.semantics.iter().all(|rule| !rule.trim().is_empty())
            && !fixture.semantics.is_empty(),
        "the fixture states its rules in prose"
    );
    assert!(
        fixture.cases.len() >= 25,
        "the task asks for at least 25 cases, the fixture has {}",
        fixture.cases.len()
    );

    let mut names = HashSet::new();
    for case in &fixture.cases {
        assert!(
            !case.name.is_empty()
                && case
                    .name
                    .bytes()
                    .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_'),
            "case name {:?} is not snake_case",
            case.name
        );
        assert!(
            names.insert(case.name.as_str()),
            "case name {} appears twice",
            case.name
        );
        if let Some(note) = &case.note {
            assert!(
                !note.trim().is_empty(),
                "case {} has an empty note",
                case.name
            );
        }
    }

    // Both outcomes are represented, so the table can't pass by always
    // answering the same.
    assert!(fixture.cases.iter().any(|case| case.blocked));
    assert!(fixture.cases.iter().any(|case| !case.blocked));
}

#[test]
fn every_shared_fixture_case_gives_the_expected_decision() {
    let cases = cases();
    let mut failures = Vec::new();
    for case in &cases {
        let got = decide_case(case);
        if got != case.blocked {
            failures.push(format!(
                "{}: url {:?}, targets {:?}, exceptions {:?}: expected blocked = {}, got {}",
                case.name, case.url, case.targets, case.exceptions, case.blocked, got
            ));
        }
    }
    assert!(
        failures.is_empty(),
        "{} of {} fixture cases failed:\n{}",
        failures.len(),
        cases.len(),
        failures.join("\n")
    );
}

#[test]
fn shared_fixture_pins_the_maintainer_decisions() {
    let cases = cases();

    let subdomains = find_case(&cases, "domain_blocks_its_subdomains");
    assert_eq!(subdomains.targets, sites(&["reddit.com"]));
    assert!(subdomains.exceptions.is_empty());
    assert_eq!(subdomains.url, "https://old.reddit.com/");
    assert!(subdomains.blocked);

    let root = find_case(&cases, "wildcard_blocks_the_root_domain");
    assert_eq!(root.targets, sites(&["*.reddit.com"]));
    assert!(root.exceptions.is_empty());
    assert_eq!(root.url, "https://reddit.com/");
    assert!(root.blocked);
}

// ---------------------------------------------------------------------------
// D1, D2: the maintainer's decisions, checked directly
// ---------------------------------------------------------------------------

#[test]
fn domain_target_blocks_its_subdomains() {
    let targets = sites(&["reddit.com"]);
    for url in [
        "https://reddit.com/",
        "https://old.reddit.com/",
        "https://www.reddit.com/r/rust",
        "https://a.b.c.reddit.com/",
    ] {
        assert!(decide(&targets, &[], url), "reddit.com should block {url}");
    }
    for url in ["https://notreddit.com/", "https://reddit.com.evil.com/"] {
        assert!(
            !decide(&targets, &[], url),
            "reddit.com should not block {url}"
        );
    }
}

#[test]
fn wildcard_target_also_blocks_its_root_domain() {
    let targets = sites(&["*.reddit.com"]);
    for url in [
        "https://reddit.com/",
        "https://reddit.com/r/rust",
        "https://old.reddit.com/",
        "https://a.b.c.reddit.com/",
    ] {
        assert!(
            decide(&targets, &[], url),
            "*.reddit.com should block {url}"
        );
    }
    for url in ["https://notreddit.com/", "https://reddit.com.evil.com/"] {
        assert!(
            !decide(&targets, &[], url),
            "*.reddit.com should not block {url}"
        );
    }
}

// ---------------------------------------------------------------------------
// R1: an exception with no domain never exempts anything
// ---------------------------------------------------------------------------

#[test]
fn blank_or_hostless_exception_never_exempts_anything() {
    let hostless = [
        "",
        " ",
        "   ",
        "\t",
        "\t \t",
        "\u{3000}",
        "\u{00A0}",
        "*",
        "*.",
        "* ",
        " *. ",
        ".",
        "..",
        "/",
        " /",
        "//",
        "/r/rust",
        "*./",
        "*./r/rust",
    ];
    let targets = sites(&["reddit.com", "youtube.com/shorts"]);
    let urls = [
        "https://reddit.com/",
        "https://old.reddit.com/r/rust",
        "https://youtube.com/shorts/abc",
    ];
    for url in urls {
        assert!(
            decide(&targets, &[], url),
            "{url} must be blocked without exceptions"
        );
        for exception in hostless {
            assert!(
                decide(&targets, &patterns(&[exception]), url),
                "exception {exception:?} must not exempt {url}"
            );
        }
        // All of them at once, too.
        assert!(
            decide(&targets, &patterns(&hostless), url),
            "hostless exceptions together must not exempt {url}"
        );
    }
}

// ---------------------------------------------------------------------------
// AC1: exceptions take precedence and only ever unblock
// ---------------------------------------------------------------------------

#[test]
fn exceptions_alone_never_block_anything() {
    for case in cases() {
        // Every site pattern of the case, as exceptions, with no targets.
        let mut exceptions = case.exceptions.clone();
        for target in &case.targets {
            if let Target::Site { pattern } = target {
                exceptions.push(pattern.clone());
            }
        }
        assert!(
            !decide(&[], &exceptions, &case.url),
            "{}: exceptions without targets blocked {:?}",
            case.name,
            case.url
        );
    }
}

#[test]
fn removing_exceptions_never_unblocks_a_url() {
    for case in cases() {
        let with = decide(&case.targets, &case.exceptions, &case.url);
        let without = decide(&case.targets, &[], &case.url);
        assert!(
            !with || without,
            "{}: {:?} is blocked with exceptions but not without them",
            case.name,
            case.url
        );
    }
}

// ---------------------------------------------------------------------------
// R4: only site targets block
// ---------------------------------------------------------------------------

#[test]
fn app_targets_never_block_sites() {
    let apps = vec![
        app("Reddit", "reddit.com"),
        app("YouTube", "youtube.com"),
        app("Steam", "steam.exe"),
    ];
    for url in ["https://reddit.com/", "https://youtube.com/shorts/abc"] {
        assert!(!decide(&apps, &[], url), "app targets blocked {url}");
    }

    // Every fixture URL, with the case's site targets turned into apps named
    // after their patterns.
    for case in cases() {
        let as_apps: Vec<Target> = case
            .targets
            .iter()
            .map(|target| match target {
                Target::Site { pattern } => app(pattern.as_str(), "placeholder.exe"),
                Target::App { .. } => target.clone(),
            })
            .collect();
        assert!(
            !decide(&as_apps, &case.exceptions, &case.url),
            "{}: app targets blocked {:?}",
            case.name,
            case.url
        );
    }
}

// ---------------------------------------------------------------------------
// R5: order, reuse and size
// ---------------------------------------------------------------------------

#[test]
fn order_of_targets_and_exceptions_does_not_change_the_decision() {
    for case in cases() {
        let mut targets = case.targets.clone();
        targets.reverse();
        let mut exceptions = case.exceptions.clone();
        exceptions.reverse();
        assert_eq!(
            decide(&targets, &exceptions, &case.url),
            case.blocked,
            "{}: reversing targets and exceptions changed the decision",
            case.name
        );
    }
}

#[test]
fn one_matcher_answers_many_urls_independently() {
    let matcher = SiteMatcher::new(&blocklist(
        sites(&["reddit.com", "youtube.com/shorts"]),
        patterns(&["old.reddit.com/r/rust"]),
    ));
    let expected = [
        ("https://reddit.com/", true),
        ("https://example.org/", false),
        ("https://youtube.com/shorts/abc", true),
        ("https://old.reddit.com/r/rust/comments/1", false),
        ("not a url", false),
        ("https://youtube.com/watch?v=abc", false),
        ("https://www.reddit.com/r/rust", true),
    ];
    // Twice, forwards and backwards: earlier questions must not change later
    // answers.
    for (url, blocked) in expected.iter().chain(expected.iter().rev()) {
        assert_eq!(matcher.is_blocked(url), *blocked, "{url}");
    }
}

#[test]
fn matching_target_is_found_in_a_blocklist_at_the_maximum_size() {
    let mut targets: Vec<Target> = (0..MAX_TARGETS - 1)
        .map(|i| Target::Site {
            pattern: SitePattern::new(format!("site{i}.example/path{i}")),
        })
        .collect();
    targets.push(Target::Site {
        pattern: SitePattern::new("reddit.com"),
    });
    let mut exceptions: Vec<SitePattern> = (0..MAX_EXCEPTIONS - 1)
        .map(|i| SitePattern::new(format!("allowed{i}.example")))
        .collect();
    exceptions.push(SitePattern::new("old.reddit.com"));
    assert_eq!(targets.len(), MAX_TARGETS);
    assert_eq!(exceptions.len(), MAX_EXCEPTIONS);

    let matcher = SiteMatcher::new(&blocklist(targets, exceptions));
    assert!(matcher.is_blocked("https://www.reddit.com/"));
    assert!(!matcher.is_blocked("https://old.reddit.com/"));
    assert!(matcher.is_blocked("https://site7.example/path7/x"));
    assert!(!matcher.is_blocked("https://site7.example/path8"));
    assert!(!matcher.is_blocked("https://allowed3.example/"));
    assert!(!matcher.is_blocked("https://example.org/"));
}

// ---------------------------------------------------------------------------
// R3: fail safe on hostile input
// ---------------------------------------------------------------------------

#[test]
fn hostile_input_never_makes_the_matcher_panic() {
    let long_host = format!("https://{}reddit.com/", "a.".repeat(50_000));
    let long_label = format!("https://{}.reddit.com/", "a".repeat(100_000));
    let long_path = format!("https://reddit.com/{}", "a/".repeat(50_000));
    let long_encoded_path = format!("https://reddit.com/{}", "%73".repeat(50_000));
    let long_query = format!("https://reddit.com/?q={}", "x".repeat(100_000));
    let urls: Vec<&str> = vec![
        "\u{0}",
        "\u{FEFF}https://reddit.com/",
        "https://\u{0}reddit.com/",
        "https://xn--/",
        "https://xn--zz.com/",
        "https://xn--a.xn--b.reddit.com/",
        "https://[::1]/",
        "https://[::1/",
        "http://127.0.0.1/",
        "http://2130706433/",
        "https://\u{1F4A9}.la/",
        "https://%zz/",
        "https://%/",
        "https://a..b/",
        "https://.reddit.com/",
        "https://reddit.com../",
        "https://reddit.com/%",
        "https://reddit.com/%e2%82",
        "https://reddit.com/\u{1F4A9}/%F0%9F%92%A9",
        "https://reddit.com:/",
        "https://@/",
        "https://:@reddit.com/",
        "https:",
        ":",
        "*",
        "https://*.reddit.com/",
        &long_host,
        &long_label,
        &long_path,
        &long_encoded_path,
        &long_query,
    ];

    let at_max = |unit: &str| {
        let count = MAX_PATTERN_LEN / unit.len();
        unit.repeat(count)
    };
    let hostile_patterns = vec![
        at_max("a"),
        at_max("a."),
        at_max("*."),
        at_max("/"),
        at_max("xn--"),
        at_max("%"),
        at_max("\u{1F4A9}"),
        format!("reddit.com/{}", "%".repeat(MAX_PATTERN_LEN - 11)),
        "xn--zz.com".to_owned(),
        "[::1]".to_owned(),
        "reddit.com:99999".to_owned(),
        "https://reddit.com".to_owned(),
        "reddit.com/%".to_owned(),
        "reddit.com/%e2%82".to_owned(),
        "**.reddit.com".to_owned(),
        "*.*.reddit.com".to_owned(),
        "reddit.*".to_owned(),
    ];
    let mut all = hostile_patterns.clone();
    all.push("reddit.com".to_owned());
    let all: Vec<&str> = all.iter().map(String::as_str).collect();

    // As targets and as exceptions, against every URL above. Only reaching
    // the end matters; the answers for these inputs aren't pinned.
    let as_targets = SiteMatcher::new(&blocklist(sites(&all), vec![]));
    let as_exceptions = SiteMatcher::new(&blocklist(sites(&["reddit.com"]), patterns(&all)));
    let mut answers = 0_usize;
    for url in &urls {
        let _ = as_targets.is_blocked(url);
        let _ = as_exceptions.is_blocked(url);
        answers += 2;
    }
    assert_eq!(answers, urls.len() * 2);

    // These aren't URLs a browser would load, so they are never blocked.
    for url in ["\u{0}", "https:", ":", "*", "https://[::1/", "https://@/"] {
        assert!(!as_targets.is_blocked(url), "{url:?} must not be blocked");
    }
}

// ---------------------------------------------------------------------------
// Rework round 1, M-1: patterns validation rejects match nothing
// ---------------------------------------------------------------------------

#[test]
fn shared_fixture_lists_valid_and_invalid_patterns() {
    let lists = fixture().pattern_validation;
    assert!(!lists.valid.is_empty(), "the fixture lists valid patterns");
    assert!(
        !lists.invalid.is_empty(),
        "the fixture lists invalid patterns"
    );
    for invalid in &lists.invalid {
        assert!(
            !invalid.reason.trim().is_empty(),
            "invalid pattern {:?} says why",
            invalid.pattern
        );
        assert!(
            !lists.valid.contains(&invalid.pattern),
            "{:?} is listed as both valid and invalid",
            invalid.pattern
        );
    }
    // The documented forms, and one trailing dot, which matching accepts.
    for pattern in [
        "reddit.com",
        "*.reddit.com",
        "youtube.com/shorts",
        "reddit.com.",
        "bücher.de",
        "127.0.0.1",
        "[::1]",
    ] {
        assert!(
            lists.valid.iter().any(|valid| valid == pattern),
            "{pattern} is not listed as valid"
        );
    }
}

#[test]
fn patterns_validation_rejects_match_nothing_in_an_unvalidated_blocklist() {
    let invalid: Vec<String> = fixture()
        .pattern_validation
        .invalid
        .into_iter()
        .map(|invalid| invalid.pattern)
        .collect();
    let urls = [
        "https://reddit.com/",
        "https://www.reddit.com/r/rust",
        "https://reddit.com:443/",
        "https://youtube.com/",
        "https://www.youtube.com/shorts/abc",
        "https://youtube.com/watch?v=abc",
        "http://[::1]/",
        "http://127.0.0.1/",
    ];
    let broad = sites(&["reddit.com", "youtube.com", "[::1]", "127.0.0.1"]);
    for pattern in &invalid {
        let as_target = SiteMatcher::new(&blocklist(sites(&[pattern.as_str()]), vec![]));
        let as_exception =
            SiteMatcher::new(&blocklist(broad.clone(), patterns(&[pattern.as_str()])));
        for url in urls {
            assert!(
                !as_target.is_blocked(url),
                "invalid target {pattern:?} blocked {url}"
            );
            assert!(
                as_exception.is_blocked(url),
                "invalid exception {pattern:?} exempted {url}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Rework round 1, M-2: lenient readings of the URL's path
// ---------------------------------------------------------------------------

#[test]
fn target_path_matches_any_lenient_reading_of_the_url_path() {
    let targets = sites(&["youtube.com/shorts"]);
    for url in [
        "https://youtube.com//shorts/abc",
        "https://youtube.com/shorts%2Fabc",
        "https://youtube.com/shorts%2fabc",
        "https://youtube.com/shorts%5Cabc",
        "https://youtube.com/shorts%5cabc",
        "https://youtube.com/shorts;x/abc",
        "https://youtube.com/feed%2F..%2Fshorts",
        "https://youtube.com/feed%5C..%5Cshorts",
        "https://youtube.com/feed/..;/shorts/abc",
        "https://youtube.com/feed;a/..;b/shorts;c/abc",
        "https://youtube.com//shorts;x/abc",
        "https://youtube.com/%2Fshorts",
        "https://m.youtube.com///shorts//abc",
    ] {
        assert!(
            decide(&targets, &[], url),
            "youtube.com/shorts should block {url}"
        );
    }
}

#[test]
fn lenient_readings_never_widen_a_target_path() {
    let targets = sites(&["youtube.com/shorts"]);
    for url in [
        "https://youtube.com/shortsfoo%2Fabc",
        "https://youtube.com/shortsfoo;x",
        "https://youtube.com/watch;shorts",
        "https://youtube.com/watch%2Fshorts",
        "https://youtube.com/sh%2Forts",
        "https://youtube.com/shorts%2F..%2Fwatch",
        "https://youtube.com//",
        "https://youtube.com/watch?next=//shorts",
    ] {
        assert!(
            !decide(&targets, &[], url),
            "youtube.com/shorts should not block {url}"
        );
    }
}

#[test]
fn exception_path_never_applies_to_an_ambiguous_url_path() {
    let targets = sites(&["x.com"]);
    let exceptions = patterns(&["x.com/a"]);
    for url in [
        "https://x.com/a",
        "https://x.com/a/",
        "https://x.com/a/b",
        "https://x.com/A/B",
        "https://www.x.com/a/b",
        "https://x.com/a/b?u=%2F%2F;x#//",
    ] {
        assert!(
            !decide(&targets, &exceptions, url),
            "x.com/a should exempt {url}"
        );
    }
    for url in [
        "https://x.com//a",
        "https://x.com/a//b",
        "https://x.com/a///",
        "https://x.com/a/..%2F..%2Fsecret",
        "https://x.com/a/b%2fc",
        "https://x.com/a/b%2Fc",
        "https://x.com/a/b%5cc",
        "https://x.com/a/b%5Cc",
        "https://x.com/a;v=1",
        "https://x.com/a/b;v=1",
        "https://x.com/a/..;/secret",
    ] {
        assert!(
            decide(&targets, &exceptions, url),
            "x.com/a must not exempt {url}"
        );
    }
}

#[test]
fn exception_without_a_path_ignores_path_ambiguity() {
    let targets = sites(&["x.com"]);
    for exception in ["x.com", "*.x.com", "x.com/"] {
        for url in [
            "https://x.com//a",
            "https://x.com/a/..%2F..%2Fsecret",
            "https://x.com/a;b",
            "https://x.com/%5C",
        ] {
            assert!(
                !decide(&targets, &patterns(&[exception]), url),
                "exception {exception} should exempt {url}"
            );
        }
    }
}

// ---------------------------------------------------------------------------
// Rework round 1, L-1: trailing dots of the URL's host
// ---------------------------------------------------------------------------

#[test]
fn every_trailing_dot_of_the_url_host_is_ignored() {
    let targets = sites(&["reddit.com"]);
    for url in [
        "https://reddit.com./",
        "https://reddit.com../",
        "https://reddit.com.../r/rust",
        "https://reddit.com\u{3002}\u{3002}/",
        "https://reddit.com.%2E/",
        "https://old.reddit.com..../",
    ] {
        assert!(decide(&targets, &[], url), "reddit.com should block {url}");
    }
    for url in [
        "https://notreddit.com../",
        "https://reddit.com..evil.com/",
        "https://../",
        "https://./",
        "https://.../",
    ] {
        assert!(
            !decide(&targets, &[], url),
            "reddit.com should not block {url}"
        );
    }
}

// ---------------------------------------------------------------------------
// Rework round 1, L-2: IP addresses
// ---------------------------------------------------------------------------

#[test]
fn ipv4_mapped_ipv6_address_is_the_same_host_as_ipv4() {
    for (target, url, blocked) in [
        ("127.0.0.1", "http://[::ffff:127.0.0.1]/", true),
        ("127.0.0.1", "http://[::ffff:7f00:1]/", true),
        ("127.0.0.1", "http://[0:0:0:0:0:ffff:7f00:1]/", true),
        ("[::ffff:127.0.0.1]", "http://127.0.0.1/", true),
        ("[::ffff:7f00:1]", "http://[::ffff:127.0.0.1]/", true),
        ("[::ffff:127.0.0.1]", "http://2130706433/", true),
        ("10.0.0.1", "http://[::ffff:a00:1]/", true),
        ("127.0.0.1", "http://[::ffff:7f00:2]/", false),
        ("127.0.0.1", "http://[::1]/", false),
        ("[::1]", "http://127.0.0.1/", false),
        ("127.0.0.1", "http://127.0.0.1.nip.io/", false),
    ] {
        assert_eq!(
            decide(&sites(&[target]), &[], url),
            blocked,
            "target {target}, url {url}"
        );
    }
}

// ---------------------------------------------------------------------------
// Rework round 1, R3 extended: hostile paths and hosts
// ---------------------------------------------------------------------------

#[test]
fn hostile_paths_and_hosts_never_make_the_matcher_panic() {
    let long = [
        format!("https://youtube.com/{}", "%2F..".repeat(20_000)),
        format!("https://youtube.com/{}shorts", "..;/".repeat(20_000)),
        format!("https://youtube.com/{}shorts", "/".repeat(100_000)),
        format!("https://youtube.com/{}", ";".repeat(100_000)),
        format!("https://youtube.com/{}", "%5c".repeat(30_000)),
        format!("https://x.com/a/{}", "b;/".repeat(30_000)),
        format!("https://reddit.com{}/", ".".repeat(10_000)),
        format!("https://reddit.com{}/", "%2E".repeat(10_000)),
    ];
    let short = [
        "http://[::ffff:0:0]/",
        "http://[::ffff:ffff:ffff]/",
        "http://[::]/",
        "http://0.0.0.0/",
        "http://255.255.255.255/",
        "https://x.com/;",
        "https://x.com/%2",
        "https://x.com/%2f%",
        "https://x.com/..;",
        "https://x.com/;/..;/../",
    ];
    let matcher = SiteMatcher::new(&blocklist(
        sites(&[
            "youtube.com/shorts",
            "reddit.com",
            "127.0.0.1",
            "[::1]",
            "x.com/a",
        ]),
        patterns(&["youtube.com/shorts/ok", "x.com/a/b", "old.reddit.com"]),
    ));
    let mut answers = 0_usize;
    for url in long.iter().map(String::as_str).chain(short) {
        let _ = matcher.is_blocked(url);
        answers += 1;
    }
    assert_eq!(answers, long.len() + short.len());

    // Hosts made only of dots are empty once their trailing dots go.
    for url in ["https://../", "https://./", "https://.../"] {
        assert!(!matcher.is_blocked(url), "{url} must not be blocked");
    }
}

// ---------------------------------------------------------------------------
// Rework round 2, N-1: an encoded semicolon counts like a semicolon
// ---------------------------------------------------------------------------

#[test]
fn encoded_semicolon_counts_like_a_semicolon() {
    let shorts = sites(&["youtube.com/shorts"]);
    for url in [
        "https://youtube.com/shorts%3Bx/abc",
        "https://youtube.com/shorts%3bx",
        "https://youtube.com/feed/..%3B/shorts/abc",
        "https://youtube.com/feed/..%3b/shorts",
        "https://youtube.com//shorts%3Bx/abc",
    ] {
        assert!(
            decide(&shorts, &[], url),
            "youtube.com/shorts should block {url}"
        );
    }
    for url in [
        "https://youtube.com/watch%3Bshorts",
        "https://youtube.com/shortsfoo%3Bx",
        "https://youtube.com/watch?x=%3B/shorts",
    ] {
        assert!(
            !decide(&shorts, &[], url),
            "youtube.com/shorts should not block {url}"
        );
    }

    let targets = sites(&["x.com"]);
    let exceptions = patterns(&["x.com/a"]);
    for url in [
        "https://x.com/a/..%3B/secret",
        "https://x.com/a%3Bx/b",
        "https://x.com/a/b%3bv=1",
        "https://x.com/a/b%3B",
    ] {
        assert!(
            decide(&targets, &exceptions, url),
            "x.com/a must not exempt {url}"
        );
    }
    assert!(
        !decide(&targets, &exceptions, "https://x.com/a/b?q=%3B#%3b"),
        "x.com/a should exempt a path whose query and fragment have %3B"
    );
    assert!(
        !decide(&targets, &patterns(&["x.com"]), "https://x.com/a%3Bx/b"),
        "an exception without a path should exempt any path"
    );

    // Long runs don't make the matcher panic.
    let long = format!("https://youtube.com/{}shorts", "..%3B/".repeat(20_000));
    let _ = SiteMatcher::new(&blocklist(
        shorts.clone(),
        patterns(&["youtube.com/shorts/ok"]),
    ))
    .is_blocked(&long);
}
