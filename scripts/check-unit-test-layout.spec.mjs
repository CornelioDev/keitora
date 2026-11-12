// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Tests for check-unit-test-layout.mjs and its rules, with Node's own test runner.
// The rules are tried directly on many sources. The script itself runs with real
// cargo in small workspaces in the system's temporary directory, each with its own
// target directory. `just check` runs them (`just test-scripts`).

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { cpSync, existsSync, mkdirSync, mkdtempSync, readFileSync, rmSync, utimesSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";

import {
  findManifestProblems,
  isDataFile,
  isInside,
  isLeftOutOfCoverage,
  isTestFilePath,
  parseDepInfo,
  pathSegments,
  scanSource,
  selectBuilds,
  show,
  tokenize,
} from "./unit-test-layout-rules.mjs";

const SCRIPT = fileURLToPath(new URL("check-unit-test-layout.mjs", import.meta.url));
const TOOLCHAIN = fileURLToPath(new URL("../rust-toolchain.toml", import.meta.url));
const PASS = 0;
const VIOLATION = 1;
const CANNOT_CHECK = 2;

const PLACEHOLDER = `#[cfg(test)]
mod tests {
    /// Comments are free to change, and to mention #[test].
    #[test]
    fn gives_the_coverage_gate_a_line_to_measure() {}
}
`;

const MEASURED = { isTestFile: false, mayHoldPlaceholder: false };
const CRATE_ROOT = { isTestFile: false, mayHoldPlaceholder: true };
const TEST_FILE = { isTestFile: true, mayHoldPlaceholder: false };

/** Declares tests that expect `scanSource` to find nothing in each source. */
function allowed(where, options, cases) {
  for (const [name, source] of cases) {
    test(`allowed in ${where}: ${name}`, () => {
      assert.deepEqual(scanSource(source, options), []);
    });
  }
}

/** Declares tests that expect `scanSource` to find a problem in each source. */
function rejected(where, options, cases) {
  for (const [name, source] of cases) {
    test(`rejected in ${where}: ${name}`, () => {
      assert.notEqual(scanSource(source, options).length, 0);
    });
  }
}

allowed("a measured file", MEASURED, [
  ["plain code", "pub fn add(a: u32, b: u32) -> u32 { a + b }\n"],
  ["the sibling test module", "pub fn a() {}\n\n#[cfg(test)]\nmod tests;\n"],
  ["the sibling test module, with a comment between", "#[cfg(test)] // why\nmod tests;\n"],
  ["a line comment", "// #[test] #[cfg(not(test))] cfg_attr(test, x)\n"],
  ["a doc comment", "/// Use `#[cfg(not(test))]` or #[test].\npub fn a() {}\n"],
  ["a block comment", "/* #[test] */ pub fn a() {}\n"],
  ["a nested block comment", "/* /* */ #[test] fn a() {} /* */ #[test] */ pub fn b() {}\n"],
  ["a block comment that looks closed", "/*/ #[test] */ pub fn a() {}\n"],
  ["a string", 'pub const A: &str = "#[test] #[cfg(not(test))]";\n'],
  ["a string with an escaped quote", 'pub const A: &str = "\\"#[test]";\n'],
  ["a raw string", 'pub const A: &str = r#"#[test] " #[cfg(not(test))]"#;\n'],
  ["a raw string with more hashes", 'pub const A: &str = r##"x"# #[test] "##;\n'],
  ["a byte string and a raw C string", 'pub const A: &[u8] = b"#[test]"; pub const B: &core::ffi::CStr = cr"#[test]";\n'],
  ["a doc attribute", '#[doc = "#[test]"]\npub fn a() {}\n'],
  ["a feature named test", '#[cfg(feature = "test")]\npub fn a() {}\n'],
  ["other cfgs", '#[cfg(not(feature = "x"))]\npub fn a() {}\n#[cfg_attr(feature = "x", derive(Debug))]\npub struct B;\n'],
  ["a lifetime next to quotes", "pub fn a<'a>(b: &'a str) -> &'a str { b }\n"],
  ["attributes that are not about tests", "#[derive(Debug)]\n#[must_use]\npub struct A;\n"],
  ["a use without test attributes", "use core::fmt::Debug;\nuse crate::latest::x;\n"],
]);

rejected("a measured file", MEASURED, [
  ["#[test]", "#[test]\nfn a() {}\n"],
  ["#[test] with spaces", "# [ test ]\nfn a() {}\n"],
  ["#[test] over several lines", "#\n[\n  test\n]\nfn a() {}\n"],
  ["#[test] with comments inside", "#/* a */[/* b */test// c\n]\nfn a() {}\n"],
  ["#[test] after a left-to-right mark, whitespace for rustc", "#\u200E[\u200Etest]\nfn a() {}\n"],
  ["#[r#test]", "#[r#test]\nfn a() {}\n"],
  ["#[::core::prelude::v1::test]", "#[::core::prelude::v1::test]\nfn a() {}\n"],
  ["#[core::prelude::rust_2021::test]", "#[core::prelude::rust_2021::test]\nfn a() {}\n"],
  ["#[std :: prelude :: v1 :: test]", "#[std :: prelude :: v1 :: test]\nfn a() {}\n"],
  ["#[bench]", "#[bench]\nfn a() {}\n"],
  ["#[test_case]", "#[test_case]\nfn a() {}\n"],
  ["a test attribute from a dependency", "#[tokio::test]\nasync fn a() {}\n#[rstest]\nfn b() {}\n"],
  ["#![test], as an inner attribute", "fn a() { #![test] }\n"],
  ["#[test] in a macro", "macro_rules! m { () => { #[test] fn a() {} } }\nm!();\n"],
  ["a use that renames #[test]", "use core::prelude::v1::test as check;\n#[check]\nfn a() {}\n"],
  ["a use that renames #[test], nested", "use core::{prelude::{v1::{test as check}}};\n#[check]\nfn a() {}\n"],
  ["#[test] in a cfg_attr", '#[cfg_attr(feature = "x", test)]\nfn a() {}\n'],
  ["#[test] after a character that looks like a string", "const A: char = '\"'; #[test] fn a() {} const B: char = '\"';\n"],
  ["#[test] after an escaped quote character", "const A: char = '\\''; #[test] fn a() {} const B: char = '\\'';\n"],
  ["#[test] after a byte quote character", "const A: u8 = b'\"'; #[test] fn a() {} const B: u8 = b'\"';\n"],
  ["#[test] after a raw string ending in a backslash", 'const A: &str = r"\\"; #[test] fn a() {} const B: &str = "";\n'],
  ["#[test] after a raw string with a false end", 'const A: &str = r#"x"#; #[test] fn a() {} const B: &str = r#"y"#;\n'],
  ["#[test] after a closed nested comment", "/* /* */ */ #[test] fn a() {}\n"],
  ["#[test] after a // inside a block comment", "/* // */ #[test] fn a() {}\n"],
  ["#[test] after a /* in a line comment", "// /*\n#[test] fn a() {}\n// */\n"],
  ["#[test] after a lifetime, not a character", "fn a<'a>(b: &'a u8) {} #[test] fn c<'c>() {}\n"],
  ["#[test] after an identifier ending in r", "const BAR: u8 = 0; #[test] fn a() { let bar = 1; }\n"],
  ["#[cfg(not(test))]", "#[cfg(not(test))]\npub fn a() {}\n"],
  ["#[cfg(not(test))] with spaces", "# [ cfg ( not ( test ) ) ]\npub fn a() {}\n"],
  ["#[cfg(not(test))] over several lines", "#[cfg(\n  not(\n    test\n  )\n)]\npub fn a() {}\n"],
  ["cfg(any(not(test)))", "#[cfg(any(not(test)))]\npub fn a() {}\n"],
  ['cfg(all(not(test), feature = "x"))', '#[cfg(all(not(test), feature = "x"))]\npub fn a() {}\n'],
  ["cfg(not(any(test)))", "#[cfg(not(any(test)))]\npub fn a() {}\n"],
  ["cfg!(not(test))", "pub fn a() -> bool { cfg!(not(test)) }\n"],
  ["cfg!(test)", "pub fn a() -> bool { cfg!(test) }\n"],
  ["cfg_attr(test, ...)", "#[cfg_attr(test, derive(Debug))]\npub struct A;\n"],
  ["cfg_attr( test, ...), spaced", "#[ cfg_attr ( test , derive(Debug) ) ]\npub struct A;\n"],
  ["cfg_attr(any(test), ...)", "#[cfg_attr(any(test), derive(Debug))]\npub struct A;\n"],
  ["cfg_attr(not(test), path = ...)", '#[cfg_attr(not(test), path = "a.rs")]\nmod a;\n'],
  ["cfg(not(coverage))", "#[cfg(not(coverage))]\npub fn a() {}\n"],
  ["cfg(coverage)", "#[cfg(coverage)]\npub fn a() {}\n"],
  ["cfg_attr(coverage, ...)", "#[cfg_attr(coverage, derive(Debug))]\npub struct A;\n"],
  ["#[coverage(off)]", "#[coverage(off)]\npub fn a() {}\n"],
  ["#[cfg(test)] on an inline module", "#[cfg(test)]\nmod tests { fn helper() {} }\n"],
  ["#[cfg(test)] on another module", "#[cfg(test)]\nmod helpers;\n"],
  ["#[cfg(test)] on a function", "#[cfg(test)]\nfn a() {}\n"],
  ["#[cfg(test)] on pub mod tests", "#[cfg(test)]\npub mod tests;\n"],
  ["#[cfg(test)] with #[path] before mod tests", '#[cfg(test)]\n#[path = "a.rs"]\nmod tests;\n'],
  ["#![cfg(test)] on a measured file", "#![cfg(test)]\nfn a() {}\n"],
  ["cfg(any(test))", "#[cfg(any(test))]\nmod tests;\n"],
  ["cfg(test) on an include", '#[cfg(test)]\ninclude!("tests.rs");\n'],
  ["the placeholder outside lib.rs", PLACEHOLDER],
]);

allowed("the lib.rs of a crate with a placeholder", CRATE_ROOT, [
  ["the placeholder", `//! Docs.\n\n#![forbid(unsafe_code)]\n\n${PLACEHOLDER}`],
  ["the placeholder, reformatted", "# [ cfg ( test ) ] mod tests { # [ test ] fn gives_the_coverage_gate_a_line_to_measure ( ) { } }\n"],
  ["the placeholder and a sibling test module of another file", `pub mod engine;\n${PLACEHOLDER}`],
]);

rejected("the lib.rs of a crate with a placeholder", CRATE_ROOT, [
  ["the placeholder with a body", PLACEHOLDER.replace("{}", "{ super::a(); }")],
  ["the placeholder under another name", PLACEHOLDER.replace("gives_the", "gives")],
  ["the placeholder with one more test", PLACEHOLDER.replace("{}\n", "{}\n    #[test]\n    fn b() {}\n")],
  ["the placeholder twice", `${PLACEHOLDER}mod other {\n${PLACEHOLDER}}\n`],
  ["the placeholder and another test", `${PLACEHOLDER}#[test]\nfn b() {}\n`],
  ["the placeholder without its #[cfg(test)]", PLACEHOLDER.replace("#[cfg(test)]\n", "")],
  ["the placeholder with #[cfg(not(test))]", PLACEHOLDER.replace("cfg(test)", "cfg(not(test))")],
]);

allowed("a test file", TEST_FILE, [
  ["tests", "use super::*;\n\n#[test]\nfn a() {}\n"],
  ["a renamed #[test]", "use core::prelude::v1::test as check;\n#[check]\nfn a() {}\n"],
  ["#[cfg(test)] and #![cfg(test)]", "#![cfg(test)]\n#[cfg(test)]\nfn a() {}\n"],
  ["#[cfg(test)] on a module", "#[cfg(test)]\nmod helpers { fn a() {} }\n"],
]);

rejected("a test file", TEST_FILE, [
  ["#[cfg(not(test))]", "#[cfg(not(test))]\nfn a() {}\n"],
  ["cfg(any(not(test)))", "#[cfg(any(not(test)))]\nfn a() {}\n"],
  ["cfg_attr(test, ...)", "#[cfg_attr(test, derive(Debug))]\nstruct A;\n"],
  ["cfg(not(coverage))", "#[cfg(not(coverage))]\nfn a() {}\n"],
]);

for (const [name, source] of [
  ["an unterminated block comment", "/* /* */ #[test] fn a() {}\n"],
  ["an unterminated string", 'const A: &str = "x;\n#[test] fn a() {}\n'],
  ["an unterminated raw string", 'const A: &str = r#"x";\n#[test] fn a() {}\n'],
  ["an unterminated character", "const A: char = '"],
  ["a #! line that would hide code", "#!/bin/sh /*\n#[test] fn a() {}\n// */\n"],
  ["a #! line after a byte order mark", "\uFEFF#!/bin/sh /*\n#[test] fn a() {}\n// */\n"],
  ["unbalanced brackets in a cfg", "#[cfg(not(test)]\nfn a() {}\n"],
]) {
  test(`no verdict: ${name}`, () => {
    assert.throws(() => scanSource(source, MEASURED));
  });
}

test("an inner attribute at the start of a file is not a #! line", () => {
  assert.deepEqual(scanSource("#![forbid(unsafe_code)]\npub fn a() {}\n", MEASURED), []);
  assert.deepEqual(scanSource("#!  [forbid(unsafe_code)]\n", MEASURED), []);
  assert.deepEqual(scanSource("\uFEFF#![forbid(unsafe_code)]\npub fn a() {}\n", MEASURED), []);
});

for (const [name, source] of [
  ["a #! that rustc reads as a line, because of a doc comment", "#!/** d */ /*\n#[test] fn a() {}\n// */ [x]\n"],
  ["a #! followed by a comment, plainly or not an attribute", "#! /* why */ [forbid(unsafe_code)]\n"],
]) {
  test(`no verdict: ${name}`, () => {
    assert.throws(() => scanSource(source, MEASURED));
  });
}

allowed("a measured file", MEASURED, [
  ["a #[path] to a Rust file", '#[path = "a.rs"]\npub mod a;\n'],
  ["an include! of a Rust file", 'include!("a.rs");\n'],
  ["an include! of a Rust file, from a raw string", 'include!(r#"a.rs"#);\n'],
  ["an include! built with concat!", 'include!(concat!(env!("OUT"), "/a.rs"));\n'],
  ["include_str! of a data file", 'pub const A: &str = include_str!("a.json");\n'],
  ["a variable named path", 'pub fn a() { let path = "a.txt"; drop(path); }\n'],
  ["precise capturing, with a name test later", "pub fn a<'a>(x: &'a u8) -> impl Sized + use<'a> { let test = x; test }\n"],
  ["a method named cfg", "pub fn a(x: X, test: u8) { x.cfg(test); }\n"],
  ["a function named cfg", "pub fn cfg(coverage: f64) -> f64 { coverage }\n"],
  ["a value named cfg, followed by a block or index", "pub fn a(cfg: bool, test: [u8; 1]) { if cfg { drop(test[0]); } }\n"],
  ["a macro forwarding attributes", "macro_rules! m { ($(#[$meta:meta])* $i:item) => { $(#[$meta])* $i }; }\n"],
  ["a #[cfg(test)] mod tests; after an inner attribute", "#![forbid(unsafe_code)]\n#[cfg(test)]\nmod tests;\n"],
  ["a literal with a suffix", 'pub const A: u8 = 1u8; pub const B: &str = "b";\n'],
]);

rejected("a measured file", MEASURED, [
  ["#[path] to a file that isn't Rust", '#[path = "a.inc"]\npub mod a;\n'],
  ["#[path] to a data file", '#[path = "a.json"]\npub mod a;\n'],
  ["#[path] from a raw string", '#[path = r#"a.txt"#]\npub mod a;\n'],
  ["#[path] in a cfg_attr", '#[cfg_attr(feature = "x", path = "a.inc")]\npub mod a;\n'],
  ["include! of a file that isn't Rust", 'include!("a.inc");\n'],
  ["include! of a data file", 'include!("a.json");\n'],
  ["include! with square brackets", 'include!["a.inc"];\n'],
  ["include! of a name a macro makes", "include!(name!());\n"],
  ["#[path] after #[cfg(test)] mod tests;", '#[cfg(test)]\n#[path = "a.rs"]\nmod tests;\n'],
  ["#[path] before #[cfg(test)] mod tests;", '#[path = "a.rs"]\n#[cfg(test)]\nmod tests;\n'],
  ["another attribute before #[cfg(test)] mod tests;", "#[expect(unused)]\n#[cfg(test)]\nmod tests;\n"],
  ["#[cfg(not(debug_assertions))]", "#[cfg(not(debug_assertions))]\npub fn a() {}\n"],
  ["#[cfg(debug_assertions)]", "#[cfg(debug_assertions)]\npub mod real;\n"],
  ["cfg_attr on debug_assertions", '#[cfg_attr(not(debug_assertions), path = "engine/tests.rs")]\nmod a;\n'],
  ["cfg! on debug_assertions", "pub const A: bool = cfg!(debug_assertions);\n"],
  ["cfg_select!", "core::cfg_select! { test => {} _ => { pub fn a() {} } }\n"],
  ["cfg_match!", "core::cfg_match! { test => {} _ => {} }\n"],
  ["cfg_if!", "cfg_if::cfg_if! { if #[cfg(unix)] {} }\n"],
  ["a renamed cfg_select", "use core::cfg_select as pick;\n"],
  ["a $ in a cfg", "macro_rules! when { ($p:meta, $i:item) => { #[cfg($p)] $i }; }\n"],
  ["a $ in a cfg_attr", "macro_rules! when { ($p:meta) => { #[cfg_attr($p, derive(Debug))] pub struct S; }; }\n"],
  ["a $ in a cfg!", "macro_rules! when { ($p:meta) => { cfg!($p) }; }\n"],
  ["cfg![not(test)]", "pub const A: bool = cfg![not(test)];\n"],
  ["cfg!{not(test)}", "pub const A: bool = cfg!{not(test)};\n"],
  ["#[cfg[not(test)]]", "#[cfg[not(test)]]\npub fn a() {}\n"],
  ["#[cfg{not(test)}]", "#[cfg{not(test)}]\npub fn a() {}\n"],
  ["#[cfg_attr[test, derive(Debug)]]", "#[cfg_attr[test, derive(Debug)]]\npub struct S;\n"],
  ["a cfg(not(test)) as a macro's argument", "when!(cfg(not(test)), pub fn a() {});\n"],
  ["cfg(any(test, feature)), for which a public module is the alternative", '#[cfg(any(test, feature = "x"))]\npub mod testing;\n'],
  ["#[test] hidden by a raw string read as a suffix", 'eat!("a"r"\\"x");\n#[test] fn a() {}\neat!("a"r"\\"x");\n'],
  ["#[test] hidden by a suffix on a character", "eat!('a'r\"\\\"x\");\n#[test] fn a() {}\neat!('a'r\"\\\"x\");\n"],
]);

rejected("a measured file", MEASURED, [
  ["a use that renames include!", 'use core::include as i;\ni!("a.json");\n'],
  ["a use that renames include!, from std and nested", 'use std::{include as i};\ni!("a.md");\n'],
  ["#[test] hidden by a raw lifetime read as a raw string", "eat!('r#r\"\\\"x\");\n#[test] fn a() {}\neat!('r#r\"\\\"x\");\n"],
]);

test("a raw lifetime is one token", () => {
  assert.deepEqual(
    tokenize("'r#a b").map((token) => token.kind),
    ["literal", "ident"],
  );
  // Without a name after the `#`, it stays a lifetime, then a `#`.
  assert.deepEqual(
    tokenize("'r #a").map((token) => token.text),
    ["", "#", "a"],
  );
});

rejected("a test file", TEST_FILE, [
  ["a use that renames include!, which could compile a data file", 'use core::include as i;\ni!("pad.json");\n'],
  ["a use that renames include!, nested", 'use std::{include as i};\ni!("pad.json");\n'],
  ["cfg(debug_assertions)", "#[cfg(debug_assertions)]\nfn a() {}\n"],
  ["cfg_select!", "core::cfg_select! { test => {} _ => {} }\n"],
  ["a $ in a cfg", "macro_rules! when { ($p:meta) => { #[cfg($p)] fn a() {} }; }\n"],
  ["#[path] to a file that isn't Rust", '#[path = "a.inc"]\nmod a;\n'],
]);

test("the scan takes linear time on nested or repeated forms", () => {
  const count = 100_000;
  for (const source of [
    `${"cfg(".repeat(count)}${")".repeat(count)}`,
    "use ".repeat(count),
    `${"#[a(".repeat(count)}${")]".repeat(count)}`,
    `/*${"/*".repeat(count)}${"*/".repeat(count)}*/`,
  ]) {
    const start = performance.now();
    scanSource(source, MEASURED);
    assert.ok(performance.now() - start < 5000, source.slice(0, 20));
  }
});

test("only a few kinds of file count as data", () => {
  for (const name of ["a.json", "a.md", "a.toml", "A.JSON"]) {
    assert.equal(isDataFile(name), true, name);
  }
  for (const name of ["a.txt", "a.inc", "a", "a.json.inc", "a.rs"]) {
    assert.equal(isDataFile(name), false, name);
  }
});

test("a rustc directory with a version or a hash is left out of coverage", () => {
  for (const path of ["src/rustc/abc123/a.rs", "src/deep/rustc/1.99.1/x/a.rs"]) {
    assert.equal(isLeftOutOfCoverage(pathSegments(path)), true, path);
    assert.equal(isTestFilePath(pathSegments(path)), false, path);
  }
  for (const path of ["src/rustc/a.rs", "src/rustc/not-hex/a.rs", "src/rustc/1.99/a.rs"]) {
    assert.equal(isLeftOutOfCoverage(pathSegments(path)), false, path);
  }
});

/** The metadata of a workspace with `crates/c` and, if given, more packages. */
function metadata({ targets = [{ name: "c", kind: ["lib"] }], dependencies = [], others = [], deps = [] } = {}) {
  const crate = { id: "c", name: "c", manifest_path: "/w/crates/c/Cargo.toml", targets, dependencies };
  return {
    packages: [crate, ...others],
    workspace_members: ["c", ...others.filter((item) => item.member).map((item) => item.id)],
    resolve: { nodes: [{ id: "c", deps }] },
  };
}

test("a manifest with a library and integration tests is fine", () => {
  const { problems, found } = findManifestProblems(
    metadata({ targets: [{ name: "c", kind: ["lib"] }, { name: "a", kind: ["test"] }] }),
    "c",
    "/w/crates/c",
  );
  assert.deepEqual(problems, []);
  assert.equal(found.id, "c");
});

for (const [name, input] of [
  ["a build script", { targets: [{ name: "build", kind: ["custom-build"] }] }],
  ["a proc-macro library", { targets: [{ name: "c", kind: ["proc-macro"] }] }],
  ["a binary", { targets: [{ name: "c", kind: ["bin"] }] }],
  ["an example", { targets: [{ name: "x", kind: ["example"] }] }],
  ["a bench", { targets: [{ name: "x", kind: ["bench"] }] }],
  [
    "a path dependency",
    { others: [{ id: "x", name: "x", source: null, targets: [{ name: "x", kind: ["lib"] }] }], deps: [{ pkg: "x" }] },
  ],
  [
    "a dependency that [patch] turns into a local crate, with no path in its entry",
    {
      dependencies: [{ name: "foo", source: "registry+https://github.com/rust-lang/crates.io-index" }],
      others: [{ id: "foo", name: "foo", source: null, targets: [{ name: "foo", kind: ["lib"] }] }],
      deps: [{ pkg: "foo" }],
    },
  ],
  [
    "a direct proc-macro dependency",
    { others: [{ id: "m", name: "m", targets: [{ name: "m", kind: ["proc-macro"] }] }], deps: [{ pkg: "m" }] },
  ],
  [
    "a workspace member inside the crate",
    { others: [{ id: "n", name: "n", member: true, manifest_path: "/w/crates/c/src/engine/Cargo.toml", targets: [] }] },
  ],
]) {
  test(`a manifest problem: ${name}`, () => {
    const crateTargets = input.targets ?? [{ name: "c", kind: ["lib"] }];
    const { problems } = findManifestProblems(
      metadata({ ...input, targets: crateTargets }),
      "c",
      "/w/crates/c",
    );
    assert.equal(problems.length, 1, problems.join());
  });
}

test("a manifest with what these crates may depend on is fine", () => {
  // keitora-ipc may use keitora-core; a proc macro behind a normal dependency is `serde`'s.
  const input = metadata({
    others: [
      { id: "serde", name: "serde", source: "registry", targets: [{ name: "serde", kind: ["lib"] }] },
      { id: "keitora-core", name: "keitora-core", source: null, targets: [{ name: "keitora_core", kind: ["lib"] }] },
    ],
    deps: [{ pkg: "serde" }, { pkg: "keitora-core" }],
  });
  input.packages[0].name = "keitora-ipc";
  assert.deepEqual(findManifestProblems(input, "keitora-ipc", "/w/crates/c").problems, []);
  input.packages[0].name = "keitora-core";
  assert.equal(findManifestProblems(input, "keitora-core", "/w/crates/c").problems.length, 1);
});

/** cargo's report of one build. */
function artifact(kind, name, isTestBuild) {
  return { target: { kind: [kind], name }, profile: { test: isTestBuild } };
}

test("the report of the builds has to list exactly the expected targets", () => {
  const library = artifact("lib", "c", false);
  const forTests = artifact("lib", "c", true);
  const one = artifact("test", "a", true);
  const builds = { library, libraryForTests: forTests, tests: [one] };
  assert.deepEqual(selectBuilds([library], [forTests, one], "c", ["a"]), builds);
  // Integration tests link the library, so the test run may build it once more.
  assert.deepEqual(selectBuilds([library], [library, forTests, one], "c", ["a"]), builds);
  assert.deepEqual(selectBuilds([library], [forTests], "c", []).tests, []);
  // Missing, repeated (as a compiler plugin could print one) or unexpected.
  assert.throws(() => selectBuilds([], [forTests], "c", []));
  assert.throws(() => selectBuilds([library], [], "c", []));
  assert.throws(() => selectBuilds([library], [forTests], "c", ["a"]));
  assert.throws(() => selectBuilds([library, library], [forTests], "c", []));
  assert.throws(() => selectBuilds([library], [forTests, forTests], "c", []));
  assert.throws(() => selectBuilds([library], [library, library, forTests], "c", []));
  assert.throws(() => selectBuilds([library], [forTests, one], "c", []));
});

test("a dep-info file only counts inside the check's target directory", () => {
  assert.equal(isInside("/w/target/u", "/w/target/u/debug/deps/a.d"), true);
  assert.equal(isInside("/w/target/u", "/w/crates/a/decoy/a.d"), false);
  assert.equal(isInside("/w/target/u", "/w/target/u"), false);
});

test("no verdict: a crate that is not at its place, or a metadata without a resolve", () => {
  assert.throws(() => findManifestProblems(metadata(), "c", "/elsewhere"));
  assert.throws(() => findManifestProblems({ ...metadata(), resolve: null }, "c", "/w/crates/c"));
});

test("a problem reports where it is in the source", () => {
  const source = "pub fn a() {}\n#[test]\nfn b() {}\n";
  const [{ index }] = scanSource(source, MEASURED);
  assert.equal(index, source.indexOf("#[test]"));
});

test("tokenize keeps the text of identifiers, in the form rustc compares them", () => {
  const names = (source) =>
    tokenize(source)
      .filter((token) => token.kind === "ident")
      .map((token) => token.text);
  assert.deepEqual(names("r#test bar r"), ["test", "bar", "r"]);
  assert.deepEqual(names("cafe\u0301"), ["caf\u00E9"]);
  assert.deepEqual(names("'a 'static '\u{1F600}' x"), ["x"]);
});

test("parseDepInfo reads what rustc writes on Windows", () => {
  const text = [
    String.raw`C:\Users\José Manuel\dir con é\target/u\debug\deps\c-1.d: crates\c\src\lib.rs crates\c\src\a\ b.rs crates\c\src\tests\..\sub.rs`,
    "",
    String.raw`C:\Users\José Manuel\dir con é\target/u\debug\deps\libc-1.rmeta: crates\c\src\lib.rs crates\c\src\a\ b.rs crates\c\src\tests\..\sub.rs`,
    "",
    String.raw`crates\c\src\lib.rs:`,
    String.raw`crates\c\src\a\ b.rs:`,
    String.raw`crates\c\src\tests\..\sub.rs:`,
    "",
    "# env-dep:CARGO_MANIFEST_DIR=C:\\dir: with a colon",
    "",
  ].join("\n");
  assert.deepEqual(parseDepInfo(text), [
    String.raw`crates\c\src\lib.rs`,
    String.raw`crates\c\src\a b.rs`,
    String.raw`crates\c\src\tests\..\sub.rs`,
  ]);
});

test("parseDepInfo reads an absolute input with spaces, and a space after a separator", () => {
  assert.deepEqual(parseDepInfo(String.raw`a.d: C:\x\ y\a.rs d\\ e.rs` + "\r\n"), [
    String.raw`C:\x y\a.rs`,
    String.raw`d\ e.rs`,
  ]);
});

test("parseDepInfo finds nothing in an empty file, and rejects what it doesn't know", () => {
  assert.deepEqual(parseDepInfo(""), []);
  assert.deepEqual(parseDepInfo("# only a comment\n"), []);
  assert.throws(() => parseDepInfo("this is not dep-info\n"));
});

for (const path of [
  "src/tests.rs",
  "src/engine_tests.rs",
  "src/engine-tests.rs",
  "src/Tests.rs",
  "src/tests/engine.rs",
  "src/tests/../engine.rs",
  "src/examples/a.rs",
  "src/benches/a.rs",
  String.raw`src\deep\tests\a.rs`,
  "../../other/tests/a.rs",
]) {
  test(`left out of coverage: ${path}`, () => {
    assert.equal(isLeftOutOfCoverage(pathSegments(path)), true);
  });
}

for (const path of [
  "src/lib.rs",
  "src/engine.rs",
  "src/engine/mod.rs",
  "src/contests.rs",
  "src/mytests/a.rs",
  "src/tests",
  "src/testsuite/a.rs",
]) {
  test(`measured: ${path}`, () => {
    assert.equal(isLeftOutOfCoverage(pathSegments(path)), false);
  });
}

test("a file only counts as a test file if coverage really leaves it out", () => {
  for (const path of ["src/tests.rs", "src/a_tests.rs", "src/a-b_tests.rs", "src/tests/a.rs", "src/examples/a.rs"]) {
    assert.equal(isTestFilePath(pathSegments(path)), true, path);
  }
  // Measured by cargo-llvm-cov, so a test in one of these would add to the score.
  for (const path of ["src/Tests.rs", "src/Tests/a.rs", "src/é_tests.rs", "src/a.rs", "src/a_tests.RS"]) {
    assert.equal(isTestFilePath(pathSegments(path)), false, path);
  }
});

test("show keeps untrusted text on one line, with no workflow command", () => {
  assert.equal(show('a\n::error::b "##[x]'), String.raw`"a\n::error::b \"#\u0023[x]"`);
});

const scratch = mkdtempSync(join(tmpdir(), "keitora-layout-"));
after(() => rmSync(scratch, { recursive: true, force: true }));

let workspaces = 0;

/**
 * A workspace laid out like this repository's: keitora-core and keitora-ipc, neither
 * with a placeholder test (no crate is in `PLACEHOLDER_CRATES`).
 */
class Workspace {
  constructor(parent = "plain") {
    workspaces += 1;
    this.dir = join(scratch, String(workspaces), parent);
    mkdirSync(this.dir, { recursive: true });
    cpSync(TOOLCHAIN, join(this.dir, "rust-toolchain.toml"));
    const lock = ["keitora-core", "keitora-ipc", "other"]
      .map((name) => `[[package]]\nname = "${name}"\nversion = "0.0.0"\n`)
      .join("\n");
    this.write({
      "Cargo.toml": WORKSPACE_MANIFEST,
      "Cargo.lock": `version = 4\n\n${lock}`,
      "crates/other/Cargo.toml": manifest("other"),
      "crates/other/src/lib.rs": "",
    });
    for (const name of ["keitora-core", "keitora-ipc"]) {
      this.write({
        [`crates/${name}/Cargo.toml`]: manifest(name),
        [`crates/${name}/src/lib.rs`]: `//! Docs.\n\n#![forbid(unsafe_code)]\n`,
      });
    }
  }

  /** Writes files; a null content deletes the file or directory. */
  write(files) {
    for (const [path, content] of Object.entries(files)) {
      const fullPath = join(this.dir, path);
      if (content === null) {
        rmSync(fullPath, { recursive: true, force: true });
      } else {
        mkdirSync(dirname(fullPath), { recursive: true });
        writeFileSync(fullPath, content);
      }
    }
  }

  /** Runs the script here. */
  guard(env = process.env, args = []) {
    return spawnSync(process.execPath, [SCRIPT, ...args], { cwd: this.dir, env, encoding: "utf8" });
  }

  /** Checks the script's exit code, showing its output when it is another one. */
  expect(status, env) {
    const result = this.guard(env);
    assert.equal(result.status, status, `${result.stdout}${result.stderr}`);
    return result;
  }
}

const WORKSPACE_MANIFEST =
  '[workspace]\nmembers = ["crates/keitora-core", "crates/keitora-ipc", "crates/other"]\nresolver = "3"\n';

/** The manifest of a package with this name. */
function manifest(name) {
  return `[package]\nname = "${name}"\nversion = "0.0.0"\nedition = "2024"\n`;
}

const CORE = "crates/keitora-core/src";
const IPC = "crates/keitora-ipc/src";

/** A crate root with the given code. */
const lib = (code) => `//! Docs.\n\n${code}\n`;

/** A crate root with sibling tests, and product code a build can see. */
const TESTED = "//! Docs.\n\n/// Docs.\npub fn a() {}\n\n#[cfg(test)]\nmod tests;\n";

/** A small source file. */
const A = "pub fn a() {}\n";

/**
 * Declares a test of one workspace holding many violations, which is much faster than a
 * workspace for each. Each case is a name that has to be listed, and the files for it.
 */
function violationBatch(title, code, cases) {
  test(`exit 1: ${title}`, () => {
    const workspace = new Workspace();
    workspace.write({ [`${CORE}/lib.rs`]: code });
    for (const [, files] of cases) {
      workspace.write(files);
    }
    const { stderr } = workspace.expect(VIOLATION);
    for (const [listed] of cases) {
      assert.ok(stderr.includes(listed), `${listed} is not listed in: ${stderr}`);
    }
  });
}

test("exit 0: neither crate has a placeholder", () => {
  const { stdout } = new Workspace().expect(PASS);
  assert.match(stdout, /all fine/);
});

test("exit 1: the placeholder in keitora-core, which has real code now", () => {
  const workspace = new Workspace();
  workspace.write({ [`${CORE}/lib.rs`]: lib(PLACEHOLDER) });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /keitora-core.*test attribute/);
});

