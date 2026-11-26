//! The catalogs each Store type reads, in one shape: `[store.<type>.catalogs.<label>]` in canoa.cfg, and what the Store types
//! say of their catalogs (the words for them, and the one offered until the person removes it).
//!
//! A label's value is a source the `read` provider reads (a bare string, written `{ provider = "read", source = "..." }` in full),
//! a table naming a provider and the fields its form asks for, or `false`, which turns a default catalog off. The host keeps four
//! keys of a table for itself, which no provider's form may use: `pin`, `ttl`, `keys` and `endpoints`. One reader serves every type,
//! so a catalog written under one type is never listed under another.
//!
//! A type's catalogs are its defaults (what its extension registers, in the order registered), then canoa.cfg's entries by label:
//! a label a default has replaces it whole, a new label is added after the defaults, and `false` turns the default of that label off
//! (it stays listed, off, so it can be turned on again).

#[cfg(test)]
use alloc::collections::BTreeMap;

use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
#[cfg(test)]
use coqui_acp_api::guatiao::{Value as GuatiaoValue, ValueError};
use coqui_aos::valid_overlay_name;
use serde_json::{Map, Value, json};

/// Guatiao-derived schema for one Store type subsection under `[store.<type>]`.
#[cfg(test)]
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
pub(crate) struct CatalogTableSchema {
    /// Catalog rows keyed by catalog label.
    #[map(default)]
    pub(crate) catalogs: BTreeMap<String, GuatiaoValue>,
}

/// Guatiao-derived fields of a catalog entry table (`[store.<type>.catalogs.<label>]`).
#[derive(ToValue, FromValue, Schema, Debug, Clone, Default, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
struct CatalogEntryHeader {
    provider: Option<String>,
}

/// The guatiao schema describing `[store.<type>]` catalog sections in `canoa.cfg`.
#[cfg(test)]
pub(crate) fn schema() -> Result<GuatiaoValue, ValueError> {
    <CatalogTableSchema as Schema>::schema()
}

/// The provider that reads an address or a file the way the Store type reads one: the default when a label names none.
pub(crate) const READ: &str = "read";

/// The keys of a catalog's table that the host owns.
const HOST_KEYS: [&str; 4] = ["pin", "ttl", "keys", "endpoints"];

/// Where a catalog of a type comes from.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Origin {
    /// The type's own: what its extension registers (or the host, for the types it carries).
    Default,
    /// A default of the type that canoa.cfg's entry of the same label replaced.
    Replaced,
    /// An entry canoa.cfg adds.
    Added,
}

impl Origin {
    pub(crate) const fn word(self) -> &'static str {
        match self {
            Self::Default => "default",
            Self::Replaced => "replaced",
            Self::Added => "added",
        }
    }
}

/// One catalog of a type, as canoa.cfg has it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Entry {
    pub(crate) label: String,
    /// The `catalog_source` provider that reads it.
    pub(crate) provider: String,
    /// What the provider's form holds: the table's keys but `provider` and the host's own.
    pub(crate) fields: Map<String, Value>,
    /// What fixes its answer: a revision the provider understands, or `sha256:<hex>` of the catalog's text.
    pub(crate) pin: Option<String>,
    /// Seconds before it is read again, in place of `[host.store] source_ttl`.
    pub(crate) ttl: Option<u64>,
    /// The publisher key fingerprints it demands a signature from, in place of its provider's.
    pub(crate) keys: Vec<String>,
    /// Further `https` origins or prefixes its files may be downloaded from, beyond the ones its type names: a person who wrote
    /// one has said where the files are, and what is fetched is checked by its digest.
    pub(crate) endpoints: Vec<String>,
    pub(crate) origin: Origin,
    /// False for a catalog turned off with `label = false`.
    pub(crate) enabled: bool,
    /// What a pinned default answers with before it has been fetched.
    pub(crate) snapshot: Option<String>,
}

impl Entry {
    /// The `source` field, for the `read` provider.
    pub(crate) fn source(&self) -> Option<&str> {
        self.fields.get("source").and_then(Value::as_str)
    }

    /// One text that differs whenever the provider, a field or a host key does, so an index can tell that canoa.cfg now says another
    /// thing.
    pub(crate) fn spec(&self) -> String {
        if self.provider == READ
            && self.pin.is_none()
            && self.keys.is_empty()
            && self.endpoints.is_empty()
            && let Some(source) = self.source().filter(|_| self.fields.len() == 1)
        {
            return source.to_owned();
        }
        let sorted: alloc::collections::BTreeMap<_, _> = self.fields.iter().collect();
        format!(
            "{}:{}:{}:{}:{}",
            self.provider,
            serde_json::to_string(&sorted).unwrap_or_default(),
            self.pin.as_deref().unwrap_or_default(),
            self.keys.join(","),
            self.endpoints.join(",")
        )
    }

