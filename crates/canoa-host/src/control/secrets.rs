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

/// `secrets/providers`, `secrets/list`, `secrets/reveal` and `secrets/forget`.
pub(crate) fn call(vault: &Arc<Vault>, method: &str, params: &Value) -> Outcome {
    match method {
        "secrets/providers" => Ok(json!({
            "providers": vault.providers().iter().map(|listed| listed.json()).collect::<Vec<_>>(),
            "entries": vault.entries().into_iter().map(|(name, entry)| json!({
                "name": name, "provider": entry.provider, "keep": entry.keep.id(),
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
