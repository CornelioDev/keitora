// SPDX-License-Identifier: MIT OR Apache-2.0
//
// The rules of check-unit-test-layout.mjs; the top of that file says why they
// exist. This file reads no files, so its spec can try many Rust sources, dep-info
// files and manifests without running cargo.
//
// The text rules work on Rust tokens, not on lines, for two reasons. Formatting
// can't hide an attribute from them (`# [ test ]`, one spread over several lines,
// a comment in the middle), and text in a comment or a string can't trigger them
// (the placeholder's doc comment, `"#[test]"`).
//
// tokenize() is written to agree with rustc's lexer on where comments and strings
// end, because that is where a real `#[test]` could hide: nested block comments, raw
// strings with any number of `#`, `'"'` as a character rather than the start of a
// string, an identifier right after a literal as its suffix. Input it can't be sure
// of, such as an unterminated string or comment, is an error and not a guess: the
// check then ends without a verdict. The passes over the tokens are linear, so that a
// large file can't make the check crawl, with one exception: the walk back from the end of
// an `include!`, which is quadratic in how deeply they nest; a job timeout covers that.

import { dirname, isAbsolute, relative } from "node:path";

/** Formats untrusted text as one line, with nothing GitHub Actions would act on. */
export function show(text) {
  return JSON.stringify(text).replaceAll("##[", "#\\u0023[");
}

