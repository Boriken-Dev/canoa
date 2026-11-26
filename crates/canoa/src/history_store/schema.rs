//! The schema a host writes into a store, as data: [`v1`] is the [`Migration`] that creates it, and [`tables`] its tables.
//!
//! Times are `int` milliseconds since the Unix epoch. A column the host may not know is nullable, never zero: a figure a provider did
//! not report is not a measured zero.

use alloc::string::ToString as _;
use alloc::vec::Vec;

use super::{ColumnInfo, ColumnType, IndexInfo, Migration, Step, TableInfo};

/// The version of the schema [`v1`] creates.
pub const VERSION: u32 = 1;

/// The table of facts about the store itself: `schema_version`, `store_id`, `created_at`.
pub const META: &str = "meta";
/// One row per recorded conversation.
pub const SESSIONS: &str = "sessions";
/// One row per turn of a conversation, with the properties of the turn.
pub const TURNS: &str = "turns";
/// One row per line of a conversation.
pub const EVENTS: &str = "events";
/// Content kept once by digest.
pub const ATTACHMENTS: &str = "attachments";
/// Which lines refer to which attachment.
pub const EVENT_ATTACHMENTS: &str = "event_attachments";
/// The list of conversations each client shows.
pub const LISTINGS: &str = "listings";
/// What an agent offered, by client.
pub const SOURCE_MEMORY: &str = "source_memory";
/// A client's favourite models.
pub const FAVOURITES: &str = "favourites";

use ColumnType::{Blob, Bool, Int, Json, Text};

struct Builder {
    table: TableInfo,
}

impl Builder {
    fn new(name: &str, key: &[&str]) -> Self {
        Self {
            table: TableInfo {
                name: name.to_string(),
                columns: Vec::new(),
                key: key.iter().map(ToString::to_string).collect(),
                indexes: Vec::new(),
                requires: Vec::new(),
            },
        }
    }

    fn col(mut self, name: &str, kind: ColumnType) -> Self {
        self.table.columns.push(ColumnInfo::new(name, kind));
        self
    }

    fn opt(mut self, name: &str, kind: ColumnType) -> Self {
        self.table.columns.push(ColumnInfo::nullable(name, kind));
        self
    }

    fn index(mut self, columns: &[&str], unique: bool) -> Self {
        let name = alloc::format!("{}_{}", self.table.name, columns.join("_"));
        self.table.indexes.push(IndexInfo {
            name,
            columns: columns.iter().map(ToString::to_string).collect(),
            unique,
        });
        self
    }

    fn requires(mut self, columns: &[&str]) -> Self {
        self.table.requires = columns.iter().map(ToString::to_string).collect();
        self
    }

    fn done(self) -> TableInfo {
        self.table
    }
}

