//! `speech/offers` and `speech/enable`: the speech-to-text providers Canoa offers out of the box, and the one that is made from a
//! program the host's Store installed. An offer is not a provider until the person adds it: a local one is installed through the
//! Store (`store/apply`) and then enabled here; a named service is added through `speech/configure` with the settings the offer
//! carries, and a key of the person's.
//!
//! The same file says which models a provider's service has, for a provider whose address names a service Canoa knows, and
//! applies the model a recording asks for in place of the one the provider's table names.
//!
//! Two offers make tables that are not complete in canoa.cfg, and [`Lend`] completes them each time a provider is built:
//! an OpenAI API key of an account the person has in Canoa (`account`, the key is read from the account's store when a
//! recording opens and is never written to canoa.cfg) and the Foundry Local service on this computer (`service`, whose address
//! changes with each start and is read from the service's own file).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::{Path, PathBuf};

use serde_json::{Map, Value, json};

use super::accounts::Accounts as HostAccounts;
use super::files::Files;
use super::{Caller, Grant, NOT_FOUND, REFUSED, Stores, allow, bad, text_param};
use crate::config::{AccountConfig, Config, Layers};
use crate::store::whisper::{MODELS, held_model, model_path};

type Outcome = Result<Value, (i32, String)>;

/// The table of canoa.cfg the providers are in.
const SECTION: &str = "speech";

/// The Store program whisper.cpp is kept as.
const WHISPER_STORE: &str = "whisper_store";

/// The address OpenAI's speech-to-text is called at.
const OPENAI_URL: &str = "https://api.openai.com/v1/audio/transcriptions";

/// The models OpenAI's transcriptions take, the recommended one first. The older three are announced to end on 2027-02-26.
const OPENAI_MODELS: [(&str, &str); 4] = [
    ("gpt-transcribe", "GPT Transcribe (recommended)"),
    (
        "gpt-4o-mini-transcribe",
        "GPT-4o mini Transcribe (ends 2027-02-26)",
    ),
    ("gpt-4o-transcribe", "GPT-4o Transcribe (ends 2027-02-26)"),
    ("whisper-1", "Whisper (ends 2027-02-26)"),
];

/// The key a provider's table keeps the offer it was made from under.
pub(super) const OFFER_KEY: &str = "offer";

/// The key of a table that names the account whose OpenAI API key the provider uses.
pub(super) const ACCOUNT_KEY: &str = "account";

/// The key of a table that names a service on this computer the provider calls, and the one that overrides its address.
pub(super) const SERVICE_KEY: &str = "service";
pub(super) const ADDRESS_KEY: &str = "service_address";

/// Every key of a provider's table that is the host's and not a setting of its kind.
pub(super) const HOST_KEYS: [&str; 4] = [OFFER_KEY, ACCOUNT_KEY, SERVICE_KEY, ADDRESS_KEY];

/// What a provider that calls Foundry Local says when its service is not up.
const NO_SERVICE: &str =
    "no Foundry Local service is running: start it (`foundry server start`) and load the model";

/// The value of [`SERVICE_KEY`] for Foundry Local.
const FOUNDRY_SERVICE: &str = "foundry_local";

/// The offer made of one of the person's accounts is `openai_account:NAME`.
const ACCOUNT_OFFER: &str = "openai_account";

/// The kind of account Codex's sign-in makes of an OpenAI API key, and the credential that holds the key.
const OPENAI_KEY_KIND: &str = "codex-api-key";
const OPENAI_KEY_FIELD: &str = "api_key";

/// The setting of that kind that moves its requests to another address.
const OPENAI_BASE_SETTING: &str = "openai_base_url";

/// What an OpenAI key's service is called at, without the path.
const OPENAI_BASE: &str = "https://api.openai.com/v1";

/// The route Foundry Local transcribes at, below the service's address.
const FOUNDRY_ROUTE: &str = "/v1/audio/transcriptions";

/// The most a model's name may be.
const MAX_MODEL: usize = 100;

