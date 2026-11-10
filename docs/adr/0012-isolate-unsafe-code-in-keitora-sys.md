# 0012 — Isolate `unsafe` code in a `keitora-sys` crate

- **Status:** Accepted
- **Date:** 2026-11-10

## Context

The service runs as `LocalSystem`, so a memory-safety bug in it is a privilege escalation. Its Win32 calls need `unsafe`, which the workspace sets to `deny`. A `deny` can be lifted with an `#[allow(unsafe_code, reason = "...")]` on any module, and `cargo clippy -- -D warnings` still passes, so the rule that `unsafe` lives only in `keitora-service/src/sys/` was enforced by review alone, in the one crate where it matters most ([#1](https://github.com/CornelioDev/keitora/issues/1), a Medium finding of the T-0.1 security review). The first Win32 code lands in T-2.1, and the native messaging host and the desktop app may later need Win32 calls of their own, such as checking the identity of the pipe server.

## Decision

- All `unsafe` code lives in **`crates/keitora-sys`**, the only crate whose code may contain it. It lifts the workspace's `deny` with one crate-level `#![allow(unsafe_code, reason = "...")]`.
- Every other crate (`keitora-core`, `keitora-ipc`, `keitora-service`, `keitora-nmh`, `keitora-desktop`) has `#![forbid(unsafe_code)]` at the root of its library and binaries, which no `#[allow]` can override. The compiler enforces the boundary.
- `keitora-sys` holds one safe public function per Win32 call, grouped in modules by API family (`library_loader`, `time`, `console`). Each `unsafe` block holds that one call (`clippy::multiple_unsafe_ops_per_block`) and a `// SAFETY:` comment (`clippy::undocumented_unsafe_blocks`). A wrapper turns a failure into a `Result` or `Option` and never panics; what to do about the failure is the caller's decision.
- Bindings come from **`windows-sys`**, with only the API families in use turned on. It is the raw binding crate that `windows-service` already uses, at the same 0.61 version, so there is one copy in the build. The higher-level `windows` crate would add a second binding crate for no gain at this size.
- `keitora-sys` has no Keitora dependencies and no policy: it doesn't know about sessions, logs or paths.
- Third-party crates keep their own `unsafe`, as every dependency does; they are vetted by `cargo deny` and by review when added. `windows-service`'s `define_windows_service!` macro, which creates the entry point the SCM calls, expands to an `unsafe` block inside `keitora-service`. The `unsafe_code` lint doesn't apply inside another crate's `macro_rules!` expansion, so `#![forbid(unsafe_code)]` accepts it. This is the only such case, and the code is `windows-service`'s, not ours.

## Alternatives considered

- **Keep `keitora-service/src/sys/` and add a `just check` step that fails on `allow(unsafe_code` or `expect(unsafe_code` anywhere else** — a text scan can be evaded (`cfg_attr`, a macro of our own) and covers only `just check`, not `cargo build`; the native messaging host and the desktop app would still have no safe place for a Win32 call.
- **Use the `windows` crate instead of `windows-sys`** — friendlier types, but a second set of bindings next to the one `windows-service` brings, and wrappers here are thin enough not to need it.

## Consequences

- A Win32 call needed anywhere means a new wrapper in `keitora-sys`, reviewed with extra care, and maybe a new `windows-sys` feature. The release gate's human security review covers `keitora-sys` ([workflow.md](../workflow.md#release-gate)).
- `keitora-service`, `keitora-nmh` and `keitora-desktop` can't contain `unsafe` even by mistake; a crate added later should get `#![forbid(unsafe_code)]` too, which only review checks.
