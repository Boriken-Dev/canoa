//! The `model_store` kind: the models a Store type lists, moved and set up by the library that knows them.
//!
//! A Store type an extension adds ([`ExtensionType`](super::extension::ExtensionType)) may name a `model_store` provider. The
//! provider lists the type's models as rows (what each is, what its `+` does, the pages' sections), runs one action on one row
//! through the spawner the host hands it (the program the type's `program_store` installed), keeps the settings of a row, and
//! says which of its models the person's speech-to-text may call. The host and the application hold no model's name: a build
//! without the library lists none.
//!
//! The models are plain data, the same in every language.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock is std's"
)]

use alloc::string::String;
use alloc::vec::Vec;
use core::time::Duration;
use std::time::Instant;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue, Value};
use coqui_acp_api::{
    ErrorOutput, ProcessSpawner, READ_LINK_DOWN, READ_TIMED_OUT, SpawnSpec, SpawnedProcess,
};
use guatiao_intake::Screen;

/// The name of the `model_store` kind.
pub const KIND: &str = "model_store";

/// The tables of canoa.cfg that are the host's own, which a provider's [`ModelSettings::table`] may not name.
pub const RESERVED_TABLES: [&str; 29] = [
    "accounts",
    "agents",
    "appearance",
    "assets",
    "attachments",
    "connections",
    "defaults",
    "desktop",
    "env",
    "extensions",
    "host",
    "loadouts",
    "models",
    "network",
    "notifications",
    "overlays",
    "permissions",
    "profile",
    "profiles",
    "say",
    "secrets",
    "sessions",
    "settings",
    "skill_pins",
    "skills",
    "speech",
    "store",
    "usage",
    "voice",
];

/// The step of a [`RowAction`] the host does itself, in place of asking [`ModelStore::act`]: it downloads the row's
/// [`ModelRow::files`] into the folder the host keeps the type's models in, each checked against its SHA-256, and refuses when the
/// type's program is not installed.
pub const STEP_FETCH_FILES: &str = "fetch_files";

/// The step the host does itself that takes the row's [`ModelRow::files`] out of that folder.
pub const STEP_DELETE_FILES: &str = "delete_files";

/// The most of a program's output that [`run`] keeps from each of its two streams.
pub const MOST_OUTPUT: usize = 8 * 1024 * 1024;

/// What a model store is, from [`ModelStore::describe`]. The same answer on every call, with nothing run.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelStoreInfo {
    /// The name of the page that lists the models, in the plural, as the Store's index says it.
    pub title: String,
    /// Where the rows come from, in the words the page's filter uses.
    pub catalog: String,
    /// Who the rows are by, when a row names no one.
    pub publisher: String,
    /// What the page says in place of rows when [`ModelList::unavailable`] is set and says nothing of its own.
    pub missing: String,
    /// The settings a row keeps, when it keeps any.
    pub settings: Option<ModelSettings>,
    /// How the person's speech-to-text calls the service that runs these models, when it can.
    pub speech: Option<SpeechService>,
    /// The id of the library's `speech_output` provider that reads text aloud with the type's installed program, when it can:
    /// [`ServiceReport::voice`] says the settings that provider is built from.
    pub voice: Option<String>,
    /// The `https` origins the files of a row are downloaded from, for a type that has no `program_store`: the host then keeps the
    /// models in a folder of its own for the type and downloads only from these. A type with a program store takes that store's
    /// endpoints and this stays empty.
    #[map(default)]
    pub endpoints: Vec<String>,
}

/// The per-model settings a store keeps: a guatiao schema of an object, and beside it an optional guatiao-intake form, drawn by
/// the application's one renderer. The host checks the values against the schema, asks [`ModelStore::keep`] to settle them and
/// writes the answer to `[<table>.models."<id>"]` of canoa.cfg, which the library may read back (a gateway's `table`).
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelSettings {
    /// The table of canoa.cfg that is the library's own: 1 to 32 lower-case letters, digits and `_`, and none of
    /// [`RESERVED_TABLES`].
    pub table: String,
    /// A line saying what the settings are for.
    pub help: Option<String>,
    /// The members the settings hold.
    pub schema: Value,
    /// How the schema is shown; none shows the form it implies.
    pub form: Option<Value>,
}

