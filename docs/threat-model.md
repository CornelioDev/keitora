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
- `unsafe` code is confined to `keitora-service/src/sys/` and reviewed with extra care.
- Release binaries are signed, and release artifacts carry build provenance attestations.

## Strict mode: bypass attempts and defenses

| Bypass attempt | Defense | Phase |
|---|---|---|
| Close the UI | Logic lives in the service | F2 |
| Kill the service | Runs as `SYSTEM`; automatic recovery | F2, F5 |
| Reboot | Persistent snapshot, restored on start | F2 |
| Change the system clock | Remaining time uses a monotonic clock | F5 |
| Disable or remove the extension | `ExtensionInstallForcelist` policy + heartbeat | F4, F5 |
| Incognito / InPrivate | Browser policies | F5 |
| Use another browser | Browsers without the extension are blocked as apps | F4 |
| Rename the executable | Match by path as well as name (MVP); hash/signer post-MVP. Launching through an 8.3 alias of the file name (`EPICGA~1.EXE`) evades name and path rules in the pure matcher, so the service expands short names before matching (T-2.6) | F1, F2 |
| Uninstall | MSI refuses while a strict session is active | F5 |
| Edit the database | `SYSTEM`-only ACL | F5 |
| Determined local administrator | **Out of scope.** The goal is to make bypassing costlier than waiting, not impossible. Users can run their daily account as a standard user for stronger guarantees. | — |

Because the code is public, defenses must not rely on obscurity: every mechanism above should hold even when the reader knows exactly how it works.
