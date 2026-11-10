# Contributing to Keitora

Thanks for your interest! This guide covers how to set up the project, how we work, and what we expect from a pull request.

By participating you agree to follow the [Code of Conduct](CODE_OF_CONDUCT.md).

## Ways to contribute

- **Report a bug** or **suggest a feature** with the issue forms.
- **Report a strict-mode bypass** with the "Strict mode bypass" form — these are feature limitations, discussed in public.
- **Report a security vulnerability** privately — see [SECURITY.md](SECURITY.md). Never open a public issue for these.
- **Improve translations** in `apps/desktop/src/locales/` and `extension/src/_locales/`.
- **Pick up an issue**: `good first issue` and `help wanted` are good starting points. Comment on the issue before starting so work isn't duplicated.

For larger changes (new features, architectural changes), open an issue first. If it changes the architecture, it will need an [ADR](docs/adr/0000-template.md).

## Development setup

Requirements (Windows 10 or 11):

- [Rust](https://rustup.rs) — the toolchain is pinned in `rust-toolchain.toml`.
- [Node.js](https://nodejs.org) LTS and [pnpm](https://pnpm.io), the version in `packageManager` in each package's `package.json`.
- [Tauri prerequisites](https://v2.tauri.app/start/prerequisites/) (WebView2, MSVC build tools).
- [`just`](https://github.com/casey/just) 1.59.0, [`cargo-deny`](https://github.com/EmbarkStudios/cargo-deny) 0.20.2 and [`lefthook`](https://github.com/evilmartians/lefthook) 2.2.2.
- [`cargo-llvm-cov`](https://github.com/taiki-e/cargo-llvm-cov) 0.9.1, only for `just coverage`: `cargo install cargo-llvm-cov --locked --version 0.9.1`. The `llvm-tools` component it needs comes with the pinned toolchain.

CI installs these same versions of `just`, `cargo-deny` and `cargo-llvm-cov` ([`.github/workflows/ci.yml`](.github/workflows/ci.yml)); change them there and here together. CI doesn't use `lefthook`, so this list is what pins its version.

```bash
git clone https://github.com/CornelioDev/keitora.git
cd keitora
lefthook install
pnpm install --dir apps/desktop
pnpm install --dir extension

# Terminal 1: the service, without installing it (an elevated terminal is needed to block processes)
cargo run -p keitora-service -- --console

# Terminal 2: the desktop app
pnpm --dir apps/desktop tauri dev
```

To run the main checks, from any terminal (see [`justfile`](justfile); `just --list` shows the recipes for running a part of them; [what CI runs](#what-ci-runs) lists the rest):

```bash
just check
```

It stops at the first check that fails and exits with an error. It installs the desktop app's and the extension's dependencies from their lockfiles first, and fails if a lockfile doesn't match its `package.json`, or if `Cargo.lock` is missing or doesn't match the `Cargo.toml` files. After changing a dependency, run `pnpm install` in that package, or `cargo check` for a Rust one, and commit the updated lockfile.

To measure test coverage, which `just check` does not do:

```bash
just coverage
```

It fails if the tests cover less than 90 % of the lines of `keitora-core` or of `keitora-ipc`, each crate counted on its own, with all features on. So that the 90 % measures production code only, the unit tests of these two crates live in a separate file, declared with `#[cfg(test)] mod tests;` (`src/engine.rs` with `src/engine/tests.rs`); lines of a test module inside a source file would count as covered code ([AGENTS.md](AGENTS.md#writing-rust-tests)).

What the gate doesn't measure:

- Files named `tests.rs` or `*_tests.rs` (`*-tests.rs` as well), and any directory named `tests`, `examples` or `benches`, at any depth. Those names must hold only test code, never product code, or that code goes unmeasured.
- Code that doesn't compile on Windows, such as `#[cfg(unix)]`.
- Code under `#[cfg(not(test))]`, which is compiled only outside tests. That includes a real function that a test double replaces during tests.
- Code under `#[cfg(not(feature = "..."))]`, which isn't compiled with `--all-features`. Code under `#[cfg(feature = "...")]` is measured.

A crate with no functions at all fails with "no coverage data found", so `keitora-ipc` has one inline placeholder test, the only inline test allowed in it, until it has real code. `keitora-core` had one too, until T-1.1 gave it real code.

`just check` enforces this layout in those two crates (`just unit-test-layout`, [`scripts/check-unit-test-layout.mjs`](scripts/check-unit-test-layout.mjs)). It exits with 0 when they follow it, with 1 when they don't (each violation is listed), and with 2 when it can't tell, which is never a pass. It is a guard against accidents and ordinary code, not a sandbox. It fails if:

- The manifest has a target other than the library and integration tests (so no build script, binary, example or bench), a direct proc-macro dependency, or a local dependency (by path or `[patch]`) other than `keitora-ipc` on `keitora-core`, or if another workspace member lives inside the crate's directory. A problem here in either crate stops both builds, as they would run that code.
- The library, built in the dev profile with all features, reads a file with one of the names above (`rustc/<hash or version>/` as well, which cargo-llvm-cov also leaves out) or a file outside the crate's directory. This comes from what the compiler reports having read, so `#[path]`, `include!` and macros that expand in that build don't get around it.
- A file only the tests read is one coverage measures. Its lines would add to the score, so a test helper has to be named `tests.rs` or `*_tests.rs`, or be in a directory named `tests`.
- A build reads a file that is neither Rust nor data (`.json`, `.md`, `.toml`) under a name coverage measures, or a written `#[path]` or `include!` names a file that isn't `.rs`, or a `use` renames `include`. What a macro assembles is listed below.
- `[profile.test]` makes optimization, debug info, debug assertions or overflow checks differ from dev.
- A test attribute (`#[test]` and its longer paths, `#[bench]`, `#[test_case]`, any other with `test` in its name) or a `use` of one appears outside a file named `tests.rs` or `*_tests.rs`, or a directory named `tests`, `examples` or `benches`. The placeholder test is the exception, exactly as it is, in the `src/lib.rs` of a crate listed in `PLACEHOLDER_CRATES` ([`scripts/unit-test-layout-rules.mjs`](scripts/unit-test-layout-rules.mjs)), now only `keitora-ipc`.
- Anywhere in the crate's directory, test files included, leaving out only a `target` directory directly in it: `#[cfg(not(test))]` or `cfg_attr(test` in any form (`cfg(any(not(test)))`, `cfg![not(test)]`), a `cfg` on `debug_assertions` or on cargo-llvm-cov's own `coverage`, a `$` inside the brackets of a written `cfg`, or `cfg_select!`, `cfg_match!` and `cfg_if!`.
- `#[cfg(test)]` or `cfg!(test)` appears in a measured file anywhere but directly before `mod tests;`, with no other attribute next to it. A test helper goes in the test file.

So `#[cfg(any(test, feature = "..."))]` is rejected: to share fakes with integration tests or other crates, make it an unconditional `pub` module, such as `pub mod testing;`, whose lines are measured.

Review is still needed for what this can't see:

- What a macro assembles: `#[$name]` called with `test`, `use core::prelude::v1::$name as t;` followed by `#[t]`, `mk!(include)` expanding to `$name!("a.json")`, `#[path = $file]` naming a data file, and a `cfg` whose own name is an argument, as in `#[$c(not($t))]` or `when!(cfg, not(test), ...)`. A `$` is only rejected inside the brackets of a written `cfg`, so that family is not closed.
- Logic in a `#[macro_export]` macro that only other crates expand, which is unmeasured.
- Padding: trivially covered lines in an unconditional `pub` module still add to the score.
- Test macros from a dependency, and what a dependency's macros expand to. A proc macro reached through a dependency is allowed, as `serde` and `thiserror` need one.
- Code the checked builds don't compile, whatever its name: under `#[cfg(not(feature = "..."))]`, `#[cfg(unix)]`, `target_feature` or `panic`.
- A `cfg` set from outside the crate, through the environment or `.cargo/config.toml` (`RUSTFLAGS`, `rustc-wrapper`).
- Product code moved to another crate, which `-p` doesn't measure.
- A pull request that changes the script itself ([what CI proves](#what-ci-proves)).

The report for the desktop app and the extension has no minimum; open `apps/desktop/coverage/index.html` or `extension/coverage/index.html` for details.

On Windows without [long paths](https://learn.microsoft.com/windows/win32/fileio/maximum-file-path-limitation) enabled, keep the clone's root path at 133 characters or fewer (measured on 2026-11-09). From 134 on, the desktop app's Vitest fails with `ERR_PACKAGE_IMPORT_NOT_DEFINED` and `"#module-evaluator"`, because the paths inside `node_modules` go over the limit. From 147 on, the unit test layout check also fails closed: `link.exe` can't write the build scripts of the ICU crates, which `url` needs, under `target/unit-test-layout`.

While the app has no screens yet, a development build (`tauri dev`, or `pnpm --dir apps/desktop dev` in a browser) opens the component gallery, [`apps/desktop/src/dev/ComponentGallery.vue`](apps/desktop/src/dev/ComponentGallery.vue): every shadcn-vue component the app uses, with a button that switches between the light and dark themes. Production builds leave the gallery out.

### Adding shadcn-vue components

The UI components in `apps/desktop/src/components/ui/` are generated by the [shadcn-vue](https://www.shadcn-vue.com) CLI, configured by `apps/desktop/components.json`, and committed exactly as the CLI writes them. The CLI is not a dependency of the app; run the version the existing components were generated with, **2.9.0**, from `apps/desktop`:

```bash
pnpm dlx shadcn-vue@2.9.0 add <name>
```

Then:

1. Remove the Google Fonts `@import` that the CLI puts at the top of `src/styles/main.css` every time it runs. The app makes no network requests at runtime and bundles Geist itself (`src/styles/fonts.css`); `pnpm test` fails while the import is there.
2. Check what the CLI changed in `src/styles/main.css` (`git diff`). Keitora's design tokens live in `src/styles/tokens.css`, and anything the CLI writes into `main.css` overrides them. Keep the `--color-…: var(--…)` lines it adds to `@theme inline`, but keep Keitora's `--radius-sm` … `--radius-4xl` lines there, offsets from `--radius` ([docs/ui-stack.md](docs/ui-stack.md#design-tokens)), if it rewrites them as multiples of it; `pnpm test` fails when they change. If it wrote `:root` or `.dark` blocks, or a `--font-sans`, delete them, and give any variable that is new in them a value in `tokens.css` for both themes, following that file's top comment (contrast included). `pnpm test` fails when a token from [docs/ui-stack.md](docs/ui-stack.md#design-tokens) or a font changes.
3. Check what the CLI changed in `package.json`, and remove the packages the app does not use. `form`, for example, adds `zod` and `@vee-validate/zod`, which are left out until a Zod release without known advisories works with VeeValidate (`pnpm audit`).
4. Install any package the new component imports that the CLI did not add (it does not add `@unovis/vue` and `@unovis/ts` for `chart`, for example), and check that `pnpm typecheck`, `pnpm lint` and `pnpm test` pass.
5. Add the component to the gallery if the app uses it.

To move to a newer CLI version, regenerate every component with it (`add <names...> --overwrite`), replace the body of `src/styles/shadcn-vue.css` with that version's `dist/tailwind.css` from the `shadcn-vue` package, and update the version here and in that file's header.

To try the extension, load `extension/dist` as an unpacked extension in Chrome or Edge, then register the development native messaging host with `installer/dev/register-nmh.ps1`.

### Adding text to the app

Every piece of text that the desktop app shows or reads out to people goes through [vue-i18n](https://vue-i18n.intlify.dev) ([ADR-0007](docs/adr/0007-english-project-language-localized-ui.md)); none is written directly in a component. The messages live in `apps/desktop/src/locales/`: `en.json` (English, also used for any message missing in another language) and `es.json` (Spanish). The app starts in the Windows display language when it is one of these, and in English otherwise.

To add a piece of text:

1. Add a message with the same key to **both** files, grouped by screen or feature, such as `timer.start` in `{ "timer": { "start": "Start focus" } }`. Write the Spanish in neutral Spanish, not that of one country. Placeholders (`{count}`) and plurals (`one | many`) follow vue-i18n's [message syntax](https://vue-i18n.intlify.dev/guide/essentials/syntax.html), in which `{`, `}`, `@`, `$` and `|` are special; write `{'@'}` to show one of them as is.
2. Use it through `t()` in the component, in text and in bound attributes:

   ```vue
   <script setup lang="ts">
   import { useI18n } from "vue-i18n";

   const { t } = useI18n();
   </script>

   <template>
     <Button :title="t('timer.startHint')">{{ t("timer.start") }}</Button>
   </template>
   ```

3. Check: `pnpm typecheck` fails when `es.json` is missing a message of `en.json`, and `pnpm test` fails when the two files have different keys or the code uses a key that is not in them.

`pnpm lint` (and the commit hook) reports text written directly in a template as an error: text in an element, a string shown with `{{ "…" }}`, and the value of attributes that people read or hear, such as `title`, `aria-label`, `placeholder` and `alt` (the full list is in `apps/desktop/eslint.config.ts`). Text made only of whitespace and separators (`: / · • - – — | ( )`, `&nbsp;`) is allowed. The rule cannot see a string literal in a bound attribute (`:title="'Stop'"`) or in a script, such as the text of a toast; those go through `t()` too.

Two folders are exempt from that rule, and only from it:

- `src/dev/`: the component gallery, which is not user-facing and is left out of production builds.
- `src/components/ui/`: the components generated by the shadcn-vue CLI, kept as it writes them. A few of them have English text for screen readers, such as "Close" in `dialog/` and `sheet/`, the default title of `command/CommandDialog.vue`, and "Toggle Sidebar" and the mobile sidebar's title in `sidebar/`. Before a screen uses one of these, its text has to be made translatable.

To see the app in another language in a development build, start it with the language that WebView2 should report, for example `WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS=--lang=es pnpm --dir apps/desktop tauri dev` in Git Bash (`$env:WEBVIEW2_ADDITIONAL_BROWSER_ARGUMENTS = "--lang=es"` first, in PowerShell). Close every window of the app first: WebView2 ignores the option while another window of the app is open.

## How we work

- **Planning** lives in [docs/roadmap.md](docs/roadmap.md). Tasks have IDs (`T-1.3`); phases map to GitHub milestones. [ROADMAP.md](ROADMAP.md) is the short public summary, one line per phase.
- **Architecture** is described in [docs/architecture.md](docs/architecture.md); decisions are recorded in [docs/adr/](docs/adr/).
- **Conventions** for code, naming and tests are in [AGENTS.md](AGENTS.md#conventions) and apply to humans too.

## Commits

We use [Conventional Commits](https://www.conventionalcommits.org):

```
feat(core): add pomodoro state machine (T-1.3)
fix(service): restore session after reboot when clock moved back
docs: explain native messaging setup
```

Types: `feat`, `fix`, `docs`, `refactor`, `test`, `perf`, `build`, `ci`, `chore`. Scopes: `core`, `ipc`, `service`, `nmh`, `desktop`, `extension`, `installer`, `deps`; `docs` commits may instead name the document they change, such as `roadmap`, `workflow` or `agents`. Scopes are optional and lowercase.

The Git hooks that `lefthook install` sets up check this format when you commit, and check the formatting of the files you stage (see [`lefthook.yml`](lefthook.yml)). The hooks run code from the repository (`lefthook.yml`, `scripts/`, and each package's ESLint and Prettier configs and `node_modules`), so review those before committing on a branch from someone you don't trust.

Every commit must be signed off under the [Developer Certificate of Origin](https://developercertificate.org):

```bash
git commit -s -m "feat(core): …"
```

The `DCO` app doesn't require a sign-off from a commit attributed to a bot, such as the one release-please makes on its own branch, which whoever merges it signs off ([Releases](#releases)). GitHub attributes a commit through its author email, so anyone can write a commit attributed to a bot, and the check passes it unsigned. Look at who a pull request's commits are attributed to with `gh pr view <number> --json commits`.

## Pull requests

1. Fork and create a branch from `main` (`feat/blocklist-exceptions`, `fix/pipe-reconnect`).
2. Keep PRs focused: one logical change.
3. Add or update tests. Logic in `keitora-core` and `keitora-ipc` is developed test-first.
4. Run `just check` locally; CI runs it and [more](#what-ci-runs). Don't modify acceptance tests (`**/tests/acceptance*`) in a feature PR; propose changes to them in a separate PR, in commits whose subject starts with `test(`, which is what [`just check-acceptance-tests`](#acceptance-tests-change-only-in-test-commits) checks.
5. Update docs and the roadmap when relevant. `CHANGELOG.md` and version numbers are generated: see [Releases](#releases).
6. Fill in the PR template. A maintainer squash-merges the PR once the required checks pass.

`main` only changes through pull requests: its ruleset has a `pull_request` rule that requires one, with squash merges only and no approvals, as a single maintainer can't approve their own, plus the required status checks. The ruleset's bypass list is empty, so this applies to the maintainer too. The squash keeps the two things a commit on `main` needs:

- **Conventional Commits subject.** The PR title becomes the commit's subject, followed by the ` (#N)` that GitHub appends, so the title must follow Conventional Commits and name the task when there is one: `feat(core): add pomodoro state machine (T-1.3)` becomes `feat(core): add pomodoro state machine (T-1.3) (#12)`. No check reads the title, so whoever merges has to look at it.
- **DCO sign-off.** The DCO app checks the PR's commits. Their `Signed-off-by` lines reach `main` only because the repository's `squash_merge_commit_message` setting is `COMMIT_MESSAGES`, which copies the commits' messages into the squash commit. Changing that setting silently loses the sign-off, and so does rewriting the message when merging without keeping such a line.

Before merging, ask GitHub what the squash commit will say, which changes nothing. The headline must be the Conventional Commits subject and the body must have the `Signed-off-by` lines:

```bash
gh api graphql -F number=<number> -f query='query($number: Int!) { repository(owner: "CornelioDev", name: "keitora") { pullRequest(number: $number) { viewerMergeHeadlineText(mergeType: SQUASH) viewerMergeBodyText(mergeType: SQUASH) } } }'
```

Then merge from the command line, with the address you sign off with, so that GitHub doesn't pick another address of your account as the author:

```bash
gh pr merge <number> --squash --delete-branch --author-email <email>
```

### What CI runs

[`.github/workflows/ci.yml`](.github/workflows/ci.yml) and [`codeql.yml`](.github/workflows/codeql.yml) run on every pull request to `main` and every push to it, on `windows-latest`. `just check` doesn't run everything CI does:

| Check | What it does | Locally |
|---|---|---|
| `just check` | Formatting, lints, unit test layout, tests, the Rust docs (`cargo doc` with warnings as errors), `cargo deny`, the frontend packages, and the tests of the scripts | `just check` |
| `just coverage` | 90 % of the lines of `keitora-core` and `keitora-ipc`; a report for the frontend packages | `just coverage` |
| `npm dependencies` | `pnpm audit` of every npm dependency, development ones included | `just npm-dependencies` |
| `Acceptance tests` | [The rule below](#acceptance-tests-change-only-in-test-commits); pull requests only | `just check-acceptance-tests origin/main` |
| `Analyze (rust)`, `Analyze (javascript-typescript)`, `Upload (…)` | [CodeQL](#security-checks) | Nothing to run |

`npm dependencies` isn't part of `just check` because a new advisory can fail it with no change in the repository. `just npm-dependencies` runs `just npm-audit`, which exits with 0 when there are no advisories of severity low or above, 1 when there are (each is listed), and 2 when it can't tell, such as with the registry down, which is never a pass. It can't tell a registry that wrongly replies `{}` from one with no advisories, because that is the normal reply, and pnpm silently drops an advisory whose severity is missing or unknown.

**npm licenses are not checked by CI.** The npm license check is off as a gate during the MVP, by the maintainer's decision (2026-11-02), and must be analyzed before the first public release. Nothing stops a dependency with any license from coming in meanwhile. `just npm-licenses` runs it by hand, with the policy in [`scripts/npm-license-policy.mjs`](scripts/npm-license-policy.mjs), and it fails today on `elkjs` (EPL-2.0). Its result relies on what each package declares in its `package.json`; nothing reads the license texts. Rust licenses are still enforced, by `cargo deny` in `just check`.

[`.github/workflows/release-please.yml`](.github/workflows/release-please.yml) also runs on every push to `main`, and is not a required check: it can fail without blocking a merge ([Releases](#releases)).

The checks in the table, with the `DCO` app's, are required to merge, and a branch must be up to date with `main`. So a new advisory can block every merge until a pull request updates the package or ignores the advisory by ID. To ignore one, add its GHSA ID to `auditConfig.ignoreGhsas` in the `pnpm-workspace.yaml` of the project it affects, with a reason and a review date next to it, and to the list in [`scripts/check-npm-audit.spec.mjs`](scripts/check-npm-audit.spec.mjs). No tool enforces the review date, so it needs someone to look at it.

### What CI proves

On a pull request, GitHub runs the workflows, the `justfile` and the scripts as that pull request has them. A pull request that edits any of them can make a check pass, or remove it, so CI proves little about such a change, and **a human has to catch it in review**. Look closely at any change to `.github/`, `justfile`, `scripts/`, `deny.toml`, `rust-toolchain.toml`, `Cargo.toml`, `clippy.toml`, `.cargo/config.toml`, any `build.rs`, `pnpm-workspace.yaml`, `.pnpmfile.cjs`, `.npmrc`, the lockfiles (`Cargo.lock`, `pnpm-lock.yaml`), the package configs and `release-please-config.json`.

- A required check is satisfied by its name. A pull request can keep `name: just check` with `run: exit 0`, or add a workflow with a job of that name, and the check passes.
- A job skipped with `if: false` counts as passed.
- A pull request from a fork needs the maintainer's approval before its workflows run, so read its `.github/` diff before approving.
- With one maintainer no review is required, so `CODEOWNERS` blocks nothing.
- `Acceptance tests` runs the script from the base branch, with no file of the pull request on disk, so editing the script or the `justfile` doesn't change its result. Editing `ci.yml` still does.
- Text can steer a release, with no file changed ([Releases](#what-decides-the-version)). Read the commit messages and the description of a pull request from someone else for that text, even inside an HTML comment. Dependabot's count, as they carry third-party text such as release notes. Reviewing descriptions is what catches `BEGIN_COMMIT_OVERRIDE`; the release pull request doesn't show what it removes.
- `scripts/check-unit-test-layout.mjs` and `scripts/unit-test-layout-rules.mjs` stay as the pull request has them: they catch accidents and ordinary code, not someone working around them. Running the base's copy wouldn't change that, as it would still build and read the pull request's code, in a job that the `justfile` and `ci.yml` of the pull request define.

### Acceptance tests change only in `test(` commits

Acceptance tests are the contract for a task ([docs/workflow.md](docs/workflow.md#acceptance-tests-are-protected)), so every commit of a pull request has to follow that rule. CI checks this on pull requests. To run it yourself, with Git 2.43 or newer, on a clone with its whole history:

```bash
just check-acceptance-tests origin/main
```

It exits with 0 when every commit in `origin/main..HEAD` follows the rule, 1 when one doesn't (each is listed), and 2 when it can't tell: an unknown base, a shallow clone, no commits since the base, a base with no history in common, or a Git error. It isn't part of `just check`, which has no base to compare to.

The rule, for files under `**/tests/acceptance*` in any letter case, added, modified, deleted, or renamed into or out of those paths:

- The commit's subject must start with exactly `test(`. `test: x`, `Test(x)`, `fix: test(x)`, `fixup! test(x)` and `Revert "test(x)"` don't pass; squash fixups first, and give a revert a `test(` subject.
- That commit must change nothing else, so that product code can't arrive under a `test(` subject. Put a dependency or configuration change that a test needs in its own commit.
- A merge only counts for what was done by hand. Merging `main` into a branch passes, even if it brings in changes to acceptance tests. A merge that edits an acceptance test, resolves a conflict in one whichever side it keeps, or throws away one side's change (`git merge -s ours`) is treated as a commit changing that test: rebase instead, or give the merge a `test(` subject. The merge is judged without the repository's `.gitattributes`, so a merge that relies on a merge attribute for an acceptance test counts too. A merge of more than two branches counts for every difference from any parent.

This catches accidental changes, mostly by agents. It doesn't prove who wrote a commit: anyone can type `test(`, and a pull request can [change the workflow that runs it](#what-ci-proves). Human review remains the control.

### Security checks

- **CodeQL** ([`.github/workflows/codeql.yml`](.github/workflows/codeql.yml)) analyzes the Rust and TypeScript/JavaScript code on every pull request, on every push to `main` and weekly. Results appear under the repository's Security tab and on the pull request. A pull request from a fork or from Dependabot may have a token that can't upload them; its check still passes, and its results stay in the run's `sarif-*` artifacts until the code is analyzed again on `main`. The job that runs CodeQL, and with it the build scripts and proc macros of a pull request, has no permission to write results; a second job with no checkout uploads them. The required CodeQL checks prove that the analysis ran, not that its results are clean, so alerts are reviewed by hand. It only runs on GitHub, so there is nothing to run locally. The repository's "default setup" for code scanning must stay off, because it conflicts with this workflow.
- **Secret scanning with push protection** is on for the repository; see [SECURITY.md](SECURITY.md#secret-scanning).
- **GitHub Actions are restricted** in the repository's settings. A `uses:` that isn't pinned to a full commit hash fails. Only GitHub's own actions and a list of third-party ones can run (`taiki-e/install-action`, `pnpm/action-setup`, `googleapis/release-please-action`), so a new third-party action must be added to that list first.

## Releases

[release-please](https://github.com/googleapis/release-please) prepares releases from the commits on `main` ([`.github/workflows/release-please.yml`](.github/workflows/release-please.yml), [`release-please-config.json`](release-please-config.json)). Nobody writes `CHANGELOG.md` or changes a version number by hand.

1. On every push to `main`, it reads the commits since the last release.
2. If they include something to list in the changelog, it opens or updates one pull request, `chore: release <version>`, from the branch `release-please--branches--main`. It holds the new version and the changelog entry, and runs the same required checks as any other.
3. **That pull request stays open until the maintainer decides to release, and only the maintainer merges it.** This is a rule we follow, not something enforced: the ruleset requires no approval, and the App's token (see [below](#what-the-repository-needs)) can merge it, create tags, even on a commit outside `main`, and publish or edit releases. The maintainer accepted the merge risk until T-6.5. The `release tags` ruleset, with no bypass, lets tags matching `v*` be created, but nobody can move or delete them. Merging it tags the commit `v<version>` and publishes a GitHub Release, marked as a pre-release while the version is a beta. No binaries yet: T-6.5 adds them.

Until F6 is done, leave it open. The MVP's version is held back only by not merging it; `0.1.0-beta.1` doesn't change however many commits arrive before then.

### What decides the version

Normally, the type in the pull request's title, which becomes the squash commit's subject:

| Type | In the changelog | Version |
|---|---|---|
| `feat` | Added | Next beta: `0.1.0-beta.1` → `0.1.0-beta.2` |
| `fix` | Fixed | Next beta |
| `perf`, `revert` | Changed | Next beta |
| `feat!`, `fix!` or a `BREAKING CHANGE` footer | Breaking changes | Next beta |
| `docs`, `chore`, `ci`, `build`, `test`, `refactor` | Not listed | None, and no release on their own |

So if users should hear about a change, the title has to say so: a `feat` or `fix` commit under a title of another type is normally not listed. The first release is `0.1.0-beta.1` (`initial-version`), and it also lists the earlier commits on `main` with a `feat` or `fix` subject.

Text inside commit messages and pull request descriptions can also change the version or the list: `Release-As:`, `BREAKING CHANGE`, `BEGIN_NESTED_COMMIT`, `BEGIN_COMMIT_OVERRIDE`, or a paragraph starting with a type. `Release-As:` can even produce a version with no `-beta.n`. The exact rules are release-please's, so don't assume such text is harmless wherever it sits ([what CI proves](#what-ci-proves)).

To go from a beta to a stable version, in this order:

1. Publish the beta, by merging its release pull request.
2. Pass the human security review (T-6.8, [release gate](docs/workflow.md#release-gate)).
3. In a pull request, set `"prerelease": false` in the config. With a `feat`, `fix`, `perf` or `revert` commit since the beta, the release pull request proposes the same version without `-beta.n`. With none, no release pull request opens, so add the footer `Release-As: 0.1.0`, with the version to release, on a single-commit branch, or read the squash message first with `viewerMergeBodyText` ([Pull requests](#pull-requests)). After merging, check that the release pull request proposes that version.

Don't change that setting before a beta is published: it would still propose `0.1.0-beta.1`, and mark that beta as not a pre-release.

### What has the version

- Set by release-please: `Cargo.toml` (`[workspace.package]`, which every crate inherits), the workspace's own entries in `Cargo.lock`, and `apps/desktop/package.json` and `extension/package.json`. `.release-please-manifest.json` records the last released version. `pnpm-lock.yaml` has no version of the project, so `--locked` and `--frozen-lockfile` keep passing.
- Derived: `tauri.conf.json` has no version, so the app takes it from `Cargo.toml`. The extension's build works out the manifest's version from `extension/package.json`, because browsers only accept numbers there ([`extension/src/manifest-version.ts`](extension/src/manifest-version.ts)): `0.1.0-beta.1` becomes `0.1.0.1`, and a final `0.1.0` becomes `0.1.0.65535`, which keeps every update higher than the one before. The `version` in `extension/src/manifest.json` never ships.

### Merging the release pull request

Like any other, it has to pass the required checks and be up to date with `main`. release-please rebuilds it on top of `main` after any merge that changes its text. Its text has the date, so the first merge of a new day counts, whatever its type. Each rebuild runs all the required checks again: about 30 runner minutes, which a public repository doesn't pay for, and about 17 minutes of waiting. While it is left open this is only noise, as no other pull request waits for it.

A merge that doesn't change its text leaves it behind `main`, so bring it up to date just before releasing:

```bash
gh pr update-branch <number> --rebase
```

If that hits a conflict, delete the branch `release-please--branches--main` and re-run the last run of the workflow on `main`, which opens it again on top of `main`.

Before merging:

1. Confirm that the version in the title has `-beta.`, while the [release gate](docs/workflow.md#release-gate) holds, and that it is the next one you expect: a jump can keep `-beta.`.
2. Read the changelog entry in full, and compare it with `git log` since the last release: reading it shows what was added, not what was removed. Changelog and release text isn't sanitized, so treat it as untrusted when reading and when using it. T-6.5 must never interpolate release or tag data into a `run:` step of a workflow.
3. Check who its commits are attributed to, with `gh pr view <number> --json commits`: one commit, from the App's bot. The `DCO` app skips commits [attributed to a bot](https://github.com/dcoapp/app/blob/main/lib/dco.js), and that attribution comes from the commit's author email, so a passing check doesn't prove who wrote it.

Then merge as usual, with one difference. The release pull request passes the `DCO` check with no sign-off, so there is none for GitHub to copy into the squash commit. Add yours when merging, because every commit on `main` has one:

```bash
gh pr merge <number> --squash --delete-branch --author-email <email> --body "Signed-off-by: <name> <email>"
```

### What the repository needs

Do this before merging the workflow into `main`, and again only if the key is replaced. Pull requests opened with `GITHUB_TOKEN` trigger no workflows, so a release pull request would wait forever for its checks; a GitHub App's token does trigger them. It doesn't bypass the `main` ruleset, so anything it merged would still need the required checks, and nothing needs to be added to the ruleset's bypass list.

1. **Create the App** under your account's Settings → Developer settings → GitHub Apps → New GitHub App. Give it a name that doesn't depend on the repository's. Set the homepage to the repository's URL, turn the webhook off, and allow installation only on this account. It needs these repository permissions, and no others:
   - **Contents: Read and write**, to create the release branch and its commit, and to publish the release and its tag.
   - **Pull requests: Read and write**, to open and update the release pull request, and for its labels (`autorelease: pending`, `autorelease: tagged`) and its comment.
   - **Metadata: Read-only**, which GitHub adds on its own.

   If a run fails on labels or comments because `Issues` turns out to be needed, add **Issues: Read and write** to the App and `permission-issues: write` to the workflow.

2. **Generate a private key** on the App's page, and note its **Client ID** (not the App ID, which `create-github-app-token` no longer recommends).
3. **Install the App** on **only** this repository.
4. **Create an environment** named `release-please` in the repository's Settings → Environments. Under "Deployment branches and tags", choose "Selected branches and tags" and add `main`, so that only `main` can read what it holds.
5. **In that environment**, add the variable `RELEASE_APP_CLIENT_ID` with the Client ID and the secret `RELEASE_APP_PRIVATE_KEY` with the whole contents of the key file. Then delete the file.

**Check the environment before merging the workflow, or any change to it.** This is mandatory: if the environment doesn't exist when the workflow first runs, GitHub creates it with no branch rule, and a key added to it later is readable from any branch. These two commands must show `custom_branch_policies: true`, and one entry, `main`, of type `branch`:

```bash
gh api repos/CornelioDev/keitora/environments/release-please --jq .deployment_branch_policy
gh api repos/CornelioDev/keitora/environments/release-please/deployment-branch-policies --jq '.branch_policies[]|{name,type}'
```

If the variable or secret is missing, the run fails at "Get a token from the GitHub App", with nothing published. No check requires this workflow, so merges carry on; run it again from the Actions tab once it is fixed.

To rotate the key, which is also the first step if it may have leaked: generate a new one on the App's page, replace the secret `RELEASE_APP_PRIVATE_KEY`, and delete the old key from the App's page.

No required check runs this workflow, so a pull request that updates `release-please-action`, Dependabot's included, proves nothing about it by passing. It receives a token that can write to the repository, so read the diff of the action's `dist/` before merging one.

If a run fails with `refusing to allow a GitHub App to create or update workflow`, GitHub rejected rebuilding the release branch on a `main` whose `.github/workflows/` has changed since. Delete the branch `release-please--branches--main` and re-run the failed job: a new branch has nothing to compare against. Don't grant the App the `workflows` permission to avoid this without a security review: it would turn writing code into controlling CI and its secrets.

## Labels

| Label | Meaning |
|---|---|
| `bug`, `enhancement`, `docs` | Issue type |
| `strict-mode` | Bypasses and hardening of strict mode |
| `security` | Used by maintainers only, after private triage |
| `area:core`, `area:service`, `area:desktop`, `area:extension`, `area:installer` | Component |
| `good first issue`, `help wanted` | Contributor-friendly |
| `needs-adr` | Requires an architecture decision before implementation |

## Using AI assistants

AI-assisted contributions are welcome. You are responsible for every line you submit: review it, test it, and make sure you can explain it. Agents should follow [AGENTS.md](AGENTS.md).
