// SPDX-License-Identifier: MIT OR Apache-2.0
//
// Tests for run-on-staged.sh, with Node's own test runner. Each test builds a
// small repository in the system's temporary directory and runs the script in
// it, in the main checkout and in a linked worktree (where hooks get an
// absolute GIT_DIR and no GIT_WORK_TREE). `just check` runs them
// (`just test-scripts`).

import assert from "node:assert/strict";
import { spawnSync } from "node:child_process";
import { mkdirSync, mkdtempSync, realpathSync, rmSync, writeFileSync } from "node:fs";
import { tmpdir } from "node:os";
import { join } from "node:path";
import { after, test } from "node:test";
import { fileURLToPath } from "node:url";

const SCRIPT = fileURLToPath(new URL("run-on-staged.sh", import.meta.url));
const root = realpathSync(mkdtempSync(join(tmpdir(), "run-on-staged-")));
after(() => rmSync(root, { recursive: true, force: true }));

function git(cwd, ...args) {
  const result = spawnSync("git", args, { cwd, encoding: "utf8" });
  assert.equal(result.status, 0, result.stderr);
  return result.stdout;
}

/** Makes a repository with one commit and returns its path. */
function makeRepository(name) {
  const repository = join(root, name);
  mkdirSync(repository);
  git(repository, "init", "-q");
  git(repository, "config", "user.email", "test@example.com");
  git(repository, "config", "user.name", "Test");
  git(repository, "config", "commit.gpgsign", "false");
  writeFileSync(join(repository, "README.md"), "x\n");
  git(repository, "add", "README.md");
  git(repository, "commit", "-q", "-m", "init");
  return repository;
}

function stage(checkout, files) {
  for (const file of files) {
    const path = join(checkout, file);
    mkdirSync(join(path, ".."), { recursive: true });
    writeFileSync(path, "x\n");
  }
  git(checkout, "add", "--", ...files);
}

/** Runs the script with `printf '%s\n'` as the command, so the output lists the arguments. */
function run(checkout, args, env = {}) {
  const result = spawnSync("sh", [SCRIPT, ...args, "sh", "-c", 'pwd; printf "%s\\n" "$@"', "arg0"], {
    cwd: checkout,
    encoding: "utf8",
    env: { ...process.env, ...env },
  });
  const [directory, ...files] = result.stdout.split("\n").filter(Boolean);
  return { status: result.status, stderr: result.stderr, directory, files };
}

const FILES = ["apps/desktop/src/a.ts", "apps/desktop/src/b.vue", "apps/desktop/skip.ts", "other/c.ts", "top.ts"];

/** Checks of the script that must hold wherever the repository is checked out. */
function behaves(label, makeCheckout, hookEnv) {
  test(`${label}: only files under -C, relative to it, starting with ./`, () => {
    const checkout = makeCheckout();
    stage(checkout, FILES);
    const { status, stderr, directory, files } = run(checkout, ["-C", "apps/desktop", "-e", "ts vue"], hookEnv(checkout));
    assert.equal(status, 0, stderr);
    assert.ok(directory.endsWith("/apps/desktop") || directory.endsWith("\\apps\\desktop"), directory);
    assert.deepEqual(files.sort(), ["./skip.ts", "./src/a.ts", "./src/b.vue"]);
  });

  test(`${label}: -x leaves a file out, relative to -C`, () => {
    const checkout = makeCheckout();
    stage(checkout, FILES);
    const { files } = run(checkout, ["-C", "apps/desktop", "-x", "skip.ts", "-e", "ts"], hookEnv(checkout));
    assert.deepEqual(files, ["./src/a.ts"]);
  });

  test(`${label}: without -C, every staged file matches from the root`, () => {
    const checkout = makeCheckout();
    stage(checkout, FILES);
    const { files } = run(checkout, ["-e", "ts"], hookEnv(checkout));
    assert.deepEqual(files.sort(), [
      "./apps/desktop/skip.ts",
      "./apps/desktop/src/a.ts",
      "./other/c.ts",
      "./top.ts",
    ]);
  });

  test(`${label}: no matching file means the command does not run`, () => {
    const checkout = makeCheckout();
    stage(checkout, ["other/c.ts"]);
    const { status, directory } = run(checkout, ["-C", "apps/desktop", "-e", "ts"], hookEnv(checkout));
    assert.equal(status, 0);
    assert.equal(directory, undefined);
  });

  test(`${label}: a missing directory exits with 0`, () => {
    const checkout = makeCheckout();
    stage(checkout, ["top.ts"]);
    const { status, directory } = run(checkout, ["-C", "nope", "-e", "ts"], hookEnv(checkout));
    assert.equal(status, 0);
    assert.equal(directory, undefined);
  });

  test(`${label}: a hostile file name stays one argument`, () => {
    const checkout = makeCheckout();
    stage(checkout, ["apps/desktop/--emit=x'$(id)'.ts"]);
    const { files } = run(checkout, ["-C", "apps/desktop", "-e", "ts"], hookEnv(checkout));
    assert.deepEqual(files, ["./--emit=x'$(id)'.ts"]);
  });
}

let counter = 0;
behaves(
  "main checkout",
  () => makeRepository(`main-${counter++}`),
  () => ({}),
);

// A linked worktree, as a hook sees it: an absolute GIT_DIR, no GIT_WORK_TREE.
behaves(
  "linked worktree",
  () => {
    const main = makeRepository(`linked-main-${counter++}`);
    const worktree = join(root, `linked-${counter++}`);
    git(main, "worktree", "add", "-q", "-b", `branch-${counter}`, worktree);
    return worktree;
  },
  (worktree) => ({ GIT_DIR: git(worktree, "rev-parse", "--absolute-git-dir").trim() }),
);

test("exits with 2 and a usage message without a command", () => {
  const checkout = makeRepository("usage");
  const result = spawnSync("sh", [SCRIPT, "-e", "ts"], { cwd: checkout, encoding: "utf8" });
  assert.equal(result.status, 2);
  assert.match(result.stderr, /usage/);
});
