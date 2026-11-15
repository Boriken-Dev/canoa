//! The index of every catalog this host lists: one normalized entry per overlay and skill, kept on disk under the host's own
//! home and answered from there while it is fresh. Nothing from a source runs; a source is read out of git's object store.
//!
//! A source is fetched at most once per refresh. A fresh index answers with no process started; a stale one answers at once
//! and is refreshed in the background, first by asking the remote which commit the ref names and fetching only when it moved.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, threads and clocks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::fs;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use coqui_aos::{OverlayTree, Settings, valid_overlay_name};
use serde_json::{Value, json};

use crate::control::Overlays;
use crate::profile::OverlaySource;

/// How long an index answers without asking anyone, unless `[host.store] index_ttl` says otherwise.
pub const DEFAULT_TTL: Duration = Duration::from_secs(5 * 60);
/// What the cached icons may take in all, unless `[host.store] index_cache_bytes` says otherwise.
pub const DEFAULT_CACHE_BYTES: u64 = 64 * 1024 * 1024;
/// The form of `index.json`; it is part of the folder name, so a reader never meets a form it does not know.
const SCHEMA: u32 = 1;
/// The most entries of one catalog the index keeps.
const MAX_ENTRIES: usize = 2000;
/// The largest catalog file read.
const CATALOG_BYTES: u64 = 4 * 1024 * 1024;
/// The largest manifest read for an entry's contents.
const MANIFEST_BYTES: u64 = 256 * 1024;
/// The paths one read asks for, so that one answer stays within what a read carries.
const MANIFEST_CHUNK: usize = 32;
const ICON_CHUNK: usize = 16;

/// A catalog the host's canoa.cfg lists: a git source whose `#path` names the catalog file, or a directory of overlays.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct CatalogSource {
    pub(crate) label: String,
    /// The source as canoa.cfg writes it.
    pub(crate) spec: String,
    pub(crate) repo: String,
    pub(crate) reference: String,
    pub(crate) path: String,
}

/// One `[catalogs]` row: the source, or why it is no source. A bad row still shows as a source with its reason.
pub(crate) type Row = Result<CatalogSource, (String, String)>;

/// Reads `[catalogs]`, a table of label to `<repository>@<ref>#<path>`, in the order of the labels.
pub(crate) fn parse_sources(section: &Value) -> Vec<Row> {
    let Some(table) = section.as_object() else {
        return Vec::new();
    };
    let mut seen = BTreeSet::new();
    table
        .iter()
        .map(|(label, value)| {
            let problem = |why: &str| (label.clone(), why.to_owned());
            if !valid_overlay_name(label) || !seen.insert(coqui_aos::normalize_name(label)) {
                return Err(problem(
                    "The label is not a name this host keeps a catalog under",
                ));
            }
            let spec = value.as_str().ok_or_else(|| {
                problem("It is not a git source, <repository>@<ref>#<catalog file>")
            })?;
            let canonical = if spec.starts_with("git+") {
                spec.to_owned()
            } else {
                format!("git+{spec}")
            };
            match OverlaySource::parse_git(&canonical) {
                Ok(OverlaySource::Git {
                    repo,
                    reference,
                    path,
                }) => Ok(CatalogSource {
                    label: label.clone(),
                    spec: spec.to_owned(),
                    repo,
                    reference,
                    path,
                }),
                _ => Err(problem(
                    "It is not a git source, <repository>@<ref>#<catalog file>",
                )),
            }
        })
        .collect()
}

/// What the index holds of one source.
#[derive(Clone, Debug)]
struct Cached {
    spec: String,
    commit: String,
    /// Milliseconds since the epoch of the last time the index was known to match the remote.
    checked: u64,
    /// Milliseconds since the epoch of the last time a refresh was tried, whether it worked or not.
    attempted: u64,
    name: Option<String>,
    description: Option<String>,
    entries: Arc<Vec<Value>>,
    /// Why the last refresh failed; the entries are then the last good ones.
    problem: Option<String>,
}