/// The service that runs a store's models, as far as the person's speech-to-text is concerned. It is a server that transcribes
/// one recording, at an address that changes and that [`ModelStore::service`] finds (`route` says where); a program run once for
/// each recording (`command`), which [`ModelStore::service`] says how to run from the copy the host installed; or a provider of
/// the library's own that transcribes inside the host's process (`provider`), which [`ModelStore::service`] says the settings of.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SpeechService {
    /// The name of the offer.
    pub title: String,
    /// What it is, in a sentence.
    pub summary: String,
    /// The licence of what transcribes, when it has one to say.
    pub license: Option<String>,
    /// Where the audio goes, in a sentence or two, as the offer says it before the person sets it up.
    pub location: String,
    /// The path below the service's address that takes a recording, starting with `/`; empty for a `command`.
    #[map(default)]
    pub route: String,
    /// Whether the recording is written to a file on this computer and its name posted, in place of the audio being sent.
    #[map(default)]
    pub local_file: bool,
    /// Whether it is a program run once for each recording, with the audio on its standard input, in place of a server: the
    /// offer is then installed through the type's first `program_store`, with a model chosen as that store's channel, and
    /// [`ServiceReport::command`] says how the installed copy is run.
    #[map(default)]
    pub command: bool,
    /// The id of the library's `speech_provider` that transcribes inside the host's process, in place of a server or a program: the
    /// models are the type's own files in the models folder, and [`ServiceReport::settings`] says the settings the provider is
    /// built from. It is ready when the report names no `unavailable`; there is no program to install.
    pub provider: Option<String>,
}

/// What a store is asked to list.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct ModelQuery {
    /// The absolute path of the program the type's `program_store` installed or found (the host's own copy first), or none when
    /// it found no copy.
    pub program: Option<String>,
    /// The person's home folder, where programs keep their own files.
    pub home: Option<String>,
    /// The folder the host keeps the models the person installed on their own in, which may not exist yet: a row's
    /// [`ModelFile::file`] is a path in it.
    pub folder: Option<String>,
}

/// One file of a model, which the host downloads when a row's [`STEP_FETCH_FILES`] runs.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelFile {
    /// Where it is served from: an `https` address in one of the endpoints of the type's first program store.
    pub url: String,
    /// Where it is kept, relative to the models folder: plain names joined by `/`.
    pub file: String,
    /// Its size in bytes, when the publisher says; the download may not pass it.
    pub bytes: Option<u64>,
    /// Its SHA-256 in hexadecimal, which the download is checked against.
    pub sha256: String,
}

/// One section of a model's page: a title, facts, paragraphs and, when `settings` is set, the form of the model's settings.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Section {
    /// Its heading.
    pub title: String,
    /// What it states, one label and value each.
    #[map(default)]
    pub facts: Vec<Fact>,
    /// What it says, a paragraph each.
    #[map(default)]
    pub paragraphs: Vec<String>,
    /// Whether the form of [`ModelSettings`] is drawn in it, after the paragraphs.
    #[map(default)]
    pub settings: bool,
}

/// One thing a page states of a model.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Fact {
    /// What it is.
    pub label: String,
    /// What it holds.
    pub value: String,
}

/// What `+` does for a row, once the person has read what it takes.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct RowAction {
    /// The title of the confirmation.
    pub title: String,
    /// What it takes, in the words the confirmation shows.
    pub confirm: String,
    /// What the confirming button says.
    pub label: String,
    /// The actions [`ModelStore::act`] is asked for, in order; the next is asked only when the one before is done.
    pub steps: Vec<String>,
    /// Whether it takes the model away: the application offers it as Remove, beside an installed model, and never as `+`.
    #[map(default)]
    pub removes: bool,
}

