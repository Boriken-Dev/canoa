//! The host as the ACP client of a session no one is attached to: a session that moved to another member and goes on with the
//! continuation. It asks for what the role chose (account, profile, model, effort), loads the session the import made, sends one
//! prompt and reads until the turn ends. A question the agent puts to a person is answered as cancelled: no one is there to answer it,
//! and nothing is allowed on the person's behalf.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the link's reads wait on the clock"
)]

use std::time::{Duration, Instant};

use serde_json::{Value, json};

use crate::local::{Link, LinkError};

/// How long one read waits before the turn's deadline is looked at.
const POLL: Duration = Duration::from_millis(200);

/// What the agent was asked for, under the keys the entry's host reads.
#[derive(Clone, Debug, Default)]
pub(crate) struct Wanted {
    pub(crate) account: Option<String>,
    pub(crate) profile: Option<String>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
}

fn send(link: &Link, message: &Value) -> Result<(), String> {
    link.send(message.to_string().as_bytes())
        .map_err(|error| error.to_string())
}

/// Sends a request and reads until its answer, answering what the agent asks meanwhile. A turn that runs past `deadline` is given up.
fn call(
    link: &Link,
    next: &mut u64,
    (method, params): (&str, Value),
    deadline: Instant,
) -> Result<Value, String> {
    *next += 1;
    let id = *next;
    send(
        link,
        &json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params}),
    )?;
    loop {
        if Instant::now() > deadline {
            return Err(format!("{method} did not end in time"));
        }
        match link.receive(Some(POLL)) {
            Ok(Some(bytes)) => {
                let Ok(message) = serde_json::from_slice::<Value>(&bytes) else {
                    continue;
                };
                if let Some(asked) = message.get("method").and_then(Value::as_str) {
                    if let Some(question) = message.get("id") {
                        let answer = if asked == "session/request_permission" {
                            json!({"jsonrpc": "2.0", "id": question, "result": {"outcome": {"outcome": "cancelled"}}})
                        } else {
                            json!({"jsonrpc": "2.0", "id": question, "error": {"code": -32601, "message": "no one is attached to answer"}})
                        };
                        send(link, &answer)?;
                    }
                } else if message.get("id") == Some(&json!(id)) {
                    return match message.get("error") {
                        Some(error) => Err(error
                            .get("message")
                            .and_then(Value::as_str)
                            .unwrap_or("an error")
                            .to_owned()),
                        None => Ok(message.get("result").cloned().unwrap_or(Value::Null)),
                    };
                }
            }
            Ok(None) | Err(LinkError::Ended | LinkError::TooLong { .. }) => {
                return Err("the agent's link ended".to_owned());
            }
            Err(LinkError::Timeout) => {}
        }
    }
}

/// The value of the config option of `category` in the answer to a session request, and the values it offers.
fn option_of<'a>(opened: &'a Value, category: &str) -> Option<(&'a str, Vec<&'a str>)> {
    let option = opened
        .get("configOptions")?
        .as_array()?
        .iter()
        .find(|option| option.get("category").and_then(Value::as_str) == Some(category))?;
    let values = option
        .get("options")?
        .as_array()?
        .iter()
        .flat_map(|each| -> Vec<&str> {
            match each.get("options").and_then(Value::as_array) {
                Some(group) => group
                    .iter()
                    .filter_map(|item| item.get("value")?.as_str())
                    .collect(),
                None => each
                    .get("value")
                    .and_then(Value::as_str)
                    .into_iter()
                    .collect(),
            }
        })
        .collect();
    Some((option.get("id")?.as_str()?, values))
}

/// Loads `session` (the import's) in `cwd` on the agent behind `link`, asked for what `wanted` names, sends `prompt` and answers the
/// turn's stop reason.
///
/// # Errors
///
/// What the agent refused, or that the link ended, or that the turn ran past `limit`.
pub(crate) fn resume(
    link: &Link,
    (session, cwd): (&str, &str),
    wanted: &Wanted,
    prompt: Vec<Value>,
    limit: Duration,
) -> Result<String, String> {
    let deadline = Instant::now() + limit;
    let mut next = 0;
    let mut meta = serde_json::Map::new();
    for (key, value) in [
        (crate::ACCOUNT_META, &wanted.account),
        (crate::profile::hosted::PROFILE_META, &wanted.profile),
        (crate::MODEL_META, &wanted.model),
    ] {
        if let Some(value) = value {
            meta.insert(key.to_owned(), json!(value));
        }
    }
    let mut initialize = json!({
        "protocolVersion": 1,
        "clientCapabilities": {},
        "clientInfo": {"name": "canoa-roles", "version": env!("CARGO_PKG_VERSION")},
    });
    if !meta.is_empty() {
        initialize["_meta"] = Value::Object(meta);
    }
    call(link, &mut next, ("initialize", initialize), deadline)?;
    let opened = call(
        link,
        &mut next,
        (
            "session/load",
            json!({"sessionId": session, "cwd": cwd, "mcpServers": []}),
        ),
        deadline,
    )?;
    if let Some(effort) = &wanted.effort
        && let Some((id, values)) = option_of(&opened, "thought_level")
        && values.contains(&effort.as_str())
    {
        call(
            link,
            &mut next,
            (
                "session/set_config_option",
                json!({"sessionId": session, "configId": id, "value": effort}),
            ),
            deadline,
        )?;
    }
    let turn = call(
        link,
        &mut next,
        (
            "session/prompt",
            json!({"sessionId": session, "prompt": prompt}),
        ),
        deadline,
    )?;
    Ok(turn
        .get("stopReason")
        .and_then(Value::as_str)
        .unwrap_or("end_turn")
        .to_owned())
}