test("exit 0: sibling test files, in each accepted form", () => {
  const workspace = new Workspace();
  workspace.write({
    [`${CORE}/lib.rs`]: "//! Docs.\n\npub mod engine;\npub mod rules;\n",
    [`${CORE}/engine.rs`]: "//! Docs.\n\n#[cfg(test)]\nmod tests;\n",
    [`${CORE}/engine/tests.rs`]: "#[test]\nfn a() {}\n",
    [`${CORE}/rules/mod.rs`]: "//! Docs.\n\n#[cfg(test)]\nmod tests;\n",
    [`${CORE}/rules/tests/mod.rs`]: "mod more;\n#[test]\nfn a() {}\n",
    [`${CORE}/rules/tests/more.rs`]: "#[test]\nfn b() {}\n",
    [`${IPC}/lib.rs`]: `#[path = "../other/a b.rs"]\npub mod a;\n`,
    "crates/keitora-ipc/other/a b.rs": A,
  });
  workspace.expect(PASS);
});

test("exit 0: a clone in a path with spaces, é and a parent directory named tests", () => {
  const workspace = new Workspace(join("tests", "examples", "dir con é"));
  workspace.write({
    [`${CORE}/lib.rs`]: lib(
      '#[path = "a b.rs"]\npub mod a;\ninclude!(concat!(env!("CARGO_MANIFEST_DIR"), "/src/c.rs"));',
    ),
    [`${CORE}/a b.rs`]: A,
    [`${CORE}/c.rs`]: "pub fn c() {}\n",
  });
  workspace.expect(PASS);
});

