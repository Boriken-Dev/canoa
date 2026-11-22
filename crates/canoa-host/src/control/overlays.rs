//! The overlay store's git work for a page that cannot run git: fetching a source, listing and digesting what it holds, and
//! installing it under the overlays folder beside canoa.cfg. Nothing from a source is ever run: files are read out of git's
//! object store and never checked out, so no hook, filter or attribute of the source applies, and what is installed has
//! the tree digest AOS defines.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, processes and clocks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
#[cfg(test)]
use std::ffi::OsString;
use std::fs;
use std::io::Write as _;
use std::path::{Path, PathBuf};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant};

use canoa::network::Network;
use coqui_aos::{TreeEntry, digest_entries, valid_overlay_name, valid_relative_path};
use serde_json::{Value, json};

use super::{Caller, REFUSED, bad, text_param};
use crate::profile::OverlaySource;
use crate::profile::prepare;

mod catalog;
mod contents;
mod git;
mod scan;
mod skills;
mod tree;

type Outcome = Result<Value, (i32, String)>;

/// Whose git settings a run may use. A source the person named themselves (an overlay or catalog in canoa.cfg, one they typed) is
/// fetched as they would from a shell, with their credentials and ssh keys. A source someone else named (a skill an overlay
/// lists, a catalog entry read ahead) gets none of that: it could be offered the person's keys and ask for their agent.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Trust {
    Own,
    Isolated,
}

/// The most files one overlay may hold.
pub const DEFAULT_MAX_FILES: usize = 4096;
/// The most bytes its files may hold in all.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// What a plan is good for after it was made.
const PLAN_TIME: Duration = Duration::from_secs(15 * 60);
/// The plans one caller may hold; the oldest goes first.
const MAX_PLANS: usize = 16;
/// The most bytes of file contents one answer carries.
pub const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;

/// The limits of what one overlay may hold.
#[derive(Clone, Copy, Debug)]
pub(crate) struct OverlayLimits {
    pub(crate) files: usize,
    pub(crate) bytes: u64,
}

impl Default for OverlayLimits {
    fn default() -> Self {
        Self {
            files: DEFAULT_MAX_FILES,
            bytes: DEFAULT_MAX_BYTES,
        }
    }
}

/// What a source's tree holds: the files by path with their blob ids, the paths left out, the tree digest and the bytes.
type Listing = (BTreeMap<String, String>, Vec<String>, String, u64);

/// What `overlay/check` found, kept by the host for the `overlay/apply` that follows it.
#[derive(Debug)]
struct Plan {
    token: String,
    owner: String,
    name: String,
    source: String,
    commit: String,
    digest: String,
    files: BTreeMap<String, String>,
    skipped: Vec<String>,
    /// What its files hold, in bytes.
    bytes: u64,
    at: Instant,
}

/// The overlays this host installs.
#[derive(Debug, Default)]
pub(crate) struct Overlays {
    /// The overlays folder, beside canoa.cfg; none leaves every method refused.
    root: Option<PathBuf>,
    limits: OverlayLimits,
    plans: Mutex<Vec<Plan>>,
    /// One install or removal at a time.
    gates: Mutex<BTreeMap<String, std::sync::Arc<Mutex<()>>>>,
    git: OnceLock<Result<PathBuf, String>>,
    /// The git processes this host has started, so a caller can tell an answer from a cache apart from one that went to git.
    git_runs: std::sync::atomic::AtomicUsize,
    /// The network git reaches its remotes through; the process's when `None`.
    network: Option<std::sync::Arc<Network>>,
    /// Configured common folders to scan for skills instead of `skill_folders.txt`.
    skill_folders: Option<Vec<String>>,
    /// The environment a test makes this host's git runs inherit: a person's home, ssh agent and programs.
    #[cfg(test)]
    person: Vec<(OsString, OsString)>,
}

fn refused(what: impl Into<String>) -> (i32, String) {
    (REFUSED, what.into())
}

impl Overlays {
    pub(crate) fn new(root: Option<PathBuf>, limits: OverlayLimits) -> Self {
        Self {
            root,
            limits,
            ..Self::default()
        }
    }

