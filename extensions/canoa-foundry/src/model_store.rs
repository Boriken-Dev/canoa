//! Foundry Local's models as a Store page: what its command line lists (one entry for each alias, the builds for each kind of
//! hardware inside it), what installing and removing do, the window a build is run with and the Whisper models that may be the
//! person's speech-to-text. The Store is only a front end: every list and every change is the program's own command, run through the
//! spawner the host hands this provider, and nothing here names a model. The host asks this provider through the `model_store` kind
//! and draws what it answers.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, paths and the clock are std's"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow,
    ModelSettings, ModelStore, ModelStoreInfo, ModelVariant, RowAction, Section, ServedModel,
    ServiceQuery, ServiceReport, SpeechService, SpeechUse, last_words, run,
};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Status, ToValue};
use guatiao_intake::Form;
use serde_json::{Map, Value, json};

use crate::models::{self, Listing, ModelAction};
use crate::providers::TABLE;
use crate::window;

/// How long each call is given before the program is ended: a download is the long one, and its size is the model's.
const LIST_TIME: Duration = Duration::from_secs(120);
const DOWNLOAD_TIME: Duration = Duration::from_secs(4 * 60 * 60);
const LOAD_TIME: Duration = Duration::from_secs(15 * 60);
const UNLOAD_TIME: Duration = Duration::from_secs(2 * 60);

/// What the page says when the program is not found: why there is no list, and where the program comes from.
const MISSING: &str = "Foundry Local's program (`foundry`) is not installed, and it is what lists these models and downloads them: the Store only shows its answers. Install it from the Store's Programs list and the models are listed here.";

/// What a load leaves behind, for the confirmation and the note after it.
const MEMORY: &str = "A loaded model stays in memory until Foundry Local's service is stopped (`foundry server stop`), and takes about as much as its download.";

/// Why the service cannot take a recording now.
const NO_SERVICE: &str = "Foundry Local's service is not running: start it (`foundry server start`), then load a Whisper model on the Foundry models page.";
const NO_WHISPER: &str = "Foundry Local's service lists no Whisper model: download and load one on the Foundry models page.";

/// The most of the program's words a refusal carries.
const MOST_WORDS: usize = 600;

/// What the page keeps for one model: the window a run of it is given.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModelSetting {
    /// How many tokens a prompt and its answer may take together. It cannot go above the model's own context length. Empty runs
    /// the model with its own window.
    #[schema(title = "Window")]
    #[form(unit = "tokens", placeholder = "the model's own")]
    pub window: Option<u64>,
}

fn refused(status: Status, why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(status, why.as_ref())
}

fn bad(why: impl AsRef<str>) -> ProviderError {
    refused(Status::GUATIAO_ERR_BAD_VALUE, why)
}

fn failed(why: impl AsRef<str>) -> ProviderError {
    refused(Status::GUATIAO_ERR_INTERNAL, why)
}

/// The models of the person's Foundry Local, listed and moved through its own command line.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "foundry_models", crate = ::coqui_acp_api::guatiao)]
pub struct FoundryModels;

/// Where Foundry Local keeps the models it downloads, under the person's home.
fn cache_of(home: &Path) -> PathBuf {
    home.join(".foundry").join("cache").join("models")
}

fn home_of(home: Option<&str>) -> Option<PathBuf> {
    home.filter(|home| !home.is_empty()).map(PathBuf::from)
}

fn tokens(value: Option<u64>) -> String {
    value.map_or_else(
        || "not known".to_owned(),
        |tokens| format!("{tokens} tokens"),
    )
}

/// The program's refusal in its own words: the end of what it wrote, which is where a program says why it stopped.
fn refusal(
    what: &str,
    program: &str,
    asked: (Option<ModelAction>, Option<&str>),
    done: &canoa::store::models::Finished,
) -> ProviderError {
    refused_with(what, program, asked, done.code, &done.out, &done.err)
}

