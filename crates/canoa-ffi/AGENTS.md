# `canoa-ffi` — API header

Header-file-style reference for the `canoa-ffi` crate: the C ABI an application uses to run the Canoa host inside its own process, so it can be used without reading the source. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). A `cdylib` (`canoa_ffi.dll`, `libcanoa_ffi.so`) and an `rlib`. It depends on `canoa-host`, `clap`, `serde`, `serde_json`, `tracing`, `tracing-subscriber`, and `rustix` on Unix or `windows-sys` on Windows. It builds with `panic = "unwind"` and refuses to build otherwise.

| Feature | Adds | Default |
| --- | --- | --- |
| `web` | `canoa-host`'s feature: the site `CANOA_WEB_DIST` names at build time, built in | off |

The only consumer is the app's plugin, `app/native`, whose `lib/canoa_native.dart` declares these functions; `app/test/host_library_test.dart` holds its status table and ABI version to the ones here. Unsafe code is in `src/exports.rs` and `src/platform.rs` alone, which a test checks.

## The C ABI (`canoa_ffi::exports`)

```c
uint32_t canoa_ffi_abi(void);                                                      // ABI_VERSION, 3; refuse any other
int32_t  canoa_ffi_job_start(const uint8_t *request, size_t len, uint64_t *out_job);
int32_t  canoa_ffi_job_read(uint64_t job, uint8_t **out_text, size_t *out_len);
void     canoa_ffi_text_free(uint8_t *text, size_t len);
int32_t  canoa_ffi_job_stop(uint64_t job);
int32_t  canoa_ffi_job_free(uint64_t job);
int32_t  canoa_ffi_link_open(uint64_t job, const uint8_t *entry, size_t entry_len, uint64_t *out_link);
int32_t  canoa_ffi_link_secret(uint64_t job, uint8_t *out, size_t len);            // len must be 32 (LINK_SECRET_LEN)
int32_t  canoa_ffi_link_send(uint64_t link, const uint8_t *message, size_t len);
int32_t  canoa_ffi_link_read(uint64_t link, uint8_t **out_text, size_t *out_len);
int32_t  canoa_ffi_link_close(uint64_t link);
int32_t  canoa_ffi_child_main(void);
```

| Status | Name | Meaning |
| --- | --- | --- |
| 0 | `STATUS_OK` | done |
| 1 | `STATUS_BAD_ARGUMENT` | a pointer null, misaligned or an alias of another, an out-parameter inside the request, a length too long, a request this library does not read |
| 2 | `STATUS_UNKNOWN_JOB` | no job has that number, or it was freed |
| 3 | `STATUS_FAILED` | a thread for the job could not be started, or this application's own executable cannot be found to run the host's process starter |
| 4 | `STATUS_PANIC` | the library panicked in this call |
| 5 | `STATUS_UNKNOWN_LINK` | no link has that number, or it was closed |
| 6 | `STATUS_NO_ENTRY` | the job is no `serve` yet, its host serves no entry of that name, or it is stopping |
| 7 | `STATUS_BUSY` | a link holds 64 MiB of unsent messages: send again once the host has taken some |
| 8 | `STATUS_ENDED` | the link has ended from either side: read what is left, then close it |

- **No function blocks**, none calls back, and no panic crosses: each body runs under `catch_unwind`, and each out-parameter that is non-null and aligned is written first with its empty value (0, null, 0), whatever else the call refuses; an `out_job` inside the request is refused unwritten. Call them from any thread.
- **A job is one `canoa` command**, on a thread of its own, numbered from 1; a number is never used twice, and a stale or invented one is `STATUS_UNKNOWN_JOB`.
- `canoa_ffi_job_start`: `request` is UTF-8 JSON, read only during the call, unknown keys refused:

