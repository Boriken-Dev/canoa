//! `kb/offers` and `kb/test`: what can do the host's own model work (the `assist` service) and a try of a choice before it is saved.
//! The Settings page that chooses the matcher and the ranker of knowledge search is drawn from them.
//!
//! `kb/offers` lists, for each operation, every backend that serves it with where its text goes, how the person's canoa.cfg orders
//! them (`[assist.OPERATION]` `use` and `fallback`) and the harnesses the person could offer. The models of one offer are asked only
//! for the offer a call names, since listing them is a request to an account's service, and come back already split by what the
//! operation needs: the models that report the capability, and the ones that report none, which a typed id may still name.
//!
//! `kb/test` sends a small built-in sample (never the person's knowledge) through the chosen backend and says what came back and how
//! long it took.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the control channel is std's"
)]

use std::time::{Duration, Instant};

use canoa::network::Network;
use canoa::sdk::{EmbedRequest, models_for};
use coqui_acp_api::ModelInfo;
use serde_json::{Value, json};

use super::accounts::Accounts;
use super::{Caller, Grant, REFUSED, allow, bad, text_param};
use crate::assist::{Assist, Backend, Operation};
use crate::config::{Config, DEFAULT_CHOICE};

type Outcome = Result<Value, (i32, String)>;

/// The operations, in the order the page shows them.
const OPERATIONS: [(Operation, &str); 3] = [
    (Operation::Complete, "Answer a prompt"),
    (Operation::Embed, "Match by meaning"),
    (Operation::Rerank, "Put the best first"),
];

/// The sample `kb/test` embeds.
const EMBED_SAMPLE: [&str; 2] = ["a canoe on a river", "bread baking in the kitchen"];

/// The sample `kb/test` orders: the second passage answers the query.
const RANK_QUERY: &str = "How do I paddle a canoe straight?";
const RANK_SAMPLE: [&str; 3] = [
    "Bread is baked in a hot oven until the crust is brown.",
    "Hold the paddle low and stroke close to the side of the canoe so it keeps straight.",
    "A compiler turns source code into a program.",
];

/// What `kb/test` waits for a backend, when the call names no time.
const TEST_SECONDS: u64 = 30;

/// `kb/offers` and `kb/test`.
pub(super) fn call(
    accounts: Option<&Accounts>,
    live: Option<&std::sync::Arc<Assist>>,
    caller: &Caller,
    method: &str,
    params: &Value,
) -> Outcome {
    allow(caller, Grant::Accounts)?;
    let accounts =
        accounts.ok_or_else(|| (REFUSED, "this host does not manage accounts".to_owned()))?;
    let config = accounts.config()?;
    let from = Some((accounts.layers(), accounts.home_dir()));
    // canoa.cfg as it is now, not as it was when the host started: the page edits it, and the harnesses go through this host.
    let assist = match live {
        Some(live) => live.fresh(&config, from),
        None => Assist::of(&config, from),
    };
    let outcome = match method {
        "kb/offers" => offers(&assist, &config, params),
        "kb/test" => test(&assist, params),
        _ => Err((super::NO_METHOD, "this host has no such method".to_owned())),
    };
    // What the call started is not kept: the sessions of a test end with it.
    assist.close();
    outcome
}

fn model_json(model: &ModelInfo) -> Value {
    let mut out = json!({"id": model.id, "capabilities": model.capabilities});
    if let Some(name) = &model.name {
        out["name"] = json!(name);
    }
    if let Some(description) = &model.description {
        out["description"] = json!(description);
    }
    if let Some(limits) = &model.limits {
        out["limits"] = json!({
            "context_tokens": limits.context_tokens,
            "max_output_tokens": limits.max_output_tokens,
            "dimensions": limits.dimensions,
            "dimensions_adjustable": limits.dimensions_adjustable,
        });
    }
    out
}

/// The models `backend` lists for `operation`, split by whether they report what it needs.
fn models_of(backend: &dyn Backend, operation: Operation) -> Value {
    match backend.models(&Network::current()) {
        Ok(listing) => {
            let split = models_for(&listing.models, operation.capability());
            json!({
                "listed": true,
                "offered": split.offered.iter().map(model_json).collect::<Vec<_>>(),
                "unreported": split.unreported.iter().map(model_json).collect::<Vec<_>>(),
            })
        }
        Err(failure) => json!({"listed": false, "why": failure.why}),
    }
}

