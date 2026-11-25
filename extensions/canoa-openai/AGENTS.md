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
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // OpenAiSpeech, OpenAiVoices, OpenAiSpeechOutput

pub struct OpenAiSpeech;   // an `account_speech`, id "openai_speech": one service, `openai`, for the standard `openai-api`
pub struct OpenAiSpeechOutput;   // an `account_speech_output`, id "openai_speech_output": offers OpenAI's voices to `openai-api` accounts at OpenAI's own address
```

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

`OpenAiVoices` is `canoa`'s generic endpoint provider (`EndpointSay`) over OpenAI's address with OpenAI's lists: the three models, thirteen voices and four styles. A voice that only `gpt-4o-mini-tts` has is refused, naming that model, when a request asks for another one; nothing is sent. `leaves` is `yes`, and `words` names api.openai.com and the key. `OpenAiSpeechOutput` describes the same service through `account_speech_output` so any matching `openai-api` key account at OpenAI's address offers OpenAI's voices (`openai:NAME`) with its key lent from the account store when speaking and never written to `[say]`.

## Gotchas

- A ChatGPT sign-in and a key that a setting moves to another address are listed by the host as not offered, with the reason: Canoa has not measured sending audio to OpenAI with either.
- The description holds no network call: `describe` runs with nothing opened.

- The lists here (`MODELS`, `SPEECH_MODELS`, `VOICES`, `STYLES`, with their end dates) are the one thing in the extensions that stays shipped data with no source: OpenAI's documentation is their only upstream (`GET /v1/models` needs a key and lists neither voices nor end dates), so a change in them is a release of this crate.

## The `sdk_provider`

`OpenAiSdk` (id `openai_sdk`, offer id `openai`, an offer made of an account is `openai:NAME`) serves the accounts whose kind provides `openai-api`, at the address their kind's model API setting says (OpenAI's own, or a gateway that speaks the shape: the offer is not pinned). It describes `chat` and `embed`; the host lends the HTTP and writes the key, and the crate sees neither. It is `canoa::sdk::OpenAiShaped` with a `Dialect` for OpenAI's own address (`max_completion_tokens`; no temperature for `gpt-5*` and the `o` series) and `rule`, the table that says what each model does, since `GET /v1/models` lists an id and an owner and nothing else:

```rust
pub struct OpenAiSdk;                       // an `sdk_provider`
pub fn rule(model: ModelInfo) -> ModelInfo; // capabilities (declared) and limits by id; an id it does not know reports none
```

`text-embedding-*`: `embed`, with `dimensions` (1536, 3072 for `-3-large`), adjustable for `-3-*`, 8191 input tokens. `*transcribe*` and `whisper-*`: `speech_to_text`, `audio_input`. `tts-*` and `*-tts`: `text_to_speech`, `audio_output`, the voices the model takes and, for `gpt-4o-mini-tts` alone, the four styles. `*realtime*` and `*-audio-*`: `audio_input`, `audio_output`. `gpt-*`, `chatgpt-*` and the `o` series: `chat`, `tool_use`, `structured_output`, with `reasoning` where the model takes no temperature and `image_input` for `gpt-4o`, `gpt-4.1` and `gpt-5`. A change in OpenAI's catalog is a release of this crate. The tests run the contract over recorded answers (`tests/fixtures/*.json`, readable, no key).
