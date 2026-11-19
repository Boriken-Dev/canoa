//! What the host knows of the person's secrets: the providers that hold them (`[secrets]` of canoa.cfg), and a cache in memory of what
//! has been given for as long as the person chose. An agent's request for a secret by name is answered from here when it can be,
//! without asking the person, and the form that does ask offers how long the answer is kept.
//!
//! A value is kept no longer than the shorter of what the person chose and what its provider said of it, so a token that expires
//! is asked for again after it does. A value kept for one session ends with it. Nothing here writes a value to a log, an error or a
//! listing: the table the person sees holds names, where each came from and how long it has left.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks and the clock are std's"
)]

use std::collections::HashMap;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::{SystemTime, UNIX_EPOCH};

use canoa::secret_provider::{Keeping, Named, SecretProvider, name_problem};
use serde_json::{Map, Value, json};
use zeroize::Zeroizing;

use super::builtin::FORM;
use super::catalog::{Catalog, Listed};
use super::settings::{Entry, Keep, Settings};

/// Where `[secrets]` is read from each time it is needed, so a change of canoa.cfg applies at once.
pub(crate) type Source = Arc<dyn Fn() -> Result<Option<Value>, String> + Send + Sync>;

type Clock = Arc<dyn Fn() -> u64 + Send + Sync>;

/// How deep a setting that names another secret may lead: a vault opened by one that is opened by another.
const MAX_DEPTH: usize = 4;

/// Whose a cached secret is.
#[derive(Clone, Debug, PartialEq, Eq)]
enum Scope {
    /// The session that asked for it.
    Session(String),
    /// Every session, until Canoa closes or its end.
    Process,
}

struct Cached {
    value: Zeroizing<String>,
    scope: Scope,
    /// Milliseconds on the clock at which it stops being good; none when it lasts as long as Canoa does.
    until: Option<u64>,
    /// The entry that gave it, or `form` for what the person typed.
    entry: String,
}

/// How a row of the table is kept.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) enum Kept {
    /// In memory for one session.
    Session(String),
    /// In memory until Canoa closes or the time left runs out.
    Process,
    /// In a provider's own store.
    Stored,
}

/// One secret the person can see: where it is, what it is called and how long it has left, never what it is.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Row {
    pub(crate) provider: String,
    pub(crate) name: String,
    pub(crate) kept: Kept,
    /// Seconds until it stops being good, rounded up; none when it lasts as long as it is kept.
    pub(crate) expires_in: Option<u64>,
}

impl Row {
    pub(crate) fn json(&self) -> Value {
        let (scope, session) = match &self.kept {
            Kept::Session(id) => ("session", Some(id.as_str())),
            Kept::Process => ("process", None),
            Kept::Stored => ("stored", None),
        };
        json!({
            "provider": self.provider, "name": self.name, "kept": scope,
            "session": session, "expires_in": self.expires_in,
        })
    }
}

pub(crate) struct Vault {
    catalog: Mutex<Arc<Catalog>>,
    source: Mutex<Source>,
    cache: Mutex<HashMap<String, Cached>>,
    clock: Clock,
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn system_clock() -> Clock {
    Arc::new(|| {
        SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map_or(0, |since| {
                u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
            })
    })
}

/// A provider's answer: the value and the end it gave itself.
struct Answer {
    value: Zeroizing<String>,
    expires_in: Option<u64>,
}

impl Vault {
    /// A vault of the built-in providers and no canoa.cfg: the form answers, and nothing is kept.
    pub(crate) fn new() -> Arc<Self> {
        Self::with_clock(Catalog::built_in(), Arc::new(|| Ok(None)), system_clock())
    }

    pub(crate) fn with_clock(catalog: Catalog, source: Source, clock: Clock) -> Arc<Self> {
        Arc::new(Self {
            catalog: Mutex::new(Arc::new(catalog)),
            source: Mutex::new(source),
            cache: Mutex::new(HashMap::new()),
            clock,
        })
    }

    /// Gives the vault the providers this host offers and where `[secrets]` is.
    pub(crate) fn configure(&self, catalog: Catalog, source: Source) {
        *lock(&self.catalog) = Arc::new(catalog);
        *lock(&self.source) = source;
    }

    fn now(&self) -> u64 {
        (self.clock)()
    }

    fn settings(&self) -> Settings {
        let source = Arc::clone(&lock(&self.source));
        match source().and_then(|section| Settings::read(section.as_ref())) {
            Ok(settings) => settings,
            Err(why) => {
                tracing::warn!("[secrets] is not used: {why}");
                Settings::default()
            }
        }
    }

    fn catalog(&self) -> Arc<Catalog> {
        Arc::clone(&lock(&self.catalog))
    }

