// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Checks that the clippy.toml of every crate keeps up with the root one, with Node's own
// test runner. Clippy reads only the clippy.toml nearest a crate, so those files replace the
// root one instead of adding to it, and have to repeat its settings (see the top of
// crates/keitora-core/clippy.toml and crates/keitora-service/clippy.toml). These tests fail
// when they drift.
//
// `just check` runs them (`just test-scripts`).
//
// The files are read in the narrow form they are written in: comments on their own lines,
// booleans, numbers and strings as `key = value` on one line, and arrays with one
// `{ ... },` entry per line. Anything else fails the test, so that a change of format is
// noticed rather than read wrongly.

import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { test } from "node:test";
import { fileURLToPath } from "node:url";

/** Reads a file relative to the root of the workspace. */
function read(path) {
  return readFileSync(fileURLToPath(new URL(`../${path}`, import.meta.url)), "utf8");
}

const ROOT = "clippy.toml";
/** The pure crates' files, which ban `PURE_BANS`. */
const CRATES = ["crates/keitora-core/clippy.toml", "crates/keitora-ipc/clippy.toml"];
/** The other crates' files, which ban `PRODUCTION_BANS`. */
const PRODUCTION_CRATES = [
  "crates/keitora-service/clippy.toml",
  "crates/keitora-nmh/clippy.toml",
  "crates/keitora-sys/clippy.toml",
  "apps/desktop/src-tauri/clippy.toml",
];

/**
 * What the crates that aren't pure ban on top of the root file: the test clock of
 * keitora-core, which in the service would let whoever moves it end a strict session (T-2.1,
 * F1 security review).
 */
const PRODUCTION_BANS = {
  "disallowed-types": ["keitora_core::clock::FakeClock"],
};

/**
 * What the crates ban on top of the root file, as AGENTS.md's "Hard rules" say: no system
 * clock, no `std::process`, no network, no other OS dependencies (L5 and L2 of the security
 * reviews), no name resolution (L-3 of the T-1.4 review), and no path resolution: paths are
 * compared as text, never opened, resolved or looked at on disk (#11 in the threat model, T-1.5).
 */
const PURE_BANS = {
  "disallowed-methods": [
    "std::time::SystemTime::now",
    "std::time::SystemTime::elapsed",
    "std::time::Instant::now",
    "std::time::Instant::elapsed",
    "std::thread::sleep",
    "chrono::Utc::now",
    "chrono::Local::now",
    "time::OffsetDateTime::now_utc",
    "time::OffsetDateTime::now_local",
    "jiff::Timestamp::now",
    "jiff::Zoned::now",
    "std::process::Command::new",
    "std::process::exit",
    "std::process::abort",
    "std::process::id",
    "std::env::var",
    "std::env::var_os",
    "std::env::vars",
    "std::env::args",
    "std::fs::read",
    "std::fs::read_to_string",
    "std::fs::write",
    "std::fs::File::open",
    "std::fs::File::create",
    "std::fs::canonicalize",
    "std::path::Path::canonicalize",
    "std::path::absolute",
    "std::env::current_dir",
    "std::fs::read_link",
    "std::path::Path::read_link",
    "std::fs::metadata",
    "std::path::Path::metadata",
    "std::fs::symlink_metadata",
    "std::path::Path::symlink_metadata",
    "std::fs::exists",
    "std::path::Path::exists",
    "std::path::Path::try_exists",
    "std::path::Path::is_file",
    "std::path::Path::is_dir",
    "std::path::Path::is_symlink",
    "std::fs::read_dir",
    "std::path::Path::read_dir",
    "std::net::ToSocketAddrs::to_socket_addrs",
    "url::Url::socket_addrs",
  ],
  "disallowed-types": [
    "std::process::Command",
    "std::process::Child",
    "std::net::TcpStream",
    "std::net::TcpListener",
    "std::net::UdpSocket",
    "std::fs::File",
    "std::fs::OpenOptions",
  ],
};

