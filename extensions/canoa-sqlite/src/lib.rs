//! SQLite for Canoa: the `history_store` provider that keeps recorded conversations, the list of them and the properties of every turn
//! in one database file.
//!
//! [`library`] is what a Canoa host loads. SQLite is built into the binary from its amalgamation, so there is nothing to install; the
//! file is `sessions.sqlite3` in the host's home folder unless the settings name another.
#![forbid(unsafe_code)]

mod db;

use std::path::PathBuf;

use canoa::history_store::{
    Access, CAP_BLOBS, CAP_COMPACT, CAP_SQL, HistoryDb, HistoryStore, Limits, OpenRequest,
    StoreError, StoreInfo,
};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::{Map, Value};

pub use db::{Journal, Options, SqliteDb, Synchronous};

/// The name this crate's library is registered under.
pub const LIBRARY: &str = "canoa-sqlite";

/// The id of the provider, which `canoa.cfg` names it by.
pub const ID: &str = "sqlite";

/// The file's name in the host's home folder when the settings name none.
pub const FILE: &str = "sessions.sqlite3";

/// The table that holds each table's description, beside the tables the host asks for.
pub(crate) const HELD: &str = "canoa_tables";

/// How the file is made durable.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum JournalSetting {
    /// A write-ahead log: readers and the writer do not wait for each other.
    Wal,
    /// A rollback journal.
    Delete,
}

/// How often SQLite waits for the disk.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum SynchronousSetting {
    /// Never: fastest, and a crash of the machine may lose the last writes.
    Off,
    /// At the moments that matter for a write-ahead log.
    Normal,
    /// After every transaction.
    Full,
}

/// The settings of the `sqlite` store.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, Default)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[allow(dead_code, reason = "read via schema/form generation and JSON parsing")]
struct SqliteSettings {
    #[schema(title = "File")]
    #[form(placeholder = "sessions.sqlite3")]
    path: Option<String>,
    #[schema(advanced, title = "Wait for another writer (ms)")]
    busy_timeout_ms: Option<u64>,
    #[schema(advanced, title = "Journal")]
    journal: Option<JournalSetting>,
    #[schema(advanced, title = "Synchronous")]
    synchronous: Option<SynchronousSetting>,
    #[schema(advanced, title = "Open read-only")]
    read_only: Option<bool>,
}

fn bad(why: impl Into<String>) -> ProviderError {
    StoreError::Bad(why.into()).into()
}

/// The options an [`OpenRequest`] asks for.
///
/// # Errors
///
/// The settings are not a JSON object of the keys the store takes.
pub fn options(request: &OpenRequest) -> Result<Options, ProviderError> {
    let text = if request.settings.trim().is_empty() {
        "{}"
    } else {
        &request.settings
    };
    let map: Map<String, Value> =
        serde_json::from_str(text).map_err(|_| bad("the settings are not a JSON object"))?;
    if let Some(key) = map.keys().find(|key| {
        ![
            "path",
            "busy_timeout_ms",
            "journal",
            "synchronous",
            "read_only",
        ]
        .contains(&key.as_str())
    }) {
        return Err(bad(format!("the sqlite store has no setting {key}")));
    }
    let get_str = |key: &str| -> Result<Option<&str>, ProviderError> {
        match map.get(key) {
            None | Some(Value::Null) => Ok(None),
            Some(Value::String(text)) => Ok(Some(text)),
            Some(_) => Err(bad(format!("{key} is text"))),
        }
    };
    let home = PathBuf::from(&request.home);
    let path = match get_str("path")? {
        Some(path) if !path.trim().is_empty() => {
            let path = PathBuf::from(path);
            if path.is_absolute() {
                path
            } else {
                home.join(path)
            }
        }
        _ => home.join(FILE),
    };
    let busy_timeout_ms = match map.get("busy_timeout_ms") {
        None | Some(Value::Null) => 5000,
        Some(value) => value
            .as_u64()
            .ok_or_else(|| bad("busy_timeout_ms is a whole number of milliseconds"))?,
    };
    let journal = match get_str("journal")? {
        None | Some("wal") => Journal::Wal,
        Some("delete") => Journal::Delete,
        Some(_) => return Err(bad("journal is wal or delete")),
    };
    let synchronous = match get_str("synchronous")? {
        None | Some("normal") => Synchronous::Normal,
        Some("full") => Synchronous::Full,
        Some("off") => Synchronous::Off,
        Some(_) => return Err(bad("synchronous is off, normal or full")),
    };
    let read_only = match map.get("read_only") {
        None | Some(Value::Null) => false,
        Some(Value::Bool(flag)) => *flag,
        Some(_) => return Err(bad("read_only is true or false")),
    };
    Ok(Options {
        path,
        busy_timeout_ms,
        journal,
        synchronous,
        read_only,
        create: request.create && !read_only,
        reply_bytes: usize::try_from(request.reply_bytes).unwrap_or(usize::MAX),
    })
}

/// The `sqlite` history store.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(HistoryStore, id = "sqlite", crate = ::coqui_acp_api::guatiao)]
pub struct SqliteStore;

impl HistoryStore for SqliteStore {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        let mut info = StoreInfo::new(ID, "SQLite");
        "One database file, built into Canoa: nothing to install.".clone_into(&mut info.summary);
        "A file named sessions.sqlite3 in Canoa's home folder, unless you choose another. It holds every conversation in plain \
         text, protected by the folder's permissions, and is not encrypted."
            .clone_into(&mut info.location);
        info.access = Access::ReadWrite;
        info.default = true;
        info.capabilities = vec![
            CAP_SQL.to_owned(),
            CAP_BLOBS.to_owned(),
            CAP_COMPACT.to_owned(),
        ];
        info.dialect = Some("sqlite".to_owned());
        info.limits = Limits {
            max_value_bytes: Some(1_000_000_000),
            max_rows: None,
        };
        info.taking::<SqliteSettings>()
    }

    fn open(&self, request: OpenRequest) -> Result<Object<dyn HistoryDb>, ProviderError> {
        let options = options(&request)?;
        Ok(SqliteDb::open(&options)?.into_object())
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-sqlite",
    version = env!("CARGO_PKG_VERSION"),
    providers = [SqliteStore]
);
