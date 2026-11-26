# `canoa-openai` — API header

Header-file-style reference for `canoa-openai`: OpenAI's speech for a Canoa host (the `account_speech` description of the accounts that hold an OpenAI API key, and the `speech_output` provider of OpenAI's voices), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (features `speech`, `speech-builtin`, `sdk`), `coqui-acp-api`, `guatiao`, `guatiao-intake` and `serde_json`.

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

```rust
pub const SPEECH_PROVIDER: &str = "speech_say_openai";   // the voice provider's id, which a `[say.NAME]` table names
pub const SPEECH: &str;                 // https://api.openai.com/v1/audio/speech: where the text is posted
pub const STYLES: [&str; 4];            // calm, cheerful, serious, whisper, each `name=instruction`: Canoa's own wording
pub struct OpenAiVoices;                // a `speech_output`, id "speech_say_openai", built `From<OpenAiVoiceSettings>`
pub struct OpenAiVoiceSettings { pub api_key: Option<Secret<String>>, pub model: Option<String>, pub voice: Option<String>, pub models: Vec<String>, pub timeout_seconds: u32 }

pub mod description {                   // OpenAI's API description, read for speech
    pub const DESCRIPTION: &str;        // https://raw.githubusercontent.com/openai/openai-openapi/main/openapi.json
    pub const REPOSITORY: &str;         // https://raw.githubusercontent.com/openai/openai-openapi/: the place the fetch is declared to reach (any branch or tag)
    pub struct Speech { pub voices: Vec<String>, pub shutdowns: BTreeMap<String, String> }   // shutdowns: model id to YYYY-MM-DD
    pub fn read(document: &str) -> Result<Speech, String>;
}
```

## The service

`OpenAiSpeech::describe` answers one service: the accounts whose kind provides `openai-api` from OpenAI's own address (a key, not a ChatGPT sign-in, and not one whose settings move the address to a gateway) can call it. Models: `gpt-transcribe` (recommended), `gpt-4o-mini-transcribe`, `gpt-4o-transcribe` and `whisper-1`; the last three are announced to end on 2027-02-26, which their labels say. The list is a fact of OpenAI's service, not of Canoa, and changes on its schedule; the host asks no network for it.

## The voices

`OpenAiVoices` is `canoa`'s generic endpoint provider (`EndpointSay`) over OpenAI's address. It names no model and no voice, and refuses none:

- **Models** are the ones the account's key lists. The host asks the account's `sdk_provider` (`openai_sdk`, below) for `GET /v1/models` and writes the ids it says can turn text into speech into the `models` setting of the account's offer (`openai:NAME`), kept five minutes. `GET /v1/models` marks nothing (an id, a date, an owner), so the one rule that says which are speech models is `rule`'s id pattern (`tts-*` and `*-tts*`). A request that names no model uses `model`, else the first the key lists. A model not in the list is still sent.
- **Voices** are read when the host lists them: `voices` asks the host's fetch for `description::DESCRIPTION` (declared in `OutputInfo::endpoints`) and takes every string `CreateSpeechRequest`'s `voice` enumerates, following its `$ref`s (`VoiceIdsShared` first, then the few the call takes besides). The host keeps the answer for `[host.store] source_ttl`. The same read gives each model's announced shutdown date (`x-openai-enum-deprecations` of `CreateSpeechRequest`'s `model`), shown beside the model: `tts-1 (ends 2027-01-06)`.
- **A voice typed by name is accepted**, and so is a pair OpenAI may not take. The call takes any string (custom voice ids too), and which voices work with which model is prose in OpenAI's documentation, not in its API description, so no restriction is encoded: `say` sends what was asked and OpenAI's own error (the first 300 bytes of its body) is the answer a person sees. Settings, Voice has a Voice by name field for it.
- **Offline, or the description out of reach**: `voices` fails with the reason, the host lists no voices and says why (`voices_problem`), and a typed voice still speaks.

`leaves` is `yes`, and `words` names api.openai.com and the key. `OpenAiSpeechOutput` describes the same service through `account_speech_output` so any matching `openai-api` key account at OpenAI's own address offers OpenAI's voices (`openai:NAME`) with its key lent from the account store when speaking and never written to `[say]`.

## Gotchas

- A ChatGPT sign-in and a key that a setting moves to another address are listed by the host as not offered, with the reason: Canoa has not measured sending audio to OpenAI with either.
- The description holds no network call: `describe` runs with nothing opened.

- What stays written in this crate, as OpenAI gives it in no form a program reads: `MODELS`, the four transcription models and their end dates (`describe` of `account_speech` holds no network call, so nothing lists them live yet); `STYLES`, Canoa's own wording of what `instructions` asks of a model; and `rule`'s table of what a model does by its id (below). A change in OpenAI's transcription catalog is a release of this crate.

## The `sdk_provider`

`OpenAiSdk` (id `openai_sdk`, offer id `openai`, an offer made of an account is `openai:NAME`) serves the accounts whose kind provides `openai-api`, at the address their kind's model API setting says (OpenAI's own, or a gateway that speaks the shape: the offer is not pinned). It describes `chat` and `embed`; the host lends the HTTP and writes the key, and the crate sees neither. It is `canoa::sdk::OpenAiShaped` with a `Dialect` for OpenAI's own address (`max_completion_tokens`; no temperature for `gpt-5*` and the `o` series) and `rule`, the table that says what each model does, since `GET /v1/models` lists an id and an owner and nothing else, and OpenAI's API description names no capability of a model:

```rust
pub struct OpenAiSdk;                       // an `sdk_provider`
pub fn rule(model: ModelInfo) -> ModelInfo; // capabilities (declared) and limits by id; an id it does not know reports none
```

`text-embedding-*`: `embed`, with `dimensions` (1536, 3072 for `-3-large`), adjustable for `-3-*`, 8191 input tokens. `*transcribe*` and `whisper-*`: `speech_to_text`, `audio_input`. `tts-*` and `*-tts*`: `text_to_speech`, `audio_output` and, for every one but the `tts-` models (whose `instructions` OpenAI's documentation says do not work), the four styles. No voice is declared per model. `*realtime*` and `*-audio-*`: `audio_input`, `audio_output`. `gpt-*`, `chatgpt-*` and the `o` series: `chat`, `tool_use`, `structured_output`, with `reasoning` where the model takes no temperature and `image_input` for `gpt-4o`, `gpt-4.1` and `gpt-5`. A change in OpenAI's catalog is a release of this crate. The tests run the contract over recorded answers (`tests/fixtures/*.json`, readable, no key; `openapi.json` is OpenAI's API description trimmed to the speech call, and `models.json` a `GET /v1/models` answer); one `#[ignore]` test reads the real description.