/// `speech/offers` and `speech/enable`.
pub(super) fn call(
    caller: &Caller,
    files: &Files,
    stores: Result<&Stores, (i32, String)>,
    lend: &Lend<'_>,
    method: &str,
    params: &Value,
) -> Outcome {
    match method {
        "speech/offers" => allow(caller, Grant::Voice).and_then(|()| offers(files, stores, lend)),
        "speech/enable" => {
            allow(caller, Grant::VoiceSetup).and_then(|()| enable(files, stores, lend, params))
        }
        _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
    }
}

fn model_rows(models: &[(&str, &str)]) -> Vec<Value> {
    models
        .iter()
        .map(|(id, label)| json!({"id": id, "label": label}))
        .collect()
}

/// The models a provider's service has, from the address in its settings; none for an address Canoa knows nothing of.
pub(super) fn models_of(settings: &Value) -> Vec<Value> {
    match settings.get("url").and_then(Value::as_str) {
        Some(url) if url.starts_with("https://api.openai.com/") => model_rows(&OPENAI_MODELS),
        _ => Vec::new(),
    }
}

/// `table` with the model a recording asks for, when it asks for one and the provider takes a model setting.
pub(super) fn with_model(table: &Value, params: &Value) -> Result<Value, (i32, String)> {
    let Some(model) = text_param(params, "model")?.filter(|model| !model.is_empty()) else {
        return Ok(table.clone());
    };
    if model.len() > 100 || model.chars().any(char::is_control) {
        return Err(bad("model is a model's name"));
    }
    let mut changed = table.clone();
    if table.get("provider").and_then(Value::as_str) == Some("speech_endpoint")
        && let Some(map) = changed.as_object_mut()
    {
        map.insert("model".to_owned(), json!(model));
    }
    Ok(changed)
}

/// What a provider's table is completed from: the accounts the host keeps, and the home Foundry Local's file is in.
pub(super) struct Lend<'a> {
    /// The canoa.cfg files accounts are listed in, and the home their credentials rest in.
    accounts: Option<(&'a Layers, &'a Path)>,
    foundry_home: Option<PathBuf>,
}

impl<'a> Lend<'a> {
    /// What this host has: `accounts` is the host's accounts, when it keeps any, and the service is looked for in the person's home.
    pub(super) fn new(accounts: Option<&'a HostAccounts>) -> Self {
        Self {
            accounts: accounts.map(|each| (each.layers(), each.home_dir())),
            foundry_home: std::env::home_dir(),
        }
    }