    /// An entry that reads `source` with the `read` provider.
    fn reading(label: &str, source: &str, origin: Origin) -> Self {
        Self::of(
            label,
            READ,
            Map::from_iter([("source".to_owned(), json!(source))]),
            origin,
        )
    }

    fn of(label: &str, provider: &str, fields: Map<String, Value>, origin: Origin) -> Self {
        Self {
            label: label.to_owned(),
            provider: provider.to_owned(),
            fields,
            pin: None,
            ttl: None,
            keys: Vec::new(),
            endpoints: Vec::new(),
            origin,
            enabled: true,
            snapshot: None,
        }
    }

    /// The catalog a type's default says.
    fn from_default(default: &Default) -> Self {
        let mut entry = Self::of(
            &default.label,
            &default.provider,
            default.fields.clone(),
            Origin::Default,
        );
        entry.pin.clone_from(&default.pin);
        entry.snapshot.clone_from(&default.snapshot);
        entry
    }

    /// What the badge a Sources page draws says of the catalog: `pinned` when its answer is fixed to a revision, else nothing.
    pub(crate) const fn trust(&self) -> Option<&'static str> {
        if self.pin.is_some() {
            Some("pinned")
        } else {
            None
        }
    }
}

/// A catalog a type offers of its own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Default {
    pub(crate) label: String,
    pub(crate) provider: String,
    pub(crate) fields: Map<String, Value>,
    pub(crate) pin: Option<String>,
    pub(crate) snapshot: Option<String>,
}

impl Default {
    /// A default read by the `read` provider from `source`.
    fn reading(label: &str, source: &str) -> Self {
        Self {
            label: label.to_owned(),
            provider: READ.to_owned(),
            fields: Map::from_iter([("source".to_owned(), json!(source))]),
            pin: None,
            snapshot: None,
        }
    }

