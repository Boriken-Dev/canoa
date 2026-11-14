//! Overlay hooks for a profile's connection: which hooks an overlay has and whether they are on, the processes that run
//! them, and what an event's hooks amount to. Nothing here speaks ACP: `events` does.
//!
//! A hook runs from a private copy of the overlay's files made from the bytes its digest covers, in the session's folder
//! with the session's environment: the host's own, and every overlay's declarations stacked in composition order.

mod allow;
mod events;
mod forward;
mod native;
mod run;

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::OsString;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};

use coqui_aos::{
    self as aos, Document, Environment, Facts, FiredEvent, Hook, HookEvent, HookFacts, HookPlan,
    HookRun, HookScope, OnFailure, Settings,
};
use indexmap::IndexMap;
use serde_json::{Map, Value, json};

use super::{Item, LayerId, LayerKind, ReadOverlay, State, Written, os, program_of};
use crate::config::HostProfile;
use crate::profile::{AosLayer, active_facts};

pub(super) use events::{Book, Seen};
pub(crate) use forward::forward;

/// The events Canoa fires: the ones it sees itself over ACP, and `cd` and `pre-compact`, which only a harness's own hooks
/// show and which it fires for the harnesses whose hooks it can reach (`native`).
const FIRES: [HookEvent; 9] = [
    HookEvent::PreEnv,
    HookEvent::SessionStart,
    HookEvent::SessionEnd,
    HookEvent::Prompt,
    HookEvent::TurnEnd,
    HookEvent::Cd,
    HookEvent::PreCompact,
    HookEvent::PreTool,
    HookEvent::PostTool,
];

/// Events Canoa fires for some occurrences only: `pre-tool` for the tool calls the agent asks permission for.
const PARTIAL: [HookEvent; 1] = [HookEvent::PreTool];

/// The reasons of `session-start` Canoa sends for every harness: it sees the sessions it opens itself.
const REASONS: [&str; 2] = ["new", "resume"];

/// The interpreters AOS names, and the programs that stand for each, in the order they are looked for.
fn programs_of(interpreter: &str) -> &'static [&'static str] {
    match (interpreter, cfg!(windows)) {
        ("python", true) => &["python", "python3"],
        ("python", false) => &["python3", "python"],
        ("node", _) => &["node"],
        ("powershell", _) => &["pwsh", "powershell"],
        ("bash", _) => &["bash"],
        ("sh", _) => &["sh"],
        _ => &[],
    }
}

/// The arguments that run `script` as `interpreter` does, the event last: a list, never a shell's parsing.
fn arguments(interpreter: &str, script: &Path, event: HookEvent) -> Vec<OsString> {
    let mut args: Vec<OsString> = match interpreter {
        "python" => vec!["-B".into()],
        "powershell" => [
            "-NoProfile",
            "-NonInteractive",
            "-ExecutionPolicy",
            "Bypass",
            "-File",
        ]
        .into_iter()
        .map(OsString::from)
        .collect(),
        _ => Vec::new(),
    };
    args.push(script.as_os_str().to_owned());
    args.push(event.as_str().into());
    args
}

/// One overlay's hooks as this host plans them.
#[derive(Debug)]
pub(super) struct OverlayHooks {
    pub(super) identity: String,
    pub(super) digest: String,
    hooks: Vec<Hook>,
    /// Whether canoa.cfg allows them at this digest.
    pub(super) allowed: bool,
    /// The digest canoa.cfg names, when it is not this one.
    changed: Option<String>,
    /// Whether any is a guard: `on_failure = "refuse"`.
    guarded: bool,
    /// What the plan reports with the hooks on (`hook-unsupported`, `hook-partial`), by report id and subject.
    on_reports: Vec<(String, String)>,
}

/// Whether an overlay is active at all: a guard that is off would fail open, so its overlay waits until the hooks are
/// allowed.
pub(super) fn activates(host: &HostProfile, layer: &AosLayer, overlay: &ReadOverlay) -> bool {
    let allowed = host.allow_hooks.get(&layer.identity).map(String::as_str);
    plan(layer, overlay, allowed, None).is_none_or(|plan| plan.activates)
}