    /// As [`Lend::new`], with the accounts from `layers` and `home`, and Foundry Local's file looked for in `foundry_home`.
    #[cfg(test)]
    pub(super) fn from(
        layers: &'a Layers,
        home: &'a Path,
        foundry_home: Option<PathBuf>,
    ) -> Lend<'a> {
        Lend {
            accounts: Some((layers, home)),
            foundry_home,
        }
    }

    /// The accounts canoa.cfg lists now.
    fn config(&self) -> Result<Config, String> {
        let (layers, _) = self.accounts.ok_or("this host keeps no accounts")?;
        Config::load_layers(layers).map_err(|error| error.to_string())
    }

    /// The account `name`, when it is one whose key may call OpenAI's transcriptions: the reason when it is not.
    fn usable(&self, config: &Config, name: &str) -> Result<(), String> {
        let account = config
            .accounts
            .get(name)
            .ok_or_else(|| format!("the account {name} is gone: add the provider again"))?;
        openai_key(account)
    }

    /// The API key of the account `name`, from the store its credentials rest in.
    fn key_of(&self, name: &str) -> Result<String, String> {
        let config = self.config()?;
        self.usable(&config, name)?;
        let (_, home) = self.accounts.ok_or("this host keeps no accounts")?;
        let account = &config.accounts[name];
        let store = crate::accounts::Accounts::open(&config, home).map_err(|error| {
            tracing::warn!("the account store could not be opened for speech: {error}");
            "the host's account store could not be used".to_owned()
        })?;
        let record = store
            .load(&account.id)
            .map_err(|error| {
                tracing::warn!("an account could not be read for speech: {error}");
                "the host's account store could not be used".to_owned()
            })?
            .ok_or_else(|| format!("the account {name} holds no key: sign in to it again"))?;
        record
            .credentials
            .get(OPENAI_KEY_FIELD)
            .filter(|key| !key.is_empty())
            .cloned()
            .ok_or_else(|| format!("the account {name} holds no key: sign in to it again"))
    }

    /// The address of the Foundry Local service a table calls: the one it names, else the running service's.
    fn foundry_base(&self, table: &Value) -> Option<String> {
        table
            .get(ADDRESS_KEY)
            .and_then(Value::as_str)
            .map(|address| address.trim_end_matches('/').to_owned())
            .or_else(|| {
                self.foundry_home
                    .as_deref()
                    .and_then(canoa_foundry::running_endpoint)
            })
    }

    /// Why a provider cannot take a recording now for want of a service, when it cannot.
    pub(super) fn missing(&self, table: &Value) -> Option<String> {
        let calls_foundry = table.get(SERVICE_KEY).and_then(Value::as_str) == Some(FOUNDRY_SERVICE);
        (calls_foundry && self.foundry_base(table).is_none()).then(|| NO_SERVICE.to_owned())
    }

    /// `table` as the settings of its kind: the host's own keys taken out, the address of the service it calls found and, with
    /// `with_key`, the key of the account it names read. Without it the account is only checked, so listing providers reads no
    /// credential.
    pub(super) fn settings(&self, table: &Value, with_key: bool) -> Result<Value, String> {
        let mut settings = table.clone();
        let map = settings
            .as_object_mut()
            .ok_or("it is not a table of settings")?;
        map.remove("provider");
        for key in HOST_KEYS {
            map.remove(key);
        }
        if let Some(account) = table.get(ACCOUNT_KEY).and_then(Value::as_str) {
            if with_key {
                map.insert("api_key".to_owned(), Value::String(self.key_of(account)?));
            } else {
                self.usable(&self.config()?, account)?;
            }
        }
        match table.get(SERVICE_KEY).and_then(Value::as_str) {
            None => {}
            Some(FOUNDRY_SERVICE) => {
                // Saving or listing a provider needs no service: only a recording does, and the listing says so with `problem`.
                let base = match self.foundry_base(table) {
                    Some(base) => base,
                    None if with_key => return Err(NO_SERVICE.to_owned()),
                    None => "http://127.0.0.1".to_owned(),
                };
                map.insert(
                    "url".to_owned(),
                    Value::String(format!("{base}{FOUNDRY_ROUTE}")),
                );
                map.insert("request".to_owned(), Value::String("local_file".to_owned()));
            }
            Some(other) => return Err(format!("this host has no service {other}")),
        }
        Ok(settings)
    }
}

/// Whether the account `account` holds an OpenAI API key that OpenAI's service takes; the reason when it does not. A ChatGPT
/// sign-in is no API key, and a key whose requests go to another address is not sent to OpenAI's.
fn openai_key(account: &AccountConfig) -> Result<(), String> {
    match account.kind.as_str() {
        OPENAI_KEY_KIND => {
            let moved = account
                .settings
                .as_ref()
                .and_then(|settings| settings.as_map().get(OPENAI_BASE_SETTING))
                .and_then(Value::as_str)
                .map(|address| address.trim().trim_end_matches('/'))
                .filter(|address| !address.is_empty() && *address != OPENAI_BASE);
            match moved {
                Some(address) => Err(format!(
                    "its key is for {address}, not for api.openai.com, and Canoa has not measured that address's speech-to-text, so it is not offered"
                )),
                None => Ok(()),
            }
        }
        "codex-chatgpt" | "codex-access-token" => Err(
            "a ChatGPT sign-in is not an OpenAI API key, and Canoa has no measured way to send audio to OpenAI with it, so it is not offered: add an API key as an account to use OpenAI's speech-to-text".to_owned(),
        ),
        other => Err(format!("an account of kind {other} is not an OpenAI API key")),
    }
}

