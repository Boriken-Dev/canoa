//! The program Store on the control channel: `store/list`, `store/report`, `store/latest`, `store/plan`, `store/apply`,
//! `store/rollback` and `store/remove`, over [`Programs`]. The params and results are the JSON of `canoa::store`'s models.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, channels and the clock are std's"
)]

use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError};
use std::sync::{Arc, Mutex};
use std::thread;
use std::time::{Duration, Instant};

use canoa::store::extension::{ExtensionType, StoreExtension};
use canoa::store::{Action, InstallPlan, ProgramStore, StoreSettings};
use coqui_acp_api::guatiao::{Alloc, FromValue, ToValue};
use coqui_acp_api::{LoginAnswer, LoginForm};
use coqui_transport::{Channel, Failure};
use serde_json::{Value, json};

use super::accounts::{open_answer, put_form};
use super::{BAD_PARAMS, NO_METHOD, POLL, REFUSED, error_line};
use crate::secrets::ask::{Asked, Asker};
use crate::store::programs::{Event, Programs, Sink, StoreError};
use crate::store::registry::BUILT_IN;
use crate::store::runtimes::RUNTIMES;
use crate::store::shelf::Shelf;

/// Nothing was found under that name.
const NOT_FOUND: i32 = -32003;

/// The stores this host offers, and the programs it keeps through them.
#[derive(Clone)]
pub(crate) struct Stores {
    programs: Arc<Programs>,
    providers: Vec<String>,
    /// The stores of the Runtimes type, listed by `store/list` only when asked for.
    runtimes: Vec<String>,
    /// The store types the extensions of canoa.cfg add.
    types: Vec<AddedType>,
}

/// A store type an extension adds, with the extension it comes from.
#[derive(Clone, Debug)]
struct AddedType {
    info: ExtensionType,
    extension: String,
    extension_title: String,
    publisher: String,
}

impl core::fmt::Debug for Stores {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stores")
            .field("providers", &self.providers)
            .field("runtimes", &self.runtimes)
            .finish_non_exhaustive()
    }
}

impl Stores {
    /// The programs `programs` keeps, through the stores named `providers`.
    pub(crate) fn new(programs: Arc<Programs>, providers: Vec<String>) -> Self {
        Self {
            programs,
            providers,
            runtimes: Vec::new(),
            types: Vec::new(),
        }
    }

    /// The same, also offering the store types the `extensions` of canoa.cfg add. Each extension's library is loaded into a
    /// registry of its own, so an id it offers is resolved from its file and a built-in store is never replaced. One that does
    /// not load, or that adds a type already taken or a store it does not offer, is skipped and logged: the others stay.
    pub(crate) fn extended(mut self, extensions: &[crate::config::Extension]) -> Self {
        for extension in extensions {
            if let Err(why) = self.load_extension(extension) {
                tracing::warn!(extension = extension.name.as_str(), "not loaded: {why}");
            }
        }
        self
    }

    fn load_extension(&mut self, extension: &crate::config::Extension) -> Result<(), String> {
        let mut registry =
            coqui_acp_api::guatiao::library::Registry::new("canoa", env!("CARGO_PKG_VERSION"));
        crate::provider::open(&mut registry, &extension.origin).map_err(|e| e.to_string())?;
        let offer = registry
            .offer::<dyn StoreExtension>(&extension.provider)
            .ok_or_else(|| format!("no provider {}", extension.provider))?
            .map_err(|_| format!("{} is not a store_extension", extension.provider))?;
        let info = offer
            .default_instance()
            .ok_or_else(|| "it needs settings".to_owned())?
            .describe()
            .map_err(|e| e.to_string())?;
        let mut added = Vec::new();
        for kind in info.types {
            if let Some(problem) = kind.problem() {
                return Err(problem);
            }
            if self
                .types
                .iter()
                .chain(&added)
                .any(|t: &AddedType| t.info.id == kind.id)
            {
                return Err(format!("the type {:?} is already added", kind.id));
            }
            for store in &kind.stores {
                let taken = self
                    .providers
                    .iter()
                    .chain(&self.runtimes)
                    .chain(self.types.iter().flat_map(|t| &t.info.stores))
                    .any(|id| id == store);
                if taken || registry.offer::<dyn ProgramStore>(store).is_none() {
                    return Err(format!(
                        "the store {store:?} is taken or not one this extension offers"
                    ));
                }
            }
            added.push(AddedType {
                info: kind,
                extension: extension.name.clone(),
                extension_title: info.title.clone(),
                publisher: info.publisher.clone(),
            });
        }
        drop(offer);
        self.programs.add_registry(Arc::new(registry));
        self.types.extend(added);
        Ok(())
    }