test("exit 0: what tests and the library may read", () => {
  const workspace = new Workspace();
  workspace.write({
    "crates/keitora-core/Cargo.toml": `${manifest("keitora-core")}\n[features]\nx = []\n`,
    // A #[cfg(unix)] module is in neither build, and a feature's module in both.
    [`${CORE}/lib.rs`]: `#![doc = include_str!("../README.md")]\n#[cfg(unix)]\nmod only_unix;\n#[cfg(feature = "x")]\npub mod gated;\npub mod shared;\n${TESTED.replace("//! Docs.\n", "")}`,
    "crates/keitora-core/README.md": "Docs.\n",
    [`${CORE}/only_unix.rs`]: A,
    [`${CORE}/gated.rs`]: A,
    [`${CORE}/shared.rs`]: A,
    // Test code under test names, and data, also from outside the crate.
    [`${CORE}/tests.rs`]:
      '#[path = "tests/support.rs"]\nmod support;\n#[test]\nfn t() {\n    support::s();\n    assert!(include_str!("a.json").is_empty());\n    assert!(include_str!("../../../fixtures/a.json").is_empty());\n}\n',
    [`${CORE}/tests/support.rs`]: "pub fn s() {}\n",
    [`${CORE}/a.json`]: "",
    "fixtures/a.json": "",
    "crates/keitora-core/tests/acceptance_t1_1.rs":
      "#![cfg(test)]\n#[test]\nfn t() {\n    keitora_core::a();\n}\n",
    // The crate's own target directory isn't source.
    "crates/keitora-core/target/x.rs": "#[cfg(not(test))]\nfn a() {}\n",
  });
  workspace.expect(PASS);
});

