// SPDX-License-Identifier: MIT OR Apache-2.0

//! The site rule matcher: whether a blocklist blocks a URL.
//!
//! A [`SiteMatcher`] is built once from a [`Blocklist`] and then answers any
//! number of URLs with [`SiteMatcher::is_blocked`]. A URL is blocked when at
//! least one site target matches it and no exception matches it: exceptions
//! always win, and on their own they block nothing. Application targets are
//! ignored here.
//!
//! The rules below are also the contract of the browser extension, which
//! checks the same table of cases, `tests/acceptance/site_matcher.json` in
//! this crate.
//!
//! # URLs
//!
//! A URL is parsed as the [WHATWG URL Standard](https://url.spec.whatwg.org/)
//! says, as a browser does. Text that doesn't parse, or whose scheme isn't
//! `http` or `https`, is not a website and is never blocked. Only the host
//! and the path count; the port, the user info, the query and the fragment
//! are never looked at. Every trailing dot of the host is ignored, so a host
//! made only of dots is never blocked.
//!
//! # Patterns
//!
//! Targets and exceptions are [`SitePattern`]s with the same meaning: an
//! optional `*.`, a host, and an optional path starting with `/`, such as
//! `reddit.com`, `*.reddit.com`, `youtube.com/shorts` or `127.0.0.1`.
//!
//! - **Domain.** Compared after IDNA processing, as WHATWG does for hosts, so
//!   letter case, Unicode or punycode (`xn--`), full-width characters and one
//!   trailing dot make no difference. A domain covers itself and every
//!   subdomain, by whole labels: `reddit.com` covers `old.reddit.com`, but
//!   not `notreddit.com` or `reddit.com.evil.com`. `*.reddit.com` means the
//!   same as `reddit.com`.
//! - **IP address.** Covers only itself, in any form WHATWG accepts
//!   (`2130706433` is `127.0.0.1`). An IPv6 address is written in brackets
//!   (`[::1]`), and an IPv4-mapped one (`[::ffff:127.0.0.1]`) is the same
//!   host as its IPv4 address, in URLs and in patterns.
//! - **Path.** The URL's path, after WHATWG resolves `.` and `..`, must equal
//!   the pattern's path or continue it after a `/`: `youtube.com/shorts`
//!   covers `/shorts`, `/shorts/` and `/shorts/abc`, not `/shortsfoo`. Both
//!   are compared without regard to ASCII letter case, with percent-encoded
//!   unreserved characters (`A-Z a-z 0-9 - . _ ~`) decoded. A trailing `/` in
//!   the pattern is ignored.
//!
//! ## Paths a server may read differently
//!
//! Servers don't all read a path the same way: some take `%2F` or `%5C` as a
//! `/`, drop `;` parameters from a segment, or merge repeated slashes. So
//! that such a path can't slip past a target, or fall under an exception it
//! doesn't belong to:
//!
//! - A **target** with a path also matches if any lenient reading of the URL's
//!   path does. A lenient reading applies any combination of these steps, in
//!   this order: read `%2F` and `%5C` as `/`; remove from each segment a `;`
//!   and everything after it; collapse each run of `/` into one. Then it
//!   resolves `.` and `..` again. `youtube.com/shorts` so blocks
//!   `/shorts%2Fabc`, `//shorts` and `/feed/..;/shorts`, but not
//!   `/shortsfoo%2Fabc`.
//! - An **exception** with a path doesn't apply at all when the URL's path
//!   contains `//`, `%2F`, `%5C` or `;`. An exception with no path doesn't
//!   look at the path.
//!
//! ## Patterns that match nothing
//!
//! A pattern the matcher can't use matches nothing, so that such an exception
//! never exempts anything: one with no host (empty, blank, `*`, `*.`, `.`, or
//! only a path), a host that isn't valid (with a scheme, a port or user info,
//! say), an empty label (a leading dot, `..`, more than one trailing dot), a
//! `*` other than a leading `*.`, or a path with a query (`?`) or a fragment
//! (`#`). [`SitePattern::check`], and so [`Blocklist::validate`], rejects
//! exactly these patterns, as it uses the same parser.
//!
//! # Untrusted input
//!
//! URLs and patterns come from clients the service doesn't trust
//! (`docs/threat-model.md`). Matching works on text alone: it never opens,
//! resolves or looks up anything (no DNS, no file system), and no input makes
//! it panic. Its work grows linearly with the length of the URL and the
//! number of patterns. It doesn't need the blocklist to have passed
//! [`Blocklist::validate`], though the service validates every blocklist
//! before using it.