    /// The same, also keeping the runtimes the stores named `runtimes` install.
    pub(crate) fn with_runtimes(mut self, runtimes: Vec<String>) -> Self {
        self.runtimes = runtimes;
        self
    }

    /// The built-in stores over this user's programs folder.
    pub(crate) fn built_in() -> Result<Self, String> {
        let root = Shelf::default_root().ok_or("this user has no data folder")?;
        let shelf = Shelf::configured(root).map_err(|error| error.to_string())?;
        let registry = crate::store::registry::built_in()?;
        Ok(Self::new(
            Arc::new(Programs::new(shelf, registry)),
            BUILT_IN.iter().map(|id| (*id).to_owned()).collect(),
        )
        .with_runtimes(RUNTIMES.iter().map(|id| (*id).to_owned()).collect()))
    }
}

type Failed = (i32, String);

fn json_of<T: ToValue>(value: &T) -> Result<Value, Failed> {
    let value = value
        .to_value()
        .map_err(|error| (REFUSED, error.to_string()))?;
    serde_json::to_value(guatiao_serde::Serializable::from(&value))
        .map_err(|error| (REFUSED, error.to_string()))
}

fn from_json<T: FromValue>(value: &Value, what: &str) -> Result<T, Failed> {
    let value = guatiao_serde::adapt::to_value_in(value, Alloc::rust())
        .map_err(|_| (BAD_PARAMS, format!("{what} cannot be read")))?;
    T::from_value(&value).map_err(|error| (BAD_PARAMS, format!("{what}: {error}")))
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, Failed> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or_else(|| (BAD_PARAMS, format!("{key} is missing or not text")))
}

fn refusal(error: &StoreError) -> Failed {
    match error {
        StoreError::Unknown(_) => (NOT_FOUND, error.to_string()),
        StoreError::Settings(_) => (BAD_PARAMS, error.to_string()),
        _ => (REFUSED, error.to_string()),
    }
}

impl Stores {
    fn store(
        &self,
        params: &Value,
    ) -> Result<(String, Arc<dyn canoa::store::ProgramStore>), Failed> {
        let provider = text(params, "provider")?;
        if !self
            .providers
            .iter()
            .chain(&self.runtimes)
            .chain(self.types.iter().flat_map(|t| &t.info.stores))
            .any(|known| known == provider)
        {
            return Err((NOT_FOUND, format!("this host has no store {provider}")));
        }
        let store = self
            .programs
            .store(provider, &StoreSettings::default())
            .map_err(|error| refusal(&error))?;
        Ok((provider.to_owned(), store))
    }

