//! The rows of a Store type's Models page that its catalogs list: for a type whose catalogs answer in `canoa.models/1`, each enabled
//! catalog (the type's defaults and canoa.cfg's entries, merged by label) is read through its source and its items become rows beside
//! the ones the type's `model_store` lists.
//!
//! A row says which catalog it came from, in a fact and in the `catalog` object `store/models` adds to it, and how far its digests
//! can be believed. An item whose files carry no digest is listed with the reason and cannot be installed unless the entry says
//! `trusted = true`; the files of an item are downloaded checked by the digest the catalog gave, and an unverified one only from such
//! an entry. An id two catalogs list is shown once for each, the later ones as `<id>@<label>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use canoa::store::Check;
use canoa::store::catalog::{FileCheck, MODELS_FORMAT, ModelItem, Trust, parse_models};
use canoa::store::models::{
    Fact, ModelFile, ModelRow, RowAction, STEP_DELETE_FILES, STEP_FETCH_FILES,
};
use serde_json::{Value, json};

use super::Stores;
use crate::control::files::Files;
use crate::store::catalog_run::Reading;
use crate::store::catalog_view::catalog_info;
use crate::store::catalogs::{Entry, merge, table_of};

/// What a type's catalogs add to its Models page.
#[derive(Default)]
pub(super) struct FromCatalogs {
    /// The rows, one for each item of an enabled catalog.
    pub(super) rows: Vec<ModelRow>,
    /// For each row, the catalog it came from (`store/models` adds it to the row).
    pub(super) meta: BTreeMap<String, Value>,
    /// For each row, how each of its files is checked when it is downloaded.
    pub(super) checks: BTreeMap<String, Vec<Check>>,
    /// For each row that cannot be installed, why.
    pub(super) refused: BTreeMap<String, String>,
    /// Each catalog of the type, whether it could be read or why not.
    pub(super) sources: Vec<Value>,
}

fn check_of(file: &canoa::store::catalog::CatalogFile) -> Option<Check> {
    match file.check.as_ref() {
        Some(FileCheck::Sha256(value)) => Some(Check::Sha256 {
            value: value.clone(),
        }),
        Some(FileCheck::Sha512(value)) => Some(Check::Sha512 {
            value: value.clone(),
        }),
        Some(FileCheck::Unverified(why)) => Some(Check::Unverified { why: why.clone() }),
        None => Some(Check::Unverified {
            why: "the catalog gives no checksum".to_owned(),
        }),
        // A git blob id says what a file holds only through its own hash, which the host's downloads do not compute.
        Some(_) => None,
    }
}

/// Whether every file of `item` is in `folder`.
fn installed(item: &ModelItem, folder: Option<&str>) -> bool {
    folder.is_some_and(|folder| {
        !item.files.is_empty()
            && item
                .files
                .iter()
                .all(|file| Path::new(folder).join(&file.file).is_file())
    })
}

fn row_of(
    id: &str,
    item: &ModelItem,
    entry: &Entry,
    trust: &Result<Trust, String>,
    checks: Option<&[Check]>,
    done: bool,
) -> ModelRow {
    let mut row = ModelRow::new(id, item.label.clone());
    row.by.clone_from(&item.by);
    row.tags.clone_from(&item.tags);
    row.facts.push(Fact::new(
        "Catalog",
        format!("{} ({})", entry.label, entry.provider),
    ));
    if let Some(license) = &item.license {
        row.facts.push(Fact::new("License", license.clone()));
    }
    row.facts.extend(
        item.facts
            .iter()
            .map(|(label, value)| Fact::new(label, value)),
    );
    row.files = item
        .files
        .iter()
        .map(|file| {
            let sha256 = match &file.check {
                Some(FileCheck::Sha256(value)) => value.clone(),
                _ => String::new(),
            };
            ModelFile::new(&file.url, &file.file, file.bytes, sha256)
        })
        .collect();
    match (trust, checks) {
        (Ok(trust), Some(_)) => {
            row.facts.push(Fact::new("Trust", trust.word()));
            row.on = done;
            row.state = if done {
                Some("installed".to_owned())
            } else if *trust == Trust::Unverified {
                Some("unverified".to_owned())
            } else {
                None
            };
            row.actions = vec![if done {
                RowAction::new(
                    format!("Remove {}", item.label),
                    "The files of this model are deleted from this computer.",
                    "Remove",
                    [STEP_DELETE_FILES],
                )
            } else {
                RowAction::new(
                    format!("Install {}", item.label),
                    if *trust == Trust::Unverified {
                        format!(
                            "{} publishes no checksum for these files, so nothing can say they are the ones meant. This catalog's entry says trusted = true.",
                            entry.label
                        )
                    } else {
                        format!(
                            "The files are downloaded and checked against the digest {} gives.",
                            entry.label
                        )
                    },
                    "Install",
                    [STEP_FETCH_FILES],
                )
            }];
        }
        (Err(why), _) => {
            row.state = Some("no checksum".to_owned());
            row.facts.push(Fact::new("Not installable", why.clone()));
        }
        (Ok(_), None) => {
            row.state = Some("not installable".to_owned());
            row.facts.push(Fact::new(
                "Not installable",
                "its files are checked by git blob id, which this host does not verify yet",
            ));
        }
    }
    row
}

