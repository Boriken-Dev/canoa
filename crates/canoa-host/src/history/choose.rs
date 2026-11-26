//! Which `history_store` provider keeps the histories, and its opening.
//!
//! The host names no store: a provider is one a loaded library offers, and the one used is the one `[host.history] store` names, else
//! the one that says it is the default. A store a newer build made is opened read-only and said so.

use std::path::Path;
use std::sync::Arc;

use canoa::history_store::schema::{self, Fit};
use canoa::history_store::{
    Aggregate, Compacted, HistoryDb, HistoryStore, Migration, OpenRequest, Rows, SchemaState,
    Select, Sql, Stats, StoreInfo, Write, Written,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Map, Value};

use crate::config::HistoryConfig;

/// An opened store the host's parts share.
#[derive(Clone)]
pub(crate) struct Db(Arc<dyn HistoryDb>);

impl Db {
    /// `db` as the store the host's parts share.
    pub(crate) fn new(db: impl HistoryDb + 'static) -> Self {
        Self(Arc::new(db))
    }
}

impl core::ops::Deref for Db {
    type Target = dyn HistoryDb;

    fn deref(&self) -> &Self::Target {
        &*self.0
    }
}

impl core::fmt::Debug for Db {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("Db")
    }
}

/// A provider's opened store, as the trait the host calls.
struct Opened(Object<dyn HistoryDb>);

impl HistoryDb for Opened {
    fn schema(&self) -> Result<SchemaState, ProviderError> {
        self.0.schema()
    }
    fn migrate(&self, migration: Migration) -> Result<(), ProviderError> {
        self.0.migrate(migration)
    }
    fn select(&self, select: Select) -> Result<Rows, ProviderError> {
        self.0.select(select)
    }
    fn count(&self, select: Select) -> Result<u64, ProviderError> {
        self.0.count(select)
    }
    fn aggregate(&self, aggregate: Aggregate) -> Result<Rows, ProviderError> {
        self.0.aggregate(aggregate)
    }
    fn write(&self, write: Write) -> Result<Written, ProviderError> {
        self.0.write(write)
    }
    fn sql(&self, sql: Sql) -> Result<Rows, ProviderError> {
        self.0.sql(sql)
    }
    fn revision(&self) -> Result<String, ProviderError> {
        self.0.revision()
    }
    fn stats(&self) -> Result<Stats, ProviderError> {
        self.0.stats()
    }
    fn compact(&self) -> Result<Compacted, ProviderError> {
        self.0.compact()
    }
    fn close(&self) -> Result<(), ProviderError> {
        self.0.close()
    }
}

/// What opening the store came to.
pub(crate) struct Chosen {
    /// The store.
    pub(crate) db: Db,
    /// This build may not change it: a newer build made its schema.
    pub(crate) read_only: bool,
    /// What the person is told about it, when there is something to say.
    pub(crate) notice: Option<String>,
}

/// The provider `wanted` names, else the one that says it is the default: its registry, key and description.
fn find(
    registries: &[Arc<Registry>],
    wanted: Option<&str>,
) -> Option<(Arc<Registry>, String, StoreInfo)> {
    let mut default = None;
    for registry in registries {
        let keys: Vec<String> = registry
            .offers::<dyn HistoryStore>()
            .filter_map(|offer| offer.key().map(str::to_owned))
            .collect();
        for key in keys {
            let Some(Ok(offer)) = registry.offer::<dyn HistoryStore>(&key) else {
                continue;
            };
            let Some(info) = offer
                .default_instance()
                .and_then(|store| store.describe().ok())
            else {
                continue;
            };
            if let Some(problem) = info.problem() {
                tracing::warn!(provider = key, "a history_store is not offered: {problem}");
                continue;
            }
            if wanted == Some(info.id.as_str()) {
                return Some((Arc::clone(registry), key, info));
            }
            if wanted.is_none() && info.default && default.is_none() {
                default = Some((Arc::clone(registry), key, info));
            }
        }
    }
    default
}

fn open_one(
    registry: &Registry,
    key: &str,
    settings: &str,
    home: &Path,
    create: bool,
) -> Result<Db, String> {
    let offer = registry
        .offer::<dyn HistoryStore>(key)
        .ok_or("the history store is gone")?
        .map_err(|_| "it is not a history_store provider".to_owned())?;
    let provider = offer
        .default_instance()
        .ok_or("the history store takes provider-level settings")?;
    let mut request = OpenRequest::new(settings, home.to_string_lossy());
    request.create = create;
    let opened = provider
        .open(request)
        .map_err(|error| error.message.to_string())?;
    Ok(Db::new(Opened(opened)))
}

/// Opens the store `config` names (or the default one) under `home`, writes the schema into a new one and says what a newer one is.
///
/// # Errors
///
/// No provider is offered, or the one chosen cannot open, and why.
pub(crate) fn open(
    home: &Path,
    config: &HistoryConfig,
    registries: &[Arc<Registry>],
) -> Result<Chosen, String> {
    let (registry, key, info) = find(registries, config.store.as_deref()).ok_or_else(|| {
        config.store.as_deref().map_or_else(
            || "no history store is offered".to_owned(),
            |id| format!("the history store {id} is not offered"),
        )
    })?;
    let settings = if config.store_settings.trim().is_empty() {
        "{}"
    } else {
        &config.store_settings
    };
    let db = open_one(&registry, &key, settings, home, true)?;
    let version = db
        .schema()
        .map_err(|error| error.message.to_string())?
        .version;
    match schema::fit(version) {
        Fit::Empty => {
            if !info.foreign {
                db.migrate(schema::v1())
                    .map_err(|error| error.message.to_string())?;
            }
            Ok(Chosen {
                db,
                read_only: false,
                notice: None,
            })
        }
        Fit::Current | Fit::Older(_) => Ok(Chosen {
            db,
            read_only: false,
            notice: None,
        }),
        Fit::Newer(stored) => {
            let _ = db.close();
            let mut map: Map<String, Value> = serde_json::from_str(settings).unwrap_or_default();
            map.insert("read_only".to_owned(), Value::Bool(true));
            let db = open_one(&registry, &key, &Value::Object(map).to_string(), home, false)
                .map_err(|why| {
                    format!(
                        "the {} store was made by a newer version (schema {stored}, this one writes {}) and cannot be opened read-only: {why}",
                        info.id,
                        schema::VERSION
                    )
                })?;
            let notice = format!(
                "the {} store was made by a newer version (schema {stored}; this one writes {}): it is read, and nothing is recorded into it",
                info.id,
                schema::VERSION
            );
            tracing::warn!("{notice}");
            Ok(Chosen {
                db,
                read_only: true,
                notice: Some(notice),
            })
        }
    }
}