/// The offers made of the person's accounts, one for each account that signs in to OpenAI through Codex.
fn account_offers(files: &Files, lend: &Lend<'_>) -> Result<Vec<Value>, (i32, String)> {
    let Ok(config) = lend.config() else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    for (name, account) in &config.accounts {
        if account.harness != "codex" {
            continue;
        }
        let id = format!("{ACCOUNT_OFFER}:{name}");
        let unavailable = openai_key(account).err();
        rows.push(json!({
            "id": id,
            "title": format!("OpenAI, account {name}"),
            "summary": format!("Speech to text by OpenAI's service, under the API key of your account {name}."),
            "local": false,
            "where": format!("The audio goes over the internet to api.openai.com, under the API key of your account {name} in Canoa, and OpenAI's terms say what it does with it. Canoa reads the key when you dictate and never shows or copies it."),
            "account": name,
            "models": model_rows(&OPENAI_MODELS),
            "default_model": OPENAI_MODELS[0].0,
            "unavailable": unavailable,
            "provider": enabled(files, &id)?,
        }));
    }
    Ok(rows)
}

/// The names of the models of a Foundry Local service that transcribe in the way measured: Whisper's, which take a file.
fn speech_models(base: &str) -> Vec<Value> {
    let Ok(agent) = canoa::network::Network::current().agent(
        ureq::Agent::config_builder()
            .http_status_as_error(false)
            .max_redirects(0)
            .timeout_global(Some(core::time::Duration::from_secs(5)))
            .build(),
    ) else {
        return Vec::new();
    };
    let Ok(mut answer) = agent.get(format!("{base}/v1/models")).call() else {
        return Vec::new();
    };
    let Ok(text) = answer.body_mut().read_to_string() else {
        return Vec::new();
    };
    let listed = serde_json::from_str::<Value>(&text).unwrap_or(Value::Null);
    listed
        .get("data")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter_map(|model| model.get("id").and_then(Value::as_str))
        .filter(|id| id.to_ascii_lowercase().contains("whisper"))
        .map(|id| json!({"id": id, "label": id}))
        .collect()
}

/// The offer of Foundry Local on this computer: ready when its service is running and lists a Whisper model, else saying what
/// is missing.
fn foundry_offer(files: &Files, lend: &Lend<'_>) -> Result<Value, (i32, String)> {
    let base = lend.foundry_base(&json!({}));
    let models = base.as_deref().map(speech_models).unwrap_or_default();
    let unavailable = if base.is_none() {
        Some(
            "Foundry Local's service is not running: start it (`foundry server start`), then load a Whisper model on the Foundry models page.",
        )
    } else if models.is_empty() {
        Some(
            "Foundry Local's service lists no Whisper model: download and load one on the Foundry models page.",
        )
    } else {
        None
    };
    Ok(json!({
        "id": "foundry",
        "title": "Foundry Local",
        "summary": "Speech to text by a Whisper model that Foundry Local runs on this computer. Load the model on the Foundry models page first: Canoa never loads one for you.",
        "license": "MIT (Whisper); Foundry Local is Microsoft's",
        "local": true,
        "where": "The audio is written to a temporary file on this computer, handed to Foundry Local's service on a loopback address and removed afterwards: nothing is sent to another service.",
        "service": FOUNDRY_SERVICE,
        "models": models,
        "default_model": models.first().and_then(|model| model["id"].as_str()),
        "installed": unavailable.is_none(),
        "unavailable": unavailable,
        "provider": enabled(files, "foundry")?,
    }))
}

