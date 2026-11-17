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
use super::context::{self, Kind};
use super::{Engine, Session, Source, lock, tools};
use crate::profile::EffectiveDelegate;

/// What the person narrowed a session's delegation to.
#[derive(Clone, Debug, Default, PartialEq, Eq)]
pub(crate) struct Setup {
    /// The only sources its tasks may name; `None` leaves what the connection may use.
    pub(crate) sources: Option<Vec<String>>,
    /// The source a task with none named goes to.
    pub(crate) default: Option<String>,
    /// Tasks of the session running at once, at most; `None` leaves `max_children`.
    pub(crate) max_running: Option<usize>,
    /// What its agent may hand a task besides the task text, by its own call: nothing unless the person lets it. The person's own
    /// start never needs this, since they are shown what goes.
    pub(crate) context: Vec<Kind>,
    /// What the session's profile suggests, which the person's own setting above replaces key by key.
    pub(crate) hint: EffectiveDelegate,
}

impl Setup {
    /// The only sources its tasks may name: the person's, else the profile's suggestion.
    pub(crate) fn source_names(&self) -> Option<&Vec<String>> {
        self.sources
            .as_ref()
            .or(self.hint.sources.as_ref().map(|hint| &hint.value))
    }

    /// Tasks of the session running at once, at most: the person's, else the profile's suggestion. The host's own limits apply
    /// beside it, so a suggestion never raises one.
    pub(crate) fn running_limit(&self) -> Option<usize> {
        self.max_running.or_else(|| {
            self.hint
                .max_running
                .as_ref()
                .map(|hint| usize::try_from(hint.value).unwrap_or(usize::MAX))
        })
    }
}

/// The source a task with none named goes to: the person's, else the profile's when the connection may use it.
pub(crate) fn default_source(engine: &Engine, session: &Session) -> Option<String> {
    let setup = lock(&session.setup).clone();
    if let Some(default) = setup.default {
        return Some(default);
    }
    let hint = setup.hint.default?;
    DelegationAgent::allowed(engine, session)
        .iter()
        .any(|source| source.name == hint.value)
        .then_some(hint.value)
}

/// `_canoa/delegate/setup {sessionId, sources?, default?, maxRunning?, context?}`: each key present replaces the setting, `null` clears
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
    if let Some(kinds) = params.get("context") {
        next.context = match kinds {
            Value::Null => Vec::new(),
            Value::Array(names) => names
                .iter()
                .map(|name| {
                    name.as_str()
                        .and_then(Kind::parse)
                        .filter(|kind| *kind != Kind::Task)
                        .ok_or("context is a list of messages and files")
                })
                .collect::<Result<_, _>>()?,
            _ => return Err("context is a list of messages and files".to_owned()),
        };
    }
    *held = next;
    Ok(json!({}))
}

/// `_canoa/delegate/context {sessionId, kind}`: what handing `kind` on would give a task now, as the text it would be given and
/// what it is made of, with an `contextId` that a `start` names to be given exactly that.
pub(crate) fn context(
    engine: &Arc<Engine>,
    session: &Arc<Session>,
    params: &Value,
) -> Result<Value, String> {
    let kind = params
        .get("kind")
        .and_then(Value::as_str)
        .and_then(Kind::parse)
        .ok_or("kind is messages or files")?;
    let bundle = context::build(session, kind, engine.config.context_bytes)?;
    let mut secret = [0_u8; 16];
    getrandom::fill(&mut secret).map_err(|error| error.to_string())?;
    let id: String = secret.iter().map(|byte| format!("{byte:02x}")).collect();
    let mut shown = bundle.describe(true);
    shown["contextId"] = json!(id);
    let mut held = lock(&session.prepared);
    held.push((id, bundle));
    if held.len() > PREPARED {
        held.remove(0);
    }
    Ok(shown)
}

/// Previews a session keeps for a start to name.
const PREPARED: usize = 8;

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
        target.insert(
            "profiles".to_owned(),
            json!(
                source
                    .profiles
                    .iter()
                    .map(|(name, loadouts)| json!({"name": name, "loadouts": loadouts}))
                    .collect::<Vec<_>>()
            ),
        );
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
    // Where each value in effect comes from: `session` for the person's own setting, else the profile layer that suggested it.
    let mut from = Map::new();
    if let Some(default) = default_source(engine, session) {
        out.insert("default".to_owned(), json!(default));
        let own = setup.default.is_some();
        from.insert(
            "default".to_owned(),
            json!(if own {
                "session".to_owned()
            } else {
                setup
                    .hint
                    .default
                    .as_ref()
                    .map_or_else(String::new, |hint| hint.layer.to_string())
            }),
        );
    }
    if let Some(limit) = setup.running_limit() {
        out.insert("maxRunning".to_owned(), json!(limit));
        from.insert(
            "maxRunning".to_owned(),
            json!(match (&setup.max_running, &setup.hint.max_running) {
                (None, Some(hint)) => hint.layer.to_string(),
                _ => "session".to_owned(),
            }),
        );
    }
    if let Some(sources) = setup.source_names() {
        out.insert("sources".to_owned(), json!(sources));
        from.insert(
            "sources".to_owned(),
            json!(match (&setup.sources, &setup.hint.sources) {
                (None, Some(hint)) => hint.layer.to_string(),
                _ => "session".to_owned(),
            }),
        );
    }
    // What the profile suggested and did not take effect: a default the connection may not use.
    if let Some(hint) = &setup.hint.default
        && setup.default.is_none()
        && !from.contains_key("default")
    {
        out.insert(
            "unusedDefault".to_owned(),
            json!({"value": hint.value, "layer": hint.layer.to_string(),
                "why": "the connection may not use that source"}),
        );
    }
    out.insert("from".to_owned(), Value::Object(from));
    out.insert(
        "context".to_owned(),
        json!(
            setup
                .context
                .iter()
                .map(|kind| kind.name())
                .collect::<Vec<_>>()
        ),
    );
    out.insert(
        "contextBytes".to_owned(),
        json!(engine.config.context_bytes),
    );
    // Every source the connection may use, narrowed or not: what the setup chooses among.
    out.insert(
        "offered".to_owned(),
        json!(
            DelegationAgent::offered(engine, session)
                .iter()
                .map(|source| json!({"source": source.name, "title": source.title.as_ref().unwrap_or(&source.name)}))
                .collect::<Vec<_>>()
        ),
    );
    Ok(Value::Object(out))
}