/// One build of a model that runs on other hardware or in another form (a CPU, a GPU or an NPU build of the same model), listed
/// inside the row of the model. It has its own name for the other methods, its own state and its own actions.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelBuild {
    /// The name the other methods take for this build.
    pub id: String,
    /// What tells it from the model's other builds, in a few words.
    pub label: String,
    /// Where it stands and what it is, in a line.
    pub summary: String,
    /// Whether it is on this computer.
    #[map(default)]
    pub installed: bool,
    /// Its state in a word, when it has one worth a badge.
    pub state: Option<String>,
    /// What its part of the page states.
    #[map(default)]
    pub facts: Vec<Fact>,
    /// What it can do; the removal, when it has one, is marked [`RowAction::removes`].
    #[map(default)]
    pub actions: Vec<RowAction>,
    /// What its part of the page shows, in order; the form of the model's settings is drawn here for this build, which keeps them
    /// under its own `id`.
    #[map(default)]
    pub sections: Vec<Section>,
    /// Present when this build may be used as the person's speech-to-text, under its own `id`.
    pub speech: Option<SpeechUse>,
}

/// How a row may be used as the person's speech-to-text.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SpeechUse {
    /// What the page says of using the model for speech, a paragraph.
    pub text: String,
    /// What the page warns of now (the model is not ready), when it does.
    pub caution: Option<String>,
}

/// One model of a type's page.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelRow {
    /// The name the other methods take, and the row's name.
    pub id: String,
    /// Who it is by, when not the store's publisher; the row's second line.
    pub by: Option<String>,
    /// Where it stands and what it is, in a line.
    pub summary: String,
    /// What the page's tag filter lists it under.
    #[map(default)]
    pub tags: Vec<String>,
    /// Whether the tile is marked: the model is in use.
    #[map(default)]
    pub on: bool,
    /// Its state in a word, when it has one worth a badge.
    pub state: Option<String>,
    /// What its page's overview states.
    #[map(default)]
    pub facts: Vec<Fact>,
    /// What it can do; `+` does the first.
    #[map(default)]
    pub actions: Vec<RowAction>,
    /// What its page shows, in order.
    #[map(default)]
    pub sections: Vec<Section>,
    /// Present when it may be used as the person's speech-to-text.
    pub speech: Option<SpeechUse>,
    /// The ways it can be run, each a pill on its tile: installed once what it reads is in the folder, available until then.
    #[map(default)]
    pub engines: Vec<RowEngine>,
    /// The files that are the model, which the host downloads for [`STEP_FETCH_FILES`] and takes away for [`STEP_DELETE_FILES`].
    /// A model whose program fetches and keeps its own files has none, and the host shows it as not verified.
    #[map(default)]
    pub files: Vec<ModelFile>,
    /// The builds of the model, when it has several; `actions` then act on the model as a whole and the program picks the build.
    #[map(default)]
    pub builds: Vec<ModelBuild>,
}

/// One way a row can be run, which the tile shows as a pill.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct RowEngine {
    /// The name the install dialog's choices are keyed by.
    pub id: String,
    /// What the pill says.
    pub label: String,
    /// `installed` once every file it reads is in the folder, else `available`.
    pub state: String,
    /// The forms the install writes its file in, when the person picks one; the first is the default.
    #[map(default)]
    pub variants: Vec<RowVariant>,
}

/// One form an engine's file can be written in.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct RowVariant {
    /// What [`ModelAct::options`] names.
    pub id: String,
    /// What the dialog says.
    pub label: String,
    /// What it says beside it.
    pub detail: Option<String>,
}

/// The rows of a page, from [`ModelStore::list`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelList {
    /// The rows, in the order the page shows them.
    #[map(default)]
    pub models: Vec<ModelRow>,
    /// Why there are none to list now, in the page's words (the program is not installed); none when there are rows or the
    /// list is empty because the program lists no model.
    pub unavailable: Option<String>,
}

/// One action on one row.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct ModelAct {
    /// The program, as [`ModelQuery::program`].
    pub program: Option<String>,
    /// The person's home folder.
    pub home: Option<String>,
    /// The row's id.
    pub model: String,
    /// One of the actions a [`RowAction`] named.
    pub action: String,
    /// The folder the host keeps the type's models in, as [`ModelQuery::folder`]: the files of the row are paths in it.
    pub folder: Option<String>,
    /// What the person picked in the install dialog, a JSON object that gives each engine's id the id of the variant chosen for
    /// it; none when the row offered no choice.
    pub options: Option<String>,
}

