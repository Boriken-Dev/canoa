//! What every provider shares: checking a query against a table, the order of values, the keyset cursor, and evaluating a filter over
//! a row (for a provider that is not a database).

use alloc::borrow::ToOwned as _;
use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::String;
use alloc::vec::Vec;
use core::cmp::Ordering;

use serde_json::{Value as Json, json};

use super::{Cell, ColumnType, Cond, Filter, Op, Order, Select, StoreError, TableInfo};

fn rank(cell: &Cell) -> u8 {
    match cell {
        Cell::Null => 0,
        Cell::Bool { .. } => 1,
        Cell::Int { .. } | Cell::Real { .. } => 2,
        Cell::Text { .. } | Cell::Json { .. } => 3,
        Cell::Bytes { .. } => 4,
    }
}

#[allow(
    clippy::cast_precision_loss,
    reason = "an integer compared with a real is compared as a real"
)]
fn number(cell: &Cell) -> f64 {
    match cell {
        Cell::Int { value } => *value as f64,
        Cell::Real { value } => *value,
        _ => 0.0,
    }
}

/// The order values sort in: null first, then booleans, numbers (an integer and a real by value), text, bytes.
#[must_use]
pub fn cmp_cells(a: &Cell, b: &Cell) -> Ordering {
    match (a, b) {
        (Cell::Int { value: x }, Cell::Int { value: y }) => x.cmp(y),
        (Cell::Bool { value: x }, Cell::Bool { value: y }) => x.cmp(y),
        (
            Cell::Text { value: x } | Cell::Json { value: x },
            Cell::Text { value: y } | Cell::Json { value: y },
        ) => x.cmp(y),
        (Cell::Bytes { .. }, Cell::Bytes { .. }) => a
            .as_bytes()
            .unwrap_or_default()
            .cmp(&b.as_bytes().unwrap_or_default()),
        _ if rank(a) == 2 && rank(b) == 2 => number(a).total_cmp(&number(b)),
        _ => rank(a).cmp(&rank(b)),
    }
}

fn fits(kind: ColumnType, cell: &Cell) -> bool {
    matches!(
        (kind, cell),
        (ColumnType::Text, Cell::Text { .. })
            | (ColumnType::Int, Cell::Int { .. })
            | (ColumnType::Real, Cell::Real { .. } | Cell::Int { .. })
            | (ColumnType::Bool, Cell::Bool { .. })
            | (ColumnType::Blob, Cell::Bytes { .. })
            | (ColumnType::Json, Cell::Json { .. })
    )
}

fn bad<T>(why: String) -> Result<T, StoreError> {
    Err(StoreError::Bad(why))
}

/// Checks one condition against `table`.
fn check_cond(table: &TableInfo, cond: &Cond) -> Result<(), StoreError> {
    let Some(column) = table.column(&cond.column) else {
        return bad(format!(
            "the table {} has no column {}",
            table.name, cond.column
        ));
    };
    let wants = match cond.op {
        Op::IsNull | Op::NotNull => Some(0),
        Op::In => None,
        _ => Some(1),
    };
    if wants.is_some_and(|n| cond.values.len() != n) {
        return bad(format!(
            "the condition on {} takes {} value(s)",
            cond.column,
            wants.unwrap_or(0)
        ));
    }
    for value in &cond.values {
        let text_op = matches!(
            cond.op,
            Op::Prefix | Op::Contains | Op::Suffix | Op::Matches
        );
        let ok = if text_op {
            matches!(column.kind, ColumnType::Text | ColumnType::Json)
                && matches!(value, Cell::Text { .. })
        } else {
            fits(column.kind, value)
                || (column.kind == ColumnType::Json && matches!(value, Cell::Text { .. }))
        };
        if !ok {
            return bad(format!(
                "the condition on {} compares with a value of another type",
                cond.column
            ));
        }
    }
    Ok(())
}

/// Checks every condition of `filter` against `table`.
///
/// # Errors
///
/// An unknown column, a wrong number of values, or a value of the wrong type.
pub fn check_filter(table: &TableInfo, filter: &Filter) -> Result<(), StoreError> {
    for cond in filter.iter().flatten() {
        check_cond(table, cond)?;
    }
    Ok(())
}

fn check_requires(table: &TableInfo, filter: &Filter) -> Result<(), StoreError> {
    for required in &table.requires {
        let all = !filter.is_empty()
            && filter.iter().all(|alternative| {
                alternative
                    .iter()
                    .any(|cond| cond.column == *required && cond.op == Op::Eq)
            });
        if !all {
            return bad(format!(
                "a select of {} must test {required} for equality",
                table.name
            ));
        }
    }
    Ok(())
}

