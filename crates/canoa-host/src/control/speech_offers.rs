//! `speech/offers` and `speech/enable`: the speech-to-text providers Canoa offers out of the box. An offer is not a provider until the
//! person adds it: a program a Store type's library runs is installed through the Store (`store/apply`) and then enabled here, and
//! the type's library says how to run the copy; a named service is added through `speech/configure` with the settings the offer
//! carries, and a key of the person's.
//!
//! The same file applies the model a recording asks for in place of the one the provider's table names.
//!
//! Two kinds of offer make tables that are not complete in canoa.cfg, and [`Lend`] completes them each time a provider is built:
//! a service a library describes for the accounts the person has in Canoa (`account`: the key is read from the account's store when a
//! recording opens and is never written to canoa.cfg; what the service is and how it is called is the library's, through
//! `account_speech`) and the service a Store type's models run in on this computer (`service`, the type's id; its address changes with
//! each start and its library says where it is, through the type's `model_store`).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::path::Path;
use std::sync::OnceLock;

use canoa::speech::account::AccountSpeechService;
use canoa::store::models::{ServiceReport, SpeechRun, SpeechService};
use serde_json::{Map, Value, json};

use super::accounts::Accounts as HostAccounts;
use super::files::Files;
use super::speech_accounts::{OutputServices, Services};
use super::{Caller, Grant, NOT_FOUND, REFUSED, Stores, allow, bad, text_param};
use crate::accounts::matching::Catalogs;
use crate::config::{Config, Layers};

type Outcome = Result<Value, (i32, String)>;

/// The table of canoa.cfg the providers are in.
const SECTION: &str = "speech";

/// The key a provider's table keeps the offer it was made from under.
pub(super) const OFFER_KEY: &str = "offer";

/// The key of a table that names the account whose API key the provider uses.
pub(super) const ACCOUNT_KEY: &str = "account";

/// The key of a table that names the Store type whose service on this computer the provider calls, and the one that overrides its
/// address.
pub(super) const SERVICE_KEY: &str = "service";
pub(super) const ADDRESS_KEY: &str = "service_address";

/// Every key of a provider's table that is the host's and not a setting of its kind.
pub(super) const HOST_KEYS: [&str; 8] = [
    OFFER_KEY,
    ACCOUNT_KEY,
    SERVICE_KEY,
    ADDRESS_KEY,
    "library",
    "source",
    "source_settings",
    "import_search",
];

/// The most a model's name may be.
const MAX_MODEL: usize = 100;

/// `speech/offers` and `speech/enable`.
pub(super) fn call(
    caller: &Caller,
    files: &Files,
    lend: &Lend<'_>,
    method: &str,
    params: &Value,
) -> Outcome {
    match method {
        "speech/offers" => allow(caller, Grant::Voice).and_then(|()| offers(files, lend)),
        "speech/enable" => {
            allow(caller, Grant::VoiceSetup).and_then(|()| enable(files, lend, params))
        }
        _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
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
    // A provider that has a `model` setting takes the one asked for; a program's model is in its arguments.
    if table.get("model").is_some()
        && let Some(map) = changed.as_object_mut()
    {
        map.insert("model".to_owned(), json!(model));
    }
    Ok(changed)
}

/// What a provider's table is completed from: the accounts the host keeps, the services the libraries describe for them, and the Store
/// types whose services it calls.
pub(super) struct Lend<'a> {
    /// The canoa.cfg files accounts are listed in, and the home their credentials rest in.
    accounts: Option<(&'a Layers, &'a Path)>,
    /// The stores, which say where a type's service is and what it serves.
    stores: Option<&'a Stores>,
    /// The speech services the libraries describe for accounts, read when first needed.
    services: OnceLock<Services>,
    /// The text-to-speech services the libraries describe for accounts, read when first needed.
    output_services: OnceLock<OutputServices>,
}

impl<'a> Lend<'a> {
    /// What this host has: `accounts` is the host's accounts, when it keeps any, and `stores` its Store.
    pub(super) fn new(accounts: Option<&'a HostAccounts>, stores: Option<&'a Stores>) -> Self {
        Self {
            accounts: accounts.map(|each| (each.layers(), each.home_dir())),
            stores,
            services: OnceLock::new(),
            output_services: OnceLock::new(),
        }
    }