/// The tables of schema version 1.
///
/// `turns` carries the properties of a turn as columns (who served it, what it cost, how it ended) and `tags`, an open map
/// namespaced by whoever set each tag, so a stat per model, role, effort or profile is one aggregate and a tag nobody planned for is
/// kept.
#[must_use]
pub fn tables() -> Vec<TableInfo> {
    alloc::vec![
        Builder::new(META, &["key"])
            .col("key", Text)
            .col("value", Text)
            .done(),
        Builder::new(SESSIONS, &["id"])
            .col("id", Text)
            .col("entry", Text)
            .col("agent_session", Text)
            .opt("harness", Text)
            .col("origin", Text)
            .opt("imported_from", Text)
            .opt("title", Text)
            .opt("cwd", Text)
            .opt("model", Text)
            .opt("mode", Text)
            .opt("effort", Text)
            .opt("acp_version", Int)
            .col("created_at", Int)
            .col("updated_at", Int)
            .col("header", Text)
            .opt("redacted", Json)
            .col("turns", Int)
            .col("events", Int)
            .col("bytes", Int)
            .col("open_turn", Bool)
            .opt("last_stop", Text)
            .opt("stopped", Text)
            .index(&["entry", "agent_session"], true)
            .index(&["updated_at"], false)
            .index(&["imported_from", "updated_at"], false)
            .done(),
        Builder::new(TURNS, &["session_id", "no"])
            .col("session_id", Text)
            .col("no", Int)
            .col("first_seq", Int)
            .col("last_seq", Int)
            .col("bytes", Int)
            .opt("started_at", Int)
            .opt("ended_at", Int)
            .opt("stop", Text)
            .opt("recorded_at", Int)
            .opt("harness", Text)
            .opt("agent", Text)
            .opt("account", Text)
            .opt("model", Text)
            .opt("effort", Text)
            .opt("role", Text)
            .opt("pool", Text)
            .opt("member", Text)
            .opt("profile", Text)
            .opt("why", Text)
            .opt("moved_from", Text)
            .opt("usage_known", Bool)
            .opt("tokens_input", Int)
            .opt("tokens_output", Int)
            .opt("tokens_cached", Int)
            .opt("cost_micros", Int)
            .opt("cost_currency", Text)
            .opt("duration_ms", Int)
            .opt("limited", Bool)
            .opt("moved_on", Bool)
            .opt("tags", Json)
            .index(&["model"], false)
            .index(&["role"], false)
            .index(&["profile"], false)
            .done(),
        Builder::new(EVENTS, &["session_id", "seq"])
            .col("session_id", Text)
            .col("seq", Int)
            .col("method", Text)
            .opt("kind", Text)
            .opt("turn_no", Int)
            .opt("call_id", Text)
            .opt("at", Text)
            .opt("at_ms", Int)
            .col("bytes", Int)
            .col("line", Text)
            .opt("text", Text)
            .index(&["session_id", "turn_no"], false)
            .index(&["session_id", "method"], false)
            .requires(&["session_id"])
            .done(),
        Builder::new(ATTACHMENTS, &["sha256"])
            .col("sha256", Text)
            .col("bytes", Int)
            .opt("media_type", Text)
            .opt("data", Blob)
            .done(),
        Builder::new(EVENT_ATTACHMENTS, &["session_id", "seq", "sha256"])
            .col("session_id", Text)
            .col("seq", Int)
            .col("sha256", Text)
            .done(),
        Builder::new(LISTINGS, &["owner", "id"])
            .col("owner", Text)
            .col("id", Text)
            .opt("session_id", Text)
            .opt("entry", Text)
            .opt("harness", Text)
            .opt("place", Text)
            .opt("kind", Text)
            .opt("cwd", Text)
            .opt("title", Text)
            .opt("account", Text)
            .opt("site", Text)
            .opt("relation_kind", Text)
            .opt("relation_key", Text)
            .opt("relation_scope", Text)
            .col("named", Bool)
            .col("pinned", Bool)
            .col("archived", Bool)
            .col("created_at", Int)
            .col("updated_at", Int)
            .opt("app", Json)
            .col("size", Int)
            .index(&["owner", "updated_at"], false)
            .index(&["owner", "harness"], false)
            .index(&["owner", "relation_kind", "relation_key"], false)
            .index(&["owner", "session_id"], false)
            .done(),
        Builder::new(SOURCE_MEMORY, &["owner", "label"])
            .col("owner", Text)
            .col("label", Text)
            .col("seen", Int)
            .col("data", Json)
            .done(),
        Builder::new(FAVOURITES, &["owner", "label", "model"])
            .col("owner", Text)
            .col("label", Text)
            .col("model", Text)
            .done(),
    ]
}

/// The migration that creates schema version 1 in an empty store.
#[must_use]
pub fn v1() -> Migration {
    Migration {
        to_version: VERSION,
        steps: tables()
            .into_iter()
            .map(|table| Step::CreateTable { table })
            .collect(),
    }
}

/// The table `name` of schema version 1.
#[must_use]
pub fn table(name: &str) -> Option<TableInfo> {
    tables().into_iter().find(|table| table.name == name)
}

/// How a store's schema version stands against the one this build writes.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Fit {
    /// The store holds no schema: [`v1`] is written into it.
    Empty,
    /// The store holds [`VERSION`].
    Current,
    /// The store is older, and the steps that bring it to [`VERSION`] only add: it is migrated.
    Older(u32),
    /// A newer build made it. It is opened read-only, and what it holds is shown but never changed, because this build does not know
    /// what a newer schema means by its rows.
    Newer(u32),
}

/// Where a store whose schema version is `stored` stands against this build.
#[must_use]
pub const fn fit(stored: u32) -> Fit {
    if stored == 0 {
        Fit::Empty
    } else if stored == VERSION {
        Fit::Current
    } else if stored < VERSION {
        Fit::Older(stored)
    } else {
        Fit::Newer(stored)
    }
}

impl Fit {
    /// Whether this build may change the store.
    #[must_use]
    pub const fn writable(self) -> bool {
        !matches!(self, Self::Newer(_))
    }
}