    /// Replaces the built-in common skill folders (`skill_folders.txt`) with `folders` when set.
    #[must_use]
    pub(crate) fn with_skill_folders(mut self, folders: Option<Vec<String>>) -> Self {
        self.skill_folders = folders;
        self
    }

    /// This host, whose git runs inherit `vars` as well as this process's environment; a variable with an empty value is one the
    /// person has not set, so it is taken out of the run's environment whatever this process's holds. (Left set and empty, a lower-case
    /// proxy variable would shadow the upper-case one the host adds, where names are case-sensitive.)
    #[cfg(test)]
    pub(crate) fn with_person_env(mut self, vars: &[(&str, OsString)]) -> Self {
        self.person = vars
            .iter()
            .map(|(name, value)| (OsString::from(name), value.clone()))
            .collect();
        self
    }

    /// This host, whose git runs reach their remotes through `network`.
    #[cfg(test)]
    pub(crate) fn with_network(mut self, network: std::sync::Arc<Network>) -> Self {
        self.network = Some(network);
        self
    }

    /// How many git processes this host has started.
    #[cfg(test)]
    pub(crate) fn git_runs(&self) -> usize {
        self.git_runs.load(std::sync::atomic::Ordering::SeqCst)
    }

    fn root(&self) -> Result<&Path, (i32, String)> {
        self.root.as_deref().ok_or_else(|| {
            refused("this host was not started with a canoa.cfg to keep overlays beside")
        })
    }

    /// The lock of one overlay or catalog: its fetch, install and removal take turns, and nobody else waits on them.
    fn gate(&self, name: &str) -> std::sync::Arc<Mutex<()>> {
        let mut gates = self.gates.lock().unwrap_or_else(PoisonError::into_inner);
        std::sync::Arc::clone(gates.entry(name.to_owned()).or_default())
    }

    /// Drops the plans that have expired. What was fetched for them stays for `overlay/discard`, `overlay/remove` and the caps.
    fn expire(&self, plans: &mut Vec<Plan>) {
        plans.retain(|plan| plan.at.elapsed() < PLAN_TIME);
    }

    fn plans(&self) -> std::sync::MutexGuard<'_, Vec<Plan>> {
        self.plans.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn cache(&self, name: &str) -> Result<PathBuf, (i32, String)> {
        Ok(self.root()?.join(".cache").join(format!("{name}.git")))
    }

    /// Where a catalog is fetched to. An extensions catalog has a folder of its own, `.catalogs/.extensions`, whose name no overlay
    /// catalog's `LABEL.git` can take, so two catalogs that share a label never share a fetch.
    fn catalog_cache(&self, label: &str, extensions: bool) -> Result<PathBuf, (i32, String)> {
        let catalogs = self.root()?.join(".catalogs");
        let folder = if extensions {
            catalogs.join(".extensions")
        } else {
            catalogs
        };
        Ok(folder.join(format!("{label}.git")))
    }

    /// The overlays folder, when this host has one.
    pub(crate) fn root_path(&self) -> Option<&Path> {
        self.root.as_deref()
    }

    /// `overlay/installed` `{"name"?}` -> `{"overlays": {name: record}}`: what is installed, as its records say.
    pub(crate) fn installed(&self, params: &Value) -> Outcome {
        let root = self.root()?;
        let mut found = serde_json::Map::new();
        let wanted = text_param(params, "name")?;
        if let Some(name) = wanted {
            checked_name(name)?;
        }
        let Ok(reader) = fs::read_dir(root) else {
            return Ok(json!({"overlays": found}));
        };
        for entry in reader.flatten() {
            let file = entry.file_name();
            let Some(name) = file.to_str().and_then(|file| file.strip_suffix(".json")) else {
                continue;
            };
            if checked_name(name).is_err() || wanted.is_some_and(|wanted| wanted != name) {
                continue;
            }
            if let Some(record) = record_of(root, name) {
                found.insert(name.to_owned(), record);
            }
        }
        Ok(json!({"overlays": found}))
    }

