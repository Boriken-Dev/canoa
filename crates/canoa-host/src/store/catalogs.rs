//! The catalogs each Store type reads, in one shape: `[store.<type>.catalogs.<label>]` in canoa.cfg, and what the Store types
//! say of their catalogs (the words for them, and the one offered until the person removes it).
//!
//! A label's value is a source the `read` provider reads (a bare string, written `{ provider = "read", source = "..." }` in full),
//! a table naming a provider and the fields its form asks for, or `false`, which takes a default catalog away. One reader serves
//! every type, so a catalog written under one type is never listed under another.

use serde_json::{Map, Value, json};

use coqui_aos::valid_overlay_name;

/// The provider that reads an address or a file the way the Store type reads one: the default when a label names none.
pub(crate) const READ: &str = "read";

/// One catalog of a type, as canoa.cfg has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) label: String,
    /// The `catalog_source` provider that reads it.
    pub(crate) provider: String,
    /// What the provider's form holds: the table's keys but `provider`.
    pub(crate) fields: Map<String, Value>,
}

impl Entry {
    /// The `source` field, for the `read` provider.
    pub(crate) fn source(&self) -> Option<&str> {
        self.fields.get("source").and_then(Value::as_str)
    }

    /// One text that differs whenever the provider or a field does, so an index can tell that canoa.cfg now says another thing.
    pub(crate) fn spec(&self) -> String {
        if self.provider == READ
            && let Some(source) = self.source().filter(|_| self.fields.len() == 1)
        {
            return source.to_owned();
        }
        let sorted: alloc::collections::BTreeMap<_, _> = self.fields.iter().collect();
        format!(
            "{}:{}",
            self.provider,
            serde_json::to_string(&sorted).unwrap_or_default()
        )
    }
}

/// A catalog, or its label and why canoa.cfg's entry is none.
pub(crate) type Parsed = Result<Entry, (String, String)>;

/// `<type>`'s table of catalogs in the `[store]` table `store`, or null.
pub(crate) fn table_of<'a>(store: &'a Value, kind: &str) -> &'a Value {
    store
        .get(kind)
        .and_then(|each| each.get("catalogs"))
        .unwrap_or(&Value::Null)
}

/// Reads one type's table of catalogs, in the order of the labels. A label set to `false` lists nothing.
pub(crate) fn read(table: &Value) -> Vec<Parsed> {
    let Some(table) = table.as_object() else {
        return Vec::new();
    };
    let mut seen = alloc::collections::BTreeSet::new();
    table
        .iter()
        .filter(|(_, value)| **value != Value::Bool(false))
        .map(|(label, value)| {
            let problem = |why: &str| (label.clone(), why.to_owned());
            if !valid_overlay_name(label) || !seen.insert(coqui_aos::normalize_name(label)) {
                return Err(problem(
                    "The label is not a name this host keeps a catalog under",
                ));
            }
            match value {
                Value::String(source) => Ok(Entry {
                    label: label.clone(),
                    provider: READ.to_owned(),
                    fields: Map::from_iter([("source".to_owned(), json!(source))]),
                }),
                Value::Object(table) => {
                    let provider = match table.get("provider") {
                        None => READ.to_owned(),
                        Some(Value::String(id)) if !id.is_empty() => id.clone(),
                        Some(_) => return Err(problem("provider is the id of a source provider")),
                    };
                    let fields = table
                        .iter()
                        .filter(|(key, _)| *key != "provider")
                        .map(|(key, field)| (key.clone(), field.clone()))
                        .collect();
                    Ok(Entry {
                        label: label.clone(),
                        provider,
                        fields,
                    })
                }
                _ => Err(problem(
                    "It is a source, or a table naming a provider and its fields, or false",
                )),
            }
        })
        .collect()
}

/// Whether `wanted`, a `[store]` table to write, names a catalog of a provider other than `read` that `current`, the `[store]`
/// table now, does not hold the same.
pub(crate) fn names_new_source(current: &Value, wanted: &Value) -> bool {
    let Some(kinds) = wanted.as_object() else {
        return false;
    };
    kinds.iter().any(|(kind, each)| {
        each.get("catalogs")
            .and_then(Value::as_object)
            .is_some_and(|catalogs| {
                catalogs.iter().any(|(label, entry)| {
                    let other = entry
                        .get("provider")
                        .and_then(Value::as_str)
                        .is_some_and(|provider| provider != READ);
                    other && table_of(current, kind).get(label) != Some(entry)
                })
            })
    })
}

/// What the Store says of one type's catalogs.
#[derive(Clone, Debug)]
pub(crate) struct TypeCatalogs {
    pub(crate) id: String,
    pub(crate) title: String,
    /// What one is called, and what it lists, in the type's words.
    pub(crate) noun: String,
    pub(crate) plural: String,
    pub(crate) lists: String,
    /// The catalog offered until the person removes it or gives its label another source.
    pub(crate) default: Option<(&'static str, &'static str)>,
    /// A catalog the type always offers first, which no canoa.cfg entry removes or replaces: its label is the type's own.
    pub(crate) fixed: Option<(&'static str, &'static str)>,
}

