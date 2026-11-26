//! The `history_store` kind: where the conversations a host records, and the list of them each client shows, are kept.
//!
//! The kind is a relational interface, not a storage format: named tables of typed columns, structured queries and atomic batches.
//! The host writes to it and reads from it with one schema ([`schema::v1`]); a provider that is a database compiles the queries to its
//! dialect, one that is a pile of files evaluates them over the files. Which store a host uses is not the host's to say: SQLite is a
//! provider of this kind, the in-memory one is another, and an extension adds more.
//!
//! The models are plain data, the same in every language. A filter is a list of alternatives, each a list of conditions that all hold
//! (an OR of ANDs), so no model nests in itself and every language writes one the same way.
//!
//! [`contract::run`] is the test every provider passes.

pub mod contract;
mod eval;
pub mod files;
pub mod memory;
pub mod rows;
pub mod schema;

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use core::fmt;

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Bytes, FromValue, Schema, Secret, Status, ToValue, Value};
use guatiao_intake::Screen;

pub use eval::{
    Plan, check_filter, cmp_cells, next_cursor, normalize_cell, normalize_row, plan_select,
    row_matches, search_words,
};

/// The name of the `history_store` kind.
pub const KIND: &str = "history_store";

/// The versions of the `history_store` kind this host offers.
pub const KIND_VERSIONS: [&str; 1] = ["1"];

/// A capability: `sql` runs one read-only statement in the provider's dialect.
pub const CAP_SQL: &str = "sql";
/// A capability: full-text search over the text of events.
pub const CAP_SEARCH: &str = "search";
/// A capability: the bytes of attachments are kept.
pub const CAP_BLOBS: &str = "blobs";
/// A capability: the store can be made smaller on request.
pub const CAP_COMPACT: &str = "compact";

/// The prefix of a [`StoreError::Conflict`] message.
pub const CONFLICT_PREFIX: &str = "conflict: ";
/// The prefix of a [`StoreError::ReadOnly`] message.
pub const READ_ONLY_PREFIX: &str = "read-only: ";
/// The prefix of a [`StoreError::Busy`] message.
pub const BUSY_PREFIX: &str = "busy: ";
/// The prefix of a [`StoreError::Corrupt`] message.
pub const CORRUPT_PREFIX: &str = "corrupt: ";
/// The prefix of a [`StoreError::Full`] message.
pub const FULL_PREFIX: &str = "full: ";

/// Why a store call did not succeed. No message holds a row's content, a password or a connection string.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum StoreError {
    /// An unknown table or column, a value of the wrong type, a condition the table cannot answer, a missing required column.
    Bad(String),
    /// An `insert` of a key that exists.
    Conflict(String),
    /// `sql`, `compact` or a write without the capability.
    Unsupported(String),
    /// A write to a read-only or foreign store.
    ReadOnly(String),
    /// Another writer holds the store; try again after `retry_ms`.
    Busy {
        /// What was refused.
        why: String,
        /// How long to wait, in milliseconds.
        retry_ms: u64,
    },
    /// The connection or the files are no longer there.
    Gone(String),
    /// The file or database is not readable as this schema.
    Corrupt(String),
    /// The disk or quota refused.
    Full(String),
    /// Anything else.
    Internal(String),
}

impl fmt::Display for StoreError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Bad(why) | Self::Unsupported(why) | Self::Gone(why) | Self::Internal(why) => {
                f.write_str(why)
            }
            Self::Conflict(why) => write!(f, "{CONFLICT_PREFIX}{why}"),
            Self::ReadOnly(why) => write!(f, "{READ_ONLY_PREFIX}{why}"),
            Self::Busy { why, retry_ms } => write!(f, "{BUSY_PREFIX}{why} retry_ms={retry_ms}"),
            Self::Corrupt(why) => write!(f, "{CORRUPT_PREFIX}{why}"),
            Self::Full(why) => write!(f, "{FULL_PREFIX}{why}"),
        }
    }
}

impl core::error::Error for StoreError {}