#[derive(Debug, Default)]
struct Slot {
    /// One refresh at a time.
    gate: Mutex<()>,
    state: Mutex<Option<Cached>>,
    background: AtomicBool,
}

/// What `store/index` answers for one source.
#[derive(Debug)]
pub(crate) struct Answer {
    pub(crate) source: Value,
    pub(crate) entries: Vec<Value>,
}

/// The catalogs' index and its cache.
#[derive(Debug)]
pub(crate) struct StoreIndex {
    /// `<home>/cache/store`; none keeps the index in memory only.
    dir: Option<PathBuf>,
    ttl: Duration,
    cache_bytes: u64,
    slots: Mutex<BTreeMap<String, Arc<Slot>>>,
}

impl Default for StoreIndex {
    fn default() -> Self {
        Self::new(None, DEFAULT_TTL, DEFAULT_CACHE_BYTES)
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

impl StoreIndex {
    pub(crate) fn new(dir: Option<PathBuf>, ttl: Duration, cache_bytes: u64) -> Self {
        Self {
            dir,
            ttl,
            cache_bytes,
            slots: Mutex::new(BTreeMap::new()),
        }
    }

    fn slot(&self, label: &str) -> Arc<Slot> {
        Arc::clone(lock(&self.slots).entry(label.to_owned()).or_default())
    }

    /// Whether any source is being refreshed in the background.
    #[cfg(test)]
    pub(crate) fn refreshing(&self) -> bool {
        lock(&self.slots)
            .values()
            .any(|slot| slot.background.load(Ordering::SeqCst))
    }

    /// The answer for each row. `refresh` makes every source be asked and, when it moved, fetched before this returns.
    pub(crate) fn catalogs(
        self: &Arc<Self>,
        overlays: &Arc<Overlays>,
        rows: &[Row],
        refresh: bool,
    ) -> Vec<Answer> {
        rows.iter()
            .map(|row| match row {
                Ok(source) => self.view(overlays, source, refresh),
                Err((label, why)) => Answer {
                    source: json!({
                        "id": format!("catalog:{label}"),
                        "kind": "catalog",
                        "label": label,
                        "stale": true,
                        "problem": why,
                    }),
                    entries: Vec::new(),
                },
            })
            .collect()
    }

    fn view(
        self: &Arc<Self>,
        overlays: &Arc<Overlays>,
        source: &CatalogSource,
        refresh: bool,
    ) -> Answer {
        let slot = self.slot(&source.label);
        let mut held = lock(&slot.state).clone();
        if held.is_none() {
            held = self.read_disk(source);
            if let Some(cached) = &held {
                *lock(&slot.state) = Some(cached.clone());
            }
        }
        // A source that canoa.cfg now writes otherwise is another source: nothing of the old one answers for it.
        let held = held.filter(|cached| cached.spec == source.spec);
        let ttl = u64::try_from(self.ttl.as_millis()).unwrap_or(u64::MAX);
        let now = now_millis();
        match held {
            Some(cached)
                if !refresh
                    && cached.problem.is_none()
                    && now.saturating_sub(cached.checked) < ttl =>
            {
                answer_of(
                    source,
                    &cached,
                    false,
                    slot.background.load(Ordering::SeqCst),
                )
            }
            Some(cached) if !refresh => {
                let due = now.saturating_sub(cached.attempted) >= ttl;
                let started = due && self.refresh_in_background(&slot, overlays, source);
                let running = started || slot.background.load(Ordering::SeqCst);
                answer_of(source, &cached, true, running)
            }
            _ => {
                let cached = self.refresh(&slot, overlays, source);
                answer_of(source, &cached, cached.problem.is_some(), false)
            }
        }
    }

    /// Starts a refresh on its own thread unless one is running. Whether this call started it.
    fn refresh_in_background(
        self: &Arc<Self>,
        slot: &Arc<Slot>,
        overlays: &Arc<Overlays>,
        source: &CatalogSource,
    ) -> bool {
        if slot
            .background
            .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
            .is_err()
        {
            return false;
        }
        let (index, slot, overlays, source) = (
            Arc::clone(self),
            Arc::clone(slot),
            Arc::clone(overlays),
            source.clone(),
        );
        let spawned = std::thread::Builder::new()
            .name("store-index".to_owned())
            .spawn({
                let slot = Arc::clone(&slot);
                move || {
                    let _ = index.refresh(&slot, &overlays, &source);
                    slot.background.store(false, Ordering::SeqCst);
                }
            });
        if spawned.is_err() {
            slot.background.store(false, Ordering::SeqCst);
            return false;
        }
        true
    }

    /// Asks the remote and, when the ref moved, fetches and indexes it. One at a time per source: a caller that waited behind
    /// another's refresh takes its result.
    fn refresh(&self, slot: &Slot, overlays: &Overlays, source: &CatalogSource) -> Cached {
        let started = now_millis();
        let _one = lock(&slot.gate);
        let prior = lock(&slot.state)
            .clone()
            .filter(|cached| cached.spec == source.spec);
        if let Some(prior) = &prior
            && prior.attempted >= started
        {
            return prior.clone();
        }
        let attempt = now_millis();
        let next = match self.refresh_once(overlays, source, prior.as_ref(), attempt) {
            Ok(next) => next,
            Err(why) => match prior {
                Some(mut kept) => {
                    kept.attempted = attempt;
                    kept.problem = Some(why);
                    kept
                }
                None => Cached {
                    spec: source.spec.clone(),
                    commit: String::new(),
                    checked: 0,
                    attempted: attempt,
                    name: None,
                    description: None,
                    entries: Arc::new(Vec::new()),
                    problem: Some(why),
                },
            },
        };
        *lock(&slot.state) = Some(next.clone());
        next
    }

    fn refresh_once(
        &self,
        overlays: &Overlays,
        source: &CatalogSource,
        prior: Option<&Cached>,
        attempt: u64,
    ) -> Result<Cached, String> {
        let revision = overlays
            .remote_revision(&source.repo, &source.reference)
            .map_err(|(_, why)| why)?;
        if let Some(prior) = prior
            && prior.problem.is_none()
            && !prior.commit.is_empty()
            && prior.commit == revision
        {
            let mut same = prior.clone();
            same.checked = attempt;
            same.attempted = attempt;
            self.write(source, &same);
            return Ok(same);
        }
        let fetched = overlays
            .catalog_fetch(
                &json!({"label": source.label, "repo": source.repo, "ref": source.reference, "path": source.path}),
                &|_| true,
            )
            .map_err(|(_, why)| why)?;
        let commit = fetched
            .get("commit")
            .and_then(Value::as_str)
            .ok_or("the fetch named no commit")?
            .to_owned();
        let built = self.build(overlays, source, &commit)?;
        let next = Cached {
            spec: source.spec.clone(),
            commit,
            checked: attempt,
            attempted: attempt,
            name: built.name,
            description: built.description,
            entries: Arc::new(built.entries),
            problem: None,
        };
        self.write(source, &next);
        self.prune(source, &next.commit);
        Ok(next)
    }

    /// The folder of a source's index: the form's number and the label, which is a valid overlay name.
    fn source_dir(&self, source: &CatalogSource) -> Option<PathBuf> {
        Some(
            self.dir
                .as_ref()?
                .join(format!("s{SCHEMA}-catalog-{}", source.label)),
        )
    }

    /// Writes `index.json` for the commit beside a temporary file and renames it over, so a stop leaves the old one whole.
    fn write(&self, source: &CatalogSource, cached: &Cached) {
        let Some(dir) = self.source_dir(source) else {
            return;
        };
        let folder = dir.join(&cached.commit);
        if crate::control::private_dir(&folder).is_err() {
            return;
        }
        let text = json!({
            "schema": SCHEMA,
            "spec": cached.spec,
            "commit": cached.commit,
            "checked": cached.checked,
            "name": cached.name,
            "description": cached.description,
            "entries": &*cached.entries,
        });
        let Ok(bytes) = serde_json::to_vec(&text) else {
            return;
        };
        let beside = folder.join("index.json.new");
        if fs::write(&beside, bytes).is_ok()
            && fs::rename(&beside, folder.join("index.json")).is_err()
        {
            let _ = fs::remove_file(&beside);
        }
    }

    /// The newest index on disk for the source, if one reads.
    fn read_disk(&self, source: &CatalogSource) -> Option<Cached> {
        let dir = self.source_dir(source)?;
        let mut best: Option<Cached> = None;
        for found in fs::read_dir(&dir).ok()?.flatten() {
            let Ok(bytes) = fs::read(found.path().join("index.json")) else {
                continue;
            };
            let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
                continue;
            };
            if value.get("schema").and_then(Value::as_u64) != Some(u64::from(SCHEMA)) {
                continue;
            }
            let text = |key: &str| value.get(key).and_then(Value::as_str).map(str::to_owned);
            let (Some(spec), Some(commit)) = (text("spec"), text("commit")) else {
                continue;
            };
            let checked = value.get("checked").and_then(Value::as_u64).unwrap_or(0);
            if best.as_ref().is_some_and(|best| best.checked >= checked) {
                continue;
            }
            best = Some(Cached {
                spec,
                commit,
                checked,
                attempted: checked,
                name: text("name"),
                description: text("description"),
                entries: Arc::new(
                    value
                        .get("entries")
                        .and_then(Value::as_array)
                        .cloned()
                        .unwrap_or_default(),
                ),
                problem: None,
            });
        }
        best
    }

    /// Keeps the newest commit's folder of a source and removes the older ones, so the cache does not grow with every change.
    fn prune(&self, source: &CatalogSource, keep: &str) {
        let Some(dir) = self.source_dir(source) else {
            return;
        };
        let Ok(reader) = fs::read_dir(&dir) else {
            return;
        };
        for found in reader.flatten() {
            let name = found.file_name();
            let Some(name) = name.to_str() else { continue };
            let commit_like = (40..=64).contains(&name.len())
                && name.bytes().all(|byte| byte.is_ascii_hexdigit());
            if name != keep && commit_like && found.path().is_dir() {
                let _ = fs::remove_dir_all(found.path());
            }
        }
    }

    /// The icon bytes of an entry, for an answer that asks for them.
    pub(crate) fn icon_bytes(
        &self,
        source: &CatalogSource,
        commit: &str,
        file: &str,
    ) -> Option<Vec<u8>> {
        if file.contains(['/', '\\']) || file.contains("..") {
            return None;
        }
        fs::read(
            self.source_dir(source)?
                .join(commit)
                .join("icons")
                .join(file),
        )
        .ok()
    }

    fn build(
        &self,
        overlays: &Overlays,
        source: &CatalogSource,
        commit: &str,
    ) -> Result<Built, String> {
        let at = |paths: &[String], max: u64, list: Option<&str>| {
            read_files(overlays, &source.label, commit, paths, max, list)
        };
        let (files, dirs) = at(
            std::slice::from_ref(&source.path),
            CATALOG_BYTES,
            Some(&source.path),
        )?;
        let base = format!("git+{}@{}#{}", source.repo, source.reference, source.path);
        let mut listed: Vec<Listed> = Vec::new();
        let (mut name, mut description) = (None, None);
        match files.get(&source.path) {
            Some(Some((size, _))) if *size > CATALOG_BYTES => {
                return Err(format!(
                    "Its file holds {size} bytes, more than the {CATALOG_BYTES} this host reads"
                ));
            }
            Some(Some((_, Some(bytes)))) => {
                let read =
                    coqui_aos::read_catalog(bytes, &source.path, Some(&base), &Settings::default())
                        .map_err(|error| {
                            format!("It is not a catalog Canoa reads ({})", error.class)
                        })?;
                (name, description) = (read.name.clone(), read.description.clone());
                for (entry_name, entry) in &read.overlays.entries {
                    listed.push(Listed::of("overlay", entry_name, entry, source));
                }
                if let Some(skills) = &read.skills {
                    for (entry_name, entry) in &skills.entries {
                        listed.push(Listed::of("skill", entry_name, entry, source));
                    }
                }
                for (refused, class) in read
                    .overlays
                    .refused
                    .iter()
                    .chain(read.skills.iter().flat_map(|skills| &skills.refused))
                {
                    listed.push(Listed::refused(refused, class.as_str()));
                }
            }
            _ => {
                // A directory is a catalog in its minimal form: each subdirectory is an overlay of that name.
                let Some(Some(names)) = dirs else {
                    return Err(format!("It has no file {} at that commit", source.path));
                };
                for entry_name in names {
                    if !valid_overlay_name(&entry_name) {
                        listed.push(Listed::refused(&entry_name, "invalid-name"));
                        continue;
                    }
                    let folder = if source.path.is_empty() {
                        entry_name.clone()
                    } else {
                        format!("{}/{entry_name}", source.path)
                    };
                    listed.push(Listed {
                        kind: "overlay",
                        name: entry_name,
                        here: Some(folder.clone()),
                        base: Some(folder.clone()),
                        shown: format!("{}@{}#{folder}", source.repo, source.reference),
                        origin: json!({"kind": "git", "location": source.repo, "ref": source.reference, "path": folder}),
                        pin: None,
                        description: None,
                        about: None,
                        refusal: None,
                    });
                }
            }
        }
        listed.sort_by(|a, b| (a.kind, &a.name).cmp(&(b.kind, &b.name)));
        listed.truncate(MAX_ENTRIES);
        // A relative icon is relative to the catalog file's folder.
        let folder = source
            .path
            .rsplit_once('/')
            .map_or("", |(folder, _)| folder);
        for item in &mut listed {
            if item.base.is_none() {
                item.base = Some(folder.to_owned());
            }
        }
        let manifests = self.read_manifests(&at, &listed);
        let icons = self.read_icons(&at, &listed);
        let mut entries = Vec::new();
        let mut icon_budget = self.icon_room();
        let icon_dir = self
            .source_dir(source)
            .map(|dir| dir.join(commit).join("icons"));
        for (index, item) in listed.iter().enumerate() {
            let mut entry = json!({
                "id": format!("{}:{}:{}", source.label, item.kind, item.name),
                "kind": item.kind,
                "catalog": source.label,
                "name": item.name,
                "summary": item.summary(),
                "source": item.shown,
                "origin": item.origin,
                "version": Value::Null,
                "tags": item.about_list("tags"),
                "requirements": [],
                "pin": item.pin,
                "contents": Value::Null,
                "icon": Value::Null,
            });
            for field in ["homepage", "repository", "license"] {
                if let Some(text) = item.about_text(field) {
                    entry[field] = Value::String(text);
                }
            }
            if let Some(authors) = item
                .about_list("authors")
                .as_array()
                .filter(|list| !list.is_empty())
            {
                entry["authors"] = Value::Array(authors.clone());
            }
            if let Some(why) = &item.refusal {
                entry["problem"] =
                    Value::String(format!("The catalog's entry was refused ({why})"));
            }
            if let Some(Ok(read)) = manifests.get(&index) {
                entry["contents"] = contents_of(&read.manifest);
                entry["requirements"] = list_of(read.manifest.get("requires"));
            } else if let Some(Err(why)) = manifests.get(&index) {
                entry["problem"] = Value::String(why.clone());
            }
            if let Some(Ok((path, bytes))) = icons.get(&index)
                && let Some(icon_dir) = &icon_dir
                && (bytes.len() as u64) <= icon_budget
                && let Ok(icon) = coqui_aos::read_icon(bytes, path, &Settings::default())
            {
                let file = format!("icon-{index}.{}", icon.format.as_str());
                if crate::control::private_dir(icon_dir).is_ok()
                    && fs::write(icon_dir.join(&file), bytes).is_ok()
                {
                    icon_budget = icon_budget.saturating_sub(bytes.len() as u64);
                    entry["icon"] = json!({
                        "file": file,
                        "format": icon.format.as_str(),
                        "width": icon.width,
                        "height": icon.height,
                        "bytes": bytes.len(),
                    });
                }
            }
            entries.push(entry);
        }
        Ok(Built {
            name,
            description,
            entries,
        })
    }

    /// What the icon cache may still take.
    fn icon_room(&self) -> u64 {
        self.dir
            .as_deref()
            .map_or(0, |dir| self.cache_bytes.saturating_sub(folder_bytes(dir)))
    }

    fn read_manifests(
        &self,
        at: &Reader,
        listed: &[Listed],
    ) -> BTreeMap<usize, Result<coqui_aos::OverlayRead, String>> {
        let mut out = BTreeMap::new();
        let wanted: Vec<(usize, &str)> = listed
            .iter()
            .enumerate()
            .filter(|(_, item)| item.kind == "overlay")
            .filter_map(|(index, item)| Some((index, item.here.as_deref()?)))
            .collect();
        for chunk in wanted.chunks(MANIFEST_CHUNK) {
            let paths: Vec<String> = chunk
                .iter()
                .flat_map(|(_, folder)| {
                    ["OVERLAY.md", "overlay.cfg"].map(|name| join_path(folder, name))
                })
                .collect();
            let files = match at(&paths, MANIFEST_BYTES, None) {
                Ok((files, _)) => files,
                Err(why) => {
                    for (index, _) in chunk {
                        out.insert(
                            *index,
                            Err(format!("Its manifest could not be read ({why})")),
                        );
                    }
                    continue;
                }
            };
            for (index, folder) in chunk {
                let found: Vec<(&str, &Vec<u8>)> = ["OVERLAY.md", "overlay.cfg"]
                    .into_iter()
                    .filter_map(|name| match files.get(&join_path(folder, name)) {
                        Some(Some((_, Some(bytes)))) => Some((name, bytes)),
                        _ => None,
                    })
                    .collect();
                let Some(&(name, bytes)) = found.first() else {
                    out.insert(
                        *index,
                        Err("It has no OVERLAY.md or overlay.cfg".to_owned()),
                    );
                    continue;
                };
                if found.len() > 1 {
                    out.insert(
                        *index,
                        Err("It has both OVERLAY.md and overlay.cfg".to_owned()),
                    );
                    continue;
                }
                let tree =
                    OverlayTree::from_texts([(name, String::from_utf8_lossy(bytes).into_owned())]);
                out.insert(
                    *index,
                    coqui_aos::read_overlay(&tree, &Settings::default()).map_err(|error| {
                        format!("Its manifest is not one Canoa reads ({})", error.class)
                    }),
                );
            }
        }
        out
    }

    fn read_icons(
        &self,
        at: &Reader,
        listed: &[Listed],
    ) -> BTreeMap<usize, Result<(String, Vec<u8>), String>> {
        let mut out = BTreeMap::new();
        if self.dir.is_none() {
            return out;
        }
        let wanted: Vec<(usize, String)> = listed
            .iter()
            .enumerate()
            .filter_map(|(index, item)| {
                let icon = item.about_text("icon")?;
                Some((index, join_path(item.base.as_deref()?, &icon)))
            })
            .collect();
        for chunk in wanted.chunks(ICON_CHUNK) {
            let paths: Vec<String> = chunk.iter().map(|(_, path)| path.clone()).collect();
            let Ok((files, _)) = at(&paths, 512 * 1024, None) else {
                continue;
            };
            for (index, path) in chunk {
                if let Some(Some((_, Some(bytes)))) = files.get(path) {
                    out.insert(*index, Ok((path.clone(), bytes.clone())));
                }
            }
        }
        out
    }
}

type Files = (
    BTreeMap<String, Option<(u64, Option<Vec<u8>>)>>,
    Option<Option<Vec<String>>>,
);

/// Reads files of the catalog at its commit: the paths, the most bytes of one, and a directory to list.
type Reader<'a> = dyn Fn(&[String], u64, Option<&str>) -> Result<Files, String> + 'a;

struct Built {
    name: Option<String>,
    description: Option<String>,
    entries: Vec<Value>,
}

/// An entry of a catalog on its way to the index.
struct Listed {
    kind: &'static str,
    name: String,
    /// Its folder in the catalog's own repository, when that is where it lives.
    here: Option<String>,
    /// Where the catalog's own folder is, for the paths of fields that are relative to it.
    base: Option<String>,
    shown: String,
    /// The source in parts, without user information or query, so a client need not guess what kind of source the text names.
    origin: Value,
    pin: Option<String>,
    description: Option<String>,
    about: Option<serde_json::Map<String, Value>>,
    refusal: Option<String>,
}

impl Listed {
    fn of(
        kind: &'static str,
        name: &str,
        entry: &coqui_aos::CatalogEntry,
        catalog: &CatalogSource,
    ) -> Self {
        // An entry that lives in the catalog's own repository is read from it; one elsewhere is the catalog's claim only.
        let here = (entry.source.kind == coqui_aos::SourceKind::Git
            && entry.source.location == catalog.repo
            && entry.source.reference.as_deref() == Some(catalog.reference.as_str()))
        .then(|| entry.source.path.clone().unwrap_or_default());
        Self {
            kind,
            name: name.to_owned(),
            here,
            base: None,
            shown: entry.source.shown(),
            origin: origin_of(&entry.source),
            pin: entry.pin.clone(),
            description: entry.description.clone(),
            about: entry.about.clone(),
            refusal: None,
        }
    }