/// The name of the provider canoa.cfg has that was made from the offer `id`, if any.
fn enabled(files: &Files, id: &str) -> Result<Option<String>, (i32, String)> {
    Ok(match files.hidden_section(SECTION)? {
        Some(Value::Object(tables)) => tables
            .iter()
            .find(|(_, table)| table.get(OFFER_KEY).and_then(Value::as_str) == Some(id))
            .map(|(name, _)| name.clone()),
        _ => None,
    })
}

fn offers(files: &Files, stores: Result<&Stores, (i32, String)>, lend: &Lend<'_>) -> Outcome {
    let installed = stores
        .and_then(|stores| stores.managed_version(WHISPER_STORE))
        .ok()
        .flatten();
    let models: Vec<Value> = MODELS
        .iter()
        .map(|model| json!({"id": model.id, "label": model.label, "bytes": model.bytes}))
        .collect();
    let mut offers = vec![
        json!({
            "id": "whisper",
            "title": "whisper.cpp",
            "summary": "Speech to text on this host's own processor, in every language the model knows. The program and the model are downloaded once, checked against their publishers' checksums.",
            "license": "MIT",
            "local": true,
            "where": "The audio goes to your host and is turned into text there by a program Canoa installed: nothing is sent to another service.",
            "store": WHISPER_STORE,
            "models": models,
            "default_model": MODELS[0].id,
            "installed": installed.is_some(),
            "version": installed,
            "provider": enabled(files, "whisper")?,
        }),
        foundry_offer(files, lend)?,
        json!({
            "id": "openai",
            "title": "OpenAI",
            "summary": "Speech to text by OpenAI's service, under your own API key.",
            "local": false,
            "where": "The audio goes over the internet to api.openai.com, under the key you enter here, and OpenAI's terms say what it does with it.",
            "kind": "speech_endpoint",
            "settings": {"url": OPENAI_URL, "model": OPENAI_MODELS[0].0},
            "models": model_rows(&OPENAI_MODELS),
            "default_model": OPENAI_MODELS[0].0,
            "provider": enabled(files, "openai")?,
        }),
    ];
    offers.extend(account_offers(files, lend)?);
    Ok(json!({"offers": offers}))
}

/// The arguments that make `whisper-cli` read a WAV stream on its standard input and print the text alone: no timestamps and none
/// of its log, the language the person said or its own detection, and their words as the prompt. `-of` names a file only so the
/// program prints to its standard output, which it does not for a stream otherwise.
fn whisper_args(model: &str) -> Vec<String> {
    [
        "-m",
        model,
        "-f",
        "-",
        "-of",
        "canoa",
        "-nt",
        "-np",
        "-l",
        "{language_code|auto}",
        "--prompt",
        "{vocabulary}",
    ]
    .map(str::to_owned)
    .into()
}

/// A provider's table and the id of the model it was made for.
type Made = (Map<String, Value>, &'static str);

/// The table of a whisper.cpp provider for the copy whose program is `program`, and the model that copy holds. The program is in
/// `bin` beside the version's `models` folder.
fn whisper_table(program: &str) -> Result<Made, (i32, String)> {
    let version = Path::new(program)
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            (
                REFUSED,
                "the installed copy has an unexpected layout".to_owned(),
            )
        })?;
    let held = held_model(version).ok_or_else(|| {
        (
            REFUSED,
            "the installed copy holds no model: install whisper.cpp again".to_owned(),
        )
    })?;
    let model = version.join(model_path(held));
    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("speech_command"));
    table.insert("program".to_owned(), json!(program));
    table.insert(
        "args".to_owned(),
        json!(whisper_args(&model.to_string_lossy())),
    );
    table.insert("input".to_owned(), json!("wav"));
    table.insert(OFFER_KEY.to_owned(), json!("whisper"));
    Ok((table, held.id))
}

/// Writes the provider `name` as `table`, in place of one of that name.
fn write_table(files: &Files, name: &str, table: Map<String, Value>) -> Result<(), (i32, String)> {
    files.hidden_update(SECTION, |current| {
        let mut map = current.as_object().cloned().unwrap_or_default();
        map.insert(name.to_owned(), Value::Object(table));
        Ok(Value::Object(map))
    })
}

