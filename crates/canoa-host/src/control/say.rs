//! `speech/voices`, `speech/say`, `speech/audio`, `speech/stop` and `speech/say_configure`: text to speech, the other direction of
//! `speech/*`. A provider is a `[say.NAME]` table of canoa.cfg naming one of the kinds this host has (`provider = "speech_say_command"`,
//! `"speech_say_endpoint"` or `"speech_say_system"`, or a provider a library this build carries offers) and its settings, checked
//! against that kind's schema; the host also offers `system` (the operating system's voices) with no table, and each Store type
//! whose installed program reads text aloud (the type's library names the provider and says its settings), until a table of that name
//! says otherwise.
//!
//! An utterance is `speech/say`, then `speech/audio` as often as the client wants the next piece of audio (it answers as soon as
//! there is some, or empty after `wait_ms`), then it is over (`done`) or `speech/stop` ends it. The client pulls, so a client that
//! plays slowly holds the provider back at the provider's own buffer and nothing grows here.
//!
//! The tables are read when a method is called. They are written under the `voice.setup` grant and no read ever answers a key.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "clocks and locks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use canoa::speech::builtin::{
    CommandSay, CommandSaySettings, EndpointSay, EndpointSaySettings, SystemSettings, SystemSpeech,
};
use canoa::speech::output::{
    READ_TIMED_OUT, SayRequest, SpeechOutput, SpeechUtterance, VoiceQuery,
};
use coqui_acp_api::ProcessSpawner as _;
use coqui_acp_api::guatiao::library::{Object, Registry};
use coqui_acp_api::guatiao::{Alloc, FromValue, ToValue};
use serde_json::{Map, Value, json};

use super::files::Files;
use super::speech::{form_of, members_of, schema_of, secrets_of, text_list, valid_name};
use super::speech_offers::Lend;
use super::{Caller, Grant, NOT_FOUND, REFUSED, Stores, allow, bad, text_param};
use crate::speech_scripts::{self, ORIGIN_KEYS};

/// Host-managed keys on synthesized account voice entries that are stripped from provider settings.
const ACCOUNT_KEYS: [&str; 4] = ["account", "offer", "credential", "title"];

/// The keys of `table` that the host wrote: those of [`ACCOUNT_KEYS`] when it is an account offer (it names an `account`), none
/// for a table the person wrote, whose `title` or `credential` is theirs.
pub(super) fn host_keys(table: &Value) -> &'static [&'static str] {
    if table.get("account").is_some() {
        &ACCOUNT_KEYS
    } else {
        &[]
    }
}

type Outcome = Result<Value, (i32, String)>;

/// The table of canoa.cfg the providers are in.
const SECTION: &str = "say";

/// The most audio one `speech/audio` carries, in bytes before base64, when the call names no bound.
pub const DEFAULT_CHUNK: usize = 32 * 1024;
/// The most audio one `speech/audio` may be asked for.
pub const MAX_CHUNK: usize = 256 * 1024;
/// How long `speech/audio` waits for audio when the call names no time, in milliseconds.
pub const DEFAULT_WAIT_MS: u64 = 500;
/// The longest `speech/audio` waits, in milliseconds: a stop is a request of its own, so this bounds one wait, not the stop.
pub const MAX_WAIT_MS: u64 = 2000;
/// The utterances one key may have open at once.
pub const MAX_PER_KEY: usize = 4;
/// The utterances open at once, whoever opened them.
pub const MAX_OPEN: usize = 16;
/// How long an utterance may go with no call before the host ends it, so a page that closed leaves no program running.
pub const IDLE: Duration = Duration::from_secs(120);
/// How long a list of voices is kept before the provider is asked again.
const VOICES_KEPT: Duration = Duration::from_secs(30);
/// The most text one `speech/say` carries, in bytes.
pub const MAX_TEXT: usize = 256 * 1024;

/// The name of the provider the operating system's voices are offered as.
pub const SYSTEM: &str = "system";
type Provider = Arc<dyn SpeechOutput>;

