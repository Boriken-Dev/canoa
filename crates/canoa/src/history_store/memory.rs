//! The store that keeps its tables in memory: the kind's own stand-in, which tests of the host run on and a provider is measured
//! against. It forgets everything when it is dropped.

#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the tables are kept behind a std lock"
)]

use alloc::borrow::ToOwned as _;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;
use core::cmp::Ordering;
use std::sync::Mutex;

use coqui_acp_api::guatiao::library::{Object, ProviderError};

use super::eval::{
    check_filter, cmp_cells, next_cursor, normalize_cell, normalize_row, plan_select, row_matches,
};
use super::{
    AggFn, Aggregate, Cell, Change, ColumnType, Compacted, HistoryDb, HistoryStore, Migration,
    OpenRequest, Rows, SchemaState, Select, Sql, Stats, Step, StoreError, StoreInfo, TableInfo,
    Update, Write, Written,
};

type Row = BTreeMap<String, Cell>;

#[derive(Clone, Default)]
struct State {
    version: u32,
    tables: Vec<TableInfo>,
    rows: BTreeMap<String, Vec<Row>>,
    revision: u64,
}

impl State {
    fn table(&self, name: &str) -> Result<&TableInfo, StoreError> {
        self.tables
            .iter()
            .find(|table| table.name == name)
            .ok_or_else(|| StoreError::Bad(format!("there is no table {name}")))
    }

    fn rows_of(&self, name: &str) -> &[Row] {
        self.rows.get(name).map_or(&[], Vec::as_slice)
    }
}

/// An in-memory store.
pub struct MemoryDb {
    state: Mutex<State>,
    reply_bytes: usize,
}

impl core::fmt::Debug for MemoryDb {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("MemoryDb").finish_non_exhaustive()
    }
}

impl Default for MemoryDb {
    fn default() -> Self {
        Self::new()
    }
}

impl MemoryDb {
    /// An empty store with no schema.
    #[must_use]
    pub fn new() -> Self {
        Self {
            state: Mutex::new(State::default()),
            reply_bytes: 4 * 1024 * 1024,
        }
    }