impl From<StoreError> for ProviderError {
    fn from(error: StoreError) -> Self {
        let status = match &error {
            StoreError::Bad(_) | StoreError::Conflict(_) => Status::GUATIAO_ERR_BAD_VALUE,
            StoreError::Unsupported(_) | StoreError::ReadOnly(_) => Status::GUATIAO_ERR_UNSUPPORTED,
            StoreError::Busy { .. } | StoreError::Gone(_) => Status::GUATIAO_ERR_GONE,
            StoreError::Corrupt(_) | StoreError::Full(_) | StoreError::Internal(_) => {
                Status::GUATIAO_ERR_INTERNAL
            }
        };
        Self::new(status, &error.to_string())
    }
}

impl From<ProviderError> for StoreError {
    fn from(error: ProviderError) -> Self {
        let message = error.message.to_string();
        let status = error.status;
        let strip = |prefix: &str| message.strip_prefix(prefix).map(ToOwned::to_owned);
        if status == Status::GUATIAO_ERR_BAD_VALUE {
            strip(CONFLICT_PREFIX).map_or(Self::Bad(message), Self::Conflict)
        } else if status == Status::GUATIAO_ERR_UNSUPPORTED {
            strip(READ_ONLY_PREFIX).map_or(Self::Unsupported(message), Self::ReadOnly)
        } else if status == Status::GUATIAO_ERR_GONE {
            match strip(BUSY_PREFIX) {
                Some(rest) => {
                    let retry_ms = rest
                        .rsplit_once("retry_ms=")
                        .and_then(|(_, n)| n.trim().parse().ok())
                        .unwrap_or(0);
                    Self::Busy {
                        why: rest,
                        retry_ms,
                    }
                }
                None => Self::Gone(message),
            }
        } else if let Some(why) = strip(CORRUPT_PREFIX) {
            Self::Corrupt(why)
        } else if let Some(why) = strip(FULL_PREFIX) {
            Self::Full(why)
        } else {
            Self::Internal(message)
        }
    }
}

/// Returns true when `word` is 1 to 32 lower-case ASCII letters, digits or `_`.
#[must_use]
pub fn is_valid_id(word: &str) -> bool {
    !word.is_empty()
        && word.len() <= 32
        && word
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

/// One value of a row.
///
/// Times are `Int` milliseconds since the Unix epoch (UTC), so no dialect decides their text. A `Json` cell holds the document as
/// text; a provider with no native type keeps it as text, and a select returns it the same either way.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, tag = "type", rename_all = "snake_case")]
pub enum Cell {
    /// No value.
    Null,
    /// A boolean.
    Bool {
        /// The value.
        value: bool,
    },
    /// A 64-bit integer.
    Int {
        /// The value.
        value: i64,
    },
    /// A floating-point number.
    Real {
        /// The value.
        value: f64,
    },
    /// Text.
    Text {
        /// The value.
        value: String,
    },
    /// Bytes.
    Bytes {
        /// The value.
        value: Bytes,
    },
    /// A JSON document, as text.
    Json {
        /// The document's text.
        value: String,
    },
}

impl Cell {
    /// A text cell.
    #[must_use]
    pub fn text(value: impl Into<String>) -> Self {
        Self::Text {
            value: value.into(),
        }
    }

    /// An integer cell.
    #[must_use]
    pub const fn int(value: i64) -> Self {
        Self::Int { value }
    }

    /// A boolean cell.
    #[must_use]
    pub const fn bool(value: bool) -> Self {
        Self::Bool { value }
    }

    /// A real cell.
    #[must_use]
    pub const fn real(value: f64) -> Self {
        Self::Real { value }
    }

    /// A JSON cell holding the document `text`.
    #[must_use]
    pub fn json(text: impl Into<String>) -> Self {
        Self::Json { value: text.into() }
    }

    /// A bytes cell.
    #[must_use]
    pub fn bytes(value: impl Into<Vec<u8>>) -> Self {
        Self::Bytes {
            value: Bytes::from(value.into()),
        }
    }

    /// A text cell, or null.
    #[must_use]
    pub fn opt_text(value: Option<&str>) -> Self {
        value.map_or(Self::Null, Self::text)
    }

    /// An integer cell, or null.
    #[must_use]
    pub fn opt_int(value: Option<i64>) -> Self {
        value.map_or(Self::Null, Self::int)
    }

    /// Whether this is null.
    #[must_use]
    pub const fn is_null(&self) -> bool {
        matches!(self, Self::Null)
    }

    /// The integer, when this is one.
    #[must_use]
    pub const fn as_int(&self) -> Option<i64> {
        match self {
            Self::Int { value } => Some(*value),
            _ => None,
        }
    }

    /// The text of a text or JSON cell.
    #[must_use]
    pub fn as_text(&self) -> Option<&str> {
        match self {
            Self::Text { value } | Self::Json { value } => Some(value),
            _ => None,
        }
    }

    /// The boolean, when this is one.
    #[must_use]
    pub const fn as_bool(&self) -> Option<bool> {
        match self {
            Self::Bool { value } => Some(*value),
            _ => None,
        }
    }

    /// The bytes, when this is a bytes cell.
    #[must_use]
    pub fn as_bytes(&self) -> Option<Vec<u8>> {
        match self {
            Self::Bytes { value } => Some(Vec::<u8>::from(value.clone())),
            _ => None,
        }
    }

    /// About how many bytes this value takes.
    #[must_use]
    pub fn size(&self) -> usize {
        match self {
            Self::Null | Self::Bool { .. } | Self::Int { .. } | Self::Real { .. } => 8,
            Self::Text { value } | Self::Json { value } => value.len(),
            Self::Bytes { value } => Vec::<u8>::from(value.clone()).len(),
        }
    }
}

/// The type of a column.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum ColumnType {
    /// Text.
    Text,
    /// A 64-bit integer; times are milliseconds since the Unix epoch.
    Int,
    /// A floating-point number.
    Real,
    /// A boolean.
    Bool,
    /// Bytes.
    Blob,
    /// A JSON document.
    Json,
}

