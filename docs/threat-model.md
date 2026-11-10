# Threat model

Keitora has two very different kinds of "attack", and this project treats them differently.

| | Strict-mode bypass | Security vulnerability |
|---|---|---|
| Who | The user, against their own commitment | Anyone, against the user or their machine |
| Example | Killing the service as an administrator | A standard user gaining `SYSTEM` through the pipe |
| Severity | Feature limitation | Security issue |
| Report via | Public issue (label `strict-mode`) | Private report, see [SECURITY.md](../SECURITY.md) |

## Assets

- The integrity of an active strict session (the user's commitment).
- The machine itself: the service runs as `LocalSystem`, so it is a privilege boundary.
- The user's blocklists and history (personal, kept local).

## Security: the service as a privilege boundary

Because the service runs as `SYSTEM` and accepts input from user-level processes, it must never let that input reach a privileged action other than the ones it was designed for.

- Every IPC message is size-limited and validated before use; parsing is fuzzed (`cargo-fuzz`) post-MVP.
- Clients can never supply a path, command, or registry key that the service then executes or writes. Process blocking terminates matching processes; it never launches anything.
- The pipe is created with an explicit DACL (interactive user + `SYSTEM`); remote clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`).
- `%ProgramData%\Keitora` is writable only by `SYSTEM`, preventing DLL planting and database tampering.
- The service binary is installed under `Program Files` (admin-only write).
- `unsafe` code is confined to `crates/keitora-sys`, one safe wrapper per Win32 call, and reviewed with extra care; every other crate has `#![forbid(unsafe_code)]`, so the compiler enforces the boundary ([ADR-0012](adr/0012-isolate-unsafe-code-in-keitora-sys.md)).
- The first thing the service does is restrict DLL loading to `System32` (`SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)`), so a DLL planted in its folder, the working folder or `PATH` isn't loaded by name; if that fails, it refuses to run. DLLs in the executable's import table are resolved before that, which the `Program Files` install covers.
- Logs never hold blocklists or other user data. A log folder that can't be used doesn't stop the service: otherwise breaking it would switch off blocking. The service only appends to today's file there and deletes the oldest files whose names start with `keitora` and end with `log` beyond 14; until the folder's ACL is enforced (T-5.3), a standard user who created `%ProgramData%\Keitora` before installation could point `logs` somewhere else with a junction.
- Release binaries are signed, and release artifacts carry build provenance attestations.

## Strict mode: bypass attempts and defenses

| Bypass attempt | Defense | Phase |
|---|---|---|
| Close the UI | Logic lives in the service | F2 |
| Kill the service | Runs as `SYSTEM`; automatic recovery | F2, F5 |
| Reboot | Persistent snapshot, restored on start | F2 |
| Change the system clock | Remaining time uses a monotonic clock | F5 |
| Sleep or hibernate until the session is over | Time asleep doesn't count toward a session: the service's clock reads the unbiased interrupt time ([ADR-0010](adr/0010-session-time-as-active-time.md)) | F2 |
| Disable or remove the extension | `ExtensionInstallForcelist` policy + heartbeat | F4, F5 |
| Incognito / InPrivate | Browser policies | F5 |
| Use another browser | Browsers without the extension are blocked as apps | F4 |
| Rename the executable | Match by path as well as name (MVP); hash/signer post-MVP. Launching through an 8.3 alias of the file name (`EPICGA~1.EXE`) evades name and path rules in the pure matcher, so the service expands short names before matching (T-2.6) | F1, F2 |
| Uninstall | MSI refuses while a strict session is active | F5 |
| Edit the database | `SYSTEM`-only ACL | F5 |
| Determined local administrator | **Out of scope.** The goal is to make bypassing costlier than waiting, not impossible. Users can run their daily account as a standard user for stronger guarantees. | — |

Because the code is public, defenses must not rely on obscurity: every mechanism above should hold even when the reader knows exactly how it works.
