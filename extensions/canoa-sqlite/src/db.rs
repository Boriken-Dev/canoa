//! The opened database: the structured calls of the kind compiled to SQLite's dialect, every value bound and never spliced in.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the connections are kept behind a std lock"
)]

use std::path::{Path, PathBuf};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use canoa::history_store::{
    AggFn, Aggregate, Cell, Change, ColumnType, Compacted, Cond, Filter, HistoryDb, IndexInfo,
    Migration, Op, Rows, SchemaState, Select, Sql, Stats, Step, StoreError, TableInfo, Update,
    Write, Written, check_filter, next_cursor, normalize_cell, normalize_row, plan_select,
    search_words,
};
use coqui_acp_api::guatiao::library::ProviderError;
use rusqlite::types::{Value, ValueRef};
use rusqlite::{Connection, OpenFlags, TransactionBehavior, params_from_iter};

/// How the file is made durable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Journal {
    /// A write-ahead log: readers and the writer do not wait for each other.
    Wal,
    /// A rollback journal.
    Delete,
}

/// How often SQLite waits for the disk.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Synchronous {
    /// Never.
    Off,
    /// At the moments that matter for a log.
    Normal,
    /// After every transaction.
    Full,
}

/// What an open takes, after the settings are read.
#[derive(Debug, Clone)]
pub struct Options {
    /// The database file.
    pub path: PathBuf,
    /// How long a writer waits for another, in milliseconds.
    pub busy_timeout_ms: u64,
    /// The journal.
    pub journal: Journal,
    /// The synchronous setting.
    pub synchronous: Synchronous,
    /// Open without the right to change it.
    pub read_only: bool,
    /// Create the file and the schema table when they are missing.
    pub create: bool,
    /// The most bytes of rows one reply holds.
    pub reply_bytes: usize,
}

struct Inner {
    conn: Connection,
    reader: Option<Connection>,
    cache: Option<(i64, Arc<Vec<TableInfo>>)>,
}

/// An opened SQLite database.
pub struct SqliteDb {
    inner: Mutex<Option<Inner>>,
    path: PathBuf,
    read_only: bool,
    busy_ms: u64,
    reply_bytes: usize,
}

impl core::fmt::Debug for SqliteDb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("SqliteDb")
            .field("path", &self.path)
            .field("read_only", &self.read_only)
            .finish_non_exhaustive()
    }
}

fn store_error(error: &rusqlite::Error, busy_ms: u64) -> StoreError {
    use rusqlite::ErrorCode as Code;
    let text = error.to_string();
    match error {
        rusqlite::Error::SqliteFailure(failure, _) => match failure.code {
            Code::DatabaseBusy | Code::DatabaseLocked => StoreError::Busy {
                why: "another writer holds the database".to_owned(),
                retry_ms: busy_ms.max(1),
            },
            Code::ConstraintViolation => StoreError::Conflict(text),
            Code::DiskFull => StoreError::Full("the disk is full".to_owned()),
            Code::NotADatabase | Code::DatabaseCorrupt => {
                StoreError::Corrupt("the file is not a readable database".to_owned())
            }
            Code::ReadOnly => StoreError::ReadOnly("the database is read-only".to_owned()),
            Code::CannotOpen => StoreError::Gone("the database file cannot be opened".to_owned()),
            _ => StoreError::Internal(text),
        },
        _ => StoreError::Internal(text),
    }
}

/// The table that holds the words of `events.text`.
const SEARCH_TABLE: &str = "events_fts";
/// The column the full-text index covers.
const SEARCH_COLUMN: &str = "text";

fn is_ident(name: &str) -> bool {
    !name.is_empty()
        && name.len() <= 64
        && name
            .bytes()
            .all(|b| b.is_ascii_lowercase() || b.is_ascii_digit() || b == b'_')
}

fn quote(name: &str) -> String {
    format!("\"{name}\"")
}

const fn sql_type(kind: ColumnType) -> &'static str {
    match kind {
        ColumnType::Text | ColumnType::Json => "TEXT",
        ColumnType::Int | ColumnType::Bool => "INTEGER",
        ColumnType::Real => "REAL",
        ColumnType::Blob => "BLOB",
    }
}

fn value_of(cell: &Cell) -> Value {
    match cell {
        Cell::Null => Value::Null,
        Cell::Bool { value } => Value::Integer(i64::from(*value)),
        Cell::Int { value } => Value::Integer(*value),
        Cell::Real { value } => Value::Real(*value),
        Cell::Text { value } | Cell::Json { value } => Value::Text(value.clone()),
        Cell::Bytes { .. } => Value::Blob(cell.as_bytes().unwrap_or_default()),
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "an integer stored in a real column"
)]
fn cell_of(kind: Option<ColumnType>, value: ValueRef<'_>) -> Cell {
    match value {
        ValueRef::Null => Cell::Null,
        ValueRef::Integer(i) => match kind {
            Some(ColumnType::Bool) => Cell::bool(i != 0),
            Some(ColumnType::Real) => Cell::real(i as f64),
            _ => Cell::int(i),
        },
        ValueRef::Real(f) => Cell::real(f),
        ValueRef::Text(bytes) => {
            let text = String::from_utf8_lossy(bytes).into_owned();
            if kind == Some(ColumnType::Json) {
                Cell::json(text)
            } else {
                Cell::text(text)
            }
        }
        ValueRef::Blob(bytes) => Cell::bytes(bytes.to_vec()),
    }
}

