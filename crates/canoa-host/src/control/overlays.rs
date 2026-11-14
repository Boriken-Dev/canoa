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
use std::ffi::OsString;
use std::fs;
use std::io::{Read as _, Write as _};
use std::path::{Path, PathBuf};
use std::process::{Command, Stdio};
use std::sync::{Mutex, OnceLock, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use coqui_aos::{
    EXCLUDED_COMPONENTS, TreeEntry, digest_entries, valid_overlay_name, valid_relative_path,
};
use serde_json::{Value, json};

use super::{Caller, REFUSED, bad, text_param};
use crate::profile::{OverlaySource, full_commit_id};

type Outcome = Result<Value, (i32, String)>;

/// The most files one overlay may hold.
pub const DEFAULT_MAX_FILES: usize = 4096;
/// The most bytes its files may hold in all.
pub const DEFAULT_MAX_BYTES: u64 = 64 * 1024 * 1024;
/// The most entries a source's tree may list, excluded ones included.
const MAX_ENTRIES: usize = 16384;
/// The most bytes one fetch may write into the cache before it is ended.
const MAX_FETCH_BYTES: u64 = 256 * 1024 * 1024;
/// How long one git command may run before it is ended.
const GIT_TIME: Duration = Duration::from_secs(5 * 60);
/// What a plan is good for after it was made.
const PLAN_TIME: Duration = Duration::from_secs(15 * 60);
/// The plans one caller may hold; the oldest goes first.
const MAX_PLANS: usize = 16;
/// The most bytes of file contents one answer carries.
pub const MAX_READ_BYTES: u64 = 8 * 1024 * 1024;
/// The files one `catalog/read` may ask for.
const MAX_CATALOG_PATHS: usize = 4096;
/// The most characters of paths one `ls-tree` is given, since Windows ends a command line at 32767.
const PATH_BUDGET: usize = 8000;

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
    changing: Mutex<()>,
    git: OnceLock<Result<PathBuf, String>>,
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

    fn root(&self) -> Result<&Path, (i32, String)> {
        self.root.as_deref().ok_or_else(|| {
            refused("this host was not started with a canoa.cfg to keep overlays beside")
        })
    }

    fn git(&self) -> Result<&Path, (i32, String)> {
        self.git
            .get_or_init(find_git)
            .as_ref()
            .map(PathBuf::as_path)
            .map_err(|why| refused(why.clone()))
    }

    fn plans(&self) -> std::sync::MutexGuard<'_, Vec<Plan>> {
        self.plans.lock().unwrap_or_else(PoisonError::into_inner)
    }

    fn cache(&self, name: &str) -> Result<PathBuf, (i32, String)> {
        Ok(self.root()?.join(".cache").join(format!("{name}.git")))
    }

    fn catalog_cache(&self, label: &str) -> Result<PathBuf, (i32, String)> {
        Ok(self.root()?.join(".catalogs").join(format!("{label}.git")))
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

    /// `overlay/check` `{"name", "source", "pin"?}`: fetches `source` into the cache for `name` and says what installing it would
    /// change; nothing installed is touched. `local` says whether a source on this computer may be read.
    pub(crate) fn check(
        &self,
        caller: &Caller,
        params: &Value,
        local: &dyn Fn(&str) -> bool,
    ) -> Outcome {
        let name = checked_name(text_param(params, "name")?.ok_or_else(|| bad("name is needed"))?)?;
        let source = text_param(params, "source")?.ok_or_else(|| bad("source is needed"))?;
        let pin = pin_of(params)?;
        let OverlaySource::Git {
            repo,
            reference,
            path,
        } = OverlaySource::parse_git(source)
            .map_err(|_| bad("source is not a git source this host fetches"))?
        else {
            return Err(bad("source is not a git source this host fetches"));
        };
        if is_local(&repo) && !local(&repo) {
            return Err(refused(
                "a source on this computer must be in a folder this key may browse",
            ));
        }
        let _one = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let cache = self.cache(name)?;
        self.init_cache(&cache)?;
        let commit = self.fetch(&cache, &repo, &reference)?;
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
        plans.retain(|plan| plan.at.elapsed() < PLAN_TIME);
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
            source: source.to_owned(),
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
        plans.retain(|plan| plan.at.elapsed() < PLAN_TIME);
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

    /// `overlay/apply` `{"token", "catalog"?}`: installs exactly what the check showed, the files at its commit checked against
    /// its tree digest as they are written. The overlay's folder is replaced whole.
    pub(crate) fn apply(&self, caller: &Caller, params: &Value) -> Outcome {
        let plan = self.plan(caller, params, true)?;
        let catalog = text_param(params, "catalog")?;
        if let Some(label) = catalog {
            checked_name(label)?;
        }
        let root = self.root()?;
        let _one = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
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
        write_beside(&root.join(format!("{name}.json")), &bytes).map_err(|error| {
            refused(format!(
                "the overlay's record could not be written: {error}"
            ))
        })?;
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
        let _one = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
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
        let _one = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        if record_of(root, name).is_none() {
            let cache = self.cache(name)?;
            if cache.exists() {
                let _ = fs::remove_dir_all(cache);
            }
        }
        Ok(json!({"discarded": true}))
    }

    /// `catalog/fetch` `{"label", "source"}` -> `{"commit"}`: the catalog repository at its ref, in the cache for `label`.
    pub(crate) fn catalog_fetch(&self, params: &Value, local: &dyn Fn(&str) -> bool) -> Outcome {
        let label =
            checked_name(text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?)?;
        let source = text_param(params, "source")?.ok_or_else(|| bad("source is needed"))?;
        let OverlaySource::Git {
            repo, reference, ..
        } = OverlaySource::parse_git(source)
            .map_err(|_| bad("source is not a git source this host fetches"))?
        else {
            return Err(bad("source is not a git source this host fetches"));
        };
        if is_local(&repo) && !local(&repo) {
            return Err(refused(
                "a source on this computer must be in a folder this key may browse",
            ));
        }
        let _one = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        let cache = self.catalog_cache(label)?;
        self.init_cache(&cache)?;
        let commit = self.fetch(&cache, &repo, &reference)?;
        Ok(json!({"commit": commit}))
    }

    /// `catalog/read` `{"label", "commit", "paths", "maxBytes"}` -> `{"files": {path: {"size", "data"?}|null}}`: regular files of
    /// a fetched catalog at `commit`, looked up literally; `data` (base64) only for one of at most `maxBytes`.
    pub(crate) fn catalog_read(&self, params: &Value) -> Outcome {
        let label =
            checked_name(text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?)?;
        let commit = text_param(params, "commit")?.ok_or_else(|| bad("commit is needed"))?;
        if !full_commit_id(commit) {
            return Err(bad("commit is a whole commit id"));
        }
        let paths: Vec<&str> = params
            .get("paths")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("paths is a list"))?
            .iter()
            .map(|path| path.as_str().ok_or_else(|| bad("paths is a list of text")))
            .collect::<Result<_, _>>()?;
        if paths.len() > MAX_CATALOG_PATHS {
            return Err(bad("too many paths"));
        }
        let max = params
            .get("maxBytes")
            .and_then(Value::as_u64)
            .unwrap_or(MAX_READ_BYTES)
            .min(MAX_READ_BYTES);
        let cache = self.catalog_cache(label)?;
        let mut found: BTreeMap<String, (String, u64)> = BTreeMap::new();
        let valid: Vec<&str> = paths
            .iter()
            .copied()
            .filter(|path| valid_relative_path(path))
            .collect::<BTreeSet<_>>()
            .into_iter()
            .collect();
        let mut at = 0;
        while at < valid.len() {
            let mut length = 0;
            let mut chunk: Vec<&str> = Vec::new();
            while at < valid.len() && (chunk.is_empty() || length + valid[at].len() < PATH_BUDGET) {
                length += valid[at].len() + 1;
                chunk.push(valid[at]);
                at += 1;
            }
            let mut args: Vec<OsString> = vec![
                format!("--git-dir={}", cache.display()).into(),
                "ls-tree".into(),
                "-l".into(),
                "-z".into(),
                "--full-tree".into(),
                commit.into(),
                "--".into(),
            ];
            args.extend(chunk.iter().map(OsString::from));
            let out = self.run(&args, None, None, 4 * 1024 * 1024)?;
            let asked: BTreeSet<&str> = chunk.iter().copied().collect();
            for record in String::from_utf8_lossy(&out).split('\0') {
                let Some((head, path)) = record.split_once('\t') else {
                    continue;
                };
                if !asked.contains(path) {
                    continue;
                }
                let fields: Vec<&str> = head.split_whitespace().collect();
                if let [mode, "blob", id, size] = fields[..]
                    && matches!(mode, "100644" | "100755")
                    && let Ok(size) = size.parse::<u64>()
                {
                    found.insert(path.to_owned(), (id.to_owned(), size));
                }
            }
        }
        let wanted: Vec<String> = unique(
            found
                .values()
                .filter(|(_, size)| *size <= max)
                .map(|(id, _)| id),
        );
        let total: u64 = found
            .values()
            .filter(|(_, size)| *size <= max)
            .map(|(_, size)| size)
            .sum();
        if total > MAX_READ_BYTES.saturating_mul(2) {
            return Err(refused("those files hold more than one answer carries"));
        }
        let blobs = self.blobs(&cache, &wanted)?;
        let mut files = serde_json::Map::new();
        for path in &paths {
            let entry = match found.get(*path) {
                None => Value::Null,
                Some((_, size)) if *size > max => json!({"size": size}),
                Some((id, size)) => {
                    let bytes = blobs
                        .get(id)
                        .ok_or_else(|| refused("git did not give a file it was asked for"))?;
                    json!({"size": size, "data": data_encoding::BASE64.encode(bytes)})
                }
            };
            files.insert((*path).to_owned(), entry);
        }
        Ok(json!({"files": files}))
    }

    /// `catalog/remove` `{"label"}`.
    pub(crate) fn catalog_remove(&self, params: &Value) -> Outcome {
        let label =
            checked_name(text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?)?;
        let cache = self.catalog_cache(label)?;
        let _one = self.changing.lock().unwrap_or_else(PoisonError::into_inner);
        if cache.exists() {
            fs::remove_dir_all(cache)
                .map_err(|error| refused(format!("the catalog could not be removed: {error}")))?;
        }
        Ok(json!({"removed": true}))
    }

    fn init_cache(&self, cache: &Path) -> Result<(), (i32, String)> {
        if cache.exists() {
            return Ok(());
        }
        fs::create_dir_all(cache)
            .map_err(|error| refused(format!("the cache could not be made: {error}")))?;
        self.run(
            &[
                "init".into(),
                "--quiet".into(),
                "--bare".into(),
                "--template=".into(),
                cache.into(),
            ],
            None,
            None,
            1024 * 1024,
        )
        .map(drop)
    }

    /// Fetches only what `reference` names, one commit deep, and returns its commit. A whole commit id is that object and
    /// nothing else: a branch or tag carrying the same name never stands in for it.
    fn fetch(&self, cache: &Path, repo: &str, reference: &str) -> Result<String, (i32, String)> {
        let dir = format!("--git-dir={}", cache.display());
        let fetch = |wants: &[&str]| -> Result<Vec<u8>, (i32, String)> {
            let mut args: Vec<OsString> = vec![
                dir.clone().into(),
                "fetch".into(),
                "--quiet".into(),
                "--no-tags".into(),
                "--no-recurse-submodules".into(),
                "--depth=1".into(),
                "--".into(),
                repo.into(),
            ];
            args.extend(wants.iter().map(OsString::from));
            self.run(&args, None, Some(cache), 1024 * 1024)
        };
        if full_commit_id(reference) {
            if fetch(&[reference]).is_err() {
                // A server that refuses a want by id still serves its branches and tags, one of which may hold the commit.
                fetch(&[
                    "+refs/heads/*:refs/canoa/heads/*",
                    "+refs/tags/*:refs/canoa/tags/*",
                ])?;
            }
            let found = self.run(
                &[
                    dir.clone().into(),
                    "rev-parse".into(),
                    "--verify".into(),
                    "--quiet".into(),
                    "--end-of-options".into(),
                    format!("{reference}^{{commit}}").into(),
                ],
                None,
                None,
                4096,
            );
            return match found {
                Ok(out) if String::from_utf8_lossy(&out).trim() == reference => {
                    Ok(reference.to_owned())
                }
                _ => Err(refused("The source has no commit of that id")),
            };
        }
        let heads = format!("refs/heads/{reference}");
        let tags = format!("refs/tags/{reference}");
        let listed = self.run(
            &[
                "ls-remote".into(),
                "--heads".into(),
                "--tags".into(),
                "--".into(),
                repo.into(),
                heads.clone().into(),
                tags.clone().into(),
            ],
            None,
            None,
            1024 * 1024,
        )?;
        let text = String::from_utf8_lossy(&listed).into_owned();
        let names: BTreeSet<&str> = text
            .lines()
            .filter_map(|line| line.rsplit('\t').next())
            .collect();
        let remote = [heads, tags]
            .into_iter()
            .find(|name| names.contains(name.as_str()))
            .ok_or_else(|| refused("The source has no branch or tag of that name"))?;
        let local = format!("refs/canoa/{}", &remote["refs/".len()..]);
        fetch(&[&format!("+{remote}:{local}")])?;
        let found = self.run(
            &[
                dir.into(),
                "rev-parse".into(),
                "--verify".into(),
                "--end-of-options".into(),
                format!("{local}^{{commit}}").into(),
            ],
            None,
            None,
            4096,
        )?;
        Ok(String::from_utf8_lossy(&found).trim().to_owned())
    }

    /// Every entry under `path` at `commit`, read as git lists it and refused as soon as its files pass the limits, then
    /// digested as AOS defines: a link, a submodule, a name that is not a relative path or two names equal without ASCII case
    /// refuse the whole overlay. Entries under an excluded name are left out, never installed.
    fn list(&self, cache: &Path, commit: &str, path: &str) -> Result<Listing, (i32, String)> {
        let prefix: Vec<u8> = if path.is_empty() {
            Vec::new()
        } else {
            format!("{path}/").into_bytes()
        };
        let mut args: Vec<OsString> = vec![
            format!("--git-dir={}", cache.display()).into(),
            "ls-tree".into(),
            "-r".into(),
            "-l".into(),
            "-z".into(),
            "--full-tree".into(),
            commit.into(),
            "--".into(),
        ];
        if !path.is_empty() {
            args.push(path.into());
        }
        let out = self.run(&args, None, None, 64 * 1024 * 1024)?;
        struct Listed {
            components: Vec<Vec<u8>>,
            kind: Kind,
            blob: String,
        }
        #[derive(PartialEq, Eq, Clone, Copy)]
        enum Kind {
            File,
            Symlink,
            Special,
        }
        let excluded = |entry: &Listed| {
            entry.components.iter().any(|part| {
                EXCLUDED_COMPONENTS
                    .iter()
                    .any(|name| name.as_bytes() == part.as_slice())
            }) || (entry.kind == Kind::File
                && entry
                    .components
                    .last()
                    .is_some_and(|last| last.ends_with(b".pyc")))
        };
        let path_of = |entry: &Listed| {
            entry
                .components
                .iter()
                .map(|part| String::from_utf8_lossy(part).into_owned())
                .collect::<Vec<_>>()
                .join("/")
        };
        let mut entries: Vec<Listed> = Vec::new();
        let mut folded: BTreeMap<String, String> = BTreeMap::new();
        let (mut count, mut bytes) = (0_usize, 0_u64);
        for record in out
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty())
        {
            if entries.len() >= MAX_ENTRIES {
                return Err(refused(format!(
                    "The source holds more than {MAX_ENTRIES} entries"
                )));
            }
            let Some(tab) = record.iter().position(|byte| *byte == b'\t') else {
                continue;
            };
            let head = String::from_utf8_lossy(&record[..tab]).into_owned();
            let full = &record[tab + 1..];
            if full.len() <= prefix.len() || !full.starts_with(&prefix) {
                continue;
            }
            let relative = &full[prefix.len()..];
            let fields: Vec<&str> = head.split_whitespace().collect();
            let [mode, kind, blob, size] = fields[..] else {
                continue;
            };
            let kind = if mode == "120000" {
                Kind::Symlink
            } else if kind == "blob" {
                Kind::File
            } else {
                Kind::Special
            };
            let entry = Listed {
                components: relative
                    .split(|byte| *byte == b'/')
                    .map(<[u8]>::to_vec)
                    .collect(),
                kind,
                blob: blob.to_owned(),
            };
            let skip = excluded(&entry);
            if !skip && kind != Kind::File {
                return Err(refused(
                    "The source holds a link or a submodule, which no overlay has",
                ));
            }
            if !skip {
                // One directory on a file system that ignores case: two names it would merge refuse the overlay.
                let path = path_of(&entry);
                let names: Vec<&str> = path.split('/').collect();
                for end in 1..=names.len() {
                    let name = names[..end].join("/");
                    let first = folded
                        .entry(name.to_lowercase())
                        .or_insert_with(|| name.clone());
                    if *first != name {
                        return Err(refused(
                            "The source holds two names that differ only in case",
                        ));
                    }
                }
                count += 1;
                bytes += size.parse::<u64>().unwrap_or(0);
                if count > self.limits.files {
                    return Err(refused(format!(
                        "The overlay holds more than {} files",
                        self.limits.files
                    )));
                }
                if bytes > self.limits.bytes {
                    return Err(refused(format!(
                        "The overlay holds more than {} bytes",
                        self.limits.bytes
                    )));
                }
            }
            entries.push(entry);
        }
        let members: Vec<&Listed> = entries
            .iter()
            .filter(|entry| entry.kind == Kind::File && !excluded(entry))
            .collect();
        if members.is_empty() {
            return Err(refused("The source has no files at that path and commit"));
        }
        let ids: Vec<String> = unique(members.iter().map(|entry| &entry.blob));
        let blobs = self.blobs(cache, &ids)?;
        let tree: Vec<(Vec<&[u8]>, TreeEntry)> = entries
            .iter()
            .map(|entry| {
                let components: Vec<&[u8]> = entry.components.iter().map(Vec::as_slice).collect();
                let tree = match entry.kind {
                    Kind::File if excluded(entry) => TreeEntry::File(Vec::new()),
                    Kind::File => {
                        TreeEntry::File(blobs.get(&entry.blob).cloned().unwrap_or_default())
                    }
                    Kind::Symlink => TreeEntry::Symlink,
                    Kind::Special => TreeEntry::Special,
                };
                (components, tree)
            })
            .collect();
        let digest = digest_entries(
            tree.iter()
                .map(|(components, entry)| (components.clone(), entry)),
        )
        .map_err(|error| refused(format!("The source cannot be an overlay ({})", error.class)))?;
        let files = members
            .iter()
            .map(|entry| (path_of(entry), entry.blob.clone()))
            .collect();
        let skipped = entries
            .iter()
            .filter(|entry| excluded(entry))
            .map(&path_of)
            .collect();
        Ok((files, skipped, digest.digest.to_string(), bytes))
    }

    /// The bytes of each blob in `ids`, read from git's object store in one `cat-file --batch`, as stored.
    fn blobs(
        &self,
        cache: &Path,
        ids: &[String],
    ) -> Result<BTreeMap<String, Vec<u8>>, (i32, String)> {
        if ids.is_empty() {
            return Ok(BTreeMap::new());
        }
        if ids
            .iter()
            .any(|id| !id.bytes().all(|b| b.is_ascii_hexdigit()) || !matches!(id.len(), 40 | 64))
        {
            return Err(refused("git did not give an object id"));
        }
        let stdin: Vec<u8> = ids
            .iter()
            .flat_map(|id| format!("{id}\n").into_bytes())
            .collect();
        let ceiling = usize::try_from(self.limits.bytes.saturating_add(64 * 1024 * 1024))
            .unwrap_or(usize::MAX);
        let data = self.run(
            &[
                format!("--git-dir={}", cache.display()).into(),
                "cat-file".into(),
                "--batch".into(),
            ],
            Some(stdin),
            None,
            ceiling,
        )?;
        let mut out = BTreeMap::new();
        let mut at = 0;
        for id in ids {
            let end = data[at..]
                .iter()
                .position(|byte| *byte == b'\n')
                .map(|offset| at + offset)
                .ok_or_else(|| refused("git cat-file answered short"))?;
            let header = String::from_utf8_lossy(&data[at..end]).into_owned();
            let fields: Vec<&str> = header.split(' ').collect();
            let [name, kind, size] = fields[..] else {
                return Err(refused("git cat-file did not give a blob it was asked for"));
            };
            if name != id || kind != "blob" {
                return Err(refused("git cat-file did not give a blob it was asked for"));
            }
            let size: usize = size
                .parse()
                .map_err(|_| refused("git cat-file answered badly"))?;
            at = end + 1;
            if at + size + 1 > data.len() {
                return Err(refused("git cat-file answered short"));
            }
            out.insert(id.clone(), data[at..at + size].to_vec());
            at += size + 1;
        }
        Ok(out)
    }

    /// `git args`, with only the transports a source may name, nothing asked of the person and nothing from the source's
    /// configuration, ended after [`GIT_TIME`]; with `guard`, also ended once that folder holds more than [`MAX_FETCH_BYTES`].
    /// What git prints is returned up to `ceiling` bytes, past which the run is ended; a failure says only that it failed.
    fn run(
        &self,
        args: &[OsString],
        stdin: Option<Vec<u8>>,
        guard: Option<&Path>,
        ceiling: usize,
    ) -> Result<Vec<u8>, (i32, String)> {
        let git = self.git()?;
        let no_hooks = if cfg!(windows) {
            "core.hooksPath=NUL"
        } else {
            "core.hooksPath=/dev/null"
        };
        let mut command = Command::new(git);
        for (name, _) in std::env::vars_os() {
            if name
                .to_string_lossy()
                .to_ascii_uppercase()
                .starts_with("GIT_")
            {
                command.env_remove(name);
            }
        }
        command.args(["-c", "protocol.allow=never"]);
        for scheme in ["https", "http", "ssh", "git", "file"] {
            command.args(["-c", &format!("protocol.{scheme}.allow=always")]);
        }
        let mut child = command
            .args([
                "-c",
                no_hooks,
                "-c",
                "core.fsmonitor=false",
                "-c",
                "credential.interactive=never",
            ])
            .args(args)
            .env("GIT_TERMINAL_PROMPT", "0")
            .env("GCM_INTERACTIVE", "never")
            .env("SSH_ASKPASS_REQUIRE", "never")
            .env("GIT_LITERAL_PATHSPECS", "1")
            .stdin(if stdin.is_some() {
                Stdio::piped()
            } else {
                Stdio::null()
            })
            .stdout(Stdio::piped())
            .stderr(Stdio::piped())
            .spawn()
            .map_err(|_| refused("git could not be started"))?;
        let writer = stdin.and_then(|bytes| {
            child.stdin.take().map(|mut pipe| {
                std::thread::spawn(move || {
                    let _ = pipe.write_all(&bytes);
                })
            })
        });
        let complaint = child.stderr.take().map(|mut pipe| {
            std::thread::spawn(move || {
                let mut said = Vec::new();
                let _ = pipe.by_ref().take(2048).read_to_end(&mut said);
                // Drained, so git never waits on a full pipe.
                let _ = std::io::copy(&mut pipe, &mut std::io::sink());
                said
            })
        });
        let mut stdout = child.stdout.take();
        let past = std::sync::Arc::new(std::sync::atomic::AtomicBool::new(false));
        let raised = std::sync::Arc::clone(&past);
        let reader = std::thread::spawn(move || {
            let mut bytes = Vec::new();
            let mut chunk = [0_u8; 64 * 1024];
            let mut over = false;
            if let Some(stdout) = stdout.as_mut() {
                loop {
                    match stdout.read(&mut chunk) {
                        Ok(0) | Err(_) => break,
                        Ok(n) => {
                            if bytes.len() + n > ceiling {
                                over = true;
                                raised.store(true, std::sync::atomic::Ordering::SeqCst);
                                break;
                            }
                            bytes.extend_from_slice(&chunk[..n]);
                        }
                    }
                }
            }
            (bytes, over)
        });
        let started = Instant::now();
        let mut checked = Instant::now();
        let mut too_large = false;
        let status = loop {
            match child.try_wait() {
                Ok(Some(status)) => break Some(status),
                Ok(None) => {}
                Err(_) => break None,
            }
            if past.load(std::sync::atomic::Ordering::SeqCst) {
                // The reader stopped before git did: git wrote more than it may.
                let _ = child.kill();
                let _ = child.wait();
                break None;
            }
            if started.elapsed() > GIT_TIME {
                let _ = child.kill();
                let _ = child.wait();
                return Err(refused("git ran too long and was ended"));
            }
            if let Some(guard) = guard
                && checked.elapsed() > Duration::from_millis(500)
            {
                checked = Instant::now();
                if folder_size(guard) > MAX_FETCH_BYTES {
                    too_large = true;
                    let _ = child.kill();
                    let _ = child.wait();
                    break None;
                }
            }
            std::thread::sleep(Duration::from_millis(10));
        };
        if let Some(writer) = writer {
            let _ = writer.join();
        }
        let (bytes, over) = reader.join().unwrap_or_default();
        let said = complaint
            .and_then(|reader| reader.join().ok())
            .map(|said| {
                String::from_utf8_lossy(&said)
                    .chars()
                    .map(|c| if c.is_control() { ' ' } else { c })
                    .take(300)
                    .collect::<String>()
            })
            .unwrap_or_default();
        if too_large {
            return Err(refused(format!(
                "The source sent more than {MAX_FETCH_BYTES} bytes: nothing was installed"
            )));
        }
        match status {
            Some(status) if status.success() && !over => Ok(bytes),
            _ if over => Err(refused("git printed more than this host reads")),
            _ => {
                let word = args
                    .iter()
                    .find(|arg| !arg.to_string_lossy().starts_with('-'))
                    .map_or_else(String::new, |arg| arg.to_string_lossy().into_owned());
                Err(refused(format!("git {word} failed: {}", said.trim())))
            }
        }
    }
}

