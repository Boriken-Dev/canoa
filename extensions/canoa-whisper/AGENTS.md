# `canoa-whisper` — API header

Header-file-style reference for `canoa-whisper`: OpenAI's Whisper as one Canoa Store type, whose models are read from Hugging Face and whose one download runs on two engines, Candle and whisper.cpp, so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (features `store`, `speech`), `canoa-candle` (its Hugging Face reader and its settings), `coqui-acp-api`, `guatiao`, `guatiao-intake`, `serde_json`, `sha2` and, with the default feature `whispercpp`, `canoa-whispercpp`. Without that feature there is no C or C++ to build, and the models run on Candle alone.

What a host loads is `library` (below). The host names none of it: it carries the library as it carries any other, and lists the type, its models and its speech offer through the generic Store calls.

## Exports

```rust
pub const LIBRARY: &str = "canoa-whisper";   // the name `library` is registered under
pub const NAME: &str = "Whisper";            // what the type is called to the person
pub const STORE_TYPE: &str = "whisper";      // the id of the Store type, and of its speech offer
pub const CANDLE: &str = "candle";           // an engine's id: on a model's pill and as the key of the install dialog's choice
pub const WHISPERCPP: &str = "whispercpp";   // (feature `whispercpp`) the same for whisper.cpp
pub const WRITE_GGML: &str = "write_ggml";   // the action the install asks for after the download

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // WhisperExtension, WhisperModels, WhisperHf

pub struct WhisperExtension;   // a `store_extension`, id "whisper_extension": the type `whisper` ("Whisper", icon `chip`, no program store, models `whisper_models`, the default catalog `openai`)
pub struct WhisperModels;      // a `model_store`, id "whisper_models": the speech offer and the action that writes whisper.cpp's file
pub struct WhisperHf;          // a `catalog_source`, id "whisper_hf", serving the type: the Whisper repositories of a Hugging Face publisher
pub struct WhisperHfSettings { pub author: String, pub search: String, pub include: Vec<String>, pub exclude: Vec<String> }   // author `openai`, search `whisper`, patterns with `*` on the name after the owner; none left out
pub const SOURCE: &str = "whisper_hf";
pub fn default_catalog() -> DefaultCatalog;   // label `openai`: `[store.whisper.catalogs.openai]`, every field at its default
pub fn catalog_of(revision: &str, models: &[canoa_candle::catalog::Model]) -> Result<ModelsCatalog, String>;   // what the source makes of the repositories it read
```

## The page

The type has no program and ships no model list. `whisper_hf` lists the publisher's repositories (`GET /api/models?author=openai&search=whisper`), describes each at the branch it serves (`main`, or an entry's `pin`: a branch or tag; `/api/models/<repo>/revision/<rev>?blobs=true`) and answers `canoa.models/1`. A model is one item, a folder `<id>/` of the repository's `config.json`, `tokenizer.json` and `model.safetensors`, each file checked by what the listing states for it (a large file's SHA-256, a small one's git blob id, at the commit the listing was read at), so the badge names those hashes. The id is the repository's name without `whisper-` (`tiny`, `large-v3-turbo`, `tiny.en`).

Each item names its engines in its own data, which the application draws as pills (filled once installed, outlined while available):

- **Candle** (`candle`): reads the three downloaded files. Left off a model whose id ends in `.en`: Candle detects the language from tokens an English-only model lacks.
- **Whisper.cpp** (`whispercpp`, with the feature): reads `<id>/ggml.bin`, which no download supplies. Its action is `write_ggml`, and its variants (`f16`, `q8_0`, `q5_1`, `q5_0`, the full-precision one first and the default) are what the install dialog asks the person to pick.

The install is the host's `fetch_files` step (the three files, checked) and then `write_ggml`, which `WhisperModels::act` runs with the models folder and the pick (`ModelAct::folder`, `ModelAct::options` = `{"whispercpp": "<variant>"}`): `canoa_whispercpp::convert` writes `<id>/ggml.bin` from the downloaded folder, and `<id>/ggml.json` records `precision`, `bytes` and the file's `sha256`. A model whose rows do not divide into blocks of 32 is refused a quantized form with the tensor's name; the folder stays installed and `f16` works. Removing the row removes `ggml.bin` and `ggml.json` with the three files.

## The speech offer

`describe().speech` is one offer, `SpeechService::in_process(.., "speech_whisper_candle")`. `service` lists, with `query.models`, every way an installed model runs: `candle:<id>` when the three files are there and the engine can run it, `whispercpp:<id>` when `ggml.bin` is, labelled with the form from `ggml.json` ("Tiny, every language (Whisper.cpp, q5_0)"). It answers `settings` and `provider` for `query.model` (`<engine>:<id>`, or a bare id for the first engine that runs it) when that is installed, else for the first that is: the settings of `canoa-candle`'s `speech_whisper_candle` or `canoa-whispercpp`'s `speech_whisper`, both of which read the folder `<models_dir>/<id>/`. With no folder or no model it answers `unavailable` in its own words.

## Gotchas

- The install of a large model runs the converter inside the host's process (tens of seconds, under 1 GB for the largest model, which is mapped, not read whole).
- A folder the person drops in the models folder with the three files (or a `ggml.bin`) is offered by the speech offer, whatever catalog it came from.
- Hugging Face's own listing is the only source of what exists: with no network and nothing read before, the page is empty, with the reason.