/// What the program refused with: its own error and what its exit code means for what was asked (`action` on `model`, none for a
/// listing), else the end of what it wrote.
fn refused_with(
    what: &str,
    program: &str,
    asked: (Option<ModelAction>, Option<&str>),
    code: Option<i64>,
    out: &str,
    err: &str,
) -> ProviderError {
    let said = models::refusal_words(asked.0, asked.1, code, out, err)
        .unwrap_or_else(|| last_words(&format!("{out}\n{err}"), MOST_WORDS));
    let how = code.map_or_else(|| "was ended".to_owned(), |code| format!("exited {code}"));
    failed(if said.is_empty() {
        format!("{what}: {program} {how}")
    } else {
        format!("{what}: {said}")
    })
}

/// The JSON a listing holds: all of what the program wrote, or, when it wrote lines of its own first (an update notice, a
/// service that is starting), the JSON from the first line that opens one.
fn json_of(out: &str) -> Option<Value> {
    if let Ok(whole) = serde_json::from_str::<Value>(out.trim()) {
        return Some(whole);
    }
    let mut at = 0;
    for line in out.split_inclusive('\n') {
        if line.trim_start().starts_with(['{', '[']) {
            let rest = &out[at + (line.len() - line.trim_start().len())..];
            if let Some(Ok(found)) = serde_json::Deserializer::from_str(rest)
                .into_iter::<Value>()
                .next()
            {
                return Some(found);
            }
        }
        at += line.len();
    }
    None
}

fn rows_of(out: &str) -> Option<Vec<models::Row>> {
    json_of(out).map(|answer| models::rows(&answer))
}

/// A listing that exited well and said something that is not JSON, with what it said.
fn not_json(what: &str, done: &canoa::store::models::Finished) -> ProviderError {
    not_json_of(what, &done.out, &done.err)
}

fn not_json_of(what: &str, out: &str, err: &str) -> ProviderError {
    let said = last_words(&format!("{out}\n{err}"), MOST_WORDS);
    failed(if said.is_empty() {
        format!("{what}: Foundry Local exited 0 and wrote nothing")
    } else {
        format!("{what}: Foundry Local did not answer in JSON; it said: {said}")
    })
}

fn size_text(mb: u64) -> String {
    if mb >= 1024 {
        format!("{:.1} GB", mb as f64 / 1024.0)
    } else {
        format!("{mb} MB")
    }
}

fn size_of(row: &models::Row) -> String {
    row.size_mb
        .map_or_else(String::new, |mb| format!(" ({})", size_text(mb)))
}

/// How much a model takes, as one size or the range its variants span.
fn span_of(entry: &models::Entry) -> Option<String> {
    entry.sizes_mb().map(|(least, most)| {
        if least == most {
            size_text(least)
        } else {
            format!("{} to {}", size_text(least), size_text(most))
        }
    })
}

/// What `+` of a model does for the whole of it: Foundry Local downloads the build that suits this computer, or takes what is
/// downloaded out of its cache.
fn entry_actions(entry: &models::Entry) -> Vec<RowAction> {
    let id = &entry.id;
    if entry.cached() {
        return vec![
            RowAction::new(
                format!("Remove {id}"),
                format!("Foundry Local takes {id} out of its cache (`foundry cache rm`)."),
                "Remove",
                ["remove"],
            )
            .removing(),
        ];
    }
    let size = span_of(entry).map_or_else(String::new, |size| format!(" ({size})"));
    vec![RowAction::new(
        format!("Install {id}"),
        format!(
            "Foundry Local downloads {id}{size} into its cache, choosing the build that suits this computer. The download is Foundry Local's own and Canoa checks nothing of it, so it is shown as not verified. It can take long."
        ),
        "Install",
        ["download"],
    )]
}

/// What the buttons of one build do: install it, or load it, unload it and take it out of the cache.
fn variant_actions(model: &models::Model) -> Vec<RowAction> {
    let id = &model.row.id;
    if !model.cached {
        return vec![RowAction::new(
            format!("Install {id}"),
            format!(
                "Foundry Local downloads {id}{} into its cache. The download is Foundry Local's own and Canoa checks nothing of it, so it is shown as not verified. It can take long.",
                size_of(&model.row)
            ),
            "Install",
            ["download"],
        )];
    }
    let move_in = if model.loaded {
        RowAction::new(
            format!("Unload {id}"),
            "Foundry Local takes the model out of memory. The service may keep what it reserved until it is stopped.",
            "Unload",
            ["unload"],
        )
    } else {
        RowAction::new(
            format!("Load {id}"),
            format!("Foundry Local loads {id}. {MEMORY}"),
            "Load",
            ["load"],
        )
    };
    vec![
        move_in,
        RowAction::new(
            format!("Remove {id}"),
            format!("Foundry Local takes {id} out of its cache (`foundry cache rm`)."),
            "Remove",
            ["remove"],
        )
        .removing(),
    ]
}

