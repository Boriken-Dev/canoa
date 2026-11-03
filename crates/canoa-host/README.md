# canoa-host

[![Version](https://img.shields.io/crates/v/canoa-host.svg)](https://crates.io/crates/canoa-host)
[![Rust version](https://img.shields.io/crates/msrv/canoa-host.svg)](https://crates.io/crates/canoa-host)
[![License: MPL-2.0](https://img.shields.io/badge/license-MPL--2.0-blue.svg)](https://github.com/Boriken-Dev/canoa/blob/main/LICENSE)
[![Docs](https://img.shields.io/docsrs/canoa-host)](https://docs.rs/canoa-host)
[![CI](https://img.shields.io/github/actions/workflow/status/Boriken-Dev/canoa/test.yml)](https://github.com/Boriken-Dev/canoa/actions/workflows/test.yml)

**Serves the agents Canoa.cfg names to keys you paired, on this machine's loopback, and to nobody else.** An entry is a
program that speaks the Agent Client Protocol on its standard streams, or a Guatiao `agent_provider` from a library or
from a source such as Python's, served through `coqui-acp-provider`'s facade. Each entry listens on a WebSocket of its own; a client proves a key the trust
store accepts, and every message after that is sealed in the session the proof ended in. See the
[documentation](https://boriken-dev.github.io/canoa/host/).

## Features

- Stdio ACP agents and Guatiao agent providers, one `Canoa.cfg` entry each, a fresh process per connection.
- A provider written in Python, Lua, JavaScript, Rhai or WebAssembly through its Guatiao source, and a provider built from
  a `settings` table checked against its schema. Settings reach the provider's process on its standard input, never its
  command line.
- The in-band WebCrypto proof only: no connection reaches an agent without a paired key, and a removed key loses its link
  within about a second.
- Loopback only. A proxy that ends TLS can carry it further, since the proof does not depend on TLS.
- A command-line tool with no ACP, such as Pi, from one `[host.tools]` entry: rules in Canoa.cfg turn its JSON lines into
  ACP updates and its questions into permission requests, and each session opens by saying what the tool does unasked.
- A client cannot have the host start a program: a stdio MCP server in `session/new`, and `_meta`, never reach a stdio agent.
- Per entry: working roots a session's `cwd` must be under, the paired keys that may use it, and how many at once.

## Installation

Not published: its git dependencies are not released. Build it from the repository:

```sh
cargo build --release -p canoa-host
```

There are no Cargo features.

## Quick start

```toml
# Canoa.cfg
[host.agents.claude]
program = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]
listen = "127.0.0.1:7801"
working_roots = ["/home/me/src"]

[host.providers.echo]
library = "echo_provider.so"
provider = "echo_provider"
listen = "127.0.0.1:7802"

[host.providers.python]
source = "libguatiao_source_py.so"
provider = "acp_echo_echo"
[host.providers.python.source_settings]
dirs = ["python/echo"]
paths = ["python/guatiao", "python/coqui_acp_provider"]
```

```sh
canoa-host serve            # one JSON line per entry: name, kind, url, and this host's peer id
canoa-host peers            # a client that connected once is waiting
canoa-host accept <PEER>    # pair it, after comparing the id with the one the client shows
```

The library reads the same file:

```rust
let config = canoa_host::Config::from_str_in(
    "[host.agents.echo]\nprogram = \"node\"\nargs = [\"agent.mjs\"]\n",
    std::path::Path::new("."),
)?;
assert_eq!(config.entries[0].name, "echo");
# Ok::<(), canoa_host::ConfigError>(())
```

## Command line

```text
canoa-host serve [--config FILE] [--until-stdin-closes]
canoa-host check [--config FILE] [--format text|json]
canoa-host id
canoa-host peers [--format text|json]
canoa-host accept PEER
canoa-host revoke PEER
canoa-host pair [--listen ADDR] [--timeout-seconds N] [--until-stdin-closes]
canoa-host provider (--library FILE | --source FILE) --provider ID [--working-root DIR]... [--max-sessions N]
                    [--directory DIR] [--settings-on-stdin]
```

`serve --until-stdin-closes` is for a program running the host as its child, such as Canoa's desktop app: when its input
closes, the host ends every agent it started, then itself. `pair` shows a PIN that a native client, such as
`coqui-acp-bridge pair-dial`, uses to pair; a browser has no PIN exchange, and pairs by connecting once and being accepted.

Exit status: 0 success, 1 the operation failed, 2 the invocation or Canoa.cfg was wrong.

## API overview

| Module | Purpose |
| --- | --- |
| `config` | the `[host]` table of Canoa.cfg |
| `serve` | binding every entry and serving connections |
| `provider` | one Guatiao provider as an ACP agent on standard input and output |
| `peers` | listing, accepting and removing keys |

## What it trusts

- **A paired key** reaches its entries' agents, which act with this user's rights as far as each agent lets them.
- **A provider library** is native code: loading it runs it, with this user's rights, in the process serving that
  connection. It is trusted as fully as a program you run, and so is whoever can write it or its directory.
- **A provider source** runs code and loads more: for Python, the interpreter the system finds, its `site-packages`, the
  host's `PYTHONPATH`, and every directory and manifest its settings name. On Windows, when the narrow search leaves an
  import missing, such as `python3.dll`, the source's imports are looked for in the working directory and on `PATH` as
  well. What it prints goes to the host's log, never to the client.
- **A stdio agent** is the program the file names, found as the operating system finds a command.

## Development

```sh
cargo fmt --all --check
cargo clippy --workspace --all-targets --all-features -- -D warnings
cargo test --workspace --all-features
CANOA_TEST_PYTHON=python3 cargo test -p canoa-host --test python_source
```

The last builds Guatiao's Python source against the Python it names, which needs its shared library, and serves
coqui-acp's Python echo provider through one entry. Without `CANOA_TEST_PYTHON` that test is ignored.

### Releasing

Not released; see the repository's [CHANGELOG](https://github.com/Boriken-Dev/canoa/blob/main/CHANGELOG.md).

## License

MPL-2.0; see [LICENSE](https://github.com/Boriken-Dev/canoa/blob/main/LICENSE).
