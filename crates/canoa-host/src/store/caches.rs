//! What the Store keeps so that opening it asks nobody: the publishers' newest versions on disk under the host's home, and the
//! answers of `store/models` and `store/catalog` in memory, each with a time to live of its own.
//!
//! The newest version of a program answers from disk while it is fresh; a stale one is answered at once and asked again in the
//! background, one request at a time per program, as the catalog index is. A publisher that does not say a version is not
//! remembered, so a failure is asked again. The other two caches answer within their time and are emptied by whatever changes
//! what they say (an install, a model's action).
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, threads and clocks are std's"
)]

use std::collections::{BTreeMap, BTreeSet};
use std::path::PathBuf;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use serde_json::{Value, json};

/// How long the publisher's newest version answers without asking, unless `[host.store] latest_ttl` says otherwise.
pub const DEFAULT_LATEST_TTL: Duration = Duration::from_secs(60 * 60);
/// How long a Store type's models answer without asking the program, unless `[host.store] models_ttl` says otherwise.
pub const DEFAULT_MODELS_TTL: Duration = Duration::from_secs(5 * 60);
/// How long a catalog's entries answer without asking the source, unless `[host.store] catalog_ttl` says otherwise.
pub const DEFAULT_CATALOG_TTL: Duration = Duration::from_secs(5 * 60);
/// How often, while a client is connected, the host asks again what has gone stale, unless `[host.store] refresh_every` says
/// otherwise. Zero never does.
pub const DEFAULT_REFRESH_EVERY: Duration = Duration::from_secs(5 * 60);
/// The longest a failed background request waits before it is tried again, whatever the time to live.
const RETRY: Duration = Duration::from_secs(60);
/// The form of `latest.json`.
const SCHEMA: u32 = 1;

/// A failure as the control channel words it: a code and a message.
pub(crate) type Failed = (i32, String);

/// The request that asks the publisher, run on the caller's thread or a background one.
pub(crate) type Fetch = Arc<dyn Fn() -> Result<Value, Failed> + Send + Sync>;

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

fn millis(time: Duration) -> u64 {
    u64::try_from(time.as_millis()).unwrap_or(u64::MAX)
}

/// Every cache of the Store, with the times that decide them.
#[derive(Debug)]
pub(crate) struct StoreCaches {
    pub(crate) latest: Arc<LatestCache>,
    pub(crate) models: TtlCache<canoa::store::models::ModelList>,
    pub(crate) catalog: TtlCache<Value>,
    /// How often the host asks again what has gone stale while a client is connected; zero never does.
    pub(crate) refresh_every: Duration,
}

impl StoreCaches {
    /// Caches that keep the newest versions under `dir` (none keeps them in memory only).
    pub(crate) fn new(
        dir: Option<PathBuf>,
        latest_ttl: Duration,
        models_ttl: Duration,
        catalog_ttl: Duration,
        refresh_every: Duration,
    ) -> Self {
        Self {
            latest: Arc::new(LatestCache::new(dir, latest_ttl)),
            models: TtlCache::new(models_ttl),
            catalog: TtlCache::new(catalog_ttl),
            refresh_every,
        }
    }

    /// Keeps nothing on disk and uses the default times.
    pub(crate) fn in_memory() -> Self {
        Self::new(
            None,
            DEFAULT_LATEST_TTL,
            DEFAULT_MODELS_TTL,
            DEFAULT_CATALOG_TTL,
            DEFAULT_REFRESH_EVERY,
        )
    }
}

#[cfg(test)]
impl StoreCaches {
    /// The times, in order: the newest versions, the models, the catalogs and the renewal.
    pub(crate) fn times(&self) -> [Duration; 4] {
        [
            self.latest.ttl,
            self.models.ttl,
            self.catalog.ttl,
            self.refresh_every,
        ]
    }
}

impl Default for StoreCaches {
    fn default() -> Self {
        Self::in_memory()
    }
}

