//! `secrets/*`: the person's own list of the secrets this host holds, under the `secrets` grant. The table shows where each is kept, what
//! it is called and how long it has left, never what it is; the value is read only when the person asks for it (`secrets/reveal`), and
//! a secret is typed only into a form sealed to the key that asked (`secrets/set`), so a typed value never crosses the link in the clear.
//!
//! No method lists values, and none is the agent's: an agent asks for a secret by name through its own tools and is answered with a
//! handle.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks and the clock are std's"
)]

use std::sync::Arc;
use std::time::{Duration, Instant};

use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use super::files::Files;
use super::{BAD_PARAMS, NOT_FOUND, REFUSED, error_line, text_param};
use crate::secrets::ask::{Asker, METHOD};
use crate::secrets::vault::Vault;

type Outcome = Result<Value, (i32, String)>;

const POLL: Duration = Duration::from_millis(50);

/// The provider entry and the secret's name a request names.
fn target(params: &Value) -> Result<(&str, &str), (i32, String)> {
    let provider = text_param(params, "provider")?
        .filter(|text| !text.is_empty())
        .ok_or((BAD_PARAMS, "provider is text".to_owned()))?;
    let name = text_param(params, "name")?
        .filter(|text| !text.is_empty())
        .ok_or((BAD_PARAMS, "name is text".to_owned()))?;
    if let Some(why) = canoa::secret_provider::name_problem(name) {
        return Err((BAD_PARAMS, why));
    }
    Ok((provider, name))
}

/// The section of canoa.cfg these methods write.
const SECTION: &str = "secrets";

/// `secrets/configure` `{"name", "provider", "settings"?, "from"?, "keep"?}` sets up the entry `name` of `[secrets.providers]`, or
/// `{"name", "remove": true}` takes it away. A setting is plain data; one the provider's schema marks secret is never written, and
/// is given instead as `from`: `{setting: {"entry", "name"}}`, the secret another entry gives (the form, for one the person types).
fn configure(vault: &Arc<Vault>, files: &Files, params: &Value) -> Outcome {
    let name = text_param(params, "name")?
        .filter(|text| !text.is_empty())
        .ok_or((BAD_PARAMS, "name is text".to_owned()))?;
    let remove = params.get("remove") == Some(&Value::Bool(true));
    let table = if remove {
        None
    } else {
        let provider = text_param(params, "provider")?
            .filter(|text| !text.is_empty())
            .ok_or((BAD_PARAMS, "provider is text".to_owned()))?;
        let mut table = serde_json::Map::new();
        table.insert("provider".to_owned(), json!(provider));
        if let Some(keep) = text_param(params, "keep")? {
            table.insert("keep".to_owned(), json!(keep));
        }
        match params.get("settings") {
            None | Some(Value::Null) => {}
            Some(Value::Object(settings)) => {
                for (key, value) in settings {
                    table.insert(key.clone(), value.clone());
                }
            }
            Some(_) => return Err((BAD_PARAMS, "settings is an object".to_owned())),
        }
        match params.get("from") {
            None | Some(Value::Null) => {}
            Some(from @ Value::Object(_)) => {
                table.insert("from".to_owned(), from.clone());
            }
            Some(_) => return Err((BAD_PARAMS, "from is an object".to_owned())),
        }
        Some(Value::Object(table))
    };
    files.hidden_update(SECTION, |current| {
        let mut section = current.as_object().cloned().unwrap_or_default();
        let mut providers = section
            .get("providers")
            .and_then(Value::as_object)
            .cloned()
            .unwrap_or_default();
        match table {
            Some(table) => {
                providers.insert(name.to_owned(), table);
            }
            None => {
                if providers.remove(name).is_none() {
                    return Err(format!("there is no provider entry {name} to remove"));
                }
            }
        }
        if providers.is_empty() {
            section.remove("providers");
        } else {
            section.insert("providers".to_owned(), Value::Object(providers));
        }
        let next = Value::Object(section);
        vault
            .check_section(&next)
            .map_err(|why| format!("those settings cannot be used: {why}"))?;
        Ok(if next.as_object().is_some_and(serde_json::Map::is_empty) {
            Value::Null
        } else {
            next
        })
    })?;
    Ok(json!({"name": name, "saved": !remove, "removed": remove}))
}

/// `secrets/providers`, `secrets/list`, `secrets/reveal`, `secrets/forget` and `secrets/configure`.
pub(crate) fn call(vault: &Arc<Vault>, files: &Files, method: &str, params: &Value) -> Outcome {
    match method {
        "secrets/configure" => configure(vault, files, params),
        "secrets/providers" => Ok(json!({
            "providers": vault.providers().iter().map(|listed| listed.json()).collect::<Vec<_>>(),
            "entries": vault.entries().into_iter().map(|(name, entry)| json!({
                "name": name, "provider": entry.provider, "keep": entry.keep.id(),
                "settings": entry.config,
                "from": entry.from.iter().map(|(setting, taken)| (
                    setting.clone(), json!({"entry": taken.entry, "name": taken.name}),
                )).collect::<serde_json::Map<String, Value>>(),
            })).collect::<Vec<_>>(),
        })),
        "secrets/list" => Ok(json!({
            "secrets": vault.rows().iter().map(crate::secrets::vault::Row::json).collect::<Vec<_>>(),
            "keeps": vault.keeps().into_iter().map(|keep| json!({"id": keep.id(), "label": keep.label()})).collect::<Vec<_>>(),
        })),
        "secrets/reveal" => {
            let (provider, name) = target(params)?;
            // The value is read for the person who asked, and goes nowhere else.
            vault
                .reveal(provider, name)
                .map(|value| json!({"value": value.as_str()}))
                .ok_or((
                    NOT_FOUND,
                    format!("{provider} holds no secret called {name}"),
                ))
        }
        "secrets/forget" => {
            let (provider, name) = target(params)?;
            vault
                .forget(provider, name)
                .map(|forgot| json!({"forgot": forgot}))
                .map_err(|why| (REFUSED, why))
        }
        _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
    }
}