/// One column of a table.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct ColumnInfo {
    /// The column's name.
    pub name: String,
    /// What it holds.
    pub kind: ColumnType,
    /// Whether it may be null.
    #[map(default)]
    pub nullable: bool,
}

impl ColumnInfo {
    /// A column that may not be null.
    #[must_use]
    pub fn new(name: impl Into<String>, kind: ColumnType) -> Self {
        Self {
            name: name.into(),
            kind,
            nullable: false,
        }
    }

    /// A column that may be null.
    #[must_use]
    pub fn nullable(name: impl Into<String>, kind: ColumnType) -> Self {
        Self {
            name: name.into(),
            kind,
            nullable: true,
        }
    }
}

/// An index over columns of a table.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct IndexInfo {
    /// The index's name, unique in the store.
    pub name: String,
    /// The columns, in order.
    pub columns: Vec<String>,
    /// Whether two rows may not share the columns' values.
    #[map(default)]
    pub unique: bool,
}

/// A table: its columns, its key and its indexes.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct TableInfo {
    /// The table's name.
    pub name: String,
    /// Its columns.
    pub columns: Vec<ColumnInfo>,
    /// The columns whose values identify a row.
    pub key: Vec<String>,
    /// Its indexes.
    #[map(default)]
    pub indexes: Vec<IndexInfo>,
    /// Columns a `select` must test for equality, as a table of one session's lines is only read for that session.
    #[map(default)]
    pub requires: Vec<String>,
}

impl TableInfo {
    /// The column called `name`.
    #[must_use]
    pub fn column(&self, name: &str) -> Option<&ColumnInfo> {
        self.columns.iter().find(|column| column.name == name)
    }
}

/// What a store holds now, from [`HistoryDb::schema`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct SchemaState {
    /// The version of the schema, `0` when the store holds none.
    pub version: u32,
    /// Its tables.
    pub tables: Vec<TableInfo>,
}

/// One typed step of a migration; never SQL text.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, tag = "step", rename_all = "snake_case")]
pub enum Step {
    /// Creates a table.
    CreateTable {
        /// The table, with its indexes.
        table: TableInfo,
    },
    /// Adds a column, which must be nullable.
    AddColumn {
        /// The table's name.
        table: String,
        /// The column.
        column: ColumnInfo,
    },
    /// Creates an index.
    CreateIndex {
        /// The table's name.
        table: String,
        /// The index.
        index: IndexInfo,
    },
    /// Drops an index.
    DropIndex {
        /// The table's name.
        table: String,
        /// The index's name.
        name: String,
    },
}

