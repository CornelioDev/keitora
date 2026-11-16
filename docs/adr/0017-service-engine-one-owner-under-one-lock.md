# 0017 — Run the service engine as one owner under one lock, observed on every call

- **Status:** Accepted
- **Date:** 2026-11-16

## Context

T-2.5 wires `keitora-core`'s engine into the service. Several parties touch it: the pipe server calls the request handler from up to 16 threads at once, the service's main loop must notice phase changes and the end of a session even when no client asks, and every change must reach the store ([ADR-0011](0011-validated-engine-snapshot.md), [ADR-0013](0013-store-snapshot-in-place-and-never-reset.md)) and the subscribers in the order it happened. The core engine's schedule moves with the clock alone, so a phase can change, or a session end, between two calls. A strict session must never run unless a reboot can restore it ([#48](https://github.com/CornelioDev/keitora/issues/48)), and the service must never overwrite a stored snapshot it couldn't read.

## Decision

- **One owner, one lock.** `keitora_service::engine::ServiceEngine` holds the core `Engine`, the `Store`, what the service knows of the session (its blocklist id, preset, lock, wall start time and `RandomText` text) and the checkpoint state, behind one `Mutex`. The service's one `SystemClock` is given to it when it is created. Every request and every tick takes the lock for its whole duration, store I/O included, and publishes its events while holding it, so changes, saves and events happen in one order. A poisoned lock is recovered: the core engine and the store change nothing on an error.
- **No thread of its own; observe on every call.** The engine does nothing between calls. The service's main loop calls `tick` once a second, inside `catch_unwind`. Every request and every tick first observes the clock: a phase or cycle change since the last one published is published as one `StateChanged` and saved; a session that ran out is recorded in the history as completed, published as `SessionEnded` and the idle state, and the idle snapshot is saved at once. A session that ended between ticks is therefore reported and recorded exactly once, before the next request is applied.
- **Strict start: clone, start, save, or revert.** Before a start the core engine and the checkpoint state are copied. If the new session is strict and its first snapshot isn't saved, both are put back and the start answers `internal`; nothing is published. The engine is never rebuilt with `Engine::new()` or `Engine::default()` in place of a running one; the copy is the engine exactly as it was.
- **Checkpoints and failures** are as ADR-0011's T-2.5 addendum records: every transition saves, a running session saves every 15 seconds, a failed save is owed and retried by every tick, an idle engine never saves over a stored snapshot.
- **No engine without a readable snapshot.** If the store can't be opened, or its snapshot can't be read, the service runs without the engine and without the pipe until it is stopped, and logs why. That protects the stored session from being overwritten, but it enforces nothing and answers no client. T-2.7 must revisit it: retry opening the store, and claim the pipe's name while the engine is down so that no one else serves it (security review M4).
- **Logs never wait for the disk.** Records go through `tracing_appender`'s non-blocking writer in lossy mode: a thread that logs, the engine's under its lock included, never waits on a log file, stderr or an oplock on a planted file, and records beyond the buffer are dropped. Each place that saves a snapshot logs from a line of its own, at `WARN` when saves start failing and at `DEBUG` while they keep failing.

### What the lock still waits on, and what T-2.6 must do

The engine's lock is held during SQLite calls, and a write waits up to SQLite's busy timeout (5 seconds) when another connection holds a lock on the database. Any process that can read `keitora.db` can take one, even with a plain read transaction, so this holds for as long as users can read the file, not only until T-5.3. Requests and ticks wait for that too. Keeping other processes off the file with `PRAGMA locking_mode = EXCLUSIVE` was tried and dropped: the service's own tests, and any tool, read the store through a second connection, which that mode locks out, and ADR-0013 already rejected it for growing the journal.

So T-2.6 must keep enforcement off this path: the process monitor never waits on the engine's lock, on SQLite or on logs. It reads what to enforce from a copy the engine publishes (for example the blocklist and phase, swapped atomically on every transition) and records block events without holding up a termination.

## Alternatives considered

- **An actor thread that owns the engine and receives messages** — the same ordering, plus a channel, a thread and a reply path for every request, with no gain while every caller needs an answer before it goes on.
- **Store I/O outside the lock** — shorter waits, but a save and the change it records could then reach the disk out of order, and a strict start's save must finish before the start is answered anyway. Left to T-2.6's needs.
- **Observing only in ticks** — a request between two ticks could then act on a session that already ended, or report it twice.
- **Refusing to start without a readable snapshot** — the service would stop, and the SCM restart it in a loop, instead of running and logging.

## Consequences

- Every request is serialized with every other one and with ticks; a slow store slows them all, by up to the busy timeout per write.
- Enforcement (T-2.6) needs a lock-free view of what to block, published by the engine.
- A store that can't be read leaves the machine unenforced until T-2.7 changes that.
- Under heavy logging, or while the log file is held, records are dropped rather than written late.