/// The model a request names, or `fallback`; refused when it is not a model's name.
fn model_of(params: &Value, fallback: Option<&str>) -> Result<String, (i32, String)> {
    let asked = text_param(params, "model")?.filter(|model| !model.is_empty());
    let model = asked.or(fallback).ok_or_else(|| bad("model is needed"))?;
    if model.len() > MAX_MODEL || model.chars().any(char::is_control) {
        return Err(bad("model is a model's name"));
    }
    Ok(model.to_owned())
}

/// The name a request gives the provider, or `fallback`.
fn name_of<'a>(params: &'a Value, fallback: &'a str) -> Result<&'a str, (i32, String)> {
    let name = text_param(params, "name")?.unwrap_or(fallback);
    if super::speech::valid_name(name) {
        Ok(name)
    } else {
        Err(bad(
            "a name is letters, digits, - and _, at most 40 characters",
        ))
    }
}

/// `speech/enable` `{offer, name?, model?}`: makes (or remakes) the provider of an offer that needs no form. `whisper` is made
/// from the copy the Store has installed, whose model is the one the copy holds (another is installed through the Store, and this
/// is called again). `foundry` takes the Whisper model `model` of Foundry Local's service, which the person loads, and
/// `openai_account:NAME` the API key of the account `NAME`.
fn enable(
    files: &Files,
    stores: Result<&Stores, (i32, String)>,
    lend: &Lend<'_>,
    params: &Value,
) -> Outcome {
    let offer = text_param(params, "offer")?.ok_or_else(|| bad("offer is needed"))?;
    if offer == "foundry" {
        return enable_foundry(files, params);
    }
    if let Some(account) = offer
        .strip_prefix(ACCOUNT_OFFER)
        .and_then(|rest| rest.strip_prefix(':'))
    {
        return enable_account(files, lend, offer, account, params);
    }
    if offer != "whisper" {
        return Err((
            NOT_FOUND,
            format!("{offer} is added with speech/configure, not enabled"),
        ));
    }
    let name = name_of(params, "whisper")?;
    let program = stores?.managed_launch(WHISPER_STORE)?.ok_or_else(|| {
        (
            REFUSED,
            "whisper.cpp is not installed: install it in the Store first".to_owned(),
        )
    })?;
    let (table, model) = whisper_table(&program)?;
    write_table(files, name, table)?;
    Ok(json!({"name": name, "provider": "speech_command", "model": model, "saved": true}))
}

/// The provider that calls the Whisper model `model` of Foundry Local's service. The model is not loaded, checked or looked for
/// here: loading one is the person's choice, and a provider whose service is not running says so when it is listed.
fn enable_foundry(files: &Files, params: &Value) -> Outcome {
    let name = name_of(params, "foundry")?;
    let model = model_of(params, None)?;
    if !canoa_foundry::models::valid_model(&model) {
        return Err(bad("model is a Foundry Local model's name"));
    }
    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("speech_endpoint"));
    table.insert("model".to_owned(), json!(model));
    table.insert(SERVICE_KEY.to_owned(), json!(FOUNDRY_SERVICE));
    table.insert(OFFER_KEY.to_owned(), json!("foundry"));
    write_table(files, name, table)?;
    Ok(json!({"name": name, "provider": "speech_endpoint", "model": model, "saved": true}))
}