    /// `overlay/needs` `{"name"}` -> `{"items": [{"key", "state", "how", "offer"?}]}`: what the installed overlay needs of this
    /// computer (`runtime`, and the interpreters its scripts name), as a session would find it: on the host's `PATH`, then in the
    /// Store's runtimes. A program that is missing carries the Store runtime that installs it. Starts the programs it asks the
    /// version of, which is why a key needs `store.programs` as well.
    pub(crate) fn needs(&self, params: &Value) -> Outcome {
        let name = checked_name(text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?)?;
        let root = self.root()?;
        let finder = crate::profile::Finder::new(
            &|variable| std::env::var(variable).ok(),
            &crate::sources::runtime_dirs(),
        );
        let lines = crate::profile::of_installed(root, name, coqui_aos::current_os(), &finder)
            .map_err(|why| refused(format!("{name}: {why}")))?;
        Ok(json!({"items": lines.iter().map(crate::profile::NeedLine::json).collect::<Vec<_>>()}))
    }

    /// `overlay/prepare` `{"name"}` -> `{"phase": "install"|"sync"|null, "steps": [..]}`: prepares the installed overlay as the
    /// specification's phases say: its dependencies are installed into an environment of its own (made beside the overlays, never in
    /// one), then its setup runs, and the preparation is recorded for the digest installed. The first preparation of a name is
    /// `install`, every later one `sync`. A step that fails ends it (`-32002` with the step and the end of what it printed) and
    /// leaves the record and the environment as they were. It starts programs, so a key needs `store.programs` as well as
    /// `overlay.write`; the app asks the person first, listing what `store/read_ahead` and `overlay/needs` say will run.
    pub(crate) fn prepare(&self, params: &Value) -> Outcome {
        let name = checked_name(text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?)?;
        let root = self.root()?;
        let gate = self.gate(name);
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let digest = record_of(root, name)
            .and_then(|record| Some(record.get("digest")?.as_str()?.to_owned()))
            .ok_or_else(|| {
                refused(format!(
                    "{name}: it is not installed, or its record holds no digest"
                ))
            })?;
        let plan = prepare::Plan::of(name, &root.join(name), coqui_aos::current_os(), &digest)
            .map_err(|why| refused(format!("{name}: {why}")))?;
        let home = prepare::Home::new(root);
        let finder = crate::profile::Finder::new(
            &|variable| std::env::var(variable).ok(),
            &crate::sources::runtime_dirs(),
        );
        let change = if home.record(name).is_some() {
            coqui_aos::Change::Sync
        } else {
            coqui_aos::Change::Add
        };
        let done = prepare::run(&plan, change, &home, &finder, &[])
            .map_err(|failure| refused(format!("{name}: {failure}")))?;
        Ok(json!({"phase": done.phase.map(coqui_aos::Phase::name), "steps": done.steps}))
    }

    /// `overlay/check` `{"name", "source", "pin"?, "catalog"?}`: fetches `source` into the cache for `name` and says what installing it
    /// would change; nothing installed is touched. `local` says whether a source on this computer may be read. A `catalog` label says
    /// the overlay was chosen from that catalog, someone else's listing: it is fetched [`Trust::Isolated`], and one without the
    /// label (a source the person typed) as the person would from a shell.
    pub(crate) fn check(
        &self,
        caller: &Caller,
        params: &Value,
        local: &dyn Fn(&str) -> bool,
    ) -> Outcome {
        let trust = match text_param(params, "catalog")? {
            Some(label) => {
                checked_name(label)?;
                Trust::Isolated
            }
            None => Trust::Own,
        };
        self.check_as(trust, caller, params, local)
    }