test("exit 0: `build.build-dir` in the configuration doesn't move what this reads", () => {
  const workspace = new Workspace();
  workspace.write({ ".cargo/config.toml": '[build]\nbuild-dir = "elsewhere"\n' });
  workspace.expect(PASS);
  workspace.write({ [`${CORE}/lib.rs`]: lib('include!("x_tests.rs");'), [`${CORE}/x_tests.rs`]: A });
  workspace.expect(VIOLATION);
});

// What the build reports, which the text rules can't see: all of it is in the library.
violationBatch(
  "what the library compiles, by name or place",
  lib(`#[path = "engine_tests.rs"] pub mod engine;
include!("x_tests.rs");
#[macro_use] mod m_tests;
real!();
#[path = "tests/a.rs"] pub mod in_tests;
#[path = "examples/a.rs"] pub mod in_examples;
#[path = "benches/a.rs"] pub mod in_benches;
#[path = "through/tests/../a.rs"] pub mod through;
#[path = "a-tests.rs"] pub mod hyphen;
#[path = "../../other/src/a.rs"] pub mod other_member;
#[path = "../../other/tests/a.rs"] pub mod other_tests;
#[path = "rustc/abc123/a.rs"] pub mod in_rustc;
pub const FROM_TESTS: &str = include_str!("tests/a.json");
pub const NOT_DATA: &str = include_str!("a.txt");
`),
  [
    ["engine_tests.rs", { [`${CORE}/engine_tests.rs`]: A }],
    ["x_tests.rs", { [`${CORE}/x_tests.rs`]: "pub fn x() {}\n" }],
    ["m_tests.rs", { [`${CORE}/m_tests.rs`]: "macro_rules! real { () => { pub fn r() {} }; }\n" }],
    ["tests\\\\a.rs", { [`${CORE}/tests/a.rs`]: A }],
    ["examples\\\\a.rs", { [`${CORE}/examples/a.rs`]: A }],
    ["benches\\\\a.rs", { [`${CORE}/benches/a.rs`]: A }],
    ["through\\\\tests\\\\..\\\\a.rs", { [`${CORE}/through/a.rs`]: A, [`${CORE}/through/tests/k.json`]: "" }],
    ["a-tests.rs", { [`${CORE}/a-tests.rs`]: A }],
    ["other\\\\src\\\\a.rs", { "crates/other/src/a.rs": A }],
    ["other\\\\tests\\\\a.rs", { "crates/other/tests/a.rs": A }],
    ["rustc\\\\abc123\\\\a.rs", { [`${CORE}/rustc/abc123/a.rs`]: A }],
    ["tests/a.json", { [`${CORE}/tests/a.json`]: "" }],
    ["a.txt", { [`${CORE}/a.txt`]: "" }],
  ],
);

