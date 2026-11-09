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
//! are never looked at.
//!
//! # Patterns
//!
//! Targets and exceptions are [`SitePattern`]s with the same meaning: an
//! optional `*.`, a domain, and an optional path starting with `/`, such as
//! `reddit.com`, `*.reddit.com` or `youtube.com/shorts`.
//!
//! - **Domain.** Compared after IDNA processing, as WHATWG does for hosts, so
//!   letter case, Unicode or punycode (`xn--`), full-width characters and one
//!   trailing dot make no difference. A domain covers itself and every
//!   subdomain, by whole labels: `reddit.com` covers `old.reddit.com`, but
//!   not `notreddit.com` or `reddit.com.evil.com`. `*.reddit.com` means the
//!   same as `reddit.com`.
//! - **Path.** The URL's path, after WHATWG resolves `.` and `..`, must equal
//!   the pattern's path or continue it after a `/`: `youtube.com/shorts`
//!   covers `/shorts`, `/shorts/` and `/shorts/abc`, not `/shortsfoo`. Both
//!   are compared without regard to ASCII letter case, with percent-encoded
//!   unreserved characters (`A-Z a-z 0-9 - . _ ~`) decoded. A trailing `/` in
//!   the pattern is ignored.
//!
//! A pattern the matcher can't use matches nothing, so that such an exception
//! never exempts anything. That covers a pattern with no domain (empty, blank,
//! `*`, `*.`, `.`, or only a path), a domain that isn't a valid host (with a
//! scheme, a port or user info, say), a `*` other than a leading `*.`, an
//! empty label, and a path with a query (`?`) or a fragment (`#`).
//!
//! # Untrusted input
//!
//! URLs and patterns come from clients the service doesn't trust
//! (`docs/threat-model.md`). Matching works on text alone: it never opens,
//! resolves or looks up anything (no DNS, no file system), and no input makes
//! it panic. It doesn't need the blocklist to have passed
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
        self.targets.iter().any(|rule| rule.matches(&page))
            && !self.exceptions.iter().any(|rule| rule.matches(&page))
    }
}

/// A web page, reduced to the parts the matcher compares.
#[derive(Debug)]
struct Page {
    /// The URL's host, with one trailing dot removed.
    host: Host<String>,
    /// The URL's path, normalized by [`normalize_path`].
    path: String,
}

impl Page {
    /// Parses `input` as a URL, or returns `None` if it isn't an `http` or
    /// `https` URL.
    fn parse(input: &str) -> Option<Self> {
        let url = Url::parse(input).ok()?;
        if !matches!(url.scheme(), "http" | "https") {
            return None;
        }
        Some(Self {
            host: without_trailing_dot(url.host()?.to_owned()),
            path: normalize_path(url.path()),
        })
    }
}

/// A usable pattern: a host and the path below which it applies.
#[derive(Debug, Clone, PartialEq, Eq)]
struct Rule {
    /// The pattern's host, with one trailing dot removed. A domain has at
    /// least one label and no empty ones.
    host: Host<String>,
    /// The pattern's path, normalized by [`normalize_path`], without a
    /// trailing `/`. Empty when the pattern covers the whole site.
    path: String,
}

impl Rule {
    /// Reads a pattern, or returns `None` if the matcher can't use it.
    fn parse(pattern: &SitePattern) -> Option<Self> {
        let text = pattern.as_str().trim();
        // WHATWG treats `\` like `/` in http(s) URLs, so both end the domain.
        let (domain, path) = match text.find(['/', '\\']) {
            Some(index) => text.split_at_checked(index)?,
            None => (text, ""),
        };
        let domain = domain.strip_prefix("*.").unwrap_or(domain);
        let host = without_trailing_dot(Host::parse(domain).ok()?);
        if let Host::Domain(name) = &host
            && (name.contains('*') || name.split('.').any(str::is_empty))
        {
            return None;
        }
        Some(Self {
            host,
            path: rule_path(path)?,
        })
    }

    /// Whether this rule covers `page`.
    fn matches(&self, page: &Page) -> bool {
        self.covers_host(&page.host) && self.covers_path(&page.path)
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

/// Removes one trailing dot from a domain: `reddit.com.` is `reddit.com`.
fn without_trailing_dot(host: Host<String>) -> Host<String> {
    match host {
        Host::Domain(mut name) => {
            if name.ends_with('.') {
                name.pop();
            }
            Host::Domain(name)
        }
        other => other,
    }
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