/// An utterance's id and the object it runs on.
type Found = (String, Arc<Object<dyn SpeechUtterance>>);

/// One kind of provider this host has.
struct Kind {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    build: fn(&Value) -> Result<Provider, String>,
    schema: fn() -> Result<Value, String>,
    form: fn() -> Result<Value, String>,
}

fn kind_of<S, P>(settings: &Value) -> Result<Provider, String>
where
    S: FromValue,
    P: SpeechOutput + From<S> + 'static,
{
    let value = guatiao_serde::adapt::to_value_in(settings, Alloc::rust())
        .map_err(|_| "the settings cannot be read".to_owned())?;
    let settings = S::from_value(&value).map_err(|error| error.to_string())?;
    Ok(Arc::new(P::from(settings)))
}

const KINDS: [Kind; 3] = [
    Kind {
        id: "speech_say_system",
        title: "System voices",
        description: "The voices of the computer this host runs on",
        build: kind_of::<SystemSettings, SystemSpeech>,
        schema: schema_of::<SystemSettings>,
        form: form_of::<SystemSettings>,
    },
    Kind {
        id: "speech_say_endpoint",
        title: "Endpoint",
        description: "Sends the text to a text-to-speech address and model you name",
        build: kind_of::<EndpointSaySettings, EndpointSay>,
        schema: schema_of::<EndpointSaySettings>,
        form: form_of::<EndpointSaySettings>,
    },
    Kind {
        id: "speech_say_command",
        title: "Command",
        description: "Runs a program that reads text and writes audio",
        build: kind_of::<CommandSaySettings, CommandSay>,
        schema: schema_of::<CommandSaySettings>,
        form: form_of::<CommandSaySettings>,
    },
];

fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|kind| kind.id == id)
}

/// The libraries this build carries, as one registry: the providers of the speech-output kind they offer are kinds a table may name.
static CARRIED: Mutex<Option<Arc<Registry>>> = Mutex::new(None);

pub(super) fn carried() -> Option<Arc<Registry>> {
    let mut held = CARRIED.lock().unwrap_or_else(PoisonError::into_inner);
    if held.is_none() {
        let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        match crate::bundled::register_included(&mut registry) {
            Ok(()) => *held = Some(Arc::new(registry)),
            Err(why) => {
                tracing::warn!("the libraries this build carries are not read for voices: {why}");
            }
        }
    }
    held.clone()
}

/// The kinds of provider the carried libraries offer.
fn carried_kinds() -> Vec<Resolved> {
    carried().map_or_else(Vec::new, |registry| Resolved::scripted(&registry))
}

/// One kind of provider a table may name, wherever its code is: the host's own, or a registry the table's `library` or `source`
/// names.
#[derive(Clone)]
struct Resolved {
    id: String,
    title: String,
    description: String,
    schema: Value,
    form: Value,
    how: How,
}

#[derive(Clone)]
enum How {
    Native(&'static Kind),
    Script {
        registry: Arc<Registry>,
        key: String,
    },
}

impl Resolved {
    fn native(kind: &'static Kind) -> Result<Self, String> {
        Ok(Self {
            id: kind.id.to_owned(),
            title: kind.title.to_owned(),
            description: kind.description.to_owned(),
            schema: (kind.schema)()?,
            form: (kind.form)()?,
            how: How::Native(kind),
        })
    }

    fn scripted(registry: &Arc<Registry>) -> Vec<Self> {
        speech_scripts::described::<dyn SpeechOutput>(registry)
            .into_iter()
            .map(|each| Self {
                id: each.id,
                title: each.title,
                description: each.description,
                schema: each.schema,
                form: each.form,
                how: How::Script {
                    registry: Arc::clone(registry),
                    key: each.key,
                },
            })
            .collect()
    }