/// Answers kept in memory for a time, each under a key.
#[derive(Debug)]
pub(crate) struct TtlCache<V> {
    ttl: Duration,
    held: Mutex<BTreeMap<String, (u64, V)>>,
}

impl<V: Clone> TtlCache<V> {
    fn new(ttl: Duration) -> Self {
        Self {
            ttl,
            held: Mutex::new(BTreeMap::new()),
        }
    }

    /// The answer kept under `key` if it is within the time and `refresh` is not asked; else `ask`'s, which is kept when it
    /// worked.
    pub(crate) fn get(
        &self,
        key: &str,
        refresh: bool,
        ask: impl FnOnce() -> Result<V, Failed>,
    ) -> Result<V, Failed> {
        let now = now_millis();
        if !refresh
            && !self.ttl.is_zero()
            && let Some((at, value)) = lock(&self.held).get(key)
            && now.saturating_sub(*at) < millis(self.ttl)
        {
            return Ok(value.clone());
        }
        let value = ask()?;
        if !self.ttl.is_zero() {
            lock(&self.held).insert(key.to_owned(), (now_millis(), value.clone()));
        }
        Ok(value)
    }

    /// Forgets what is kept, because something it says has changed.
    pub(crate) fn clear(&self) {
        lock(&self.held).clear();
    }
}

#[derive(Clone, Debug)]
struct Entry {
    report: Value,
    /// When the publisher last said it, in milliseconds since the epoch.
    checked: u64,
    /// When the publisher was last asked, whether it answered or not.
    attempted: u64,
}

#[derive(Debug, Default)]
struct State {
    loaded: bool,
    entries: BTreeMap<String, Entry>,
    running: BTreeSet<String>,
}

/// The newest version each program's publisher last said, by program and channel.
#[derive(Debug)]
pub(crate) struct LatestCache {
    file: Option<PathBuf>,
    ttl: Duration,
    state: Mutex<State>,
}

fn key_of(provider: &str, channel: Option<&str>) -> String {
    format!("{provider}\n{}", channel.unwrap_or_default())
}

fn split(key: &str) -> (String, Option<String>) {
    let (provider, channel) = key.split_once('\n').unwrap_or((key, ""));
    (
        provider.to_owned(),
        (!channel.is_empty()).then(|| channel.to_owned()),
    )
}

impl LatestCache {
    pub(crate) fn new(dir: Option<PathBuf>, ttl: Duration) -> Self {
        Self {
            file: dir.map(|dir| dir.join("latest.json")),
            ttl,
            state: Mutex::new(State::default()),
        }
    }

    fn load(&self, state: &mut State) {
        if state.loaded {
            return;
        }
        state.loaded = true;
        let Some(file) = &self.file else {
            return;
        };
        let Ok(bytes) = std::fs::read(file) else {
            return;
        };
        let Ok(value) = serde_json::from_slice::<Value>(&bytes) else {
            return;
        };
        if value.get("schema").and_then(Value::as_u64) != Some(u64::from(SCHEMA)) {
            return;
        }
        for (key, held) in value
            .get("entries")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
        {
            if let (Some(report), Some(checked)) = (
                held.get("report").filter(|report| report.is_object()),
                held.get("checked").and_then(Value::as_u64),
            ) {
                state.entries.insert(
                    key.clone(),
                    Entry {
                        report: report.clone(),
                        checked,
                        attempted: checked,
                    },
                );
            }
        }
    }

    fn save(&self, state: &State) {
        let Some(file) = &self.file else {
            return;
        };
        let entries: serde_json::Map<String, Value> = state
            .entries
            .iter()
            .map(|(key, entry)| {
                (
                    key.clone(),
                    json!({"report": entry.report, "checked": entry.checked}),
                )
            })
            .collect();
        let Ok(bytes) = serde_json::to_vec(&json!({"schema": SCHEMA, "entries": entries})) else {
            return;
        };
        if let Some(folder) = file.parent() {
            let _ = std::fs::create_dir_all(folder);
        }
        let _ = crate::replace::write_file_in_folder(file, &bytes, crate::replace::SHARED);
    }