/// What an action did.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelActed {
    /// A line the person may want to read afterwards.
    pub note: Option<String>,
}

/// A row's settings, as the person gave them.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct ModelKeep {
    /// The person's home folder.
    pub home: Option<String>,
    /// The row's id.
    pub model: String,
    /// A JSON object keyed by the schema's members, already checked against it; a member left empty is absent, and no member at
    /// all takes the row's settings away.
    pub values: String,
}

/// The settings to keep.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelKept {
    /// A JSON object of what the host writes: the values as given, or settled (a number held to the model's own limit). `{}`
    /// takes the row's settings away.
    pub values: String,
    /// A line saying what was changed (a value lowered to what the model allows).
    pub note: Option<String>,
}

/// What the person's speech-to-text asks of the service.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct ServiceQuery {
    /// The person's home folder.
    pub home: Option<String>,
    /// The absolute path of the host's own copy of the type's program, for a `command` service, when the host has one.
    pub program: Option<String>,
    /// The folder the host keeps the models the person installed on their own in, as [`ModelQuery::folder`].
    pub folder: Option<String>,
    /// The model to run, for a `command` service, when the person chose one of several installed; none is the library's own choice.
    pub model: Option<String>,
    /// Whether to list the models that can transcribe, which may take a request to the service.
    #[map(default)]
    pub models: bool,
}

/// One model that can transcribe.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ServedModel {
    /// The name a recording's request names.
    pub id: String,
    /// What the offer calls it.
    pub label: String,
}

/// Where the service is and what it serves.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ServiceReport {
    /// The address, with no path, when the service is up.
    pub address: Option<String>,
    /// The models that can transcribe, when asked for and the service is up.
    #[map(default)]
    pub models: Vec<ServedModel>,
    /// Why the service cannot take a recording now, in the offer's words; none when it can.
    pub unavailable: Option<String>,
    /// How to run the copy of a `command` service, when there is a copy that can be.
    pub command: Option<SpeechRun>,
    /// The settings of the `speech_output` provider [`ModelStoreInfo::voice`] names, when the installed copy can read text aloud:
    /// an object that provider's schema accepts.
    pub voice: Option<Value>,
    /// The settings of the `speech_provider` [`SpeechService::provider`] names, for the model the person asked for when it is
    /// installed and else the first that is: an object that provider's schema accepts.
    pub settings: Option<Value>,
    /// The id of the `speech_provider` `settings` are for, when it is not [`SpeechService::provider`]: a service that several
    /// engines run the same model on names the one that runs it here.
    pub provider: Option<String>,
}

/// How a program turns one recording into text, which the host writes as the person's speech provider.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SpeechRun {
    /// The absolute path of the program.
    pub program: String,
    /// Its arguments. `{language_code|auto}` is the language the person said (or `auto`) and `{vocabulary}` their own words.
    #[map(default)]
    pub args: Vec<String>,
    /// How the audio reaches it: `wav` or `pcm`.
    pub input: String,
    /// The model this copy runs, as one of [`ServiceReport::models`] names it.
    pub model: String,
}

impl ModelSettings {
    /// Settings of the members `S` holds, shown with the screen `S` declares, kept in `table`.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn asking<S: Schema + Screen>(table: impl Into<String>) -> Result<Self, ProviderError> {
        let (schema, form) = crate::settings::describe::<S>()?;
        Ok(Self {
            table: table.into(),
            help: None,
            schema,
            form: Some(form),
        })
    }
}