test("exit 1: pub mod tests; without cfg(test)", () => {
  const workspace = new Workspace();
  workspace.write({ [`${CORE}/lib.rs`]: "pub mod tests;\n", [`${CORE}/tests.rs`]: A });
  workspace.expect(VIOLATION);
});

// Each file breaks a text rule, wherever it is in the crate; no build reads most of them.
violationBatch("what the source says, anywhere in the crate", lib(""), [
  ["inline.rs", { [`${IPC}/inline.rs`]: "#[cfg(test)]\nmod more {\n    #[test]\n    fn a() {}\n}\n" }],
  ["double.rs", { [`${IPC}/double.rs`]: "#[cfg(not(test))]\npub fn a() {}\n#[cfg(test)]\npub fn a() {}\n" }],
  ["cfg_attr.rs", { [`${IPC}/tests/cfg_attr.rs`]: "#[cfg_attr(test, derive(Debug))]\nstruct A;\n" }],
  ["nested.rs", { [`${IPC}/a/nested.rs`]: "#[test]\nfn a() {}\n" }],
  ["extra", { "crates/keitora-core/extra/x.rs": "#[test]\nfn t() {}\n" }],
  ["release.rs", { "crates/keitora-ipc/tests/release.rs": "#![allow(unused)]\n#[cfg(debug_assertions)]\nfn a() {}\nfn main() {}\n" }],
  ["integration.rs", { "crates/keitora-ipc/tests/integration.rs": "#[cfg(not(test))]\nfn a() {}\nfn main() {}\n" }],
  ["builder.rs", { "crates/keitora-core/extra/examples/builder.rs": "#[cfg_attr(test, derive(Debug))]\nstruct A;\n" }],
  ["path_inc.rs", { [`${IPC}/path_inc.rs`]: '#[path = "a.inc"]\nmod a;\n' }],
  ["include_inc.rs", { [`${IPC}/include_inc.rs`]: 'include!("a.inc");\n' }],
  ["select.rs", { [`${IPC}/select.rs`]: "core::cfg_select! { test => {} _ => {} }\n" }],
]);