    /// The answer to a read: `store/list`, `store/report`, `store/latest` or `store/plan`.
    pub(crate) fn read(
        &self,
        method: &str,
        params: &Value,
        owner: Option<&str>,
    ) -> Result<Value, Failed> {
        if method == "store/extensions" {
            let types: Vec<Value> = self
                .types
                .iter()
                .map(|t| {
                    json!({
                        "id": t.info.id, "title": t.info.title, "icon": t.info.icon,
                        "stores": t.info.stores, "extension": t.extension,
                        "extension_title": t.extension_title, "publisher": t.publisher,
                    })
                })
                .collect();
            return Ok(json!({"types": types}));
        }
        if method == "store/list" {
            // The Agents type's stores, with `{"kind": "runtimes"}` the Runtimes type's, or with
            // `{"kind": "extension", "type": ID}` those of a type an extension adds.
            let kind = params.get("kind").and_then(Value::as_str);
            let added;
            let of = match kind {
                None | Some("agents") => &self.providers,
                Some("runtimes") => &self.runtimes,
                Some("extension") => {
                    let id = text(params, "type")?;
                    added = self
                        .types
                        .iter()
                        .find(|t| t.info.id == id)
                        .ok_or_else(|| (NOT_FOUND, format!("no extension adds the type {id}")))?
                        .info
                        .stores
                        .clone();
                    &added
                }
                Some(_) => {
                    return Err((
                        BAD_PARAMS,
                        "kind is agents, runtimes or extension".to_owned(),
                    ));
                }
            };
            let ids: Vec<&str> = of.iter().map(String::as_str).collect();
            let listed = self.programs.list(&ids).map_err(|error| refusal(&error))?;
            let mut stores = Vec::new();
            for each in &listed {
                let mut info = json_of(&each.info)?;
                info["provider"] = Value::String(each.provider.clone());
                stores.push(info);
            }
            return Ok(json!({"stores": stores}));
        }
        let (provider, store) = self.store(params)?;
        match method {
            "store/report" => {
                let probe = params
                    .get("probe")
                    .and_then(Value::as_bool)
                    .unwrap_or(false);
                let pick = params.get("pick").and_then(Value::as_str);
                let report = self
                    .programs
                    .report(&*store, probe, pick)
                    .map_err(|error| refusal(&error))?;
                json_of(&report)
            }
            "store/latest" => {
                let channel = params
                    .get("channel")
                    .and_then(Value::as_str)
                    .map(str::to_owned);
                let latest = self
                    .programs
                    .latest(&*store, channel)
                    .map_err(|error| refusal(&error))?;
                json_of(&latest)
            }
            "store/plan" => {
                let action: Action = from_json(
                    &json!(text(params, "action")?),
                    "action (install, update or uninstall)",
                )?;
                let optional =
                    |key: &str| params.get(key).and_then(Value::as_str).map(str::to_owned);
                // A plan is kept for the caller that may apply it and for no other: a read-only key's is not kept, and one
                // caller's cannot replace or push out another one's.
                let plan = match owner {
                    Some(owner) => self.programs.plan_for(
                        owner,
                        &provider,
                        &*store,
                        action,
                        optional("channel"),
                        optional("version"),
                    ),
                    None => self.programs.plan_to_read(
                        &provider,
                        &*store,
                        action,
                        optional("channel"),
                        optional("version"),
                    ),
                }
                .map_err(|error| refusal(&error))?;
                json_of(&plan)
            }
            _ => Err((NO_METHOD, "this host has no such method".to_owned())),
        }
    }

    /// `store/rollback` and `store/remove`.
    pub(crate) fn change(&self, method: &str, params: &Value) -> Result<Value, Failed> {
        let (provider, store) = self.store(params)?;
        match method {
            "store/rollback" => {
                let current = self
                    .programs
                    .rollback(&*store)
                    .map_err(|error| refusal(&error))?;
                Ok(json!({"provider": provider, "version": current.version, "dir": current.dir}))
            }
            "store/remove" => {
                self.programs
                    .remove(&*store)
                    .map_err(|error| refusal(&error))?;
                Ok(json!({"provider": provider, "removed": true}))
            }
            _ => Err((NO_METHOD, "this host has no such method".to_owned())),
        }
    }
}

impl Stores {
    /// A program's plan for `action`, kept under `owner` alone so that only `owner` can apply it.
    pub(crate) fn plan_kept_for(
        &self,
        owner: &str,
        provider: &str,
        action: &str,
        version: Option<&str>,
    ) -> Result<Value, Failed> {
        let mut params = json!({"provider": provider, "action": action});
        if let Some(version) = version {
            params["version"] = json!(version);
        }
        self.read("store/plan", &params, Some(owner))
    }

