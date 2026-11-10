// SPDX-License-Identifier: MIT OR Apache-2.0

//! Acceptance tests for T-1.8: rustdoc for every public item, and
//! crate-level docs with a usage example (`cargo doc` builds without
//! warnings).
//!
//! Documentation has no runtime behavior, so most of this task is proven by
//! commands, which `just check` runs or should run:
//!
//! - Every public item is documented: the `missing_docs` lint, which
//!   `cargo clippy --workspace --all-targets -- -D warnings` turns into an
//!   error.
//! - `cargo doc` builds without warnings:
//!   `cargo doc --locked --workspace --no-deps` with
//!   `RUSTDOCFLAGS=-D warnings`.
//! - The crate-level example compiles and runs: it is a doctest, which
//!   `cargo test --workspace` runs.
//!
//! These tests pin what those commands can't see on their own: that the
//! lints stay switched on and unsilenced, and that the crate-level docs do
//! hold a usage example that runs.
//!
//! The crate-level docs are the `//!` comments at the top of `src/lib.rs`,
//! where they already are. A Markdown code block in them counts as a runnable
//! example when its info string is empty, `rust`, or `rust` with an
//! `edition…` attribute: `ignore`, `no_run`, `compile_fail`, `should_panic`,
//! `text` and other languages don't count. Hidden lines (starting with `# `)
//! don't count as what the example shows.
//!
//! # Coverage of the acceptance criteria
//!
//! **"Rustdoc for every public item"**
//! - `missing_docs_lint_is_active_for_keitora_core`
//! - `no_source_file_silences_a_documentation_lint`
//! - `no_item_is_hidden_from_the_docs`
//!
//! **"Crate-level docs with a usage example"**
//! - `crate_level_docs_contain_a_rust_example_that_runs_as_a_doctest`
//! - `crate_level_example_shows_the_engine_and_a_clock_through_the_public_api`
//! - `crate_level_example_checks_its_result_with_an_assertion`
//! - `doctests_of_keitora_core_are_not_disabled`
//!
//! **"`cargo doc` builds without warnings"**: proven by the command above, not
//! by a test here.

#![cfg(test)]

use std::fs;
use std::path::{Path, PathBuf};

const LIB_RS: &str = include_str!("../src/lib.rs");
const CRATE_CARGO_TOML: &str = include_str!("../Cargo.toml");
const WORKSPACE_CARGO_TOML: &str = include_str!("../../../Cargo.toml");

/// Lints whose job is to keep the docs complete. Silencing any of them
/// anywhere in the crate would let an undocumented public item through.
const DOC_LINTS: &[&str] = &[
    "missing_docs",
    "rustdoc::",
    "missing_errors_doc",
    "missing_panics_doc",
];

/// A fenced code block of the crate-level docs.
#[derive(Debug)]
struct CodeBlock {
    info: String,
    lines: Vec<String>,
}

impl CodeBlock {
    /// Whether rustdoc compiles and runs this block as a doctest.
    fn runs_as_doctest(&self) -> bool {
        self.info
            .split(|c: char| c == ',' || c.is_whitespace())
            .filter(|token| !token.is_empty())
            .all(|token| token == "rust" || token.starts_with("edition"))
    }

    /// The lines rustdoc shows to the reader: everything but hidden lines.
    fn visible_code(&self) -> String {
        self.lines
            .iter()
            .filter(|line| {
                let trimmed = line.trim_start();
                !(trimmed == "#" || trimmed.starts_with("# ") || trimmed.starts_with("#\t"))
            })
            .map(String::as_str)
            .collect::<Vec<_>>()
            .join("\n")
    }
}

/// The crate-level docs: the `//!` lines of `src/lib.rs`, without the marker
/// and the one space that usually follows it.
fn crate_level_docs() -> Vec<String> {
    LIB_RS
        .lines()
        .filter_map(|line| line.trim_start().strip_prefix("//!"))
        .map(|text| text.strip_prefix(' ').unwrap_or(text).to_owned())
        .collect()
}