    fn refused(name: &str, class: &str) -> Self {
        Self {
            kind: "overlay",
            name: name.to_owned(),
            here: None,
            base: None,
            shown: String::new(),
            origin: Value::Null,
            pin: None,
            description: None,
            about: None,
            refusal: Some(class.to_owned()),
        }
    }

    fn summary(&self) -> Value {
        self.about_text("summary")
            .or_else(|| self.description.clone())
            .map_or(Value::Null, Value::String)
    }

    fn about_text(&self, field: &str) -> Option<String> {
        self.about.as_ref()?.get(field)?.as_str().map(str::to_owned)
    }

    fn about_list(&self, field: &str) -> Value {
        list_of(self.about.as_ref().and_then(|about| about.get(field)))
    }
}

fn origin_of(source: &coqui_aos::Source) -> Value {
    let kind = match source.kind {
        coqui_aos::SourceKind::Git => "git",
        coqui_aos::SourceKind::Url => "url",
        _ => "directory",
    };
    let mut bare = source.clone();
    bare.path = None;
    json!({
        "kind": kind,
        "location": bare.shown_with(None),
        "ref": source.reference,
        "path": source.path,
    })
}

fn list_of(value: Option<&Value>) -> Value {
    Value::Array(
        value
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|item| item.as_str().map(|text| Value::String(text.to_owned())))
                    .collect()
            })
            .unwrap_or_default(),
    )
}