    /// As [`Lend::new`], with the accounts from `layers` and `home`.
    #[cfg(test)]
    pub(super) fn from(layers: &'a Layers, home: &'a Path, stores: Option<&'a Stores>) -> Lend<'a> {
        Lend {
            accounts: Some((layers, home)),
            stores,
            services: OnceLock::new(),
            output_services: OnceLock::new(),
        }
    }

    /// The same, with the services a test names in place of the libraries this build carries.
    #[cfg(test)]
    pub(super) fn with_services(self, services: Services) -> Self {
        let _ = self.services.set(services);
        self
    }

    /// The same, with the speech-output services a test names in place of the libraries this build carries.
    #[cfg(test)]
    pub(super) fn with_output_services(&self, services: OutputServices) {
        let _ = self.output_services.set(services);
    }

    /// The accounts canoa.cfg lists now.
    fn config(&self) -> Result<Config, String> {
        let (layers, _) = self.accounts.ok_or("this host keeps no accounts")?;
        Config::load_layers(layers).map_err(|error| error.to_string())
    }

    /// The services the libraries describe: those this build carries and, when the host has a canoa.cfg, the ones its entries and
    /// extensions load.
    fn services(&self) -> &Services {
        self.services
            .get_or_init(|| Services::of(self.config().ok().as_ref()))
    }

    /// The text-to-speech services the libraries describe: those this build carries and canoa.cfg's extensions.
    fn output_services(&self) -> &OutputServices {
        self.output_services
            .get_or_init(|| OutputServices::of(self.config().ok().as_ref()))
    }

    /// The built-in `[say]` tables made of the person's matching accounts (`SERVICE:NAME`), holding no key.
    pub(super) fn account_output_tables(&self) -> Vec<(String, Value)> {
        let Ok(config) = self.config() else {
            return Vec::new();
        };
        let kinds = Catalogs::new(&config);
        let mut tables = Vec::new();
        for (name, account) in &config.accounts {
            for (service, verdict) in self.output_services().offered(&kinds, account) {
                if verdict.is_err() {
                    continue;
                }
                let offer = format!("{}:{name}", service.id);
                let mut map = Map::new();
                map.insert("provider".to_owned(), json!(service.provider));
                map.insert("account".to_owned(), json!(name));
                map.insert("offer".to_owned(), json!(offer));
                map.insert("credential".to_owned(), json!(service.credential));
                map.insert(
                    "title".to_owned(),
                    json!(format!("{}, account {name}", service.title)),
                );
                if service.endpoint {
                    map.insert("url".to_owned(), json!(service.url));
                    if !service.models.is_empty() {
                        map.insert("models".to_owned(), json!(service.models));
                    }
                    if !service.voices.is_empty() {
                        map.insert("voices".to_owned(), json!(service.voices));
                    }
                    if !service.styles.is_empty() {
                        map.insert("styles".to_owned(), json!(service.styles));
                    }
                }
                tables.push((offer, Value::Object(map)));
            }
        }
        tables
    }

    /// Reads the key of `account` for speech output from the credential store, checking the account still matches when `service` is
    /// named.
    pub(super) fn output_key(
        &self,
        account_name: &str,
        wanted_service: Option<&str>,
        fallback_credential: &str,
    ) -> Result<String, String> {
        let config = self.config()?;
        let account = config
            .accounts
            .get(account_name)
            .ok_or_else(|| format!("the account {account_name} is gone: add it again"))?;
        let credential = if let Some(id) = wanted_service
            && let Some(service) = self.output_services().find(id)
        {
            let kinds = Catalogs::new(&config);
            match self
                .output_services()
                .offered(&kinds, account)
                .into_iter()
                .find(|(each, _)| each.id == service.id)
            {
                Some((_, Ok(()))) => service.credential.clone(),
                Some((_, Err(why))) => return Err(why),
                None => {
                    return Err(format!(
                        "an account of kind {} does not offer {} for text-to-speech",
                        account.kind, service.title
                    ));
                }
            }
        } else {
            fallback_credential.to_owned()
        };
        let (_, home) = self.accounts.ok_or("this host keeps no accounts")?;
        super::speech_accounts::read_credential(&config, home, account_name, &credential)
    }

    /// The models the account `account` lists that can turn text into speech, from the `sdk_provider` that serves the service
    /// `service`: what the account's key lists, not a list of the host's.
    pub(super) fn account_speech_models(
        &self,
        account: &str,
        service: &str,
    ) -> Result<Vec<String>, String> {
        let config = self.config()?;
        let assist = crate::assist::Assist::of(&config, self.accounts);
        let id = format!("{service}:{account}");
        let backend = assist
            .backends()
            .iter()
            .find(|backend| backend.id() == id)
            .ok_or_else(|| {
                format!("no library lists the models of the account {account} for {service}")
            })?;
        let listing = backend
            .models(&canoa::network::Network::current())
            .map_err(|failure| failure.why)?;
        Ok(
            canoa::sdk::models_for(&listing.models, canoa::sdk::capability::TEXT_TO_SPEECH)
                .offered
                .into_iter()
                .map(|model| model.id)
                .collect(),
        )
    }

    /// The models of the service a recording is posted to at the address in `settings`: none for an address no library describes.
    pub(super) fn models_at(&self, settings: &Map<String, Value>) -> Vec<Value> {
        settings
            .get("url")
            .and_then(Value::as_str)
            .map(|url| self.services().models_at(url))
            .unwrap_or_default()
    }

    /// What the kind of the account `name` is called, for the provider made of it: none when the account is gone.
    pub(super) fn account_kind(&self, name: &str) -> Option<String> {
        let config = self.config().ok()?;
        let account = config.accounts.get(name)?;
        let kinds = Catalogs::new(&config);
        Some(
            kinds
                .made(&account.kind)
                .map(|(_, kind)| kind.title)
                .unwrap_or_else(|| account.kind.clone()),
        )
    }

    /// The service the account `name` can call, as a library describes it: the one `wanted` names, else the first its kind is a key for.
    /// The reason when it is none.
    fn service_for(
        &self,
        config: &Config,
        name: &str,
        wanted: Option<&str>,
    ) -> Result<AccountSpeechService, String> {
        let account = config
            .accounts
            .get(name)
            .ok_or_else(|| format!("the account {name} is gone: add the provider again"))?;
        let mut reason = None;
        for (service, verdict) in self
            .services()
            .offered(&Catalogs::new(config), account)
            .into_iter()
            .filter(|(service, _)| wanted.is_none_or(|id| id == service.id))
        {
            match verdict {
                Ok(()) => return Ok(service.clone()),
                Err(why) => reason = reason.or(Some(why)),
            }
        }
        Err(reason.unwrap_or_else(|| {
            format!(
                "no library describes a speech-to-text service for an account of kind {}",
                account.kind
            )
        }))
    }

    /// The API key of the account `name`, from the store its credentials rest in, for the service `wanted` names or else the one its
    /// kind is a key for.
    fn key_of(&self, name: &str, wanted: Option<&str>) -> Result<String, String> {
        let config = self.config()?;
        let service = self.service_for(&config, name, wanted)?;
        let (_, home) = self.accounts.ok_or("this host keeps no accounts")?;
        super::speech_accounts::read_credential(&config, home, name, &service.credential)
    }

    /// Where the service of the Store type `service` is now, as its library says, with the models that can transcribe when asked
    /// for: the reason when it cannot be asked.
    fn report(&self, service: &str, models: bool) -> Result<ServiceReport, String> {
        self.stores
            .ok_or_else(|| "this host keeps no Store".to_owned())?
            .speech_report(service, models)
    }

    /// The address of the service a table calls: the one it names, else the running service's; the reason when there is none.
    fn service_base(&self, table: &Value, service: &str) -> Result<String, String> {
        if let Some(address) = table.get(ADDRESS_KEY).and_then(Value::as_str) {
            return Ok(address.trim_end_matches('/').to_owned());
        }
        let report = self.report(service, false)?;
        report.address.ok_or_else(|| {
            report
                .unavailable
                .unwrap_or_else(|| format!("the service of {service} is not running"))
        })
    }

    /// Why a provider cannot take a recording now for want of a service, when it cannot.
    pub(super) fn missing(&self, table: &Value) -> Option<String> {
        let service = table.get(SERVICE_KEY).and_then(Value::as_str)?;
        self.service_base(table, service).err()
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
            let wanted = table
                .get(OFFER_KEY)
                .and_then(Value::as_str)
                .and_then(|offer| offer.split_once(':'))
                .map(|(service, _)| service);
            if with_key {
                map.insert(
                    "api_key".to_owned(),
                    Value::String(self.key_of(account, wanted)?),
                );
            } else {
                self.service_for(&self.config()?, account, wanted)?;
            }
        }
        if let Some(service) = table.get(SERVICE_KEY).and_then(Value::as_str) {
            let how = self
                .stores
                .and_then(|stores| {
                    stores
                        .speech_services()
                        .into_iter()
                        .find(|(id, _)| id == service)
                })
                .map(|(_, how)| how)
                .ok_or_else(|| format!("this host has no service {service}"))?;
            // Saving or listing a provider needs no service: only a recording does, and the listing says so with `problem`.
            let base = match self.service_base(table, service) {
                Ok(base) => base,
                Err(why) if with_key => return Err(why),
                Err(_) => "http://127.0.0.1".to_owned(),
            };
            map.insert(
                "url".to_owned(),
                Value::String(format!("{base}{}", how.route)),
            );
            if how.local_file {
                map.insert("request".to_owned(), Value::String("local_file".to_owned()));
            }
        }
        Ok(settings)
    }
}