    /// The plan kept for `owner` under `token`, as it would be applied.
    pub(crate) fn kept(&self, owner: &str, provider: &str, token: &str) -> Option<Value> {
        let plan = self.programs.kept_plan(owner, provider, token)?;
        json_of(&plan).ok()
    }

    /// Gives up the plan kept for `owner` under `token`.
    pub(crate) fn forget(&self, owner: &str, provider: &str, token: &str) {
        self.programs.forget_plan(owner, provider, token);
    }

    /// Whether `path` is on, in or holding a place of the protected list.
    pub(crate) fn protects(&self, path: &std::path::Path) -> bool {
        self.programs.protects_place(path)
    }

    /// The version the managed copy of `provider`'s program would go back to.
    pub(crate) fn previous_version(&self, provider: &str) -> Result<Option<String>, Failed> {
        let (_, store) = self.store(&json!({"provider": provider}))?;
        self.programs
            .previous_version(&*store)
            .map_err(|error| refusal(&error))
    }

    /// The version of the copy the host manages for `provider`'s program.
    pub(crate) fn managed_version(&self, provider: &str) -> Result<Option<String>, Failed> {
        let (_, store) = self.store(&json!({"provider": provider}))?;
        self.programs
            .managed_version(&*store)
            .map_err(|error| refusal(&error))
    }

    /// Carries out the plan kept for `owner` under `token` with no one to ask: an install that needs a typed answer fails.
    pub(crate) fn apply_kept(
        &self,
        owner: &str,
        provider: &str,
        token: &str,
        cancel: Arc<AtomicBool>,
    ) -> Result<Value, Failed> {
        let (provider, store) = self.store(&json!({"provider": provider}))?;
        let plan = self.programs.kept_plan(owner, &provider, token).ok_or((
            REFUSED,
            "this host holds no such plan any more: plan again".to_owned(),
        ))?;
        let done = self.programs.apply_store_for(
            owner,
            store,
            &provider,
            &plan,
            Arc::new(crate::store::programs::Recorder::default()),
            cancel,
        );
        applied(&done)
    }
}

/// Where the person's answer to a question goes.
type Reply = mpsc::Sender<Result<LoginAnswer, String>>;

/// The question put to the person, waiting for its answer.
struct Pending {
    asked: Asked,
    reply: Reply,
    since: Instant,
}

/// What a running apply hands the link that carries it.
enum Out {
    Event(Event),
    Ask(LoginForm, Reply),
    Done(Box<Result<Option<crate::store::programs::Applied>, StoreError>>),
}

struct LinkSink {
    out: Mutex<mpsc::Sender<Out>>,
}

impl Sink for LinkSink {
    fn event(&self, event: Event) {
        if let Ok(out) = self.out.lock() {
            let _ = out.send(Out::Event(event));
        }
    }

    fn ask(&self, form: LoginForm) -> Result<LoginAnswer, String> {
        let (reply, answer) = mpsc::channel();
        self.out
            .lock()
            .map_err(|_| "the link is gone".to_owned())?
            .send(Out::Ask(form, reply))
            .map_err(|_| "the link is gone".to_owned())?;
        // The link's loop answers every question, and does so once its wait is over.
        match answer.recv() {
            Ok(Ok(answer)) => Ok(answer),
            Ok(Err(why)) => Err(why),
            Err(_) => Err("the person gave no answer".to_owned()),
        }
    }
}

fn notification(request: &Value, event: &Event) -> Vec<u8> {
    let id = request.get("id").cloned().unwrap_or(Value::Null);
    let params = match event {
        Event::Say(line) => json!({"id": id, "text": line}),
        Event::Progress(progress) => {
            let mut params = json!({"id": id, "text": progress.label});
            if let Some(total) = progress.total.filter(|total| *total > 0) {
                #[allow(clippy::cast_precision_loss, reason = "a fraction for a bar")]
                let fraction = (progress.done as f64 / total as f64).clamp(0.0, 1.0);
                params["fraction"] = json!(fraction);
            }
            params
        }
    };
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": "store/progress", "params": params}))
        .unwrap_or_default()
}