use url::{Host, Url};

use crate::model::{Blocklist, SitePattern, Target};

#[cfg(test)]
mod tests;

/// Decides which URLs a blocklist blocks.
///
/// Built from a blocklist's site targets and exceptions with
/// [`SiteMatcher::new`]; the blocklist itself isn't kept, so a matcher must
/// be rebuilt when its blocklist changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct SiteMatcher {
    /// The usable patterns of the blocklist's site targets.
    targets: Vec<Rule>,
    /// The usable patterns of the blocklist's exceptions.
    exceptions: Vec<Rule>,
}

impl SiteMatcher {
    /// Builds a matcher from the site targets and exceptions of `blocklist`.
    ///
    /// It never fails: a pattern the matcher can't use (see the
    /// [module documentation](self)) matches nothing.
    #[must_use]
    pub fn new(blocklist: &Blocklist) -> Self {
        let targets = blocklist
            .targets
            .iter()
            .filter_map(|target| match target {
                Target::Site { pattern } => Rule::parse(pattern),
                Target::App { .. } => None,
            })
            .collect();
        let exceptions = blocklist
            .exceptions
            .iter()
            .filter_map(Rule::parse)
            .collect();
        Self {
            targets,
            exceptions,
        }
    }

    /// Returns `true` if `url` is blocked: some site target matches it and
    /// no exception does.
    ///
    /// Text that isn't an `http` or `https` URL is never blocked. The answer
    /// depends only on `url` and the blocklist, not on earlier calls.
    #[must_use]
    pub fn is_blocked(&self, url: &str) -> bool {
        let Some(page) = Page::parse(url) else {
            return false;
        };
        let ambiguous = is_ambiguous(&page.path);
        let target_covers = |path: &str| {
            self.targets
                .iter()
                .any(|rule| rule.covers_host(&page.host) && rule.covers_path(path))
        };
        let blocked = if ambiguous {
            lenient_readings(&page.path)
                .iter()
                .any(|reading| target_covers(reading))
        } else {
            target_covers(&page.path)
        };
        blocked
            && !self.exceptions.iter().any(|rule| {
                rule.covers_host(&page.host)
                    && (rule.path.is_empty() || (!ambiguous && rule.covers_path(&page.path)))
            })
    }
}

/// Tells if the matcher can use `pattern`; [`SitePattern::check`] rejects
/// every pattern for which this is `false`.
pub(crate) fn is_usable(pattern: &SitePattern) -> bool {
    Rule::parse(pattern).is_some()
}

/// A web page, reduced to the parts the matcher compares.
#[derive(Debug)]
struct Page {
    /// The URL's host, without trailing dots, and an IPv4-mapped IPv6
    /// address as IPv4.
    host: Host<String>,
    /// The URL's path, normalized by [`normalize_path`].
    path: String,
}

impl Page {
    /// Parses `input` as a URL, or returns `None` if it isn't an `http` or
    /// `https` URL, or its host is made only of dots.
    fn parse(input: &str) -> Option<Self> {
        let url = Url::parse(input).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        let host = match canonical_ip(url.host()?.to_owned()) {
            Host::Domain(name) => {
                let name = name.trim_end_matches('.');
                if name.is_empty() {
                    return None;
                }
                Host::Domain(name.to_owned())
            }
            ip => ip,
        };
        Some(Self {
            host,
            path: normalize_path(url.path()),
        })
    }
}