/// What the overlay's hooks amount to when `allowed` is the digest the person allowed them at and `interpreters` the ones
/// the host has (none is every one); none when it has no hooks.
fn plan(
    layer: &AosLayer,
    overlay: &ReadOverlay,
    allowed: Option<&str>,
    interpreters: Option<&[String]>,
) -> Option<HookPlan> {
    let mut facts = HookFacts::new(os(), layer.identity.clone());
    facts.digest = Some(layer.digest.clone());
    facts.allowed = allowed.map(str::to_owned);
    facts.interpreters = interpreters.map(<[String]>::to_vec);
    facts.fires = Some(FIRES.to_vec());
    facts.partial = PARTIAL.to_vec();
    let plan = aos::hook_plan(&layer.manifest, &overlay.tree, &facts, &Settings::default()).ok()?;
    (!plan.hooks.is_empty()).then_some(plan)
}

/// Removes what a host that was killed mid-connection left in `dir`: copies of overlays' files (`canoa-hooks-*`) and the
/// plugins and skills written for a connection (`canoa-profile-*`), those this user owns and no running host holds.
pub(super) fn remove_stale(dir: &Path) {
    super::private::sweep(dir, &["canoa-hooks", "canoa-profile"]);
}

/// The hooks of a connection's overlays and the means to run them.
#[derive(Debug)]
pub(super) struct Hooks {
    overlays: Vec<OverlayHooks>,
    settings: Settings,
    /// The interpreters found on the host's `PATH` when the connection started, by AOS name.
    interpreters: BTreeMap<&'static str, PathBuf>,
    /// The environment the agent starts from, which a hook gets too: this process's, and the person's own layers' variables.
    base: Vec<(String, String)>,
    /// The places a hook's allow never answers for.
    refusals: allow::Refusals,
    /// Each overlay's manifest and facts, in composition order: what a hook's environment is made from.
    active: Vec<(Document, Facts)>,
    /// The manifests by normalized identity, which the order of hooks is taken from.
    manifests: BTreeMap<String, Document>,
    /// Where the copies of the overlays' files are made, removed with the connection.
    under: super::private::Private,
    copies: Mutex<BTreeMap<String, Written>>,
}

impl Drop for Hooks {
    /// The copies of the overlays' files are removed with the connection.
    fn drop(&mut self) {
        self.copies
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
    }
}

impl Hooks {
    /// The hooks of `layers`, the overlays that made layers in composition order, whose files are `overlays`.
    pub(super) fn new(
        host: &HostProfile,
        layers: &[AosLayer],
        overlays: &[ReadOverlay],
        shared: &IndexMap<String, LayerId>,
        lookup: &dyn Fn(&str) -> Option<String>,
    ) -> Self {
        let mut interpreters = BTreeMap::new();
        for name in aos::INTERPRETERS {
            for program in programs_of(name) {
                if let Ok(found) = program_of(program, None, lookup) {
                    interpreters.insert(name, PathBuf::from(found));
                    break;
                }
            }
        }
        // A Windows search for a program never starts at the folder the hook runs in, which is the session's.
        let base: Vec<(String, String)> = std::env::vars()
            .chain(cfg!(windows).then(|| {
                (
                    "NoDefaultCurrentDirectoryInExePath".to_owned(),
                    "1".to_owned(),
                )
            }))
            .collect();
        let have: Vec<String> = interpreters.keys().map(|name| (*name).to_owned()).collect();
        let mut planned = Vec::new();
        for layer in layers {
            let Some(overlay) = overlays
                .iter()
                .find(|overlay| overlay.identity == layer.identity)
            else {
                continue;
            };
            let named = host.allow_hooks.get(&layer.identity).map(String::as_str);
            let Some(found) = plan(layer, overlay, named, Some(&have)) else {
                continue;
            };
            // What the plan reports once the hooks are on, which only a plan with them on gives.
            let on_reports = plan(layer, overlay, Some(&layer.digest), Some(&have))
                .map(|ifon| ifon.reports)
                .unwrap_or_default();
            let guarded = found
                .hooks
                .iter()
                .any(|hook| hook.on_failure == OnFailure::Refuse);
            let changed = named
                .filter(|named| !found.on && !named.is_empty())
                .map(str::to_owned);
            planned.push(OverlayHooks {
                identity: layer.identity.clone(),
                digest: layer.digest.clone(),
                allowed: found.on,
                changed,
                guarded,
                on_reports,
                hooks: found.hooks,
            });
        }
        remove_stale(&std::env::temp_dir());
        if super::private::base() != std::env::temp_dir() {
            remove_stale(&super::private::base());
        }
        Self {
            overlays: planned,
            settings: Settings::default(),
            interpreters,
            base,
            refusals: allow::Refusals::new(host.protected.as_deref()),
            active: active_facts(layers, shared, lookup)
                .into_iter()
                .map(|(manifest, facts)| (manifest.clone(), facts))
                .collect(),
            manifests: layers
                .iter()
                .map(|layer| (aos::normalize_name(&layer.identity), layer.manifest.clone()))
                .collect(),
            under: super::private::Private::new("canoa-hooks"),
            copies: Mutex::new(BTreeMap::new()),
        }
    }