impl ModelStoreInfo {
    /// A store of rows from `catalog`, with no settings, no speech and no voice.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        catalog: impl Into<String>,
        publisher: impl Into<String>,
        missing: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            catalog: catalog.into(),
            publisher: publisher.into(),
            missing: missing.into(),
            settings: None,
            speech: None,
            voice: None,
            endpoints: Vec::new(),
        }
    }

    /// Why this store cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        for (name, text) in [
            ("title", &self.title),
            ("catalog", &self.catalog),
            ("publisher", &self.publisher),
        ] {
            if text.trim().is_empty() {
                return Some(format!("a model store has no {name}"));
            }
        }
        if let Some(settings) = &self.settings {
            let word = (1..=32).contains(&settings.table.len())
                && settings
                    .table
                    .bytes()
                    .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_');
            if !word {
                return Some(format!(
                    "the table {:?} is not 1 to 32 lower-case letters, digits and _",
                    settings.table
                ));
            }
            if RESERVED_TABLES.contains(&settings.table.as_str()) {
                return Some(format!("{:?} is a table the host keeps", settings.table));
            }
            if let Some(why) =
                crate::settings::problem(Some(&settings.schema), settings.form.as_ref())
            {
                return Some(why);
            }
        }
        if let Some(bad) = self
            .endpoints
            .iter()
            .find(|origin| !origin.starts_with("https://"))
        {
            return Some(format!("the endpoint {bad:?} is not an https address"));
        }
        if self.voice.as_deref().is_some_and(|id| id.trim().is_empty()) {
            return Some("its voice provider has no id".into());
        }
        if let Some(speech) = &self.speech {
            if speech.title.trim().is_empty() {
                return Some("its speech service has no title".into());
            }
            if let Some(provider) = &speech.provider {
                if provider.trim().is_empty() {
                    return Some("its speech service names a provider with no id".into());
                }
                if speech.command || !speech.route.is_empty() {
                    return Some(
                        "its speech service runs in the host's process and is also a program or has a route"
                            .into(),
                    );
                }
            } else {
                if speech.command && !speech.route.is_empty() {
                    return Some("its speech service is a program and has a route".into());
                }
                if !speech.command && !speech.route.starts_with('/') {
                    return Some(
                        "its speech service has a route that does not start with /".into(),
                    );
                }
            }
        }
        None
    }
}

/// The models of a Store type, and what the person does with each.
///
/// Every method is required and synchronous, and every object a method is handed is the provider's to destroy. A provider with no
/// settings refuses `keep`, and one with no speech service refuses `service`, `UNSUPPORTED` with
/// its reason; the host asks neither unless `describe` offers it.
#[coqui_acp_api::guatiao::kind(name = "model_store", crate = ::coqui_acp_api::guatiao)]
pub trait ModelStore: Send + Sync {
    /// What this store is. The same answer on every call, with nothing run.
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError>;