/// Steps that take a store to a schema version, all or none.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Migration {
    /// The version the store has after the steps.
    pub to_version: u32,
    /// The steps, in order.
    pub steps: Vec<Step>,
}

/// A comparison a condition makes.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Op {
    /// Equal to the one value.
    Eq,
    /// Not equal to the one value; a null column is not.
    Ne,
    /// Less than the one value.
    Lt,
    /// Less than or equal to the one value.
    Le,
    /// Greater than the one value.
    Gt,
    /// Greater than or equal to the one value.
    Ge,
    /// Equal to any of the values.
    In,
    /// The column is null; no value.
    IsNull,
    /// The column is not null; no value.
    NotNull,
    /// Text that starts with the one value, which is literal.
    Prefix,
    /// Text that holds the one value, which is literal.
    Contains,
    /// Text that ends with the one value, which is literal.
    Suffix,
    /// Text that holds every word of the one value, as whole words with the case ignored (see [`search_words`]); only a store with
    /// the capability `search` answers it, and for the columns it indexes (`events.text`).
    Matches,
}

/// One condition on one column of a row.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Cond {
    /// The column.
    pub column: String,
    /// The comparison.
    pub op: Op,
    /// What it compares with: none for `is_null` and `not_null`, one for the comparisons and the text tests, any for `in`.
    #[map(default)]
    pub values: Vec<Cell>,
}

impl Cond {
    /// `column` compared by `op` with one `value`.
    #[must_use]
    pub fn new(column: impl Into<String>, op: Op, value: Cell) -> Self {
        Self {
            column: column.into(),
            op,
            values: alloc::vec![value],
        }
    }

    /// `column` equal to `value`.
    #[must_use]
    pub fn eq(column: impl Into<String>, value: Cell) -> Self {
        Self::new(column, Op::Eq, value)
    }

    /// `column` is null.
    #[must_use]
    pub fn is_null(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            op: Op::IsNull,
            values: Vec::new(),
        }
    }

    /// `column` is not null.
    #[must_use]
    pub fn not_null(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            op: Op::NotNull,
            values: Vec::new(),
        }
    }

    /// `column` equal to any of `values`.
    #[must_use]
    pub fn any_of(column: impl Into<String>, values: Vec<Cell>) -> Self {
        Self {
            column: column.into(),
            op: Op::In,
            values,
        }
    }
}

/// A filter: any one of its alternatives, each of which is every one of its conditions. No alternatives is every row.
pub type Filter = Vec<Vec<Cond>>;

/// A filter of one alternative.
#[must_use]
pub fn all_of(conds: Vec<Cond>) -> Filter {
    alloc::vec![conds]
}

/// How rows are ordered by one column.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Order {
    /// The column; nulls sort lowest.
    pub column: String,
    /// Largest first.
    #[map(default)]
    pub desc: bool,
}

impl Order {
    /// Smallest first.
    #[must_use]
    pub fn asc(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            desc: false,
        }
    }

    /// Largest first.
    #[must_use]
    pub fn desc(column: impl Into<String>) -> Self {
        Self {
            column: column.into(),
            desc: true,
        }
    }
}

/// Rows of one table.
///
/// The key's columns are appended to `order`, so the order is total and a page boundary never moves when rows are added. A reply
/// ends at `limit` rows or at the opened store's `reply_bytes`, whichever comes first, with `next`.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Select {
    /// The table.
    pub table: String,
    /// The columns to answer, in order; none is all of them.
    #[map(default)]
    pub columns: Vec<String>,
    /// Which rows.
    #[map(default)]
    pub filter: Filter,
    /// The order.
    #[map(default)]
    pub order: Vec<Order>,
    /// The most rows to answer.
    pub limit: u32,
    /// The `next` of the last reply to this same select.
    pub after: Option<String>,
}