    /// What an extension registered, or why it is not offered.
    pub(crate) fn registered(
        each: &canoa::store::extension::DefaultCatalog,
    ) -> Result<Self, String> {
        if !valid_overlay_name(&each.label) {
            return Err(format!(
                "the catalog label {:?} is not a name this host keeps a catalog under",
                each.label
            ));
        }
        let fields = match each.fields.as_deref() {
            None => Map::new(),
            Some(text) => serde_json::from_str::<Map<String, Value>>(text)
                .map_err(|_| format!("the fields of {:?} are not a JSON object", each.label))?,
        };
        Ok(Self {
            label: each.label.clone(),
            provider: each.provider.clone(),
            fields,
            pin: each.pin.clone(),
            snapshot: each.snapshot.clone(),
        })
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
    read_all(table)
        .into_iter()
        .filter(|parsed| parsed.as_ref().map_or(true, |entry| entry.enabled))
        .collect()
}

/// As [`read`], with a label set to `false` listed as an entry that is off (the `read` provider, no fields).
pub(crate) fn read_all(table: &Value) -> Vec<Parsed> {
    let Some(table) = table.as_object() else {
        return Vec::new();
    };
    let mut seen = alloc::collections::BTreeSet::new();
    table
        .iter()
        .map(|(label, value)| {
            let problem = |why: &str| (label.clone(), why.to_owned());
            if !valid_overlay_name(label) || !seen.insert(coqui_aos::normalize_name(label)) {
                return Err(problem(
                    "The label is not a name this host keeps a catalog under",
                ));
            }
            match value {
                Value::Bool(false) => {
                    let mut off = Entry::of(label, READ, Map::new(), Origin::Added);
                    off.enabled = false;
                    Ok(off)
                }
                Value::String(source) => Ok(Entry::reading(label, source, Origin::Added)),
                Value::Object(table) => {
                    let gval = match guatiao_serde::from_serde(Value::Object(table.clone())) {
                        Ok(value) => value,
                        Err(_) => {
                            return Err(problem("provider is the id of a source provider"));
                        }
                    };
                    let header = match CatalogEntryHeader::from_value(&gval) {
                        Ok(header) => header,
                        Err(_) => {
                            return Err(problem("provider is the id of a source provider"));
                        }
                    };
                    let provider = match header.provider {
                        None => READ.to_owned(),
                        Some(id) if !id.is_empty() => id,
                        Some(_) => return Err(problem("provider is the id of a source provider")),
                    };
                    let fields = table
                        .iter()
                        .filter(|(key, _)| *key != "provider" && !HOST_KEYS.contains(&key.as_str()))
                        .map(|(key, field)| (key.clone(), field.clone()))
                        .collect();
                    let mut entry = Entry::of(label, &provider, fields, Origin::Added);
                    entry.pin = match table.get("pin") {
                        None | Some(Value::Null) => None,
                        Some(Value::String(pin)) if !pin.trim().is_empty() => Some(pin.clone()),
                        Some(_) => {
                            return Err(problem("pin is a revision, or sha256: and a digest"));
                        }
                    };
                    entry.ttl = match table.get("ttl") {
                        None | Some(Value::Null) => None,
                        Some(seconds) => Some(
                            seconds
                                .as_u64()
                                .ok_or_else(|| problem("ttl is a number of seconds"))?,
                        ),
                    };
                    entry.keys = match table.get("keys") {
                        None | Some(Value::Null) => Vec::new(),
                        Some(Value::Array(keys)) => keys
                            .iter()
                            .map(|key| key.as_str().map(str::to_owned))
                            .collect::<Option<Vec<_>>>()
                            .ok_or_else(|| problem("keys is a list of key fingerprints"))?,
                        Some(_) => return Err(problem("keys is a list of key fingerprints")),
                    };
                    entry.endpoints = match table.get("endpoints") {
                        None | Some(Value::Null) => Vec::new(),
                        Some(Value::Array(places)) => places
                            .iter()
                            .map(|place| place.as_str().map(str::to_owned))
                            .collect::<Option<Vec<_>>>()
                            .filter(|places| {
                                places
                                    .iter()
                                    .all(|place| canoa::store::source::is_endpoint(place))
                            })
                            .ok_or_else(|| {
                                problem("endpoints is a list of https origins or prefixes")
                            })?,
                        Some(_) => {
                            return Err(problem(
                                "endpoints is a list of https origins or prefixes",
                            ));
                        }
                    };
                    Ok(entry)
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
    /// The catalogs offered until the person replaces one by its label, adds another or turns one off, in the order registered.
    pub(crate) defaults: Vec<Default>,
    /// A catalog the type always offers first, which no canoa.cfg entry removes or replaces: its label is the type's own.
    pub(crate) fixed: Option<(&'static str, &'static str)>,
    /// The format its catalogs answer in (`canoa.models/1`, `canoa.releases/1`); none for a type the host reads itself.
    pub(crate) format: Option<String>,
}

/// The types the Store has of its own that read catalogs.
pub(crate) fn built_in_types() -> Vec<TypeCatalogs> {
    let words = |id: &str,
                 title: &str,
                 noun: &str,
                 plural: &str,
                 lists: &str,
                 default: Option<(&str, &str)>| TypeCatalogs {
        id: id.to_owned(),
        title: title.to_owned(),
        noun: noun.to_owned(),
        plural: plural.to_owned(),
        lists: lists.to_owned(),
        defaults: default
            .map(|(label, source)| Default::reading(label, source))
            .into_iter()
            .collect(),
        fixed: None,
        format: None,
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
            Some((DEFAULT_SKILL_LABEL, DEFAULT_SKILL_SOURCE)),
        ),
        words(
            "mcps",
            "MCP servers",
            "registry",
            "registries",
            "MCP servers",
            Some((OFFICIAL_LABEL, crate::control::registry::OFFICIAL)),
        ),
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

/// The label and source of the skills source Canoa offers first.
pub(crate) const DEFAULT_SKILL_LABEL: &str = "anthropics";
pub(crate) const DEFAULT_SKILL_SOURCE: &str = "https://github.com/anthropics/skills";

/// Whether `(label, spec)` names the default skills source offered by Canoa itself.
pub(crate) fn is_default_skill_source(label: &str, spec: &str) -> bool {
    let short = DEFAULT_SKILL_SOURCE
        .strip_prefix("https://github.com/")
        .unwrap_or(DEFAULT_SKILL_SOURCE);
    same_label(label, DEFAULT_SKILL_LABEL) && (spec == DEFAULT_SKILL_SOURCE || spec == short)
}

/// The label of the official MCP registry, which the MCP servers type offers until removed or replaced.
pub(crate) const OFFICIAL_LABEL: &str = "official";

pub(crate) fn same_label(a: &str, b: &str) -> bool {
    coqui_aos::normalize_name(a) == coqui_aos::normalize_name(b)
}

/// The catalogs of `kind` in force: the fixed one, then the type's defaults and canoa.cfg's entries merged by label. A default whose
/// label an entry names is replaced whole by it (or kept, off, when the entry is `false`); a new label is added after the defaults,
/// in the file's order; an entry that is no catalog is listed with the reason, in the place of the default it names.
pub(crate) fn merge(kind: &TypeCatalogs, table: &Value) -> Vec<Merged> {
    let mut merged: Vec<Merged> = Vec::new();
    if let Some((label, source)) = kind.fixed {
        merged.push(Merged {
            parsed: Ok(Entry::reading(label, source, Origin::Default)),
            fixed: true,
        });
    }
    for default in &kind.defaults {
        merged.push(Merged {
            parsed: Ok(Entry::from_default(default)),
            fixed: false,
        });
    }
    for parsed in read_all(table) {
        let label = match &parsed {
            Ok(entry) => entry.label.clone(),
            Err((label, _)) => label.clone(),
        };
        let at = merged.iter().position(|each| {
            each.parsed.as_ref().map_or_else(
                |(own, _)| same_label(own, &label),
                |entry| same_label(&entry.label, &label),
            )
        });
        match (at, parsed) {
            (Some(at), _) if merged[at].fixed => merged.push(Merged {
                parsed: Err((
                    label,
                    "The label is the host's own: the official registry is always offered and cannot be replaced"
                        .to_owned(),
                )),
                fixed: false,
            }),
            (Some(at), Ok(entry)) if !entry.enabled => {
                if let Ok(kept) = &mut merged[at].parsed {
                    kept.enabled = false;
                }
            }
            (Some(at), Ok(mut entry)) => {
                entry.origin = Origin::Replaced;
                merged[at].parsed = Ok(entry);
            }
            (Some(at), Err(problem)) => merged[at].parsed = Err(problem),
            // An entry that is `false` and names no default turns nothing off: there is no row for it.
            (None, Ok(entry)) if !entry.enabled => {}
            (None, parsed) => merged.push(Merged {
                parsed,
                fixed: false,
            }),
        }
    }
    merged
}

/// One catalog of a type after the merge.
#[derive(Clone, Debug)]
pub(crate) struct Merged {
    pub(crate) parsed: Parsed,
    /// Whether it is the type's fixed catalog.
    pub(crate) fixed: bool,
}

/// The catalog `label` of `kind`: the fixed one, one the merge of the type's defaults and canoa.cfg's `table` holds. The error says
/// why there is none, or that it is off.
pub(crate) fn find(kind: &TypeCatalogs, table: &Value, label: &str) -> Result<Entry, String> {
    for each in merge(kind, table) {
        match each.parsed {
            Ok(entry) if same_label(&entry.label, label) => {
                return if entry.enabled {
                    Ok(entry)
                } else {
                    Err(format!(
                        "{} is turned off ({} = false in canoa.cfg)",
                        entry.label, entry.label
                    ))
                };
            }
            Err((each, why)) if same_label(&each, label) => return Err(why),
            _ => {}
        }
    }
    Err(format!("{} has no catalog {label:?}", kind.title))
}

/// What the Store says of one catalog's entry: its provider and fields as canoa.cfg has them, where it comes from, whether it is on,
/// and the keys the host keeps for it.
pub(crate) fn row_of(kind: &TypeCatalogs, merged: &Merged) -> Value {
    let entry = match &merged.parsed {
        Ok(entry) => entry,
        Err((label, why)) => {
            return json!({"label": label, "problem": why, "default": false, "enabled": true});
        }
    };
    let mut row = json!({
        "label": entry.label,
        "provider": entry.provider,
        "fields": entry.fields,
        "default": entry.origin == Origin::Default,
        "origin": entry.origin.word(),
        "enabled": entry.enabled,
    });
    if let Some(source) = entry.source() {
        row["source"] = json!(source);
    }
    if merged.fixed {
        row["fixed"] = json!(true);
    }
    if let Some(pin) = &entry.pin {
        row["pin"] = json!(pin);
    }
    if let Some(ttl) = entry.ttl {
        row["ttl"] = json!(ttl);
    }
    if !entry.keys.is_empty() {
        row["keys"] = json!(entry.keys);
    }
    if !entry.endpoints.is_empty() {
        row["endpoints"] = json!(entry.endpoints);
    }
    if kind.format.is_some() {
        row["trust"] = json!(entry.trust());
        row["snapshot"] = json!(entry.snapshot.is_some());
    }
    row
}

/// One type's rows for the Store: the fixed catalog, then the defaults and canoa.cfg's own, each with the provider and fields as
/// canoa.cfg has them. A default turned off with `false` is listed, off; a row that is no catalog says why.
pub(crate) fn rows_of(kind: &TypeCatalogs, table: &Value) -> Vec<Value> {
    merge(kind, table)
        .iter()
        .map(|merged| row_of(kind, merged))
        .collect()
}

#[cfg(test)]
#[path = "catalogs_tests.rs"]
mod tests;
