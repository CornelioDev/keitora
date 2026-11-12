# 0015 — Named-pipe server: byte mode, bounded resources and an accepted DACL risk

- **Status:** Accepted
- **Date:** 2026-11-12

## Context

T-2.4 adds the server for `\\.\pipe\keitora` to `keitora-service`. It runs as `SYSTEM`, any local client can connect, and every byte a client sends is untrusted ([ADR-0014](0014-ipc-protocol-wire-format.md), `docs/threat-model.md`). The documentation gave no numbers for the per-connection rate limit or the connection cap, and said the pipe used message mode. The acceptance tests split a frame across writes, and frames are lines of text up to 8 MiB.

## Decision

- **Byte mode**, not message mode: frames are newline-terminated lines that can be longer than a pipe message, and `FrameReader` already reassembles them from a stream. Remote clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`) on every instance.
- **DACL** `D:P(A;;FA;;;SY)(A;;FRFW;;;IU)`: SYSTEM full access, the interactive user read and write, nobody else, inheritance off. The service owns the first instance (`FILE_FLAG_FIRST_PIPE_INSTANCE`), so the name can't be taken over before it starts.
- **Limits:** 64 simultaneous connections (a semaphore; a connection beyond it is closed at once), 50 requests per second per connection with a burst of 100 (the server delays reading the next frame, so no new error code), 16 MiB of unsent data per connection (`2 * MAX_FRAME_LEN`; a connection over it is closed, so a slow reader never blocks `publish` or other clients), 4 idle instances waiting for clients, and an explicit `max_instances` of 72 (64 served, 4 idle, up to 4 more that exist briefly while a refused client's instance is dropped).
- **Runtime:** one dedicated thread with a current-thread `tokio` runtime. Handler calls run in `spawn_blocking`, so a slow handler doesn't stall other connections. The server answers `Subscribe` and every frame that fails to decode itself; the handler only sees valid requests.
- **`keitora-sys` depends on `tokio` (`net` only).** tokio's `ServerOptions::create_with_security_attributes_raw` is `unsafe`, and `keitora-service` forbids `unsafe`, so the one safe wrapper lives in `keitora-sys` ([ADR-0012](0012-isolate-unsafe-code-in-keitora-sys.md)), next to the one for `ConvertStringSecurityDescriptorToSecurityDescriptorW`.
- **Accepted risk (security review findings M1 and M1b).** `FRFW` includes `FILE_APPEND_DATA`, which on a pipe is `FILE_CREATE_PIPE_INSTANCE`, so a process of the interactive user can create extra instances of the pipe. It can read what clients send it and show them false state, and it can fill the 72 instances to deny new clients. It cannot affect blocking or strict mode, which don't depend on the pipe. The fix is a narrower ACE for the interactive user plus a safe client-open wrapper in `keitora-sys` (tokio's client can't ask for an explicit access mask), with a maintainer-approved change to the T-2.4 acceptance tests. It is tracked in issue #133 and must land before T-3.1 and T-4.1 write clients against the current DACL. The other Medium findings are issues #134 (idle timeout, per-client cap) and #135 (rate limit by bytes, decode off the runtime thread).

## Alternatives considered

- **Message mode:** keeps pipe-level framing, but frames can exceed a message and the protocol already frames by line.
- **Narrowing the DACL now:** closes M1 and M1b, but breaks 30 of the 31 acceptance tests (they run the server as the interactive user, which then can't create its own instances) and every tokio client, so it needs the wrapper and a test change; done as its own task.
- **Failing a request when the rate limit is hit:** needs a new `ProtocolError` code, and the client simply waits on a full pipe instead.
- **Letting the service call tokio's unsafe API:** forbidden by ADR-0012.