impl Select {
    /// Up to `limit` rows of `table`.
    #[must_use]
    pub fn new(table: impl Into<String>, limit: u32) -> Self {
        Self {
            table: table.into(),
            columns: Vec::new(),
            filter: Vec::new(),
            order: Vec::new(),
            limit,
            after: None,
        }
    }

    /// Only the rows `filter` keeps.
    #[must_use]
    pub fn filtered(mut self, filter: Filter) -> Self {
        self.filter = filter;
        self
    }

    /// In this order.
    #[must_use]
    pub fn ordered(mut self, order: Vec<Order>) -> Self {
        self.order = order;
        self
    }
}

/// Rows answered by a query.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Rows {
    /// The columns of each row.
    pub columns: Vec<String>,
    /// The rows.
    pub rows: Vec<Vec<Cell>>,
    /// Where the next page starts, when there is one.
    pub next: Option<String>,
}

impl Rows {
    /// The cell of `column` in row `row`.
    #[must_use]
    pub fn get(&self, row: usize, column: &str) -> Option<&Cell> {
        let at = self.columns.iter().position(|name| name == column)?;
        self.rows.get(row)?.get(at)
    }
}

/// A function an aggregate computes.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum AggFn {
    /// The rows, or the rows where `column` is not null.
    Count,
    /// The sum of an integer or real column.
    Sum,
    /// The least value.
    Min,
    /// The greatest value.
    Max,
}

/// One figure of an aggregate.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Figure {
    /// The function.
    pub func: AggFn,
    /// The column, none for `count` of every row.
    pub column: Option<String>,
    /// The column of the answer holding it.
    pub name: String,
}

impl Figure {
    /// `func` of `column`, answered as `name`.
    #[must_use]
    pub fn of(func: AggFn, column: Option<&str>, name: impl Into<String>) -> Self {
        Self {
            func,
            column: column.map(ToOwned::to_owned),
            name: name.into(),
        }
    }
}

/// Figures over the rows of a table, by group.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Aggregate {
    /// The table.
    pub table: String,
    /// Which rows.
    #[map(default)]
    pub filter: Filter,
    /// The columns the rows are grouped by; they come first in the answer, which is ordered by them.
    #[map(default)]
    pub group_by: Vec<String>,
    /// The figures.
    pub figures: Vec<Figure>,
}

/// A change of the rows a filter keeps.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Update {
    /// The table.
    pub table: String,
    /// Which rows.
    #[map(default)]
    pub filter: Filter,
    /// Columns set to a value.
    #[map(default)]
    pub set: BTreeMap<String, Cell>,
    /// Integer columns increased in place by an amount (negative decreases), so a counter moves with the rows that made it.
    #[map(default)]
    pub add: BTreeMap<String, i64>,
}

/// One change of a [`Write`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, tag = "op", rename_all = "snake_case")]
pub enum Change {
    /// Adds a row; a key that exists is a [`StoreError::Conflict`].
    Insert {
        /// The table.
        table: String,
        /// The row, by column; a column left out is null.
        row: BTreeMap<String, Cell>,
    },
    /// Adds a row, or replaces the one with the same key.
    Upsert {
        /// The table.
        table: String,
        /// The row, by column; a column left out is null.
        row: BTreeMap<String, Cell>,
    },
    /// Changes rows.
    Update {
        /// The change.
        update: Update,
    },
    /// Removes rows.
    Delete {
        /// The table.
        table: String,
        /// Which rows.
        filter: Filter,
    },
}

/// Changes applied together: every one or none.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Write {
    /// The changes, in order.
    pub ops: Vec<Change>,
}

/// What a write changed.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Written {
    /// Rows added, replaced, changed and removed.
    pub affected: u64,
}

/// One read-only statement in the provider's dialect.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Sql {
    /// The statement, `SELECT` or `WITH`, with `?` for each parameter.
    pub text: String,
    /// The parameters, bound and never spliced in.
    #[map(default)]
    pub params: Vec<Cell>,
}

/// How big a store is.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Stats {
    /// Bytes it takes, when it knows.
    pub bytes: Option<u64>,
    /// Rows by table.
    #[map(default)]
    pub rows: BTreeMap<String, u64>,
}

