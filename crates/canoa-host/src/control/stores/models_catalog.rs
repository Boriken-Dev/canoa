//! The rows of a Store type's Models page that its catalogs list: for a type whose catalogs answer in `canoa.models/1`, each enabled
//! catalog (the type's defaults and canoa.cfg's entries, merged by label) is read through its source and its items become rows beside
//! the ones the type's `model_store` lists.
//!
//! A row says which catalog it came from, in a fact and in the `catalog` object `store/models` adds to it, and which algorithm its
//! files' digests are made with (`verified`). The files of an item are downloaded checked by the digest the catalog gave; an item
//! whose files carry none is listed all the same, as not verified. An id two catalogs list is shown once for each, the later ones as
//! `<id>@<label>`.

use std::collections::{BTreeMap, BTreeSet};
use std::path::Path;

use canoa::store::Check;
use canoa::store::catalog::{FileCheck, MODELS_FORMAT, ModelItem, parse_models};
use canoa::store::models::{
    Fact, ModelFile, ModelRow, RowAction, RowEngine, RowVariant, STEP_DELETE_FILES,
    STEP_FETCH_FILES,
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
    /// For each row, the further origins its catalog's entry says its files are downloaded from.
    pub(super) endpoints: BTreeMap<String, Vec<String>>,
    /// For each row, the files its engines read that no download supplies: the install's steps write them, and removing the row
    /// takes them away.
    pub(super) made: BTreeMap<String, Vec<String>>,
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
        Some(FileCheck::Md5(value)) => Some(Check::Md5 {
            value: value.clone(),
        }),
        Some(FileCheck::GitBlob(value)) => Some(Check::GitBlob {
            value: value.clone(),
        }),
        Some(FileCheck::Unverified(why)) => Some(Check::Unverified { why: why.clone() }),
        None => Some(Check::Unverified {
            why: "the catalog gives no checksum".to_owned(),
        }),
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

/// The files the engines of `item` read that none of its downloads supplies.
fn made_files(item: &ModelItem) -> Vec<String> {
    let mut made: Vec<String> = Vec::new();
    for file in item.engines.iter().flat_map(|engine| &engine.files) {
        if !item.files.iter().any(|each| &each.file == file) && !made.contains(file) {
            made.push(file.clone());
        }
    }
    made
}

/// What the install of `item` asks for: the download, then each engine's `make` once.
fn install_steps(item: &ModelItem) -> Vec<String> {
    let mut steps = vec![STEP_FETCH_FILES.to_owned()];
    for make in item
        .engines
        .iter()
        .filter_map(|engine| engine.make.as_ref())
    {
        if !steps.contains(make) {
            steps.push(make.clone());
        }
    }
    steps
}

/// The pills of `item`: each engine, installed when every file it reads is in `folder`.
fn engines_of(item: &ModelItem, folder: Option<&str>, done: bool) -> Vec<RowEngine> {
    item.engines
        .iter()
        .map(|engine| {
            let installed = if engine.files.is_empty() {
                done
            } else {
                folder.is_some_and(|folder| {
                    engine
                        .files
                        .iter()
                        .all(|file| Path::new(folder).join(file).is_file())
                })
            };
            let mut shown = RowEngine::new(&engine.id, &engine.label, installed);
            shown.variants = engine
                .variants
                .iter()
                .map(|variant| RowVariant::new(&variant.id, &variant.label, variant.detail.clone()))
                .collect();
            shown
        })
        .collect()
}

fn row_of(
    id: &str,
    item: &ModelItem,
    entry: &Entry,
    verified: Option<&str>,
    checks: Option<&[Check]>,
    folder: Option<&str>,
    done: bool,
) -> ModelRow {
    let mut row = ModelRow::new(id, item.label.clone());
    row.by.clone_from(&item.by);
    row.tags.clone_from(&item.tags);
    row.engines = engines_of(item, folder, done);
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
    if checks.is_some() {
        row.on = done;
        row.state = done.then(|| "installed".to_owned());
        row.actions = if done {
            let mut actions = vec![RowAction::new(
                format!("Remove {}", item.label),
                "The files of this model are deleted from this computer.",
                "Remove",
                [STEP_DELETE_FILES],
            )];
            // An engine added to the host after the download has nothing to read yet: its action writes the files it reads from
            // the ones already here, so the model need not be downloaded again.
            for (engine, shown) in item.engines.iter().zip(&row.engines) {
                if let Some(make) = engine.make.as_ref().filter(|_| shown.state != "installed") {
                    actions.push(RowAction::new(
                        format!("Add {} to {}", engine.label, item.label),
                        format!(
                            "The file {} reads is written from the files already downloaded.",
                            engine.label
                        ),
                        "Write",
                        [make.as_str()],
                    ));
                }
            }
            actions
        } else {
            let mut confirm = match verified {
                Some(algorithm) => format!(
                    "The files are downloaded and checked against the {algorithm} digest {} gives.",
                    entry.label
                ),
                None => format!(
                    "{} gives no checksum for these files, so nothing says they are the ones meant.",
                    entry.label
                ),
            };
            for engine in item.engines.iter().filter(|engine| engine.make.is_some()) {
                confirm.push_str(&format!(
                    " Then the file {} reads is written from them.",
                    engine.label
                ));
            }
            vec![RowAction::new(
                format!("Install {}", item.label),
                confirm,
                "Install",
                install_steps(item),
            )]
        };
    } else {
        row.state = Some("not installable".to_owned());
        row.facts.push(Fact::new(
            "Not installable",
            "its files are checked in a way this host does not know",
        ));
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
        let mut made_by: BTreeMap<String, Vec<String>> = BTreeMap::new();
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
                "endpoints": entry.endpoints,
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
                let verified = item.verified_with();
                let checks: Option<Vec<Check>> = item.files.iter().map(check_of).collect();
                let done = installed(item, folder.as_deref());
                let row = row_of(
                    &id,
                    item,
                    &entry,
                    verified.as_deref(),
                    checks.as_deref(),
                    folder.as_deref(),
                    done,
                );
                let written = made_files(item);
                if !written.is_empty() {
                    made_by.insert(id.clone(), written);
                }
                if let Some(checks) = checks {
                    made.checks.insert(id.clone(), checks);
                } else {
                    made.refused.insert(
                        id.clone(),
                        "its files are checked in a way this host does not know".to_owned(),
                    );
                }
                if !entry.endpoints.is_empty() {
                    made.endpoints.insert(id.clone(), entry.endpoints.clone());
                }
                made.meta
                    .insert(id, json!({"catalog": info.clone(), "verified": verified}));
                made.rows.push(row);
            }
        }
        made.made = made_by;
        made
    }
}

#[cfg(test)]
#[path = "models_catalog_tests.rs"]
mod tests;