    /// The rows now. Never changes anything; a program that cannot answer is a refusal with its own words, and a program the host
    /// found no copy of is the list's `unavailable`.
    fn list(
        &self,
        query: ModelQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelList, ProviderError>;

    /// One action on one row, to its end: the provider runs its program through `spawner`, within the time its own action takes,
    /// and ends it when it is over. A program's refusal is a refusal here, in the program's words.
    fn act(
        &self,
        request: ModelAct,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelActed, ProviderError>;

    /// Settles the settings of one row; the host keeps what it answers.
    fn keep(
        &self,
        request: ModelKeep,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelKept, ProviderError>;

    /// Where the service that runs the models is now, and which of them can transcribe.
    fn service(
        &self,
        query: ServiceQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ServiceReport, ProviderError>;
}

/// What a program printed and how it ended.
#[derive(Debug, Clone, PartialEq, Eq)]
#[non_exhaustive]
pub struct Finished {
    /// Its exit code, when it exited by itself.
    pub code: Option<i64>,
    /// What it wrote to standard output, at most [`MOST_OUTPUT`] bytes.
    pub out: String,
    /// What it wrote to standard error, at most [`MOST_OUTPUT`] bytes.
    pub err: String,
}

/// Runs `program` with `args` through `spawner` until the program exits, or ends it (and what it started) when `limit` is over.
///
/// It ends when the program exits, not when its pipes close: a service the program started and left running keeps them open,
/// so once the program has exited only what is already buffered is read, for at most a second. `run` ends nothing the
/// program started once it has exited (the spawner may, when it lets go of the process); only the limit ends the program,
/// and what it started.
///
/// # Errors
///
/// What the spawner refuses, or that the program did not finish in time (it is killed first).
pub fn run(
    spawner: &Object<dyn ProcessSpawner>,
    program: &str,
    args: &[String],
    limit: Duration,
) -> Result<Finished, ProviderError> {
    let mut spec = SpawnSpec::new(program).error_output(ErrorOutput::Separate);
    for arg in args {
        spec = spec.arg(arg.as_str());
    }
    let process = spawner.spawn(spec)?;
    let _ = process.close_input();
    let started = Instant::now();
    let mut out = Stream::default();
    let mut err = Stream::default();
    let exit = loop {
        if started.elapsed() >= limit {
            let _ = process.kill();
            let _ = process.wait(500);
            let said = last_words(
                &format!(
                    "{}\n{}",
                    String::from_utf8_lossy(&out.bytes),
                    String::from_utf8_lossy(&err.bytes)
                ),
                LAST_WORDS,
            );
            let how = format!(
                "{program} did not finish within {} minutes and was ended",
                limit.as_secs().div_ceil(60)
            );
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                &if said.is_empty() {
                    format!("{how}; it had written nothing")
                } else {
                    format!("{how}; its last words: {said}")
                },
            ));
        }
        let both = !out.done && !err.done;
        if !out.done {
            out.take(&process, false, if both { 25 } else { 100 })?;
        }
        if !err.done {
            err.take(&process, true, if both { 0 } else { 100 })?;
        }
        let exit = process.wait(0)?;
        if !exit.running {
            break exit;
        }
        if out.done && err.done {
            // Both pipes are closed and the program runs on: wait for it, within the limit.
            let _ = process.wait(100)?;
        }
    };
    let drain = Instant::now();
    for (stream, error) in [(&mut out, false), (&mut err, true)] {
        // A read that times out is not the end: the pipe's reader may be late on a busy machine, so it is read until it is
        // closed or the time is over.
        while !stream.done && drain.elapsed() < DRAIN {
            stream.take(&process, error, DRAIN_POLL_MS)?;
        }
    }
    Ok(Finished {
        code: exit.code,
        out: String::from_utf8_lossy(&out.bytes).into_owned(),
        err: String::from_utf8_lossy(&err.bytes).into_owned(),
    })
}

/// The most [`run`] reads after the program has exited, for what its pipes still hold.
const DRAIN: Duration = Duration::from_secs(1);

/// How much of a program's last words an error carries.
const LAST_WORDS: usize = 600;

/// The end of `text` as a person reads it: the escapes a terminal would draw and the control characters taken out, blank
/// lines dropped, and at most the last `most` characters (marked with a leading `…` when it was cut). Empty when the program
/// said nothing.
#[must_use]
pub fn last_words(text: &str, most: usize) -> String {
    let mut clean = String::new();
    let mut chars = text.chars().peekable();
    while let Some(c) = chars.next() {
        if c == '\u{1b}' {
            if chars.next_if_eq(&'[').is_some() {
                for next in chars.by_ref() {
                    if next.is_ascii_alphabetic() {
                        break;
                    }
                }
            }
        } else if c == '\n' || !c.is_control() {
            clean.push(c);
        }
    }
    let lines: Vec<&str> = clean
        .lines()
        .map(str::trim)
        .filter(|line| !line.is_empty())
        .collect();
    let joined = lines.join("\n");
    let count = joined.chars().count();
    if count <= most {
        return joined;
    }
    let tail: String = joined.chars().skip(count - most).collect();
    format!("…{}", tail.trim_start())
}

/// How long one read waits for a pipe's last bytes once the program has exited.
const DRAIN_POLL_MS: u64 = 50;

/// One of a process's two output streams, read as far as it goes.
#[derive(Default)]
struct Stream {
    bytes: Vec<u8>,
    done: bool,
}

impl Stream {
    /// Reads once; whether it found anything (a closed stream counts: it is settled).
    fn take(
        &mut self,
        process: &Object<dyn SpawnedProcess>,
        error: bool,
        wait_ms: u64,
    ) -> Result<bool, ProviderError> {
        let mut buffer = [0u8; 8192];
        let count = if error {
            process.read_error(&mut buffer, wait_ms)?
        } else {
            process.read(&mut buffer, wait_ms)?
        };
        if count == READ_TIMED_OUT || count == READ_LINK_DOWN {
            return Ok(false);
        }
        if count <= 0 {
            self.done = true;
            return Ok(true);
        }
        let count = usize::try_from(count).unwrap_or(0).min(buffer.len());
        let room = MOST_OUTPUT.saturating_sub(self.bytes.len());
        self.bytes.extend_from_slice(&buffer[..count.min(room)]);
        Ok(true)
    }
}

impl Fact {
    /// A fact: what it is and what it holds.
    #[must_use]
    pub fn new(label: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            label: label.into(),
            value: value.into(),
        }
    }
}

impl Section {
    /// A section called `title` with nothing in it.
    #[must_use]
    pub fn new(title: impl Into<String>) -> Self {
        Self {
            title: title.into(),
            facts: Vec::new(),
            paragraphs: Vec::new(),
            settings: false,
        }
    }
}

impl RowAction {
    /// An action that confirms with `title`, `confirm` and `label` and then asks for `steps` in order.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        confirm: impl Into<String>,
        label: impl Into<String>,
        steps: impl IntoIterator<Item = impl Into<String>>,
    ) -> Self {
        Self {
            title: title.into(),
            confirm: confirm.into(),
            label: label.into(),
            steps: steps.into_iter().map(Into::into).collect(),
            removes: false,
        }
    }