| Key | Value | Default |
| --- | --- | --- |
| `args` | the command line without the program's name, as `canoa` takes it (see `crates/canoa-host/AGENTS.md`) | required |
| `home` | the directory of the identity, trust store and lock | this user's home |
| `holder` | what `serve` calls itself to another host that finds its home held | `an application` |
| `program` | what runs the host's own `provider`, `tool`, `launch`, `mcp-bridge` and `hook-forward` | this application, with `--canoa-child` first |
| `program_args` | what goes before the subcommand on `program`'s command line; only with `program` | none |

- `canoa_ffi_job_read`: what the job has said since the last read, `{"out": [line...], "err": [line...], "dropped": n, "exit": status or null}`, which `canoa_ffi_text_free` frees with the same length. `out` is what `canoa` prints on standard output, `err` what it says on standard error. `dropped` counts lines lost past 4096 unread of a kind, the oldest first. `exit` is set once the command has ended and has given up all it held, a home's lock and its addresses included; a command line `clap` refuses is status 2, a panicked command 101.
- `canoa_ffi_job_stop`: closes the job's input. `serve --until-stdin-closes` then ends every listener, link and agent, and `pair --until-stdin-closes` gives up. It returns at once; `exit` says when the job has ended. A command with no such flag does not hear it.
- `canoa_ffi_job_free`: forgets the job, closing its input first. A job still running goes on to its end with nobody reading it.
- `canoa_ffi_link_open`: a connection to entry `entry` (UTF-8) of the `serve` job `job` runs, inside this process: no proof, no trust store, no sealing, and the entry's `peers` list is ignored, since only this process can call it. The entry serves it as a paired key's connection, its agent starting on the first message. Two threads of the link's own move messages, so no call blocks. `canoa_ffi_link_send` copies one ACP message and queues it in order (a worktree request goes first, as `_canoa/worktree/*`); `canoa_ffi_link_read` takes `{"messages": [text...], "closed": bool}`, the host's side holding 1024 unread before it waits; `canoa_ffi_link_close` ends the link and its agent. Stopping the job closes every link, which then read `closed`.
- `canoa_ffi_link_secret`: writes the 32-byte secret the `serve` job's host minted for the links of this process, so the application can answer the host's secret prompts bound by it (`link-hmac-sha256`, `coqui_acp::secret::LinkKey`). The secret goes over no wire and to no agent, and is the application's to keep in memory only; the host drops it when the job ends. Statuses: `STATUS_OK`; `STATUS_BAD_ARGUMENT` for a null `out` or a `len` other than 32, with nothing written; `STATUS_UNKNOWN_JOB` for a number no job has (or a freed one); `STATUS_NO_ENTRY` for a job that is no `serve` yet, has ended or whose host has no secret; `STATUS_PANIC`. Every status but `STATUS_BAD_ARGUMENT` leaves `out` zeroed. Only the links this function's job opens are bound by it, a connection with a relay never is.
- `canoa_ffi_child_main`: this process as the host's own `provider`, `tool`, `agent`, `launch`, `mcp-bridge`, `hook-forward` or (Unix) `spawner`, from the process's command line, whose first argument is `--canoa-child` (`jobs::CHILD_FLAG`). Anything else is status 2. It returns the status to exit with, and is called before the application starts anything of its own.

## What a long command changes in this process

The first command in a process installs a `tracing` subscriber writing to the log (`CANOA_LOG` as in `canoa`, default `info`), which is delivered to every running `serve` and `pair` as `err`. The first `serve` or `pair`, and only those, also:

- puts this process's standard error on a pipe a thread reads for good, delivered the same way, so that an agent inheriting it is heard and never blocks. What the application itself writes to standard error goes there too, and no longer reaches where it went before;
- on Windows, gives a process with no console one made with no window, by attaching to a helper's, so that a console program it starts opens no window of its own.

- on Unix, sets `SIGPIPE` aside when the application left it as the system gives it, as a Rust program's `main` does, so that writing to an agent that has died is an error and not the application's end; a handler of the application's own stays.

