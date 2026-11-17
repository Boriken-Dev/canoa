//! `speech/*`: speech providers the host runs for a client that captures audio but cannot recognize it, which is every page and
//! every platform without a service of its own. A provider is a `[speech.NAME]` table of canoa.cfg, naming one of the kinds this
//! host has (`provider = "speech_command"` or `"speech_endpoint"`) and its settings, checked against that kind's schema.
//!
//! The tables are read when a method is called, so a change in the file takes effect at once. They are not a section `config/set`
//! edits: a table names a program to run or an address audio goes to, and may hold a key, so `speech/configure` writes them under
//! its own grant and no read ever answers a key.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "clocks and locks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, Instant};

use canoa::speech::builtin::{CommandSettings, CommandSpeech, EndpointSettings, EndpointSpeech};
use canoa::speech::{
    AudioFormat, OpenRequest, PCM_S16LE, SpeechProvider, SpeechSession, VoiceProfile,
};
use coqui_acp_api::ProcessSpawner as _;
use coqui_acp_api::guatiao::library::Object;
use coqui_acp_api::guatiao::{Alloc, FromValue, Schema};
use guatiao_intake::Screen;
use serde_json::{Map, Value, json};

use super::files::Files;
use super::speech_offers;
use super::{BAD_PARAMS, Caller, Grant, NOT_FOUND, REFUSED, allow, bad, text_param};

type Outcome = Result<Value, (i32, String)>;

/// A recording's id and the session it runs.
type Found = (String, Arc<Object<dyn SpeechSession>>);

/// The table of canoa.cfg the providers are in.
const SECTION: &str = "speech";

/// The most audio one `speech/push` carries, in bytes before base64: about half a minute of 16 kHz mono, far more than a page
/// sends at once.
pub const MAX_CHUNK: usize = 1024 * 1024;
/// The recordings one key may have open at once.
pub const MAX_PER_KEY: usize = 4;
/// The recordings open at once, whoever opened them.
pub const MAX_OPEN: usize = 16;
/// How long a recording may go with no call before the host ends it, so a page that closed leaves no program running.
pub const IDLE: Duration = Duration::from_secs(120);
/// The longest name of a provider.
const MAX_NAME: usize = 40;

/// One kind of provider this host has.
struct Kind {
    id: &'static str,
    title: &'static str,
    description: &'static str,
    /// Builds one from its settings, as canoa.cfg holds them.
    build: fn(&Value) -> Result<Arc<dyn SpeechProvider>, String>,
    schema: fn() -> Result<Value, String>,
    form: fn() -> Result<Value, String>,
}

fn kind_of<S, P>(settings: &Value) -> Result<Arc<dyn SpeechProvider>, String>
where
    S: FromValue,
    P: SpeechProvider + From<S> + 'static,
{
    let value = guatiao_serde::adapt::to_value_in(settings, Alloc::rust())
        .map_err(|_| "the settings cannot be read".to_owned())?;
    let settings = S::from_value(&value).map_err(|error| error.to_string())?;
    Ok(Arc::new(P::from(settings)))
}

fn schema_of<S: Schema>() -> Result<Value, String> {
    let value = S::schema().map_err(|error| error.to_string())?;
    serde_json::to_value(guatiao_serde::Serializable::from(&value)).map_err(|e| e.to_string())
}

fn form_of<S: Screen>() -> Result<Value, String> {
    let value = S::form(Alloc::rust()).map_err(|error| error.to_string())?;
    serde_json::to_value(guatiao_serde::Serializable::from(&value)).map_err(|e| e.to_string())
}

const KINDS: [Kind; 2] = [
    Kind {
        id: "speech_command",
        title: "Command",
        description: "Runs a program that reads audio and writes text",
        build: kind_of::<CommandSettings, CommandSpeech>,
        schema: schema_of::<CommandSettings>,
        form: form_of::<CommandSettings>,
    },
    Kind {
        id: "speech_endpoint",
        title: "Endpoint",
        description: "Sends the audio to a speech-to-text address and model you name",
        build: kind_of::<EndpointSettings, EndpointSpeech>,
        schema: schema_of::<EndpointSettings>,
        form: form_of::<EndpointSettings>,
    },
];

fn kind(id: &str) -> Option<&'static Kind> {
    KINDS.iter().find(|kind| kind.id == id)
}

/// The settings a schema marks as secrets: a read never answers them.
fn secrets_of(schema: &Value) -> BTreeSet<String> {
    schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .filter(|(_, field)| field["writeOnly"] == true || field["x-sensitive"] == true)
        .map(|(key, _)| key.clone())
        .collect()
}