    fn lock(&self) -> std::sync::MutexGuard<'_, State> {
        self.state
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner)
    }

    fn rows_matching(
        state: &State,
        table: &str,
        filter: &super::Filter,
    ) -> Result<Vec<Row>, StoreError> {
        let info = state.table(table)?;
        check_filter(info, filter)?;
        Ok(state
            .rows_of(table)
            .iter()
            .filter(|row| row_matches(filter, &|name| row.get(name).cloned().unwrap_or(Cell::Null)))
            .cloned()
            .collect())
    }

    fn do_select(&self, select: &Select) -> Result<Rows, StoreError> {
        let state = self.lock();
        let info = state.table(&select.table)?;
        let plan = plan_select(info, select)?;
        if plan.empty {
            return Ok(Rows {
                columns: plan.columns,
                ..Rows::default()
            });
        }
        let mut found: Vec<Row> = state
            .rows_of(&select.table)
            .iter()
            .filter(|row| {
                row_matches(&plan.filter, &|name| {
                    row.get(name).cloned().unwrap_or(Cell::Null)
                })
            })
            .cloned()
            .collect();
        found.sort_by(|a, b| {
            for by in &plan.order {
                let (x, y) = (
                    a.get(&by.column).unwrap_or(&Cell::Null),
                    b.get(&by.column).unwrap_or(&Cell::Null),
                );
                let order = cmp_cells(x, y);
                let order = if by.desc { order.reverse() } else { order };
                if order != Ordering::Equal {
                    return order;
                }
            }
            Ordering::Equal
        });
        let mut rows = Vec::new();
        let mut bytes = 0_usize;
        let mut more = false;
        let mut last: Option<&Row> = None;
        for row in &found {
            let cells: Vec<Cell> = plan
                .columns
                .iter()
                .map(|name| row.get(name).cloned().unwrap_or(Cell::Null))
                .collect();
            let size: usize = cells.iter().map(Cell::size).sum();
            if rows.len() >= plan.limit || (!rows.is_empty() && bytes + size > self.reply_bytes) {
                more = true;
                break;
            }
            bytes += size;
            rows.push(cells);
            last = Some(row);
        }
        let next = if more {
            last.map(|row| {
                next_cursor(&plan.order, &|name| {
                    row.get(name).cloned().unwrap_or(Cell::Null)
                })
            })
        } else {
            None
        };
        Ok(Rows {
            columns: plan.columns,
            rows,
            next,
        })
    }

    fn do_aggregate(&self, aggregate: &Aggregate) -> Result<Rows, StoreError> {
        let state = self.lock();
        let info = state.table(&aggregate.table)?;
        for name in aggregate
            .group_by
            .iter()
            .chain(aggregate.figures.iter().filter_map(|f| f.column.as_ref()))
        {
            if info.column(name).is_none() {
                return Err(StoreError::Bad(format!(
                    "the table {} has no column {name}",
                    aggregate.table
                )));
            }
        }
        for figure in &aggregate.figures {
            let needs = !matches!(figure.func, AggFn::Count);
            if needs && figure.column.is_none() {
                return Err(StoreError::Bad(format!("{} needs a column", figure.name)));
            }
            if figure.func == AggFn::Sum
                && figure
                    .column
                    .as_deref()
                    .and_then(|c| info.column(c))
                    .is_some_and(|c| !matches!(c.kind, ColumnType::Int | ColumnType::Real))
            {
                return Err(StoreError::Bad(format!(
                    "{} sums a column that is not a number",
                    figure.name
                )));
            }
        }
        let rows = Self::rows_matching(&state, &aggregate.table, &aggregate.filter)?;
        let mut groups: Vec<(Vec<Cell>, Vec<Row>)> = Vec::new();
        for row in rows {
            let key: Vec<Cell> = aggregate
                .group_by
                .iter()
                .map(|c| row.get(c).cloned().unwrap_or(Cell::Null))
                .collect();
            match groups.iter_mut().find(|(have, _)| {
                have.len() == key.len()
                    && have
                        .iter()
                        .zip(&key)
                        .all(|(a, b)| cmp_cells(a, b) == Ordering::Equal)
            }) {
                Some((_, members)) => members.push(row),
                None => groups.push((key, alloc::vec![row])),
            }
        }
        if groups.is_empty() && aggregate.group_by.is_empty() {
            groups.push((Vec::new(), Vec::new()));
        }
        groups.sort_by(|(a, _), (b, _)| {
            a.iter()
                .zip(b)
                .map(|(x, y)| cmp_cells(x, y))
                .find(|o| *o != Ordering::Equal)
                .unwrap_or(Ordering::Equal)
        });
        let mut columns = aggregate.group_by.clone();
        columns.extend(aggregate.figures.iter().map(|f| f.name.clone()));
        let out = groups
            .into_iter()
            .map(|(mut cells, members)| {
                for figure in &aggregate.figures {
                    cells.push(figure_of(figure, &members));
                }
                cells
            })
            .collect();
        Ok(Rows {
            columns,
            rows: out,
            next: None,
        })
    }

    fn do_write(&self, write: &Write) -> Result<Written, StoreError> {
        let mut guard = self.lock();
        let mut next = guard.clone();
        let mut affected = 0_u64;
        for change in &write.ops {
            affected += apply(&mut next, change)?;
        }
        check_unique(&next)?;
        next.revision += 1;
        *guard = next;
        Ok(Written { affected })
    }

    fn do_migrate(&self, migration: &Migration) -> Result<(), StoreError> {
        let mut guard = self.lock();
        let mut next = guard.clone();
        for step in &migration.steps {
            match step {
                Step::CreateTable { table } => {
                    if next.tables.iter().any(|have| have.name == table.name) {
                        return Err(StoreError::Conflict(format!(
                            "the table {} exists",
                            table.name
                        )));
                    }
                    next.rows.insert(table.name.clone(), Vec::new());
                    next.tables.push(table.clone());
                }
                Step::AddColumn { table, column } => {
                    if !column.nullable {
                        return Err(StoreError::Bad(
                            "a column added to a table may be null".to_owned(),
                        ));
                    }
                    let info = table_mut(&mut next, table)?;
                    if info.column(&column.name).is_some() {
                        return Err(StoreError::Conflict(format!(
                            "the column {} exists",
                            column.name
                        )));
                    }
                    info.columns.push(column.clone());
                }
                Step::CreateIndex { table, index } => {
                    let info = table_mut(&mut next, table)?;
                    if index.columns.iter().any(|c| info.column(c).is_none()) {
                        return Err(StoreError::Bad(format!(
                            "the index {} names a column the table lacks",
                            index.name
                        )));
                    }
                    info.indexes.push(index.clone());
                }
                Step::DropIndex { table, name } => {
                    let info = table_mut(&mut next, table)?;
                    info.indexes.retain(|index| &index.name != name);
                }
            }
        }
        check_unique(&next)?;
        next.version = migration.to_version;
        next.revision += 1;
        *guard = next;
        Ok(())
    }
}