/// The page's window section: what the model's own files say, and the form that sets the one it is run with.
fn window_section(model: &models::Model, home: Option<&Path>) -> Section {
    let own = home
        .filter(|_| model.cached)
        .and_then(|home| window::cached_folder(&cache_of(home), &model.row.id))
        .and_then(|folder| window::read(&folder).ok());
    let mut facts = Vec::new();
    let mut paragraphs = Vec::new();
    match own {
        Some(own) if own.context_length.is_some() || own.max_length.is_some() => {
            facts.push(Fact::new(
                "Most the model takes (context length)",
                tokens(own.context_length),
            ));
            facts.push(Fact::new(
                "Prompt and answer together, as cached (max length)",
                tokens(own.max_length),
            ));
        }
        _ => paragraphs.push(
            if model.cached {
                "The model's own files did not say."
            } else {
                "Not downloaded: its own window is read once it is."
            }
            .to_owned(),
        ),
    }
    paragraphs.push("A longer window lets a longer prompt through and keeps more memory for the text so far; a shorter one keeps less. It cannot go above the model's context length.".to_owned());
    paragraphs.push("It applies when an account runs this model with the sdk engine: Canoa then makes a folder of its own for the model, with links to Foundry Local's files and its own configuration, and runs the model in a models folder of its own as well, so Foundry Local's cache and its catalog file are not changed (only when Canoa's folder is on another drive than Foundry Local's cache does it use Foundry Local's folder and put its catalog file back afterwards). It takes effect the next time the account's gateway starts. The Foundry Local service engine runs the model as it is cached and does not use this setting.".to_owned());
    let mut section = Section::new("Window");
    section.facts = facts;
    section.paragraphs = paragraphs;
    section.settings = true;
    section
}

/// One build of a model: where it stands, what hardware it is for, what its buttons do and what its part of the page shows.
fn variant_of(model: &models::Model, home: Option<&Path>) -> ModelVariant {
    let row = &model.row;
    let stand = if model.loaded {
        "Loaded"
    } else if model.cached {
        "Downloaded"
    } else {
        "Not downloaded"
    };
    let device = row.device.as_deref().map(str::to_ascii_uppercase);
    let runtime = row
        .provider
        .as_deref()
        .map(|provider| provider.trim_end_matches("ExecutionProvider"));
    let label = [device.as_deref(), runtime]
        .into_iter()
        .flatten()
        .collect::<Vec<_>>()
        .join(" · ");
    let size = row.size_mb.map(size_text);
    let summary = std::iter::once(stand)
        .chain(size.as_deref())
        .collect::<Vec<_>>()
        .join(" · ");
    let whisper = format!("{} {}", row.id, row.alias.as_deref().unwrap_or_default())
        .to_ascii_lowercase()
        .contains("whisper");
    let mut variant = ModelVariant::new(
        row.id.clone(),
        if label.is_empty() {
            row.id.clone()
        } else {
            label
        },
        summary,
    );
    variant.installed = model.cached;
    variant.state = if model.loaded {
        Some("Loaded".to_owned())
    } else if model.cached {
        Some("Downloaded".to_owned())
    } else {
        None
    };
    for (label, value) in [
        ("Name", Some(row.id.as_str())),
        ("Hardware", row.device.as_deref()),
        ("Runtime", row.provider.as_deref()),
        ("Task", row.task.as_deref()),
        ("License", row.license.as_deref()),
    ] {
        if let Some(value) = value {
            variant.facts.push(Fact::new(label, value));
        }
    }
    if let Some(size) = &size {
        variant.facts.push(Fact::new("Size", size.clone()));
    }
    variant.actions = variant_actions(model);
    variant.sections = vec![window_section(model, home)];
    variant.speech = whisper.then(|| {
        let mut speech = SpeechUse::new("Dictation can send its audio to this model through the Foundry Local service on the host: the audio is written to a temporary file there, handed to the service and removed afterwards, and nothing is sent to another service. Canoa never loads a model for you, so load it on this page first.");
        speech.caution = (!model.loaded).then(|| "It is not loaded now.".to_owned());
        speech
    });
    variant
}

