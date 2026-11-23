//! Foundry Local's models as a Store page: what its command line lists, what `+` does for each, the window a model is run with and
//! the Whisper models that may be the person's speech-to-text. The host knows none of it: it asks this provider through the
//! `model_store` kind and draws what it answers.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, paths and the clock are std's"
)]

use std::path::{Path, PathBuf};
use std::time::Duration;

use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow,
    ModelSettings, ModelStore, ModelStoreInfo, RowAction, Section, ServedModel, ServiceQuery,
    ServiceReport, SpeechService, SpeechUse, run,
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

/// What the page says when the program is not found.
const MISSING: &str =
    "Not installed: install it from the Store's Programs list, and its models are listed here.";

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

/// The program's refusal in its own words.
fn refusal(what: &str, program: &str, done: &canoa::store::models::Finished) -> ProviderError {
    let said = models::plain(&format!("{}\n{}", done.err, done.out), MOST_WORDS);
    let how = done
        .code
        .map_or_else(|| "was ended".to_owned(), |code| format!("exited {code}"));
    failed(if said.is_empty() {
        format!("{what}: {program} {how}")
    } else {
        format!("{what}: {said}")
    })
}

fn rows_of(out: &str) -> Option<Vec<models::Row>> {
    serde_json::from_str::<Value>(out.trim())
        .ok()
        .map(|answer| models::rows(&answer))
}

fn size_of(row: &models::Row) -> String {
    match row.size_mb {
        Some(mb) if mb >= 1024 => format!(" ({:.1} GB)", mb as f64 / 1024.0),
        Some(mb) => format!(" ({mb} MB)"),
        None => String::new(),
    }
}

/// What a click on a model's `+` does, as the person confirms it.
fn action_of(model: &models::Model) -> RowAction {
    let id = &model.row.id;
    let (title, confirm, label, steps) = if model.loaded {
        (
            format!("Unload {id}"),
            "Foundry Local takes the model out of memory. The service may keep what it reserved until it is stopped.".to_owned(),
            "Unload",
            vec!["unload"],
        )
    } else if model.cached {
        (
            format!("Load {id}"),
            format!("Foundry Local loads {id}. {MEMORY}"),
            "Load",
            vec!["load"],
        )
    } else {
        (
            format!("Download and load {id}"),
            format!(
                "Foundry Local downloads {id}{} into its cache, which can take long, and then loads it. {MEMORY}",
                size_of(&model.row)
            ),
            "Download and load",
            vec!["download", "load"],
        )
    };
    RowAction::new(title, confirm, label, steps)
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

/// A model's row: where it stands, what it is, what `+` does and what its page shows.
fn row_of(model: &models::Model, home: Option<&Path>) -> ModelRow {
    let row = &model.row;
    let stand = if model.loaded {
        "Loaded"
    } else if model.cached {
        "Downloaded"
    } else {
        "Not downloaded"
    };
    let described = models::describe(row);
    let summary = if described.is_empty() {
        stand.to_owned()
    } else {
        format!("{stand} · {described}")
    };
    let whisper = format!("{} {}", row.id, row.alias.as_deref().unwrap_or_default())
        .to_ascii_lowercase()
        .contains("whisper");
    let mut facts = Vec::new();
    for (label, value) in [
        ("Alias", row.alias.as_deref()),
        ("Task", row.task.as_deref()),
        ("Device", row.device.as_deref()),
    ] {
        if let Some(value) = value {
            facts.push(Fact::new(label, value));
        }
    }
    let mut shown = ModelRow::new(row.id.clone(), summary);
    shown.by.clone_from(&row.alias);
    shown.tags = [row.task.clone(), row.device.clone()]
        .into_iter()
        .flatten()
        .collect();
    shown.on = model.loaded;
    shown.state = if model.loaded {
        Some("Loaded".to_owned())
    } else if model.cached {
        Some("Downloaded".to_owned())
    } else {
        None
    };
    shown.facts = facts;
    shown.actions = vec![action_of(model)];
    shown.sections = vec![window_section(model, home)];
    shown.speech = whisper.then(|| {
        let mut speech = SpeechUse::new("Dictation can send its audio to this model through the Foundry Local service on the host: the audio is written to a temporary file there, handed to the service and removed afterwards, and nothing is sent to another service. Canoa never loads a model for you, so load it with + on the list first.");
        speech.caution = (!model.loaded).then(|| "It is not loaded now.".to_owned());
        speech
    });
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
            return Err(refusal("the model catalog was not listed", &program, &all));
        }
        let all = rows_of(&all.out).ok_or_else(|| {
            failed("the model catalog was not listed: Foundry Local did not answer in JSON")
        })?;
        let cached = ask(Listing::Cached)?;
        if cached.code != Some(0) {
            return Err(refusal(
                "the downloaded models were not listed",
                &program,
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
        list.models = models::merge(all, &cached, &loaded)
            .iter()
            .map(|model| row_of(model, home.as_deref()))
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
            ModelAction::Unload => UNLOAD_TIME,
        };
        let done = run(&spawner, &program, &args, limit)?;
        if done.code != Some(0) {
            return Err(refusal(
                &format!("{} {}", request.action, request.model),
                &program,
                &done,
            ));
        }
        let mut acted = ModelActed::default();
        acted.note = match action {
            ModelAction::Load => Some(MEMORY.to_owned()),
            ModelAction::Download => Some("The model is in Foundry Local's cache.".to_owned()),
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

    fn model(id: &str, cached: bool, loaded: bool) -> models::Model {
        let rows = models::rows(
            &json!([{"id": id, "alias": "qwen3-14b", "task": "chat", "device": "gpu", "fileSizeMb": 10137}]),
        );
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

    #[test]
    fn what_plus_does_follows_where_the_model_stands() {
        let steps =
            |cached, loaded| action_of(&model("qwen3-14b-generic-gpu:2", cached, loaded)).steps;
        assert_eq!(steps(false, false), ["download", "load"]);
        assert_eq!(steps(true, false), ["load"]);
        assert_eq!(steps(true, true), ["unload"]);
        let first = action_of(&model("qwen3-14b-generic-gpu:2", false, false));
        assert!(first.confirm.contains("(9.9 GB)"), "{}", first.confirm);
    }

    #[test]
    fn a_row_says_where_the_model_stands_and_a_whisper_model_may_be_used_for_speech() {
        let row = row_of(&model("qwen3-14b-generic-gpu:2", true, false), None);
        assert_eq!(row.summary, "Downloaded · chat, gpu, 9.9 GB");
        assert_eq!(row.by.as_deref(), Some("qwen3-14b"));
        assert_eq!(row.tags, ["chat", "gpu"]);
        assert!(row.speech.is_none());
        let speech = row_of(
            &model("openai-whisper-tiny-generic-cpu:1", true, false),
            None,
        );
        assert!(
            speech
                .speech
                .as_ref()
                .is_some_and(|speech| speech.caution.is_some())
        );
        let loaded = row_of(
            &model("openai-whisper-tiny-generic-cpu:1", true, true),
            None,
        );
        assert!(loaded.on && loaded.speech.is_some_and(|speech| speech.caution.is_none()));
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