/// The offers made of the person's accounts: one for each service a library describes that an account of a provider that signs in to
/// it can call.
fn account_offers(files: &Files, lend: &Lend<'_>) -> Result<Vec<Value>, (i32, String)> {
    let Ok(config) = lend.config() else {
        return Ok(Vec::new());
    };
    let kinds = Catalogs::new(&config);
    let mut rows = Vec::new();
    for (name, account) in &config.accounts {
        for (service, verdict) in lend.services().offered(&kinds, account) {
            let id = format!("{}:{name}", service.id);
            let models: Vec<Value> = service
                .models
                .iter()
                .map(|model| json!({"id": model.id, "label": model.label, "capabilities": ["speech_to_text"]}))
                .collect();
            rows.push(json!({
                "id": id,
                "title": format!("{}, account {name}", service.title),
                "summary": format!("{} Under the API key of your account {name}.", service.summary).trim(),
                "local": false,
                "where": format!("{} It is sent under the API key of your account {name} in Canoa, which Canoa reads when you dictate and never shows or copies.", service.location).trim(),
                "account": name,
                "default_model": models.first().map(|model| model["id"].clone()),
                "models": models,
                "unavailable": verdict.err(),
                "provider": enabled(files, &id)?,
            }));
        }
    }
    Ok(rows)
}

