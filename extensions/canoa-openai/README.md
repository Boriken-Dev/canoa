# canoa-openai

Canoa's package for OpenAI: the speech-to-text service the accounts that hold an OpenAI API key can call. It says which accounts
those are, where a recording is posted, which credential holds the key and which models transcribe, so a host offers each such
account as dictation and lends the key only while a recording runs.

A build that does not carry it offers no OpenAI transcription. Not published; `canoa-host` carries it. Every export is listed in
[`AGENTS.md`](AGENTS.md).

## Development

```sh
cargo test -p canoa-openai --all-features
```

## License

MPL-2.0; see [LICENSE](../../LICENSE).
