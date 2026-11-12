# Development workflow

Keitora is built mostly by AI agents under the direction of a maintainer who acts as **product owner**: the maintainer decides what gets built and accepts or rejects deliverables, while correctness and security are verified by automated checks and by agents that are independent from the one writing the code ([ADR-0008](adr/0008-agent-based-development-workflow.md)).

Human contributors follow the same quality gates through pull requests; see [CONTRIBUTING.md](../CONTRIBUTING.md).

## Roles

| Role | Who | Does | Never does |
|---|---|---|---|
| Maintainer | Human | Picks tasks, reads reports, runs manual checklists, approves | — |
| Orchestrator | Main Claude Code session | Runs the loop below, coordinates subagents, compiles the report | Write product code or tests itself |
| QA engineer | Subagent `qa-engineer` | Writes acceptance tests first; validates afterwards | Implement features |
| Developer | Subagent `developer` | Implements until tests and checks pass | Modify acceptance tests |
| Security reviewer | Subagent `security-reviewer` | Reviews against the threat model | Edit code |
| CI | GitHub Actions + `just check` | Deterministic checks | Have opinions |

Subagent definitions live in [`.claude/agents/`](../.claude/agents/).

## The task loop

The maintainer starts a task with: **"Run task T-x.y"** (or several, see *Running a group of tasks*). The orchestrator then:

1. **Prepare.** Check that the task's dependencies are done. Create a branch `task/t<phase>-<n>-<short-slug>` (e.g. `task/t1-3-pomodoro-state-machine`) from `main`. Mark the task `[~]` in the roadmap, and move its issue to *In progress* on the [project board](#project-board).
2. **Tests first.** Invoke `qa-engineer` in `write-tests` mode. Record the commit hash of the acceptance tests. If QA reports ambiguous criteria → stop and ask the maintainer.
3. **Implement.** Invoke `developer` with the task ID and the tests commit hash. If the developer reports a wrong test → stop and ask the maintainer.
4. **Automated checks.** Run `just check`. If anything fails, send the failure back to `developer` (step 3).
5. **Independent review.** Invoke `qa-engineer` in `validate` mode and `security-reviewer` in parallel. If either returns FAIL, send the findings to `developer` (step 3).
6. **Retry limit.** After **3** round-trips from step 4 or 5 back to step 3, stop and escalate to the maintainer with the reports so far.
7. **Report.** Compile the maintainer report (template below), move the task's issue to *Awaiting approval*, and **stop**. Do not merge.
8. **On approval** ("aprobado" / "approved"): tick the task `[x]`, update **Current status**, add ADRs the developer flagged, and push the branch. `CHANGELOG.md` isn't edited by hand: `release-please` generates it from the squash commit, so the type in the pull request's title, which becomes its subject, normally decides whether the change is listed (`feat`, `fix`, `perf`, `revert`) or not ([CONTRIBUTING.md](../CONTRIBUTING.md#what-decides-the-version)). Open a pull request to `main` titled with the squash commit's subject (`feat(core): add pomodoro state machine (T-1.3)`; GitHub appends ` (#N)`), with `Closes #<the task's issue>` in its description, wait for the required checks, squash-merge it on GitHub, and delete the branch ([CONTRIBUTING.md](../CONTRIBUTING.md#pull-requests) has the command, and how the subject and the DCO sign-off carry over). `main` takes no direct push: its ruleset's `pull_request` rule requires a pull request, with an empty bypass list.
   **On rejection:** move the task's issue back to *In progress*, take the maintainer's feedback to `developer` (step 3) and continue.

Reports to the maintainer are written in **Spanish**. Everything committed to the repository stays in English.

### Tasks without testable behavior

Some tasks (mostly in F0: tooling, configuration, CI) have no product behavior to test. For those, `qa-engineer` skips `write-tests` and instead lists in its validation the exact commands that prove the acceptance criteria (for example, `cargo build --workspace` succeeds), runs them, and reports their output. The rest of the loop is unchanged.

### Running a group of tasks

The maintainer can also start several consecutive tasks at once, usually one group from the roadmap: **"Run tasks T-x.y to T-x.z"**. The orchestrator then runs the loop for each task in roadmap order, with these changes:

- Each task gets its own branch, created from the previous task's branch instead of `main`, so it builds on work that is not merged yet.
- Step 7 does not stop: the orchestrator records the task's report and moves on to the next task.
- The orchestrator still stops early whenever a single task would: ambiguous criteria, a wrong test, the retry limit, or a decision only the maintainer can make. Actions outside the repository, such as changing GitHub settings, are confirmed with the maintainer before they are taken.
- After the last task, the orchestrator sends **one report** with a section per task, each following the template below, and stops.
- On approval, step 8 is done for each task in order, giving one pull request and one squash commit per task on `main`. CI only runs on pull requests to `main`, so a task's pull request opens when the one before it has merged: rebase the task's own commits onto `main` first (`git rebase --onto main <previous task's branch>`). If the maintainer rejects a task, the tasks before it can still be merged; the rejected task goes back to step 3, and the tasks after it are rebased onto the fixed branch and checked again.

Tasks are still run one at a time; this changes when the maintainer approves, not how many loops run in parallel (see *Working in parallel*).

## Project board

The public [Keitora project](https://github.com/users/CornelioDev/projects/4) shows the roadmap's tasks and the open issues. [docs/roadmap.md](roadmap.md) stays the source of truth for a task's scope, acceptance criteria and status; the board mirrors it, and when the two disagree the roadmap wins.

- **One issue per task.** From F2 on, every roadmap task has an issue titled `T-x.y: <short name>`. Its body holds the task's text and sub-bullets, with links to the issues it depends on. It is in the phase's milestone and has the area label. When a task is added to the roadmap, the orchestrator creates its issue and adds it to the board.
- **Status.** *Todo*, then *In progress* (steps 1 to 6), then *Awaiting approval* (step 7), then *Done*. The orchestrator moves the issue through the first three. A pull request with `Closes #N` closes the issue when it merges, and the board's built-in workflows set it to *Done*.
- **Other issues.** Security findings and other issues are added to the board too. The *Security* view lists the open issues labeled `security`.
- **Permissions.** Moving an item needs a token with the `project` scope. The orchestrator uses a separate classic token with that scope only, kept apart from the token it uses for the repository.

## Acceptance tests are protected

Acceptance tests are the contract between the maintainer and the developer, so only the QA engineer writes them:

| Code | Acceptance tests (QA only) | Unit tests (developer) |
|---|---|---|
| `keitora-core`, `keitora-ipc` | `crates/<crate>/tests/acceptance_t<phase>_<n>.rs` | Sibling file declared with `#[cfg(test)] mod tests;` (`src/engine.rs` with `src/engine/tests.rs`) |
| Other Rust crates | `crates/<crate>/tests/acceptance_t<phase>_<n>.rs` | `#[cfg(test)] mod tests` in the source file |
| Desktop UI | `apps/desktop/tests/acceptance/` | `*.spec.ts` next to the file |
| Extension | `extension/tests/acceptance/` | `*.spec.ts` next to the file |

`keitora-core` and `keitora-ipc` keep unit tests in separate files because `just coverage` doesn't measure those, so its 90 % covers production code only; they are still `#[cfg(test)]` modules, so the test lint exemptions in [AGENTS.md](../AGENTS.md#writing-rust-tests) apply. `just check` enforces this layout, and reviewers still look for what a build and a text scan can't see ([CONTRIBUTING.md](../CONTRIBUTING.md#development-setup)).

CI's `Acceptance tests` check on pull requests, which is `just check-acceptance-tests <base>` run from the base's copy of the script, fails if a commit changes these files without a subject starting with `test(`, or changes other files along with them ([CONTRIBUTING.md](../CONTRIBUTING.md#acceptance-tests-change-only-in-test-commits)). That catches accidents; it doesn't prove who wrote a commit.

QA validation fails automatically if acceptance tests changed after the QA commit. Changing them requires the maintainer's explicit approval, recorded in the report.

## Maintainer report template

```markdown
# Informe — T-x.y <título de la tarea>

**Veredicto:** ✅ Listo para aprobar | ❌ Necesita tu decisión
**Rama:** task/t…   **Intentos de corrección:** n de 3

## Qué se hizo
- (3–6 puntos en lenguaje claro, sin jerga)

## Calidad (agente de QA): PASS | FAIL
- Pruebas de aceptación: N escritas, N pasan. Criterios cubiertos: N de N.
- Verificación automática (`just check`): pasa | falla en …
- Observaciones: …

## Seguridad (agente de seguridad): PASS | FAIL
- Hallazgos: ninguno | lista con severidad y explicación en una frase.
- Qué se revisó: …

## Prueba manual (si aplica)
1. Paso… → **Resultado esperado:** …
2. …

## Decisiones que necesitan tu atención
- (vacío si no hay)

## Pendientes detectados (fuera de alcance)
- …
```

## Phase gates

When the last task of a phase is approved:

1. `security-reviewer` performs a **phase review** of everything built in the phase. All Medium findings are either fixed or opened as issues.
2. `qa-engineer` writes an **acceptance checklist** for the whole phase that the maintainer runs end to end.
3. The orchestrator closes the GitHub milestone, moves **Current status** to the next phase, and updates the phase status in [ROADMAP.md](../ROADMAP.md), the public summary.

## Working in parallel

Until **T-2.3** (the IPC protocol) is approved, run one task at a time. After it, up to three loops may run in parallel, one per front, each in its own git worktree:

| Front | Phases |
|---|---|
| Service | F2 (rest), F5 |
| Desktop UI | F3 |
| Extension | F4 |

`main`'s ruleset requires a branch to be up to date before merging, so each merge makes the other open branches update and run CI again. Revisit that requirement when parallel loops start.

Parallel loops must not change `keitora-ipc`; protocol changes go through a dedicated task on `main` that the other fronts then pull in. The maintainer still approves each task individually.

## Release gate

A release may be published as stable only after a **human** security review of `keitora-service`, `keitora-sys`, `keitora-ipc`, `keitora-nmh` and the installer. Until then, releases are tagged as pre-releases (`v0.x.y-beta.n`) and the README says so.

`release-please` is configured for this: it proposes `-beta.n` versions and marks those GitHub Releases as pre-releases. Configuration alone doesn't hold the gate: text in a commit message, such as a `Release-As:` footer, can override it and produce a version with no `-beta.n`. So whoever merges the release pull request confirms that its title has `-beta.` while the gate holds ([CONTRIBUTING.md](../CONTRIBUTING.md#merging-the-release-pull-request)). It keeps that pull request open, and by rule a release only happens when the maintainer merges it, which nothing enforces. Orchestrators and agents never merge it as part of a task.