fn condition(cond: &Cond, params: &mut Vec<Value>) -> String {
    let column = quote(&cond.column);
    let one = || cond.values.first().map_or(Value::Null, value_of);
    let needle = || {
        let text = cond.values.first().and_then(Cell::as_text).unwrap_or("");
        (
            text.to_owned(),
            i64::try_from(text.chars().count()).unwrap_or(i64::MAX),
        )
    };
    match cond.op {
        Op::IsNull => format!("{column} IS NULL"),
        Op::NotNull => format!("{column} IS NOT NULL"),
        Op::In if cond.values.is_empty() => "0".to_owned(),
        Op::In => {
            params.extend(cond.values.iter().map(value_of));
            let marks = vec!["?"; cond.values.len()].join(",");
            format!("{column} IN ({marks})")
        }
        Op::Matches => {
            let words = search_words(cond.values.first().and_then(Cell::as_text).unwrap_or(""));
            if words.is_empty() || cond.column != SEARCH_COLUMN {
                return "0".to_owned();
            }
            let query: Vec<String> = words.iter().map(|w| format!("\"{w}\"")).collect();
            params.push(Value::Text(query.join(" ")));
            format!("rowid IN (SELECT rowid FROM {SEARCH_TABLE} WHERE {SEARCH_TABLE} MATCH ?)")
        }
        Op::Contains => {
            params.push(Value::Text(needle().0));
            format!("instr({column}, ?) > 0")
        }
        Op::Prefix => {
            let (text, chars) = needle();
            params.extend([Value::Integer(chars), Value::Text(text)]);
            format!("substr({column}, 1, ?) = ?")
        }
        Op::Suffix => {
            let (text, chars) = needle();
            if chars == 0 {
                return format!("{column} IS NOT NULL");
            }
            params.extend([Value::Integer(chars), Value::Text(text)]);
            format!("substr({column}, -?) = ?")
        }
        op => {
            params.push(one());
            let sign = match op {
                Op::Eq => "=",
                Op::Ne => "<>",
                Op::Lt => "<",
                Op::Le => "<=",
                Op::Gt => ">",
                _ => ">=",
            };
            format!("{column} {sign} ?")
        }
    }
}

fn where_clause(filter: &Filter, params: &mut Vec<Value>) -> String {
    if filter.is_empty() {
        return String::new();
    }
    let alternatives: Vec<String> = filter
        .iter()
        .map(|alternative| {
            if alternative.is_empty() {
                "1".to_owned()
            } else {
                let conds: Vec<String> = alternative.iter().map(|c| condition(c, params)).collect();
                format!("({})", conds.join(" AND "))
            }
        })
        .collect();
    format!(" WHERE {}", alternatives.join(" OR "))
}

fn find<'a>(tables: &'a [TableInfo], name: &str) -> Result<&'a TableInfo, StoreError> {
    tables
        .iter()
        .find(|table| table.name == name)
        .ok_or_else(|| StoreError::Bad(format!("there is no table {name}")))
}

fn table_json(table: &TableInfo) -> String {
    let kind = |k: ColumnType| match k {
        ColumnType::Text => "text",
        ColumnType::Int => "int",
        ColumnType::Real => "real",
        ColumnType::Bool => "bool",
        ColumnType::Blob => "blob",
        ColumnType::Json => "json",
    };
    serde_json::json!({
        "name": table.name,
        "columns": table.columns.iter().map(|c| serde_json::json!({"name": c.name, "kind": kind(c.kind), "nullable": c.nullable})).collect::<Vec<_>>(),
        "key": table.key,
        "indexes": table.indexes.iter().map(|i| serde_json::json!({"name": i.name, "columns": i.columns, "unique": i.unique})).collect::<Vec<_>>(),
        "requires": table.requires,
    })
    .to_string()
}