/// What `store/apply` reports when it ends.
fn applied(
    done: &Result<Option<crate::store::programs::Applied>, StoreError>,
) -> Result<Value, Failed> {
    match done {
        Ok(Some(applied)) => Ok(json!({
            "applied": true,
            "version": applied.current.version,
            "dir": applied.current.dir,
            "lines": applied.lines,
        })),
        Ok(None) => Ok(json!({"applied": false})),
        Err(error) => Err(refusal(error)),
    }
}

impl Stores {
    /// `store/apply`: carries out a plan the host made and handed back, on `channel`, which carries `store/progress` and
    /// the form's request until the answer. A question goes to the person as the sign-in's does, sealed to the key that asked
    /// (`asker`), so a typed value never crosses the link in the clear; with no `asker` it is not asked. The link closing, or
    /// the person giving no answer within `wait`, a declined form or an answer bound to anything else cancels the install. Other
    /// requests on the link are answered as they come by `others`.
    pub(crate) fn apply<C: Channel>(
        &self,
        owner: &str,
        request: &Value,
        (channel, asker): (&C, Option<&Asker>),
        wait: Duration,
        others: &dyn Fn(&[u8]) -> Option<Vec<u8>>,
    ) -> Vec<u8> {
        let id = request.get("id").cloned().unwrap_or(Value::Null);
        let params = request.get("params").cloned().unwrap_or(Value::Null);
        let started = (|| {
            let (provider, store) = self.store(&params)?;
            let plan: InstallPlan = from_json(
                params
                    .get("plan")
                    .ok_or((BAD_PARAMS, "plan is missing".to_owned()))?,
                "plan",
            )?;
            Ok::<_, Failed>((provider, store, plan))
        })();
        let (provider, store, plan) = match started {
            Ok(started) => started,
            Err((code, message)) => return error_line(&id, code, &message),
        };
        let cancel = Arc::new(AtomicBool::new(false));
        let (out, events) = mpsc::channel();
        let sink = Arc::new(LinkSink {
            out: Mutex::new(out.clone()),
        });
        let programs = Arc::clone(&self.programs);
        let stop = Arc::clone(&cancel);
        let owner = owner.to_owned();
        let session = format!("store:{provider}");
        let worker = thread::spawn(move || {
            let done = programs.apply_store_for(&owner, store, &provider, &plan, sink, stop);
            let _ = out.send(Out::Done(Box::new(done)));
        });
        let mut asking: Option<Pending> = None;
        let answer = loop {
            match events.recv_timeout(POLL) {
                Ok(Out::Event(event)) => {
                    if channel.send(&notification(request, &event)).is_err() {
                        cancel.store(true, Ordering::SeqCst);
                    }
                }
                Ok(Out::Ask(form, reply)) => {
                    let put = match asker {
                        _ if asking.is_some() => Err("a question is already open".to_owned()),
                        Some(asker) => put_form(asker, &form, &session, wait),
                        None => Err(
                            "this host has no secret to seal a typed answer with, so it cannot ask"
                                .to_owned(),
                        ),
                    };
                    match put {
                        Ok((line, asked)) => {
                            if channel.send(&line).is_ok() {
                                asking = Some(Pending {
                                    asked,
                                    reply,
                                    since: Instant::now(),
                                });
                            } else {
                                cancel.store(true, Ordering::SeqCst);
                                let _ = reply.send(Err("the link ended".to_owned()));
                            }
                        }
                        Err(why) => {
                            let _ = reply.send(Err(why));
                        }
                    }
                }
                Ok(Out::Done(done)) => {
                    break match applied(&done) {
                        Ok(result) => serde_json::to_vec(
                            &json!({"jsonrpc": "2.0", "id": id, "result": result}),
                        )
                        .unwrap_or_default(),
                        Err((code, message)) => error_line(&id, code, &message),
                    };
                }
                Err(RecvTimeoutError::Timeout) => {}
                Err(RecvTimeoutError::Disconnected) => {
                    break error_line(&id, REFUSED, "the install ended without an answer");
                }
            }
            if asking
                .as_ref()
                .is_some_and(|pending| pending.since.elapsed() > wait)
                && let Some(pending) = asking.take()
            {
                cancel.store(true, Ordering::SeqCst);
                let _ = pending
                    .reply
                    .send(Err("the person gave no answer".to_owned()));
            }
            match channel.receive(Some(Duration::from_millis(1))) {
                Ok(Some(line)) => {
                    let message: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
                    let is_answer = message.get("method").is_none();
                    if is_answer {
                        let waiting = asking.take_if(|pending| {
                            message.get("id").and_then(Value::as_str)
                                == Some(pending.asked.request_id())
                        });
                        if let (Some(Pending { asked, reply, .. }), Some(asker)) = (waiting, asker)
                        {
                            let answer = match message.get("result") {
                                Some(result) => open_answer(asker, asked, result),
                                None => Err("the person gave no answer".to_owned()),
                            };
                            // A form declined, cancelled or answered under another binding ends the install, as the link closing does.
                            if answer.is_err() {
                                cancel.store(true, Ordering::SeqCst);
                            }
                            let _ = reply.send(answer);
                        }
                    } else if let Some(reply) = others(&line)
                        && channel.send(&reply).is_err()
                    {
                        cancel.store(true, Ordering::SeqCst);
                    }
                }
                Err(Failure::Timeout) => {}
                Ok(None) | Err(_) => {
                    cancel.store(true, Ordering::SeqCst);
                    if let Some(Pending { reply, .. }) = asking.take() {
                        let _ = reply.send(Err("the link ended".to_owned()));
                    }
                }
            }
        };
        let _ = worker.join();
        answer
    }
}

