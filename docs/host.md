# The host

`canoa-host` serves agents to Canoa's chat on this machine. Each entry of Canoa.cfg's `[host]` table is one agent, on a
loopback WebSocket of its own:

- **A stdio ACP agent**: any program that speaks the Agent Client Protocol on its standard streams, such as Claude's
  adapter, Codex's adapter or `gemini --acp`.
- **A Guatiao provider**: a library offering coqui-acp-provider's `agent_provider` kind, written once in any language
  Guatiao builds a library from, and served through its ACP facade.

The API and every setting are in the crate's [API header](https://github.com/Boriken-Dev/canoa/blob/main/crates/canoa-host/AGENTS.md).

## Configuration

```toml
[host.agents.claude]
program = "npx"
args = ["-y", "@agentclientprotocol/claude-agent-acp@0.99.1"]
listen = "127.0.0.1:7801"
working_roots = ["/home/me/src"]

[host.providers.echo]
library = "echo_provider.dll"
provider = "echo_provider"
listen = "127.0.0.1:7802"
```

`canoa-host serve` reads the same Canoa.cfg as the app (`--config`, then `CANOA_CONFIG`, then the platform's place) and
prints one JSON line per entry: its name, kind, address and the host's peer id. The app keeps `[host]` as it is when it
saves its own settings.

## Pairing

The host proves itself and its clients with the same WebCrypto method as every Canoa connection, using this user's
`coqui-agentic-peer` identity and trust store, the ones `coqui-acp-bridge serve --paired` uses.

1. In Canoa's connect dialog, give the entry's address (`ws://127.0.0.1:7801`) and the host's peer id (`canoa-host id`).
2. The first connection is turned away, and the browser's key waits on the host.
3. `canoa-host peers` lists it. Compare its id with the one Canoa shows, then `canoa-host accept <id>`.
4. Connect again. `canoa-host revoke <id>` ends that key's links within about a second.

A key's first use is always the person's decision: nothing is accepted automatically.

## What the host allows

- **Nothing beyond loopback.** An address that is not loopback is refused. The proof does not depend on TLS, so a proxy
  that ends TLS can carry a connection further and can neither read nor change it.
- **Nothing without a paired, proven key.** Every message after the proof is sealed in the session it ended in. An
  entry's `peers` narrows it to some paired keys.
- **No program a client names.** ACP's `session/new` can list a stdio MCP server, which an agent starts as a program
  on this machine without asking anyone. The host applies [the page scope](https://github.com/Boriken-Dev/coqui-acp/blob/main/docs/browser.md#scope)'s
  rule to a stdio agent's clients: such an entry is refused whole, an `http` or `sse` server must be `https`, and `_meta`,
  which an agent may read as launch options, is removed. A provider's facade takes no MCP server at all.
- **Only the working roots it names**, when an entry names any.

## What it trusts

- **A paired key** gets the agents its entries offer, which act with this user's rights as far as each agent lets them.
  An agent's own permission requests still go to the chat.
- **A provider library is native code.** Loading it runs it, with this user's rights, before anything can check what it
  does. It is mapped only in the process serving one connection, so a crash ends that connection alone, but it is
  trusted as fully as a program you run, and so is whoever can write it or its directory.
- **A stdio agent** is the program the file names, with the host's environment and working directory.

## Claude and ChatGPT

Each is reached through the vendor's own command-line program, signed in by the person through the vendor's own flow.
The host never sees, stores or forwards a vendor credential, and never reuses a consumer app's session.

| | Claude | ChatGPT |
| --- | --- | --- |
| Entry | `npx -y @agentclientprotocol/claude-agent-acp@0.99.1`, which drives Claude Code (`CLAUDE_CODE_EXECUTABLE` names it) | `npx -y @agentclientprotocol/codex-acp@2.5.0`, which starts Codex's app server |
| Sign-in | `claude` signed in on this machine: a subscription, or an API key | `codex` signed in with a ChatGPT plan, or an API key |
| Without the adapter | `claude -p --output-format stream-json`, `--resume` | `codex exec --json`, `codex app-server` |

**What the terms say.** Anthropic's [legal and compliance page](https://code.claude.com/docs/en/legal-and-compliance)
does not prevent "an end user from signing in to the unmodified Claude Code binary with their own Claude subscription",
says subscription limits "assume ordinary, individual usage of Claude Code and the Agent SDK", and does not permit
third parties to "route requests through Free, Pro, Max, or Ultra plan credentials on behalf of their users". So an entry
running Claude on a subscription is for its owner's own keys: pairing someone else's key would make the account available
to them. OpenAI documents ChatGPT sign-in for Codex and its programmatic modes; its Terms of Use could not be read for this
page, so check its sharing rules before pairing anyone else.

**What they cannot do here.**

- Nothing of the consumer apps: no claude.ai or ChatGPT conversation history, projects, connectors, custom GPTs or image
  generation. A session exists only in the agent that made it.
- The program has to be installed and signed in on the host's machine. A headless machine signs in with
  `claude setup-token` or Codex's device-code login.
- The plan's usage limits apply, shared with the person's other use.
- An `ANTHROPIC_API_KEY` in the host's environment takes precedence over Claude's subscription sign-in, so it changes who
  pays. The agent inherits the host's environment.
- A turn in progress ends with its connection: each connection runs a fresh agent process.