/// What [`HistoryDb::compact`] did.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Compacted {
    /// Bytes before.
    pub before: u64,
    /// Bytes after.
    pub after: u64,
}

/// Whether the rows can be changed.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Access {
    /// Read and written.
    ReadWrite,
    /// Only read.
    ReadOnly,
}

/// Facts of a store the host does not decide.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct Limits {
    /// The longest value it holds, in bytes.
    pub max_value_bytes: Option<u64>,
    /// The most rows it holds.
    pub max_rows: Option<u64>,
}

/// What a store provider is and the settings it asks for, from [`HistoryStore::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct StoreInfo {
    /// The id the store is named by in `canoa.cfg`: 1 to 32 lower-case ASCII letters, digits and `_`.
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// One line saying what it is.
    pub summary: String,
    /// Where the data lives, in a sentence or two, shown before the person chooses it.
    pub location: String,
    /// Whether rows can be changed.
    pub access: Access,
    /// The rows are another program's files presented in the schema; the host never writes the schema into it.
    #[map(default)]
    pub foreign: bool,
    /// What a host uses when `canoa.cfg` names no store; at most one provider of a host sets it.
    #[map(default)]
    pub default: bool,
    /// What it can do beyond the structured calls: the `CAP_*` names.
    #[map(default)]
    pub capabilities: Vec<String>,
    /// The SQL dialect (`sqlite`, `postgres`) when it has `sql`.
    pub dialect: Option<String>,
    /// Facts of the store.
    #[map(default)]
    pub limits: Limits,
    /// The settings it takes, as a guatiao schema of an object.
    pub schema: Option<Value>,
    /// How the schema is shown, as a guatiao-intake form document.
    pub form: Option<Value>,
    /// The tables a foreign store presents; none: it holds the canonical ones.
    pub tables: Option<Vec<TableInfo>>,
}

impl StoreInfo {
    /// A read-write store called `id`, shown as `title`, taking no settings.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            summary: String::new(),
            location: String::new(),
            access: Access::ReadWrite,
            foreign: false,
            default: false,
            capabilities: Vec::new(),
            dialect: None,
            limits: Limits::default(),
            schema: None,
            form: None,
            tables: None,
        }
    }

    /// Whether it has the capability `name`.
    #[must_use]
    pub fn can(&self, name: &str) -> bool {
        self.capabilities.iter().any(|have| have == name)
    }

    /// This store taking the settings `S` describes, shown with the screen `S` declares.
    ///
    /// # Errors
    ///
    /// When `S`'s schema or form cannot be written.
    pub fn taking<S: Schema + Screen>(mut self) -> Result<Self, ProviderError> {
        let (schema, form) = crate::settings::describe::<S>()?;
        self.schema = Some(schema);
        self.form = Some(form);
        Ok(self)
    }

    /// Why this description cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !is_valid_id(&self.id) {
            return Some(format!(
                "the history store id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(format!("the history store {:?} has no title", self.id));
        }
        for (at, name) in self.capabilities.iter().enumerate() {
            if self.capabilities[..at].contains(name) {
                return Some(format!(
                    "the history store {:?} lists the capability {name:?} twice",
                    self.id
                ));
            }
        }
        if self.can(CAP_SQL) && self.dialect.is_none() {
            return Some(format!(
                "the history store {:?} has `sql` and names no dialect",
                self.id
            ));
        }
        if self.access == Access::ReadOnly && self.default {
            return Some(format!(
                "the history store {:?} is read-only and cannot be the default",
                self.id
            ));
        }
        crate::settings::problem(self.schema.as_ref(), self.form.as_ref())
    }
}

/// What a host sends to [`HistoryStore::open`].
#[derive(ToValue, FromValue, Schema, Debug)]
#[map(crate = ::coqui_acp_api::guatiao)]
pub struct OpenRequest {
    /// The store's settings as a JSON object keyed by the schema's members.
    pub settings: String,
    /// The host's state directory, for a default path; the provider reads no other place.
    pub home: String,
    /// A connection password the host resolved from its secret store; never in `settings` and never answered.
    pub secret: Option<Secret<String>>,
    /// Whether to create what is missing; `false` makes a store with no schema [`StoreError::Corrupt`] or [`StoreError::Gone`].
    #[map(default)]
    pub create: bool,
    /// The most bytes of rows one reply holds (at least one row is always answered).
    pub reply_bytes: u64,
}