    /// This action marked as the one that takes the model away.
    #[must_use]
    pub const fn removing(mut self) -> Self {
        self.removes = true;
        self
    }
}

impl ModelBuild {
    /// A build named `id`, told from the others by `label`, standing as `summary` says, with nothing else.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        label: impl Into<String>,
        summary: impl Into<String>,
    ) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            summary: summary.into(),
            installed: false,
            state: None,
            facts: Vec::new(),
            actions: Vec::new(),
            sections: Vec::new(),
            speech: None,
        }
    }
}

impl SpeechUse {
    /// What the page says of using the model for speech, with no warning.
    #[must_use]
    pub fn new(text: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            caution: None,
        }
    }
}

impl ModelRow {
    /// A row named `id` that stands as `summary` says, with nothing else.
    #[must_use]
    pub fn new(id: impl Into<String>, summary: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            by: None,
            summary: summary.into(),
            tags: Vec::new(),
            on: false,
            state: None,
            facts: Vec::new(),
            actions: Vec::new(),
            sections: Vec::new(),
            speech: None,
            engines: Vec::new(),
            files: Vec::new(),
            builds: Vec::new(),
        }
    }
}

impl RowEngine {
    /// An engine called `label`, `installed` or not, with no choice of forms.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>, installed: bool) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            state: if installed { "installed" } else { "available" }.to_owned(),
            variants: Vec::new(),
        }
    }
}

impl RowVariant {
    /// A form named `id`, said as `label`, with what the dialog says beside it.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>, detail: Option<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            detail,
        }
    }
}

impl ModelFile {
    /// The file `file` of the models folder, downloaded from `url` and checked against `sha256`.
    #[must_use]
    pub fn new(
        url: impl Into<String>,
        file: impl Into<String>,
        bytes: Option<u64>,
        sha256: impl Into<String>,
    ) -> Self {
        Self {
            url: url.into(),
            file: file.into(),
            bytes,
            sha256: sha256.into(),
        }
    }
}

impl ServedModel {
    /// A model a recording may name, as the offer calls it.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
        }
    }
}