The console comes before the pipe: attaching to one replaces the standard handles of a process started with none, as Explorer starts an application. These stay for the life of the process; stopping a host does not undo them, and the library is never to be unloaded, since its threads run on in it.

## Accounts as jobs

`canoa accounts list|add|login|remove` (see `canoa-host`'s header) run as jobs like any command that changes nothing about the process, with `home` naming the host's directory, where the accounts keep their state and, with `[host] account_store = "file"`, their sealed files.

- `args`: `["accounts", "add", NAME, "--harness", ID, "--method", METHOD, "--format", "json", "--until-stdin-closes", "--config", FILE]`. With `--format json` the job's `out` carries one JSON object per line: `{"say": TEXT}`, `{"open_url": {"url", "user_code", "expires_ms"}}` for a page to open and the code to confirm there, and at the end `{"account": {"name", "harness", "kind", "id", "label", "checked"}}`. `list --format json` prints `{"name", "harness", "kind", "id", "state", "revision", "expires_at"}` per account.
- `canoa_ffi_job_stop` cancels a sign-in (`--until-stdin-closes`): the provider kills what it started and the job ends with status 1.
- **A job cannot take a typed secret.** Its front answers `ask` with nothing, so a login method that asks for a code or a key (Claude's, an API key) fails the job with status 1 and the account is not made; no credential is read from anywhere else. A method that only shows a page and waits (Codex's `chatgpt`, `chatgpt-device`) works. The app will ask in a sealed form once the host takes one; the C ABI has no input for it today and gains none here.

## Environment variables

| Variable | Effect | Default |
| --- | --- | --- |
| `CANOA_LOG` | what to log, a `tracing` filter | `info` |
| `CANOA_HOST_LOG` | a file the log is appended to as well | none |
| `CANOA_WEB_DIST` | at build time, with feature `web`: the absolute directory of the site built in | none; the build stops without it |

## Gotchas

- `provider`, `tool`, `agent`, `launch`, `mcp-bridge` and `hook-forward` are refused as jobs, status 2: they change the process they run in (its directory, its standard input, its end), so they run only as a process of their own, through `--canoa-child`. The app runs Claude and Codex that way, as `--canoa-child agent claude|codex`.
- On Linux and macOS, while the Dart side has a process of its own running, Dart's process handling collects any child of the application's process that ends, so a child the host started itself would have no status to read (a worktree request's `git` failed as if it had run too long) and a stop could signal a process id already collected. Hosting therefore starts the host's own process starter, `<program> spawner` (`canoa_host::spawn`): every job installs it (a job's `program` is what runs it, else this application with `--canoa-child`; the executable not being found is `STATUS_FAILED`, never a quiet fallback), and it starts at the first child's start (again if it ended). Every agent, provider, tool, hook and `git` the host starts is a child of it, not of the application, and it keeps each uncollected until the host collects it. The starter's command is prepared so that it starts with `SIGCHLD` at the default (an ignored disposition outlives `exec`), whatever program runs it, and `canoa_ffi_child_main` does the same. The starter ends every child it holds when the application ends, a crash included. `canoa_ffi_child_main` puts `SIGCHLD` back to the default, since an application that ignored it would have the starter's children collected as they end. Nothing changes on Windows.
- `held` naming the application's own process is an earlier host of its own that was given up on before it ended: the app waits it out and starts again.

- The application's executable has to take `--canoa-child` before it starts anything else, or `canoa_ffi_child_main` is never reached. `app/native` adds that entry to the runner on Windows and Linux; on macOS it is a load-time entry in this crate, which has not been run.
- The first argument is what decides: a host started with `program` names its own, and `--canoa-child` is not added.
- If the process dies, agents on Windows end with it (each is in a job object); on Unix each sees its standard input close, and most then exit.
- A thread the job started for a link still in its proof can outlive `exit` by up to `canoa_host::serve::PROOF_LIMIT`; it holds neither the home nor an address.