/// The types the Store has of its own that read catalogs.
pub(crate) fn built_in_types() -> Vec<TypeCatalogs> {
    let words =
        |id: &str, title: &str, noun: &str, plural: &str, lists: &str, default| TypeCatalogs {
            id: id.to_owned(),
            title: title.to_owned(),
            noun: noun.to_owned(),
            plural: plural.to_owned(),
            lists: lists.to_owned(),
            default,
            fixed: None,
        };
    vec![
        words(
            "overlays",
            "Overlays",
            "catalog",
            "catalogs",
            "overlays",
            Some((
                super::index::DEFAULT_CATALOG_LABEL,
                super::index::DEFAULT_CATALOG_SPEC,
            )),
        ),
        words(
            "skills",
            "Skills",
            "source",
            "sources",
            "skills",
            Some(("anthropics", "https://github.com/anthropics/skills")),
        ),
        TypeCatalogs {
            // The official registry is always offered first and cannot be taken away or replaced.
            fixed: Some((OFFICIAL_LABEL, crate::control::registry::OFFICIAL)),
            ..words(
                "mcps",
                "MCP servers",
                "registry",
                "registries",
                "MCP servers",
                None,
            )
        },
        words(
            "extensions",
            "Extensions",
            "catalog",
            "catalogs",
            "extensions",
            None,
        ),
    ]
}

/// The label of the official MCP registry, which the MCP servers type always offers.
pub(crate) const OFFICIAL_LABEL: &str = "official";

fn same_label(a: &str, b: &str) -> bool {
    coqui_aos::normalize_name(a) == coqui_aos::normalize_name(b)
}

/// The catalog `label` of `kind`: the fixed one, one canoa.cfg's `table` names, or the default (while the table does not name or
/// remove its label). The error says why there is none.
pub(crate) fn find(kind: &TypeCatalogs, table: &Value, label: &str) -> Result<Entry, String> {
    let read = |label: &str, source: &str| Entry {
        label: label.to_owned(),
        provider: READ.to_owned(),
        fields: Map::from_iter([("source".to_owned(), json!(source))]),
    };
    if let Some((fixed, source)) = kind.fixed
        && same_label(fixed, label)
    {
        return Ok(read(fixed, source));
    }
    let named = table
        .as_object()
        .is_some_and(|table| table.keys().any(|key| same_label(key, label)));
    if !named
        && let Some((default, source)) = kind.default
        && same_label(default, label)
    {
        return Ok(read(default, source));
    }
    for parsed in self::read(table) {
        match parsed {
            Ok(entry) if same_label(&entry.label, label) => return Ok(entry),
            Err((each, why)) if same_label(&each, label) => return Err(why),
            _ => {}
        }
    }
    Err(format!("{} has no catalog {label:?}", kind.title))
}

/// One type's rows for the Store: the fixed catalog, the default (unless the table names its label or sets it to `false`) and
/// then the table's own, each with the provider and fields as canoa.cfg has them. A row that is no catalog says why.
pub(crate) fn rows_of(kind: &TypeCatalogs, table: &Value) -> Vec<Value> {
    let named = |label: &str| {
        table.as_object().is_some_and(|table| {
            table
                .keys()
                .any(|key| coqui_aos::normalize_name(key) == coqui_aos::normalize_name(label))
        })
    };
    let mut rows = Vec::new();
    if let Some((label, source)) = kind.fixed {
        rows.push(json!({
            "label": label,
            "provider": READ,
            "fields": {"source": source},
            "source": source,
            "default": true,
            "fixed": true,
        }));
    }
    if let Some((label, source)) = kind.default
        && !named(label)
    {
        rows.push(json!({
            "label": label,
            "provider": READ,
            "fields": {"source": source},
            "source": source,
            "default": true,
        }));
    }
    for parsed in read(table) {
        let taken = |label: &str| {
            kind.fixed
                .is_some_and(|(fixed, _)| same_label(fixed, label))
        };
        rows.push(match parsed {
            Ok(entry) if taken(&entry.label) => json!({
                "label": entry.label,
                "problem": "The label is the host's own: the official registry is always offered and cannot be replaced",
                "default": false,
            }),
            Ok(entry) => {
                let mut row = json!({
                    "label": entry.label,
                    "provider": entry.provider,
                    "fields": entry.fields,
                    "default": false,
                });
                if let Some(source) = entry.source() {
                    row["source"] = json!(source);
                }
                row
            }
            Err((label, why)) => json!({"label": label, "problem": why, "default": false}),
        });
    }
    rows
}

#[cfg(test)]
#[path = "catalogs_tests.rs"]
mod tests;