/// A row of `table` made of `row`: every column present (a column left out is null), a whole number given for a real column made real.
///
/// # Errors
///
/// An unknown column, a null in a column that may not be, or a value of the wrong type.
pub fn normalize_row(
    table: &TableInfo,
    row: &BTreeMap<String, Cell>,
) -> Result<BTreeMap<String, Cell>, StoreError> {
    if let Some(name) = row.keys().find(|name| table.column(name).is_none()) {
        return bad(format!("the table {} has no column {name}", table.name));
    }
    let mut out = BTreeMap::new();
    for column in &table.columns {
        let cell = row.get(&column.name).cloned().unwrap_or(Cell::Null);
        let cell = match (column.kind, cell) {
            (ColumnType::Real, Cell::Int { value }) =>
            {
                #[allow(
                    clippy::cast_precision_loss,
                    reason = "a whole number given for a real column"
                )]
                Cell::real(value as f64)
            }
            (_, cell) => cell,
        };
        if cell.is_null() {
            if !column.nullable {
                return bad(format!("{}.{} may not be null", table.name, column.name));
            }
        } else if !fits(column.kind, &cell) {
            return bad(format!(
                "{}.{} holds a value of another type",
                table.name, column.name
            ));
        }
        out.insert(column.name.clone(), cell);
    }
    Ok(out)
}

/// `cell` as the column `name` of `table` holds it: a whole number given for a real column made real.
///
/// # Errors
///
/// An unknown column, a null in a column that may not be, or a value of the wrong type.
pub fn normalize_cell(table: &TableInfo, name: &str, cell: &Cell) -> Result<Cell, StoreError> {
    let Some(column) = table.column(name) else {
        return bad(format!("the table {} has no column {name}", table.name));
    };
    let cell = match (column.kind, cell) {
        (ColumnType::Real, Cell::Int { value }) =>
        {
            #[allow(
                clippy::cast_precision_loss,
                reason = "a whole number given for a real column"
            )]
            Cell::real(*value as f64)
        }
        (_, cell) => cell.clone(),
    };
    if cell.is_null() {
        if !column.nullable {
            return bad(format!("{}.{name} may not be null", table.name));
        }
    } else if !fits(column.kind, &cell) {
        return bad(format!(
            "{}.{name} holds a value of another type",
            table.name
        ));
    }
    Ok(cell)
}

fn holds(text: &str, op: Op, needle: &str) -> bool {
    match op {
        Op::Prefix => text.starts_with(needle),
        Op::Suffix => text.ends_with(needle),
        _ => text.contains(needle),
    }
}

/// The words of `text` as a full-text search sees them: runs of letters and digits, lower-cased.
#[must_use]
pub fn search_words(text: &str) -> Vec<String> {
    text.split(|c: char| !c.is_alphanumeric())
        .filter(|word| !word.is_empty())
        .map(str::to_lowercase)
        .collect()
}

/// Whether one condition holds for the value `have` of its column.
fn cond_holds(cond: &Cond, have: &Cell) -> bool {
    match cond.op {
        Op::IsNull => have.is_null(),
        Op::NotNull => !have.is_null(),
        _ if have.is_null() => false,
        Op::In => cond
            .values
            .iter()
            .any(|value| cmp_cells(have, value) == Ordering::Equal),
        Op::Matches => match (have.as_text(), cond.values.first().and_then(Cell::as_text)) {
            (Some(text), Some(query)) => {
                let (have, want) = (search_words(text), search_words(query));
                !want.is_empty() && want.iter().all(|word| have.contains(word))
            }
            _ => false,
        },
        Op::Prefix | Op::Contains | Op::Suffix => {
            match (have.as_text(), cond.values.first().and_then(Cell::as_text)) {
                (Some(text), Some(needle)) => holds(text, cond.op, needle),
                _ => false,
            }
        }
        op => {
            let Some(value) = cond.values.first() else {
                return false;
            };
            let order = cmp_cells(have, value);
            match op {
                Op::Eq => order == Ordering::Equal,
                Op::Ne => order != Ordering::Equal,
                Op::Lt => order == Ordering::Less,
                Op::Le => order != Ordering::Greater,
                Op::Gt => order == Ordering::Greater,
                _ => order != Ordering::Less,
            }
        }
    }
}

/// Whether a row, whose value of a column `get` answers, is kept by `filter`.
pub fn row_matches(filter: &Filter, get: &dyn Fn(&str) -> Cell) -> bool {
    filter.is_empty()
        || filter.iter().any(|alternative| {
            alternative
                .iter()
                .all(|cond| cond_holds(cond, &get(&cond.column)))
        })
}

fn cell_json(cell: &Cell) -> Json {
    match cell {
        Cell::Null => Json::Null,
        Cell::Bool { value } => json!({"b": value}),
        Cell::Int { value } => json!({"i": value}),
        Cell::Real { value } => json!({"r": value}),
        Cell::Text { value } => json!({"t": value}),
        Cell::Json { value } => json!({"j": value}),
        Cell::Bytes { .. } => {
            json!({"x": data_encoding::HEXLOWER.encode(&cell.as_bytes().unwrap_or_default())})
        }
    }
}