    /// A provider of this kind with `settings`, and the registry that has to stay loaded while it or what it opens is used.
    fn build(&self, settings: &Value) -> Result<(Provider, Option<Arc<Registry>>), String> {
        match &self.how {
            How::Native(kind) => Ok(((kind.build)(settings)?, None)),
            How::Script { registry, key } => {
                let offer = registry
                    .offer::<dyn SpeechOutput>(key)
                    .ok_or_else(|| format!("the provider {} is gone", self.id))?
                    .map_err(|_| format!("{} is not a speech output provider", self.id))?;
                let provider: Provider = if offer.builds_instances() {
                    let value = speech_scripts::to_guatiao(settings)?;
                    Arc::new(
                        offer
                            .instantiate(&value)
                            .map_err(|error| error.to_string())?,
                    )
                } else if settings.as_object().is_none_or(Map::is_empty) {
                    offer
                        .shared()
                        .ok_or_else(|| format!("{} needs settings", self.id))?
                } else {
                    return Err(format!("{} takes no settings", self.id));
                };
                Ok((provider, Some(Arc::clone(registry))))
            }
        }
    }
}

/// Where canoa.cfg's relative paths are taken from.
fn base_of(files: &Files) -> &std::path::Path {
    files
        .config_file()
        .and_then(std::path::Path::parent)
        .unwrap_or_else(|| std::path::Path::new("."))
}

/// The kind of provider `id` is for a table that names `origin` (the registry its `library` or `source` loads): the registry's own,
/// else one the host carries.
fn resolve(id: &str, origin: Option<&Arc<Registry>>) -> Result<Resolved, String> {
    if let Some(registry) = origin {
        return Resolved::scripted(registry)
            .into_iter()
            .find(|each| each.id == id)
            .ok_or_else(|| format!("the library or source it names has no provider {id}"));
    }
    if let Some(kind) = kind(id) {
        return Resolved::native(kind);
    }
    carried_kinds()
        .into_iter()
        .find(|each| each.id == id)
        .ok_or_else(|| format!("this host has no provider {id}"))
}

/// The kind of provider a table names, its origin loaded.
fn resolve_table(
    files: &Files,
    table: &Value,
) -> Result<(Resolved, Option<Arc<Registry>>), String> {
    let id = table
        .get("provider")
        .and_then(Value::as_str)
        .ok_or("it names no provider")?;
    let origin = speech_scripts::registry_of(table, base_of(files))?;
    resolve(id, origin.as_ref()).map(|resolved| (resolved, origin))
}

/// A provider made from a table.
struct Built {
    provider: Provider,
    /// What keeps its code loaded: dropped after the provider.
    code: Option<Arc<Registry>>,
}

/// One utterance a client has open.
struct Open {
    owner: String,
    utterance: Arc<Object<dyn SpeechUtterance>>,
    last: Instant,
    /// The code the utterance runs in, kept loaded until it ends: dropped after the utterance.
    _code: Option<Arc<Registry>>,
}

/// A list of voices, kept for a while.
struct Kept {
    at: Instant,
    table: Value,
    answer: Value,
}

/// The tables the host offers for the Store types whose installed program reads text aloud: each named for its type, with the provider
/// and the settings its library says.
type Offered = Box<dyn Fn(Option<&Stores>) -> Vec<(String, Value)> + Send + Sync>;

fn from_stores(stores: Option<&Stores>) -> Vec<(String, Value)> {
    let Some(stores) = stores else {
        return Vec::new();
    };
    stores
        .voice_services()
        .into_iter()
        .filter_map(|(name, provider)| {
            let mut table = stores.voice_settings(&name)?;
            table
                .as_object_mut()?
                .insert("provider".to_owned(), json!(provider));
            Some((name, table))
        })
        .collect()
}

/// The utterances the host has open for clients, and the voices it has listed lately.
pub(crate) struct Say {
    open: Mutex<BTreeMap<String, Open>>,
    next: AtomicU64,
    kept: Mutex<BTreeMap<String, Kept>>,
    offered: Offered,
    #[cfg(test)]
    output_services: Mutex<Option<super::speech_accounts::OutputServices>>,
}

impl Default for Say {
    fn default() -> Self {
        Self {
            open: Mutex::default(),
            next: AtomicU64::new(0),
            kept: Mutex::default(),
            offered: Box::new(from_stores),
            #[cfg(test)]
            output_services: Mutex::new(None),
        }
    }
}

impl core::fmt::Debug for Say {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Say").finish_non_exhaustive()
    }
}