/// The fenced code blocks (```` ``` ```` or `~~~`) of the crate-level docs.
fn crate_level_code_blocks() -> Vec<CodeBlock> {
    let mut blocks = Vec::new();
    let mut open: Option<(char, usize, CodeBlock)> = None;
    for line in crate_level_docs() {
        let trimmed = line.trim();
        let fence_char = trimmed.chars().next().filter(|c| *c == '`' || *c == '~');
        let fence_len = fence_char.map_or(0, |c| trimmed.chars().take_while(|x| *x == c).count());
        match open.take() {
            None => {
                if let Some(c) = fence_char.filter(|_| fence_len >= 3) {
                    let info = trimmed
                        .trim_start_matches(c)
                        .trim()
                        .trim_matches(|x| x == '{' || x == '}')
                        .to_owned();
                    open = Some((
                        c,
                        fence_len,
                        CodeBlock {
                            info,
                            lines: Vec::new(),
                        },
                    ));
                }
            }
            Some((c, len, mut block)) => {
                let closes = fence_char == Some(c)
                    && fence_len >= len
                    && trimmed.trim_start_matches(c).is_empty();
                if closes {
                    blocks.push(block);
                } else {
                    block.lines.push(line);
                    open = Some((c, len, block));
                }
            }
        }
    }
    assert!(
        open.is_none(),
        "the crate-level docs of keitora-core have a code block that is never closed"
    );
    blocks
}

/// The crate-level code blocks that run as doctests.
fn runnable_examples() -> Vec<CodeBlock> {
    crate_level_code_blocks()
        .into_iter()
        .filter(CodeBlock::runs_as_doctest)
        .collect()
}

/// Every `.rs` file under `dir`, with its contents.
#[expect(
    clippy::disallowed_methods,
    reason = "this test reads the crate's own sources; the ban on the file system is for the library"
)]
fn rust_sources(dir: &Path) -> Vec<(PathBuf, String)> {
    let mut files = Vec::new();
    let mut entries: Vec<_> = fs::read_dir(dir)
        .expect("read a source directory")
        .map(|entry| entry.expect("read a source directory entry"))
        .collect();
    entries.sort_by_key(fs::DirEntry::path);
    for entry in entries {
        let path = entry.path();
        if entry.file_type().expect("read a file type").is_dir() {
            files.extend(rust_sources(&path));
        } else if path.extension().is_some_and(|ext| ext == "rs") {
            let text = fs::read_to_string(&path).expect("read a source file");
            files.push((path, text));
        }
    }
    files
}

/// The sources of the library: everything under `src/`.
fn library_sources() -> Vec<(PathBuf, String)> {
    let src = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
    let files = rust_sources(&src);
    assert!(
        files.iter().any(|(path, _)| path.ends_with("lib.rs")),
        "found no src/lib.rs under {}",
        src.display()
    );
    files
}

/// The contents of every attribute (`#[…]` or `#![…]`) in `text`, without
/// whitespace, so that `allow( missing_docs )` and a line break inside an
/// attribute read the same as `allow(missing_docs)`.
fn attributes(text: &str) -> Vec<String> {
    let mut found = Vec::new();
    let mut rest = text;
    while let Some(start) = rest.find('#') {
        let after_hash = rest.get(start..).expect("slice at a char boundary");
        let after_hash = after_hash.get(1..).expect("'#' is one byte");
        let body = after_hash.strip_prefix('!').unwrap_or(after_hash);
        let Some(body) = body.strip_prefix('[') else {
            rest = after_hash;
            continue;
        };
        let mut depth = 1_usize;
        let mut in_string = false;
        let mut escaped = false;
        let mut content = String::new();
        let mut consumed = body.len();
        for (index, c) in body.char_indices() {
            if in_string {
                if escaped {
                    escaped = false;
                } else if c == '\\' {
                    escaped = true;
                } else if c == '"' {
                    in_string = false;
                }
            } else if c == '"' {
                in_string = true;
            } else if c == '[' {
                depth = depth.checked_add(1).expect("bracket depth fits");
            } else if c == ']' {
                depth = depth.checked_sub(1).expect("brackets are balanced");
                if depth == 0 {
                    consumed = index;
                    break;
                }
            }
            if !c.is_whitespace() {
                content.push(c);
            }
        }
        found.push(content);
        rest = body.get(consumed..).expect("slice at a char boundary");
    }
    found
}

// ---------------------------------------------------------------------------
// "Rustdoc for every public item"
// ---------------------------------------------------------------------------

