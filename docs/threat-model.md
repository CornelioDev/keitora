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
- The pipe is created with an explicit DACL: `SYSTEM` full access, the interactive user `0x12019b` (everything a client needs, without `FILE_CREATE_PIPE_INSTANCE`) and the account the service itself runs as; remote clients are rejected (`PIPE_REJECT_REMOTE_CLIENTS`).
- **Pipe instances (M1 and M1b), fixed in production:** the pipe's DACL used to give the interactive user `FRFW`, which includes `FILE_APPEND_DATA`, that is `FILE_CREATE_PIPE_INSTANCE` on a pipe, so a process of that user could create extra instances of `\.pipekeitora` and answer clients in the service's place (spoofed responses) or use up its instances. The interactive user's ACE is now `0x12019b`, which doesn't include that right (`keitora_service::pipe::pipe_sddl`), and clients open the pipe through `keitora_sys::named_pipe::open_client` with that explicit access mask (`CLIENT_ACCESS`) instead of tokio's `GENERIC_READ | GENERIC_WRITE` (issue #133). **Installed, the service runs as `SYSTEM`** and the DACL grants the right to create an instance to no user, so neither attack is open. **Under `--console`** (development) the service runs as the interactive user and needs an ACE of its own to create the instances it serves clients on, so that same user keeps the right and the development mode keeps the exposure by design; what is deployed to users does not. Measured on Windows 11 26200: Windows checks `FILE_CREATE_PIPE_INSTANCE` in `CreateNamedPipeW` only, not when a client opens the pipe's other end, so the narrow DACL is what closes both findings; asking for the narrow mask on the client side is defense in depth (every client asks for only what it needs) and a guard in case a future Windows build does check the right there. Impact where the exposure remains: a spoofing instance can read the contents of the requests of clients that connect to it (for example typed text or lock reasons the UI sends) and can show false state in the UI (for example "session ended"), but it cannot affect actual enforcement: blocking and strict mode live in the service and do not depend on the pipe, so this is not a strict-session bypass. Other mitigations, unchanged: the service creates the first instance with `FILE_FLAG_FIRST_PIPE_INSTANCE`, so no one can take the name over before it, and it never trusts what clients send; the instance count is capped (`max_instances` = 72) and the service serves at most 64 connections. Filling the instances to deny new clients now needs the create right, which an installed service grants nobody; under `--console` a process of the same user can still fill the slots and make the service's own attempts to create the next instance fail (it retries every 500 ms). Administrators are out of scope either way: they can take ownership of any object and rewrite its DACL.
- `%ProgramData%\Keitora` is writable only by `SYSTEM`, preventing DLL planting and database tampering. This holds from T-5.3 on; until then any standard user can write there (see the logs below).
- The service binary is installed under `Program Files` (admin-only write).
- `unsafe` code is confined to `crates/keitora-sys`, one safe wrapper per Win32 call, and reviewed with extra care; every other crate has `#![forbid(unsafe_code)]`, so the compiler enforces the boundary ([ADR-0012](adr/0012-isolate-unsafe-code-in-keitora-sys.md)).
- The first thing the service does is restrict DLL loading to `System32` (`SetDefaultDllDirectories(LOAD_LIBRARY_SEARCH_SYSTEM32)`), so a DLL planted in its folder, the working folder or `PATH` isn't loaded by name; if that fails, it refuses to run. DLLs in the executable's import table are resolved before that, which the `Program Files` install covers.
- Logs never hold blocklists or other user data. A log folder that can't be used doesn't stop the service: otherwise breaking it would switch off blocking. The service only appends to today's file in `logs` and deletes the oldest files whose names start with `keitora` and end with `log` beyond 14, oldest by creation time.
- **Open until T-5.3:** `C:\ProgramData` grants `BUILTIN\Users:(CI)(WD,AD,WEA,WA)`, which `Keitora` and `logs` inherit even when `SYSTEM` creates them. So any standard user can, at any time, not only before installation:
  - create files in `logs`, or with Developer Mode symbolic links, such as tomorrow's `keitora.<date>.log`, which the service would then open and append to;
  - hold an opportunistic lock on a planted file, so that the appender's open or delete hangs while it holds its lock, and with it every thread that logs, since logs are written synchronously;
  - steer which files the retention deletes, through the creation times of planted files;
  - create `%ProgramData%\Keitora` before installation and point it or `logs` elsewhere with a junction;
  - plant `keitora.db`, the store's database, before the service creates it. Once `SYSTEM` has created it, Users can only read it and can't delete it. The store validates what it reads, refuses a schema, root pages or format other than its own, runs no SQL from the file, and refuses to run with an SQLite that has WAL, so a planted `keitora.db-wal` is never read ([ADR-0013](adr/0013-store-snapshot-in-place-and-never-reset.md)). But the data in a planted file, a strict session's snapshot included, is whatever its author chose;
  - plant `keitora.db-journal` before the database exists and keep it open without `FILE_SHARE_DELETE`. Under `journal_mode = PERSIST` it then stays the store's journal, so its owner can later write a hot journal into it, which `SYSTEM` plays back into the database at the next read on its running connection, so at any time while the service runs. That injects pages into a database `SYSTEM` created, after the store checked it.

  T-5.3 fixes this: the service creates `Keitora` with a protected, non-inherited DACL (`SYSTEM` and Administrators full control, Users read only), checks or resets the owner and DACL if the folder already exists, refuses `Keitora` or `logs` if either is a reparse point, and falls back to logging on stderr if any of that fails. Before opening the store, it checks that `keitora.db` and its `-journal`, `-wal` and `-shm` files are absent or are regular files owned by `SYSTEM` or Administrators, with no write access for users, not reparse points, and with a link count of 1, and it refuses a foreign journal.
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