fn members_of(schema: &Value) -> BTreeSet<String> {
    schema["properties"]
        .as_object()
        .into_iter()
        .flatten()
        .map(|(key, _)| key.clone())
        .collect()
}

/// One recording a client has open.
struct Open {
    owner: String,
    session: Arc<Object<dyn SpeechSession>>,
    last: Instant,
}

/// The recordings the host has open for clients.
#[derive(Default)]
pub(crate) struct Speech {
    open: Mutex<BTreeMap<String, Open>>,
    next: AtomicU64,
}

impl core::fmt::Debug for Speech {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Speech").finish_non_exhaustive()
    }
}

pub(super) fn valid_name(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= MAX_NAME
        && name
            .bytes()
            .all(|b| b.is_ascii_alphanumeric() || b == b'_' || b == b'-')
}

/// The providers canoa.cfg defines, as name to its table.
fn tables(files: &Files) -> Result<Map<String, Value>, (i32, String)> {
    Ok(match files.hidden_section(SECTION)? {
        Some(Value::Object(map)) => map,
        _ => Map::new(),
    })
}

/// What one table says, built: its kind and the provider, or why it cannot be.
fn built(table: &Value) -> Result<(&'static Kind, Arc<dyn SpeechProvider>), String> {
    let id = table
        .get("provider")
        .and_then(Value::as_str)
        .ok_or("it names no provider")?;
    let kind = kind(id).ok_or_else(|| format!("this host has no provider {id}"))?;
    let mut settings = table.clone();
    if let Some(map) = settings.as_object_mut() {
        map.remove("provider");
        map.remove(speech_offers::OFFER_KEY);
    }
    (kind.build)(&settings).map(|provider| (kind, provider))
}

impl Speech {
    pub(crate) fn call(
        &self,
        caller: &Caller,
        files: &Files,
        method: &str,
        params: &Value,
    ) -> Outcome {
        match method {
            "speech/providers" => allow(caller, Grant::Voice).and_then(|()| providers(files)),
            "speech/configure" => {
                allow(caller, Grant::VoiceSetup).and_then(|()| configure(files, params))
            }
            "speech/open" => {
                allow(caller, Grant::Voice).and_then(|()| self.open(caller, files, params))
            }
            "speech/push" => allow(caller, Grant::Voice).and_then(|()| self.push(caller, params)),
            "speech/finish" => {
                allow(caller, Grant::Voice).and_then(|()| self.finish(caller, params))
            }
            "speech/cancel" => {
                allow(caller, Grant::Voice).and_then(|()| self.cancel(caller, params))
            }
            _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
        }
    }

    /// Makes every open recording look `by` older, so a test need not wait out [`IDLE`].
    #[cfg(test)]
    pub(crate) fn age(&self, by: Duration) {
        for open in self.held().values_mut() {
            open.last = open.last.checked_sub(by).unwrap_or(open.last);
        }
    }

    fn held(&self) -> MutexGuard<'_, BTreeMap<String, Open>> {
        self.open.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// Ends the recordings nobody has called for in [`IDLE`], and answers the rest, so a closed page leaves nothing running.
    fn reap(held: &mut BTreeMap<String, Open>) -> Vec<Open> {
        let stale: Vec<String> = held
            .iter()
            .filter(|(_, open)| open.last.elapsed() > IDLE)
            .map(|(id, _)| id.clone())
            .collect();
        stale.iter().filter_map(|id| held.remove(id)).collect()
    }

    fn open(&self, caller: &Caller, files: &Files, params: &Value) -> Outcome {
        let name = text_param(params, "provider")?.ok_or_else(|| bad("provider is needed"))?;
        let tables = tables(files)?;
        let table = tables.get(name).ok_or_else(|| {
            (
                NOT_FOUND,
                format!("this host has no speech provider {name}"),
            )
        })?;
        let table = &speech_offers::with_model(table, params)?;
        let (_, provider) = built(table).map_err(|why| (REFUSED, format!("{name}: {why}")))?;
        let request = request_of(params)?;
        let owner = caller.owner().to_owned();
        let stale = {
            let mut held = self.held();
            let stale = Self::reap(&mut held);
            if held.len() >= MAX_OPEN
                || held.values().filter(|open| open.owner == owner).count() >= MAX_PER_KEY
            {
                drop(held);
                end_all(stale);
                return Err((
                    REFUSED,
                    "too many recordings are open; finish one".to_owned(),
                ));
            }
            stale
        };
        end_all(stale);
        let info = provider
            .describe()
            .map_err(|error| (REFUSED, error.to_string()))?;
        // The host's own spawner starts any program, as it does for every program it runs for the person.
        let spawner = coqui_acp_harness::LocalSpawner::new().into_object();
        let session = provider
            .open(request, spawner)
            .map_err(|error| (REFUSED, error.to_string()))?;
        let id = format!("sp-{}", self.next.fetch_add(1, Ordering::Relaxed) + 1);
        self.held().insert(
            id.clone(),
            Open {
                owner,
                session: Arc::new(session),
                last: Instant::now(),
            },
        );
        Ok(json!({"session": id, "partials": info.partials}))
    }