    /// [`Self::check`], fetching the source under `trust`.
    fn check_as(
        &self,
        trust: Trust,
        caller: &Caller,
        params: &Value,
        local: &dyn Fn(&str) -> bool,
    ) -> Outcome {
        let name = checked_name(text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?)?;
        let pin = pin_of(params)?;
        let (repo, reference, path, source) = git_source(params)?;
        if is_local(&repo) && !local(&repo) {
            return Err(refused(
                "a source on this computer must be in a folder this key may browse",
            ));
        }
        let gate = self.gate(name);
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let cache = self.cache(name)?;
        self.init_cache(&cache)?;
        let commit = self.fetch(trust, &cache, &repo, &reference)?;
        let (files, skipped, digest, bytes) = self.list(&cache, &commit, &path)?;
        if let Some(pin) = &pin
            && !same_digest(pin, &digest)
        {
            return Err(refused(format!(
                "Its tree digest is {digest}, not the pin {pin} canoa.cfg names: nothing was installed"
            )));
        }
        let before = record_of(self.root()?, name);
        let old: BTreeMap<String, String> = before
            .as_ref()
            .and_then(|record| record.get("files"))
            .and_then(Value::as_object)
            .map(|files| {
                files
                    .iter()
                    .filter_map(|(path, blob)| Some((path.clone(), blob.as_str()?.to_owned())))
                    .collect()
            })
            .unwrap_or_default();
        let manifest = match files.get("OVERLAY.md").or_else(|| files.get("overlay.cfg")) {
            Some(blob) => {
                let mut read = self.blobs(&cache, std::slice::from_ref(blob))?;
                read.remove(blob)
                    .map(|bytes| String::from_utf8_lossy(&bytes).into_owned())
            }
            None => None,
        };
        let added: Vec<&String> = files
            .keys()
            .filter(|path| !old.contains_key(*path))
            .collect();
        let changed: Vec<&String> = files
            .iter()
            .filter(|(path, blob)| old.get(*path).is_some_and(|before| before != *blob))
            .map(|(path, _)| path)
            .collect();
        let removed: Vec<&String> = old
            .keys()
            .filter(|path| !files.contains_key(*path))
            .collect();
        let mut token = [0_u8; 12];
        getrandom::fill(&mut token).map_err(|_| refused("no randomness to name the plan"))?;
        let token = format!("ov-{}", data_encoding::HEXLOWER.encode(&token));
        let answer = json!({
            "token": token,
            "name": name,
            "from": before.as_ref().and_then(|r| r.get("commit")),
            "fromSource": before.as_ref().and_then(|r| r.get("source")),
            "fromDigest": before.as_ref().and_then(|r| r.get("digest")),
            "commit": commit,
            "digest": digest,
            "files": files,
            "skipped": skipped,
            "added": added,
            "changed": changed,
            "removed": removed,
            "manifest": manifest,
        });
        let mut plans = self.plans();
        self.expire(&mut plans);
        let owner = caller.owner().to_owned();
        while plans.iter().filter(|plan| plan.owner == owner).count() >= MAX_PLANS {
            if let Some(oldest) = plans.iter().position(|plan| plan.owner == owner) {
                plans.remove(oldest);
            }
        }
        plans.push(Plan {
            token,
            owner,
            name: name.to_owned(),
            source,
            commit,
            digest,
            files,
            skipped,
            bytes,
            at: Instant::now(),
        });
        Ok(answer)
    }

    /// The plan `params` names, if it is this caller's and still good; kept unless `take`.
    fn plan(&self, caller: &Caller, params: &Value, take: bool) -> Result<Plan, (i32, String)> {
        let token = text_param(params, "token")?.ok_or_else(|| bad("token is needed"))?;
        let mut plans = self.plans();
        self.expire(&mut plans);
        let at = plans
            .iter()
            .position(|plan| plan.token == token && plan.owner == caller.owner())
            .ok_or_else(|| refused("that plan is not one this host made for this key, or it has expired: check again"))?;
        if take {
            Ok(plans.remove(at))
        } else {
            let plan = &plans[at];
            Ok(Plan {
                token: plan.token.clone(),
                owner: plan.owner.clone(),
                name: plan.name.clone(),
                source: plan.source.clone(),
                commit: plan.commit.clone(),
                digest: plan.digest.clone(),
                files: plan.files.clone(),
                skipped: plan.skipped.clone(),
                bytes: plan.bytes,
                at: plan.at,
            })
        }
    }