/// The provider that sends audio to OpenAI under the API key of the account `account`. The key is not copied: the table names the
/// account, and the key is read from its store whenever a recording opens.
fn enable_account(
    files: &Files,
    lend: &Lend<'_>,
    offer: &str,
    account: &str,
    params: &Value,
) -> Outcome {
    let fallback: String = format!("openai-{account}").chars().take(40).collect();
    let name = name_of(params, &fallback)?;
    let model = model_of(params, Some(OPENAI_MODELS[0].0))?;
    // The key is read once so a missing one is said now and not at the first dictation, and dropped at once.
    drop(
        lend.key_of(account)
            .map_err(|why| (REFUSED, format!("{account}: {why}")))?,
    );
    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("speech_endpoint"));
    table.insert("url".to_owned(), json!(OPENAI_URL));
    table.insert("model".to_owned(), json!(model));
    table.insert(ACCOUNT_KEY.to_owned(), json!(account));
    table.insert(OFFER_KEY.to_owned(), json!(offer));
    write_table(files, name, table)?;
    Ok(
        json!({"name": name, "provider": "speech_endpoint", "model": model, "account": account, "saved": true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::store::whisper::program_file;

    fn copy(model: Option<&str>) -> std::path::PathBuf {
        let root = std::env::temp_dir().join(format!(
            "canoa-offers-{}-{}",
            std::process::id(),
            model.unwrap_or("none")
        ));
        let _ = std::fs::remove_dir_all(&root);
        std::fs::create_dir_all(root.join("bin")).expect("bin");
        std::fs::create_dir_all(root.join("models")).expect("models");
        std::fs::write(
            root.join("bin").join(program_file(std::env::consts::OS)),
            b"p",
        )
        .expect("program");
        if let Some(file) = model {
            std::fs::write(root.join("models").join(file), b"m").expect("model");
        }
        root
    }

    #[test]
    fn the_provider_made_of_a_copy_reads_a_wav_stream_and_names_the_model_the_copy_holds() {
        let root = copy(Some("ggml-small.en.bin"));
        let program = root.join("bin").join(program_file(std::env::consts::OS));
        let (table, model) = whisper_table(&program.to_string_lossy()).expect("a table");
        assert_eq!(model, "small.en");
        assert_eq!(table["provider"], "speech_command");
        assert_eq!(table["input"], "wav");
        assert_eq!(table[OFFER_KEY], "whisper");
        let args: Vec<&str> = table["args"]
            .as_array()
            .expect("args")
            .iter()
            .filter_map(Value::as_str)
            .collect();
        let held = root
            .join("models/ggml-small.en.bin")
            .to_string_lossy()
            .into_owned();
        assert_eq!(args[..2], ["-m", held.as_str()]);
        assert!(args.windows(2).any(|pair| pair == ["-f", "-"]), "{args:?}");
        assert!(args.contains(&"{language_code|auto}"), "{args:?}");
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_copy_with_no_model_makes_no_provider() {
        let root = copy(None);
        let program = root.join("bin").join(program_file(std::env::consts::OS));
        let refused = whisper_table(&program.to_string_lossy()).expect_err("no model");
        assert!(refused.1.contains("no model"), "{}", refused.1);
        let _ = std::fs::remove_dir_all(&root);
    }

    #[test]
    fn a_model_is_applied_to_an_endpoint_and_to_nothing_else() {
        let endpoint = json!({"provider": "speech_endpoint", "url": "https://api.openai.com/v1/audio/transcriptions", "model": "whisper-1"});
        let asked = json!({"model": "gpt-transcribe"});
        assert_eq!(
            with_model(&endpoint, &asked).expect("applied")["model"],
            "gpt-transcribe"
        );
        assert_eq!(
            with_model(&endpoint, &json!({})).expect("kept")["model"],
            "whisper-1"
        );
        let command = json!({"provider": "speech_command", "program": "x"});
        assert_eq!(with_model(&command, &asked).expect("kept"), command);
        assert!(with_model(&endpoint, &json!({"model": "bad\nname"})).is_err());
    }

    #[test]
    fn only_an_address_canoa_knows_lists_models() {
        let known = json!({"url": "https://api.openai.com/v1/audio/transcriptions"});
        let rows = models_of(&known);
        assert_eq!(rows[0]["id"], "gpt-transcribe");
        assert!(rows.iter().any(|row| row["id"] == "whisper-1"));
        assert!(
            models_of(&json!({"url": "https://example.test/v1/audio/transcriptions"})).is_empty()
        );
        assert!(models_of(&json!({"program": "x"})).is_empty());
    }
}
