//! canoa.cfg in TOML, edited in place: one table is set or removed and every other byte of the file, its comments, its order and the
//! way the person spelled their keys, stays. A file in YAML or JSON is a document with no layout of its own to keep, so those
//! are rewritten whole by their callers.
//!
//! Every key is written quoted when it needs to be, so a name is whatever its person wrote.

use toml_edit::{Array, DocumentMut, InlineTable, Item, Table, TableLike, Value};

/// A canoa.cfg's text as an editable document.
///
/// # Errors
///
/// The text is not TOML.
pub(crate) fn parse(text: &str) -> Result<DocumentMut, String> {
    text.parse::<DocumentMut>()
        .map_err(|error| error.to_string())
}

/// Where tables made while editing go: after every table the file already has, so a new one never lands between two of the
/// person's.
struct Place {
    next: isize,
}

impl Place {
    fn after(document: &DocumentMut) -> Self {
        Self {
            next: last_position(document.as_table()).map_or(0, |last| last + 1),
        }
    }

    fn take(&mut self) -> isize {
        let at = self.next;
        self.next += 1;
        at
    }
}

fn last_position(table: &dyn TableLike) -> Option<isize> {
    table
        .iter()
        .filter_map(|(_, item)| match item {
            Item::Table(table) => [table.position(), last_position(table)]
                .into_iter()
                .flatten()
                .max(),
            Item::ArrayOfTables(tables) => tables
                .iter()
                .flat_map(|table| [table.position(), last_position(table)])
                .flatten()
                .max(),
            Item::Value(Value::InlineTable(table)) => last_position(table),
            _ => None,
        })
        .max()
}

/// The table at `path`, which is made (as tables with no header of their own) when `make` is set and absent otherwise.
fn walk<'a>(
    mut at: &'a mut dyn TableLike,
    path: &[&str],
    make: bool,
) -> Result<Option<&'a mut dyn TableLike>, String> {
    for key in path {
        if !at.contains_key(key) {
            if !make {
                return Ok(None);
            }
            let mut table = Table::new();
            table.set_implicit(true);
            at.insert(key, Item::Table(table));
        }
        at = at
            .get_mut(key)
            .and_then(Item::as_table_like_mut)
            .ok_or_else(|| format!("{key} is not a table"))?;
    }
    Ok(Some(at))
}

/// Makes the table at `path` hold `table`: a key it already holds with the same value keeps its text, a changed one is written in its
/// place, a missing one is added, and one `table` lacks is taken out. The table is made, with its parents, when the file has none.
///
/// # Errors
///
/// A name on `path` is a value in the file, not a table.
pub(crate) fn replace(
    document: &mut DocumentMut,
    path: &[&str],
    table: &toml::Table,
) -> Result<(), String> {
    set(document, path, table, true)
}

/// Sets the keys of `table` in the table at `path`, made when the file has none; its other keys stay.
///
/// # Errors
///
/// A name on `path` is a value in the file, not a table.
pub(crate) fn merge(
    document: &mut DocumentMut,
    path: &[&str],
    table: &toml::Table,
) -> Result<(), String> {
    set(document, path, table, false)
}

fn set(
    document: &mut DocumentMut,
    path: &[&str],
    table: &toml::Table,
    drop_others: bool,
) -> Result<(), String> {
    let mut place = Place::after(document);
    let (last, parents) = path.split_last().ok_or("no table is named")?;
    // The text of a file with no table yet is a comment the first table is made after, not below.
    let comment = document
        .as_table()
        .is_empty()
        .then(|| document.trailing().as_str().unwrap_or_default().to_owned())
        .filter(|text| !text.trim().is_empty());
    let at = walk(document.as_table_mut(), parents, true)?.ok_or("no table")?;
    if let Some(existing) = at.get_mut(last).and_then(Item::as_table_like_mut) {
        merge_into(existing, table, &mut place, drop_others);
    } else {
        // A value in the way of a table is replaced by it.
        at.insert(last, table_item(table, &mut place));
    }
    if let Some(mut comment) = comment {
        if !comment.ends_with('\n') {
            comment.push('\n');
        }
        comment.push('\n');
        let mut header = Vec::new();
        if first_header(document.as_table(), &mut header) {
            let keys: Vec<&str> = header.iter().map(String::as_str).collect();
            if let Some((first, parents)) = keys.split_last()
                && let Some(Some(at)) = walk(document.as_table_mut(), parents, false).ok()
                && let Some(Item::Table(table)) = at.get_mut(first)
            {
                table.decor_mut().set_prefix(comment);
                document.set_trailing("");
            }
        }
    }
    Ok(())
}

/// The path of the first table that shows a header of its own, in the order the file holds them.
fn first_header(at: &dyn TableLike, path: &mut Vec<String>) -> bool {
    for (key, item) in at.iter() {
        if let Item::Table(table) = item {
            path.push(key.to_owned());
            let shown =
                !table.is_implicit() || table.iter().any(|(_, inner)| !inner.is_table_like());
            if shown || first_header(table, path) {
                return true;
            }
            path.pop();
        }
    }
    false
}

/// Takes the table at `path` out, with the blank line before it, and says whether the file had one.
///
/// # Errors
///
/// A name on `path` is a value in the file, not a table.
pub(crate) fn remove(document: &mut DocumentMut, path: &[&str]) -> Result<bool, String> {
    let Some((last, parents)) = path.split_last() else {
        return Ok(false);
    };
    let Some(at) = walk(document.as_table_mut(), parents, false)? else {
        return Ok(false);
    };
    Ok(at.get(last).is_some_and(Item::is_table_like) && at.remove(last).is_some())
}

