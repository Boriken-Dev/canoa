//! What the person does themselves with a session's delegation: ask what it may delegate to, start a task of their own, and
//! narrow it. A task they start is the same A2A task an agent's `delegate_task` makes, in the same session's store, so the
//! agent's `task_status` sees it and every bound of the session applies to it alike.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks are std's"
)]

use std::sync::Arc;

use serde_json::{Map, Value, json};

use super::agent::DelegationAgent;
use super::{Engine, Session, Source, lock, tools};

/// What the person narrowed a session's delegation to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Setup {
    /// The only sources its tasks may name; `None` leaves what the connection may use.
    pub(crate) sources: Option<Vec<String>>,
    /// The source a task with none named goes to.
    pub(crate) default: Option<String>,
    /// Tasks of the session running at once, at most; `None` leaves `max_children`.
    pub(crate) max_running: Option<usize>,
}

/// `_canoa/delegate/setup {sessionId, sources?, default?, maxRunning?}`: each key present replaces the setting, `null` clears
/// it. It only narrows: a source the connection may not use stays out whatever the list says.
pub(crate) fn setup(session: &Session, params: &Value) -> Result<Value, String> {
    let mut held = lock(&session.setup);
    let mut next = held.clone();
    if let Some(sources) = params.get("sources") {
        next.sources = match sources {
            Value::Null => None,
            Value::Array(names) => Some(
                names
                    .iter()
                    .map(|name| {
                        name.as_str()
                            .map(str::to_owned)
                            .ok_or("sources are names, as text")
                    })
                    .collect::<Result<_, _>>()?,
            ),
            _ => return Err("sources is a list of names".to_owned()),
        };
    }
    if let Some(default) = params.get("default") {
        next.default = match default {
            Value::Null => None,
            Value::String(name) => Some(name.clone()),
            _ => return Err("default is a name, as text".to_owned()),
        };
    }
    if let Some(limit) = params.get("maxRunning") {
        next.max_running = match limit {
            Value::Null => None,
            other => Some(
                other
                    .as_u64()
                    .and_then(|limit| usize::try_from(limit).ok())
                    .ok_or("maxRunning is a number of tasks")?,
            ),
        };
    }
    *held = next;
    Ok(json!({}))
}

/// `_canoa/delegate/start {sessionId, source?, task, requestId?, folder?, workspace?, title?, model?, effort?, account?,
/// profile?, loadout?, context?}`: `{taskId, state}`. A request sent again with the `requestId` of one that was made finds that
/// task and makes none, so a client that lost its answer, or its link, never starts the work twice.
pub(crate) fn start(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    params: &Value,
) -> Result<Value, String> {
    let request = match params.get("requestId") {
        None | Some(Value::Null) => None,
        Some(Value::String(id)) if !id.is_empty() => Some(id.clone()),
        Some(_) => return Err("requestId is text".to_owned()),
    };
    // One start at a time: a request sent again while its first is under way waits for it and then finds its task.
    let _one = lock(&session.starting);
    if let Some(task) = request
        .as_ref()
        .and_then(|id| lock(&session.submitted).get(id).cloned())
    {
        let mut said = tools::task_of(engine, session, &task)?;
        said["repeated"] = json!(true);
        return Ok(said);
    }
    let mut args = params.clone();
    if let Some(args) = args.as_object_mut() {
        args.remove("sessionId");
        args.remove("requestId");
    }
    let made = tools::delegate(engine, session, &args)?;
    if let (Some(request), Some(task)) = (request, made.get("taskId").and_then(Value::as_str)) {
        lock(&session.submitted).insert(request, task.to_owned());
    }
    Ok(made)
}

/// `_canoa/delegate/targets {sessionId, source?}`: what the session may delegate to. With a `source`, the models it lists too.
pub(crate) fn targets(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    params: &Value,
) -> Result<Value, String> {
    let only = match params.get("source") {
        None | Some(Value::Null) => None,
        Some(Value::String(name)) => Some(name.as_str()),
        Some(_) => return Err("source is a name, as text".to_owned()),
    };
    listing(engine, session, only)
}

/// What `session` may delegate to: each source with where it runs, its accounts, whether it is ready and what a workspace is
/// for it; never a credential, and no account its connection may not use. With `only`, that source's models as well.
pub(crate) fn listing(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    only: Option<&str>,
) -> Result<Value, String> {
    let allowed = DelegationAgent::allowed(engine, session);
    let who = session
        .conn()
        .map(|conn| conn.principal().clone())
        .ok_or("this session is not open yet")?;
    if let Some(name) = only
        && !allowed.iter().any(|source| source.name == name)
    {
        return Err(format!("{name} is not a source this session may use"));
    }
    let shown: Vec<&Source> = allowed
        .iter()
        .filter(|source| only.is_none_or(|name| source.name == name))
        .collect();
    let mut targets = Vec::new();
    for source in shown {
        let mut target = Map::new();
        target.insert("source".to_owned(), json!(source.name));
        target.insert(
            "title".to_owned(),
            json!(source.title.as_ref().unwrap_or(&source.name)),
        );
        if let Some(harness) = &source.harness {
            target.insert("harness".to_owned(), json!(harness));
        }
        target.insert(
            "host".to_owned(),
            json!(if source.remote.is_some() {
                "another machine"
            } else {
                "this host"
            }),
        );
        target.insert("remote".to_owned(), json!(source.remote.is_some()));
        target.insert("accounts".to_owned(), json!(source.accounts));
        target.insert("ready".to_owned(), json!(source.missing.is_empty()));
        if !source.missing.is_empty() {
            target.insert("missing".to_owned(), json!(source.missing));
        }
        let workspace = if source.remote.is_some() {
            json!({"kind": "required", "where": "the other machine",
                "note": "name a folder there; this session's own folder is never sent"})
        } else {
            json!({"kind": "folder", "default": session.cwd.to_string_lossy(),
                "note": "this session's folder or one below it"})
        };
        target.insert("workspace".to_owned(), workspace);
        if only.is_some() && source.remote.is_none() {
            let account = None;
            let host = engine.host().ok_or("the host is stopping")?;
            let choices = host.choices(&source.name, &who, account);
            target.insert(
                "models".to_owned(),
                choices.map_or_else(
                    || json!({"known": false}),
                    |choices| {
                        json!({
                            "known": choices.get("known").cloned().unwrap_or(json!(false)),
                            "list": choices.get("models").cloned().unwrap_or(json!([])),
                        })
                    },
                ),
            );
        }
        targets.push(Value::Object(target));
    }
    let setup = lock(&session.setup).clone();
    let mut out = Map::new();
    out.insert("targets".to_owned(), Value::Array(targets));
    if let Some(default) = setup.default {
        out.insert("default".to_owned(), json!(default));
    }
    if let Some(limit) = setup.max_running {
        out.insert("maxRunning".to_owned(), json!(limit));
    }
    Ok(Value::Object(out))
}