const WHITESPACE = /\p{Pattern_White_Space}+/uy;
const IDENTIFIER = /[\p{XID_Start}_]\p{XID_Continue}*/uy;
const NUMBER = /[0-9]\p{XID_Continue}*/uy;
const RAW_STRING_START = /(#*)"/y;

/** Returns what `pattern`, a sticky regular expression, matches at `index`. */
function matchAt(pattern, source, index) {
  pattern.lastIndex = index;
  return pattern.exec(source)?.[0];
}

/** Returns the index after a block comment that starts at `start`; they nest. */
function skipBlockComment(source, start) {
  let depth = 1;
  let index = start + 2;
  while (depth > 0) {
    if (index >= source.length) {
      throw new Error("a block comment is not closed");
    }
    if (source.startsWith("/*", index)) {
      depth += 1;
      index += 2;
    } else if (source.startsWith("*/", index)) {
      depth -= 1;
      index += 2;
    } else {
      index += 1;
    }
  }
  return index;
}

/** Returns the index after a string with escapes whose opening quote is at `start`. */
function skipString(source, start) {
  for (let index = start + 1; index < source.length; index += 1) {
    if (source[index] === "\\") {
      index += 1;
    } else if (source[index] === '"') {
      return index + 1;
    }
  }
  throw new Error("a string is not closed");
}

/** Returns the index after a raw string, or nothing if `start` doesn't begin one. */
function skipRawString(source, start) {
  const opening = matchAt(RAW_STRING_START, source, start);
  if (opening === undefined) {
    return undefined;
  }
  const closing = `"${opening.slice(0, -1)}`;
  const end = source.indexOf(closing, start + opening.length);
  if (end === -1) {
    throw new Error("a raw string is not closed");
  }
  return end + closing.length;
}

/** Returns the index after a character literal or lifetime whose quote is at `start`. */
function skipQuote(source, start) {
  const next = source.codePointAt(start + 1);
  if (next === undefined) {
    throw new Error("a character literal is not closed");
  }
  if (next === 0x5c) {
    // An escape: `'\''`, `'\\'`, `'\u{1F600}'`. None of them holds a line break.
    for (let index = start + 1; index < source.length && source[index] !== "\n"; index += 1) {
      if (source[index] === "\\") {
        index += 1;
      } else if (source[index] === "'") {
        return index + 1;
      }
    }
    throw new Error("a character literal is not closed");
  }
  const afterFirst = start + 1 + String.fromCodePoint(next).length;
  if (next !== 0x27 && source[afterFirst] === "'") {
    return afterFirst + 1;
  }
  const lifetime = matchAt(IDENTIFIER, source, start + 1);
  if (lifetime === undefined) {
    throw new Error("a character literal is not closed");
  }
  // `'r#name` is one raw lifetime: the `r` can't start a raw string or a raw identifier.
  const afterName = start + 1 + lifetime.length;
  const rawName =
    lifetime === "r" && source[afterName] === "#"
      ? matchAt(IDENTIFIER, source, afterName + 1)
      : undefined;
  return rawName === undefined ? afterName : afterName + 1 + rawName.length;
}

/**
 * Splits Rust source into its tokens, leaving out whitespace and comments. Returns
 * `{ kind, text, index }`, where `kind` is `ident` (raw identifiers lose their `r#`,
 * as `r#test` is `test`), `punct` (one character, so `::` is two tokens) or `literal`
 * (a string, character, number or lifetime, whose `text` is empty so that it never
 * matches a name, and whose `raw` is what the source says).
 *
 * Throws if a comment, a string or a character literal has no end.
 */
export function tokenize(source) {
  const tokens = [];
  // rustc reads an identifier right after any literal as its suffix, even where it
  // later rejects it: in `"a"r"\"x"`, the `r` doesn't start a raw string.
  const literal = (start, end) => {
    const suffix = matchAt(IDENTIFIER, source, end) ?? "";
    tokens.push({ kind: "literal", text: "", raw: source.slice(start, end), index: start });
    return end + suffix.length;
  };
  // A byte order mark is only skipped at the very start.
  let index = source.startsWith("﻿") ? 1 : 0;
  while (index < source.length) {
    const start = index;
    const space = matchAt(WHITESPACE, source, index);
    const number = matchAt(NUMBER, source, index);
    const name = matchAt(IDENTIFIER, source, index);
    if (space !== undefined) {
      index += space.length;
    } else if (source.startsWith("//", index)) {
      // Only a line feed ends a line comment, for rustc too: not a lone `\r`.
      const lineEnd = source.indexOf("\n", index);
      index = lineEnd === -1 ? source.length : lineEnd + 1;
    } else if (source.startsWith("/*", index)) {
      index = skipBlockComment(source, index);
    } else if (source[index] === '"') {
      index = literal(start, skipString(source, index));
    } else if (source[index] === "'") {
      index = literal(start, skipQuote(source, index));
    } else if (number !== undefined) {
      index += number.length;
      tokens.push({ kind: "literal", text: "", raw: number, index: start });
    } else if (name !== undefined) {
      index += name.length;
      const rawStringEnd = ["r", "br", "cr"].includes(name)
        ? skipRawString(source, index)
        : undefined;
      const rawIdentifier =
        name === "r" && source[index] === "#" ? matchAt(IDENTIFIER, source, index + 1) : undefined;
      if (rawStringEnd !== undefined) {
        index = literal(start, rawStringEnd);
      } else if (rawIdentifier !== undefined) {
        index += 1 + rawIdentifier.length;
        tokens.push({ kind: "ident", text: rawIdentifier.normalize("NFC"), index: start });
      } else if ((name === "b" || name === "c") && source[index] === '"') {
        index = literal(start, skipString(source, index));
      } else if (name === "b" && source[index] === "'") {
        index = literal(start, skipQuote(source, index));
      } else {
        // rustc compares identifiers in NFC, so this is the form it would see.
        tokens.push({ kind: "ident", text: name.normalize("NFC"), index: start });
      }
    } else {
      const text = String.fromCodePoint(source.codePointAt(index));
      index += text.length;
      tokens.push({ kind: "punct", text, index: start });
    }
  }
  return tokens;
}

/**
 * The crates whose `src/lib.rs` may still hold the placeholder test. A crate leaves this
 * list, and deletes its placeholder, once it has tested code: the placeholder only
 * gives `just coverage` a line to measure in a crate that has none.
 */
export const PLACEHOLDER_CRATES = new Set();

/** The one inline test allowed, in the `src/lib.rs` of the crates above, until it is deleted. */
const PLACEHOLDER = tokenTexts(`
  #[cfg(test)]
  mod tests {
      #[test]
      fn gives_the_coverage_gate_a_line_to_measure() {}
  }
`);

/** The only `#[cfg(test)]` a measured file may have, apart from the placeholder's. */
const CFG_TEST_ON_MOD_TESTS = tokenTexts("#[cfg(test)] mod tests;");

/**
 * Names a `cfg` can't depend on, in any form or file, because they differ between the
 * build `just coverage` measures and the builds it doesn't: cargo-llvm-cov sets
 * `coverage` and `coverage_nightly`, and a release build turns `debug_assertions` off.
 */
const CFG_BANNED = new Set(["coverage", "coverage_nightly", "debug_assertions"]);

/** Macros that choose code by `cfg` in a syntax of their own; `cfg_select!` is stable. */
const CFG_MACROS = new Set(["cfg_select", "cfg_match", "cfg_if"]);

/** Attribute names that make a test: `test`, `bench`, `test_case`, and dependencies' `rstest`. */
const TEST_ATTRIBUTE = /test|bench/i;

/** Names a `use` could rename, so that the rules on them no longer see them. */
const RENAMABLE_NAMES = new Set(["test", "bench", "test_case", "include"]);

/**
 * The ones a test file can't rename either: tests belong there, but an `include!` under
 * another name could compile a data file, whose lines coverage measures.
 */
const RENAMABLE_IN_TEST_FILES = new Set(["include"]);

/** A string literal, raw or not, that names a Rust file. */
const NAMES_RUST_FILE = /\.rs"#*$/i;

function tokenTexts(source) {
  return tokenize(source).map((token) => token.text);
}

/** Tells if the tokens at `start` are exactly `texts`; literals never match. */
function matchesAt(tokens, start, texts) {
  return texts.every((text, offset) => {
    const token = tokens[start + offset];
    return token !== undefined && token.kind !== "literal" && token.text === text;
  });
}

/** Maps each bracket's index to its partner's, in one pass. Throws if they don't match. */
function matchBrackets(tokens) {
  const closers = { "(": ")", "[": "]", "{": "}" };
  const closing = new Set(Object.values(closers));
  const partners = new Map();
  const open = [];
  tokens.forEach(({ kind, text }, index) => {
    if (kind !== "punct") {
      return;
    }
    if (closers[text] !== undefined) {
      open.push(index);
    } else if (closing.has(text)) {
      const opener = open.pop();
      if (opener === undefined || closers[tokens[opener].text] !== text) {
        throw new Error("brackets don't match");
      }
      partners.set(opener, index);
      partners.set(index, opener);
    }
  });
  if (open.length > 0) {
    throw new Error("a bracket is not closed");
  }
  return partners;
}

/**
 * Returns the last segment of the attribute whose `[` is at `open`: `test` for
 * `#[::core::prelude::v1::test]`. Returns nothing if it doesn't start with a path.
 */
function attributeName(tokens, open) {
  let index = open + 1;
  if (matchesAt(tokens, index, [":", ":"])) {
    index += 2;
  }
  let name;
  while (tokens[index]?.kind === "ident") {
    name = tokens[index].text;
    if (!matchesAt(tokens, index + 1, [":", ":"])) {
      break;
    }
    index += 3;
  }
  return name;
}

/** Returns the index of the `[` of the attribute whose `#` is at `index`, if one is there. */
function attributeBracket(tokens, index) {
  if (!matchesAt(tokens, index, ["#"])) {
    return undefined;
  }
  const open = matchesAt(tokens, index + 1, ["!"]) ? index + 2 : index + 1;
  return matchesAt(tokens, open, ["["]) ? open : undefined;
}

/** Tells if the token at `index` is the first one inside an attribute's brackets. */
function startsAttribute(tokens, index) {
  return (
    matchesAt(tokens, index - 1, ["["]) &&
    (matchesAt(tokens, index - 2, ["#"]) || matchesAt(tokens, index - 3, ["#", "!"]))
  );
}

/**
 * Returns the index of the bracket that opens the predicate of the `cfg` or `cfg_attr`
 * at `index`, or nothing if this is another use of the name. Attributes and `cfg!` take
 * any of the three brackets. Other than those, `cfg(` still counts, so that a macro's
 * argument such as `when!(cfg(not(test)))` is seen, unless it is `.cfg(` or `fn cfg(`.
 */
function cfgPredicate(tokens, index) {
  const isOpener = (at) => ["(", "[", "{"].some((text) => matchesAt(tokens, at, [text]));
  if (matchesAt(tokens, index + 1, ["!"])) {
    return isOpener(index + 2) ? index + 2 : undefined;
  }
  if (startsAttribute(tokens, index)) {
    return isOpener(index + 1) ? index + 1 : undefined;
  }
  const isOtherName = matchesAt(tokens, index - 1, ["."]) || matchesAt(tokens, index - 1, ["fn"]);
  return !isOtherName && matchesAt(tokens, index + 1, ["("]) ? index + 1 : undefined;
}

/** Tells if an outer attribute comes right before the attribute whose `#` is at `index`. */
function followsOuterAttribute(tokens, index, partners) {
  return (
    matchesAt(tokens, index - 1, ["]"]) && matchesAt(tokens, partners.get(index - 1) - 1, ["#"])
  );
}

/** Describes what is wrong with the predicate from `open` to `close` of the `cfg` at `index`. */
function cfgProblem(tokens, index, open, close, partners, isTestFile) {
  const inside = tokens.slice(open, close);
  if (inside.some((token) => token.kind === "punct" && token.text === "$")) {
    return "`cfg` with a `$`: a macro decides the condition, which this check can't read";
  }
  const names = inside.filter((token) => token.kind === "ident").map((token) => token.text);
  const banned = names.find((name) => CFG_BANNED.has(name));
  if (banned !== undefined) {
    return `\`cfg\` on \`${banned}\`: such code differs between what \`just coverage\` measures and the rest`;
  }
  if (!names.includes("test")) {
    return undefined;
  }
  if (tokens[index].text === "cfg_attr") {
    return "`cfg_attr` that depends on `test`, or that adds a test attribute";
  }
  if (names.includes("not")) {
    return "`cfg` with `not(test)`: such code is not in the build `just coverage` measures";
  }
  if (isTestFile) {
    return undefined;
  }
  const standsAlone =
    matchesAt(tokens, index - 2, CFG_TEST_ON_MOD_TESTS) &&
    !followsOuterAttribute(tokens, index - 2, partners);
  return standsAlone
    ? undefined
    : "`cfg(test)` is only allowed directly before `mod tests;`, with no other attribute; test code goes in that file";
}

/** Tells if the `include!` whose opening bracket is at `open` ends in a `.rs` name. */
function includesRustFile(tokens, open, partners) {
  let last = partners.get(open) - 1;
  while (
    last > open &&
    tokens[last].kind === "punct" &&
    [")", "]", "}", ","].includes(tokens[last].text)
  ) {
    last -= 1;
  }
  return last > open && tokens[last].kind === "literal" && NAMES_RUST_FILE.test(tokens[last].raw);
}

/** Tells if the `use` at `index`, up to its `;`, names one of `names`. */
function usesRenamableName(tokens, index, names) {
  // `impl Trait + use<'a>` is precise capturing, not an import.
  if (matchesAt(tokens, index + 1, ["<"])) {
    return false;
  }
  // Stopping at the next `use` as well keeps a run of them from taking quadratic time.
  for (let at = index + 1; at < tokens.length; at += 1) {
    const { kind, text } = tokens[at];
    if (text === ";" || text === "use") {
      return false;
    }
    if (kind === "ident" && names.has(text)) {
      return true;
    }
  }
  return false;
}

/**
 * Returns the broken rules in a Rust source file, as `{ index, reason }`, in the order
 * they come in `source`; `index` is where the rule is broken.
 *
 * `isTestFile` is for files `just coverage` doesn't measure, where tests belong: only
 * what makes production code differ under test is banned there. `mayHoldPlaceholder` is
 * for the `src/lib.rs` of a crate in `PLACEHOLDER_CRATES`, the one place for the
 * placeholder test.
 *
 * Throws if the source can't be read for sure: see tokenize().
 */
export function scanSource(source, { isTestFile, mayHoldPlaceholder }) {
  // After a `#!` that doesn't start an attribute, rustc skips the first line, and what
  // it counts as an attribute depends on the kinds of comment that follow. A library
  // has no use for one, so anything but the plain form is no verdict.
  if (/^﻿?#!/.test(source) && !/^﻿?#!\p{Pattern_White_Space}*\[/u.test(source)) {
    throw new Error("the file starts with a `#!` that isn't plainly an attribute");
  }
  const tokens = tokenize(source);
  const partners = matchBrackets(tokens);

  const placeholderStart =
    mayHoldPlaceholder && !isTestFile
      ? tokens.findIndex((_, index) => matchesAt(tokens, index, PLACEHOLDER))
      : -1;
  const isPlaceholder = (index) =>
    placeholderStart !== -1 &&
    index >= placeholderStart &&
    index < placeholderStart + PLACEHOLDER.length;

  const problems = [];
  const report = (index, reason) => problems.push({ index: tokens[index].index, reason });
  // Where the current attribute and predicate end. A predicate inside another was already
  // read as part of it, which also keeps nested ones from taking quadratic time.
  let attributeEnd = -1;
  let predicateEnd = -1;

  for (let index = 0; index < tokens.length; index += 1) {
    if (isPlaceholder(index)) {
      continue;
    }
    const { kind, text } = tokens[index];
    const open = attributeBracket(tokens, index);
    if (open !== undefined) {
      attributeEnd = Math.max(attributeEnd, partners.get(open));
      const name = attributeName(tokens, open);
      if (!isTestFile && name !== undefined && TEST_ATTRIBUTE.test(name)) {
        report(index, `test attribute \`${name}\` outside a test file`);
      } else if (!isTestFile && name === "coverage") {
        report(index, "`coverage` attribute: it leaves code out of `just coverage`");
      }
    }
    if (kind !== "ident") {
      continue;
    }
    if (text === "cfg" || text === "cfg_attr") {
      const predicate = index > predicateEnd ? cfgPredicate(tokens, index) : undefined;
      if (predicate !== undefined) {
        predicateEnd = partners.get(predicate);
        const problem = cfgProblem(tokens, index, predicate, predicateEnd, partners, isTestFile);
        if (problem !== undefined) {
          report(index, problem);
        }
      }
    } else if (CFG_MACROS.has(text)) {
      report(index, `\`${text}\` chooses code in a form this check doesn't read; use \`#[cfg]\``);
    } else if (text === "use") {
      if (usesRenamableName(tokens, index, isTestFile ? RENAMABLE_IN_TEST_FILES : RENAMABLE_NAMES)) {
        report(index, "`use` of a test attribute or `include!`, which could rename it");
      }
    } else if (
      text === "path" &&
      index < attributeEnd &&
      matchesAt(tokens, index + 1, ["="]) &&
      tokens[index + 2]?.kind === "literal" &&
      !NAMES_RUST_FILE.test(tokens[index + 2].raw)
    ) {
      report(index, "`path` names a file that isn't `.rs`, so this check wouldn't read its code");
    } else if (
      text === "include" &&
      matchesAt(tokens, index + 1, ["!"]) &&
      partners.has(index + 2) &&
      !includesRustFile(tokens, index + 2, partners)
    ) {
      report(index, "`include!` of a file that isn't plainly `.rs`, so this check wouldn't read it");
    }
  }
  return problems;
}

/**
 * Lists the files in a dep-info (`.d`) file exactly as rustc wrote them. Its lines are:
 *
 *     <output>: <input> <input>        rules: the inputs are what the compiler read
 *     <input>:                         an empty rule per input, adding nothing
 *     # env-dep:NAME=value             comments
 *
 * rustc writes a space in an input as `\ `, but leaves spaces in the output as they
 * are, so a rule is split at the first colon that ends a word: a drive letter's isn't
 * one, and Windows allows no colon in a file name. Throws on any other kind of line.
 */
export function parseDepInfo(text) {
  const files = new Set();
  for (const line of text.split(/\r?\n/)) {
    if (line === "" || line.startsWith("#")) {
      continue;
    }
    const separator = line.search(/:( |$)/);
    if (separator === -1) {
      throw new Error(`unexpected line in the dep-info file: ${show(line)}`);
    }
    // Between one space and the next, `\ ` is part of a name.
    for (const file of line.slice(separator + 1).split(/(?<!\\) /)) {
      if (file !== "") {
        files.add(file.replaceAll("\\ ", " "));
      }
    }
  }
  return [...files];
}

/** Splits a path at its separators, either kind; `.` and `..` stay in. */
export function pathSegments(path) {
  return path.split(/[/\\]+/).filter(Boolean);
}

/**
 * Tells if a compiled file's path could be left out of `just coverage`. These are the
 * parts of cargo-llvm-cov 0.9.1's filter (`report.rs`) that depend on a name. It applies
 * them to a file's path without resolving `..`:
 *
 *     \rustc\([0-9a-f]+|[0-9]+\.[0-9]+\.[0-9]+)\
 *     <workspace>(\.*)?\(tests|examples|benches)\
 *     <workspace>(\.*)?\(tests\.rs|[0-9a-zA-Z_-]+[_-]tests\.rs)$
 *
 * So `tests/../engine.rs` isn't measured, and neither is `engine-tests.rs`. The rest of
 * the filter is locations (other members, the target directory, the registry and the
 * toolchain), which the rule against files outside the crate covers. Letter case is
 * ignored, although cargo-llvm-cov looks at it, so that these names don't come to mean
 * something else to a reader on Windows; so are the limits on characters.
 */
export function isLeftOutOfCoverage(segments) {
  const name = segments.at(-1) ?? "";
  const directories = segments.slice(0, -1);
  return (
    /^(.*[_-])?tests\.rs$/i.test(name) ||
    directories.some((segment) => /^(tests|examples|benches)$/i.test(segment)) ||
    directories.some(
      (segment, index) =>
        /^rustc$/i.test(segment) &&
        /^([0-9a-f]+|[0-9]+\.[0-9]+\.[0-9]+)$/i.test(directories[index + 1] ?? ""),
    )
  );
}

/**
 * Tells if a file, given its path, is where test code may live. This is narrower than
 * isLeftOutOfCoverage(), in exactly the way that keeps it safe: every file it accepts is
 * one cargo-llvm-cov really leaves out, so that its lines can't add to the score.
 * `Tests.rs` is measured, for one, so test code can't go in it.
 */
export function isTestFilePath(segments) {
  const name = segments.at(-1) ?? "";
  return (
    /^(tests|[0-9a-zA-Z_-]+_tests)\.rs$/.test(name) ||
    segments.slice(0, -1).some((segment) => /^(tests|examples|benches)$/.test(segment))
  );
}

/**
 * Tells if a file a build read counts as data, the only kind other than Rust these
 * crates may read. dep-info can't tell `mod` and `include!` from `include_str!`, so a
 * file is judged by its extension here, and the text rules reject `#[path]` and
 * `include!` on anything but `.rs`, so that code can't be in a data file.
 */
export function isDataFile(name) {
  return /\.(json|md|toml)$/i.test(name);
}

/** The target kinds these crates may have: a library, and integration tests. */
const TARGET_KINDS = new Set(["lib", "test"]);

/** The path dependencies these crates may have, by crate. */
const PATH_DEPENDENCIES = { "keitora-core": [], "keitora-ipc": ["keitora-core"] };

/**
 * Returns what is wrong with a crate's manifest and place in the workspace, given
 * `cargo metadata`, as `problems`, plus the crate's package as `found`: its targets are
 * what the builds have to report. `realpath` turns a path from cargo into the form of
 * `crateDir`.
 *
 * Throws if the crate is not at `crateDir`.
 */
export function findManifestProblems(metadata, crateName, crateDir, realpath = (path) => path) {
  const directory = (manifest) => realpath(dirname(manifest));
  const byId = new Map(metadata.packages.map((item) => [item.id, item]));
  const members = metadata.workspace_members.map((id) => byId.get(id));
  const found = members.find(
    (item) => item?.name === crateName && directory(item.manifest_path) === crateDir,
  );
  if (found === undefined) {
    throw new Error(`no workspace member ${show(crateName)} at ${show(crateDir)}`);
  }
  const node = metadata.resolve?.nodes?.find((item) => item.id === found.id);
  if (node === undefined) {
    throw new Error(`cargo metadata did not resolve ${show(crateName)}`);
  }

  const problems = [];
  for (const target of found.targets) {
    for (const kind of target.kind) {
      if (!TARGET_KINDS.has(kind)) {
        const what = kind === "custom-build" ? "a build script" : `a \`${kind}\` target`;
        problems.push(`has ${what}, ${show(target.name)}: only a library and tests are expected`);
      }
    }
  }
  // Direct ones only: `thiserror` and `serde` reach theirs through a crate of their own.
  for (const { pkg } of node.deps) {
    const dependency = byId.get(pkg);
    // As resolved, not as written: `[patch]` can turn `foo = "1"` into a local crate.
    if (dependency?.source === null && !(PATH_DEPENDENCIES[crateName] ?? []).includes(dependency.name)) {
      problems.push(`has a local dependency, ${show(dependency.name)}: \`-p\` wouldn't measure it`);
    }
    if (dependency?.targets.some((target) => target.kind.includes("proc-macro"))) {
      problems.push(`has a proc-macro dependency, ${show(dependency.name)}: it runs in the build`);
    }
  }
  for (const member of members) {
    if (member && isInside(crateDir, directory(member.manifest_path))) {
      problems.push(`holds a workspace member, ${show(member.name)}, which \`-p\` wouldn't measure`);
    }
  }
  return { problems, found };
}

/** Tells if `path` is inside `directory`; both are real paths. */
export function isInside(directory, path) {
  const inside = relative(directory, path);
  return inside !== "" && !inside.startsWith("..") && !isAbsolute(inside);
}

/**
 * Picks the crate's builds from cargo's reports of the two runs: the library from the
 * first, and from the second the library compiled for tests and each test in
 * `testNames`. The second run may also build the library once, for integration tests to
 * link. There has to be exactly one of each and nothing else, because a compiler plugin
 * can print to the same stream as cargo. Throws otherwise.
 */
export function selectBuilds(libraryRun, testRun, libraryName, testNames) {
  const matching = (artifacts, kind, name, isTestBuild) =>
    artifacts.filter(
      (message) =>
        message.target.kind.includes(kind) &&
        message.target.name === name &&
        message.profile?.test === isTestBuild,
    );
  const only = (artifacts, kind, name, isTestBuild) => {
    const found = matching(artifacts, kind, name, isTestBuild);
    if (found.length !== 1) {
      throw new Error(
        `cargo reported ${found.length} builds of the ${kind} ${show(name)} (test: ${isTestBuild})`,
      );
    }
    return found[0];
  };
  if (libraryRun.length !== 1) {
    throw new Error(`cargo reported ${libraryRun.length} builds where one library was expected`);
  }
  const library = only(libraryRun, "lib", libraryName, false);
  const libraryForTests = only(testRun, "lib", libraryName, true);
  const tests = testNames.map((name) => only(testRun, "test", name, true));
  const linked = matching(testRun, "lib", libraryName, false).length;
  if (linked > 1 || testRun.length !== 1 + tests.length + linked) {
    throw new Error(`cargo reported ${testRun.length} test builds, not ${1 + tests.length}`);
  }
  return { library, libraryForTests, tests };
}