/// Takes `key` out of the table at `path`, and says whether it was there.
///
/// # Errors
///
/// A name on `path` is a value in the file, not a table.
pub(crate) fn remove_key(
    document: &mut DocumentMut,
    path: &[&str],
    key: &str,
) -> Result<bool, String> {
    let Some(at) = walk(document.as_table_mut(), path, false)? else {
        return Ok(false);
    };
    Ok(at.remove(key).is_some())
}

/// Whether the table at `path` holds a table of its own (`[path.x]`).
#[cfg(test)]
pub(crate) fn has_subtable(document: &DocumentMut, path: &[&str]) -> bool {
    let mut at: &dyn TableLike = document.as_table();
    for key in path {
        match at.get(key).and_then(Item::as_table_like) {
            Some(next) => at = next,
            None => return false,
        }
    }
    at.iter().any(|(_, item)| item.is_table_like())
}

fn merge_into(
    existing: &mut dyn TableLike,
    table: &toml::Table,
    place: &mut Place,
    drop_others: bool,
) {
    if drop_others {
        let gone: Vec<String> = existing
            .iter()
            .map(|(key, _)| key.to_owned())
            .filter(|key| !table.contains_key(key))
            .collect();
        for key in gone {
            existing.remove(&key);
        }
    }
    for (key, value) in table {
        match (existing.get_mut(key), value) {
            (Some(item), toml::Value::Table(inner)) if item.is_table_like() => {
                if let Some(inner_existing) = item.as_table_like_mut() {
                    merge_into(inner_existing, inner, place, drop_others);
                }
            }
            (Some(item), value) if same_item(item, value) => {}
            (Some(item), value) => {
                let mut next = item_of(value, place);
                // The key's own comment and spacing stay with the value that replaces it.
                if let (Item::Value(old), Item::Value(new)) = (&*item, &mut next) {
                    *new.decor_mut() = old.decor().clone();
                }
                *item = next;
            }
            (None, value) => {
                existing.insert(key, item_of(value, place));
            }
        }
    }
}

fn same_item(item: &Item, value: &toml::Value) -> bool {
    match (item, value) {
        (Item::Value(old), value) => same_value(old, value),
        (Item::Table(old), toml::Value::Table(new)) => same_table(old, new),
        (Item::ArrayOfTables(old), toml::Value::Array(new)) => {
            old.len() == new.len()
                && old
                    .iter()
                    .zip(new)
                    .all(|(old, new)| new.as_table().is_some_and(|new| same_table(old, new)))
        }
        _ => false,
    }
}

fn same_table(old: &dyn TableLike, new: &toml::Table) -> bool {
    old.len() == new.len()
        && old
            .iter()
            .all(|(key, item)| new.get(key).is_some_and(|value| same_item(item, value)))
}

fn same_value(old: &Value, new: &toml::Value) -> bool {
    match (old, new) {
        (Value::String(old), toml::Value::String(new)) => old.value() == new,
        (Value::Integer(old), toml::Value::Integer(new)) => old.value() == new,
        (Value::Float(old), toml::Value::Float(new)) => old.value().to_bits() == new.to_bits(),
        (Value::Boolean(old), toml::Value::Boolean(new)) => old.value() == new,
        (Value::Datetime(old), toml::Value::Datetime(new)) => {
            old.value().to_string() == new.to_string()
        }
        (Value::Array(old), toml::Value::Array(new)) => {
            old.len() == new.len() && old.iter().zip(new).all(|(old, new)| same_value(old, new))
        }
        (Value::InlineTable(old), toml::Value::Table(new)) => same_table(old, new),
        _ => false,
    }
}

fn item_of(value: &toml::Value, place: &mut Place) -> Item {
    match value {
        toml::Value::Table(table) => table_item(table, place),
        other => Item::Value(value_of(other)),
    }
}

/// A table with a header of its own, or none when it holds only tables, which then carry theirs.
fn table_item(table: &toml::Table, place: &mut Place) -> Item {
    let mut made = Table::new();
    if !table.is_empty() && table.values().all(toml::Value::is_table) {
        made.set_implicit(true);
    } else {
        made.set_position(Some(place.take()));
    }
    for (key, value) in table {
        made.insert(key, item_of(value, place));
    }
    Item::Table(made)
}

fn value_of(value: &toml::Value) -> Value {
    match value {
        toml::Value::String(text) => Value::from(text.as_str()),
        toml::Value::Integer(number) => Value::from(*number),
        toml::Value::Float(number) => Value::from(*number),
        toml::Value::Boolean(flag) => Value::from(*flag),
        toml::Value::Datetime(date) => date
            .to_string()
            .parse::<toml_edit::Datetime>()
            .map_or_else(|_| Value::from(date.to_string()), Value::from),
        toml::Value::Array(items) => Value::Array(items.iter().map(value_of).collect::<Array>()),
        toml::Value::Table(table) => {
            let mut inline = InlineTable::new();
            for (key, value) in table {
                inline.insert(key, value_of(value));
            }
            Value::InlineTable(inline)
        }
    }
}

#[cfg(test)]
mod tests;