fn join_path(folder: &str, name: &str) -> String {
    if folder.is_empty() {
        name.to_owned()
    } else {
        format!("{folder}/{name}")
    }
}

/// What an overlay's manifest says it carries: skills, hooks, environment scripts and MCP servers, by name.
fn contents_of(manifest: &serde_json::Map<String, Value>) -> Value {
    let names = |value: Option<&Value>, key: &str| -> Vec<Value> {
        value
            .and_then(Value::as_array)
            .map(|list| {
                list.iter()
                    .filter_map(|item| match item {
                        Value::String(text) => Some(text.clone()),
                        Value::Object(map) => {
                            map.get(key).and_then(Value::as_str).map(str::to_owned)
                        }
                        _ => None,
                    })
                    .map(Value::String)
                    .collect()
            })
            .unwrap_or_default()
    };
    let hooks: Vec<Value> = manifest
        .get("hooks")
        .and_then(Value::as_object)
        .map(|hooks| {
            hooks
                .iter()
                .filter(|(_, hook)| **hook != Value::Bool(false))
                .map(|(event, _)| Value::String(event.clone()))
                .collect()
        })
        .unwrap_or_default();
    let servers = manifest
        .get("mcp")
        .and_then(Value::as_object)
        .and_then(|mcp| mcp.get("servers"));
    json!({
        "skills": names(manifest.get("skills"), "name"),
        "hooks": hooks,
        "env_scripts": names(manifest.get("env_scripts"), "run"),
        "mcp_servers": names(servers, "name"),
    })
}