/**
 * Parses a clippy.toml in the narrow form described at the top of this file, into a map
 * from each key to its value: a string for a scalar, an array of entry texts for an array.
 */
function parse(text) {
  const settings = new Map();
  let array = null;
  for (const [index, raw] of text.split("\n").entries()) {
    const line = raw.trim();
    const where = `line ${index + 1}: ${JSON.stringify(raw)}`;
    if (line === "" || line.startsWith("#")) {
      continue;
    }
    if (array !== null) {
      if (line === "]") {
        array = null;
      } else if (/^\{[^{}]*\},$/.test(line)) {
        array.push(line.slice(0, -1));
      } else {
        throw new Error(`not one \`{ ... },\` entry per line, ${where}`);
      }
      continue;
    }
    const match = /^([a-z-]+) = (.+)$/.exec(line);
    if (match === null) {
      throw new Error(`not \`key = value\`, ${where}`);
    }
    const [, key, value] = match;
    if (settings.has(key)) {
      throw new Error(`\`${key}\` is set twice, ${where}`);
    }
    if (value === "[") {
      array = [];
      settings.set(key, array);
    } else if (/^(true|false|\d+|"[^"]*")$/.test(value)) {
      settings.set(key, value);
    } else {
      throw new Error(`not a boolean, number, string or array on its own lines, ${where}`);
    }
  }
  if (array !== null) {
    throw new Error("an array isn't closed");
  }
  return settings;
}

/** The `path` of every entry in an array setting. */
function paths(entries) {
  return (entries ?? []).map((entry) => /path = "([^"]*)"/.exec(entry)?.[1]);
}

test("the clippy.toml of keitora-core and keitora-ipc are identical", () => {
  const [core, ipc] = CRATES.map(read);
  assert.equal(ipc, core, `${CRATES[1]} differs from ${CRATES[0]}`);
});

/** The only key a crate file may set that the root file doesn't. */
const CRATE_ONLY_KEYS = ["disallowed-types"];

/**
 * How a crate's clippy.toml differs from the root one, as a list of problems: a root
 * setting it lacks or sets differently, a root entry missing from one of its arrays, or a
 * key of its own other than those in `CRATE_ONLY_KEYS`, which could weaken a lint that the
 * root file leaves at its default.
 */
function drift(rootText, crateText) {
  const root = parse(rootText);
  const own = parse(crateText);
  const problems = [];
  for (const [key, value] of root) {
    const ownValue = own.get(key);
    if (!Array.isArray(value)) {
      if (ownValue !== value) {
        problems.push(`\`${key}\` differs from the root file`);
      }
    } else if (!Array.isArray(ownValue)) {
      problems.push(`\`${key}\` is missing or isn't an array`);
    } else {
      for (const entry of value.filter((rootEntry) => !ownValue.includes(rootEntry))) {
        problems.push(`\`${key}\` lacks the root entry ${entry}`);
      }
    }
  }
  for (const key of own.keys()) {
    if (!root.has(key) && !CRATE_ONLY_KEYS.includes(key)) {
      problems.push(`\`${key}\` isn't in the root file`);
    }
  }
  return problems;
}

for (const crate of CRATES) {
  test(`${crate} has exactly the settings of the root clippy.toml, plus its bans`, () => {
    assert.ok(parse(read(ROOT)).size > 0, "the root clippy.toml has no settings");
    assert.deepEqual(drift(read(ROOT), read(crate)), []);
  });

  test(`${crate} bans what a pure crate mustn't do`, () => {
    const own = parse(read(crate));
    for (const [key, banned] of Object.entries(PURE_BANS)) {
      const listed = paths(own.get(key));
      const missing = banned.filter((path) => !listed.includes(path));
      assert.deepEqual(missing, [], `\`${key}\` lacks bans`);
    }
  });
}

