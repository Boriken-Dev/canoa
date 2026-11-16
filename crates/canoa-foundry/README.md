# canoa-foundry

Canoa's package for Foundry Local: the three wire APIs agents speak (Anthropic's Messages, OpenAI's Responses and chat
completions) translated to one chat completion a local model answers, the engines that run it (the Foundry Local service the
person runs, or its Rust SDK), and the account kind that points Claude Code, Codex and Pi at the gateway the host runs.

The translations are pure functions of JSON, tested against requests recorded from the real programs. Not published; `canoa-host`
depends on it. Every export is listed in [`AGENTS.md`](AGENTS.md).
