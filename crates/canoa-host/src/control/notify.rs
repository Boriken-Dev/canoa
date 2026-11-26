//! `notify/*`: who the host tells about what happens, and how. `notify/subscribe` and `notify/attended` belong to the window's link
//! (see [`Link`]); the others read the catalog of events and the notifiers, send a test notice, and keep a notifier's secrets.
//! Writes of the routes and of the entries go through `config/change`, which is how every other table is written.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "links and JSON answers are std's"
)]

use std::sync::{Arc, Mutex, PoisonError};

use canoa::config_store::Key;
use serde_json::{Map, Value, json};

use super::settings_sections::Keeper;
use super::{BAD_PARAMS, NOT_FOUND, REFUSED, bad, text_param};
use crate::notify::desktop::LinkState;
use crate::notify::directory::{Directory, secret_name};
use crate::notify::{Hub, events};

type Outcome = Result<Value, (i32, String)>;

/// The methods of a window's link that need the link itself: it is what the `desktop` notifier pushes notices on, and what says
/// which session the person is looking at.
pub(crate) fn is_link_method(method: &str) -> bool {
    matches!(method, "notify/subscribe" | "notify/attended")
}

/// What a link has asked of the notifications, ended with the link.
pub(crate) struct Link {
    hub: Option<Arc<Hub>>,
    state: Mutex<Option<Arc<LinkState>>>,
}

impl Link {
    pub(crate) fn new(hub: Option<Arc<Hub>>) -> Self {
        Self {
            hub,
            state: Mutex::new(None),
        }
    }

    /// `notify/subscribe` `{language?}` and `notify/attended` `{session?}`.
    pub(crate) fn call(&self, method: &str, params: &Value) -> Outcome {
        let hub = self.hub.as_ref().ok_or((
            REFUSED,
            "this host does not tell the person of events".to_owned(),
        ))?;
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        match method {
            "notify/subscribe" => {
                let language = text_param(params, "language")?.unwrap_or("en");
                match state.as_ref() {
                    Some(held) => {
                        *held.language.lock().unwrap_or_else(PoisonError::into_inner) =
                            language.to_owned();
                    }
                    None => *state = Some(hub.subscribe(language)),
                }
                Ok(json!({}))
            }
            _ => {
                let session = text_param(params, "session")?.map(str::to_owned);
                let held = state.as_ref().ok_or((
                    REFUSED,
                    "ask for notices with notify/subscribe first".to_owned(),
                ))?;
                *held.attended.lock().unwrap_or_else(PoisonError::into_inner) = session;
                Ok(json!({}))
            }
        }
    }

    /// The lines waiting to be sent on the link.
    pub(crate) fn pending(&self) -> Vec<Vec<u8>> {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .as_ref()
            .map(|state| state.take())
            .unwrap_or_default()
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        let state = self
            .state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let (Some(hub), Some(state)) = (&self.hub, state) {
            hub.unsubscribe(&state);
        }
    }
}

fn no_hub() -> (i32, String) {
    (
        REFUSED,
        "this host does not tell the person of events".to_owned(),
    )
}

/// A notifier as Settings shows it.
fn row(
    hub: &Hub,
    directory: &Directory,
    name: &str,
    keeper: Option<&Keeper<'_>>,
    layer: Option<&str>,
) -> Value {
    let entry = directory.entry(name);
    let provider = entry.map_or(events::DESKTOP, |entry| entry.provider.as_str());
    let offered = directory.provider(provider);
    let mut state = match directory.resolve(name) {
        Ok(_) => "ready".to_owned(),
        Err(why) => why,
    };
    if name == events::DESKTOP && !hub.has_window() {
        state = "no window is open".to_owned();
    }
    let schema = offered.map_or(Value::Null, |offered| offered.schema());
    let secrets = crate::forms::secrets_of(&schema);
    let set: Vec<&String> = secrets
        .iter()
        .filter(|member| {
            keeper.is_some_and(|keeper| {
                keeper
                    .get_named(&secret_name(name, member))
                    .ok()
                    .flatten()
                    .is_some()
            })
        })
        .collect();
    let settings: Map<String, Value> = entry
        .map(|entry| {
            entry
                .settings
                .as_map()
                .iter()
                .filter(|(key, _)| !secrets.contains(*key))
                .map(|(key, value)| (key.clone(), value.clone()))
                .collect()
        })
        .unwrap_or_default();
    json!({
        "name": name,
        "provider": provider,
        "title": offered.map_or(provider, |offered| offered.info.title.as_str()),
        "help": offered.and_then(|offered| offered.info.help.clone()),
        "replies": offered.is_some_and(|offered| offered.info.replies),
        "state": state,
        "origin": layer,
        "removable": entry.is_some() && layer.is_none_or(|layer| layer == "own"),
        "settings": settings,
        "secrets_set": set,
        "schema": schema,
        "form": offered.map_or(Value::Null, |offered| offered.form()),
    })
}