fn folder_size(dir: &Path) -> u64 {
    let mut size = 0;
    let mut pending = vec![dir.to_path_buf()];
    while let Some(next) = pending.pop() {
        let Ok(reader) = fs::read_dir(&next) else {
            continue;
        };
        for entry in reader.flatten() {
            match entry.file_type() {
                Ok(kind) if kind.is_dir() => pending.push(entry.path()),
                Ok(kind) if kind.is_file() => size += entry.metadata().map_or(0, |meta| meta.len()),
                _ => {}
            }
        }
    }
    size
}

fn unique<'a>(ids: impl IntoIterator<Item = &'a String>) -> Vec<String> {
    ids.into_iter()
        .cloned()
        .collect::<BTreeSet<_>>()
        .into_iter()
        .collect()
}

/// An overlay's or a catalog's name: 1 to 64 letters, digits, `-` or `_`, and an overlay name AOS takes.
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

fn record_of(root: &Path, name: &str) -> Option<Value> {
    let bytes = fs::read(root.join(format!("{name}.json"))).ok()?;
    let value: Value = serde_json::from_slice(&bytes).ok()?;
    let object = value.as_object()?;
    (object.get("source")?.is_string()
        && object.get("commit")?.is_string()
        && object.get("files")?.is_object())
    .then_some(value)
}

