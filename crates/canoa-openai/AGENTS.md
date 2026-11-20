# `canoa-openai` — API header

Header-file-style reference for `canoa-openai`: OpenAI's speech for a Canoa host (the `account_speech` description of the accounts that hold an OpenAI API key, and the `speech_output` provider of OpenAI's voices), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (features `speech`, `speech-builtin`), `coqui-acp-api`, `guatiao` and `guatiao-intake`.

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
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // OpenAiSpeech, OpenAiVoices

pub struct OpenAiSpeech;   // an `account_speech`, id "openai_speech": one service, `openai`, for the standard `openai-api`

pub const SPEECH_PROVIDER: &str = "speech_say_openai";   // the voice provider's id, which a `[say.NAME]` table names
pub const SPEECH: &str;                 // https://api.openai.com/v1/audio/speech: where the text is posted
pub const SPEECH_MODELS: [(&str, &str); 3];   // gpt-4o-mini-tts, tts-1, tts-1-hd
pub const VOICES: [&str; 13];           // alloy ... cedar
pub const STYLED_MODEL: &str;           // gpt-4o-mini-tts: takes a style, and the only model the voices ballad, verse, marin and cedar work with
pub const STYLES: [&str; 4];            // calm, cheerful, serious, whisper, each `name=instruction`
pub struct OpenAiVoices;                // a `speech_output`, id "speech_say_openai", built `From<OpenAiVoiceSettings>`
pub struct OpenAiVoiceSettings { pub api_key: Option<Secret<String>>, pub model: Option<String>, pub voice: Option<String>, pub timeout_seconds: u32 }
```

## The service

`OpenAiSpeech::describe` answers one service: the accounts whose kind provides `openai-api` from OpenAI's own address (a key, not a ChatGPT sign-in, and not one whose settings move the address to a gateway) can call it. Models: `gpt-transcribe` (recommended), `gpt-4o-mini-transcribe`, `gpt-4o-transcribe` and `whisper-1`; the last three are announced to end on 2027-02-26, which their labels say. The list is a fact of OpenAI's service, not of Canoa, and changes on its schedule; the host asks no network for it.

## The voices

`OpenAiVoices` is `canoa`'s generic endpoint provider (`EndpointSay`) over OpenAI's address with OpenAI's lists: the three models, thirteen voices and four styles. A voice that only `gpt-4o-mini-tts` has is refused, naming that model, when a request asks for another one; nothing is sent. `leaves` is `yes`, and `words` names api.openai.com and the key. The key is the provider's own setting: lending an account's key to speech output, as `account_speech` does for dictation, is not done.

## Gotchas

- A ChatGPT sign-in and a key that a setting moves to another address are listed by the host as not offered, with the reason: Canoa has not measured sending audio to OpenAI with either.
- The description holds no network call: `describe` runs with nothing opened.
