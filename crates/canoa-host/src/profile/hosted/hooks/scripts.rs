//! An overlay's env scripts: the programs that compute environment values, run once when a connection's environment is
//! built, at the digest canoa.cfg allowed the overlay's hooks and env scripts at. Their output joins the session's
//! environment where the order of application puts it.
//!
//! A script runs from the private copy of the overlay's files made from the bytes its digest covers, with the environment the
//! stack has when its turn comes, and with that copy as its working directory.

use alloc::collections::{BTreeMap, BTreeSet};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::OsString;

use coqui_aos::{self as aos, Environment, HookRun, ScriptCall, ScriptFailure, ScriptResult};

use super::{Hooks, launch, run};
use crate::profile::hosted::{Item, LayerId, LayerKind, ReadOverlay, State};
use crate::profile::overlay::ScriptRuns;

/// How long a script that asks for no time limit may run: a figure no run reaches, since a deadline needs a number.
const NO_LIMIT: Duration = Duration::from_secs(10 * 365 * 24 * 3600);
/// The most of a script's standard error a report shows.
const TAIL_CHARS: usize = 300;

/// What the connection's env scripts came to.
#[derive(Debug, Default)]
pub(in crate::profile::hosted) struct EnvScripts {
    /// What each gave, for every environment built from the overlays to apply as it was.
    pub(in crate::profile::hosted) runs: ScriptRuns,
    /// The variables they set, which the agent is given besides those an overlay declares.
    pub(in crate::profile::hosted) exported: Vec<String>,
    /// What the report says of each script that ran, and of each variable it set that is non-default.
    pub(in crate::profile::hosted) items: Vec<Item>,
}

/// What one run left besides its output.
struct Ran {
    /// Why it was not started, when it was not.
    why: Option<String>,
    /// How it ended, when it failed after starting.
    status: Option<i32>,
    seconds: u64,
    stderr: Vec<u8>,
}

impl Hooks {
    /// Runs every overlay's env scripts that canoa.cfg's allowance lets run, in the order of application, and says what
    /// they gave. `extra` is what the session adds to this process's own environment, which a script gets, and `pre_env` the
    /// runs of the `pre-env` hooks, which the stack applies before any overlay's declarations.
    pub(in crate::profile::hosted) fn env_scripts(
        &mut self,
        overlays: &[ReadOverlay],
        extra: &[(String, String)],
        pre_env: &BTreeMap<String, HookRun>,
    ) -> EnvScripts {
        let mut out = EnvScripts::default();
        out.runs.interpreters = Some(self.interpreters.keys().map(ToString::to_string).collect());
        if self.active.is_empty() {
            return out;
        }
        let active: Vec<(&aos::Document, aos::Facts)> = self
            .active
            .iter()
            .map(|(manifest, facts)| (manifest, facts.clone()))
            .collect();
        let mut base = Environment::new(self.base.clone(), cfg!(windows));
        for (name, value) in extra {
            base.set(name, value.clone(), false);
        }
        let mut non_defaults = Vec::new();
        let mut results: Vec<((String, usize), ScriptResult)> = Vec::new();
        let mut ran: Vec<Ran> = Vec::new();
        let composed = {
            let mut running = |call: &ScriptCall<'_>| -> ScriptResult {
                let (result, left) = self.run_script(overlays, call);
                results.push(((call.identity.to_owned(), call.index), result.clone()));
                ran.push(left);
                result
            };
            aos::composed_environment_running(
                &active,
                base,
                pre_env,
                &mut non_defaults,
                &mut running,
            )
        };
        let settings = &self.settings;
        let mut sets = Vec::new();
        for (key, result) in &results {
            out.runs.results.insert(key.clone(), result.clone());
            if let Ok(bytes) = result
                && let Ok(set) = aos::read_env_script_output(bytes, settings)
            {
                sets.push(set);
            }
        }
        self.script_sets = sets;
        // A default script no interpreter of this computer can run: the overlay's other scripts still do.
        for each in non_defaults
            .iter()
            .filter(|each| each.id == aos::NonDefaultId::ScriptNotRun)
        {
            for (_, facts) in &self.active {
                if facts.tree.contains(&each.subject) {
                    out.items.push(Item::new(
                        format!("env_scripts.{}.{}", facts.identity, each.subject),
                        LayerId::new(LayerKind::Overlay, facts.identity.clone()),
                        State::Unsupported,
                        "script-not-run: it does not apply on this OS, or no interpreter for it is on the host's PATH",
                    ));
                }
            }
        }
        let mut exported = BTreeSet::new();
        for (report, left) in composed.script_reports.iter().zip(&ran) {
            let layer = LayerId::new(LayerKind::Overlay, report.identity.clone());
            let key = format!("env_scripts.{}.{}", report.identity, report.path);
            if let Some(failure) = report.failure {
                tracing::warn!(
                    "an env script of {} failed ({}), so none of its output applies",
                    report.identity,
                    failure.as_str()
                );
                out.items.push(Item::new(
                    key,
                    &layer,
                    State::Refused,
                    failed(failure, left),
                ));
                continue;
            }
            let names: Vec<&str> = report
                .set
                .iter()
                .filter(|(_, set)| *set)
                .map(|(name, _)| name.as_str())
                .collect();
            let how = if names.is_empty() {
                "env-script: ran; it set no variable".to_owned()
            } else {
                format!("env-script: ran; it set {}", names.join(", "))
            };
            out.items.push(Item::new(key, &layer, State::Applied, how));
            for (name, set) in &report.set {
                if *set && exported.insert(name.clone()) {
                    out.exported.push(name.clone());
                }
                if *set && aos::is_sensitive_variable(name) {
                    out.items.push(Item::new(
                        format!("env_scripts.{}.{}.{name}", report.identity, report.path),
                        &layer,
                        State::Applied,
                        "env-sensitive: the script sets a variable that changes how programs find code, trust certificates or reach the network",
                    ));
                }
            }
        }
        out
    }

    /// One env script, run from the private copy of its overlay's files.
    fn run_script(&self, overlays: &[ReadOverlay], call: &ScriptCall<'_>) -> (ScriptResult, Ran) {
        let mut left = Ran {
            why: None,
            status: None,
            seconds: 0,
            stderr: Vec::new(),
        };
        let mut not_run = |why: String| {
            left.why = Some(why);
            Err(ScriptFailure::NotRun)
        };
        let script = call.script;
        let Some(program) = self.interpreters.get(script.interpreter.as_str()) else {
            let why = format!("{} is not on the host's PATH", script.interpreter);
            let result = not_run(why);
            return (result, left);
        };
        let Some(read) = overlays.iter().find(|read| read.identity == call.identity) else {
            let result = not_run("its files are not among the ones read".to_owned());
            return (result, left);
        };
        if !aos::valid_relative_path(&script.path) || !read.tree.contains(&script.path) {
            let result = not_run(format!(
                "{} is not a file the overlay's digest covers",
                script.path
            ));
            return (result, left);
        }
        let Some(root) = self.copy_of(call.identity, read) else {
            let result = not_run("a private copy of its files could not be made".to_owned());
            return (result, left);
        };
        let mut file = root.clone();
        file.extend(script.path.split('/'));
        let mut args = launch(&script.interpreter, &file);
        args.extend(script.args.iter().map(OsString::from));
        let limit = self.settings.hook_output_bytes.get().unwrap_or(u64::MAX);
        let max_output = usize::try_from(limit).unwrap_or(usize::MAX - 1);
        let outcome = run::run(&run::Job {
            program,
            args,
            cwd: &root,
            env: &call.env,
            input: b"",
            timeout: if script.timeout == 0 {
                NO_LIMIT
            } else {
                Duration::from_secs(script.timeout)
            },
            max_output,
        });
        left.seconds = outcome.seconds;
        left.stderr = outcome.stderr;
        let result = if !outcome.started {
            left.why = Some(format!("{} could not be started", script.interpreter));
            Err(ScriptFailure::NotRun)
        } else if outcome.timed_out {
            Err(ScriptFailure::Timeout)
        } else if outcome.stdout.len() > max_output {
            Err(ScriptFailure::TooLarge)
        } else if outcome.unreadable || outcome.exit != 0 {
            left.status = Some(outcome.exit);
            Err(ScriptFailure::Exit)
        } else {
            Ok(outcome.stdout)
        };
        (result, left)
    }
}

