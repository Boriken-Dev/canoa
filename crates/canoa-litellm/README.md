# canoa-litellm

Canoa's package for a LiteLLM proxy: one account kind that provides the OpenAI API and the Anthropic Messages API on one address with
one virtual key, so Codex, Claude Code and any other program that takes either runs on the same account and counts against the same
budget.

A person makes the account by signing in through the proxy's own SSO (their browser, the key handed back to an address on this
computer) or by setting a key they were given. The account lists the proxy's models and reports the key's budget and limits.
Not published; `canoa-host` carries it. Every export is listed in [`AGENTS.md`](AGENTS.md).