/// The offers of what the Store types' libraries run on this computer, the programs before the services. A program is installed
/// through its type's first store, with the model chosen as the channel, and is ready when the host has a copy the library can run;
/// a service is ready when it is running and lists a model that transcribes. Each says what is missing in the library's words.
fn store_offers(files: &Files, lend: &Lend<'_>) -> Result<Vec<Value>, (i32, String)> {
    let Some(stores) = lend.stores else {
        return Ok(Vec::new());
    };
    let mut rows = Vec::new();
    let (programs, services): (Vec<_>, Vec<_>) = stores
        .speech_services()
        .into_iter()
        .partition(|(_, how)| how.command);
    for (id, how) in programs {
        rows.push(program_offer(files, stores, &id, &how)?);
    }
    for (id, how) in services {
        rows.push(service_offer(files, stores, &id, &how)?);
    }
    Ok(rows)
}

fn model_choices(report: &Result<ServiceReport, String>) -> Vec<Value> {
    report
        .as_ref()
        .map(|report| {
            report
                .models
                .iter()
                .map(|model| json!({"id": model.id, "label": model.label, "capabilities": ["speech_to_text"]}))
                .collect()
        })
        .unwrap_or_default()
}

/// The offer of a program the type's library runs: not installed until the Store has a copy, and then ready or saying why not.
fn program_offer(
    files: &Files,
    stores: &Stores,
    id: &str,
    how: &SpeechService,
) -> Result<Value, (i32, String)> {
    let store = stores.speech_store(id);
    let installed = store
        .as_deref()
        .and_then(|store| stores.managed_version(store).ok().flatten());
    let report = stores.speech_report(id, true);
    let models = model_choices(&report);
    let unavailable = match (&installed, &report) {
        (None, _) => None,
        (Some(_), Ok(report)) => report.unavailable.clone(),
        (Some(_), Err(why)) => Some(why.clone()),
    };
    Ok(json!({
        "id": id,
        "title": how.title,
        "summary": how.summary,
        "license": how.license,
        "local": true,
        "where": how.location,
        "store": store,
        "default_model": models.first().map(|model| model["id"].clone()),
        "models": models,
        "installed": installed.is_some(),
        "version": installed,
        "unavailable": unavailable,
        "provider": enabled(files, id)?,
    }))
}