    /// What the cache holds, with what ran out dropped.
    fn live(&self) -> MutexGuard<'_, HashMap<String, Cached>> {
        let now = self.now();
        let mut cache = lock(&self.cache);
        cache.retain(|_, cached| cached.until.is_none_or(|until| until > now));
        cache
    }

    /// The most a request waits for the person, whatever the agent asks for.
    pub(crate) fn max_wait(&self) -> core::time::Duration {
        core::time::Duration::from_secs(self.settings().max_wait)
    }

    /// The ways a typed secret may be kept, the one `[secrets] keep` names first, and `Remember` when there is a provider to keep it.
    pub(crate) fn keeps(&self) -> Vec<Keep> {
        let settings = self.settings();
        let mut keeps = vec![
            Keep::Once,
            Keep::Session,
            Keep::Close,
            Keep::Ttl(15 * 60),
            Keep::Ttl(60 * 60),
            Keep::Ttl(8 * 60 * 60),
            Keep::Ttl(24 * 60 * 60),
        ];
        if let Some(at) = keeps.iter().position(|keep| *keep == settings.first) {
            let first = keeps.remove(at);
            keeps.insert(0, first);
        } else {
            keeps.insert(0, settings.first);
        }
        if self.store_entry(&settings).is_some() {
            keeps.push(Keep::Store);
        }
        keeps
    }

    /// The entry "remember" keeps in, when it exists and can keep.
    fn store_entry(&self, settings: &Settings) -> Option<(String, Entry)> {
        let name = settings.store.as_ref()?;
        let entry = settings.entries.get(name)?;
        self.catalog()
            .info(&entry.provider)
            .is_some_and(|info| info.can_store)
            .then(|| (name.clone(), entry.clone()))
    }

    /// The secret `name` for `session`, from the cache or from the provider that answers for it, never by asking the person. `None`
    /// when nothing holds it: the caller then asks.
    pub(crate) fn recall(&self, session: &str, name: &str) -> Option<Zeroizing<Vec<u8>>> {
        self.find(Some(session), name)
            .map(|value| Zeroizing::new(value.as_bytes().to_vec()))
    }

    fn find(&self, session: Option<&str>, name: &str) -> Option<Zeroizing<String>> {
        if name_problem(name).is_some() {
            return None;
        }
        if let Some(value) = self.cached(session, name) {
            return Some(value);
        }
        let settings = self.settings();
        let mut asked: Vec<&str> = vec![settings.entry_for(name)];
        if let Some(store) = settings.store.as_deref()
            && !asked.contains(&store)
        {
            asked.push(store);
        }
        for entry_name in asked {
            if let Some(answer) = self.ask_entry(&settings, entry_name, name, 0) {
                self.keep_given(session, name, entry_name, &settings, &answer);
                return Some(answer.value);
            }
        }
        None
    }

    fn cached(&self, session: Option<&str>, name: &str) -> Option<Zeroizing<String>> {
        let cache = self.live();
        let cached = cache.get(name)?;
        let visible = match (&cached.scope, session) {
            (Scope::Process, _) => true,
            (Scope::Session(owner), Some(session)) => owner == session,
            (Scope::Session(_), None) => false,
        };
        visible.then(|| cached.value.clone())
    }

    /// What entry `entry_name` gives for `name`. An entry whose provider needs a person gives nothing here: only the form asks.
    fn ask_entry(
        &self,
        settings: &Settings,
        entry_name: &str,
        name: &str,
        depth: usize,
    ) -> Option<Answer> {
        let entry = settings.entries.get(entry_name)?;
        let catalog = self.catalog();
        if catalog.info(&entry.provider)?.needs_person {
            return None;
        }
        let provider = self.instance(&catalog, settings, entry, depth)?;
        match provider.get(Named::new(name), super::spawner::spawner()) {
            Ok(held) => {
                if held.expires_in == Some(0) {
                    return None;
                }
                held.value.map(|value| Answer {
                    value: Zeroizing::new(value.expose().clone()),
                    expires_in: held.expires_in,
                })
            }
            Err(error) => {
                // The provider's words name what failed and never a value.
                tracing::info!(
                    entry = entry_name,
                    "a secret provider gave nothing: {}",
                    error.message()
                );
                None
            }
        }
    }