/// The providers canoa.cfg defines, as name to its table.
fn tables(files: &Files) -> Result<Map<String, Value>, (i32, String)> {
    Ok(match files.hidden_section(SECTION)? {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    })
}

impl Say {
    /// The tables offered for Store types, as a test says them.
    #[cfg(test)]
    pub(crate) fn offering(mut self, tables: Vec<(String, Value)>) -> Self {
        self.offered = Box::new(move |_| tables.clone());
        self
    }

    /// Replaces the account speech-output descriptions with `services` for tests.
    #[cfg(test)]
    pub(crate) fn with_output_services(&self, services: super::speech_accounts::OutputServices) {
        *self
            .output_services
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(services);
    }

    pub(crate) fn call(
        &self,
        caller: &Caller,
        files: &Files,
        lend: &Lend<'_>,
        stores: Option<&Stores>,
        method: &str,
        params: &Value,
    ) -> Outcome {
        #[cfg(test)]
        if let Some(services) = self
            .output_services
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
        {
            lend.with_output_services(services);
        }
        match method {
            "speech/voices" => {
                allow(caller, Grant::Voice).and_then(|()| self.voices(files, lend, stores, params))
            }
            "speech/say_configure" => {
                allow(caller, Grant::VoiceSetup).and_then(|()| self.configure(files, lend, params))
            }
            "speech/say" => allow(caller, Grant::Voice)
                .and_then(|()| self.say(caller, files, lend, stores, params)),
            "speech/audio" => allow(caller, Grant::Voice).and_then(|()| self.audio(caller, params)),
            "speech/stop" => allow(caller, Grant::Voice).and_then(|()| self.stop(caller, params)),
            _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
        }
    }

    /// Makes every open utterance look `by` older, so a test need not wait out [`IDLE`].
    #[cfg(test)]
    pub(crate) fn age(&self, by: Duration) {
        for open in self.held().values_mut() {
            open.last = open.last.checked_sub(by).unwrap_or(open.last);
        }
    }

    fn held(&self) -> MutexGuard<'_, BTreeMap<String, Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// The providers this call may use: the file's tables, and the ones the host offers by itself that the file does not name.
    fn all_tables(
        &self,
        files: &Files,
        lend: &Lend<'_>,
        stores: Option<&Stores>,
    ) -> Result<Map<String, Value>, (i32, String)> {
        let mut all = tables(files)?;
        if !all.contains_key(SYSTEM) && cfg!(any(windows, unix)) {
            all.insert(SYSTEM.to_owned(), json!({"provider": "speech_say_system"}));
        }
        for (name, table) in (self.offered)(stores) {
            all.entry(name).or_insert(table);
        }
        for (name, table) in lend.account_output_tables() {
            all.entry(name).or_insert(table);
        }
        Ok(all)
    }

    /// What one table says, built: its kind and the provider, or why it cannot be. When `with_key` is set and the table names an
    /// `account`, the credential is read from the account's store without writing it to `canoa.cfg`.
    fn built(
        &self,
        files: &Files,
        lend: &Lend<'_>,
        table: &Value,
        with_key: bool,
    ) -> Result<(Resolved, Built), String> {
        let (resolved, origin) = resolve_table(files, table)?;
        let mut settings = table.clone();
        if let Some(map) = settings.as_object_mut() {
            map.remove("provider");
            for key in ORIGIN_KEYS {
                map.remove(key);
            }
            for key in host_keys(table) {
                map.remove(*key);
            }
            if with_key && let Some(account) = table.get("account").and_then(Value::as_str) {
                let service = table
                    .get("offer")
                    .and_then(Value::as_str)
                    .and_then(|offer| offer.split_once(':'))
                    .map(|(service, _)| service);
                let credential = table
                    .get("credential")
                    .and_then(Value::as_str)
                    .unwrap_or("api_key");
                map.insert(
                    "api_key".to_owned(),
                    Value::String(lend.output_key(account, service, credential)?),
                );
            }
        }
        let (provider, code) = resolved.build(&settings)?;
        let built = Built {
            provider,
            code: code.or(origin),
        };
        Ok((resolved, built))
    }