/// A model's row: one for each alias, where it stands, what it is, what `+` does and the builds for each kind of hardware inside it.
/// It lists no file, so the host shows it as not verified: Foundry Local fetches its own files and states no digest for the large ones.
fn row_of(entry: &models::Entry, home: Option<&Path>) -> ModelRow {
    let stand = if entry.loaded() {
        "Loaded"
    } else if entry.cached() {
        "Downloaded"
    } else {
        "Not downloaded"
    };
    let devices = entry.devices();
    let described = [
        entry.task().map(str::to_owned),
        (!devices.is_empty()).then(|| devices.join("/")),
        span_of(entry),
    ]
    .into_iter()
    .flatten()
    .collect::<Vec<_>>()
    .join(", ");
    let summary = if described.is_empty() {
        stand.to_owned()
    } else {
        format!("{stand} · {described}")
    };
    let mut shown = ModelRow::new(entry.id.clone(), summary);
    shown.tags = entry
        .task()
        .map(str::to_owned)
        .into_iter()
        .chain(devices.iter().cloned())
        .collect();
    shown.on = entry.cached();
    shown.state = if entry.loaded() {
        Some("Loaded".to_owned())
    } else if entry.cached() {
        Some("Downloaded".to_owned())
    } else {
        None
    };
    for (label, value) in [
        ("Task", entry.task().map(str::to_owned)),
        (
            "Hardware",
            (!devices.is_empty()).then(|| devices.join(", ")),
        ),
        ("License", entry.license().map(str::to_owned)),
    ] {
        if let Some(value) = value {
            shown.facts.push(Fact::new(label, value));
        }
    }
    shown
        .facts
        .push(Fact::new("Builds", entry.variants.len().to_string()));
    shown.actions = entry_actions(entry);
    shown.variants = entry
        .variants
        .iter()
        .map(|variant| variant_of(variant, home))
        .collect();
    shown
}

