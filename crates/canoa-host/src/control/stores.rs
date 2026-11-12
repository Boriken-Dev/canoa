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
use std::time::Duration;

use canoa::store::{Action, InstallPlan, StoreSettings};
use coqui_acp_api::guatiao::{Alloc, FromValue, ToValue};
use coqui_acp_api::{LoginAnswer, LoginForm};
use coqui_web_channel::{Channel, Failure};
use serde_json::{Value, json};

use super::{BAD_PARAMS, NO_METHOD, POLL, REFUSED, error_line};
use crate::store::programs::{Event, Programs, Sink, StoreError};
use crate::store::registry::BUILT_IN;
use crate::store::shelf::Shelf;

/// Nothing was found under that name.
const NOT_FOUND: i32 = -32003;

/// How long a question put to the person waits for its answer.
const ASK_WAIT: Duration = Duration::from_secs(600);

/// The stores this host offers, and the programs it keeps through them.
#[derive(Clone)]
pub(crate) struct Stores {
    programs: Arc<Programs>,
    providers: Vec<String>,
}

impl core::fmt::Debug for Stores {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Stores")
            .field("providers", &self.providers)
            .finish_non_exhaustive()
    }
}

impl Stores {
    /// The programs `programs` keeps, through the stores named `providers`.
    pub(crate) fn new(programs: Arc<Programs>, providers: Vec<String>) -> Self {
        Self {
            programs,
            providers,
        }
    }

    /// The built-in stores over this user's programs folder.
    pub(crate) fn built_in() -> Result<Self, String> {
        let root = Shelf::default_root().ok_or("this user has no data folder")?;
        let shelf = Shelf::configured(root).map_err(|error| error.to_string())?;
        let registry = crate::store::registry::built_in()?;
        Ok(Self::new(
            Arc::new(Programs::new(shelf, registry)),
            BUILT_IN.iter().map(|id| (*id).to_owned()).collect(),
        ))
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
        if !self.providers.iter().any(|known| known == provider) {
            return Err((NOT_FOUND, format!("this host has no store {provider}")));
        }
        let store = self
            .programs
            .store(provider, &StoreSettings::default())
            .map_err(|error| refusal(&error))?;
        Ok((provider.to_owned(), store))
    }

    /// The answer to a read: `store/list`, `store/report`, `store/latest` or `store/plan`.
    pub(crate) fn read(&self, method: &str, params: &Value, owner: bool) -> Result<Value, Failed> {
        if method == "store/list" {
            let ids: Vec<&str> = self.providers.iter().map(String::as_str).collect();
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
                // Only the owner applies a plan, so a key's is not kept: it could not replace or push out the owner's.
                let make = if owner {
                    Programs::plan
                } else {
                    Programs::plan_to_read
                };
                let plan = make(
                    &self.programs,
                    &provider,
                    &*store,
                    action,
                    optional("channel"),
                    optional("version"),
                )
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

/// What a running apply hands the link that carries it.
enum Out {
    Event(Event),
    Ask(LoginForm, mpsc::Sender<Option<LoginAnswer>>),
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
        match answer.recv_timeout(ASK_WAIT) {
            Ok(Some(answer)) => Ok(answer),
            _ => Err("the person gave no answer".to_owned()),
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
    /// `store/ask` until the answer. The link closing, or the person giving no answer, cancels the install. Other requests on
    /// the link are answered as they come by `others`.
    pub(crate) fn apply<C: Channel>(
        &self,
        request: &Value,
        channel: &C,
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
        let worker = thread::spawn(move || {
            let done = programs.apply_store(store, &provider, &plan, sink, stop);
            let _ = out.send(Out::Done(Box::new(done)));
        });
        let mut asking: Option<(Value, mpsc::Sender<Option<LoginAnswer>>)> = None;
        let mut asked = 0_u32;
        let answer = loop {
            match events.recv_timeout(POLL) {
                Ok(Out::Event(event)) => {
                    if channel.send(&notification(request, &event)).is_err() {
                        cancel.store(true, Ordering::SeqCst);
                    }
                }
                Ok(Out::Ask(form, reply)) => {
                    asked += 1;
                    let ask_id = json!(format!("ask-{asked}"));
                    let ask = json!({
                        "jsonrpc": "2.0",
                        "id": ask_id,
                        "method": "store/ask",
                        "params": json_of(&form).unwrap_or(Value::Null),
                    });
                    let sent = serde_json::to_vec(&ask)
                        .ok()
                        .is_some_and(|bytes| channel.send(&bytes).is_ok());
                    if sent {
                        asking = Some((ask_id, reply));
                    } else {
                        cancel.store(true, Ordering::SeqCst);
                        let _ = reply.send(None);
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
            match channel.receive(Some(Duration::from_millis(1))) {
                Ok(Some(line)) => {
                    let message: Value = serde_json::from_slice(&line).unwrap_or(Value::Null);
                    let is_answer = message.get("method").is_none();
                    if is_answer {
                        if let Some((_, reply)) =
                            asking.take_if(|(ask_id, _)| message.get("id") == Some(ask_id))
                        {
                            let answer = message
                                .get("result")
                                .and_then(|result| from_json::<LoginAnswer>(result, "answer").ok());
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
                    if let Some((_, reply)) = asking.take() {
                        let _ = reply.send(None);
                    }
                }
            }
        };
        let _ = worker.join();
        answer
    }
}