fn table_mut<'a>(state: &'a mut State, name: &str) -> Result<&'a mut TableInfo, StoreError> {
    state
        .tables
        .iter_mut()
        .find(|table| table.name == name)
        .ok_or_else(|| StoreError::Bad(format!("there is no table {name}")))
}

fn figure_of(figure: &super::Figure, members: &[Row]) -> Cell {
    let values: Vec<&Cell> = match &figure.column {
        Some(column) => members
            .iter()
            .filter_map(|row| row.get(column))
            .filter(|cell| !cell.is_null())
            .collect(),
        None => Vec::new(),
    };
    match figure.func {
        AggFn::Count => {
            let n = if figure.column.is_some() {
                values.len()
            } else {
                members.len()
            };
            Cell::int(i64::try_from(n).unwrap_or(i64::MAX))
        }
        AggFn::Sum => {
            if values.is_empty() {
                Cell::Null
            } else if values.iter().all(|c| c.as_int().is_some()) {
                Cell::int(
                    values
                        .iter()
                        .filter_map(|c| c.as_int())
                        .fold(0_i64, i64::saturating_add),
                )
            } else {
                Cell::real(
                    values
                        .iter()
                        .map(|c| match c {
                            Cell::Int { value } => {
                                #[allow(clippy::cast_precision_loss, reason = "summed with reals")]
                                let v = *value as f64;
                                v
                            }
                            Cell::Real { value } => *value,
                            _ => 0.0,
                        })
                        .sum(),
                )
            }
        }
        AggFn::Min => values
            .into_iter()
            .min_by(|a, b| cmp_cells(a, b))
            .cloned()
            .unwrap_or(Cell::Null),
        AggFn::Max => values
            .into_iter()
            .max_by(|a, b| cmp_cells(a, b))
            .cloned()
            .unwrap_or(Cell::Null),
    }
}

fn key_of(info: &TableInfo, row: &Row) -> Vec<Cell> {
    info.key
        .iter()
        .map(|c| row.get(c).cloned().unwrap_or(Cell::Null))
        .collect()
}

fn same(a: &[Cell], b: &[Cell]) -> bool {
    a.len() == b.len()
        && a.iter()
            .zip(b)
            .all(|(x, y)| cmp_cells(x, y) == Ordering::Equal)
}

fn apply(state: &mut State, change: &Change) -> Result<u64, StoreError> {
    match change {
        Change::Insert { table, row } | Change::Upsert { table, row } => {
            let info = state.table(table)?.clone();
            let row = normalize_row(&info, row)?;
            let key = key_of(&info, &row);
            let rows = state.rows.entry(table.clone()).or_default();
            let at = rows
                .iter()
                .position(|have| same(&key_of(&info, have), &key));
            match (at, change) {
                (Some(_), Change::Insert { .. }) => Err(StoreError::Conflict(format!(
                    "{table} already holds that key"
                ))),
                (Some(at), _) => {
                    rows[at] = row;
                    Ok(1)
                }
                (None, _) => {
                    rows.push(row);
                    Ok(1)
                }
            }
        }
        Change::Update { update } => apply_update(state, update),
        Change::Delete { table, filter } => {
            let info = state.table(table)?.clone();
            check_filter(&info, filter)?;
            let rows = state.rows.entry(table.clone()).or_default();
            let before = rows.len();
            rows.retain(|row| {
                !row_matches(filter, &|name| row.get(name).cloned().unwrap_or(Cell::Null))
            });
            Ok((before - rows.len()) as u64)
        }
    }
}