test("exit 1: a compiled .rs file in the crate's target directory is still read", () => {
  const workspace = new Workspace();
  workspace.write({
    [`${CORE}/lib.rs`]: lib('#[path = "../target/a.rs"]\npub mod a;'),
    "crates/keitora-core/target/a.rs": "#[test]\nfn a() {}\n",
  });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /test attribute/);
});

violationBatch("what only tests read, under names coverage measures", TESTED, [
  ["support.rs", { [`${CORE}/support.rs`]: A }],
  ["Tests\\\\up.rs", { [`${CORE}/Tests/up.rs`]: A }],
  ["other\\\\src\\\\s.rs", { "crates/other/src/s.rs": A }],
  ["fixture.bin", { [`${CORE}/fixture.bin`]: "" }],
  ["neither Rust nor data", {}],
  [
    "pad_runner",
    {
      "crates/keitora-core/Cargo.toml": `${manifest("keitora-core")}\n[[test]]\nname = "pad"\npath = "pad_runner/run.rs"\nharness = false\n`,
      "crates/keitora-core/pad_runner/run.rs": "fn main() {}\n",
      [`${CORE}/tests.rs`]:
        '#[path = "support.rs"]\nmod a;\n#[path = "Tests/up.rs"]\nmod b;\n#[path = "../../other/src/s.rs"]\nmod c;\npub const D: &[u8] = include_bytes!("fixture.bin");\n',
    },
  ],
]);