impl OpenRequest {
    /// A request to open the store `settings` name under `home`, creating it, with replies of up to 4 MiB.
    #[must_use]
    pub fn new(settings: impl Into<String>, home: impl Into<String>) -> Self {
        Self {
            settings: settings.into(),
            home: home.into(),
            secret: None,
            create: true,
            reply_bytes: 4 * 1024 * 1024,
        }
    }
}

/// An opened store.
#[coqui_acp_api::guatiao::kind(object, name = "history_db", crate = ::coqui_acp_api::guatiao)]
pub trait HistoryDb: Send + Sync {
    /// The schema the store holds.
    fn schema(&self) -> Result<SchemaState, ProviderError>;

    /// Applies typed steps, all or none; only a read-write store.
    fn migrate(&self, migration: Migration) -> Result<(), ProviderError>;

    /// Rows of a table, a page at a time.
    fn select(&self, select: Select) -> Result<Rows, ProviderError>;

    /// How many rows a select keeps, ignoring its order, limit and cursor.
    fn count(&self, select: Select) -> Result<u64, ProviderError>;

    /// Figures over rows, by group.
    fn aggregate(&self, aggregate: Aggregate) -> Result<Rows, ProviderError>;

    /// Applies changes, every one or none.
    fn write(&self, write: Write) -> Result<Written, ProviderError>;

    /// One read-only statement; only with the capability `sql`.
    fn sql(&self, sql: Sql) -> Result<Rows, ProviderError>;

    /// A text that moves when anything changed, by any writer.
    fn revision(&self) -> Result<String, ProviderError>;

    /// How big the store is.
    fn stats(&self) -> Result<Stats, ProviderError>;

    /// Makes the store smaller; only with the capability `compact`.
    fn compact(&self) -> Result<Compacted, ProviderError>;

    /// Lets go of the connection or the files.
    fn release(&self) -> Result<(), ProviderError>;
}

/// A history store provider: describes itself and its settings, and opens the store.
#[coqui_acp_api::guatiao::kind(name = "history_store", crate = ::coqui_acp_api::guatiao)]
pub trait HistoryStore: Send + Sync {
    /// What this store is, what settings it takes, and how Settings draws its form.
    fn describe(&self) -> Result<StoreInfo, ProviderError>;

    /// Opens the store.
    fn open(&self, request: OpenRequest) -> Result<Object<dyn HistoryDb>, ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_store_error_round_trips_through_provider_error() {
        for error in [
            StoreError::Bad("no such column".to_owned()),
            StoreError::Conflict("key exists".to_owned()),
            StoreError::Unsupported("no sql".to_owned()),
            StoreError::ReadOnly("foreign".to_owned()),
            StoreError::Busy {
                why: "locked".to_owned(),
                retry_ms: 250,
            },
            StoreError::Gone("closed".to_owned()),
            StoreError::Corrupt("no schema".to_owned()),
            StoreError::Full("disk".to_owned()),
            StoreError::Internal("other".to_owned()),
        ] {
            let round = StoreError::from(ProviderError::from(error.clone()));
            if let StoreError::Busy { retry_ms, .. } = &round {
                assert_eq!(*retry_ms, 250);
            } else {
                assert_eq!(round, error);
            }
        }
    }

    #[test]
    fn a_description_is_checked() {
        let mut info = StoreInfo::new("sqlite", "SQLite");
        assert_eq!(info.problem(), None);
        info.capabilities = alloc::vec![CAP_SQL.to_owned()];
        assert!(info.problem().is_some());
        info.dialect = Some("sqlite".to_owned());
        assert_eq!(info.problem(), None);
        info.access = Access::ReadOnly;
        info.default = true;
        assert!(info.problem().is_some());
        assert!(StoreInfo::new("Bad-id", "x").problem().is_some());
    }
}
