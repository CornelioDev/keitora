# 0009 — Match sites and apps as text, failing toward the safe side

- **Status:** Accepted
- **Date:** 2026-11-09

## Context

`keitora-core` decides whether a URL or a running program is blocked (T-1.4, T-1.5). The same rules must hold in the service, in the desktop app's forms (T-3.7, T-3.8) and in the browser extension (T-4.3). The crate is pure: it can't open files, resolve names or ask the OS anything. Users enter patterns by hand, and a user in a strict session has reasons to look for a way around a rule.

## Decision

- **Sites** are parsed with the WHATWG URL standard (the `url` crate) and compared after IDNA. Only `http` and `https` URLs are ever blocked. A domain covers its subdomains, `*.x` is the same as `x`, and `www.x` doesn't cover `x`. Paths match whole segments, ignoring letter case, after decoding percent-encoded unreserved characters.
- **A pattern the matcher can't use matches nothing**, and validation rejects it (`SitePattern::check`, `NotASitePattern`). One parser serves both, so validation and matching can't disagree.
- **Paths a server may read differently fail toward blocking.** A target with a path matches if any of its lenient readings does (`//`, `%2F`, `%5C`, `;`, `%3B`), and an exception with a path doesn't apply to such a path.
- **Apps** are matched by exact file name or exact full path, compared as text: no resolution, no prefix match. Letter case is folded one character at a time, and a non-ASCII character never folds into ASCII. A target with both a name and a path blocks on either.
- **Protected processes** are a fixed list of full paths under the Windows directory, plus Keitora's own executables, all passed in by the service. Protection is by full path only and always wins over a target.
- The shared fixture `crates/keitora-core/tests/acceptance/site_matcher.json` is the contract for every other implementation of the site rules.

## Alternatives considered

- **Resolving paths in the matcher** (`canonicalize`, short-name expansion): breaks the pure crate and lets a mapped drive make SYSTEM touch the network (#11). Short names are expanded by the service in T-2.6 instead.
- **Rejecting unusable patterns only in the UI:** the service would still accept them from any client, and a target that blocks nothing would fail silently.
- **Protecting by file name or by folder:** any user could copy a program to a lookalike name or folder and make it unkillable (#8).

## Consequences

- The extension can't express every rule with `declarativeNetRequest` filters; path rules need a navigation listener with the same logic (T-4.2, T-4.3).
- Copying a program under another name and folder, or launching it through an 8.3 alias, evades app rules in the pure matcher; the service expands short names (T-2.6), and copies remain an accepted limitation.
- Some legitimate URLs are over-blocked: a path exception doesn't apply on sites that use `;` or `//` in normal paths.
