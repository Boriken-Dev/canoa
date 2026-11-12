//! Overlay hooks for a profile's connection: which hooks an overlay has and whether they are on, the processes that run
//! them, and what an event's hooks amount to. Nothing here speaks ACP: `events` does.
//!
//! A hook runs from a private copy of the overlay's files made from the bytes its digest covers, with only the
//! environment named below, never the session's.

mod events;
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
    self as aos, FiredEvent, Hook, HookEvent, HookFacts, HookPlan, HookRun, OnFailure, Settings,
};
use serde_json::{Map, Value, json};

use super::{Item, LayerId, LayerKind, ReadOverlay, State, Written, os, program_of};
use crate::config::HostProfile;
use crate::profile::AosLayer;

pub(super) use events::{Book, Seen};

/// The events Canoa fires: the ones it sees itself over ACP. `cd` and `pre-compact` it cannot see.
const FIRES: [HookEvent; 7] = [
    HookEvent::PreEnv,
    HookEvent::SessionStart,
    HookEvent::SessionEnd,
    HookEvent::Prompt,
    HookEvent::TurnEnd,
    HookEvent::PreTool,
    HookEvent::PostTool,
];

/// Events Canoa fires for some occurrences only: `pre-tool` for the tool calls the agent asks permission for.
const PARTIAL: [HookEvent; 1] = [HookEvent::PreTool];

/// The reasons of `session-start` Canoa sends: it sees only the sessions it opens itself.
const REASONS: [&str; 2] = ["new", "resume"];

/// The variables a hook gets from the host's own environment, besides its event and its overlay's root: nothing the person
/// put there for the agent, or a credential, is among them.
#[cfg(windows)]
const PASSED: [&str; 14] = [
    "PATHEXT",
    "SystemRoot",
    "SystemDrive",
    "windir",
    "ComSpec",
    "TEMP",
    "TMP",
    "USERPROFILE",
    "HOMEDRIVE",
    "HOMEPATH",
    "APPDATA",
    "LOCALAPPDATA",
    "ProgramData",
    "PROCESSOR_ARCHITECTURE",
];
#[cfg(not(windows))]
const PASSED: [&str; 7] = ["HOME", "LANG", "LC_ALL", "LC_CTYPE", "TZ", "TMPDIR", "USER"];

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

/// What a host that was killed mid-connection left in `dir`: copies of overlays' files (`canoa-hooks-*`) older than this are
/// removed when a connection starts, so the temporary directory does not fill.
const STALE: Duration = Duration::from_secs(3600);

/// Removes the `canoa-hooks-*` entries of `dir` older than [`STALE`].
pub(super) fn remove_stale(dir: &Path) {
    let Ok(entries) = std::fs::read_dir(dir) else {
        return;
    };
    for entry in entries.flatten() {
        if !entry
            .file_name()
            .to_string_lossy()
            .starts_with("canoa-hooks-")
        {
            continue;
        }
        let old = entry
            .metadata()
            .and_then(|meta| meta.modified())
            .ok()
            .and_then(|made| made.elapsed().ok())
            .is_some_and(|age| age > STALE);
        if old {
            let _ = std::fs::remove_dir_all(entry.path());
        }
    }
}

/// The hooks of a connection's overlays and the means to run them.
#[derive(Debug)]
pub(super) struct Hooks {
    overlays: Vec<OverlayHooks>,
    settings: Settings,
    /// The interpreters found on the host's `PATH` when the connection started, by AOS name.
    interpreters: BTreeMap<&'static str, PathBuf>,
    /// What a hook is given of the host's environment.
    base: Vec<(String, String)>,
    /// Where the copies of the overlays' files are made, removed with the connection.
    under: PathBuf,
    copies: Mutex<BTreeMap<String, Written>>,
}

impl Drop for Hooks {
    /// The copies of the overlays' files are removed with the connection.
    fn drop(&mut self) {
        self.copies
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        let _ = std::fs::remove_dir(&self.under);
    }
}

impl Hooks {
    /// The hooks of `layers`, the overlays that made layers in composition order, whose files are `overlays`.
    pub(super) fn new(
        host: &HostProfile,
        layers: &[AosLayer],
        overlays: &[ReadOverlay],
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
        let mut base: Vec<(String, String)> = PASSED
            .iter()
            .filter_map(|name| Some(((*name).to_owned(), lookup(name)?)))
            .collect();
        // Only absolute directories: a relative one would name whatever the overlay's root holds.
        let path = lookup("PATH")
            .map(|path| {
                std::env::split_paths(&path)
                    .filter(|directory| directory.is_absolute())
                    .collect::<Vec<_>>()
            })
            .and_then(|directories| std::env::join_paths(directories).ok());
        if let Some(path) = path {
            base.push(("PATH".to_owned(), path.to_string_lossy().into_owned()));
        }
        #[cfg(windows)]
        base.push((
            "NoDefaultCurrentDirectoryInExePath".to_owned(),
            "1".to_owned(),
        ));
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
        let mut random = [0_u8; 16];
        let _ = getrandom::fill(&mut random);
        let name: String = random.iter().map(|byte| format!("{byte:02x}")).collect();
        Self {
            overlays: planned,
            settings: Settings::default(),
            interpreters,
            base,
            under: std::env::temp_dir().join(format!("canoa-hooks-{name}")),
            copies: Mutex::new(BTreeMap::new()),
        }
    }