fn find_git() -> Result<PathBuf, String> {
    let names: &[&str] = if cfg!(windows) {
        &["git.exe", "git.cmd"]
    } else {
        &["git"]
    };
    let path = std::env::var_os("PATH").unwrap_or_default();
    for dir in std::env::split_paths(&path) {
        if !dir.is_absolute() {
            continue;
        }
        for name in names {
            let candidate = dir.join(name);
            if candidate.is_file() {
                return Ok(candidate);
            }
        }
    }
    Err("git is not in an absolute directory of PATH".to_owned())
}

fn write_beside(path: &Path, bytes: &[u8]) -> std::io::Result<()> {
    let mut beside = path.as_os_str().to_owned();
    beside.push(".new");
    let beside = PathBuf::from(beside);
    {
        let mut file = fs::File::create(&beside)?;
        file.write_all(bytes)?;
        file.sync_all()?;
    }
    fs::rename(&beside, path)
}

/// Now as an ISO 8601 time in UTC, to the second.
fn iso_now() -> String {
    let seconds = SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| i64::try_from(since.as_secs()).unwrap_or(0));
    let (days, rest) = (seconds.div_euclid(86_400), seconds.rem_euclid(86_400));
    // Civil date from days since 1970-01-01 (Howard Hinnant's algorithm).
    let z = days + 719_468;
    let era = z.div_euclid(146_097);
    let day_of_era = z.rem_euclid(146_097);
    let year_of_era =
        (day_of_era - day_of_era / 1460 + day_of_era / 36_524 - day_of_era / 146_096) / 365;
    let day_of_year = day_of_era - (365 * year_of_era + year_of_era / 4 - year_of_era / 100);
    let shifted = (5 * day_of_year + 2) / 153;
    let day = day_of_year - (153 * shifted + 2) / 5 + 1;
    let month = if shifted < 10 {
        shifted + 3
    } else {
        shifted - 9
    };
    let year = year_of_era + era * 400 + i64::from(month <= 2);
    format!(
        "{year:04}-{month:02}-{day:02}T{:02}:{:02}:{:02}Z",
        rest / 3600,
        rest % 3600 / 60,
        rest % 60
    )
}