/// `kb/offers` `{"offer"?}`.
pub(super) fn offers(assist: &Assist, config: &Config, params: &Value) -> Outcome {
    let wanted = text_param(params, "offer")?;
    let mut operations = Vec::new();
    for (operation, title) in OPERATIONS {
        let chosen = config.assist.operations.get(operation.name());
        let (asked, problem) = match assist.chain(operation, DEFAULT_CHOICE) {
            Ok(chain) => (
                chain
                    .iter()
                    .map(|backend| backend.id().to_owned())
                    .collect::<Vec<_>>(),
                None,
            ),
            Err(why) => (Vec::new(), Some(why)),
        };
        let offered: Vec<Value> = assist
            .backends()
            .iter()
            .filter(|backend| backend.serves(operation))
            .map(|backend| {
                let mut offer = json!({
                    "id": backend.id(),
                    "title": backend.title(),
                    "kind": backend.kind(),
                    "location": backend.location(),
                    "address": backend.address(),
                });
                if wanted == Some(backend.id()) {
                    offer["models"] = models_of(backend.as_ref(), operation);
                }
                offer
            })
            .collect();
        operations.push(json!({
            "id": operation.name(),
            "title": title,
            "capability": operation.capability(),
            "use": chosen.and_then(|each| each.choice.clone()),
            "fallback": chosen.map(|each| each.fallback.clone()).unwrap_or_default(),
            "asked": asked,
            "problem": problem,
            "offers": offered,
        }));
    }
    Ok(json!({
        "operations": operations,
        "harnesses": harnesses(config),
        "pool": {
            "idle_seconds": config
                .assist
                .pool
                .idle_seconds
                .unwrap_or(crate::assist::DEFAULT_IDLE.as_secs()),
            "max_sessions": config
                .assist
                .pool
                .max_sessions
                .unwrap_or(crate::assist::DEFAULT_SESSIONS),
        },
    }))
}

/// The agents the person could offer as a harness backend, and the profiles and accounts each has.
fn harnesses(config: &Config) -> Vec<Value> {
    config
        .entries
        .iter()
        .filter_map(|entry| {
            let (accounts, profiles) = match &entry.kind {
                crate::config::EntryKind::Harness {
                    accounts, profiles, ..
                } => (accounts.clone(), profiles.clone()),
                crate::config::EntryKind::Agent { profiles, .. } => (Vec::new(), profiles.clone()),
                _ => return None,
            };
            Some(json!({
                "source": entry.name,
                "accounts": accounts,
                "profiles": profiles,
            }))
        })
        .collect()
}

/// `kb/test` `{"role": "embedder" | "ranker", "use"?, "model", "dimensions"?, "timeout_seconds"?}`: the sample through the backend.
pub(super) fn test(assist: &std::sync::Arc<Assist>, params: &Value) -> Outcome {
    let role = text_param(params, "role")?.ok_or_else(|| bad("role is embedder or ranker"))?;
    let choice = text_param(params, "use")?.unwrap_or(DEFAULT_CHOICE);
    let model = text_param(params, "model")?
        .filter(|model| !model.trim().is_empty())
        .ok_or_else(|| bad("model is the model to try"))?;
    let timeout = Duration::from_secs(
        params
            .get("timeout_seconds")
            .and_then(Value::as_u64)
            .filter(|seconds| *seconds > 0)
            .unwrap_or(TEST_SECONDS),
    );
    let network = Network::current();
    let started = Instant::now();
    let took = || u64::try_from(started.elapsed().as_millis()).unwrap_or(u64::MAX);
    match role {
        "embedder" => {
            let mut request = EmbedRequest::new(
                model,
                EMBED_SAMPLE.iter().map(|text| (*text).to_owned()).collect(),
            );
            request.dimensions = params
                .get("dimensions")
                .and_then(Value::as_u64)
                .and_then(|each| u32::try_from(each).ok());
            Ok(match assist.embed(&network, choice, request, timeout) {
                Ok(done) => {
                    let length = done.value.vectors.first().map_or(0, Vec::len);
                    json!({
                        "ok": length > 0 && done.value.vectors.len() == EMBED_SAMPLE.len(),
                        "backend": done.backend,
                        "model": model,
                        "vectors": done.value.vectors.len(),
                        "length": length,
                        "ms": took(),
                    })
                }
                Err(why) => json!({"ok": false, "why": why, "ms": took()}),
            })
        }
        "ranker" => {
            let ranker = crate::kb_mcp::Ranker::new(
                std::sync::Arc::clone(assist),
                choice,
                model,
                RANK_SAMPLE.len(),
                timeout,
                600,
            );
            let passages: Vec<String> = RANK_SAMPLE.iter().map(|text| (*text).to_owned()).collect();
            Ok(match ranker.order(&network, RANK_QUERY, &passages) {
                Ok(order) => json!({
                    "ok": true,
                    "order": order,
                    "best_first": order.first() == Some(&1),
                    "native": assist.can(Operation::Rerank, choice),
                    "model": model,
                    "ms": took(),
                }),
                Err(why) => json!({"ok": false, "why": why, "ms": took()}),
            })
        }
        _ => Err(bad("role is embedder or ranker")),
    }
}