fn apply_update(state: &mut State, update: &Update) -> Result<u64, StoreError> {
    let info = state.table(&update.table)?.clone();
    check_filter(&info, &update.filter)?;
    for (name, cell) in &update.set {
        normalize_cell(&info, name, cell)?;
    }
    for name in update.add.keys() {
        if !info.column(name).is_some_and(|c| c.kind == ColumnType::Int) {
            return Err(StoreError::Bad(format!(
                "{name} is not an integer column of {}",
                info.name
            )));
        }
    }
    let rows = state.rows.entry(update.table.clone()).or_default();
    let mut changed = 0;
    for row in rows.iter_mut() {
        if !row_matches(&update.filter, &|name| {
            row.get(name).cloned().unwrap_or(Cell::Null)
        }) {
            continue;
        }
        let mut next = row.clone();
        for (name, cell) in &update.set {
            next.insert(name.clone(), cell.clone());
        }
        for (name, by) in &update.add {
            let have = next.get(name).and_then(Cell::as_int).unwrap_or(0);
            next.insert(name.clone(), Cell::int(have.saturating_add(*by)));
        }
        *row = normalize_row(&info, &next)?;
        changed += 1;
    }
    Ok(changed)
}

fn check_unique(state: &State) -> Result<(), StoreError> {
    for table in &state.tables {
        let rows = state.rows_of(&table.name);
        for index in table.indexes.iter().filter(|i| i.unique) {
            let keys: Vec<Vec<Cell>> = rows
                .iter()
                .map(|row| {
                    index
                        .columns
                        .iter()
                        .map(|c| row.get(c).cloned().unwrap_or(Cell::Null))
                        .collect::<Vec<_>>()
                })
                .filter(|key| key.iter().all(|c| !c.is_null()))
                .collect();
            for (at, key) in keys.iter().enumerate() {
                if keys[..at].iter().any(|other| same(other, key)) {
                    return Err(StoreError::Conflict(format!(
                        "the unique index {} already holds that value",
                        index.name
                    )));
                }
            }
        }
    }
    Ok(())
}

impl HistoryDb for MemoryDb {
    fn schema(&self) -> Result<SchemaState, ProviderError> {
        let state = self.lock();
        Ok(SchemaState {
            version: state.version,
            tables: state.tables.clone(),
        })
    }

    fn migrate(&self, migration: Migration) -> Result<(), ProviderError> {
        Ok(self.do_migrate(&migration)?)
    }

    fn select(&self, select: Select) -> Result<Rows, ProviderError> {
        Ok(self.do_select(&select)?)
    }

    fn count(&self, select: Select) -> Result<u64, ProviderError> {
        let state = self.lock();
        let rows = Self::rows_matching(&state, &select.table, &select.filter)?;
        Ok(rows.len() as u64)
    }

    fn aggregate(&self, aggregate: Aggregate) -> Result<Rows, ProviderError> {
        Ok(self.do_aggregate(&aggregate)?)
    }

    fn write(&self, write: Write) -> Result<Written, ProviderError> {
        Ok(self.do_write(&write)?)
    }

    fn sql(&self, _sql: Sql) -> Result<Rows, ProviderError> {
        Err(StoreError::Unsupported("the memory store has no sql".to_owned()).into())
    }

    fn revision(&self) -> Result<String, ProviderError> {
        Ok(self.lock().revision.to_string())
    }

    fn stats(&self) -> Result<Stats, ProviderError> {
        let state = self.lock();
        Ok(Stats {
            bytes: None,
            rows: state
                .rows
                .iter()
                .map(|(name, rows)| (name.clone(), rows.len() as u64))
                .collect(),
        })
    }

    fn compact(&self) -> Result<Compacted, ProviderError> {
        Err(StoreError::Unsupported("the memory store has nothing to compact".to_owned()).into())
    }

    fn release(&self) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// The provider of [`MemoryDb`]: each open is a new empty store.
#[derive(Debug, Default, Clone, Copy)]
pub struct MemoryStore;

impl HistoryStore for MemoryStore {
    fn describe(&self) -> Result<StoreInfo, ProviderError> {
        let mut info = StoreInfo::new("memory", "Memory");
        "Sessions kept in memory, for tests and short runs.".clone_into(&mut info.summary);
        "Nowhere: it is forgotten when the host stops.".clone_into(&mut info.location);
        info.capabilities = vec![super::CAP_SEARCH.to_owned()];
        Ok(info)
    }

    fn open(&self, request: OpenRequest) -> Result<Object<dyn HistoryDb>, ProviderError> {
        let mut db = MemoryDb::new();
        db.reply_bytes = usize::try_from(request.reply_bytes)
            .unwrap_or(usize::MAX)
            .max(1);
        Ok(db.into_object())
    }
}
