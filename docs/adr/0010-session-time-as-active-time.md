# 0010 — Keep a session's time as active time on a fixed schedule

- **Status:** Accepted
- **Date:** 2026-11-10

## Context

The pomodoro engine (T-1.3) must report the phase, cycle and remaining time of a session at any moment, including a strict session whose end time is shown when it starts. The service may observe the engine late, the wall clock can be changed by the user, and the session must later survive a restart (T-1.7) without trusting a clock reading from before it.

## Decision

- A session's schedule is fixed when it starts. Every phase starts on its own; nothing waits for the user.
- The engine keeps the session's **active time**: time run, not paused, measured only on the monotonic clock. While running it stores the monotonic instant it started counting from plus the active time before it; while paused, just the frozen active time.
- `Engine::status` derives phase, cycle and remaining time from the active time with a few divisions, not a loop, so observing late is as cheap as observing on time and lands on the same phase.
- Only a pause or skipping a break changes the schedule. Skipping a short break starts a new segment at the next cycle with zero active time.
- All arithmetic saturates. A monotonic reading earlier than the stored instant counts as no time passing.

## Alternatives considered

- **Advance one phase per tick, each starting when it is noticed** — the schedule drifts later whenever the service is busy, and the end time shown at the start of a strict session becomes wrong.
- **Store wall-clock start and end times** — changing the system clock would end or shorten a session.

## Consequences

- The wall clock never affects a running session; it is only for history and display.
- A snapshot (T-1.7) stores the active time, the segment's first cycle, the preset and the lock, never a `MonotonicInstant`, whose origin changes at every service start.
- The service must give the engine one monotonic clock that never goes backwards or wraps (T-2.1, T-2.5): if it went backwards, a session would stand still until it caught up.
- Time while the machine is off is not counted by this model; whether and how to credit it is decided in [#19](https://github.com/CornelioDev/keitora/issues/19).

## Addendum (T-2.1): the service's monotonic source, and sleep

- The service's clock, `keitora_service::clock::SystemClock`, takes its monotonic reading from Windows' unbiased interrupt time (`QueryUnbiasedInterruptTime`, through `keitora-sys`, [ADR-0012](0012-isolate-unsafe-code-in-keitora-sys.md)): 64 bits in 100 ns units, so it doesn't wrap, and it ignores changes to the system time. The clock also never returns less than a reading it has already returned. `GetTickCount`, 32 bits, wraps after 49.7 days and is never used.
- That source stops while the machine sleeps or hibernates, so **time asleep or hibernated doesn't count toward a session's active time**, strict or not: a phase with 10 minutes left when the lid closes has 10 minutes left when the machine wakes up. This matches time while the machine is off, which isn't credited either ([ADR-0011](0011-validated-engine-snapshot.md)), and [#19](https://github.com/CornelioDev/keitora/issues/19) decides whether to credit either.
- `std::time::Instant` (QPC on Windows) was not chosen because the standard library leaves unspecified whether it counts time asleep, so the behavior couldn't be documented.
- The service creates one clock and shares it; no client chooses or moves it, and `keitora_core::clock::FakeClock` is banned outside tests in every crate but the pure ones, by `disallowed-types` in their `clippy.toml`.