    /// The parts of `context` that fit the session's bound, counting what the session already holds in `used`; a part past it is
    /// not added and is reported `hook-not-applied`.
    pub(super) fn admit(
        &self,
        used: &mut usize,
        context: Vec<(String, String)>,
        event: &str,
    ) -> (Vec<(String, String)>, Vec<Item>) {
        let mut kept = Vec::new();
        let mut items = Vec::new();
        for (overlay, text) in context {
            if self
                .settings
                .context_bytes
                .allows(used.saturating_add(text.len()))
            {
                *used += text.len();
                kept.push((overlay, text));
            } else {
                tracing::info!(
                    overlay,
                    event,
                    "a hook's context passes the session's bound and was not added"
                );
                items.push(Item::new(
                    format!("hooks.{overlay}.{event}"),
                    LayerId::new(LayerKind::Overlay, overlay),
                    State::Unsupported,
                    "hook-not-applied: its context passes the session's context bound",
                ));
            }
        }
        (kept, items)
    }

    /// Whether any overlay has hooks.
    pub(super) fn any(&self) -> bool {
        !self.overlays.is_empty()
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
    pub(super) fn items(&self, person: &BTreeSet<String>) -> Vec<Item> {
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
    fn copy_of(&self, identity: &str, overlay: &ReadOverlay) -> Option<PathBuf> {
        let mut copies = self.copies.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(made) = copies.get(identity) {
            return Some(made.0.clone());
        }
        let files: BTreeMap<String, Vec<u8>> = overlay
            .tree
            .paths()
            .filter_map(|path| Some((path.to_owned(), overlay.tree.bytes(path)?.to_vec())))
            .collect();
        let made = Written::create(&self.under, &files).ok()?;
        let root = made.0.clone();
        copies.insert(identity.to_owned(), made);
        Some(root)
    }

    /// One hook of `overlay` run for `event`, told `payload`.
    fn run_one(
        &self,
        overlay: &OverlayHooks,
        read: &ReadOverlay,
        hook: &Hook,
        payload: &Map<String, Value>,
        spent: u64,
    ) -> HookRun {
        let mut run = HookRun::new(overlay.identity.clone(), Vec::new());
        run.on_failure = Some(hook.on_failure);
        run.sets.clone_from(&hook.sets);
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
        let mut env = self.base.clone();
        env.push(("AOS_HOOK_EVENT".to_owned(), hook.event.as_str().to_owned()));
        env.push((
            aos::root_variable(&overlay.identity),
            root.to_string_lossy().into_owned(),
        ));
        let limit = self.settings.hook_output_bytes.get().unwrap_or(u64::MAX);
        let outcome = run::run(&run::Job {
            program,
            args: arguments(interpreter, &script, hook.event),
            cwd: &root,
            env: &env,
            input: &bytes,
            timeout: Duration::from_secs(hook.timeout),
            max_output: usize::try_from(limit).unwrap_or(usize::MAX - 1),
        });
        run.not_run = !outcome.started;
        run.timed_out = outcome.timed_out;
        run.exit = outcome.exit;
        run.seconds = outcome.seconds;
        run.stdout = outcome.stdout;
        run
    }

    /// Every hook of `event` that is on, in composition order, run until one blocks, denies or refuses, and combined to
    /// the tightest: what the event amounts to. `known` holds what the consumer knows of the payload, `select` the reason
    /// or tool kind `match` selects on, `person` the overlays the person allowed, `applies` the outputs Canoa can apply
    /// for this event.
    pub(super) fn fire(
        &self,
        overlays: &[ReadOverlay],
        event: HookEvent,
        known: &Map<String, Value>,
        select: Option<&str>,
        person: &BTreeSet<String>,
        applies: &[&str],
    ) -> FiredEvent {
        let payload = aos::hook_payload(event, known);
        let mut runs: Vec<HookRun> = Vec::new();
        let mut spent = 0_u64;
        for overlay in &self.overlays {
            if !(overlay.allowed || person.contains(&overlay.identity)) {
                continue;
            }
            let Some(read) = overlays
                .iter()
                .find(|read| read.identity == overlay.identity)
            else {
                continue;
            };
            for hook in overlay.hooks.iter().filter(|hook| hook.event == event) {
                if let Some(select) = select
                    && !aos::hook_matches(event, hook.select.as_deref(), select)
                {
                    continue;
                }
                let run = self.run_one(overlay, read, hook, &payload, spent);
                spent = spent.saturating_add(run.seconds);
                runs.push(run);
                let so_far =
                    aos::fire_event(event, &runs, Some(applies), &self.settings, cfg!(windows));
                if so_far
                    .decision
                    .is_some_and(|decision| decision != aos::Decision::Ask)
                {
                    return so_far;
                }
            }
        }
        aos::fire_event(event, &runs, Some(applies), &self.settings, cfg!(windows))
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
            let Some(read) = overlays
                .iter()
                .find(|read| read.identity == overlay.identity)
            else {
                continue;
            };
            let run = self.run_one(overlay, read, hook, &payload, 0);
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