fn table_of(text: &str) -> Option<TableInfo> {
    use canoa::history_store::ColumnInfo;
    let json: serde_json::Value = serde_json::from_str(text).ok()?;
    let strings = |value: &serde_json::Value| -> Option<Vec<String>> {
        value
            .as_array()?
            .iter()
            .map(|s| s.as_str().map(ToOwned::to_owned))
            .collect()
    };
    let columns = json["columns"]
        .as_array()?
        .iter()
        .map(|c| {
            let kind = match c["kind"].as_str()? {
                "text" => ColumnType::Text,
                "int" => ColumnType::Int,
                "real" => ColumnType::Real,
                "bool" => ColumnType::Bool,
                "blob" => ColumnType::Blob,
                "json" => ColumnType::Json,
                _ => return None,
            };
            Some(ColumnInfo {
                name: c["name"].as_str()?.to_owned(),
                kind,
                nullable: c["nullable"].as_bool()?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    let indexes = json["indexes"]
        .as_array()?
        .iter()
        .map(|i| {
            Some(IndexInfo {
                name: i["name"].as_str()?.to_owned(),
                columns: strings(&i["columns"])?,
                unique: i["unique"].as_bool()?,
            })
        })
        .collect::<Option<Vec<_>>>()?;
    Some(TableInfo {
        name: json["name"].as_str()?.to_owned(),
        columns,
        key: strings(&json["key"])?,
        indexes,
        requires: strings(&json["requires"])?,
    })
}

impl SqliteDb {
    /// Opens the database `options` describe.
    ///
    /// # Errors
    ///
    /// The file cannot be opened, is not a database, or has no schema when `create` is false.
    pub fn open(options: &Options) -> Result<Self, StoreError> {
        let busy_ms = options.busy_timeout_ms;
        let fail = |e: &rusqlite::Error| store_error(e, busy_ms);
        let exists = options.path.exists();
        if !exists && !options.create {
            return Err(StoreError::Gone(
                "the database file is not there".to_owned(),
            ));
        }
        if !exists && let Some(parent) = options.path.parent() {
            std::fs::create_dir_all(parent)
                .map_err(|_| StoreError::Gone("the database folder cannot be made".to_owned()))?;
        }
        if !exists {
            make_private(&options.path)?;
        }
        let flags = if options.read_only {
            OpenFlags::SQLITE_OPEN_READ_ONLY
        } else {
            OpenFlags::SQLITE_OPEN_READ_WRITE | OpenFlags::SQLITE_OPEN_CREATE
        } | OpenFlags::SQLITE_OPEN_NO_MUTEX;
        let conn = Connection::open_with_flags(&options.path, flags).map_err(|e| fail(&e))?;
        conn.busy_timeout(Duration::from_millis(busy_ms))
            .map_err(|e| fail(&e))?;
        conn.pragma_update(None, "foreign_keys", "OFF")
            .map_err(|e| fail(&e))?;
        let version: i64 = conn
            .pragma_query_value(None, "user_version", |row| row.get(0))
            .map_err(|e| fail(&e))?;
        if !options.read_only {
            let journal = match options.journal {
                Journal::Wal => "WAL",
                Journal::Delete => "DELETE",
            };
            conn.pragma_update_and_check(None, "journal_mode", journal, |row| {
                row.get::<_, String>(0)
            })
            .map_err(|e| fail(&e))?;
            let sync = match options.synchronous {
                Synchronous::Off => "OFF",
                Synchronous::Normal => "NORMAL",
                Synchronous::Full => "FULL",
            };
            conn.pragma_update(None, "synchronous", sync)
                .map_err(|e| fail(&e))?;
        }
        if version == 0 && !options.create {
            return Err(StoreError::Corrupt(
                "the database holds no schema".to_owned(),
            ));
        }
        let reader = if options.read_only {
            None
        } else {
            let reader = Connection::open_with_flags(
                &options.path,
                OpenFlags::SQLITE_OPEN_READ_ONLY | OpenFlags::SQLITE_OPEN_NO_MUTEX,
            )
            .map_err(|e| fail(&e))?;
            reader
                .busy_timeout(Duration::from_millis(busy_ms))
                .map_err(|e| fail(&e))?;
            Some(reader)
        };
        let db = Self {
            inner: Mutex::new(Some(Inner {
                conn,
                reader,
                cache: None,
            })),
            path: options.path.clone(),
            read_only: options.read_only,
            busy_ms,
            reply_bytes: options.reply_bytes.max(1),
        };
        Ok(db)
    }

    fn with<T>(
        &self,
        run: impl FnOnce(&mut Inner) -> Result<T, StoreError>,
    ) -> Result<T, ProviderError> {
        let mut guard: MutexGuard<'_, Option<Inner>> =
            self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        let inner = guard
            .as_mut()
            .ok_or_else(|| StoreError::Gone("the database is closed".to_owned()))?;
        Ok(run(inner)?)
    }

    fn tables(&self, inner: &mut Inner) -> Result<Arc<Vec<TableInfo>>, StoreError> {
        let busy = self.busy_ms;
        let fail = move |e: rusqlite::Error| store_error(&e, busy);
        let schema_version: i64 = inner
            .conn
            .pragma_query_value(None, "schema_version", |row| row.get(0))
            .map_err(fail)?;
        if let Some((have, tables)) = &inner.cache
            && *have == schema_version
        {
            return Ok(Arc::clone(tables));
        }
        let present: bool = inner
            .conn
            .query_row(
                "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE type = 'table' AND name = ?)",
                [crate::HELD],
                |row| row.get(0),
            )
            .map_err(fail)?;
        let mut found = Vec::new();
        if present {
            let mut stmt = inner
                .conn
                .prepare(&format!("SELECT info FROM {} ORDER BY rowid", crate::HELD))
                .map_err(fail)?;
            let rows = stmt
                .query_map([], |row| row.get::<_, String>(0))
                .map_err(fail)?;
            for text in rows {
                let text = text.map_err(fail)?;
                found.push(table_of(&text).ok_or_else(|| {
                    StoreError::Corrupt("a table's description cannot be read".to_owned())
                })?);
            }
        }
        let tables = Arc::new(found);
        inner.cache = Some((schema_version, Arc::clone(&tables)));
        Ok(tables)
    }

    fn writable(&self) -> Result<(), StoreError> {
        if self.read_only {
            Err(StoreError::ReadOnly(
                "the database was opened read-only".to_owned(),
            ))
        } else {
            Ok(())
        }
    }

    fn migrate_in(&self, inner: &mut Inner, migration: &Migration) -> Result<(), StoreError> {
        self.writable()?;
        let busy = self.busy_ms;
        let fail = move |e: rusqlite::Error| store_error(&e, busy);
        let mut tables: Vec<TableInfo> = (*self.tables(inner)?).clone();
        let tx = inner
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(fail)?;
        tx.execute_batch(&format!(
            "CREATE TABLE IF NOT EXISTS {} (name TEXT PRIMARY KEY, info TEXT NOT NULL)",
            crate::HELD
        ))
        .map_err(fail)?;
        for step in &migration.steps {
            match step {
                Step::CreateTable { table } => {
                    if tables.iter().any(|have| have.name == table.name) {
                        return Err(StoreError::Conflict(format!(
                            "the table {} exists",
                            table.name
                        )));
                    }
                    if !is_ident(&table.name)
                        || table.name.starts_with("sqlite_")
                        || table.name == crate::HELD
                        || table.columns.iter().any(|c| !is_ident(&c.name))
                    {
                        return Err(StoreError::Bad(format!(
                            "{:?} is not a name a table or column may have",
                            table.name
                        )));
                    }
                    if table.key.is_empty() || table.key.iter().any(|k| table.column(k).is_none()) {
                        return Err(StoreError::Bad(format!(
                            "the key of {} names a column it lacks",
                            table.name
                        )));
                    }
                    let columns: Vec<String> = table
                        .columns
                        .iter()
                        .map(|c| {
                            format!(
                                "{} {}{}",
                                quote(&c.name),
                                sql_type(c.kind),
                                if c.nullable { "" } else { " NOT NULL" }
                            )
                        })
                        .collect();
                    let key: Vec<String> = table.key.iter().map(|k| quote(k)).collect();
                    tx.execute_batch(&format!(
                        "CREATE TABLE {} ({}, PRIMARY KEY ({}))",
                        quote(&table.name),
                        columns.join(", "),
                        key.join(", ")
                    ))
                    .map_err(fail)?;
                    for index in &table.indexes {
                        create_index(&tx, &table.name, table, index, busy)?;
                    }
                    tables.push(table.clone());
                }
                Step::AddColumn { table, column } => {
                    if !column.nullable {
                        return Err(StoreError::Bad(
                            "a column added to a table may be null".to_owned(),
                        ));
                    }
                    if !is_ident(&column.name) {
                        return Err(StoreError::Bad(format!(
                            "{:?} is not a name a column may have",
                            column.name
                        )));
                    }
                    let info = tables
                        .iter_mut()
                        .find(|t| &t.name == table)
                        .ok_or_else(|| StoreError::Bad(format!("there is no table {table}")))?;
                    if info.column(&column.name).is_some() {
                        return Err(StoreError::Conflict(format!(
                            "the column {} exists",
                            column.name
                        )));
                    }
                    tx.execute_batch(&format!(
                        "ALTER TABLE {} ADD COLUMN {} {}",
                        quote(table),
                        quote(&column.name),
                        sql_type(column.kind)
                    ))
                    .map_err(fail)?;
                    info.columns.push(column.clone());
                }
                Step::CreateIndex { table, index } => {
                    let info = tables
                        .iter_mut()
                        .find(|t| &t.name == table)
                        .ok_or_else(|| StoreError::Bad(format!("there is no table {table}")))?;
                    create_index(&tx, table, info, index, busy)?;
                    info.indexes.push(index.clone());
                }
                Step::DropIndex { table, name } => {
                    let info = tables
                        .iter_mut()
                        .find(|t| &t.name == table)
                        .ok_or_else(|| StoreError::Bad(format!("there is no table {table}")))?;
                    if !info.indexes.iter().any(|i| &i.name == name) {
                        return Err(StoreError::Bad(format!("there is no index {name}")));
                    }
                    tx.execute_batch(&format!("DROP INDEX {}", quote(name)))
                        .map_err(fail)?;
                    info.indexes.retain(|i| &i.name != name);
                }
            }
        }
        if tables
            .iter()
            .any(|t| t.name == "events" && t.column(SEARCH_COLUMN).is_some())
        {
            ensure_search(&tx, busy)?;
        }
        for table in &tables {
            tx.execute(
                &format!(
                    "INSERT OR REPLACE INTO {} (name, info) VALUES (?, ?)",
                    crate::HELD
                ),
                [&table.name, &table_json(table)],
            )
            .map_err(fail)?;
        }
        tx.execute_batch(&format!("PRAGMA user_version = {}", migration.to_version))
            .map_err(fail)?;
        tx.commit().map_err(fail)?;
        inner.cache = None;
        Ok(())
    }

    fn select_in(&self, inner: &mut Inner, select: &Select) -> Result<Rows, StoreError> {
        let busy = self.busy_ms;
        let fail = move |e: rusqlite::Error| store_error(&e, busy);
        let tables = self.tables(inner)?;
        let info = find(&tables, &select.table)?;
        let plan = plan_select(info, select)?;
        if plan.empty {
            return Ok(Rows {
                columns: plan.columns,
                ..Rows::default()
            });
        }
        let mut fetched = plan.columns.clone();
        for by in &plan.order {
            if !fetched.contains(&by.column) {
                fetched.push(by.column.clone());
            }
        }
        let mut params = Vec::new();
        let filter = where_clause(&plan.filter, &mut params);
        let order: Vec<String> = plan
            .order
            .iter()
            .map(|o| format!("{}{}", quote(&o.column), if o.desc { " DESC" } else { "" }))
            .collect();
        let columns: Vec<String> = fetched.iter().map(|c| quote(c)).collect();
        let sql = format!(
            "SELECT {} FROM {}{filter} ORDER BY {} LIMIT {}",
            columns.join(", "),
            quote(&info.name),
            order.join(", "),
            plan.limit + 1
        );
        let kinds: Vec<Option<ColumnType>> = fetched
            .iter()
            .map(|name| info.column(name).map(|c| c.kind))
            .collect();
        let mut stmt = inner.conn.prepare_cached(&sql).map_err(fail)?;
        let mut rows = stmt.query(params_from_iter(params)).map_err(fail)?;
        let mut out: Vec<Vec<Cell>> = Vec::new();
        let mut last: Option<Vec<Cell>> = None;
        let mut bytes = 0_usize;
        let mut more = false;
        while let Some(row) = rows.next().map_err(fail)? {
            let cells: Vec<Cell> = kinds
                .iter()
                .enumerate()
                .map(|(at, kind)| row.get_ref(at).map_or(Cell::Null, |v| cell_of(*kind, v)))
                .collect();
            let size: usize = cells.iter().map(Cell::size).sum();
            if out.len() >= plan.limit || (!out.is_empty() && bytes + size > self.reply_bytes) {
                more = true;
                break;
            }
            bytes += size;
            last = Some(cells.clone());
            out.push(cells[..plan.columns.len()].to_vec());
        }
        let next = match (more, last) {
            (true, Some(cells)) => Some(next_cursor(&plan.order, &|name| {
                fetched
                    .iter()
                    .position(|c| c == name)
                    .and_then(|at| cells.get(at).cloned())
                    .unwrap_or(Cell::Null)
            })),
            _ => None,
        };
        Ok(Rows {
            columns: plan.columns,
            rows: out,
            next,
        })
    }

    fn aggregate_in(&self, inner: &mut Inner, aggregate: &Aggregate) -> Result<Rows, StoreError> {
        let busy = self.busy_ms;
        let fail = move |e: rusqlite::Error| store_error(&e, busy);
        let tables = self.tables(inner)?;
        let info = find(&tables, &aggregate.table)?;
        check_filter(info, &aggregate.filter)?;
        for name in aggregate
            .group_by
            .iter()
            .chain(aggregate.figures.iter().filter_map(|f| f.column.as_ref()))
        {
            if info.column(name).is_none() {
                return Err(StoreError::Bad(format!(
                    "the table {} has no column {name}",
                    info.name
                )));
            }
        }
        let mut select: Vec<String> = aggregate.group_by.iter().map(|c| quote(c)).collect();
        let mut kinds: Vec<Option<ColumnType>> = aggregate
            .group_by
            .iter()
            .map(|c| info.column(c).map(|c| c.kind))
            .collect();
        for figure in &aggregate.figures {
            let column = figure.column.as_deref();
            let kind = column.and_then(|c| info.column(c)).map(|c| c.kind);
            let (call, out) = match (figure.func, column) {
                (AggFn::Count, None) => ("COUNT(*)".to_owned(), Some(ColumnType::Int)),
                (AggFn::Count, Some(c)) => (format!("COUNT({})", quote(c)), Some(ColumnType::Int)),
                (_, None) => {
                    return Err(StoreError::Bad(format!("{} needs a column", figure.name)));
                }
                (AggFn::Sum, Some(c)) => {
                    if !matches!(kind, Some(ColumnType::Int | ColumnType::Real)) {
                        return Err(StoreError::Bad(format!(
                            "{} sums a column that is not a number",
                            figure.name
                        )));
                    }
                    (format!("SUM({})", quote(c)), None)
                }
                (AggFn::Min, Some(c)) => (format!("MIN({})", quote(c)), kind),
                (AggFn::Max, Some(c)) => (format!("MAX({})", quote(c)), kind),
            };
            select.push(call);
            kinds.push(out);
        }
        let mut params = Vec::new();
        let filter = where_clause(&aggregate.filter, &mut params);
        let group = if aggregate.group_by.is_empty() {
            String::new()
        } else {
            let by: Vec<String> = aggregate.group_by.iter().map(|c| quote(c)).collect();
            format!(" GROUP BY {0} ORDER BY {0}", by.join(", "))
        };
        let sql = format!(
            "SELECT {} FROM {}{filter}{group}",
            select.join(", "),
            quote(&info.name)
        );
        let mut stmt = inner.conn.prepare_cached(&sql).map_err(fail)?;
        let mut rows = stmt.query(params_from_iter(params)).map_err(fail)?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(fail)? {
            out.push(
                kinds
                    .iter()
                    .enumerate()
                    .map(|(at, kind)| row.get_ref(at).map_or(Cell::Null, |v| cell_of(*kind, v)))
                    .collect(),
            );
        }
        let mut columns = aggregate.group_by.clone();
        columns.extend(aggregate.figures.iter().map(|f| f.name.clone()));
        Ok(Rows {
            columns,
            rows: out,
            next: None,
        })
    }

    fn write_in(&self, inner: &mut Inner, write: &Write) -> Result<Written, StoreError> {
        self.writable()?;
        let busy = self.busy_ms;
        let fail = move |e: rusqlite::Error| store_error(&e, busy);
        let tables = self.tables(inner)?;
        let tx = inner
            .conn
            .transaction_with_behavior(TransactionBehavior::Immediate)
            .map_err(fail)?;
        let mut affected = 0_u64;
        for change in &write.ops {
            affected += match change {
                Change::Insert { table, row } | Change::Upsert { table, row } => {
                    let info = find(&tables, table)?;
                    let row = normalize_row(info, row)?;
                    let names: Vec<String> = info.columns.iter().map(|c| quote(&c.name)).collect();
                    let marks = vec!["?"; names.len()].join(", ");
                    let mut sql = format!(
                        "INSERT INTO {} ({}) VALUES ({marks})",
                        quote(table),
                        names.join(", ")
                    );
                    if matches!(change, Change::Upsert { .. }) {
                        let key: Vec<String> = info.key.iter().map(|k| quote(k)).collect();
                        let sets: Vec<String> = info
                            .columns
                            .iter()
                            .filter(|c| !info.key.contains(&c.name))
                            .map(|c| format!("{0} = excluded.{0}", quote(&c.name)))
                            .collect();
                        if sets.is_empty() {
                            sql.push_str(&format!(" ON CONFLICT ({}) DO NOTHING", key.join(", ")));
                        } else {
                            sql.push_str(&format!(
                                " ON CONFLICT ({}) DO UPDATE SET {}",
                                key.join(", "),
                                sets.join(", ")
                            ));
                        }
                    }
                    let values: Vec<Value> = info
                        .columns
                        .iter()
                        .map(|c| value_of(row.get(&c.name).unwrap_or(&Cell::Null)))
                        .collect();
                    tx.prepare_cached(&sql)
                        .and_then(|mut stmt| stmt.execute(params_from_iter(values)))
                        .map_err(fail)?;
                    1
                }
                Change::Update { update } => update_in(&tx, &tables, update, busy)?,
                Change::Delete { table, filter } => {
                    let info = find(&tables, table)?;
                    check_filter(info, filter)?;
                    let mut params = Vec::new();
                    let clause = where_clause(filter, &mut params);
                    let sql = format!("DELETE FROM {}{clause}", quote(table));
                    tx.prepare_cached(&sql)
                        .and_then(|mut stmt| stmt.execute(params_from_iter(params)))
                        .map_err(fail)? as u64
                }
            };
        }
        tx.commit().map_err(fail)?;
        Ok(Written { affected })
    }

    fn sql_in(&self, inner: &mut Inner, sql: &Sql) -> Result<Rows, StoreError> {
        let busy = self.busy_ms;
        let first = sql
            .text
            .trim_start()
            .split(|c: char| !c.is_ascii_alphabetic())
            .next()
            .unwrap_or("")
            .to_ascii_lowercase();
        if first != "select" && first != "with" {
            return Err(StoreError::Bad(
                "only a SELECT or WITH statement runs".to_owned(),
            ));
        }
        let conn = inner.reader.as_ref().unwrap_or(&inner.conn);
        conn.pragma_update(None, "query_only", "ON")
            .map_err(|e| store_error(&e, busy))?;
        let mut stmt = conn
            .prepare(&sql.text)
            .map_err(|e| StoreError::Bad(format!("the statement is refused: {e}")))?;
        if !stmt.readonly() {
            return Err(StoreError::Bad(
                "the statement changes the database".to_owned(),
            ));
        }
        let columns: Vec<String> = stmt.column_names().map(ToOwned::to_owned).collect();
        let params: Vec<Value> = sql.params.iter().map(value_of).collect();
        let mut rows = stmt
            .query(params_from_iter(params))
            .map_err(|e| store_error(&e, busy))?;
        let mut out = Vec::new();
        while let Some(row) = rows.next().map_err(|e| store_error(&e, busy))? {
            out.push(
                (0..columns.len())
                    .map(|at| row.get_ref(at).map_or(Cell::Null, |v| cell_of(None, v)))
                    .collect(),
            );
        }
        Ok(Rows {
            columns,
            rows: out,
            next: None,
        })
    }
}

/// The full-text index of `events.text` and the triggers that keep it, made once and filled from what the table already holds.
fn ensure_search(tx: &rusqlite::Transaction<'_>, busy: u64) -> Result<(), StoreError> {
    let fail = |e: rusqlite::Error| store_error(&e, busy);
    let exists: bool = tx
        .query_row(
            "SELECT EXISTS(SELECT 1 FROM sqlite_master WHERE name = ?)",
            [SEARCH_TABLE],
            |row| row.get(0),
        )
        .map_err(fail)?;
    if exists {
        return Ok(());
    }
    tx.execute_batch(&format!(
        "CREATE VIRTUAL TABLE {t} USING fts5(text, content='events', content_rowid='rowid');
         CREATE TRIGGER {t}_ai AFTER INSERT ON events BEGIN
           INSERT INTO {t}(rowid, text) VALUES (new.rowid, new.text);
         END;
         CREATE TRIGGER {t}_ad AFTER DELETE ON events BEGIN
           INSERT INTO {t}({t}, rowid, text) VALUES ('delete', old.rowid, old.text);
         END;
         CREATE TRIGGER {t}_au AFTER UPDATE ON events BEGIN
           INSERT INTO {t}({t}, rowid, text) VALUES ('delete', old.rowid, old.text);
           INSERT INTO {t}(rowid, text) VALUES (new.rowid, new.text);
         END;
         INSERT INTO {t}({t}) VALUES ('rebuild');",
        t = SEARCH_TABLE
    ))
    .map_err(fail)
}

fn create_index(
    tx: &rusqlite::Transaction<'_>,
    table: &str,
    info: &TableInfo,
    index: &IndexInfo,
    busy: u64,
) -> Result<(), StoreError> {
    if !is_ident(&index.name) || index.columns.is_empty() {
        return Err(StoreError::Bad(format!(
            "{:?} is not a name an index may have",
            index.name
        )));
    }
    if index.columns.iter().any(|c| info.column(c).is_none()) {
        return Err(StoreError::Bad(format!(
            "the index {} names a column the table lacks",
            index.name
        )));
    }
    let columns: Vec<String> = index.columns.iter().map(|c| quote(c)).collect();
    tx.execute_batch(&format!(
        "CREATE {}INDEX {} ON {} ({})",
        if index.unique { "UNIQUE " } else { "" },
        quote(&index.name),
        quote(table),
        columns.join(", ")
    ))
    .map_err(|e| store_error(&e, busy))
}

fn update_in(
    tx: &rusqlite::Transaction<'_>,
    tables: &[TableInfo],
    update: &Update,
    busy: u64,
) -> Result<u64, StoreError> {
    let info = find(tables, &update.table)?;
    check_filter(info, &update.filter)?;
    let mut sets = Vec::new();
    let mut params = Vec::new();
    for (name, cell) in &update.set {
        let cell = normalize_cell(info, name, cell)?;
        sets.push(format!("{} = ?", quote(name)));
        params.push(value_of(&cell));
    }
    for (name, by) in &update.add {
        if !info.column(name).is_some_and(|c| c.kind == ColumnType::Int) {
            return Err(StoreError::Bad(format!(
                "{name} is not an integer column of {}",
                info.name
            )));
        }
        sets.push(format!("{0} = COALESCE({0}, 0) + ?", quote(name)));
        params.push(Value::Integer(*by));
    }
    if sets.is_empty() {
        return Err(StoreError::Bad("an update changes no column".to_owned()));
    }
    let clause = where_clause(&update.filter, &mut params);
    let sql = format!(
        "UPDATE {} SET {}{clause}",
        quote(&update.table),
        sets.join(", ")
    );
    tx.prepare_cached(&sql)
        .and_then(|mut stmt| stmt.execute(params_from_iter(params)))
        .map(|n| n as u64)
        .map_err(|e| store_error(&e, busy))
}

#[cfg(unix)]
fn make_private(path: &Path) -> Result<(), StoreError> {
    use std::os::unix::fs::OpenOptionsExt as _;
    std::fs::OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
        .map(drop)
        .map_err(|_| StoreError::Gone("the database file cannot be made".to_owned()))
}

#[cfg(not(unix))]
fn make_private(_path: &Path) -> Result<(), StoreError> {
    // The home folder's own permissions protect the file.
    Ok(())
}

impl HistoryDb for SqliteDb {
    fn schema(&self) -> Result<SchemaState, ProviderError> {
        self.with(|inner| {
            let tables = self.tables(inner)?;
            let version: i64 = inner
                .conn
                .pragma_query_value(None, "user_version", |row| row.get(0))
                .map_err(|e| store_error(&e, self.busy_ms))?;
            Ok(SchemaState {
                version: u32::try_from(version).unwrap_or(0),
                tables: (*tables).clone(),
            })
        })
    }

    fn migrate(&self, migration: Migration) -> Result<(), ProviderError> {
        self.with(|inner| self.migrate_in(inner, &migration))
    }

    fn select(&self, select: Select) -> Result<Rows, ProviderError> {
        self.with(|inner| self.select_in(inner, &select))
    }

    fn count(&self, select: Select) -> Result<u64, ProviderError> {
        self.with(|inner| {
            let tables = self.tables(inner)?;
            let info = find(&tables, &select.table)?;
            check_filter(info, &select.filter)?;
            let mut params = Vec::new();
            let clause = where_clause(&select.filter, &mut params);
            let sql = format!("SELECT COUNT(*) FROM {}{clause}", quote(&info.name));
            let n: i64 = inner
                .conn
                .prepare_cached(&sql)
                .and_then(|mut stmt| stmt.query_row(params_from_iter(params), |row| row.get(0)))
                .map_err(|e| store_error(&e, self.busy_ms))?;
            Ok(u64::try_from(n).unwrap_or(0))
        })
    }

    fn aggregate(&self, aggregate: Aggregate) -> Result<Rows, ProviderError> {
        self.with(|inner| self.aggregate_in(inner, &aggregate))
    }

    fn write(&self, write: Write) -> Result<Written, ProviderError> {
        self.with(|inner| self.write_in(inner, &write))
    }

    fn sql(&self, sql: Sql) -> Result<Rows, ProviderError> {
        self.with(|inner| self.sql_in(inner, &sql))
    }

    fn revision(&self) -> Result<String, ProviderError> {
        self.with(|inner| {
            let fail = |e: rusqlite::Error| store_error(&e, self.busy_ms);
            let data: i64 = inner
                .conn
                .pragma_query_value(None, "data_version", |row| row.get(0))
                .map_err(fail)?;
            let schema: i64 = inner
                .conn
                .pragma_query_value(None, "schema_version", |row| row.get(0))
                .map_err(fail)?;
            Ok(format!("{data}.{}.{schema}", inner.conn.total_changes()))
        })
    }

    fn stats(&self) -> Result<Stats, ProviderError> {
        self.with(|inner| {
            let fail = |e: rusqlite::Error| store_error(&e, self.busy_ms);
            let tables = self.tables(inner)?;
            let pages: i64 = inner
                .conn
                .pragma_query_value(None, "page_count", |row| row.get(0))
                .map_err(fail)?;
            let size: i64 = inner
                .conn
                .pragma_query_value(None, "page_size", |row| row.get(0))
                .map_err(fail)?;
            let mut rows = std::collections::BTreeMap::new();
            for table in tables.iter() {
                let n: i64 = inner
                    .conn
                    .query_row(
                        &format!("SELECT COUNT(*) FROM {}", quote(&table.name)),
                        [],
                        |row| row.get(0),
                    )
                    .map_err(fail)?;
                rows.insert(table.name.clone(), u64::try_from(n).unwrap_or(0));
            }
            Ok(Stats {
                bytes: u64::try_from(pages.saturating_mul(size)).ok(),
                rows,
            })
        })
    }

    fn compact(&self) -> Result<Compacted, ProviderError> {
        self.with(|inner| {
            self.writable()?;
            let fail = |e: rusqlite::Error| store_error(&e, self.busy_ms);
            let size = |path: &Path| {
                let main = std::fs::metadata(path).map_or(0, |m| m.len());
                let mut wal = path.as_os_str().to_owned();
                wal.push("-wal");
                main + std::fs::metadata(wal).map_or(0, |m| m.len())
            };
            let before = size(&self.path);
            inner
                .conn
                .query_row("PRAGMA wal_checkpoint(TRUNCATE)", [], |_| Ok(()))
                .map_err(fail)?;
            inner.conn.execute_batch("VACUUM").map_err(fail)?;
            Ok(Compacted {
                before,
                after: size(&self.path),
            })
        })
    }

    fn release(&self) -> Result<(), ProviderError> {
        let mut guard = self.inner.lock().unwrap_or_else(PoisonError::into_inner);
        *guard = None;
        Ok(())
    }
}