test("every workspace member has one of the clippy.toml checked here", () => {
  const members = /^members = \[\n((?:\s+"[^"]+",\n)*)\]$/m.exec(read("Cargo.toml"));
  assert.ok(members !== null, "the root Cargo.toml has no `members = [` list, one per line");
  const checked = [...CRATES, ...PRODUCTION_CRATES];
  for (const [, member] of members[1].matchAll(/"([^"]+)"/g)) {
    assert.ok(checked.includes(`${member}/clippy.toml`), `${member} has no clippy.toml checked here`);
  }
});

test("the clippy.toml of the crates that aren't pure are identical", () => {
  const [first, ...others] = PRODUCTION_CRATES.map(read);
  for (const [index, text] of others.entries()) {
    assert.equal(text, first, `${PRODUCTION_CRATES[index + 1]} differs from ${PRODUCTION_CRATES[0]}`);
  }
});

for (const crate of PRODUCTION_CRATES) {
  test(`${crate} has exactly the settings of the root clippy.toml, plus its bans`, () => {
    assert.deepEqual(drift(read(ROOT), read(crate)), []);
  });

  test(`${crate} bans what production code mustn't use`, () => {
    const own = parse(read(crate));
    for (const [key, banned] of Object.entries(PRODUCTION_BANS)) {
      const listed = paths(own.get(key));
      const missing = banned.filter((path) => !listed.includes(path));
      assert.deepEqual(missing, [], `\`${key}\` lacks bans`);
    }
  });
}

test("drift finds a root setting that the crate file lacks or changes", () => {
  const root = 'flag = true\nlist = [\n    { path = "a" },\n]\n';
  assert.deepEqual(drift(root, root), []);
  assert.deepEqual(drift(root, 'flag = false\nlist = [\n    { path = "a" },\n]\n'), [
    "`flag` differs from the root file",
  ]);
  assert.deepEqual(drift(root, 'list = [\n    { path = "a" },\n]\n'), [
    "`flag` differs from the root file",
  ]);
  assert.deepEqual(drift(root, "flag = true\nlist = [\n]\n"), [
    '`list` lacks the root entry { path = "a" }',
  ]);
  assert.deepEqual(drift(root, "flag = true\n"), ["`list` is missing or isn't an array"]);
});

test("drift finds a setting that only the crate file has, other than disallowed-types", () => {
  const root = "flag = true\n";
  const types = 'disallowed-types = [\n    { path = "a" },\n]\n';
  assert.deepEqual(drift(root, `flag = true\n${types}`), []);
  assert.deepEqual(drift(root, "flag = true\nallow-dbg-in-tests = true\n"), [
    "`allow-dbg-in-tests` isn't in the root file",
  ]);
  assert.deepEqual(drift(root, "flag = true\ntoo-many-arguments-threshold = 99\n"), [
    "`too-many-arguments-threshold` isn't in the root file",
  ]);
});

test("the parser rejects what it can't read, instead of skipping it", () => {
  assert.equal(parse("# a comment\n\nflag = true\n").get("flag"), "true");
  assert.equal(parse("threshold = 42\n").get("threshold"), "42");
  assert.equal(parse('name = "x"\n').get("name"), '"x"');
  assert.throws(() => parse("threshold = d\n"), /not a boolean/);
  assert.throws(() => parse("threshold = dd\n"), /not a boolean/);
  assert.deepEqual(parse('list = [\n    { path = "a" },\n]\n').get("list"), ['{ path = "a" }']);
  assert.throws(() => parse("[table]\n"), /not `key = value`/);
  assert.throws(() => parse("list = [\n    { path = \"a\" }, { path = \"b\" },\n]\n"), /entry per line/);
  assert.throws(() => parse('list = ["a", "b"]\n'), /not a boolean/);
  assert.throws(() => parse("list = [\n    { path = \"a\" },\n"), /isn't closed/);
  assert.throws(() => parse("flag = true\nflag = false\n"), /set twice/);
});