test("exit 1: [profile.test] that differs from dev", () => {
  const workspace = new Workspace();
  workspace.write({ "Cargo.toml": `${WORKSPACE_MANIFEST}\n[profile.test]\ndebug-assertions = false\n` });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /profile\.test/);
});

test("exit 1: a build script, which doesn't run", () => {
  const workspace = new Workspace();
  workspace.write({
    "crates/keitora-core/build.rs": 'fn main() {\n    std::fs::write("ran", "").unwrap();\n}\n',
  });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /build script/);
  assert.equal(existsSync(join(workspace.dir, "crates/keitora-core/ran")), false);
});

test("exit 1: one crate's manifest problem stops the other's build, which would run it", () => {
  const workspace = new Workspace();
  workspace.write({
    "Cargo.lock":
      'version = 4\n\n[[package]]\nname = "keitora-core"\nversion = "0.0.0"\n\n[[package]]\nname = "keitora-ipc"\nversion = "0.0.0"\ndependencies = ["keitora-core"]\n\n[[package]]\nname = "other"\nversion = "0.0.0"\n',
    "crates/keitora-ipc/Cargo.toml": `${manifest("keitora-ipc")}\n[dependencies]\nkeitora-core = { path = "../keitora-core" }\n`,
    "crates/keitora-core/build.rs": 'fn main() {\n    std::fs::write("ran", "").unwrap();\n}\n',
  });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /build script/);
  assert.equal(existsSync(join(workspace.dir, "crates/keitora-core/ran")), false);
});

test("exit 1: a workspace member inside the crate", () => {
  const workspace = new Workspace();
  const lock = ["keitora-core", "keitora-ipc", "engine-shell", "other"]
    .map((name) => `[[package]]\nname = "${name}"\nversion = "0.0.0"\n`)
    .join("\n");
  workspace.write({
    "Cargo.toml":
      '[workspace]\nmembers = ["crates/keitora-core", "crates/keitora-core/src/engine", "crates/keitora-ipc", "crates/other"]\nresolver = "3"\n',
    "Cargo.lock": `version = 4\n\n${lock}`,
    [`${CORE}/engine/Cargo.toml`]: `${manifest("engine-shell")}\n[lib]\npath = "shell.rs"\n`,
    [`${CORE}/engine/shell.rs`]: "",
  });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /workspace member/);
});