    fn spawner() -> Object<dyn coqui_acp_api::ProcessSpawner> {
        // The host's own spawner starts any program, as it does for every program it runs for the person.
        coqui_acp_harness::LocalSpawner::new().into_object()
    }

    /// `speech/voices`: the kinds this host has, each with the schema and the form its settings are drawn with, and the providers it
    /// offers, each with what it says of itself, where the text goes and the voices it has. A setting that is a secret is never
    /// answered, only that it is set. `provider` asks for one provider, `language` for the voices that speak it.
    fn voices(
        &self,
        files: &Files,
        lend: &Lend<'_>,
        stores: Option<&Stores>,
        params: &Value,
    ) -> Outcome {
        let only = text_param(params, "provider")?;
        let language = text_param(params, "language")?;
        let all = self.all_tables(files, lend, stores)?;
        if let Some(only) = only
            && !all.contains_key(only)
        {
            return Err((NOT_FOUND, format!("this host has no voice provider {only}")));
        }
        let mut kinds = Vec::new();
        for kind in &KINDS {
            kinds.push(Resolved::native(kind).map_err(|why| (REFUSED, why))?);
        }
        kinds.extend(carried_kinds());
        let in_file = tables(files)?;
        let mut listed = Vec::new();
        for (name, table) in &all {
            if only.is_some_and(|only| only != name) {
                continue;
            }
            listed.push(self.entry(
                files,
                lend,
                (name, table, in_file.contains_key(name)),
                language,
                &mut kinds,
            )?);
        }
        let mut seen = BTreeSet::new();
        let types: Vec<Value> = kinds
            .iter()
            .filter(|kind| seen.insert(kind.id.clone()))
            .map(|kind| {
                json!({
                    "id": kind.id,
                    "title": kind.title,
                    "description": kind.description,
                    "schema": kind.schema,
                    "form": kind.form,
                })
            })
            .collect();
        Ok(json!({"types": types, "providers": listed}))
    }

    fn entry(
        &self,
        files: &Files,
        lend: &Lend<'_>,
        (name, table, written): (&str, &Value, bool),
        language: Option<&str>,
        kinds: &mut Vec<Resolved>,
    ) -> Outcome {
        let mut entry =
            json!({"name": name, "provider": table.get("provider"), "builtin": !written});
        if let Some(account) = table.get("account").and_then(Value::as_str) {
            entry["account"] = json!(account);
        }
        let found = resolve_table(files, table);
        let secrets = match &found {
            Ok((resolved, _)) => secrets_of(&resolved.schema),
            Err(_) => BTreeSet::new(),
        };
        if let Ok((resolved, _)) = &found
            && matches!(resolved.how, How::Script { .. })
        {
            // A kind a table's own library or source adds is listed with the host's, so a settings screen can draw it.
            kinds.push(resolved.clone());
        }
        let mut settings = Map::new();
        let mut set = Vec::new();
        let host = host_keys(table);
        if let Some(map) = table.as_object() {
            for (key, value) in map.iter().filter(|(key, _)| {
                *key != "provider"
                    && !ORIGIN_KEYS.contains(&key.as_str())
                    && !host.contains(&key.as_str())
            }) {
                if secrets.contains(key) {
                    set.push(key.clone());
                } else {
                    settings.insert(key.clone(), value.clone());
                }
            }
        }
        entry["settings"] = Value::Object(settings);
        entry["secrets_set"] = json!(set);
        let made = match self.built(files, lend, table, false) {
            Ok((_, made)) => made,
            Err(why) => {
                entry["problem"] = json!(why);
                return Ok(entry);
            }
        };
        let provider = &made.provider;
        match provider.describe() {
            Ok(info) => {
                let described = json_of(&info)?;
                if let (Some(into), Value::Object(from)) = (entry.as_object_mut(), described) {
                    for (key, value) in from {
                        into.insert(key, value);
                    }
                }
            }
            Err(error) => {
                entry["problem"] = json!(error.to_string());
                return Ok(entry);
            }
        }
        if let Some(title) = table.get("title").and_then(Value::as_str) {
            entry["title"] = json!(title);
        }
        match self.voices_of(name, table, provider, language) {
            Ok(voices) => entry["voices"] = voices,
            Err(why) => {
                entry["voices"] = json!([]);
                entry["voices_problem"] = json!(why);
            }
        }
        Ok(entry)
    }

