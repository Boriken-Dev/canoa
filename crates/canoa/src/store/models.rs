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

/// The versions of the `model_store` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

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

/// The service that runs a store's models, as far as the person's speech-to-text is concerned: it is called as a server that
/// transcribes one recording, at an address that changes and that [`ModelStore::service`] finds.
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
    /// The path below the service's address that takes a recording.
    pub route: String,
    /// Whether the recording is written to a file on this computer and its name posted, in place of the audio being sent.
    #[map(default)]
    pub local_file: bool,
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
    /// A store of rows from `catalog`, with no settings and no speech.
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
        if let Some(speech) = &self.speech
            && (speech.title.trim().is_empty() || !speech.route.starts_with('/'))
        {
            return Some(
                "its speech service has no title, or a route that does not start with /".into(),
            );
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

/// Runs `program` with `args` through `spawner` to its end, or ends it (and what it started) when `limit` is over.
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
    loop {
        if started.elapsed() >= limit {
            let _ = process.kill();
            let _ = process.wait(500);
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                &format!(
                    "{program} did not finish within {} minutes and was ended",
                    limit.as_secs().div_ceil(60)
                ),
            ));
        }
        let both = !out.done && !err.done;
        if !out.done {
            out.take(&process, false, if both { 25 } else { 100 })?;
        }
        if !err.done {
            err.take(&process, true, if both { 0 } else { 100 })?;
        }
        if out.done && err.done {
            break;
        }
    }
    let exit = loop {
        let exit = process.wait(100)?;
        if !exit.running {
            break exit;
        }
        if started.elapsed() >= limit {
            let _ = process.kill();
            let _ = process.wait(500);
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_GONE,
                &format!("{program} did not end and was killed"),
            ));
        }
    };
    Ok(Finished {
        code: exit.code,
        out: String::from_utf8_lossy(&out.bytes).into_owned(),
        err: String::from_utf8_lossy(&err.bytes).into_owned(),
    })
}

/// One of a process's two output streams, read as far as it goes.
#[derive(Default)]
struct Stream {
    bytes: Vec<u8>,
    done: bool,
}

impl Stream {
    fn take(
        &mut self,
        process: &Object<dyn SpawnedProcess>,
        error: bool,
        wait_ms: u64,
    ) -> Result<(), ProviderError> {
        let mut buffer = [0u8; 8192];
        let count = if error {
            process.read_error(&mut buffer, wait_ms)?
        } else {
            process.read(&mut buffer, wait_ms)?
        };
        if count == READ_TIMED_OUT || count == READ_LINK_DOWN {
            return Ok(());
        }
        if count <= 0 {
            self.done = true;
            return Ok(());
        }
        let count = usize::try_from(count).unwrap_or(0).min(buffer.len());
        let room = MOST_OUTPUT.saturating_sub(self.bytes.len());
        self.bytes.extend_from_slice(&buffer[..count.min(room)]);
        Ok(())
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