    /// Whether any overlay has hooks.
    pub(super) fn any(&self) -> bool {
        !self.overlays.is_empty()
    }

    /// What the harness's own hooks are to report for a session whose person allowed `person`: which reports, and which of
    /// them stop the harness when the host cannot be asked because an overlay's hook that guards it is on.
    pub(in crate::profile::hosted::hooks) fn native(
        &self,
        person: &BTreeSet<String>,
    ) -> (Vec<(forward::Kind, bool)>, bool) {
        let (mut start, mut pre, mut batch) = (false, false, false);
        let (mut refuse, mut guarded) = (false, false);
        for overlay in self
            .overlays
            .iter()
            .filter(|overlay| overlay.allowed || person.contains(&overlay.identity))
        {
            for hook in &overlay.hooks {
                match hook.event {
                    HookEvent::Cd => {
                        start = true;
                        batch = true;
                        guarded |= hook.on_failure == OnFailure::Refuse;
                    }
                    HookEvent::PreCompact => {
                        pre = true;
                        if hook.on_failure == OnFailure::Refuse {
                            refuse = true;
                            guarded = true;
                        }
                    }
                    HookEvent::SessionStart => {
                        let compacts = hook.select.as_ref().is_none_or(|select| {
                            select
                                .iter()
                                .any(|reason| reason == "compact" || reason == "clear")
                        });
                        start |= compacts;
                        guarded |= compacts && hook.on_failure == OnFailure::Refuse;
                    }
                    _ => {}
                }
            }
        }
        // Only a compaction can be stopped, so only its forwarder is told to stop when the host cannot be asked.
        let kinds = [
            (start, forward::Kind::Start, false),
            (pre, forward::Kind::PreCompact, refuse),
            (batch, forward::Kind::Batch, false),
        ]
        .into_iter()
        .filter_map(|(wanted, kind, stops)| wanted.then_some((kind, stops)))
        .collect();
        (kinds, guarded)
    }

    /// Whether the permission request's `call` names a place the person protects, or one that cannot be placed, from the folder
    /// `cwd`: an allow never answers for it.
    pub(super) fn protects(&self, call: &Value, cwd: &Path) -> bool {
        self.refusals.touches(call, cwd)
    }

    /// Adds to the environment hooks run in the variables the person's own layers give the agent.
    pub(super) fn add_to_environment(
        &mut self,
        variables: impl IntoIterator<Item = (String, String)>,
    ) {
        for (name, value) in variables {
            let same = |each: &str| {
                if cfg!(windows) {
                    each.eq_ignore_ascii_case(&name)
                } else {
                    each == name
                }
            };
            self.base.retain(|(each, _)| !same(each));
            self.base.push((name, value));
        }
    }

    /// Whether an event has a hook that is on for `person`'s allowances plus canoa.cfg's.
    pub(super) fn listens(&self, event: HookEvent, person: &BTreeSet<String>) -> bool {
        self.overlays.iter().any(|overlay| {
            (overlay.allowed || person.contains(&overlay.identity))
                && overlay.hooks.iter().any(|hook| hook.event == event)
        })
    }