/// The form `secrets/set` puts to the person: the value, and how long to keep it when the provider does not keep it itself.
fn form(vault: &Vault, provider: &str) -> Result<String, (i32, String)> {
    let keeps_itself = vault
        .entries()
        .into_iter()
        .find(|(entry, _)| entry == provider)
        .and_then(|(_, entry)| {
            vault
                .providers()
                .into_iter()
                .find(|each| each.id == entry.provider)
        })
        .ok_or((NOT_FOUND, format!("there is no provider entry {provider}")))?
        .info
        .can_store;
    let mut properties = serde_json::Map::new();
    properties.insert(
        "value".to_owned(),
        json!({"type": "string", "writeOnly": true, "title": "Value"}),
    );
    if !keeps_itself {
        let keeps: Vec<Value> = vault
            .keeps()
            .into_iter()
            .filter(|keep| keep.id() != "store")
            .map(|keep| json!({"const": keep.id(), "title": keep.label()}))
            .collect();
        let first = keeps
            .first()
            .and_then(|keep| keep["const"].as_str())
            .unwrap_or("session")
            .to_owned();
        properties.insert(
            "keep".to_owned(),
            json!({"type": "string", "title": "Keep it", "oneOf": keeps, "default": first}),
        );
    }
    Ok(json!({"type": "object", "properties": properties, "required": ["value"]}).to_string())
}

/// `secrets/set` on `channel`, which carries the form's request until the answer: the host asks the person for the value, sealed to the
/// key that asked, and keeps it in the provider entry's own store or in memory as the person chose. Other requests on the link are
/// answered as they come by `others`. A form declined, a link that ends, or no answer within `wait` sets nothing.
pub(crate) fn set<C: Channel>(
    vault: &Arc<Vault>,
    asker: Option<&Asker>,
    wait: Duration,
    request: &Value,
    channel: &C,
    others: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
) -> Vec<u8> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = request.get("params").cloned().unwrap_or(Value::Null);
    let started = (|| {
        let asker = asker.ok_or((
            REFUSED,
            "this host has no secret to seal a typed answer with, so it cannot ask".to_owned(),
        ))?;
        let (provider, name) = target(&params)?;
        let schema = form(vault, provider)?;
        let title = format!("Set the secret {name} in {provider}");
        let (asked_params, asked) = asker
            .ask("secrets", &title, &schema, wait)
            .map_err(|why| (REFUSED, why))?;
        let line = serde_json::to_vec(&json!({
            "jsonrpc": "2.0", "id": asked.request_id(), "method": METHOD, "params": asked_params,
        }))
        .map_err(|_| (REFUSED, "the form cannot be put to the person".to_owned()))?;
        Ok::<_, (i32, String)>((asker, provider.to_owned(), name.to_owned(), asked, line))
    })();
    let (asker, provider, name, asked, line) = match started {
        Ok(started) => started,
        Err((code, message)) => return error_line(&id, code, &message),
    };
    if channel.send(&line).is_err() {
        return error_line(&id, REFUSED, "the link ended");
    }
    let since = Instant::now();
    let reply = loop {
        if since.elapsed() > wait {
            return error_line(&id, REFUSED, "the person gave no answer");
        }
        match channel.receive(Some(POLL)) {
            Ok(Some(received)) => {
                let message: Value = serde_json::from_slice(&received).unwrap_or(Value::Null);
                if message.get("method").is_some() {
                    if let Some(answer) = others(&received)
                        && channel.send(&answer).is_err()
                    {
                        return error_line(&id, REFUSED, "the link ended");
                    }
                    continue;
                }
                if message.get("id").and_then(Value::as_str) == Some(asked.request_id()) {
                    break message;
                }
            }
            Err(Failure::Timeout) => {}
            Ok(None) | Err(_) => return error_line(&id, REFUSED, "the link ended"),
        }
    };
    let Some(result) = reply.get("result") else {
        return error_line(&id, REFUSED, "the person gave no answer");
    };
    let answer = match asker.open(asked, result) {
        Ok(Some(answer)) => answer,
        Ok(None) => return error_line(&id, REFUSED, "the person declined"),
        Err(why) => return error_line(&id, REFUSED, &why),
    };
    let Some(value) = answer.secrets.get("value") else {
        return error_line(&id, BAD_PARAMS, "the form came back with no value");
    };
    let keep = answer
        .plain
        .get("keep")
        .and_then(Value::as_str)
        .unwrap_or("session");
    match vault.set(&provider, &name, value.as_bytes(), keep) {
        Ok(()) => serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": {"set": true}}))
            .unwrap_or_default(),
        Err(why) => error_line(&id, REFUSED, &why),
    }
}

/// Whether the request is a call `secrets/set` answers.
pub(crate) fn is_set(method: &str) -> bool {
    method == "secrets/set"
}