test("exit 1: what is wrong in both crates is listed, with its line", () => {
  const workspace = new Workspace();
  workspace.write({
    [`${CORE}/lib.rs`]: lib("#[test]\nfn a() {}"),
    [`${IPC}/lib.rs`]: lib('include!("x_tests.rs");'),
    [`${IPC}/x_tests.rs`]: A,
  });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /keitora-core\\\\src\\\\lib\.rs":3:|keitora-core\/src\/lib\.rs":3:/);
  assert.match(stderr, /x_tests\.rs/);
});

test("exit 1: a hostile file name is printed on one line, with no workflow command", () => {
  const workspace = new Workspace();
  workspace.write({
    [`${CORE}/##[error]x_tests.rs`]: A,
    [`${CORE}/lib.rs`]: lib('include!("##[error]x_tests.rs");'),
  });
  const { stderr } = workspace.expect(VIOLATION);
  assert.doesNotMatch(stderr, /##\[/);
});

test("no stale result: a violation appears and disappears with the source", () => {
  const workspace = new Workspace();
  workspace.expect(PASS);
  workspace.write({ [`${CORE}/lib.rs`]: lib('include!("x_tests.rs");'), [`${CORE}/x_tests.rs`]: A });
  workspace.expect(VIOLATION);
  workspace.write({ [`${CORE}/lib.rs`]: lib(""), [`${CORE}/x_tests.rs`]: null });
  workspace.expect(PASS);
});

test("no stale result: a file changed with its old time, which cargo would not rebuild", () => {
  const workspace = new Workspace();
  const root = join(workspace.dir, CORE, "lib.rs");
  const longAgo = new Date("2020-01-01T00:00:00Z");
  utimesSync(root, longAgo, longAgo);
  workspace.expect(PASS);
  workspace.write({ [`${CORE}/lib.rs`]: lib('include!("x_tests.rs");'), [`${CORE}/x_tests.rs`]: A });
  utimesSync(root, longAgo, longAgo);
  utimesSync(join(workspace.dir, CORE, "x_tests.rs"), longAgo, longAgo);
  workspace.expect(VIOLATION);
});

test("exit 2: a stale dep-info file doesn't stand in for a build that fails", () => {
  const workspace = new Workspace();
  workspace.expect(PASS);
  workspace.write({ [`${CORE}/lib.rs`]: lib("this does not compile") });
  workspace.expect(CANNOT_CHECK);
});

for (const [name, change] of [
  ["a crate's directory is missing", { "crates/keitora-ipc": null }],
  ["a crate's src has no Rust files", { [`${IPC}/lib.rs`]: null, [`${IPC}/notes.txt`]: "x" }],
  [
    "the crate is not built from src/lib.rs",
    {
      "crates/keitora-core/Cargo.toml": `${manifest("keitora-core")}\n[lib]\npath = "other/lib.rs"\n`,
      "crates/keitora-core/other/lib.rs": A,
    },
  ],
  [
    "the workspace has another crate under that name",
    {
      "Cargo.toml":
        '[workspace]\nmembers = ["moved/keitora-core", "crates/keitora-ipc", "crates/other"]\nresolver = "3"\n',
      "moved/keitora-core/Cargo.toml": manifest("keitora-core"),
      "moved/keitora-core/src/lib.rs": A,
    },
  ],
  ["the library has another name", { "crates/keitora-core/Cargo.toml": `${manifest("keitora-core")}\n[lib]\nname = "renamed"\n` }],
  ["the library has no test build", { "crates/keitora-core/Cargo.toml": `${manifest("keitora-core")}\n[lib]\ntest = false\n` }],
  ["Cargo.lock doesn't match, because of --locked", { "Cargo.lock": "version = 4\n" }],
  ["a source file with an unterminated comment", { [`${IPC}/orphan.rs`]: "/* #[test] fn a() {}\n" }],
  ["a source file that isn't UTF-8", { [`${IPC}/orphan.rs`]: Buffer.from([0x2f, 0x2f, 0xff, 0x0a]) }],
]) {
  test(`exit 2: ${name}`, () => {
    const workspace = new Workspace();
    workspace.write(change);
    const { stderr } = workspace.expect(CANNOT_CHECK);
    assert.match(stderr, /Cannot check the unit test layout/);
  });
}

test("exit 2: run from anywhere but the workspace root", () => {
  const workspace = new Workspace();
  const result = spawnSync(process.execPath, [SCRIPT], {
    cwd: join(workspace.dir, "crates"),
    encoding: "utf8",
  });
  assert.equal(result.status, CANNOT_CHECK, result.stderr);
});

test("exit 2: arguments are not read", () => {
  const workspace = new Workspace();
  const result = workspace.guard(process.env, ["--skip"]);
  assert.equal(result.status, CANNOT_CHECK, result.stderr);
});

test("exit 2: cargo is not there", () => {
  const workspace = new Workspace();
  workspace.expect(CANNOT_CHECK, { ...process.env, PATH: scratch, Path: scratch });
});

test("exit 1 beats exit 2: a violation in one crate is listed if the other doesn't build", () => {
  const workspace = new Workspace();
  workspace.write({
    [`${CORE}/lib.rs`]: lib("this does not compile"),
    [`${IPC}/lib.rs`]: lib("#[test]\nfn a() {}"),
  });
  const { stderr } = workspace.expect(VIOLATION);
  assert.match(stderr, /Cannot check the unit test layout of keitora-core/);
});

test(
  "exit 1: a cargo.exe in the workspace does not run instead of cargo",
  { skip: process.platform !== "win32" },
  () => {
    const workspace = new Workspace();
    workspace.write({
      [`${CORE}/lib.rs`]: lib("#[macro_use]\nmod m_tests;"),
      [`${CORE}/m_tests.rs`]: "",
    });
    const fake = join(process.env.SystemRoot ?? "C:\\Windows", "System32", "whoami.exe");
    cpSync(fake, join(workspace.dir, "cargo.exe"));
    const env = Object.fromEntries(
      Object.entries(process.env).filter(([name]) => name !== "NoDefaultCurrentDirectoryInExePath"),
    );
    const result = workspace.guard(env);
    assert.equal(result.status, VIOLATION, result.stderr);
    assert.match(result.stderr, /m_tests\.rs/);
  },
);

test("the justfile runs the script as part of `just check`", () => {
  // A recipe line is one command, so that its exit code stops `just check`.
  const justfile = readFileSync(fileURLToPath(new URL("../justfile", import.meta.url)), "utf8");
  assert.match(justfile, /^ {4}node scripts\/check-unit-test-layout\.mjs$/m);
  assert.match(justfile, /^check: .*\bunit-test-layout\b/m);
});