    /// `overlay/read` `{"token"}` -> `{"files": {path: base64}}`: the files a checked overlay holds, to show what it would do.
    pub(crate) fn read(&self, caller: &Caller, params: &Value) -> Outcome {
        let plan = self.plan(caller, params, false)?;
        if plan.bytes > MAX_READ_BYTES {
            return Err(refused(format!(
                "the overlay holds more than {MAX_READ_BYTES} bytes, more than one answer carries"
            )));
        }
        let cache = self.cache(&plan.name)?;
        let blobs = self.blobs(&cache, &unique(plan.files.values()))?;
        let mut files = serde_json::Map::new();
        for (path, blob) in &plan.files {
            let bytes = blobs
                .get(blob)
                .ok_or_else(|| refused("git did not give a file it was asked for"))?;
            files.insert(
                path.clone(),
                Value::String(data_encoding::BASE64.encode(bytes)),
            );
        }
        Ok(json!({"files": files}))
    }

    /// The tree digest of the plan `caller` holds under `token`, left in place.
    pub(crate) fn kept_digest(
        &self,
        caller: &Caller,
        token: &str,
    ) -> Result<String, (i32, String)> {
        Ok(self.plan(caller, &json!({"token": token}), false)?.digest)
    }

    /// Gives up the plan `caller` holds under `token`, if it holds one.
    pub(crate) fn forget_plan(&self, caller: &Caller, token: &str) {
        let _ = self.plan(caller, &json!({"token": token}), true);
    }

    /// Whether the app recorded a skill by this name as one the person chose.
    pub(crate) fn chosen_skill(&self, name: &str) -> bool {
        if !valid_skill_name(name) {
            return false;
        }
        let Ok(root) = self.root() else {
            return false;
        };
        let key = name.to_ascii_lowercase();
        let Ok(bytes) = fs::read(root.join(".skills").join(format!("{key}.json"))) else {
            return false;
        };
        serde_json::from_slice::<Value>(&bytes)
            .ok()
            .and_then(|record| record.get("chosen")?.as_bool())
            .unwrap_or(false)
    }

    /// `overlay/apply` `{"token", "catalog"?}`: installs exactly what the check showed, the files at its commit checked against
    /// its tree digest as they are written. The overlay's folder is replaced whole.
    pub(crate) fn apply(&self, caller: &Caller, params: &Value) -> Outcome {
        let plan = self.plan(caller, params, true)?;
        let catalog = text_param(params, "catalog")?;
        if let Some(label) = catalog {
            checked_name(label)?;
        }
        let root = self.root()?;
        let gate = self.gate(&plan.name);
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let name = checked_name(&plan.name)?;
        let before = record_of(root, name);
        let label: Option<String> = catalog.map(str::to_owned).or_else(|| {
            before
                .as_ref()
                .filter(|record| {
                    record.get("source").and_then(Value::as_str) == Some(plan.source.as_str())
                })
                .and_then(|record| record.get("catalog")?.as_str().map(str::to_owned))
        });
        let cache = self.cache(name)?;
        let staging = root.join(format!(".staging-{name}"));
        let _ = fs::remove_dir_all(&staging);
        fs::create_dir_all(&staging)
            .map_err(|error| refused(format!("the overlay could not be written: {error}")))?;
        let written = self.write_files(&cache, &plan, &staging);
        if let Err(error) = written {
            let _ = fs::remove_dir_all(&staging);
            return Err(error);
        }
        let target = root.join(name);
        let old = root.join(format!(".old-{name}"));
        let _ = fs::remove_dir_all(&old);
        if target.exists() {
            fs::rename(&target, &old)
                .map_err(|error| refused(format!("the overlay could not be replaced: {error}")))?;
        }
        fs::rename(&staging, &target)
            .map_err(|error| refused(format!("the overlay could not be placed: {error}")))?;
        let _ = fs::remove_dir_all(&old);
        let mut record = json!({
            "source": plan.source,
            "commit": plan.commit,
            "digest": plan.digest,
            "files": plan.files,
            "skipped": plan.skipped,
            "installed": iso_now(),
        });
        if let Some(label) = &label {
            record["catalog"] = Value::String(label.clone());
        }
        let bytes =
            serde_json::to_vec_pretty(&record).map_err(|error| refused(error.to_string()))?;
        crate::replace::write_file(&root.join(format!("{name}.json")), &bytes, 0o600).map_err(
            |error| {
                refused(format!(
                    "the overlay's record could not be written: {error}"
                ))
            },
        )?;
        Ok(record)
    }