/// The offer of the service a type's library runs models in: ready when the service is running and lists a model that transcribes.
fn service_offer(
    files: &Files,
    stores: &Stores,
    id: &str,
    how: &SpeechService,
) -> Result<Value, (i32, String)> {
    let report = stores.speech_report(id, true);
    let unavailable = match &report {
        Ok(report) => report.unavailable.clone(),
        Err(why) => Some(why.clone()),
    };
    let models = model_choices(&report);
    Ok(json!({
        "id": id,
        "title": how.title,
        "summary": how.summary,
        "license": how.license,
        "local": true,
        "where": how.location,
        "service": id,
        "default_model": models.first().map(|model| model["id"].clone()),
        "models": models,
        "installed": unavailable.is_none(),
        "unavailable": unavailable,
        "provider": enabled(files, id)?,
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

fn offers(files: &Files, lend: &Lend<'_>) -> Outcome {
    let mut offers = store_offers(files, lend)?;
    offers.extend(account_offers(files, lend)?);
    Ok(json!({"offers": offers}))
}

/// The table of a program's provider, from how its library says to run the copy.
fn program_table(run: &SpeechRun, offer: &str) -> Map<String, Value> {
    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("speech_command"));
    table.insert("program".to_owned(), json!(run.program));
    table.insert("args".to_owned(), json!(run.args));
    table.insert("input".to_owned(), json!(run.input));
    table.insert(OFFER_KEY.to_owned(), json!(offer));
    table
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

/// `speech/enable` `{offer, name?, model?}`: makes (or remakes) the provider of an offer that needs no form. The id of a Store type
/// whose library runs a program is made from the copy the Store has installed, with the model `model` when it is installed and
/// else the first the library finds (another is installed from the Models page, and this is called again). The id of a type whose models run in a service takes the model `model`
/// of that service, which the person loads, and `SERVICE:NAME` the API key of the account `NAME` for the service a library describes.
fn enable(files: &Files, lend: &Lend<'_>, params: &Value) -> Outcome {
    let offer = text_param(params, "offer")?.ok_or_else(|| bad("offer is needed"))?;
    if let Some(stores) = lend.stores
        && let Some((_, how)) = stores
            .speech_services()
            .into_iter()
            .find(|(id, _)| id == offer)
    {
        return if how.command {
            enable_program(files, stores, offer, params)
        } else if let Some(provider) = &how.provider {
            enable_in_process(files, stores, offer, provider, params)
        } else {
            enable_service(files, offer, params)
        };
    }
    if let Some((service, account)) = offer.split_once(':')
        && let Some(service) = lend.services().find(service)
    {
        return enable_account(files, lend, service, account, params);
    }
    Err((
        NOT_FOUND,
        format!("{offer} is added with speech/configure, not enabled"),
    ))
}

/// The provider that runs the copy of the program of the Store type `service` the host has installed, as its library says.
fn enable_program(files: &Files, stores: &Stores, service: &str, params: &Value) -> Outcome {
    let name = name_of(params, service)?;
    let asked = text_param(params, "model")?.filter(|model| !model.is_empty());
    let report = stores
        .speech_report_of(service, false, asked)
        .map_err(|why| (REFUSED, why))?;
    let run = report.command.ok_or_else(|| {
        (
            REFUSED,
            report
                .unavailable
                .unwrap_or_else(|| format!("{service} has no copy that can be run")),
        )
    })?;
    write_table(files, name, program_table(&run, service))?;
    Ok(json!({"name": name, "provider": "speech_command", "model": run.model, "saved": true}))
}

/// The provider of the library that runs inside this host, made from the settings its library reports for the model asked for when it
/// is installed and else the first that is.
fn enable_in_process(
    files: &Files,
    stores: &Stores,
    offer: &str,
    provider: &str,
    params: &Value,
) -> Outcome {
    let name = name_of(params, offer)?;
    let asked = text_param(params, "model")?.filter(|model| !model.is_empty());
    let report = stores
        .speech_report_of(offer, false, asked)
        .map_err(|why| (REFUSED, why))?;
    let settings = report
        .settings
        .as_ref()
        .and_then(|settings| crate::forms::json_of(settings).ok());
    let Some(Value::Object(settings)) = settings else {
        return Err((
            REFUSED,
            report
                .unavailable
                .unwrap_or_else(|| format!("{offer} has no model that can be run")),
        ));
    };
    let model = settings.get("model").cloned().unwrap_or(Value::Null);
    // A service several engines run a model on names the engine that runs this one.
    let provider = report.provider.as_deref().unwrap_or(provider);
    let mut table = settings;
    table.insert("provider".to_owned(), json!(provider));
    table.insert(OFFER_KEY.to_owned(), json!(offer));
    write_table(files, name, table)?;
    Ok(json!({"name": name, "provider": provider, "model": model, "saved": true}))
}

/// The provider that calls the model `model` of the service of the Store type `service`. The model is not loaded, checked or looked
/// for here: loading one is the person's choice, and a provider whose service is not running says so when it is listed.
fn enable_service(files: &Files, service: &str, params: &Value) -> Outcome {
    let name = name_of(params, service)?;
    let model = model_of(params, None)?;
    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("speech_endpoint"));
    table.insert("model".to_owned(), json!(model));
    table.insert(SERVICE_KEY.to_owned(), json!(service));
    table.insert(OFFER_KEY.to_owned(), json!(service));
    write_table(files, name, table)?;
    Ok(json!({"name": name, "provider": "speech_endpoint", "model": model, "saved": true}))
}

/// The provider that sends audio to `service` under the API key of the account `account`. The key is not copied: the table names the
/// account, and the key is read from its store whenever a recording opens.
fn enable_account(
    files: &Files,
    lend: &Lend<'_>,
    service: &AccountSpeechService,
    account: &str,
    params: &Value,
) -> Outcome {
    let fallback: String = format!("{}-{account}", service.id)
        .chars()
        .take(40)
        .collect();
    let name = name_of(params, &fallback)?;
    let model = model_of(
        params,
        service.models.first().map(|model| model.id.as_str()),
    )?;
    // The key is read once so a missing one is said now and not at the first dictation, and dropped at once.
    drop(
        lend.key_of(account, Some(&service.id))
            .map_err(|why| (REFUSED, format!("{account}: {why}")))?,
    );
    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("speech_endpoint"));
    table.insert("url".to_owned(), json!(service.url));
    table.insert("model".to_owned(), json!(model));
    table.insert(ACCOUNT_KEY.to_owned(), json!(account));
    table.insert(
        OFFER_KEY.to_owned(),
        json!(format!("{}:{account}", service.id)),
    );
    write_table(files, name, table)?;
    Ok(
        json!({"name": name, "provider": "speech_endpoint", "model": model, "account": account, "saved": true}),
    )
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_provider_of_a_program_runs_it_as_its_library_says_and_names_its_offer() {
        let run = SpeechRun::new(
            "/programs/stt",
            ["-m", "/models/m.bin", "-f", "-"],
            "wav",
            "m",
        );
        let table = program_table(&run, "talk");
        assert_eq!(table["provider"], "speech_command");
        assert_eq!(table["program"], "/programs/stt");
        assert_eq!(table["args"], json!(["-m", "/models/m.bin", "-f", "-"]));
        assert_eq!(table["input"], "wav");
        assert_eq!(table[OFFER_KEY], "talk");
        assert!(
            !table.contains_key("model"),
            "the model is in the arguments, not a setting of the kind"
        );
    }

    #[test]
    fn a_model_is_applied_to_an_endpoint_and_to_nothing_else() {
        let endpoint = json!({"provider": "speech_endpoint", "url": "https://stt.example/v1/audio/transcriptions", "model": "model-a"});
        let asked = json!({"model": "model-b"});
        assert_eq!(
            with_model(&endpoint, &asked).expect("applied")["model"],
            "model-b"
        );
        assert_eq!(
            with_model(&endpoint, &json!({})).expect("kept")["model"],
            "model-a"
        );
        let command = json!({"provider": "speech_command", "program": "x"});
        assert_eq!(with_model(&command, &asked).expect("kept"), command);
        assert!(with_model(&endpoint, &json!({"model": "bad\nname"})).is_err());
    }
}