/// A usable pattern: a host and the path below which it applies.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    /// The pattern's host, with one trailing dot removed, and an IPv4-mapped
    /// IPv6 address as IPv4. A domain has at least one label and no empty
    /// ones.
    host: Host<String>,
    /// The pattern's path, normalized by [`normalize_path`], without a
    /// trailing `/`. Empty when the pattern covers the whole site.
    path: String,
}

impl Rule {
    /// Reads a pattern, or returns `None` if the matcher can't use it.
    fn parse(pattern: &SitePattern) -> Option<Self> {
        let text = pattern.as_str().trim();
        // WHATWG treats `\` like `/` in http(s) URLs, so both end the host.
        let (host, path) = match text.find(['/', '\\']) {
            Some(index) => text.split_at_checked(index)?,
            None => (text, ""),
        };
        let host = host.strip_prefix("*.").unwrap_or(host);
        let host = match canonical_ip(Host::parse(host).ok()?) {
            Host::Domain(name) => {
                let name = name.strip_suffix('.').unwrap_or(&name);
                if name.contains('*') || name.split('.').any(str::is_empty) {
                    return None;
                }
                Host::Domain(name.to_owned())
            }
            ip => ip,
        };
        Some(Self {
            host,
            path: rule_path(path)?,
        })
    }

    /// Whether `host` is this rule's host or, for a domain, a subdomain of
    /// it. An IP address only covers itself.
    fn covers_host(&self, host: &Host<String>) -> bool {
        match (&self.host, host) {
            (Host::Domain(rule), Host::Domain(page)) => {
                page == rule
                    || page
                        .strip_suffix(rule.as_str())
                        .is_some_and(|subdomains| subdomains.ends_with('.'))
            }
            (rule, page) => rule == page,
        }
    }

    /// Whether `path` is this rule's path or continues it after a `/`.
    fn covers_path(&self, path: &str) -> bool {
        path.strip_prefix(self.path.as_str())
            .is_some_and(|rest| rest.is_empty() || rest.starts_with('/'))
    }
}

/// Turns an IPv4-mapped IPv6 address (`::ffff:127.0.0.1`) into its IPv4
/// address, so that both forms are the same host. Other hosts stay as they
/// are; `::1` is not `0.0.0.1`.
fn canonical_ip(host: Host<String>) -> Host<String> {
    match host {
        Host::Ipv6(address) => address
            .to_ipv4_mapped()
            .map_or(Host::Ipv6(address), Host::Ipv4),
        other => other,
    }
}

/// Normalizes the path part of a pattern, which is empty or starts with `/`
/// or `\`, the way a URL's path is: resolved by WHATWG, then by
/// [`normalize_path`], without one trailing `/`. Returns `None` if the path
/// has a query or a fragment, which the matcher doesn't use.
fn rule_path(path: &str) -> Option<String> {
    if path.is_empty() {
        return Some(String::new());
    }
    if path.contains(['?', '#']) {
        return None;
    }
    // The path starts with a separator, so it can't change the host. The
    // `.invalid` top-level domain is reserved, and nothing is looked up.
    let url = Url::parse(&format!("https://pattern.invalid{path}")).ok()?;
    let mut normalized = normalize_path(url.path());
    if normalized.ends_with('/') {
        normalized.pop();
    }
    Some(normalized)
}