/// `catalog/read` for these paths at `commit`: for each path its size and, when it is within `max`, its bytes; and, when `list`
/// names a directory, the names of its subdirectories.
fn read_files(
    overlays: &Overlays,
    label: &str,
    commit: &str,
    paths: &[String],
    max: u64,
    list: Option<&str>,
) -> Result<Files, String> {
    let mut params = json!({"label": label, "commit": commit, "paths": paths, "maxBytes": max});
    if let Some(list) = list {
        params["list"] = Value::String(list.to_owned());
    }
    let answer = overlays.catalog_read(&params).map_err(|(_, why)| why)?;
    let mut files = BTreeMap::new();
    for (path, found) in answer
        .get("files")
        .and_then(Value::as_object)
        .into_iter()
        .flatten()
    {
        let entry = found.as_object().map(|found| {
            let size = found.get("size").and_then(Value::as_u64).unwrap_or(0);
            let data = found
                .get("data")
                .and_then(Value::as_str)
                .and_then(|text| data_encoding::BASE64.decode(text.as_bytes()).ok());
            (size, data)
        });
        files.insert(path.clone(), entry);
    }
    let dirs = answer.get("directories").map(|dirs| {
        dirs.as_array().map(|names| {
            names
                .iter()
                .filter_map(|name| name.as_str().map(str::to_owned))
                .collect()
        })
    });
    Ok((files, dirs))
}

/// The bytes the files under `dir` hold.
fn folder_bytes(dir: &Path) -> u64 {
    let Ok(reader) = fs::read_dir(dir) else {
        return 0;
    };
    reader
        .flatten()
        .map(|found| match found.metadata() {
            Ok(meta) if meta.is_dir() => folder_bytes(&found.path()),
            Ok(meta) => meta.len(),
            Err(_) => 0,
        })
        .sum()
}

fn answer_of(source: &CatalogSource, cached: &Cached, stale: bool, refreshing: bool) -> Answer {
    let mut shown = json!({
        "id": format!("catalog:{}", source.label),
        "kind": "catalog",
        "label": source.label,
        "spec": source.spec,
        "commit": cached.commit,
        "checked": cached.checked,
        "stale": stale,
        "refreshing": refreshing,
        "name": cached.name,
        "description": cached.description,
    });
    if let Some(why) = &cached.problem {
        shown["problem"] = Value::String(why.clone());
    }
    Answer {
        source: shown,
        entries: cached.entries.as_ref().clone(),
    }
}