    /// The recording `id` of this caller, its idle clock restarted.
    fn find(&self, caller: &Caller, params: &Value) -> Result<Found, (i32, String)> {
        let id = text_param(params, "session")?.ok_or_else(|| bad("session is needed"))?;
        let mut held = self.held();
        match held.get_mut(id) {
            Some(open) if caller.reaches(&open.owner) => {
                open.last = Instant::now();
                Ok((id.to_owned(), Arc::clone(&open.session)))
            }
            _ => Err((NOT_FOUND, "this host has no such recording".to_owned())),
        }
    }

    fn push(&self, caller: &Caller, params: &Value) -> Outcome {
        let (id, session) = self.find(caller, params)?;
        let text = text_param(params, "audio")?.ok_or_else(|| bad("audio is needed"))?;
        let audio = data_encoding::BASE64
            .decode(text.as_bytes())
            .map_err(|_| bad("audio is base64"))?;
        if audio.len() > MAX_CHUNK {
            return Err((
                REFUSED,
                format!("a chunk of audio is at most {MAX_CHUNK} bytes; send it in parts"),
            ));
        }
        // The call returns when the provider has taken the chunk; no lock is held meanwhile.
        match session.push(&audio) {
            Ok(heard) => Ok(json!({"partial": heard.partial})),
            Err(error) => {
                // A recording that failed is over: nothing more is started or kept for it.
                self.held().remove(&id);
                let _ = session.cancel();
                Err((REFUSED, error.to_string()))
            }
        }
    }

    fn finish(&self, caller: &Caller, params: &Value) -> Outcome {
        let (id, session) = self.find(caller, params)?;
        self.held().remove(&id);
        let transcript = session
            .finish()
            .map_err(|error| (REFUSED, error.to_string()))?;
        let front_matter =
            serde_json::to_value(guatiao_serde::Serializable::from(&transcript.front_matter))
                .map_err(|error| (REFUSED, error.to_string()))?;
        Ok(json!({"text": transcript.text, "front_matter": front_matter}))
    }

    fn cancel(&self, caller: &Caller, params: &Value) -> Outcome {
        let (id, session) = self.find(caller, params)?;
        self.held().remove(&id);
        let _ = session.cancel();
        Ok(json!({}))
    }
}

fn end_all(stale: Vec<Open>) {
    for open in stale {
        let _ = open.session.cancel();
    }
}

fn text_list(params: &Value, key: &str) -> Result<Vec<String>, (i32, String)> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(Vec::new()),
        Some(Value::Array(items)) => items
            .iter()
            .map(|item| {
                item.as_str()
                    .map(str::to_owned)
                    .ok_or_else(|| bad(&format!("{key} is a list of text")))
            })
            .collect(),
        Some(_) => Err(bad(&format!("{key} is a list of text"))),
    }
}

/// What a recording opens with, from `speech/open`'s parameters: the audio's format, then the person's own hints.
fn request_of(params: &Value) -> Result<OpenRequest, (i32, String)> {
    let encoding = text_param(params, "encoding")?.unwrap_or(PCM_S16LE);
    let rate = match params.get("rate") {
        None | Some(Value::Null) => 16_000,
        Some(value) => value
            .as_u64()
            .and_then(|rate| u32::try_from(rate).ok())
            .filter(|rate| *rate > 0)
            .ok_or_else(|| bad("rate is samples a second"))?,
    };
    let channels = match params.get("channels") {
        None | Some(Value::Null) => 1,
        Some(value) => value
            .as_u64()
            .and_then(|channels| u8::try_from(channels).ok())
            .filter(|channels| *channels > 0)
            .ok_or_else(|| bad("channels is a number from 1"))?,
    };
    let mut format = AudioFormat::new(encoding, rate);
    format.channels = channels;
    let mut request = OpenRequest::new(format);
    request.hints.language = text_param(params, "language")?.map(str::to_owned);
    request.hints.vocabulary = text_list(params, "vocabulary")?;
    if let Some(profile) = params.get("profile").filter(|profile| !profile.is_null()) {
        let mut made = VoiceProfile::default();
        made.accent = text_param(profile, "accent")?.map(str::to_owned);
        made.voice = text_param(profile, "voice")?.map(str::to_owned);
        if let Some(traits) = profile.get("traits").and_then(Value::as_object) {
            for (key, value) in traits {
                let value = value
                    .as_str()
                    .ok_or_else(|| (BAD_PARAMS, "a trait is text".to_owned()))?;
                made.traits.insert(key.clone(), value.to_owned());
            }
        }
        request.hints.profile = Some(made);
    }
    Ok(request)
}