    /// The overlays whose hooks canoa.cfg does not allow, as the question to the person lists them.
    pub(super) fn approvable(&self) -> Vec<Value> {
        self.overlays
            .iter()
            .filter(|overlay| !overlay.allowed)
            .map(|overlay| {
                let events: Vec<Value> = overlay
                    .hooks
                    .iter()
                    .map(|hook| {
                        let mut each = json!({
                            "event": hook.event.as_str(),
                            "run": hook.path,
                            "timeout": hook.timeout,
                            "onFailure": if hook.on_failure == OnFailure::Refuse { "refuse" } else { "report" },
                        });
                        if let Some(select) = &hook.select {
                            each["match"] = json!(select);
                        }
                        if !hook.sets.is_empty() {
                            each["sets"] = json!(hook.sets);
                        }
                        each["fires"] = json!(FIRES.contains(&hook.event));
                        if hook.event == HookEvent::PreEnv {
                            // It runs before the agent starts, when nobody can be asked: only canoa.cfg's allowance runs it.
                            each["fires"] = json!(false);
                            each["note"] = json!(
                                "runs before the agent starts, so only canoa.cfg's allowance runs it, never this question"
                            );
                        }
                        each
                    })
                    .collect();
                json!({
                    "overlay": overlay.identity,
                    "digest": overlay.digest,
                    "changed": overlay.changed.is_some(),
                    "failClosed": overlay.guarded,
                    "events": events,
                })
            })
            .collect()
    }

    /// The identities among `names` the person may allow: those listed by [`Hooks::approvable`].
    pub(super) fn offered(&self, names: &[String]) -> BTreeSet<String> {
        self.overlays
            .iter()
            .filter(|overlay| !overlay.allowed && names.contains(&overlay.identity))
            .map(|overlay| overlay.identity.clone())
            .collect()
    }

    /// What the report says of each overlay's hooks.
    pub(super) fn items(&self, person: &BTreeSet<String>, harness: Option<&str>) -> Vec<Item> {
        self.items_with(person, harness, None)
    }

    /// As [`Self::items`], for a session whose harness's own hooks could not be written for the reason `failed` gives: the
    /// events that need them are reported failed, not applied.
    pub(super) fn items_with(
        &self,
        person: &BTreeSet<String>,
        harness: Option<&str>,
        failed: Option<&str>,
    ) -> Vec<Item> {
        let mut items = Vec::new();
        for overlay in &self.overlays {
            let layer = LayerId::new(LayerKind::Overlay, overlay.identity.clone());
            let on = overlay.allowed || person.contains(&overlay.identity);
            if !on {
                let how = if overlay.changed.is_some() {
                    "hook-not-allowed: canoa.cfg allows another version of the overlay's files; its hooks are off until allowed again"
                } else {
                    "hook-not-allowed: its hooks are off until canoa.cfg names the overlay's digest or the person allows them"
                };
                items.push(Item::new(
                    format!("hooks.{}", overlay.identity),
                    &layer,
                    State::Off,
                    how,
                ));
                tracing::info!(
                    "the hooks of {layer} are off: agents.NAME.hooks.{} = \"{}\" allows them",
                    overlay.identity,
                    overlay.digest
                );
                continue;
            }
            for hook in &overlay.hooks {
                let event = hook.event.as_str();
                let key = format!("hooks.{}.{event}", overlay.identity);
                let unsupported = overlay
                    .on_reports
                    .iter()
                    .any(|(id, subject)| id == "hook-unsupported" && subject == event);
                let partial = overlay
                    .on_reports
                    .iter()
                    .any(|(id, subject)| id == "hook-partial" && subject == event);
                let native = native::support(harness, hook.event, hook.select.as_deref());
                let never = hook.event == HookEvent::SessionStart
                    && hook.select.as_ref().is_some_and(|select| {
                        !select
                            .iter()
                            .any(|reason| REASONS.contains(&reason.as_str()))
                    });
                let only_for_session = hook.event == HookEvent::PreEnv && !overlay.allowed;
                let item = if only_for_session {
                    Item::new(
                        key,
                        &layer,
                        State::Unsupported,
                        "hook-not-applied: pre-env runs before the agent starts, when nobody can be asked; only canoa.cfg's allowance runs it",
                    )
                } else if let Some(support) = native {
                    match support {
                        Ok(_) if failed.is_some() => Item::new(
                            key,
                            &layer,
                            State::Refused,
                            format!(
                                "hook-failed: the harness's own hooks could not be written ({}), so this hook never runs",
                                failed.unwrap_or_default()
                            ),
                        ),
                        Ok(how) => Item::new(key, &layer, State::Applied, how),
                        Err(why) => Item::new(
                            key,
                            &layer,
                            State::Unsupported,
                            format!("hook-unsupported: {why}, so this hook never runs"),
                        ),
                    }
                } else if unsupported {
                    Item::new(
                        key,
                        &layer,
                        State::Unsupported,
                        format!(
                            "hook-unsupported: Canoa does not see {event}, so this hook never runs"
                        ),
                    )
                } else if never {
                    Item::new(
                        key,
                        &layer,
                        State::Unsupported,
                        "hook-unsupported: it matches only reasons Canoa never sends (it sends new and resume)",
                    )
                } else if partial {
                    Item::new(
                        key,
                        &layer,
                        State::Applied,
                        "hook-partial: it runs for the calls the agent asks permission for, not for those its own rules allow",
                    )
                } else {
                    Item::new(
                        key,
                        &layer,
                        State::Applied,
                        "run by Canoa as ACP shows the event",
                    )
                };
                items.push(item);
                if !hook.sets.is_empty() && hook.event != HookEvent::PreEnv {
                    items.push(Item::new(
                        format!("hooks.{}.{event}.env", overlay.identity),
                        &layer,
                        State::Unsupported,
                        "hook-not-applied: Canoa cannot set a variable for the agent's later commands over ACP",
                    ));
                }
            }
        }
        items
    }