#[cfg(test)]
mod runtimes_tests {
    use super::*;

    fn stores() -> (Stores, std::path::PathBuf) {
        let root = std::env::temp_dir().join(format!("canoa-runtime-list-{}", std::process::id()));
        let programs = Programs::new(
            Shelf::unchecked(&root),
            crate::store::registry::built_in().expect("the built-in stores"),
        );
        let stores = Stores::new(
            Arc::new(programs),
            BUILT_IN.iter().map(|id| (*id).to_owned()).collect(),
        )
        .with_runtimes(RUNTIMES.iter().map(|id| (*id).to_owned()).collect());
        (stores, root)
    }

    fn programs_of(listed: &Value) -> Vec<(String, String)> {
        listed["stores"]
            .as_array()
            .expect("a list")
            .iter()
            .map(|store| {
                (
                    store["provider"].as_str().expect("provider").to_owned(),
                    store["program"].as_str().expect("program").to_owned(),
                )
            })
            .collect()
    }

    #[test]
    fn runtimes_are_listed_apart_from_agents_and_both_are_asked_through_the_same_calls() {
        let (stores, root) = stores();
        let agents = stores
            .read("store/list", &json!({}), None)
            .expect("the agents");
        let agents = programs_of(&agents);
        assert!(agents.iter().any(|(_, program)| program == "claude"));
        assert!(
            agents
                .iter()
                .all(|(_, program)| program != "node" && program != "python"),
            "the Agents type does not list runtimes: {agents:?}"
        );
        let runtimes = stores
            .read("store/list", &json!({"kind": "runtimes"}), None)
            .expect("the runtimes");
        assert_eq!(
            programs_of(&runtimes),
            [
                ("node_store".to_owned(), "node".to_owned()),
                ("python_store".to_owned(), "python".to_owned())
            ]
        );
        let bad = stores.read("store/list", &json!({"kind": "toys"}), None);
        assert_eq!(bad.expect_err("refused").0, BAD_PARAMS);
        // Nothing is installed: a report asks the shelf and the search path, and a runtime's store answers it.
        let report = stores.read("store/report", &json!({"provider": "node_store"}), None);
        assert!(report.is_ok(), "{report:?}");
        let unknown = stores.read("store/report", &json!({"provider": "bun_store"}), None);
        assert_eq!(unknown.expect_err("unknown").0, NOT_FOUND);
        let _ = std::fs::remove_dir_all(&root);
    }
}
