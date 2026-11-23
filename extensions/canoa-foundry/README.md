# canoa-foundry

Canoa's package for Foundry Local: the three wire APIs agents speak (Anthropic's Messages, OpenAI's Responses and chat
completions) translated to one chat completion a local model answers, the engines that run it (the Foundry Local service the
person runs, or its Rust SDK), and the account kind that points Claude Code, Codex and Pi at the gateway the host runs.

## Features

- `default = ["daemon"]` — uses the person's running Foundry Local service over loopback HTTP (`ureq`, `canoa/network`), keeping builds free of the heavier native SDK stack.
- `sdk` — runs models in-process through `foundry-local-sdk` and `tokio`.

## Usage & API

Not published (`publish = false`); `canoa-host` depends on it. Every export is listed in [`AGENTS.md`](AGENTS.md).

## License

MPL-2.0; see [LICENSE](../../LICENSE).
