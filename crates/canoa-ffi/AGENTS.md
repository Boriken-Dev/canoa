# `canoa-ffi` — API header

Header-file-style reference for the `canoa-ffi` crate: the C ABI an application uses to run the Canoa host inside its own process, so it can be used without reading the source. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). A `cdylib` (`canoa_ffi.dll`, `libcanoa_ffi.so`) and an `rlib`. It depends on `canoa-host`, `clap`, `serde`, `serde_json`, `tracing`, `tracing-subscriber`, and `rustix` on Unix or `windows-sys` on Windows. It builds with `panic = "unwind"` and refuses to build otherwise.

| Feature | Adds | Default |
| --- | --- | --- |
| `web` | `canoa-host`'s feature: the site `CANOA_WEB_DIST` names at build time, built in | off |

The only consumer is the app's plugin, `app/native`, whose `lib/canoa_native.dart` declares these functions; `app/test/host_library_test.dart` holds its status table and ABI version to the ones here. Unsafe code is in `src/exports.rs` and `src/platform.rs` alone, which a test checks.

## The C ABI (`canoa_ffi::exports`)

```c
uint32_t canoa_ffi_abi(void);                                                      // ABI_VERSION, 1; refuse any other
int32_t  canoa_ffi_job_start(const uint8_t *request, size_t len, uint64_t *out_job);
int32_t  canoa_ffi_job_read(uint64_t job, uint8_t **out_text, size_t *out_len);
void     canoa_ffi_text_free(uint8_t *text, size_t len);
int32_t  canoa_ffi_job_stop(uint64_t job);
int32_t  canoa_ffi_job_free(uint64_t job);
int32_t  canoa_ffi_child_main(void);
```

| Status | Name | Meaning |
| --- | --- | --- |
| 0 | `STATUS_OK` | done |
| 1 | `STATUS_BAD_ARGUMENT` | a pointer null, misaligned or an alias of another, an out-parameter inside the request, a length too long, a request this library does not read |
| 2 | `STATUS_UNKNOWN_JOB` | no job has that number, or it was freed |
| 3 | `STATUS_FAILED` | a thread for the job could not be started |
| 4 | `STATUS_PANIC` | the library panicked in this call |

- **No function blocks**, none calls back, and no panic crosses: each body runs under `catch_unwind`, and each out-parameter that is non-null and aligned is written first with its empty value (0, null, 0), whatever else the call refuses; an `out_job` inside the request is refused unwritten. Call them from any thread.
- **A job is one `canoa` command**, on a thread of its own, numbered from 1; a number is never used twice, and a stale or invented one is `STATUS_UNKNOWN_JOB`.
- `canoa_ffi_job_start`: `request` is UTF-8 JSON, read only during the call, unknown keys refused:

| Key | Value | Default |
| --- | --- | --- |
| `args` | the command line without the program's name, as `canoa` takes it (see `crates/canoa-host/AGENTS.md`) | required |
| `home` | the directory of the identity, trust store and lock | this user's home |
| `holder` | what `serve` calls itself to another host that finds its home held | `an application` |
| `program` | what runs the host's own `provider`, `tool` and `launch` | this application, with `--canoa-child` first |
| `program_args` | what goes before the subcommand on `program`'s command line; only with `program` | none |

- `canoa_ffi_job_read`: what the job has said since the last read, `{"out": [line...], "err": [line...], "dropped": n, "exit": status or null}`, which `canoa_ffi_text_free` frees with the same length. `out` is what `canoa` prints on standard output, `err` what it says on standard error. `dropped` counts lines lost past 4096 unread of a kind, the oldest first. `exit` is set once the command has ended and has given up all it held, a home's lock and its addresses included; a command line `clap` refuses is status 2, a panicked command 101.
- `canoa_ffi_job_stop`: closes the job's input. `serve --until-stdin-closes` then ends every listener, link and agent, and `pair --until-stdin-closes` gives up. It returns at once; `exit` says when the job has ended. A command with no such flag does not hear it.
- `canoa_ffi_job_free`: forgets the job, closing its input first. A job still running goes on to its end with nobody reading it.
- `canoa_ffi_child_main`: this process as the host's own `provider`, `tool` or `launch`, from the process's command line, whose first argument is `--canoa-child` (`jobs::CHILD_FLAG`). Anything else is status 2. It returns the status to exit with, and is called before the application starts anything of its own.

## What a long command changes in this process

The first command in a process installs a `tracing` subscriber writing to the log (`CANOA_LOG` as in `canoa`, default `info`), which is delivered to every running `serve` and `pair` as `err`. The first `serve` or `pair`, and only those, also:

- puts this process's standard error on a pipe a thread reads for good, delivered the same way, so that an agent inheriting it is heard and never blocks. What the application itself writes to standard error goes there too, and no longer reaches where it went before;
- on Windows, gives a process with no console one made with no window, by attaching to a helper's, so that a console program it starts opens no window of its own.

- on Unix, sets `SIGPIPE` aside when the application left it as the system gives it, as a Rust program's `main` does, so that writing to an agent that has died is an error and not the application's end; a handler of the application's own stays.

The console comes before the pipe: attaching to one replaces the standard handles of a process started with none, as Explorer starts an application. These stay for the life of the process; stopping a host does not undo them, and the library is never to be unloaded, since its threads run on in it.

## Environment variables

| Variable | Effect | Default |
| --- | --- | --- |
| `CANOA_LOG` | what to log, a `tracing` filter | `info` |
| `CANOA_HOST_LOG` | a file the log is appended to as well | none |
| `CANOA_WEB_DIST` | at build time, with feature `web`: the absolute directory of the site built in | none; the build stops without it |

## Gotchas

- `provider`, `tool` and `launch` are refused as jobs, status 2: they change the process they run in (its directory, its standard input, its end), so they run only as a process of their own, through `--canoa-child`.
- On Linux and macOS, while the Dart side has a process of its own running, Dart's process handling collects any child that ends, the host's among them. The host then cannot learn a child's status: a worktree request's `git` fails as if it had run too long, and an agent's stop signals a process id already collected. Not yet solved; it needs the host's children started outside the application's own process.
- `held` naming the application's own process is an earlier host of its own that was given up on before it ended: the app waits it out and starts again.

- The application's executable has to take `--canoa-child` before it starts anything else, or `canoa_ffi_child_main` is never reached. `app/native` adds that entry to the runner on Windows and Linux; on macOS it is a load-time entry in this crate, which has not been run.
- The first argument is what decides: a host started with `program` names its own, and `--canoa-child` is not added.
- If the process dies, agents on Windows end with it (each is in a job object); on Unix each sees its standard input close, and most then exit.
- A thread the job started for a link still in its proof can outlive `exit` by up to `canoa_host::serve::PROOF_LIMIT`; it holds neither the home nor an address.