    /// `entry`'s report with when it was said and whether that is past the time.
    fn shown(&self, entry: &Entry, now: u64) -> Value {
        let mut shown = entry.report.clone();
        shown["checked"] = json!(entry.checked);
        shown["stale"] = json!(now.saturating_sub(entry.checked) >= millis(self.ttl));
        shown
    }

    /// What is kept for the program and channel, fresh or stale, and nothing asked.
    pub(crate) fn peek(&self, provider: &str, channel: Option<&str>) -> Option<Value> {
        let mut state = lock(&self.state);
        self.load(&mut state);
        let entry = state.entries.get(&key_of(provider, channel))?;
        Some(self.shown(entry, now_millis()))
    }

    /// The newest version of the program: what is kept while it is fresh, what is kept at once when it is stale (the publisher
    /// is asked again in the background, one asker at a time per program), and the publisher's own answer when nothing is kept
    /// or `refresh` asks. A time of zero asks every time.
    pub(crate) fn get(
        self: &Arc<Self>,
        provider: &str,
        channel: Option<&str>,
        refresh: bool,
        fetch: Fetch,
    ) -> Result<Value, Failed> {
        let key = key_of(provider, channel);
        if !refresh && !self.ttl.is_zero() {
            let mut state = lock(&self.state);
            self.load(&mut state);
            if let Some(entry) = state.entries.get(&key) {
                let now = now_millis();
                let shown = self.shown(entry, now);
                if shown["stale"] == json!(true)
                    && now.saturating_sub(entry.attempted) >= millis(self.ttl.min(RETRY))
                    && state.running.insert(key.clone())
                {
                    drop(state);
                    self.ask_in_background(key, fetch);
                }
                return Ok(shown);
            }
        }
        self.ask(&key, &fetch)
    }

    /// Asks the publisher now and keeps a version it gives.
    fn ask(&self, key: &str, fetch: &Fetch) -> Result<Value, Failed> {
        let answer = fetch();
        let now = now_millis();
        let mut state = lock(&self.state);
        self.load(&mut state);
        match answer {
            Ok(report) if report.get("known") == Some(&json!(true)) => {
                let entry = Entry {
                    report,
                    checked: now,
                    attempted: now,
                };
                let shown = self.shown(&entry, now);
                state.entries.insert(key.to_owned(), entry);
                self.save(&state);
                Ok(shown)
            }
            other => {
                if let Some(entry) = state.entries.get_mut(key) {
                    entry.attempted = now;
                }
                other
            }
        }
    }

    fn ask_in_background(self: &Arc<Self>, key: String, fetch: Fetch) {
        let cache = Arc::clone(self);
        let spawned = std::thread::Builder::new()
            .name("store-latest".to_owned())
            .spawn({
                let key = key.clone();
                move || {
                    let _ = cache.ask(&key, &fetch);
                    lock(&cache.state).running.remove(&key);
                }
            });
        if spawned.is_err() {
            lock(&self.state).running.remove(&key);
        }
    }

    /// The programs and channels whose kept version is past the time and not being asked about.
    pub(crate) fn stale(&self) -> Vec<(String, Option<String>)> {
        let mut state = lock(&self.state);
        self.load(&mut state);
        let now = now_millis();
        state
            .entries
            .iter()
            .filter(|(key, entry)| {
                now.saturating_sub(entry.checked) >= millis(self.ttl)
                    && now.saturating_sub(entry.attempted) >= millis(self.ttl.min(RETRY))
                    && !state.running.contains(*key)
            })
            .map(|(key, _)| split(key))
            .collect()
    }

    /// Asks the publisher again about a program the cache holds, on this thread.
    pub(crate) fn renew(&self, provider: &str, channel: Option<&str>, fetch: &Fetch) {
        let key = key_of(provider, channel);
        if lock(&self.state).running.insert(key.clone()) {
            let _ = self.ask(&key, fetch);
            lock(&self.state).running.remove(&key);
        }
    }
}

#[cfg(test)]
#[path = "caches_tests.rs"]
mod tests;