    /// Where `identity`'s files are copied to, made when first needed from the bytes the digest covers.
    pub(super) fn copy_of(&self, identity: &str, overlay: &ReadOverlay) -> Option<PathBuf> {
        let mut copies = self.copies.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(made) = copies.get(identity) {
            return Some(made.0.clone());
        }
        let files: BTreeMap<String, Vec<u8>> = overlay
            .tree
            .paths()
            .filter_map(|path| Some((path.to_owned(), overlay.tree.bytes(path)?.to_vec())))
            .collect();
        let made = Written::create(self.under.ensure().ok()?, &files).ok()?;
        let root = made.0.clone();
        copies.insert(identity.to_owned(), made);
        Some(root)
    }

    /// The environment `hook` of `identity` runs with, as the specification gives it: the session's, with every overlay's root
    /// the private copy made of its verified files. None when a copy cannot be made or the overlay's declarations cannot be
    /// applied.
    fn environment(
        &self,
        event: HookEvent,
        identity: &str,
        turn: &Turn<'_>,
    ) -> Option<Vec<(String, String)>> {
        let mut active = Vec::new();
        for (manifest, facts) in &self.active {
            let read = turn
                .overlays
                .iter()
                .find(|read| read.identity == facts.identity)?;
            let mut facts = facts.clone();
            facts.root = self
                .copy_of(&facts.identity, read)?
                .to_string_lossy()
                .into_owned();
            active.push((manifest, facts));
        }
        let base = Environment::new(self.base.clone(), cfg!(windows));
        let made = aos::hook_environment(
            event,
            &active,
            &base,
            turn.pre_env,
            Some(identity),
            &turn.earlier,
        )
        .ok()?;
        Some(made.values().to_vec())
    }

    /// One hook of `overlay` run in `turn`.
    fn run_one(&self, overlay: &OverlayHooks, hook: &Hook, turn: &Turn<'_>) -> HookRun {
        let Turn {
            overlays,
            payload,
            spent,
            ..
        } = *turn;
        let mut run = HookRun::new(overlay.identity.clone(), Vec::new());
        run.on_failure = Some(hook.on_failure);
        run.sets.clone_from(&hook.sets);
        let Some(read) = overlays
            .iter()
            .find(|read| read.identity == overlay.identity)
        else {
            run.not_run = true;
            return run;
        };
        let bytes = serde_json::to_vec(payload).unwrap_or_default();
        run.payload_size = bytes.len() as u64;
        let event_over = self
            .settings
            .hook_event_timeout
            .get()
            .is_some_and(|bound| spent >= bound);
        let toolbox = hook
            .interpreter
            .as_deref()
            .and_then(|name| self.interpreters.get(name).map(|program| (name, program)));
        let (Some(path), Some((interpreter, program))) = (&hook.path, toolbox) else {
            run.not_run = true;
            return run;
        };
        if event_over || !hook.runs || !read.tree.contains(path) {
            run.not_run = true;
            return run;
        }
        if !self.settings.hook_payload_bytes.allows(bytes.len()) {
            return run;
        }
        let Some(root) = self.copy_of(&overlay.identity, read) else {
            run.not_run = true;
            return run;
        };
        let mut script = root.clone();
        script.extend(path.split('/'));
        let Some(env) = self.environment(hook.event, &overlay.identity, turn) else {
            run.not_run = true;
            return run;
        };
        let limit = self.settings.hook_output_bytes.get().unwrap_or(u64::MAX);
        let outcome = run::run(&run::Job {
            program,
            args: arguments(interpreter, &script, hook.event),
            cwd: turn.cwd,
            env: &env,
            input: &bytes,
            timeout: Duration::from_secs(hook.timeout),
            max_output: usize::try_from(limit).unwrap_or(usize::MAX - 1),
        });
        apply(&mut run, outcome);
        run
    }

