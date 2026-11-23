# `canoa-whisper` — API header

Header-file-style reference for `canoa-whisper`: whisper.cpp for a Canoa host (the Store program that installs its speech-to-text command line, the Store type that lists it, the models page of that type, and the speech offer a host makes of the installed copy), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (feature `store`), `coqui-acp-api`, `serde_json`, `time`.

What a host loads is `library` (below). The host names none of it: it carries the library as it carries any other, lists the type and its models through the generic Store calls and makes the speech offer from what `WhisperModels::service` answers.

## Exports

```rust
pub const LIBRARY: &str = "canoa-whisper";    // the name `library` is registered under
pub const NAME: &str = "whisper.cpp";    // what the program is called to the person: its rows, facts and speech offer
pub const STORE_TYPE: &str = "whisper";       // the id of the Store type, and of its speech offer

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // WhisperStore, WhisperExtension, WhisperModels

pub struct WhisperStore;       // a `program_store`, id "whisper_store", over `store::WhisperRecipe`
pub struct WhisperExtension;   // a `store_extension`, id "whisper_extension": the type `whisper` ("whisper.cpp", icon `chip`, store `whisper_store`, models `whisper_models`)
pub struct WhisperModels;      // a `model_store`, id "whisper_models"; `WhisperModels::over(&'static [Model])` lists another set of models, which a test serves

pub fn arguments(model: &str) -> Vec<String>;                          // whisper-cli -m MODEL -f - -of canoa -nt -np -l {language_code|auto} --prompt {vocabulary}
pub fn run_of(program: &str, folder: Option<&Path>, wanted: Option<&str>) -> Result<SpeechRun, String>;   // how to run the copy whose program is `program`: with `wanted` when that model is installed, else the first installed (the copy's own, else one in the host's models `folder`)

pub mod store {
    pub const RELEASES_API: &str;   // https://api.github.com/repos/ggml-org/whisper.cpp/releases
    pub const DOWNLOADS: &str;      // where a release's files are served from
    pub const MODELS_ORIGIN: &str;  // the publisher's Hugging Face folder at the revision the digests were read from
    pub const PROGRAM_DIR: &str;    // "bin", the program's folder in a version's folder
    pub struct Model { pub id: &'static str, pub label: &'static str, pub file: &'static str, pub bytes: u64, pub sha256: &'static str }
    pub const MODELS: [Model; 8];   // the default (`base`) first
    pub fn model(id: &str) -> Option<&'static Model>;
    pub fn model_path(model: &Model) -> String;                         // "models/ggml-<model>.bin", relative to a version's folder
    pub fn held_model(version: &Path) -> Option<&'static Model>;        // the first of MODELS whose file is in the version's own `models` folder (a copy installed with a model)
    pub fn program_file(os: &str) -> &'static str;                      // whisper-cli.exe on "windows", else whisper-cli
    pub struct WhisperRecipe;       // `Recipe`: Default, or `with_models(&'static [Model])` for a test's own list
}
```

## The program

`WhisperRecipe` keeps whisper.cpp's command line as `whisper` (MIT, ggml-org), with no model and no channel in its listing: the models (`base`, `base.en`, `tiny`, `tiny.en`, `small-q5_1`, `small`, `small.en`, `large-v3-turbo-q5_0`) are the `ggml-*.bin` files of the publisher's Hugging Face repository at one pinned revision, checked against the SHA-256 that revision lists, and are installed on their own (below). A release channel that names a model (the speech setup does) still installs that model with the program, as it always did, and `channel_held` makes an update of such a copy keep it. The program is the Windows or Linux build (x86-64 and ARM64; none for macOS) from the release's own files, or, for a `vX.Y.Z` release that carries none, from the `bNNNN` build release published up to six hours before it (the program reports the version of that commit, which the install checks); each is checked against the SHA-256 GitHub lists. The copy is `bin/whisper-cli` in the version's folder (and `models/ggml-<model>.bin` there when a channel named a model), on no `PATH`; a plan names such a model in its downloads, and the engine asks `channel_of` for it again at apply. Neither publisher's signature is checked, and the plan says so.

## The models page and the speech offer

`WhisperModels` is the `model_store` of the type `whisper`. `list` answers the eight models as rows (runtime, size, file and digest as facts; `speech-to-text` and `English only` or `Every language` as the tags), each with its file (`files`: the publisher's address, the name in the host's models folder, the size and the SHA-256). A model is its own install: its row offers `fetch_files` while it is in neither place, which the host performs once the program is installed (and refuses, naming whisper.cpp, before), and `delete_files` once it is in the host's models folder (`query.folder`). Every model installed is `on`: state `installed` in the host's folder, `installed with the program` in the copy's own `models` folder, which has no action since it goes with the program. `act` and `keep` refuse `UNSUPPORTED`. Its `describe` says the speech service is a program (`SpeechService::program`, licence MIT). `service` is that offer: with `query.models` it lists every model (the default first), and with `query.program` (the launch path of the host's managed copy) it answers `command`, `speech_command`'s `program`, `arguments(model)`, `input` `wav` and the model it runs (`query.model` when that is installed, else the first installed), or `unavailable` in its own words (`whisper.cpp is not installed: install it in the Store first`, `no model is installed: install one from the Models page`, an unexpected layout). `-of` only makes the program print to its standard output, which it does not for a stream otherwise; nothing is written to a file.

## Gotchas

- The program prints `whisper.cpp version: X.Y.Z`; `parse_version` accepts that line and nothing around it.
- `held_model` reads the version's folder, so a copy the person installed themselves (no `models/` folder) holds no model of its own; it runs one installed on its own from the host's folder.
- The recipe's tests and the plan's checks run in `canoa-host`'s `tests/store_runtimes.rs`, which serves the publishers' listings from a world of documents.
