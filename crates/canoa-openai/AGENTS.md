# `canoa-openai` — API header

Header-file-style reference for `canoa-openai`: OpenAI's speech-to-text for a Canoa host (the `account_speech` description of the accounts that hold an OpenAI API key), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `speech`) and `coqui-acp-api`.

What a host loads is `library` (below). The host names none of it: it carries the library as it carries any other, reads the description through the `account_speech` kind, offers each of the person's accounts whose kind it names and lends the key while a recording runs.

## Exports

```rust
pub const LIBRARY: &str = "canoa-openai";   // the name `library` is registered under
pub const SERVICE: &str = "openai";         // the service's id: an offer made of an account is `openai:NAME`
pub const ADDRESS: &str;                    // https://api.openai.com/v1: where an account's kind calls its model API when nothing moves it
pub const TRANSCRIPTIONS: &str;             // https://api.openai.com/v1/audio/transcriptions: where a recording is posted
pub const CREDENTIAL: &str = "api_key";     // the credential that holds the key
pub const MODELS: [(&str, &str); 4];        // id and label, the recommended first

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // OpenAiSpeech

pub struct OpenAiSpeech;   // an `account_speech`, id "openai_speech": one service, `openai`, for the standard `openai-api`
```

## The service

`OpenAiSpeech::describe` answers one service: the accounts whose kind provides `openai-api` from OpenAI's own address (a key, not a ChatGPT sign-in, and not one whose settings move the address to a gateway) can call it. Models: `gpt-transcribe` (recommended), `gpt-4o-mini-transcribe`, `gpt-4o-transcribe` and `whisper-1`; the last three are announced to end on 2027-02-26, which their labels say. The list is a fact of OpenAI's service, not of Canoa, and changes on its schedule; the host asks no network for it.

## Gotchas

- A ChatGPT sign-in and a key that a setting moves to another address are listed by the host as not offered, with the reason: Canoa has not measured sending audio to OpenAI with either.
- The description holds no network call: `describe` runs with nothing opened.