    /// Every hook of the event that is on, in the order the specification gives, run until one blocks, denies or
    /// refuses, and combined to the tightest: what the event amounts to. `overlays` are the overlays' files.
    pub(super) fn fire(&self, overlays: &[ReadOverlay], firing: &Firing<'_>) -> FiredEvent {
        let event = firing.event;
        let payload = aos::hook_payload(event, firing.known);
        let on =
            |overlay: &OverlayHooks| overlay.allowed || firing.person.contains(&overlay.identity);
        let has: Vec<String> = self
            .overlays
            .iter()
            .filter(|overlay| on(overlay) && overlay.hooks.iter().any(|hook| hook.event == event))
            .map(|overlay| aos::normalize_name(&overlay.identity))
            .collect();
        let requested: Vec<String> = self.manifests.keys().cloned().collect();
        // Canoa has no hook of its own user's or project's: only the overlays' run.
        let order = aos::hook_order(&self.manifests, &requested, &has, &[]).map_or_else(
            |_| has.clone(),
            |order| {
                order
                    .into_iter()
                    .filter(|(scope, _)| *scope == HookScope::Overlay)
                    .map(|(_, identity)| identity)
                    .collect()
            },
        );
        let mut runs: Vec<HookRun> = Vec::new();
        let mut turn = Turn {
            overlays,
            payload: &payload,
            pre_env: firing.pre_env,
            cwd: Path::new(
                firing
                    .known
                    .get("cwd")
                    .and_then(Value::as_str)
                    .unwrap_or("."),
            ),
            earlier: Vec::new(),
            spent: 0,
        };
        let fired = |runs: &[HookRun]| {
            aos::fire_event_after(
                event,
                runs,
                Some(firing.applies),
                &self.settings,
                cfg!(windows),
                firing.context_used,
            )
        };
        for key in order {
            let Some(overlay) = self
                .overlays
                .iter()
                .find(|overlay| aos::normalize_name(&overlay.identity) == key)
            else {
                continue;
            };
            for hook in overlay.hooks.iter().filter(|hook| hook.event == event) {
                if let Some(select) = firing.select
                    && !aos::hook_matches(event, hook.select.as_deref(), select)
                {
                    continue;
                }
                let run = self.run_one(overlay, hook, &turn);
                turn.spent = turn.spent.saturating_add(run.seconds);
                // What this hook set is part of the environment of the hooks after it, whether or not the host can apply it.
                if let Ok(out) = aos::hook_result(event, &run, &self.settings)
                    && let Some(set) = out.env
                {
                    turn.earlier.push(set);
                }
                runs.push(run);
                let so_far = fired(&runs);
                // A block, a deny or a refusal ends the event; an ask or an allow leaves the next hook to decide.
                if so_far.decision.is_some_and(|decision| {
                    !matches!(decision, aos::Decision::Ask | aos::Decision::Allow)
                }) {
                    return so_far;
                }
            }
        }
        fired(&runs)
    }

    /// The `pre-env` run of each overlay canoa.cfg allows hooks for, by identity: made before the agent starts, when nobody
    /// can be asked.
    pub(super) fn pre_env(&self, overlays: &[ReadOverlay], harness: &str, cwd: &Path) -> PreEnv {
        let mut known = Map::new();
        known.insert("harness".to_owned(), json!(harness));
        known.insert("cwd".to_owned(), json!(cwd.to_string_lossy()));
        let payload = aos::hook_payload(HookEvent::PreEnv, &known);
        let mut out = BTreeMap::new();
        let (mut exported, mut refused) = (Vec::new(), Vec::new());
        for overlay in self.overlays.iter().filter(|overlay| overlay.allowed) {
            let Some(hook) = overlay
                .hooks
                .iter()
                .find(|hook| hook.event == HookEvent::PreEnv)
            else {
                continue;
            };
            // A `pre-env` hook reads what the `pre-env` hooks before it in composition order set.
            let turn = Turn {
                overlays,
                payload: &payload,
                pre_env: &out,
                cwd,
                earlier: Vec::new(),
                spent: 0,
            };
            let run = self.run_one(overlay, hook, &turn);
            // What the environment script's output sets, which the agent is given besides the overlay's declared variables;
            // a name the profile may not set from an overlay is left out and said so.
            let fired = aos::fire_event(
                HookEvent::PreEnv,
                core::slice::from_ref(&run),
                None,
                &self.settings,
                cfg!(windows),
            );
            for (name, value) in fired.env {
                if value.is_none() {
                    continue;
                }
                if crate::profile::is_reserved_env(&name) {
                    refused.push(Item::new(
                        format!("hooks.{}.pre-env.{name}", overlay.identity),
                        LayerId::new(LayerKind::Overlay, overlay.identity.clone()),
                        State::Refused,
                        "hook-not-applied: an overlay may not set that variable for the agent",
                    ));
                } else if !exported.contains(&name) {
                    exported.push(name);
                }
            }
            out.insert(overlay.identity.clone(), run);
        }
        PreEnv {
            runs: out,
            exported,
            refused,
        }
    }
}