impl SpeechService {
    /// A service that takes a recording at `route`, in an offer of `title`.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        summary: impl Into<String>,
        location: impl Into<String>,
        route: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            summary: summary.into(),
            license: None,
            location: location.into(),
            route: route.into(),
            local_file: false,
            command: false,
            provider: None,
        }
    }

    /// A program run for each recording, in an offer of `title`.
    #[must_use]
    pub fn program(
        title: impl Into<String>,
        summary: impl Into<String>,
        location: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            summary: summary.into(),
            license: None,
            location: location.into(),
            route: String::new(),
            local_file: false,
            command: true,
            provider: None,
        }
    }

    /// The library's own `speech_provider` `provider`, which transcribes inside the host's process, in an offer of `title`.
    #[must_use]
    pub fn in_process(
        title: impl Into<String>,
        summary: impl Into<String>,
        location: impl Into<String>,
        provider: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            summary: summary.into(),
            license: None,
            location: location.into(),
            route: String::new(),
            local_file: false,
            command: false,
            provider: Some(provider.into()),
        }
    }
}

impl SpeechRun {
    /// `program` run with `args`, reading `input`, with the model `model`.
    #[must_use]
    pub fn new(
        program: impl Into<String>,
        args: impl IntoIterator<Item = impl Into<String>>,
        input: impl Into<String>,
        model: impl Into<String>,
    ) -> Self {
        Self {
            program: program.into(),
            args: args.into_iter().map(Into::into).collect(),
            input: input.into(),
            model: model.into(),
        }
    }
}

impl ModelAct {
    /// `action` on the row `model`, with no program and no home.
    #[must_use]
    pub fn new(model: impl Into<String>, action: impl Into<String>) -> Self {
        Self {
            program: None,
            home: None,
            model: model.into(),
            action: action.into(),
            folder: None,
            options: None,
        }
    }
}

impl ModelKeep {
    /// The settings `values` of the row `model`.
    #[must_use]
    pub fn new(model: impl Into<String>, values: impl Into<String>) -> Self {
        Self {
            home: None,
            model: model.into(),
            values: values.into(),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn last_words_keep_the_end_of_what_a_program_said_without_its_escapes() {
        assert_eq!(last_words("", 50), "");
        assert_eq!(last_words("\n  \n", 50), "");
        assert_eq!(
            last_words(
                "\u{1b}[31merror:\u{1b}[0m no space\u{7}\n\n  try again",
                100
            ),
            "error: no space\ntry again"
        );
        assert_eq!(
            last_words("a banner that is long\nthe real reason", 15),
            "…the real reason"
        );
    }

    fn info(speech: SpeechService) -> ModelStoreInfo {
        let mut info = ModelStoreInfo::new("t", "c", "p", "m");
        info.speech = Some(speech);
        info
    }

    #[test]
    fn a_voice_provider_is_named_by_an_id() {
        let mut named = ModelStoreInfo::new("t", "c", "p", "m");
        assert_eq!(named.problem(), None, "a store needs no voice");
        named.voice = Some("speech_say_x".to_owned());
        assert_eq!(named.problem(), None);
        named.voice = Some(" ".to_owned());
        assert!(named.problem().is_some_and(|why| why.contains("voice")));
    }

    #[test]
    fn a_speech_service_is_a_server_with_a_route_or_a_program_with_none() {
        assert_eq!(
            info(SpeechService::new("s", "a server", "here", "/v1/audio")).problem(),
            None
        );
        assert_eq!(
            info(SpeechService::program("s", "a program", "here")).problem(),
            None
        );
        let mut routed = SpeechService::program("s", "a program", "here");
        routed.route = "/v1/audio".to_owned();
        assert!(
            info(routed)
                .problem()
                .is_some_and(|why| why.contains("route"))
        );
        for route in ["", "v1/audio"] {
            let why = info(SpeechService::new("s", "a server", "here", route)).problem();
            assert!(why.is_some_and(|why| why.contains("route")), "{route:?}");
        }
        let untitled = SpeechService::program(" ", "a program", "here");
        assert!(
            info(untitled)
                .problem()
                .is_some_and(|why| why.contains("title"))
        );
    }

    #[test]
    fn a_run_is_the_program_its_arguments_its_input_and_the_model_it_runs() {
        let run = SpeechRun::new("/p/stt", ["-m", "x"], "wav", "x");
        assert_eq!(run.program, "/p/stt");
        assert_eq!(run.args, ["-m", "x"]);
        assert_eq!((run.input.as_str(), run.model.as_str()), ("wav", "x"));
    }
}
