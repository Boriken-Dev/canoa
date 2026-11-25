# `canoa-piper` — API header

Header-file-style reference for `canoa-piper`: Piper for a Canoa host (the Store program that installs its text-to-speech command line, the Store type that lists it, the voices page of that type, and the `speech_output` provider that speaks with the installed copy), so it can be used without reading its source. It ships inside the package and is self-contained. Development documentation lives with the source at <https://github.com/Boriken-Dev/canoa>.

MSRV Rust 1.89. Version `0.0.0-alpha.0`, not published (`publish = false`). `#![forbid(unsafe_code)]`. Depends on `canoa` (features `store`, `speech-builtin`), `coqui-acp-api`, `guatiao`, `guatiao-intake` and `serde_json`.

What a host loads is `library` (below). The host names none of it: it carries the library as it carries any other, lists the type and its voices through the generic Store calls, and offers the provider under the type's id (`piper`) once the program is installed, from the settings `PiperModels::service` answers.

## Exports

```rust
pub const LIBRARY: &str = "canoa-piper";   // the name `library` is registered under
pub const STORE_TYPE: &str = "piper";      // the id of the Store type, and the name the host offers the installed copy under
pub const PROVIDER: &str = "speech_say_piper";   // the id of the speech-output provider

/// What a host links: handed to `Registry::register_local`.
pub fn library(host: guatiao::library::Host) -> Option<LibraryParts>;   // PiperStore, PiperExtension, PiperModels, PiperVoices, PiperSpeech

pub struct PiperStore;        // a `program_store`, id "piper_store", over `store::PiperRecipe`
pub struct PiperExtension;    // a `store_extension`, id "piper_extension": the type `piper` ("Piper", icon `speaker`, store `piper_store`, models `piper_voices`)
pub struct PiperModels;       // a `model_store`, id "piper_voices": `describe().voice` is `speech_say_piper`; `service` says which voices the installed copy speaks with; its page lists nothing of its own, the voices are the catalogs'
pub struct PiperSpeech;       // a `speech_output`, id "speech_say_piper", built `From<PiperSettings>`
pub struct PiperSettings { pub program: String, pub voices_dir: String /* folders, separated as a search path is */, pub default_voice: Option<String>, pub timeout_seconds: u32 }   // `PiperSettings::new(program, voices_dir)`

pub struct PiperVoices;       // a `catalog_source`, id "piper_voices", serving the type `piper`: the voices of a Hugging Face repository laid out as `rhasspy/piper-voices` is
pub struct PiperVoicesSettings { pub repo: String, pub include: Vec<String>, pub languages: Vec<String> }   // voice keys as patterns with `*`; language families (`en`, `es`); none meaning every one
pub const SOURCE: &str = "piper_voices";
pub fn catalog_of(repo, revision, voices_json, include, languages, listing: impl Fn(&str) -> Result<String, ProviderError>) -> Result<ModelsCatalog, String>;

pub mod catalog {
    pub const LABEL: &str = "rhasspy";       // the default catalog's label: `[store.piper.catalogs.rhasspy]`
    pub const DEFAULT_REPO: &str;            // rhasspy/piper-voices
    pub const REVISION: &str;                // the commit it is pinned to
    pub const DEFAULT_VOICES: [&str; 2];     // en_US-ljspeech-medium, es_MX-claude-high
    pub const SNAPSHOT: &str;                // what the source answers at REVISION (`catalogs/rhasspy.json`)
    pub fn default_catalog() -> DefaultCatalog;
}

pub mod store {
    pub const VERSION: &str;        // "1.2.0", what Piper prints and the one release this store offers
    pub const RELEASE: &str;        // where the release's files are
    pub const PROGRAM_DIR: &str;    // "piper", the program's folder in a version's folder
    pub struct Catalog { /* version, release, endpoints, engines: Vec<Engine> */ }   // Default is the shipped one: the archives and the SHA-256 Canoa measured of each
    pub struct PiperRecipe;         // `Recipe`: Default, or `over(Catalog)` for a test's own
}
```

## The program

`PiperRecipe` keeps Piper's last binary release (`2023.11.14-2`, which prints `1.2.0`; the project is archived and the release publishes no checksums, so each digest is the SHA-256 Canoa measured from the published file) as `piper/`, with no voice and no channel in its listing: voices are installed on their own from the Models page (see below). Of the voices this store once installed with the program, `en_US-ljspeech-medium` (public domain) and `es_MX-claude-high` (Apache 2.0) are the catalog's defaults, each pinned to one commit of `rhasspy/piper-voices`, are installed on their own (below). A release channel (`standard` for both voices, `english`, `spanish`) still installs those voices with the program, in the copy's own `voices/` folder, as the speech setup does, and `channel_held` makes an update of such a copy keep them. Windows on ARM runs the x64 build under emulation (measured); a Linux archive's libraries' short names, which an unpack leaves out as links, are written as copies by the install. The program is on no `PATH`.

## The voices page and the voice offer

The type registers one catalog, `rhasspy` (`canoa.models/1`), pinned to one commit of the publisher's repository with its two voices shipped as a snapshot. Its source `piper_voices` reads `voices.json` (which gives an MD5 only, so it names voices and files, nothing it is believed by) and, for each voice, the listing of its folder: the model's SHA-256 is the listing's `lfs.oid`, and the small `.onnx.json`, which the listing gives only by its git blob id, is checked by that id, which says what the file holds because its address names the commit. A source that follows a branch first turns it into the commit it is at. The host lists the catalog's voices on the page and downloads a voice's two files checked by those digests, once the program is installed (and refuses, naming Piper, before); a voice in the host's models folder offers `delete_files`. A voice's licence is shown only for the two whose model cards were read. `PiperModels::service` answers the settings the voice provider is built from, which read the host's folder and the copy's `voices` folder.

## The provider

`PiperSpeech` runs the program per request over folders of voices (`voices_dir`, separated as the system's search path is, a voice found in two being the first one's; a folder that is not there holds none; `NAME.onnx` with `NAME.onnx.json`). A voice's id is `NAME`; its language, sample rate and speakers come from the json, and a speaker of a model with several is `NAME#N` (the first 32 named in the model are listed). The program gets `--model`, `--output-raw`, `--quiet`, `--length_scale` (1 / speed, 0.5 to 2) and `--speaker`. A voice left out is `default_voice`, else the first in the request's language, else the first. The text stays on the computer.

## Gotchas

- The program prints `1.2.0` alone; `parse_version` accepts that and nothing around it.
- The program's digests are the one list Canoa still holds: the release publishes none, so they are measured and pinned here, and its scope (`piper`) is the type's, which the voices' catalogs use.
- The recipe's plan checks run in `canoa-host`'s `tests/store_piper.rs`; the provider's tests run in this crate's `tests/speech.rs` against `examples/piper_standin.rs`, which needs the examples built first.