    /// An instance of `entry`'s provider, with each of its settings that names another secret given that secret.
    fn instance(
        &self,
        catalog: &Catalog,
        settings: &Settings,
        entry: &Entry,
        depth: usize,
    ) -> Option<Arc<dyn SecretProvider>> {
        let mut config = Map::new();
        for (key, value) in &entry.config {
            let resolved = match reference(value) {
                Some((from, name)) => {
                    if depth >= MAX_DEPTH {
                        tracing::info!("a setting names a secret through too many providers");
                        return None;
                    }
                    let held = self
                        .cached(None, name)
                        .map(|value| Answer {
                            value,
                            expires_in: None,
                        })
                        .or_else(|| self.ask_entry(settings, from, name, depth + 1));
                    let Some(held) = held else {
                        tracing::info!(setting = key, "a setting names a secret that is not held");
                        return None;
                    };
                    Value::String(held.value.to_string())
                }
                None => value.clone(),
            };
            config.insert(key.clone(), resolved);
        }
        match catalog.instance(&entry.provider, &config) {
            Ok(provider) => Some(provider),
            Err(why) => {
                tracing::info!(
                    provider = entry.provider,
                    "a secret provider cannot be made: {why}"
                );
                None
            }
        }
    }

    /// Keeps what a provider gave as its entry says, and never longer than the provider said.
    fn keep_given(
        &self,
        session: Option<&str>,
        name: &str,
        entry_name: &str,
        settings: &Settings,
        answer: &Answer,
    ) {
        let keep = settings
            .entries
            .get(entry_name)
            .map_or(Keep::Session, |entry| entry.keep);
        self.put_cache(
            session,
            name,
            entry_name,
            &answer.value,
            keep,
            answer.expires_in,
        );
    }

    fn put_cache(
        &self,
        session: Option<&str>,
        name: &str,
        entry_name: &str,
        value: &str,
        keep: Keep,
        provider_ttl: Option<u64>,
    ) {
        let (scope, ttl) = match (keep, session) {
            (Keep::Once | Keep::Store, _) => return,
            (Keep::Session, Some(session)) => (Scope::Session(session.to_owned()), None),
            // With no session to end it with, what is kept "for a session" is kept until Canoa closes.
            (Keep::Session | Keep::Close, None) | (Keep::Close, Some(_)) => (Scope::Process, None),
            (Keep::Ttl(seconds), _) => (Scope::Process, Some(seconds)),
        };
        // The shorter of what the person chose and what the provider said.
        let seconds = match (ttl, provider_ttl) {
            (Some(chosen), Some(said)) => Some(chosen.min(said)),
            (one, other) => one.or(other),
        };
        let now = self.now();
        let until = seconds.map(|seconds| now.saturating_add(seconds.saturating_mul(1000)));
        lock(&self.cache).insert(
            name.to_owned(),
            Cached {
                value: Zeroizing::new(value.to_owned()),
                scope,
                until,
                entry: entry_name.to_owned(),
            },
        );
    }

    /// A secret the person typed for `session`, kept as `keep` says (the id the form offered it by).
    ///
    /// # Errors
    ///
    /// Why it could not be kept, in words that hold no value.
    pub(crate) fn remember(
        &self,
        session: Option<&str>,
        name: &str,
        value: &[u8],
        keep: &str,
    ) -> Result<(), String> {
        let keep = Keep::parse(keep).ok_or("that is not a way to keep a secret")?;
        let value = std::str::from_utf8(value).map_err(|_| "the value is not text")?;
        if let Some(why) = name_problem(name) {
            return Err(why);
        }
        if keep == Keep::Store {
            let settings = self.settings();
            let (entry_name, entry) = self
                .store_entry(&settings)
                .ok_or("no provider is set to keep secrets")?;
            let catalog = self.catalog();
            let provider = self
                .instance(&catalog, &settings, &entry, 0)
                .ok_or("the provider that keeps secrets cannot be reached")?;
            provider
                .put(Keeping::new(name, value), super::spawner::spawner())
                .map_err(|error| error.message().to_owned())?;
            // What the store holds is read from it from now on, so no copy of an older value stays in memory.
            lock(&self.cache).remove(name);
            tracing::debug!(entry = entry_name, "a secret was kept in its store");
            return Ok(());
        }
        self.put_cache(session, name, FORM, value, keep, None);
        Ok(())
    }

    /// Ends what was kept for `session`.
    pub(crate) fn end_session(&self, session: &str) {
        lock(&self.cache).retain(
            |_, cached| !matches!(&cached.scope, Scope::Session(owner) if owner == session),
        );
    }

    /// The providers this host offers.
    pub(crate) fn providers(&self) -> Vec<Listed> {
        self.catalog().list()
    }

    /// The entries the settings hold, as the table names them.
    pub(crate) fn entries(&self) -> Vec<(String, Entry)> {
        self.settings().entries.into_iter().collect()
    }