/// What a hook is run in: the overlays' files, the payload, the `pre-env` runs of the connection, the session's folder, what
/// the event's earlier hooks set and the seconds the event's hooks have used.
struct Turn<'a> {
    overlays: &'a [ReadOverlay],
    payload: &'a Map<String, Value>,
    pre_env: &'a BTreeMap<String, HookRun>,
    /// The session's folder, where a hook runs.
    cwd: &'a Path,
    /// What each earlier hook of the event set, in run order.
    earlier: Vec<aos::EnvSet>,
    spent: u64,
}

/// One firing of an event.
pub(super) struct Firing<'a> {
    pub(super) event: HookEvent,
    /// What the consumer knows of the payload.
    pub(super) known: &'a Map<String, Value>,
    /// The reason or tool kind `match` selects on.
    pub(super) select: Option<&'a str>,
    /// The overlays the person allowed for the session.
    pub(super) person: &'a BTreeSet<String>,
    /// The outputs Canoa can apply for this event.
    pub(super) applies: &'a [&'a str],
    /// The connection's `pre-env` runs.
    pub(super) pre_env: &'a BTreeMap<String, HookRun>,
    /// The bytes of hook context the session already holds.
    pub(super) context_used: usize,
}

/// The `pre-env` runs a connection's environment was built with. Their output is the environment's own and is not shown.
#[derive(Default)]
pub(super) struct PreEnv {
    /// By overlay.
    pub(super) runs: BTreeMap<String, HookRun>,
    /// The variables the runs set, which the agent is given.
    pub(super) exported: Vec<String>,
    /// What was left out of them.
    pub(super) refused: Vec<Item>,
}

impl core::fmt::Debug for PreEnv {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("PreEnv")
            .field("overlays", &self.runs.len())
            .field("exported", &self.exported)
            .finish()
    }
}

/// What a run says of its hook, for `coqui-aos` to judge. A run whose status could not be read, because someone else
/// collected the process, says only that: what the hook decided is not known, so it is a failure that refuses whatever the
/// hook's `on_failure` is, and no timeout, which passes.
fn apply(run: &mut HookRun, outcome: run::Outcome) {
    run.not_run = !outcome.started;
    run.timed_out = outcome.timed_out;
    run.exit = outcome.exit;
    run.seconds = outcome.seconds;
    run.stdout = outcome.stdout;
    if outcome.unreadable {
        run.on_failure = Some(OnFailure::Refuse);
        run.timed_out = false;
        run.exit = -1;
    }
}

/// A report of the engine as the profile's report words it.
pub(super) fn report_item(report: &aos::HookReport) -> Item {
    let source = report.about.first().cloned().unwrap_or_default();
    let event = report.about.get(1).cloned().unwrap_or_default();
    let rest = report.about.get(2..).unwrap_or_default().join(", ");
    let state = match report.id {
        "hook-failed" => State::Refused,
        _ => State::Unsupported,
    };
    Item::new(
        format!("hooks.{source}.{event}"),
        LayerId::new(LayerKind::Overlay, source),
        state,
        format!("{}: {rest}", report.id),
    )
}

/// The identities named in an answer to the question, as text.
pub(super) fn names(value: Option<&Value>) -> Vec<String> {
    value
        .and_then(Value::as_array)
        .map(|list| {
            list.iter()
                .filter_map(Value::as_str)
                .map(ToString::to_string)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(test)]
mod tests;