#[test]
fn missing_docs_lint_is_active_for_keitora_core() {
    let level_ok = |text: &str| {
        ["\"warn\"", "\"deny\"", "\"forbid\""]
            .iter()
            .any(|level| text.contains(level))
    };

    // Either lib.rs switches the lint on itself...
    let in_lib_rs = attributes(LIB_RS).iter().any(|attr| {
        ["warn(", "deny(", "forbid("]
            .iter()
            .any(|level| attr.starts_with(level))
            && attr.contains("missing_docs")
    });

    // ...or the crate inherits the workspace lints, and they switch it on.
    let mut section = String::new();
    let mut inherits = false;
    for line in CRATE_CARGO_TOML.lines() {
        let line: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        if line.starts_with('[') {
            section = line.clone();
        } else if (section == "[lints]" && line == "workspace=true")
            || (section.is_empty() && line == "lints.workspace=true")
        {
            inherits = true;
        }
    }
    let mut in_workspace_lints = false;
    let mut workspace_level_ok = false;
    for line in WORKSPACE_CARGO_TOML.lines() {
        let trimmed = line.trim();
        if trimmed.starts_with('[') {
            in_workspace_lints = trimmed == "[workspace.lints.rust]";
        } else if in_workspace_lints
            && trimmed.starts_with("missing_docs")
            && trimmed
                .strip_prefix("missing_docs")
                .is_some_and(|rest| rest.trim_start().starts_with('='))
        {
            workspace_level_ok = level_ok(trimmed);
        }
    }

    assert!(
        in_lib_rs || (inherits && workspace_level_ok),
        "keitora-core must have the `missing_docs` lint at warn, deny or forbid, either in \
         src/lib.rs or through `[lints] workspace = true` and `missing_docs` in \
         `[workspace.lints.rust]`; otherwise an undocumented public item compiles silently \
         (inherits workspace lints: {inherits}, workspace level ok: {workspace_level_ok}, \
         set in lib.rs: {in_lib_rs})"
    );
}

#[test]
fn no_source_file_silences_a_documentation_lint() {
    let mut offenders = Vec::new();
    for (path, text) in library_sources() {
        for attr in attributes(&text) {
            let silences = attr.contains("allow(") || attr.contains("expect(");
            if silences && DOC_LINTS.iter().any(|lint| attr.contains(lint)) {
                offenders.push(format!("{}: #[{attr}]", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "documentation lints must not be silenced in keitora-core; document the item \
         instead:\n{}",
        offenders.join("\n")
    );
}

#[test]
fn no_item_is_hidden_from_the_docs() {
    let mut offenders = Vec::new();
    for (path, text) in library_sources() {
        for attr in attributes(&text) {
            let is_doc_attr = attr.starts_with("doc(") || attr.contains(",doc(");
            if is_doc_attr && attr.contains("hidden") {
                offenders.push(format!("{}: #[{attr}]", path.display()));
            }
        }
    }
    assert!(
        offenders.is_empty(),
        "`#[doc(hidden)]` turns a public item into an undocumented one that `missing_docs` \
         no longer reports:\n{}",
        offenders.join("\n")
    );
}

// ---------------------------------------------------------------------------
// "Crate-level docs with a usage example"
// ---------------------------------------------------------------------------

#[test]
fn crate_level_docs_contain_a_rust_example_that_runs_as_a_doctest() {
    let blocks = crate_level_code_blocks();
    let examples = runnable_examples();
    assert!(
        !examples.is_empty(),
        "the crate-level docs of keitora-core (the `//!` comments in src/lib.rs) must hold \
         a Rust code block that rustdoc compiles and runs (no `ignore`, `no_run`, \
         `compile_fail`, `should_panic` or `text`); found these code blocks: {blocks:#?}"
    );
}

#[test]
fn crate_level_example_shows_the_engine_and_a_clock_through_the_public_api() {
    let examples = runnable_examples();
    let shows_usage = examples.iter().any(|block| {
        let code = block.visible_code();
        code.contains("keitora_core::") && code.contains("Engine") && code.contains("Clock")
    });
    assert!(
        shows_usage,
        "a runnable crate-level example of keitora-core must show, in lines the reader \
         sees, the crate's main entry point: an `Engine` driven by a `Clock`, reached \
         through `keitora_core::` paths; found these runnable examples: {examples:#?}"
    );
}

#[test]
fn crate_level_example_checks_its_result_with_an_assertion() {
    let examples = runnable_examples();
    let asserts = examples.iter().any(|block| {
        let code = block.visible_code();
        code.contains("keitora_core::") && code.contains("assert")
    });
    assert!(
        asserts,
        "a runnable crate-level example of keitora-core must check what it shows with a \
         visible `assert!`, `assert_eq!` or `assert_ne!`, so that `cargo test` fails if \
         the example stops describing the real behavior; found these runnable examples: \
         {examples:#?}"
    );
}

#[test]
fn doctests_of_keitora_core_are_not_disabled() {
    let disabled = CRATE_CARGO_TOML.lines().any(|line| {
        let line: String = line.chars().filter(|c| !c.is_whitespace()).collect();
        line.starts_with("doctest=false") || line.contains("lib.doctest=false")
    });
    assert!(
        !disabled,
        "keitora-core's Cargo.toml must not set `doctest = false`: `cargo test` has to run \
         the crate-level example"
    );
}