fn json_cell(value: &Json) -> Option<Cell> {
    if value.is_null() {
        return Some(Cell::Null);
    }
    let object = value.as_object()?;
    let (key, inner) = object.iter().next()?;
    Some(match key.as_str() {
        "b" => Cell::bool(inner.as_bool()?),
        "i" => Cell::int(inner.as_i64()?),
        "r" => Cell::real(inner.as_f64()?),
        "t" => Cell::text(inner.as_str()?),
        "j" => Cell::json(inner.as_str()?),
        "x" => Cell::bytes(
            data_encoding::HEXLOWER
                .decode(inner.as_str()?.as_bytes())
                .ok()?,
        ),
        _ => return None,
    })
}

/// The cursor that continues after a row: the values of its order columns, as text only the same select can read.
pub fn next_cursor(order: &[Order], get: &dyn Fn(&str) -> Cell) -> String {
    let cells: Vec<Json> = order.iter().map(|o| cell_json(&get(&o.column))).collect();
    Json::Array(cells).to_string()
}

/// The alternatives that keep the rows after the cursor's row, nulls being the lowest values.
fn keyset(order: &[Order], values: &[Cell]) -> Filter {
    let mut out: Filter = Vec::new();
    for (at, (by, value)) in order.iter().zip(values).enumerate() {
        let mut prefix: Vec<Cond> = order[..at]
            .iter()
            .zip(values)
            .map(|(o, v)| {
                if v.is_null() {
                    Cond::is_null(&o.column)
                } else {
                    Cond::eq(&o.column, v.clone())
                }
            })
            .collect();
        let lasts: Vec<Cond> = match (by.desc, value.is_null()) {
            (false, false) => alloc::vec![Cond::new(&by.column, Op::Gt, value.clone())],
            (false, true) => alloc::vec![Cond::not_null(&by.column)],
            (true, false) => alloc::vec![
                Cond::new(&by.column, Op::Lt, value.clone()),
                Cond::is_null(&by.column)
            ],
            (true, true) => Vec::new(),
        };
        for last in lasts {
            let mut alternative = prefix.clone();
            alternative.push(last);
            out.push(alternative);
        }
        prefix.clear();
    }
    out
}

/// A select checked against its table: what to answer, in what total order, and which rows.
#[derive(Debug, Clone, PartialEq)]
pub struct Plan {
    /// The columns to answer.
    pub columns: Vec<String>,
    /// The order, the key's columns appended.
    pub order: Vec<Order>,
    /// The rows to keep, the cursor's condition included.
    pub filter: Filter,
    /// The cursor leaves no row.
    pub empty: bool,
    /// The most rows to answer.
    pub limit: usize,
}

/// Checks `select` against `table` and works out its order and the rows after its cursor.
///
/// # Errors
///
/// An unknown table or column, a condition of the wrong type, a missing required equality, a cursor of another select, or no limit.
pub fn plan_select(table: &TableInfo, select: &Select) -> Result<Plan, StoreError> {
    if select.limit == 0 {
        return bad("a select needs a limit of at least one".to_owned());
    }
    check_filter(table, &select.filter)?;
    check_requires(table, &select.filter)?;
    let columns: Vec<String> = if select.columns.is_empty() {
        table.columns.iter().map(|c| c.name.clone()).collect()
    } else {
        select.columns.clone()
    };
    for name in columns.iter().chain(select.order.iter().map(|o| &o.column)) {
        if table.column(name).is_none() {
            return bad(format!("the table {} has no column {name}", table.name));
        }
    }
    let mut order = select.order.clone();
    for key in &table.key {
        if !order.iter().any(|o| &o.column == key) {
            order.push(Order::asc(key));
        }
    }
    let mut filter = select.filter.clone();
    let mut empty = false;
    if let Some(cursor) = &select.after {
        let values: Option<Vec<Cell>> = serde_json::from_str::<Json>(cursor)
            .ok()
            .and_then(|json| json.as_array().map(|a| a.iter().map(json_cell).collect()))
            .flatten();
        let Some(values) = values.filter(|v| v.len() == order.len()) else {
            return bad("the cursor is not from this select".to_owned());
        };
        let after = keyset(&order, &values);
        if after.is_empty() {
            empty = true;
        } else if filter.is_empty() {
            filter = after;
        } else {
            filter = filter
                .iter()
                .flat_map(|user| {
                    after.iter().map(move |keyed| {
                        let mut both = user.clone();
                        both.extend(keyed.iter().cloned());
                        both
                    })
                })
                .collect();
        }
    }
    Ok(Plan {
        columns,
        order,
        filter,
        empty,
        limit: select.limit as usize,
    })
}