/// How the report words a script that gave nothing: its cause, never its output, and the end of its standard error, which is
/// the person's to read and never the agent's.
fn failed(failure: ScriptFailure, left: &Ran) -> String {
    let cause = match failure {
        ScriptFailure::NotRun => format!(
            "env-script-failed: not run ({})",
            left.why.as_deref().unwrap_or("it could not be started")
        ),
        ScriptFailure::Timeout => format!(
            "env-script-failed: stopped at its time limit after {} s",
            left.seconds
        ),
        ScriptFailure::TooLarge => {
            "env-script-failed: its output was past the bound, so none of it was read".to_owned()
        }
        ScriptFailure::Exit => match left.status {
            Some(status) if status != -1 => format!("env-script-failed: it exited with status {status}"),
            _ => "env-script-failed: it did not end normally".to_owned(),
        },
        ScriptFailure::Output => {
            "env-script-failed: its output is not the JSON object or lines the specification defines".to_owned()
        }
        _ => "env-script-failed".to_owned(),
    };
    let tail = tail_of(&left.stderr);
    if tail.is_empty() {
        format!("{cause}; nothing of its output was applied")
    } else {
        format!("{cause}; nothing of its output was applied. Its standard error ended: {tail}")
    }
}

/// The end of `bytes` as one line of text: control characters read as spaces, at most [`TAIL_CHARS`] of it.
pub(super) fn tail_of(bytes: &[u8]) -> String {
    let text = String::from_utf8_lossy(bytes);
    let flat: String = text
        .chars()
        .map(|each| {
            if each.is_control()
                || matches!(each, '\u{202a}'..='\u{202e}' | '\u{2066}'..='\u{2069}')
            {
                ' '
            } else {
                each
            }
        })
        .collect();
    let flat = flat.split_whitespace().collect::<Vec<_>>().join(" ");
    let skip = flat.chars().count().saturating_sub(TAIL_CHARS);
    flat.chars().skip(skip).collect()
}