/// `{"notifiers": [...], "providers": [...]}`: every notifier by name (`desktop` first) and every provider an entry may choose.
pub(crate) fn notifiers(
    hub: Option<&Arc<Hub>>,
    store: Option<&dyn canoa::config_store::ConfigStore>,
    keeper: Option<&Keeper<'_>>,
) -> Outcome {
    let hub = hub.ok_or_else(no_hub)?;
    let directory = hub.directory().map_err(|why| (REFUSED, why))?;
    let rows: Vec<Value> = directory
        .names()
        .iter()
        .map(|name| {
            let layer = store
                .and_then(|store| {
                    store
                        .get(Key::new(["notifications", "notifiers", name.as_str()]))
                        .ok()
                })
                .and_then(|held| held.layer);
            row(hub, &directory, name, keeper, layer.as_deref())
        })
        .collect();
    let providers: Vec<Value> = directory
        .providers()
        .iter()
        .filter(|offered| offered.info.id != events::DESKTOP)
        .map(|offered| {
            json!({
                "id": offered.info.id, "title": offered.info.title, "help": offered.info.help,
                "replies": offered.info.replies, "origin": offered.origin,
                "schema": offered.schema(), "form": offered.form(),
            })
        })
        .collect();
    Ok(json!({"notifiers": rows, "providers": providers}))
}

/// `{"events": [...]}`: the catalog with the notifiers each event goes to.
pub(crate) fn events_of(hub: Option<&Arc<Hub>>) -> Outcome {
    let hub = hub.ok_or_else(no_hub)?;
    let directory = hub.directory().map_err(|why| (REFUSED, why))?;
    Ok(crate::notify::directory::routes_json(&directory))
}

/// `notify/test` `{name}`: a notice of the words "Test" to one notifier. A refusal is an answer, not an error.
pub(crate) fn test(hub: Option<&Arc<Hub>>, params: &Value) -> Outcome {
    let hub = hub.ok_or_else(no_hub)?;
    let name = text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?;
    if !hub
        .directory()
        .is_ok_and(|directory| directory.names().iter().any(|each| each == name))
    {
        return Err((NOT_FOUND, format!("this host has no notifier {name}")));
    }
    Ok(match hub.test(name) {
        Ok(()) => json!({"ok": true}),
        Err(why) => json!({"ok": false, "why": why}),
    })
}

/// `notify/keep` `{name, values?: {member: text}, clear?: [member]}`: keeps the sensitive members of the notifier entry `name`, which
/// canoa.cfg never holds, in the host's secrets.
pub(crate) fn keep(hub: Option<&Arc<Hub>>, keeper: &Keeper<'_>, params: &Value) -> Outcome {
    let hub = hub.ok_or_else(no_hub)?;
    let name = text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?;
    let directory = hub.directory().map_err(|why| (REFUSED, why))?;
    let entry = directory
        .entry(name)
        .ok_or_else(|| (NOT_FOUND, format!("this host has no notifier entry {name}")))?;
    let schema = directory
        .provider(&entry.provider)
        .map_or(Value::Null, |offered| offered.schema());
    let secrets = crate::forms::secrets_of(&schema);
    let values = match params.get("values") {
        Some(Value::Object(map)) => map.clone(),
        None | Some(Value::Null) => Map::new(),
        Some(_) => return Err(bad("values is an object")),
    };
    let clear: Vec<&str> = match params.get("clear") {
        Some(Value::Array(items)) => items.iter().filter_map(Value::as_str).collect(),
        _ => Vec::new(),
    };
    for member in values
        .keys()
        .map(String::as_str)
        .chain(clear.iter().copied())
    {
        if !secrets.contains(member) {
            return Err((
                BAD_PARAMS,
                format!("{member} is not a sensitive setting of {name}"),
            ));
        }
    }
    for (member, value) in &values {
        let Some(text) = value.as_str().filter(|text| !text.is_empty()) else {
            continue;
        };
        keeper
            .put_named(&secret_name(name, member), text)
            .map_err(|why| {
                (
                    REFUSED,
                    format!("{name}: the secret {member} was not kept: {why}"),
                )
            })?;
    }
    for member in clear {
        keeper
            .forget_named(&secret_name(name, member))
            .map_err(|why| {
                (
                    REFUSED,
                    format!("{name}: the secret {member} was not forgotten: {why}"),
                )
            })?;
    }
    Ok(json!({"name": name, "saved": true}))
}