/// Decodes percent-encoded unreserved characters (`A-Z a-z 0-9 - . _ ~`) in
/// a path and lowercases its ASCII letters. Other percent-encoded bytes are
/// kept encoded, with lowercase hexadecimal digits.
fn normalize_path(path: &str) -> String {
    let mut normalized = String::with_capacity(path.len());
    let mut chars = path.chars();
    while let Some(c) = chars.next() {
        if c == '%' {
            let mut lookahead = chars.clone();
            if let (Some(high), Some(low)) = (lookahead.next(), lookahead.next())
                && let Some(decoded) = decode_unreserved(high, low)
            {
                normalized.push(decoded.to_ascii_lowercase());
                chars = lookahead;
                continue;
            }
        }
        normalized.push(c.to_ascii_lowercase());
    }
    normalized
}

/// The unreserved character that the hexadecimal digits `high` and `low`
/// encode, or `None` if they aren't two digits or encode anything else.
fn decode_unreserved(high: char, low: char) -> Option<char> {
    let value = high
        .to_digit(16)?
        .checked_mul(16)?
        .checked_add(low.to_digit(16)?)?;
    let decoded = char::from_u32(value)?;
    let unreserved = decoded.is_ascii_alphanumeric() || matches!(decoded, '-' | '.' | '_' | '~');
    unreserved.then_some(decoded)
}

/// Tells if a server could read a normalized path in more than one way: it
/// contains `//`, an encoded `/` or `\` (`%2f`, `%5c`), or a `;`.
fn is_ambiguous(path: &str) -> bool {
    path.contains("//") || path.contains("%2f") || path.contains("%5c") || path.contains(';')
}

/// The eight lenient readings of a normalized path (see the module
/// documentation): every combination of [`decode_slashes`],
/// [`strip_parameters`] and [`collapse_slashes`], in that order, each then
/// resolved by [`resolve_dot_segments`]. The first is the path itself.
fn lenient_readings(path: &str) -> Vec<String> {
    let mut readings = Vec::with_capacity(8);
    for decode in [false, true] {
        for strip in [false, true] {
            for collapse in [false, true] {
                let mut reading = path.to_owned();
                if decode {
                    reading = decode_slashes(&reading);
                }
                if strip {
                    reading = strip_parameters(&reading);
                }
                if collapse {
                    reading = collapse_slashes(&reading);
                }
                readings.push(resolve_dot_segments(&reading));
            }
        }
    }
    readings
}

/// Reads an encoded `/` or `\` (`%2f`, `%5c`) in a normalized path as `/`.
fn decode_slashes(path: &str) -> String {
    path.replace("%2f", "/").replace("%5c", "/")
}

/// Removes from each segment of a path a `;` and everything after it.
fn strip_parameters(path: &str) -> String {
    let mut stripped = String::with_capacity(path.len());
    let mut in_parameters = false;
    for c in path.chars() {
        if c == '/' {
            in_parameters = false;
        } else if c == ';' {
            in_parameters = true;
        }
        if !in_parameters {
            stripped.push(c);
        }
    }
    stripped
}

/// Collapses each run of `/` in a path into one.
fn collapse_slashes(path: &str) -> String {
    let mut collapsed = String::with_capacity(path.len());
    for c in path.chars() {
        if !(c == '/' && collapsed.ends_with('/')) {
            collapsed.push(c);
        }
    }
    collapsed
}

/// Resolves `.` and `..` segments in a path that starts with `/`, as WHATWG
/// does: a `..` removes the segment before it, and either one at the end
/// leaves a trailing `/`.
fn resolve_dot_segments(path: &str) -> String {
    let mut segments: Vec<&str> = Vec::new();
    let mut parts = path.strip_prefix('/').unwrap_or(path).split('/').peekable();
    while let Some(segment) = parts.next() {
        let last = parts.peek().is_none();
        match segment {
            "." | ".." => {
                if segment == ".." {
                    segments.pop();
                }
                if last {
                    segments.push("");
                }
            }
            other => segments.push(other),
        }
    }
    let mut resolved = String::with_capacity(path.len());
    for segment in segments {
        resolved.push('/');
        resolved.push_str(segment);
    }
    // `split` yields at least one segment, and the last one always adds an
    // entry, so the result is never empty: at least `/`.
    resolved
}