impl Stores {
    /// The rows the catalogs of `type_id` list, when its catalogs answer in `canoa.models/1`. `taken` are the ids the type's
    /// `model_store` already lists, which keep their rows.
    pub(super) fn catalog_models(
        &self,
        type_id: &str,
        files: &Files,
        taken: &BTreeSet<String>,
    ) -> FromCatalogs {
        let mut made = FromCatalogs::default();
        let Some(scope) = self
            .catalog_types()
            .into_iter()
            .find(|scope| scope.id == type_id && scope.format.as_deref() == Some(MODELS_FORMAT))
        else {
            return made;
        };
        let store = files
            .config_get(&json!({"section": "store"}))
            .ok()
            .and_then(|answer| answer.get("value").cloned())
            .unwrap_or(Value::Null);
        let run = self.catalog_run();
        let folder = self.models_folder(type_id);
        let mut used: BTreeSet<String> = taken.clone();
        for merged in merge(&scope, table_of(&store, type_id)) {
            let entry = match merged.parsed {
                Ok(entry) => entry,
                Err((label, why)) => {
                    made.sources
                        .push(json!({"label": label, "problem": why, "enabled": true}));
                    continue;
                }
            };
            let mut status = json!({
                "label": entry.label,
                "provider": entry.provider,
                "origin": entry.origin.word(),
                "enabled": entry.enabled,
                "trust": entry.trust(),
                "pin": entry.pin,
            });
            if !entry.enabled {
                made.sources.push(status);
                continue;
            }
            let reading: Reading = match run.read(&scope, &entry, false) {
                Ok(reading) => reading,
                Err(why) => {
                    status["problem"] = json!(why);
                    made.sources.push(status);
                    continue;
                }
            };
            let catalog = match parse_models(&reading.text) {
                Ok(catalog) => catalog,
                Err(why) => {
                    status["problem"] = json!(why);
                    made.sources.push(status);
                    continue;
                }
            };
            let info = catalog_info(&entry, catalog.revision.as_deref(), &reading);
            status["fetched"] = json!(reading.fetched);
            status["from"] = json!(reading.from);
            status["stale"] = json!(reading.stale);
            status["problem"] = json!(reading.problem);
            status["items"] = json!(catalog.items.len());
            made.sources.push(status);
            for item in &catalog.items {
                let id = if used.contains(&item.id) {
                    format!("{}@{}", item.id, entry.label)
                } else {
                    item.id.clone()
                };
                used.insert(id.clone());
                let trust = item.trust(entry.trusted);
                let checks: Option<Vec<Check>> = item.files.iter().map(check_of).collect();
                let done = installed(item, folder.as_deref());
                let shown = trust
                    .as_ref()
                    .map_or(Value::Null, |trust| json!(trust.word()));
                let row = row_of(&id, item, &entry, &trust, checks.as_deref(), done);
                match (&trust, checks) {
                    (Ok(_), Some(checks)) => {
                        made.checks.insert(id.clone(), checks);
                    }
                    (Err(why), _) => {
                        made.refused.insert(id.clone(), why.clone());
                    }
                    (Ok(_), None) => {
                        made.refused.insert(
                            id.clone(),
                            "its files are checked by git blob id, which this host does not verify yet"
                                .to_owned(),
                        );
                    }
                }
                made.meta
                    .insert(id, json!({"catalog": info.clone(), "trust": shown}));
                made.rows.push(row);
            }
        }
        made
    }
}

#[cfg(test)]
#[path = "models_catalog_tests.rs"]
mod tests;