/// `speech/providers`: the kinds this host has, each with the schema and the form its settings are drawn with, and the providers
/// canoa.cfg defines, each with what it says of itself. A setting that is a secret is never answered, only that it is set.
fn providers(files: &Files) -> Outcome {
    let mut types = Vec::new();
    for kind in &KINDS {
        types.push(json!({
            "id": kind.id,
            "title": kind.title,
            "description": kind.description,
            "schema": (kind.schema)().map_err(|why| (REFUSED, why))?,
            "form": (kind.form)().map_err(|why| (REFUSED, why))?,
        }));
    }
    let mut listed = Vec::new();
    for (name, table) in tables(files)? {
        let mut entry = json!({"name": name, "provider": table.get("provider")});
        let kind = table.get("provider").and_then(Value::as_str).and_then(kind);
        let secrets = match kind {
            Some(kind) => secrets_of(&(kind.schema)().map_err(|why| (REFUSED, why))?),
            None => BTreeSet::new(),
        };
        let mut settings = Map::new();
        let mut set = Vec::new();
        if let Some(map) = table.as_object() {
            for (key, value) in map
                .iter()
                .filter(|(key, _)| *key != "provider" && *key != speech_offers::OFFER_KEY)
            {
                if secrets.contains(key) {
                    set.push(key.clone());
                } else {
                    settings.insert(key.clone(), value.clone());
                }
            }
        }
        entry["models"] = json!(speech_offers::models_of(&Value::Object(settings.clone())));
        entry["offer"] = table
            .get(speech_offers::OFFER_KEY)
            .cloned()
            .unwrap_or(Value::Null);
        entry["settings"] = Value::Object(settings);
        entry["secrets_set"] = json!(set);
        match built(&table) {
            Ok((_, provider)) => match provider.describe() {
                Ok(info) => {
                    entry["title"] = json!(info.title);
                    entry["description"] = json!(info.description);
                    entry["encodings"] = json!(info.encodings);
                    entry["partials"] = json!(info.partials);
                    entry["max_seconds"] = json!(info.max_seconds);
                    entry["hints"] = json!(info.hints);
                }
                Err(error) => entry["problem"] = json!(error.to_string()),
            },
            Err(why) => entry["problem"] = json!(why),
        }
        listed.push(entry);
    }
    Ok(json!({"types": types, "providers": listed}))
}

/// `speech/configure`: writes (or, with `remove`, takes out) the `[speech.NAME]` table. Settings the kind's schema does not name
/// are refused, and a secret the request leaves out or leaves empty keeps what the table had, so a form that never showed it
/// does not clear it; `clear` names the ones to take out.
fn configure(files: &Files, params: &Value) -> Outcome {
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
        return Ok(json!({"name": name, "removed": true}));
    }
    let id = text_param(params, "provider")?.ok_or_else(|| bad("provider is needed"))?;
    let kind = kind(id).ok_or_else(|| (NOT_FOUND, format!("this host has no provider {id}")))?;
    let settings = match params.get("settings") {
        None | Some(Value::Null) => Map::new(),
        Some(Value::Object(map)) => map.clone(),
        Some(_) => return Err(bad("settings is an object")),
    };
    let schema = (kind.schema)().map_err(|why| (REFUSED, why))?;
    let members = members_of(&schema);
    let secrets = secrets_of(&schema);
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
            // A provider made from an offer stays that offer's when its settings are edited.
            if let Some(offer) = before.get(speech_offers::OFFER_KEY) {
                table.insert(speech_offers::OFFER_KEY.to_owned(), offer.clone());
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
        built(&table).map_err(|why| format!("those settings cannot be used: {why}"))?;
        map.insert(name.to_owned(), table);
        Ok(Value::Object(map))
    })?;
    Ok(json!({"name": name, "provider": id, "saved": true}))
}
