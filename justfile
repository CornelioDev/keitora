# SPDX-License-Identifier: MIT OR Apache-2.0
#
# Project commands, run with just (https://just.systems). `just check` is the
# main gate used by contributors, agents and CI, which also runs other recipes
# (.github/workflows/ci.yml); `just --list` shows every recipe.
#
# How a failure stops the run: just gives each line of a recipe its own shell,
# and stops at the first line that exits with anything other than 0, skipping
# the rest of the recipe and the recipes after it. just then exits non-zero as
# well. This only works if a line's exit code is that of its command, so keep
# one command per line, without `&&`, `|` or `;`.

# Windows runs each line with cmd.exe, which returns the exit code of the
# command it ran. It is the one shell that fits all of this:
# - `sh`, just's default, is not on PATH in cmd.exe or PowerShell with a default
#   install of Git for Windows.
# - PowerShell's execution policy can block the `pnpm.ps1` that an npm install
#   of pnpm creates.
#
# `/d` skips any AutoRun command set in the registry, so that a user's cmd.exe
# configuration cannot change the result.
set windows-shell := ["cmd.exe", "/d", "/c"]

# cmd.exe looks for a command in the current directory before PATH, so a
# `cargo.cmd` or `pnpm.cmd` in the repository would run instead of the real
# tool and could make every check pass. This variable makes it use PATH only.
export NoDefaultCurrentDirectoryInExePath := "1"

# Run the main checks; stops at the first failure
check: fmt clippy unit-test-layout test doc deny desktop extension test-scripts

# Check the formatting of the Rust code
fmt:
    cargo fmt --all --check

# Lint the Rust code; warnings are errors
clippy:
    cargo clippy --locked --workspace --all-targets -- -D warnings

# Keeps the 90 % of `just coverage` measuring production code: see the top of the
# script. It builds in target/unit-test-layout, so it doesn't rebuild anything else.
# It comes after clippy, which shows compile errors in a more readable form.

# Fail if keitora-core or keitora-ipc break the unit test convention
unit-test-layout:
    node scripts/check-unit-test-layout.mjs

# Run the Rust tests
test:
    cargo test --workspace --locked

# `-D warnings` turns rustdoc's warnings, such as a broken or private intra-doc
# link, into errors. `[env]` sets it for this recipe only, not for `cargo test`,
# which passes it to the doctests too, and no argument to `just doc` can change it.

# Build the Rust docs; warnings are errors
[env("RUSTDOCFLAGS", "-D warnings")]
doc:
    cargo doc --locked --workspace --no-deps

# Check Rust dependencies: licenses, advisories, duplicates, sources
deny:
    cargo deny --locked check

# Lint, typecheck and test the desktop app
desktop:
    pnpm --dir apps/desktop install --frozen-lockfile
    pnpm --dir apps/desktop lint
    pnpm --dir apps/desktop typecheck
    pnpm --dir apps/desktop test

# Lint, typecheck, test and build the extension
extension:
    pnpm --dir extension install --frozen-lockfile
    pnpm --dir extension lint
    pnpm --dir extension typecheck
    pnpm --dir extension test
    pnpm --dir extension build

# Test the scripts in scripts/ that run on Node.js
test-scripts:
    node --test scripts/check-acceptance-tests.spec.mjs scripts/check-npm-audit.spec.mjs scripts/check-npm-licenses.spec.mjs scripts/check-unit-test-layout.spec.mjs scripts/clippy-config.spec.mjs scripts/run-on-staged.spec.mjs

# Not part of `check`: a new advisory can fail it with no change in the repository,
# and CI runs it as a job of its own. Licenses aren't part of it: see `npm-licenses`.

# What CI's `npm dependencies` job runs: the advisories of the npm dependencies
npm-dependencies: npm-audit

# Not a gate: nothing runs this but a person. The npm license check is off as a gate
# during the MVP, by the maintainer's decision (2026-11-02), and must be analyzed before
# the first public release. Today it fails on elkjs (EPL-2.0). `pnpm licenses` reads
# what is installed, so this installs first.

# Report npm licenses that scripts/npm-license-policy.mjs rejects; not a gate, fails on elkjs
npm-licenses:
    pnpm --dir apps/desktop install --frozen-lockfile
    pnpm --dir extension install --frozen-lockfile
    node scripts/check-npm-licenses.mjs

# Development dependencies count, as they do for cargo-deny, and so does every severity
# from low up: the script has no `--prod`, and passes `--audit-level low` so that a
# setting in pnpm-workspace.yaml can't raise it. It audits both packages even when the
# first fails, which two recipe lines wouldn't.

# Fail if an npm dependency has a known advisory
npm-audit:
    node scripts/check-npm-audit.mjs

# Not part of `check`, which has no base to compare a branch to. CI runs the script
# directly, from the base's copy, for pull requests (.github/workflows/ci.yml). It
# fails on a shallow clone. On `main` it would fail anyway, because squash-merged
# tasks change acceptance tests.
#
# `$` exports the parameter as an environment variable, and the script reads it
# from there. The base comes from a pull request, so it stays out of the command
# line: cmd.exe would run the `&` in a base written as `{{ACCEPTANCE_BASE}}`.

# Fail if commits since the base change acceptance tests outside `test(` commits
check-acceptance-tests $ACCEPTANCE_BASE:
    node scripts/check-acceptance-tests.mjs

# Not part of `check`, which stays the fast gate: this needs extra tools
# (cargo-llvm-cov, llvm-tools), compiles the crates a second time and slows the
# frontend tests down. CI runs it as a job of its own.

# Measure test coverage; fails if keitora-core or keitora-ipc is below 90 %
coverage: coverage-rust coverage-desktop coverage-extension

# Each crate has to reach 90 % of its lines on its own, so a well-tested crate
# cannot make up for the other. `-p` measures only that crate's files: not
# dependencies, not other workspace crates. `--all-features` measures code under
# `#[cfg(feature = "...")]`, but not code under `#[cfg(not(feature = "..."))]`,
# which it stops compiling (CONTRIBUTING.md lists all that the gate doesn't
# measure). cargo-llvm-cov also leaves out, at any depth, files named tests.rs or
# *_tests.rs and directories named tests, examples or benches: those names must
# hold only test code, never product code. Unit tests of these
# two crates live in such files, so that the 90 % measures production code only.
# A crate with no functions at all is an error ("no coverage data found"), not a
# pass; see the placeholder test in keitora-ipc's lib.rs.

# Fail if line coverage of keitora-core or keitora-ipc is below 90 %
coverage-rust:
    cargo llvm-cov --locked --all-features -p keitora-core --fail-under-lines 90
    cargo llvm-cov --locked --all-features -p keitora-ipc --fail-under-lines 90

# Report the desktop app's test coverage in apps/desktop/coverage; no minimum
coverage-desktop:
    pnpm --dir apps/desktop install --frozen-lockfile
    pnpm --dir apps/desktop test:coverage

# Report the extension's test coverage in extension/coverage; no minimum
coverage-extension:
    pnpm --dir extension install --frozen-lockfile
    pnpm --dir extension test:coverage