    /// The voices `provider` has, from what was listed lately when the table is the same.
    fn voices_of(
        &self,
        name: &str,
        table: &Value,
        provider: &Provider,
        language: Option<&str>,
    ) -> Result<Value, String> {
        let key = format!("{name}\u{0}{}", language.unwrap_or(""));
        {
            let kept = self.kept.lock().unwrap_or_else(PoisonError::into_inner);
            if let Some(kept) = kept.get(&key)
                && kept.table == *table
                && kept.at.elapsed() < VOICES_KEPT
            {
                return Ok(kept.answer.clone());
            }
        }
        let mut query = VoiceQuery::default();
        query.language = language.map(str::to_owned);
        let list = provider
            .voices(query, Self::spawner())
            .map_err(|error| error.to_string())?;
        let answer = json_of(&list).map_err(|(_, why)| why)?["voices"].clone();
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(
                key,
                Kept {
                    at: Instant::now(),
                    table: table.clone(),
                    answer: answer.clone(),
                },
            );
        Ok(answer)
    }

    fn say(
        &self,
        caller: &Caller,
        files: &Files,
        lend: &Lend<'_>,
        stores: Option<&Stores>,
        params: &Value,
    ) -> Outcome {
        let name = text_param(params, "provider")?.ok_or_else(|| bad("provider is needed"))?;
        let text = text_param(params, "text")?.ok_or_else(|| bad("text is needed"))?;
        if text.len() > MAX_TEXT {
            return Err((
                REFUSED,
                format!("a text is at most {MAX_TEXT} bytes; send it in parts"),
            ));
        }
        let all = self.all_tables(files, lend, stores)?;
        let table = all
            .get(name)
            .ok_or_else(|| (NOT_FOUND, format!("this host has no voice provider {name}")))?;
        let (_, made) = self
            .built(files, lend, table, true)
            .map_err(|why| (REFUSED, format!("{name}: {why}")))?;
        let (provider, code) = (&made.provider, made.code.clone());
        let mut request = SayRequest::new(text);
        request.voice = text_param(params, "voice")?.map(str::to_owned);
        request.model = text_param(params, "model")?.map(str::to_owned);
        request.style = text_param(params, "style")?.map(str::to_owned);
        request.language = text_param(params, "language")?.map(str::to_owned);
        request.speed = match params.get("speed") {
            None | Some(Value::Null) => None,
            Some(value) => Some(
                value
                    .as_f64()
                    .filter(|speed| speed.is_finite() && *speed > 0.0)
                    .ok_or_else(|| bad("speed is a number above 0, 1 being normal"))?,
            ),
        };
        let owner = caller.owner().to_owned();
        let stale = {
            let mut held = self.held();
            let stale = Self::reap(&mut held);
            if held.len() >= MAX_OPEN
                || held.values().filter(|open| open.owner == owner).count() >= MAX_PER_KEY
            {
                drop(held);
                end_all(stale);
                return Err((REFUSED, "too many utterances are open; stop one".to_owned()));
            }
            stale
        };
        end_all(stale);
        let utterance = provider
            .say(request, Self::spawner())
            .map_err(|error| (REFUSED, error.to_string()))?;
        let format = utterance
            .format()
            .map_err(|error| (REFUSED, error.to_string()))?;
        let id = format!("say-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        self.held().insert(
            id.clone(),
            Open {
                owner,
                utterance: Arc::new(utterance),
                last: Instant::now(),
                _code: code,
            },
        );
        Ok(json!({
            "utterance": id,
            "encoding": format.encoding,
            "rate": format.rate,
            "channels": format.channels,
        }))
    }