    fn write_files(&self, cache: &Path, plan: &Plan, staging: &Path) -> Result<(), (i32, String)> {
        let blobs = self.blobs(cache, &unique(plan.files.values()))?;
        let mut kept: Vec<(Vec<u8>, Vec<u8>)> = Vec::new();
        for (path, blob) in &plan.files {
            if !valid_relative_path(path) {
                return Err(refused(
                    "the source holds a name that is not a relative path",
                ));
            }
            let bytes = blobs
                .get(blob)
                .ok_or_else(|| refused("git did not give a file it was asked for"))?;
            let file = staging.join(path);
            if let Some(parent) = file.parent() {
                fs::create_dir_all(parent).map_err(|error| {
                    refused(format!("the overlay could not be written: {error}"))
                })?;
            }
            // Exclusive: a name the file system takes for one already written fails the install rather than replacing it.
            let mut out = fs::OpenOptions::new()
                .write(true)
                .create_new(true)
                .open(&file)
                .map_err(|error| refused(format!("the overlay could not be written: {error}")))?;
            out.write_all(bytes)
                .and_then(|()| out.sync_all())
                .map_err(|error| refused(format!("the overlay could not be written: {error}")))?;
            kept.push((path.clone().into_bytes(), bytes.clone()));
        }
        // The copy used is the copy hashed.
        let entries: Vec<(Vec<&[u8]>, TreeEntry)> = kept
            .iter()
            .map(|(path, bytes)| {
                (
                    path.split(|byte| *byte == b'/').collect(),
                    TreeEntry::File(bytes.clone()),
                )
            })
            .collect();
        let digest = digest_entries(
            entries
                .iter()
                .map(|(components, entry)| (components.clone(), entry)),
        )
        .map_err(|_| refused("the files read cannot be an overlay: nothing was installed"))?;
        if !same_digest(&digest.digest.to_string(), &plan.digest) {
            return Err(refused(
                "The files read do not have the digest shown: nothing was installed",
            ));
        }
        Ok(())
    }

    /// `overlay/remove` `{"name"}`: what `name` installed and fetched.
    pub(crate) fn remove(&self, params: &Value) -> Outcome {
        let name = checked_name(text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?)?;
        let root = self.root()?;
        let gate = self.gate(name);
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        // What was prepared is undone as the specification says: setup runs once more as `remove`, then its environments, state
        // directory and record go, whether it succeeded or not.
        let home = prepare::Home::new(root);
        if home.record(name).is_some()
            && let Some(digest) = record_of(root, name)
                .and_then(|record| Some(record.get("digest")?.as_str()?.to_owned()))
            && let Ok(plan) =
                prepare::Plan::of(name, &root.join(name), coqui_aos::current_os(), &digest)
        {
            let finder = crate::profile::Finder::new(
                &|variable| std::env::var(variable).ok(),
                &crate::sources::runtime_dirs(),
            );
            let _ = prepare::run(&plan, coqui_aos::Change::Remove, &home, &finder, &[]);
        }
        prepare::delete(&home, name);
        for dir in [root.join(name), self.cache(name)?] {
            if dir.exists() {
                fs::remove_dir_all(&dir).map_err(|error| {
                    refused(format!("the overlay could not be removed: {error}"))
                })?;
            }
        }
        let record = root.join(format!("{name}.json"));
        if record.exists() {
            fs::remove_file(&record)
                .map_err(|error| refused(format!("the overlay could not be removed: {error}")))?;
        }
        Ok(json!({"removed": true}))
    }

    /// `overlay/discard` `{"name"}`: what a check fetched for an overlay that was not installed.
    pub(crate) fn discard(&self, params: &Value) -> Outcome {
        let name = checked_name(text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?)?;
        let root = self.root()?;
        let gate = self.gate(name);
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        if record_of(root, name).is_none() {
            let cache = self.cache(name)?;
            if cache.exists() {
                let _ = fs::remove_dir_all(cache);
            }
        }
        Ok(json!({"discarded": true}))
    }
}

fn unique<'a>(ids: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    ids.into_iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