impl ModelStore for FoundryModels {
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
        let mut info = ModelStoreInfo::new("Foundry models", "Foundry Local", "Microsoft", MISSING);
        let mut settings = ModelSettings::asking::<ModelSetting>(TABLE)?;
        settings.help = Some(
            "The window a model is run with: how many tokens a prompt and its answer may take together.".to_owned(),
        );
        info.settings = Some(settings);
        let mut speech = SpeechService::new(
            "Foundry Local",
            "Speech to text by a Whisper model that Foundry Local runs on this computer. Load the model on the Foundry models page first: Canoa never loads one for you.",
            "The audio is written to a temporary file on this computer, handed to Foundry Local's service on a loopback address and removed afterwards: nothing is sent to another service.",
            "/v1/audio/transcriptions",
        );
        speech.license = Some("MIT (Whisper); Foundry Local is Microsoft's".to_owned());
        speech.local_file = true;
        info.speech = Some(speech);
        Ok(info)
    }

    fn list(
        &self,
        query: ModelQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelList, ProviderError> {
        let Some(program) = query.program else {
            let mut list = ModelList::default();
            list.unavailable = Some(MISSING.to_owned());
            return Ok(list);
        };
        let home = home_of(query.home.as_deref());
        let ask = |which: Listing| run(&spawner, &program, &models::list_args(which), LIST_TIME);
        let all = ask(Listing::All)?;
        if all.code != Some(0) {
            return Err(refusal(
                "the model catalog was not listed",
                &program,
                (None, None),
                &all,
            ));
        }
        let all =
            rows_of(&all.out).ok_or_else(|| not_json("the model catalog was not listed", &all))?;
        let cached = ask(Listing::Cached)?;
        if cached.code != Some(0) {
            return Err(refusal(
                "the downloaded models were not listed",
                &program,
                (None, None),
                &cached,
            ));
        }
        let cached = rows_of(&cached.out).unwrap_or_default();
        // Nothing is in memory when no service runs; a service that went away leaves its file and the program fails: also none.
        let serving = home
            .as_deref()
            .is_some_and(|home| crate::running_endpoint(home).is_some());
        let loaded = if serving {
            let loaded = ask(Listing::Loaded)?;
            if loaded.code == Some(0) {
                rows_of(&loaded.out).unwrap_or_default()
            } else {
                Vec::new()
            }
        } else {
            Vec::new()
        };
        let mut list = ModelList::default();
        list.models = models::group(models::merge(all, &cached, &loaded))
            .iter()
            .map(|entry| row_of(entry, home.as_deref()))
            .collect();
        Ok(list)
    }

    fn act(
        &self,
        request: ModelAct,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelActed, ProviderError> {
        let action = ModelAction::parse(&request.action)
            .ok_or_else(|| bad("action is download, load or unload"))?;
        let args =
            models::action_args(action, &request.model).map_err(|why| bad(why.to_string()))?;
        let program = request.program.ok_or_else(|| {
            failed("Foundry Local is not installed: install it from the Store's Programs list")
        })?;
        let limit = match action {
            ModelAction::Download => DOWNLOAD_TIME,
            ModelAction::Load => LOAD_TIME,
            ModelAction::Unload | ModelAction::Remove => UNLOAD_TIME,
        };
        let done = run(&spawner, &program, &args, limit)?;
        if done.code != Some(0) {
            return Err(refusal(
                &format!("{} {}", request.action, request.model),
                &program,
                (Some(action), Some(&request.model)),
                &done,
            ));
        }
        // What the program printed last: a download rewrites its percentage line, so its closing sentence or last progress.
        let said = models::outcome_line(&done.out);
        let mut acted = ModelActed::default();
        acted.note = match action {
            ModelAction::Load => Some(MEMORY.to_owned()),
            ModelAction::Download | ModelAction::Remove => said,
            ModelAction::Unload => None,
        };
        Ok(acted)
    }

    fn keep(
        &self,
        request: ModelKeep,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelKept, ProviderError> {
        drop(spawner);
        if !models::valid_model(&request.model) {
            return Err(bad("a model is 1 to 128 letters, digits and . _ - : + /"));
        }
        let given: Map<String, Value> = serde_json::from_str(&request.values)
            .map_err(|_| bad("the values are not a JSON object"))?;
        let mut kept = Map::new();
        let mut note = None;
        if let Some(value) = given.get("window").filter(|value| !value.is_null()) {
            let wanted = value.as_u64().filter(|tokens| *tokens > 0).ok_or_else(|| {
                bad("window is a whole number of tokens, at least 1, or empty to remove the setting")
            })?;
            let limit = home_of(request.home.as_deref())
                .and_then(|home| window::cached_folder(&cache_of(&home), &request.model))
                .and_then(|folder| window::read(&folder).ok())
                .and_then(|own| own.limit());
            let (window, said) = match limit {
                Some(limit) => window::clamp(wanted, limit).map_err(|why| bad(why.to_string()))?,
                // A model that is not downloaded has no limit to check against, so the number is kept as given.
                None => (wanted, None),
            };
            kept.insert("window".to_owned(), json!(window));
            note = said;
        }
        let mut answer = ModelKept::default();
        answer.values = Value::Object(kept).to_string();
        answer.note = note;
        Ok(answer)
    }

    fn service(
        &self,
        query: ServiceQuery,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ServiceReport, ProviderError> {
        drop(spawner);
        let mut report = ServiceReport::default();
        let Some(base) = home_of(query.home.as_deref())
            .as_deref()
            .and_then(crate::running_endpoint)
        else {
            report.unavailable = Some(NO_SERVICE.to_owned());
            return Ok(report);
        };
        if query.models {
            report.models = whisper_models(&base)
                .into_iter()
                .map(|id| ServedModel::new(id.clone(), id))
                .collect();
            if report.models.is_empty() {
                report.unavailable = Some(NO_WHISPER.to_owned());
            }
        }
        report.address = Some(base);
        Ok(report)
    }
}

/// The names of the models of the service at `base` that transcribe in the way measured: Whisper's, which take a file.
#[cfg(feature = "daemon")]
fn whisper_models(base: &str) -> Vec<String> {
    use std::io::Read as _;
    let Ok(agent) = canoa::network::Network::current().agent(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(Duration::from_secs(5)))
            .build(),
    ) else {
        return Vec::new();
    };
    let Ok(mut answer) = agent.get(format!("{base}/v1/models")).call() else {
        return Vec::new();
    };
    let mut text = String::new();
    if answer
        .body_mut()
        .as_reader()
        .take(4 << 20)
        .read_to_string(&mut text)
        .is_err()
    {
        return Vec::new();
    }
    let listed = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    listed
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .filter(|id| id.to_ascii_lowercase().contains("whisper"))
        .map(str::to_owned)
        .collect()
}

/// Without the `daemon` feature this build makes no request to the service, so it lists no model.
#[cfg(not(feature = "daemon"))]
fn whisper_models(_base: &str) -> Vec<String> {
    Vec::new()
}

#[cfg(test)]
mod tests {
    use super::*;

    const RECORDED: &str = include_str!("../tests/fixtures/model_list_recorded.json");
    const LEADING: &str = include_str!("../tests/fixtures/model_list_leading.txt");
    const NOT_JSON: &str = include_str!("../tests/fixtures/model_list_not_json.txt");

    #[test]
    fn a_listing_is_read_whole_or_after_the_lines_the_program_wrote_first() {
        let whole = rows_of(RECORDED).expect("a recorded answer");
        assert_eq!(whole.len(), 2);
        assert_eq!(whole[0].id, "deepseek-r1-distill-qwen-14b-qnn-npu:2");
        let after = rows_of(LEADING).expect("an answer after an update notice and a banner");
        assert_eq!(after, whole, "the notice lines change nothing");
        assert!(
            rows_of("[info] starting\n{\"variants\": []}").is_some_and(|rows| rows.is_empty()),
            "a bracket that opens no JSON is skipped"
        );
        assert_eq!(rows_of(NOT_JSON), None);
        assert_eq!(rows_of(""), None);
    }

    #[test]
    fn an_answer_that_is_not_json_says_what_it_was() {
        let why = not_json_of("the model catalog was not listed", "", NOT_JSON)
            .message()
            .to_owned();
        assert!(
            why.starts_with(
                "the model catalog was not listed: Foundry Local did not answer in JSON; it said: "
            ),
            "{why}"
        );
        assert!(why.contains("error: the service did not start"), "{why}");
        assert!(!why.contains('\u{1b}'), "no terminal escapes: {why}");
        assert_eq!(
            not_json_of("the model catalog was not listed", "\n", "").message(),
            "the model catalog was not listed: Foundry Local exited 0 and wrote nothing"
        );
    }

    #[test]
    fn a_refusal_keeps_the_end_of_what_the_program_wrote() {
        let banner = "Foundry Local is checking for updates\n".repeat(40);
        let why = refused_with(
            "the model catalog was not listed",
            "foundry",
            (None, None),
            Some(1),
            &banner,
            "error: the catalog is not reachable\n",
        )
        .message()
        .to_owned();
        assert!(why.contains("the catalog is not reachable"), "{why}");
        let long = format!("{}error: out of space\n", "chatter\n".repeat(200));
        let why = refused_with("a", "foundry", (None, None), Some(1), "", &long)
            .message()
            .to_owned();
        assert!(why.ends_with("error: out of space"), "{why}");
    }

    fn build(id: &str, device: &str, mb: u64, cached: bool, loaded: bool) -> models::Model {
        let rows = models::rows(&json!([{
            "variantId": id, "alias": "qwen3-14b", "type": "chat", "device": device,
            "executionProvider": "SomeExecutionProvider", "fileSizeMb": mb, "license": "mit"
        }]));
        models::merge(rows, &[], &[])
            .into_iter()
            .map(|mut model| {
                model.cached = cached;
                model.loaded = loaded;
                model
            })
            .next()
            .expect("a model")
    }

    fn entry(variants: Vec<models::Model>) -> models::Entry {
        models::group(variants)
            .into_iter()
            .next()
            .expect("one entry")
    }

    #[test]
    fn what_the_buttons_do_follows_where_the_model_and_each_build_stand() {
        let one = |cached, loaded| {
            entry(vec![build(
                "qwen3-14b-generic-gpu:2",
                "Gpu",
                10137,
                cached,
                loaded,
            )])
        };
        let words = |actions: &[RowAction]| -> Vec<(Vec<String>, bool)> {
            actions
                .iter()
                .map(|action| (action.steps.clone(), action.removes))
                .collect()
        };
        let install = entry_actions(&one(false, false));
        assert_eq!(words(&install), [(vec!["download".to_owned()], false)]);
        assert!(
            install[0].confirm.contains("(9.9 GB)"),
            "{}",
            install[0].confirm
        );
        assert!(
            install[0].confirm.contains("checks nothing"),
            "{}",
            install[0].confirm
        );
        assert_eq!(
            words(&entry_actions(&one(true, false))),
            [(vec!["remove".to_owned()], true)],
            "a downloaded model offers its removal, never an install"
        );
        let build_of = |cached, loaded| {
            let model = build("qwen3-14b-generic-gpu:2", "Gpu", 10137, cached, loaded);
            variant_actions(&model)
        };
        assert_eq!(
            words(&build_of(false, false)),
            [(vec!["download".to_owned()], false)]
        );
        assert_eq!(
            words(&build_of(true, false)),
            [
                (vec!["load".to_owned()], false),
                (vec!["remove".to_owned()], true)
            ]
        );
        assert_eq!(
            words(&build_of(true, true)),
            [
                (vec!["unload".to_owned()], false),
                (vec!["remove".to_owned()], true)
            ]
        );
    }

    #[test]
    fn a_row_is_one_model_with_its_builds_inside_and_lists_no_file() {
        let both = entry(vec![
            build("qwen3-14b-generic-gpu:2", "Gpu", 10137, true, false),
            build("qwen3-14b-generic-cpu:3", "Cpu", 5000, false, false),
        ]);
        let row = row_of(&both, None);
        assert_eq!(
            row.id, "qwen3-14b",
            "the row is named by the alias the builds share"
        );
        assert_eq!(row.summary, "Downloaded · chat, cpu/gpu, 4.9 GB to 9.9 GB");
        assert_eq!(row.tags, ["chat", "cpu", "gpu"]);
        assert!(row.on && row.state.as_deref() == Some("Downloaded"));
        assert!(
            row.files.is_empty(),
            "no file of its own, so the host shows it as not verified"
        );
        let ids: Vec<_> = row
            .variants
            .iter()
            .map(|variant| variant.id.as_str())
            .collect();
        assert_eq!(ids, ["qwen3-14b-generic-cpu:3", "qwen3-14b-generic-gpu:2"]);
        assert_eq!(row.variants[1].label, "GPU · Some");
        assert!(row.variants[1].installed && !row.variants[0].installed);
        assert!(row.variants.iter().all(|variant| variant.speech.is_none()));
        assert_eq!(
            row.variants[1]
                .sections
                .iter()
                .map(|section| section.settings)
                .collect::<Vec<_>>(),
            [true],
            "each build keeps its own window"
        );
    }

    #[test]
    fn a_whisper_build_may_be_used_for_speech() {
        let whisper = |loaded| {
            let rows = models::rows(
                &json!([{"variantId": "openai-whisper-tiny-generic-cpu:1", "alias": "whisper-tiny"}]),
            );
            let mut model = models::merge(rows, &[], &[]).remove(0);
            model.cached = true;
            model.loaded = loaded;
            variant_of(&model, None)
        };
        assert!(
            whisper(false)
                .speech
                .is_some_and(|speech| speech.caution.is_some())
        );
        assert!(
            whisper(true)
                .speech
                .is_some_and(|speech| speech.caution.is_none())
        );
    }

    #[test]
    fn it_describes_a_form_that_fits_its_schema_and_a_table_that_is_its_own() {
        let info = ModelStoreInfo::new("t", "c", "p", "m");
        assert_eq!(info.problem(), None);
        let described = FoundryModels.describe().expect("described");
        assert_eq!(described.problem(), None);
        assert_eq!(
            described.settings.as_ref().map(|each| each.table.as_str()),
            Some(TABLE)
        );
    }
}

#[cfg(test)]
#[path = "model_store_cli_tests.rs"]
mod cli_tests;