    /// Takes the utterances nobody has called for in [`IDLE`] out of `held`.
    fn reap(held: &mut BTreeMap<String, Open>) -> Vec<Open> {
        let stale: Vec<String> = held
            .iter()
            .filter(|(_, open)| open.last.elapsed() > IDLE)
            .map(|(id, _)| id.clone())
            .collect();
        stale.iter().filter_map(|id| held.remove(id)).collect()
    }

    /// The utterance `id` of this caller, its idle clock restarted.
    fn find(&self, caller: &Caller, params: &Value) -> Result<Found, (i32, String)> {
        let id = text_param(params, "utterance")?.ok_or_else(|| bad("utterance is needed"))?;
        let mut held = self.held();
        match held.get_mut(id) {
            Some(open) if caller.reaches(&open.owner) => {
                open.last = Instant::now();
                Ok((id.to_owned(), Arc::clone(&open.utterance)))
            }
            _ => Err((NOT_FOUND, "this host has no such utterance".to_owned())),
        }
    }

    /// `speech/audio`: the next audio of the utterance, base64, waiting up to `wait_ms` for some; `done` once it is over, and
    /// `error` when it ended in a failure, after the audio it made.
    fn audio(&self, caller: &Caller, params: &Value) -> Outcome {
        let (id, utterance) = self.find(caller, params)?;
        let number = |key: &str, default: u64, most: u64| -> Result<u64, (i32, String)> {
            match params.get(key) {
                None | Some(Value::Null) => Ok(default),
                Some(value) => value
                    .as_u64()
                    .map(|n| n.min(most))
                    .ok_or_else(|| bad(&format!("{key} is a number"))),
            }
        };
        let wait = number("wait_ms", DEFAULT_WAIT_MS, MAX_WAIT_MS)?;
        let limit = usize::try_from(number("max_bytes", DEFAULT_CHUNK as u64, MAX_CHUNK as u64)?)
            .unwrap_or(DEFAULT_CHUNK)
            .max(1);
        let mut audio = Vec::new();
        let mut chunk = vec![0_u8; limit.min(16 * 1024)];
        let (mut done, mut failure) = (false, None);
        let mut waiting = wait;
        while audio.len() < limit {
            let room = chunk.len().min(limit - audio.len());
            match utterance.read(&mut chunk[..room], waiting) {
                Ok(0) => {
                    done = true;
                    break;
                }
                Ok(READ_TIMED_OUT) => break,
                Ok(count) => {
                    let count = usize::try_from(count).unwrap_or(0).min(room);
                    audio.extend_from_slice(&chunk[..count]);
                    // What is there already is taken; what is not is not waited for again.
                    waiting = 0;
                }
                Err(error) => {
                    done = true;
                    failure = Some(error.to_string());
                    break;
                }
            }
        }
        if done {
            self.held().remove(&id);
            let _ = utterance.stop();
        }
        let mut answer = json!({"audio": data_encoding::BASE64.encode(&audio), "done": done});
        if let Some(why) = failure {
            answer["error"] = json!(why);
        }
        Ok(answer)
    }

    fn stop(&self, caller: &Caller, params: &Value) -> Outcome {
        let (id, utterance) = self.find(caller, params)?;
        self.held().remove(&id);
        let _ = utterance.stop();
        Ok(json!({}))
    }