    /// What the person can see: what is cached, and what a provider that keeps and lists says it keeps.
    pub(crate) fn rows(&self) -> Vec<Row> {
        let now = self.now();
        let mut rows: Vec<Row> = self
            .live()
            .iter()
            .map(|(name, cached)| Row {
                provider: cached.entry.clone(),
                name: name.clone(),
                kept: match &cached.scope {
                    Scope::Session(id) => Kept::Session(id.clone()),
                    Scope::Process => Kept::Process,
                },
                expires_in: cached
                    .until
                    .map(|until| until.saturating_sub(now).div_ceil(1000)),
            })
            .collect();
        let settings = self.settings();
        let catalog = self.catalog();
        for (entry_name, entry) in &settings.entries {
            let Some(info) = catalog.info(&entry.provider) else {
                continue;
            };
            if !(info.can_store && info.can_list) {
                continue;
            }
            let Some(provider) = self.instance(&catalog, &settings, entry, 0) else {
                continue;
            };
            let Ok(names) = provider.names(super::spawner::spawner()) else {
                continue;
            };
            for name in names.names {
                rows.push(Row {
                    provider: entry_name.clone(),
                    name,
                    kept: Kept::Stored,
                    expires_in: None,
                });
            }
        }
        rows.sort_by(|a, b| (&a.provider, &a.name).cmp(&(&b.provider, &b.name)));
        rows
    }

    /// The value of the secret `name` in `provider`'s entry, for the person who asked to see it.
    pub(crate) fn reveal(&self, provider: &str, name: &str) -> Option<Zeroizing<String>> {
        if let Some(cached) = self
            .live()
            .get(name)
            .filter(|cached| cached.entry == provider)
        {
            return Some(cached.value.clone());
        }
        let settings = self.settings();
        self.ask_entry(&settings, provider, name, 0)
            .map(|answer| answer.value)
    }

    /// Sets a secret by hand: in `provider`'s entry when it keeps, else in memory as `keep` says.
    ///
    /// # Errors
    ///
    /// Why it could not be set, in words that hold no value.
    pub(crate) fn set(
        &self,
        provider: &str,
        name: &str,
        value: &[u8],
        keep: &str,
    ) -> Result<(), String> {
        let settings = self.settings();
        let entry = settings
            .entries
            .get(provider)
            .ok_or_else(|| format!("there is no provider entry {provider}"))?;
        let info = self
            .catalog()
            .info(&entry.provider)
            .map(|info| (info.can_store, info.needs_person))
            .ok_or_else(|| format!("this host has no provider {}", entry.provider))?;
        if info.0 {
            let text = std::str::from_utf8(value).map_err(|_| "the value is not text")?;
            let catalog = self.catalog();
            let instance = self
                .instance(&catalog, &settings, entry, 0)
                .ok_or("that provider cannot be reached")?;
            instance
                .put(Keeping::new(name, text), super::spawner::spawner())
                .map_err(|error| error.message().to_owned())?;
            lock(&self.cache).remove(name);
            return Ok(());
        }
        if !info.1 {
            return Err(format!(
                "{provider} keeps nothing: it answers from what it holds"
            ));
        }
        self.remember(None, name, value, keep).map(|()| {
            // What the person typed by hand is the form's, whichever entry they chose to open it from.
            if let Some(cached) = lock(&self.cache).get_mut(name) {
                cached.entry = provider.to_owned();
            }
        })
    }

    /// Forgets the secret `name` of `provider`: what is cached, and what the provider keeps when it keeps.
    ///
    /// # Errors
    ///
    /// Why the provider could not forget it.
    pub(crate) fn forget(&self, provider: &str, name: &str) -> Result<bool, String> {
        let mut forgot = lock(&self.cache).remove(name).is_some();
        let settings = self.settings();
        let Some(entry) = settings.entries.get(provider) else {
            return Ok(forgot);
        };
        let catalog = self.catalog();
        if catalog
            .info(&entry.provider)
            .is_some_and(|info| info.can_store)
            && let Some(instance) = self.instance(&catalog, &settings, entry, 0)
        {
            instance
                .forget(Named::new(name), super::spawner::spawner())
                .map_err(|error| error.message().to_owned())?;
            forgot = true;
        }
        Ok(forgot)
    }
}

/// The secret a setting names: `{"from": entry, "name": secret}` and nothing else in the table.
fn reference(value: &Value) -> Option<(&str, &str)> {
    let table = value.as_object().filter(|table| table.len() == 2)?;
    Some((table.get("from")?.as_str()?, table.get("name")?.as_str()?))
}

impl core::fmt::Debug for Vault {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Vault")
            .field("cached", &lock(&self.cache).len())
            .finish_non_exhaustive()
    }
}

#[cfg(test)]
mod tests;