fn checked_name(name: &str) -> Result<&str, (i32, String)> {
    let shaped = !name.is_empty()
        && name.len() <= crate::profile::MAX_NAME
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '-' | '_'));
    if shaped && valid_overlay_name(name) {
        Ok(name)
    } else {
        Err(bad("a name has 1 to 64 letters, digits, - or _"))
    }
}

/// The source `params` name: `repo`, `ref` and `path` (empty for the root), held to what the host fetches, and `spec`, the text
/// canoa.cfg writes it as, which the record keeps (the canonical form when none is given).
fn git_source(params: &Value) -> Result<(String, String, String, String), (i32, String)> {
    let wrong = || bad("repo and ref name a git source this host fetches");
    let repo = text_param(params, "repo")?.ok_or_else(wrong)?;
    let reference = text_param(params, "ref")?.ok_or_else(wrong)?;
    let path = text_param(params, "path")?.unwrap_or("");
    let repo = repo.strip_prefix("git+").unwrap_or(repo);
    let canonical = if path.is_empty() {
        format!("git+{repo}@{reference}")
    } else {
        format!("git+{repo}@{reference}#{path}")
    };
    match OverlaySource::parse_git(&canonical) {
        Ok(OverlaySource::Git {
            repo,
            reference,
            path,
        }) => {
            let spec = text_param(params, "spec")?.map_or(canonical, str::to_owned);
            if spec.len() > 4096 || spec.chars().any(char::is_control) {
                return Err(bad("spec is a line of text"));
            }
            Ok((repo, reference, path, spec))
        }
        _ => Err(wrong()),
    }
}

fn pin_of(params: &Value) -> Result<Option<String>, (i32, String)> {
    match text_param(params, "pin")? {
        None => Ok(None),
        Some(pin) if coqui_aos::Digest::parse(pin).is_ok() => Ok(Some(pin.to_owned())),
        Some(_) => Err(bad("pin is sha256: and 64 hexadecimal digits")),
    }
}

fn same_digest(a: &str, b: &str) -> bool {
    matches!((coqui_aos::Digest::parse(a), coqui_aos::Digest::parse(b)), (Ok(a), Ok(b)) if a == b)
}

/// Whether a repository is a place on this computer, which only a key allowed to browse it may name.
fn is_local(repo: &str) -> bool {
    if let Some((scheme, _)) = repo.split_once("://") {
        return scheme == "file";
    }
    // An scp-like `host:path` goes to another computer; a drive letter or a path does not.
    let drive =
        repo.len() >= 2 && repo.as_bytes()[0].is_ascii_alphabetic() && repo.as_bytes()[1] == b':';
    drive || repo.starts_with('/') || repo.starts_with('.') || !repo.contains(':')
}

/// The folder a local `repo` names, for the check against the places a key may browse.
pub(crate) fn local_path(repo: &str) -> PathBuf {
    let path = repo.strip_prefix("file://").unwrap_or(repo);
    // `file:///C:/x` names `C:/x`.
    let bytes = path.as_bytes();
    if cfg!(windows)
        && bytes.len() >= 3
        && bytes[0] == b'/'
        && bytes[1].is_ascii_alphabetic()
        && bytes[2] == b':'
    {
        return PathBuf::from(&path[1..]);
    }
    PathBuf::from(path)
}

/// Whether `name` can be a skill's name, and so a file name of the app's own record of it.
pub(crate) fn valid_skill_name(name: &str) -> bool {
    name.len() <= 120
        && name.starts_with(|c: char| c.is_ascii_alphanumeric())
        && name
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || matches!(c, '_' | '.' | '-'))
        && !name.contains("..")
}

fn record_of(root: &Path, name: &str) -> Option<Value> {
    let bytes = fs::read(root.join(format!("{name}.json"))).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let object = value.as_object()?;
    (object.get("source")?.is_string()
        && object.get("commit")?.is_string()
        && object.get("files")?.is_object())
    .then_some(value)
}

pub(in crate::control) use git::find_git;

/// Now as an ISO 8601 time in UTC, to the second.
pub(crate) fn iso_now() -> String {
    crate::utc::iso_now()
}