    /// `speech/say_configure`: writes (or, with `remove`, takes out) the `[say.NAME]` table. Settings the kind's schema does not name
    /// are refused, and a secret the request leaves out or leaves empty keeps what the table had; `clear` names the ones to take out.
    fn configure(&self, files: &Files, lend: &Lend<'_>, params: &Value) -> Outcome {
        let name = text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?;
        if !valid_name(name) {
            return Err(bad(
                "a name is letters, digits, - and _, at most 40 characters",
            ));
        }
        if params.get("remove") == Some(&Value::Bool(true)) {
            files.hidden_update(SECTION, |current| {
                let mut map = current.as_object().cloned().unwrap_or_default();
                map.remove(name);
                Ok(if map.is_empty() {
                    Value::Null
                } else {
                    Value::Object(map)
                })
            })?;
            self.kept
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .clear();
            return Ok(json!({"name": name, "removed": true}));
        }
        let id = text_param(params, "provider")?.ok_or_else(|| bad("provider is needed"))?;
        // A provider of a table that names a library or a source of its own is found in that registry, so editing its settings keeps it.
        let existing = tables(files)?
            .get(name)
            .filter(|table| table.get("provider").and_then(Value::as_str) == Some(id))
            .cloned()
            .unwrap_or(Value::Null);
        let origin =
            speech_scripts::registry_of(&existing, base_of(files)).map_err(|why| (REFUSED, why))?;
        let kind = resolve(id, origin.as_ref()).map_err(|why| (NOT_FOUND, why))?;
        let settings = match params.get("settings") {
            None | Some(Value::Null) => Map::new(),
            Some(Value::Object(map)) => map.clone(),
            Some(_) => return Err(bad("settings is an object")),
        };
        let members = members_of(&kind.schema);
        let secrets = secrets_of(&kind.schema);
        if let Some(unknown) = settings.keys().find(|key| !members.contains(*key)) {
            return Err(bad(&format!("{unknown} is not a setting of {id}")));
        }
        let clear = text_list(params, "clear")?;
        files.hidden_update(SECTION, |current| {
            let mut map = current.as_object().cloned().unwrap_or_default();
            let before = map.get(name).cloned().unwrap_or(Value::Null);
            let mut table = Map::new();
            table.insert("provider".to_owned(), json!(id));
            for (key, value) in settings {
                let empty = value.is_null() || value.as_str().is_some_and(str::is_empty);
                if !(secrets.contains(&key) && empty) {
                    table.insert(key, value);
                }
            }
            if before.get("provider").and_then(Value::as_str) == Some(id) {
                // Where its code comes from stays when its settings are edited.
                for key in ORIGIN_KEYS {
                    if let Some(kept) = before.get(key) {
                        table.insert(key.to_owned(), kept.clone());
                    }
                }
                for key in &secrets {
                    if !table.contains_key(key)
                        && !clear.contains(key)
                        && let Some(kept) = before.get(key)
                    {
                        table.insert(key.clone(), kept.clone());
                    }
                }
            }
            let table = Value::Object(table);
            self.built(files, lend, &table, false)
                .map_err(|why| format!("those settings cannot be used: {why}"))?;
            map.insert(name.to_owned(), table);
            Ok(Value::Object(map))
        })?;
        self.kept
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        Ok(json!({"name": name, "provider": id, "saved": true}))
    }
}

fn end_all(stale: Vec<Open>) {
    for open in stale {
        let _ = open.utterance.stop();
    }
}

/// A model of the kind as JSON, the way a guatiao value writes it.
fn json_of<T: ToValue>(model: &T) -> Outcome {
    let value = model
        .to_value()
        .map_err(|error| (REFUSED, error.to_string()))?;
    serde_json::to_value(guatiao_serde::Serializable::from(&value))
        .map_err(|error| (REFUSED, error.to_string()))
}
