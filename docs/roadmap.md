# Roadmap

The detailed plan, task by task, for contributors and agents. [ROADMAP.md](../ROADMAP.md) is the short public summary.

## Current status

- **Active phase:** F1 — Domain core
- **Next task:** T-1.6
- **Last updated:** 2026-11-10
- **Notes:** Planning complete. Stack: Rust service + Tauri 2 + Vue 3 + shadcn-vue. Dual-licensed MIT OR Apache-2.0. Every task runs through the loop in `docs/workflow.md`. T-0.1 done (Rust pinned to 1.99.1, MSRV 1.99). T-0.2 done (LF everywhere via `.gitattributes`, anchored ignore rules, forced text diffs for sources). T-0.3 done (`deny.toml`: permissive licenses only, advisories in all dependencies, no duplicates, crates.io only; checks `x86_64-pc-windows-msvc` only, confirmed by the maintainer). T-0.4 done (lefthook: rustfmt, Prettier and ESLint on staged files, Conventional Commits check; run `lefthook install` once per clone). T-0.5 done (labels, milestones F0–F6, squash-only merges, `main` ruleset without bypass, private vulnerability reporting, Discussions, DCO app). Decided for T-0.6: accept MPL-2.0 only for the five Tauri crates that need it (`cssparser`, `cssparser-macros`, `dtoa-short`, `option-ext`, `selectors`), as per-crate exceptions in `deny.toml`, not in the global allow list. T-0.6 done (`apps/desktop` from the Tauri 2 Vue + TypeScript template; the app is named Keitora, identifier `io.github.corneliodev.keitora`; its Rust crate `keitora-desktop` is a workspace member; strict CSP, no Tauri permissions, Tauri's own bundler off until T-6.1; a release build without the embedded frontend refuses to start). T-0.7 done (ESLint flat config, Prettier, `vue-tsc --build` and Vitest + jsdom in `apps/desktop`; `pnpm lint` includes the format check; `apps/desktop/pnpm-workspace.yaml` waits a day for new releases and allows no dependency build scripts). T-0.8 done (Tailwind CSS v4; shadcn-vue 2.9.0 run with `pnpm dlx`, the 20 components kept identical to the CLI output; a dev-only component gallery). Zod is deferred to the first real form in F3: no released VeeValidate adapter supports a zod version fixed for GHSA-mrhx-c7xc-m3w5. T-0.9 done (tokens in `src/styles/tokens.css`; one radius scale from the 12px base, decided by the maintainer: controls and menus 8px, cards, dialogs and toasts 12px; Geist and Geist Mono bundled with their OFL texts). T-0.10 done (vue-i18n with `en`/`es`, locale from the Windows display language, raw-text lint rule; the generated shadcn-vue components still have English screen-reader strings, to translate in F3). T-0.11 done (`extension/` as its own pnpm project with the desktop app's tooling and install safeguards; no permissions, an empty service worker, visible name Keitora, `en`/`es` from `src/_locales`; a spec lists the exact manifest keys, so new capabilities are added on purpose; tested in Edge, Chrome only if the maintainer has it). T-0.12 done (11 images of the 9 MVP screens in `docs/assets/mockups/`, the visual reference in `docs/ui-stack.md`, with exceptions for the name, the radius scale and an overflow on the Sites tab). T-0.16 done (`just check` runs every check through `cmd.exe`, one command per line, and stops at the first failure; frontend dependencies install with a frozen lockfile, cargo steps use `--locked`, and a `cargo.cmd` or `pnpm.cmd` in the repository can't replace the real tool; a clone path of 134 characters or more breaks Vitest on Windows (measured in T-1.4), documented in `CONTRIBUTING.md`). T-0.17 done (`just coverage` fails if `keitora-core` or `keitora-ipc` is below 90 % of lines, per crate, with `--all-features`; TS coverage is reported with no minimum; decided by the maintainer: unit tests of those two crates live in sibling files, so the 90 % measures production code only; each crate has an inline placeholder test to delete when it gets real code). T-0.18 done (CodeQL for Rust and TypeScript in `.github/workflows/codeql.yml`, advanced setup, on `windows-latest` so it sees `#[cfg(windows)]` code; secret scanning and push protection were already on; `just check-acceptance-tests <base>` fails if acceptance tests change outside a `test(` commit, and such a commit may touch nothing else, as decided by the maintainer; CI runs it on pull requests since T-0.13). T-0.19 done (`just check` enforces the unit test layout of `keitora-core` and `keitora-ipc` with `scripts/check-unit-test-layout.mjs`: it reads what the compiler read and scans every `.rs` in the crates; it also bans `debug_assertions` in `cfg`, build scripts, binaries and examples there, and direct proc-macro or local dependencies other than `keitora-ipc` on `keitora-core`; accepted by the maintainer, including its size; what it can't see is listed in `CONTRIBUTING.md` and stays with human review). T-0.13 done (`.github/workflows/ci.yml` on `windows-latest`: `just check`, `just coverage`, the npm advisory audit and the acceptance test check, which runs the base's copy of the script on a bare repository with no pull request files on disk; CodeQL analyzes without `security-events: write` and a second job with no checkout uploads the SARIF; `just`, `cargo-deny`, `cargo-llvm-cov` and pnpm are pinned; the `main` ruleset now requires a pull request, squash only with 0 approvals, the branch up to date and nine checks: `just check`, `just coverage`, `npm dependencies`, `Acceptance tests`, `Analyze (rust)`, `Analyze (javascript-typescript)`, `Upload (rust)`, `Upload (javascript-typescript)` and `DCO`; first run: `just check` 7.5 min and `Analyze (rust)` 13.5 min with no cache, CodeQL extracted 7 of 7 Rust files with no errors; in pull requests CodeQL reports only changed lines). T-0.14 done (`release-please` keeps one release pull request open, with one version for the whole project; the first is `0.1.0-beta.1`, and they stay betas until T-6.8; the changelog is generated; the pull request title normally decides the version; the build turns the version into one a Manifest V3 extension accepts, `0.1.0-beta.1` into `0.1.0.1`; a GitHub App limited to this repository opens the pull requests, because those opened with `GITHUB_TOKEN` trigger no checks, and its key is in an environment `release-please` limited to `main`; a ruleset on `v*` tags stops anyone moving or deleting them; Actions allows only hash-pinned GitHub-owned actions and three third-party ones, listed in `CONTRIBUTING.md`). The release pull request stays open until F6 is done; nobody merges it before that. Decided 2026-11-02 by the maintainer: during the MVP licenses don't block anything, so the npm license check is off as a gate (`just npm-licenses` stays, by hand, and fails on `elkjs`, EPL-2.0, through `@unovis/ts`; it reaches the built app only if something uses `VisGraph`). Decided 2026-11-02: GHSA-g3hv-75wg-m249 (`minimatch`) and GHSA-3g3m-2fjr-6pw4 (`glob`) are ignored by ID, per project, because they reach dev dependencies only and upstream has no fix yet. Open by 2026-12-02: review those two ignored advisories; no tool enforces this date. Open before the first public release: analyze npm licenses, including `elkjs`, and decide whether the check becomes a gate again. Open for T-6.5 ([#3](https://github.com/CornelioDev/keitora/issues/3), accepted until then): stop the release App from merging into `main`; turn on Immutable Releases and publish as a draft; check that a tagged commit descends from `main`. Open for T-6.1: an MSI needs its own numeric version, as the extension does. Open for T-6.3: decide whether the stores show `0.1.0-beta.1` through `version_name`. Open before the first release: the README must say releases are pre-releases. F0 closed on 2026-11-03: the phase security review passed with no Critical or High finding; its Medium findings are issues #7 to #19, the F1 ones also as Security criteria in T-1.1, T-1.3, T-1.5 and T-1.6; the maintainer ran the phase acceptance checklist and everything passed. T-1.1 done (`keitora_core::model`: `Blocklist`, `Target` (`Site` | `App`), `AppIdentifiers` and `SitePattern` as serde JSON, `Target` tagged by `kind` and read through a flat struct so that unknown fields are skipped, not buffered; `Blocklist::validate()` is a separate step that the service must call on every blocklist it receives or loads (T-2.3, T-2.5); a Windows app path must start with a drive letter, `:` and `\`, which rejects UNC, device, NT and relative paths, and it is only compared, never opened (#11); maximums count UTF-8 bytes: 500 targets, 200 exceptions, names 256, patterns 512, app identifiers 1024, ids 64 made of ASCII letters, digits and `-`, colors `#rrggbb` (#15); text rejects control characters, bidi controls, line and paragraph separators and the BOM, and keeps ZWJ and ZWNJ; `keitora-core` no longer has a placeholder test). T-1.2 done (`keitora_core::clock`: a `Clock` trait with a monotonic reading, `MonotonicInstant`, and a wall reading, `SystemTime`; remaining time must come from the monotonic reading only, and a `MonotonicInstant` is never persisted, since its origin is each clock's own; `FakeClock` is an unconditional `pub` type shared between clones, whose `advance` fails without changing anything on overflow and whose `set_wall` moves only the wall clock; the service must never use it (T-2.1, T-2.3); `crates/keitora-core/clippy.toml` and `crates/keitora-ipc/clippy.toml` repeat the root settings and ban the system clock, processes, network, files, environment variables and `sleep` in those crates, and `scripts/clippy-config.spec.mjs` fails if they drift from the root file). T-1.4 done (`keitora_core::site::SiteMatcher`: URLs parsed per WHATWG with the `url` crate, the only new direct dependency, which adds no package to `Cargo.lock`; only `http` and `https` URLs are ever blocked; a domain covers its subdomains by whole labels, `*.x` equals `x`, and `www.x` doesn't cover `x`; paths match whole segments, ignoring case, after decoding percent-encoded unreserved characters, all decided by the maintainer on 2026-11-09; exceptions win; after the security review, by the maintainer's decision, `Blocklist::validate` rejects site patterns the matcher can't use (`SitePattern::check`, `ValidationErrorKind::NotASitePattern`, one parser for both), a target with a path also matches the lenient readings of a page path (`//`, `%2F`, `%5C`, `;`, `%3B`) and a path exception doesn't apply to such a path, every trailing dot of a page host is ignored, and an IPv4-mapped IPv6 address equals its IPv4; the shared fixture `crates/keitora-core/tests/acceptance/site_matcher.json`, 174 cases plus `pattern_validation`, is the contract for T-3.7 and T-4.3; the pure crates' `clippy.toml` also ban name resolution; the clone path limit is 133 characters, measured: Vitest fails from 134 and the unit test layout check from 147). Open for T-2.3 and T-4.1: cap a URL's size well below 2 MB before matching (a hostile 2 MB URL takes about 0.3 s against 700 rules). Open for T-3.7: store and show a pattern's canonical form, trim spaces, warn that `www.x` doesn't block `x` and about public-suffix exceptions such as `com`, and explain that a path exception doesn't apply to URLs with `;` or `//`. Open for T-4.2: Chrome's `declarativeNetRequest` filters can't express the lenient path readings or the ambiguous-path rule, so path rules need a navigation listener with the same logic; decide before T-4.2, and run the fixture through the real browser in T-4.3. T-1.5 done (`keitora_core::app`: `AppMatcher::verdict` returns `Allowed`, `Blocked` or `Protected` for a process image path, compared as text only, with no resolution; `\\?\`, `\??\` and `\\.\` before a drive are ignored, `/` equals `\`, repeated separators count as one, and letter case is folded one character at a time, never folding a non-ASCII character into ASCII; a name target matches the whole file name and a path target the whole path; decided by the maintainer on 2026-11-09: a target with both a name and a path blocks on either, `steam` doesn't match `steam.exe`, and `ProtectedProcesses` protects, by full path only, `explorer.exe` and 16 programs in `System32` under the Windows directory the service passes in, plus Keitora's service, desktop app and native messaging host; the pure crates' `clippy.toml` also ban path resolution and file metadata; an 8.3 alias of a file name evades name and path targets in the pure matcher, which T-2.6 closes; ADR-0009 records how sites and apps are matched). Open: more `clippy.toml` bans in the pure crates and the outdated sentence in `AGENTS.md` ([#41](https://github.com/CornelioDev/keitora/issues/41)). Open for T-3.8: with name OR path, a generic `windows_exe` such as `launcher.exe` blocks every program with that name; warn or don't fill it; don't offer Windows shell processes such as `msedgewebview2.exe` (Keitora's own UI), `TextInputHost.exe` or `StartMenuExperienceHost.exe` in the picker (T-2.8). Open for T-6.1: install the three Keitora executables per machine in `Program Files`. T-1.3 done (`keitora_core::engine`: decided by the maintainer on 2026-11-09, a session has `cycles` focus phases with a short break between them and the long break after the last, then the engine is idle again; every phase starts on its own; only breaks can be skipped (a short one starts the next focus with its full time, the long one ends the session; skipping while paused stays paused); the schedule is fixed from the start and only pauses and skips move it, so a late observation lands on the right phase; a break keeps the cycle number of the focus before it; `PomodoroPreset::new` rejects, never clamps, values outside 1 to 120 min of focus, 1 to 30 of short break, 1 to 60 of long break and 1 to 8 cycles, so a session lasts at most 20 h 30 min (#15); `LockKind` is `None | UntilTimerEnds` and a strict session refuses pause; time comes only from the monotonic clock, kept as active time (ADR-0010)). Open for F2/F3: the "start breaks automatically" setting, which never applies to strict sessions. Open for F3: the strict confirmation dialog shows the session's total length. Open for #11: extend `docs/threat-model.md` from "executes or writes" to "opens, reads or resolves". Dependabot ran its first weekly version updates on 2026-11-04 (#25 to #28); decided 2026-11-09 by the maintainer: `@types/node` stays on the major of the Node version CI runs (22), so Dependabot ignores its major updates. Open: `just coverage` failed once in CI on a hook timeout (30 s) in `apps/desktop/tests/acceptance/t0-8-component-gallery.spec.ts`, in a pull request that didn't touch `apps/desktop` (#28); it's a required check, so a flaky test blocks merges; for QA. Open: nothing checks the pull request title, which now normally decides releases too. Open: harden `scripts/check-npm-audit.spec.mjs` (real pnpm test that `--audit-level low` beats `auditLevel`; catch every way of writing an ignored ID; fail if an ignored package enters production); add a Dependabot `cooldown`; check that Dependabot reads the lockfile's new second YAML document; Dependabot alert GHSA-wrw7-89jp-8q8g on `glib` and its failing updates; the `tool:` versions in `ci.yml` change by hand. T-0.15 done (name check on 2026-11-02: `keitora` and our crate and package names are free on crates.io and npm, no match in the Chrome, Edge or Microsoft stores, though those pages render with JavaScript so that isn't conclusive; the maintainer searched WIPO Global Brand Database and J-PlatPat in Latin script with no results, which doesn't cover Japanese spellings; this is no legal clearance. Accepted by the maintainer: `rudoi/keitora` is an active Tauri app of the same name, `keitora.jp` is a kei truck exporter, and the word is the common Japanese term for a kei truck. The GitHub repository is `CornelioDev/keitora`, and the old name must not be recreated, since that breaks the redirect. The whole tree is renamed, ADRs and past notes included, as a plain substitution. Decided: the mockups keep the exception in `docs/ui-stack.md` during the MVP. The pipe and `ProgramData` names exist only in docs so far). Decided 2026-10-30: from T-0.13 on, merges go through GitHub pull requests, with no ruleset bypass. Decided 2026-11-03 for T-1.4: a domain covers its subdomains (`reddit.com` blocks `old.reddit.com`), and a wildcard also covers the root domain (`*.reddit.com` blocks `reddit.com`), as the mockups assume; both are pinned in the shared fixture. Decided 2026-11-03 for T-1.6: only `UntilTimerEnds` is strict; `RandomText` is friction, so typing the text may stop the session. Open for F2/F5: how a strict session's time behaves when the machine is off or the clock changes ([#19](https://github.com/CornelioDev/keitora/issues/19)); the service rejects blocklist and settings edits that weaken an active strict session ([#18](https://github.com/CornelioDev/keitora/issues/18)). Open for F3: the Sites tab needs scrolling or a layout that fits; in the mockup the ring shows time remaining and the tray bar time elapsed; `DialogScrollContent.vue` uses 8px where dialogs are 12px. Open for F4: the block page says both that a site returns at the next break and that it can't be unblocked until the session ends. Open for F3: the CSP blocks the chart styles that Unovis and `ChartStyle.vue` inject at runtime; focus, break and strict colors need text variants in the light theme; translate the English screen-reader strings in the generated components. Open for F4: QA pins `permissions`, `host_permissions` and `externally_connectable` in acceptance tests when they are added; enable `no-floating-promises` and `no-misused-promises` before T-4.2; add a `dev` script and `@types/chrome`; the extension's description already says it blocks sites. Accepted risk: the acceptance test check doesn't recognize Windows short names (`ACCEPT~1.RS`); revisit if CI ever uses a self-hosted runner that reuses its workspace. Open for F2: delete the placeholder test in `keitora-ipc` once it has tested code, and remove the crate from `PLACEHOLDER_CRATES` in `scripts/unit-test-layout-rules.mjs` (`keitora-core`'s went in T-1.1). Open in `CONTRIBUTING.md`: loading `extension/dist` needs a build first, and `register-nmh.ps1` doesn't exist until T-4.1. Open: a new `.ts` file outside `src/` and `tests/` in `extension` or `apps/desktop` isn't typechecked. Open: nothing formats Markdown, JSON or YAML outside `apps/desktop` and `extension` (no root Prettier). Open before F2: ADR on isolating `unsafe` code ([#1](https://github.com/CornelioDev/keitora/issues/1)).

Legend: `[ ]` todo · `[~]` in progress / blocked · `[x]` done. Each phase maps to a GitHub milestone; each task can be mirrored as an issue titled `T-x.y: …`.

---

## MVP scope (Windows)

**In:** pomodoro timer (focus / short break / long break, cycles); blocklists with sites and apps; app blocking by process; site blocking through an extension for Chrome and Edge; strict mode with a time lock; service survives closing the UI and reboots; tray icon; notifications; simple history; English and Spanish UI; signed MSI installer.

**Out (post-MVP):** Firefox extension, network-level blocking (DNS/WFP), recurring schedules, advanced statistics, sync, mobile, auto-update of the service.

MVP phases: **F0 → F6**. Later: **F7 → F9**.

---

## F0 — Foundations

### Repository and community

- [x] **T-0.1** Initialize the Cargo workspace with empty crates `keitora-core`, `keitora-ipc`, `keitora-service`, `keitora-nmh`; set `rust-version` (MSRV), `edition = "2024"`, `license = "MIT OR Apache-2.0"`, and `[workspace.lints]`; add `rust-toolchain.toml`.
  - Acceptance: `cargo build --workspace` and `cargo test --workspace` pass.
- [x] **T-0.2** Add `rustfmt.toml`, `.editorconfig`, `.gitattributes` (LF, `*.rs text eol=lf`), `.gitignore`, and `.vscode/extensions.json` (rust-analyzer, Vue - Official, ESLint, Prettier, Tailwind CSS IntelliSense, EditorConfig).
- [x] **T-0.3** Add `deny.toml` for `cargo-deny`: allowed licenses compatible with MIT/Apache-2.0, advisories, banned duplicates.
- [x] **T-0.4** Configure `lefthook.yml`: pre-commit runs `cargo fmt --check`, Prettier, ESLint on staged files; commit-msg checks Conventional Commits.
- [x] **T-0.5** GitHub settings: create the labels listed in CONTRIBUTING, milestones F0–F6, squash-only merges, a ruleset on `main` (no deletion, no force-push, linear history; no required review while there is a single maintainer, since GitHub doesn't let authors approve their own PRs), private vulnerability reporting, Discussions, and the DCO app. Issue forms, PR template, `CODEOWNERS` and `dependabot.yml` are already in `.github/`.
  - Depends on: T-0.1

### Frontend and extension scaffolding

- [x] **T-0.6** Create `apps/desktop` from the official Tauri 2 Vue + TypeScript template (pnpm).
  - Acceptance: `pnpm tauri dev` opens an empty window.
- [x] **T-0.7** Add ESLint (flat config with `eslint-plugin-vue` and `typescript-eslint`), Prettier, `vue-tsc` typecheck, Vitest + Vue Test Utils.
  - Depends on: T-0.6
- [x] **T-0.8** Add Tailwind CSS v4 and initialize shadcn-vue; install the components listed in `docs/ui-stack.md`.
  - Depends on: T-0.7
  - Acceptance: a dev-only page renders all components in light and dark themes.
- [x] **T-0.9** Apply the design tokens from `docs/ui-stack.md` to the shadcn-vue theme; bundle Geist fonts locally.
  - Depends on: T-0.8
- [x] **T-0.10** Set up `vue-i18n` with `en.json` and `es.json`, locale detection from the OS, and a lint rule against raw strings in templates.
  - Depends on: T-0.7
- [x] **T-0.11** Create `extension/` with a Manifest V3 skeleton in TypeScript (Vite), with ESLint, Prettier and Vitest.

### CI and release

- [x] **T-0.12** Export the MVP mockup screens to `docs/assets/mockups/` and link them from `docs/ui-stack.md`.
- [x] **T-0.13** CI (`.github/workflows/ci.yml`, `windows-latest`) running `just check`, plus the jobs from T-0.17 and T-0.18. Required for merging into `main`: add it as a required status check to the `main` ruleset.
  - Depends on: T-0.16, T-0.17, T-0.18
- [x] **T-0.14** Release automation with `release-please` (workspace versioning, changelog from Conventional Commits, GitHub Releases).
  - Depends on: T-0.13
- [x] **T-0.15** Verify the product name Keitora is free (crates.io, npm, Chrome Web Store, trademarks) before the first public release, then rename the project from Bastion to Keitora: the GitHub repository, crates and binaries (`bastion-*`), the pipe `\\.\pipe\bastion`, `%ProgramData%\Bastion`, the extension, the installer and the docs. The desktop app (T-0.6) and the extension's visible name (T-0.11) are already Keitora. The images in `docs/assets/mockups/` still show Bastion: decide whether to re-export them from the private canvas or keep the exception in `docs/ui-stack.md`. Renaming the pipe and the `ProgramData` folder must keep their DACL, `PIPE_REJECT_REMOTE_CLIENTS` and SYSTEM-only ACL.

### Quality and security gates (required by `docs/workflow.md`)

- [x] **T-0.16** `justfile` with `just check` running every check in one command: fmt, clippy, Rust tests, `cargo deny`, and lint/typecheck/test for the desktop app and the extension. Exits non-zero on any failure.
  - Depends on: T-0.3, T-0.7, T-0.11
  - Acceptance: introducing a failing test, a clippy warning, or a lint error each makes `just check` fail.
- [x] **T-0.17** Coverage with `cargo-llvm-cov` (Rust) and Vitest coverage (TS); CI fails if line coverage of `keitora-core` and `keitora-ipc` drops below 90 %.
  - Depends on: T-0.16
- [x] **T-0.18** Static security analysis in CI (CodeQL for Rust and TypeScript where supported, otherwise Semgrep), GitHub secret scanning with push protection, and a check on pull-request branches that acceptance tests (`**/tests/acceptance*`) only change in commits with a `test(` prefix.
  - Depends on: T-0.16
- [x] **T-0.19** Enforce the unit test convention of `keitora-core` and `keitora-ipc` from `just check`, so the 90 % of T-0.17 keeps measuring production code. Must run before T-1.1.
  - Depends on: T-0.17
  - Acceptance: `just check` fails if either crate's non-test library compiles a file `just coverage` doesn't measure (`tests.rs`, `*_tests.rs`, a directory named `tests`, `examples` or `benches`), if `#[test]` appears outside such a file (except the inline placeholder test, until it is deleted), or if `#[cfg(not(test))]` or `cfg_attr(test` appears in those crates.

## F1 — Domain core (`keitora-core`, 100 % TDD)

- [x] **T-1.1** Data model: `Blocklist`, `Target` (`Site` | `App`), cross-platform `AppIdentifiers`, `SitePattern` exceptions.
  - Acceptance: types are `serde`-serializable; JSON round-trip tests.
  - Security: an app path is opaque text that is only compared, never opened or resolved; validation rejects UNC, device (`\\?\`, `\\.\`) and relative paths ([#11](https://github.com/CornelioDev/keitora/issues/11)). The number of targets and the length of names and patterns have defined maximums, and validation rejects values above them ([#15](https://github.com/CornelioDev/keitora/issues/15)).
- [x] **T-1.2** `Clock` trait (monotonic + wall clock) with a fake implementation for tests.
- [x] **T-1.3** Pomodoro state machine: `Idle → Focus → ShortBreak | LongBreak → Focus …` with configurable durations and cycles (default 25/5/15, 4 cycles).
  - Depends on: T-1.2
  - Acceptance: tests for every transition, pause (non-strict only), skip break, long-break boundary.
  - Security: durations and cycle counts have defined maximums, and out-of-range values are rejected ([#15](https://github.com/CornelioDev/keitora/issues/15)).
- [x] **T-1.4** Site rule matcher: domain, subdomain wildcard (`*.reddit.com`), path (`youtube.com/shorts`), exceptions taking precedence; IDN and case normalization.
  - Acceptance: table-driven tests, at least 25 cases, exported as a shared JSON fixture for the extension.
- [x] **T-1.5** App rule matcher: executable name (case-insensitive) and path; a protected-process list that is never blocked (`explorer.exe`, `csrss.exe`, `winlogon.exe`, the service itself, …).
  - Security: protected processes match by full path, so an executable renamed to a protected name in another folder isn't protected ([#8](https://github.com/CornelioDev/keitora/issues/8)). The matcher never opens or resolves a path ([#11](https://github.com/CornelioDev/keitora/issues/11)). Its result is only a candidate: the service terminates only the interactive user's own processes, enforced in T-2.6 ([#10](https://github.com/CornelioDev/keitora/issues/10)).
- [~] **T-1.6** Strict-mode policy: `LockKind` (`None`, `UntilTimerEnds`, `RandomText { len }`) and `can_stop(&session, now) -> Result<(), LockReason>`.
  - Depends on: T-1.3
  - Acceptance: an active strict session rejects stop, pause, shortening, and edits to its blocklist.
  - Security: `RandomText { len }` has a defined maximum, and validation rejects larger values ([#15](https://github.com/CornelioDev/keitora/issues/15)).
  - Only `UntilTimerEnds` is strict: the rules for strict sessions apply to it alone. `RandomText` is friction, not a lock: typing the text correctly stops the session.
  - From the T-1.3 review: today `Engine::stop` and `Engine::skip_break` still work on a strict session, and skipping the long break ends it up to 60 min early. The `Engine` itself must refuse them, not only a separate `can_stop` the service could forget to call; decide whether skipping a short break counts as shortening. The end of a session also unlocks blocklist edits ([#18](https://github.com/CornelioDev/keitora/issues/18)), so it goes through the same rule.
- [ ] **T-1.7** Serializable `EngineState` snapshot, restorable after reboot with correct remaining time.
  - Depends on: T-1.3, T-1.6
  - Security (from the T-1.3 review, ADR-0010): never persist a `MonotonicInstant`; store the active time, the segment's first cycle, the preset and the lock, and checkpoint the active time every few seconds, not only at transitions. Restoring goes through the same validation as `PomodoroPreset::new` (#15), rejects a first cycle outside 1 to `cycles` and a strict session that is paused, and never restores a session that had already ended. Time while the machine was off is never credited to a strict session from the wall clock: changing the date in the firmware must not end it ([#19](https://github.com/CornelioDev/keitora/issues/19)).
- [ ] **T-1.8** Rustdoc for every public item; crate-level docs with a usage example (`cargo doc` builds without warnings).

## F2 — Windows service (`keitora-service`)

- [ ] **T-2.1** Binary with `--console` (development) and service mode (`windows-service`); `tracing` to rotating files in `%ProgramData%\Keitora\logs`.
  - Security (from the T-1.3 review, ADR-0010): the engine gets one monotonic clock instance for the whole service, from a 64-bit source that never wraps or goes backwards (`Instant`/QPC or `QueryUnbiasedInterruptTime`, never `GetTickCount`); it never comes from a client and is never `FakeClock`. The service changes the engine only through its methods, never by replacing it with `Engine::default()`.
- [ ] **T-2.2** SQLite persistence with migrations: blocklists, settings, sessions, history, block events.
  - Depends on: T-1.1
- [ ] **T-2.3** Protocol in `keitora-ipc`: versioned requests, responses, events, typed errors; size limits.
  - Acceptance: serialization tests and version-compatibility tests.
  - Also: extend `disallowed-methods` in the root `clippy.toml` and in both `crates/keitora-core/clippy.toml` and `crates/keitora-ipc/clippy.toml` (which replace the root one for those crates; `scripts/clippy-config.spec.mjs` fails if they drift) with the panicking methods the framing code could reach and the current list misses: `Vec::split_off`, `String::split_off`, `core::iter::Sum::sum`, `core::iter::Product::product`, plus those of any buffer crate added here (e.g. `bytes`: `split_to`, `split_off`, `advance`, `get_u32*`).
- [ ] **T-2.4** Named-pipe server `\\.\pipe\keitora` with explicit DACL and `PIPE_REJECT_REMOTE_CLIENTS`; multiple clients; event subscriptions.
  - Depends on: T-2.3
- [ ] **T-2.5** Service engine: wires `keitora-core`, applies requests, emits events, persists a snapshot on every transition.
  - Depends on: T-1.7, T-2.2, T-2.4
- [ ] **T-2.6** Process monitor: detects new processes (500 ms polling with `sysinfo` in the MVP), terminates matches during focus, logs each block.
  - Depends on: T-1.5, T-2.5
  - Acceptance: a blocked app closes in under 1 s during a session; nothing is touched outside a session.
  - Security (from the T-1.5 review; the contract is in `keitora_core::app`'s docs, "What the service must do"): the image path comes from `QueryFullProcessImageNameW` with flags 0, never from the process's own memory (check where `sysinfo` reads it); if that query fails, the process isn't ended. The Windows directory comes from `GetSystemWindowsDirectoryW`, and if that fails no blocks are applied. Keitora's executables are found from the service's own module path plus fixed names, never from a client, HKCU or a manifest. A path with a `~` component on a local drive (`GetDriveTypeW`, never a network drive or UNC) is expanded with `GetLongPathNameW` and both forms are checked: blocked if either is, protected if the long form is, and protected wins; if the expansion fails, only the short form is checked and the failure is logged. Only processes whose token is the interactive user's SID, in that user's session, are ended; never `SYSTEM` or protected (PPL) processes ([#10](https://github.com/CornelioDev/keitora/issues/10)).
- [ ] **T-2.7** Restore after reboot: the service reloads the snapshot and resumes a strict session with correct timing.
  - Depends on: T-2.5
- [ ] **T-2.8** Installed and running apps endpoint for the app picker (Start Menu shortcuts + running processes, with icons where possible).
- [ ] **T-2.9** End-to-end test harness on the Windows CI runner: starts the service in `--console` mode, connects as a client over the pipe, starts a strict session, launches a dummy executable on the blocklist and asserts it is terminated; asserts stop requests are rejected while strict.
  - Depends on: T-2.6

## F3 — Desktop UI (Tauri + Vue)

- [ ] **T-3.1** `src-tauri` IPC client: Tauri commands forward to the pipe, service events are re-emitted to the frontend; TS bindings generated with `tauri-specta`.
  - Depends on: T-2.4
- [ ] **T-3.2** Data layer: `services/ipc.ts`, composables on `@tanstack/vue-query` (`useEngineState`, `useBlocklists`, `useSettings`), Pinia store for UI state; invalidate queries on service events.
- [ ] **T-3.3** App shell: shadcn-vue `Sidebar` (Timer, Blocklists, History, Settings), custom title bar, Vue Router, theme following the OS.
- [ ] **T-3.4** **Timer** page (idle): `TimerRing`, preset select, blocklist select, strict switch, Start.
- [ ] **T-3.5** **Active session** page: remaining time, phase, cycle, `StrictLockBanner`, disabled Stop with explanation, blocks this session.
- [ ] **T-3.6** Strict-session confirmation (`AlertDialog`) with summary and end time.
- [ ] **T-3.7** **Blocklists** page: list of lists; detail with Sites / Apps / Exceptions tabs; add-site form (VeeValidate + Zod, same rules as T-1.4); editing locked while in use by a strict session.
- [ ] **T-3.8** **Add application** dialog: `Command` search over T-2.8 results, plus "Browse for .exe".
- [ ] **T-3.9** **Settings** page: durations, cycles, launch at login, sounds, strict by default, browser/extension status, language, theme.
- [ ] **T-3.10** **History** page: sessions per day (shadcn-vue `Chart`), most-attempted targets.
- [ ] **T-3.11** Tray icon: remaining time in tooltip; menu Open / Stop (disabled when strict) / Quit UI; closing the window hides to tray.
- [ ] **T-3.12** Native notifications (`tauri-plugin-notification`) on phase change and when an app is blocked.
- [ ] **T-3.13** Launch at login (`tauri-plugin-autostart`) and single instance (`tauri-plugin-single-instance`).
- [ ] **T-3.14** Accessibility pass: keyboard-only walkthrough of every screen, focus order, screen-reader labels, contrast check in both themes; automated checks with `axe-core` in component tests.
- [ ] **T-3.15** Desktop end-to-end tests with WebdriverIO and `tauri-driver` on the Windows runner: start a session from the UI, see the active-session screen, confirm Stop is disabled in strict mode.
  - Depends on: T-2.9, T-3.6

## F4 — Website blocking

- [ ] **T-4.1** `keitora-nmh.exe`: stdin/stdout ⇄ named pipe bridge; native messaging manifests for Chrome and Edge; `installer/dev/register-nmh.ps1` for development.
  - Depends on: T-2.4
- [ ] **T-4.2** Extension service worker: connects to the host, receives the active blocklist, applies dynamic `declarativeNetRequest` rules (redirect to block page).
  - Depends on: T-0.11, T-4.1
- [ ] **T-4.3** Path rules and exceptions in the extension, validated against the shared fixture from T-1.4.
- [ ] **T-4.4** Block page: site, remaining time; no skip control in strict mode; localized.
- [ ] **T-4.5** Reload already-open tabs that match when a session starts.
- [ ] **T-4.6** Extension ⇄ service heartbeat: a supported browser without an active extension during a session is treated as a blocked app.
- [ ] **T-4.7** Unsupported browsers are blocked as apps during focus.

## F5 — Strict mode hardening

- [ ] **T-5.1** Browser policies in HKLM applied by the service: `ExtensionInstallForcelist`, `IncognitoModeAvailability` (Chrome), `InPrivateModeAvailability` (Edge).
- [ ] **T-5.2** Service recovery actions (restart on failure) and automatic start.
- [ ] **T-5.3** ACL on `%ProgramData%\Keitora`: write for `SYSTEM` only.
- [ ] **T-5.4** Clock-change resistance: remaining time uses the monotonic clock; integration test.
- [ ] **T-5.5** Uninstall guard: the MSI queries the service and aborts while a strict session is active.
- [ ] **T-5.6** Optional (off by default): block Task Manager and service management tools during strict sessions.
- [ ] **T-5.7** Documented manual bypass test checklist in `docs/strict-mode-tests.md`.
- [ ] **T-5.8** Review the service against `docs/threat-model.md` (privilege boundary section) and record findings.
- [ ] **T-5.9** Fuzzing with `cargo-fuzz` for the IPC parser and the native messaging framing; a short fuzz run in CI on every PR touching `keitora-ipc` or `keitora-nmh`, longer runs nightly.

## F6 — Packaging and distribution

- [ ] **T-6.1** MSI (WiX v4): installs UI, service, NMH; registers the service and native messaging manifests.
- [ ] **T-6.2** Code signing for binaries and MSI (e.g. SignPath Foundation's free program for open-source projects).
- [ ] **T-6.3** Publish the extension to the Chrome Web Store and Edge Add-ons (required for `ExtensionInstallForcelist`).
- [ ] **T-6.4** UI updates with `tauri-plugin-updater`; the service updates through the MSI.
- [ ] **T-6.5** Release workflow: build on tag, sign, attach artifacts, generate build provenance attestations.
- [ ] **T-6.6** Smoke test on clean Windows 10 and Windows 11 VMs using a checklist.
- [ ] **T-6.7** User documentation: install, first session, uninstall, FAQ (including the "administrator can bypass" limitation).
- [ ] **T-6.8** Human security review of `keitora-service`, `keitora-ipc`, `keitora-nmh` and the installer (experienced contributor or paid audit). Required before any stable release; see the release gate in `docs/workflow.md`.

**🎯 MVP milestone:** F0–F6 complete → `v0.1.0-beta.1`. Stable `v0.1.0` after T-6.8.

---

## F7 — Sync (post-MVP, opt-in)

- [ ] **T-7.1** Self-hostable backend (PocketBase or Supabase): `blocklists`, `targets`, `sessions`, `devices`. Write an ADR first.
- [ ] **T-7.2** Accounts and device linking.
- [ ] **T-7.3** Blocklist sync (sites universal; apps by platform identifier).
- [ ] **T-7.4** Shared sessions: starting on one device starts on all (real time).

## F8 — Android (post-MVP)

- [ ] **T-8.1** ADR: Tauri 2 mobile + Kotlin plugins vs native Kotlin app.
- [ ] **T-8.2** Expose `keitora-core` to Kotlin with `uniffi`.
- [ ] **T-8.3** Site blocking with a local `VpnService` (DNS filter).
- [ ] **T-8.4** App blocking with `UsageStatsManager` + overlay.
- [ ] **T-8.5** Strict mode with Device Administrator.

## F9 — iOS (post-MVP)

- [ ] **T-9.1** Request the Family Controls entitlement from Apple.
- [ ] **T-9.2** Integrate `FamilyControls` / `ManagedSettings` / `DeviceActivity`; device-local app selection via `FamilyActivityPicker`.
- [ ] **T-9.3** Sync domains only; shield extension in Swift.
