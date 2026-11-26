//! What the control channel says of a catalog read in one of the host's formats: its items (models) or releases (a program's
//! versions), each with the catalog it came from and how far its digest can be believed.
//!
//! An item whose files all carry a digest the publisher states says which algorithm in `verified` (`"SHA-256"`, `"MD5"`, ...); one
//! with a file that has none says null, and is offered all the same.

use canoa::store::catalog::{
    CatalogFile, FileCheck, ItemEngine, MODELS_FORMAT, ModelItem, RELEASES_FORMAT, Release,
    parse_models, parse_releases, verified_with,
};
use serde_json::{Value, json};

use super::catalog_run::Reading;
use super::catalogs::{Entry, TypeCatalogs};

/// A file's check as the control channel words it: `{"kind", "value"}`, or null when the catalog gives none.
fn check_json(check: Option<&FileCheck>) -> Value {
    match check {
        Some(FileCheck::Sha256(value)) => json!({"kind": "sha256", "value": value}),
        Some(FileCheck::Sha512(value)) => json!({"kind": "sha512", "value": value}),
        Some(FileCheck::Md5(value)) => json!({"kind": "md5", "value": value}),
        Some(FileCheck::GitBlob(value)) => json!({"kind": "git_blob", "value": value}),
        Some(FileCheck::Unverified(why)) => json!({"kind": "unverified", "value": why}),
        _ => Value::Null,
    }
}

fn file_json(file: &CatalogFile) -> Value {
    json!({
        "url": file.url,
        "file": file.file,
        "bytes": file.bytes,
        "check": check_json(file.check.as_ref()),
        "derived": file.derived,
        "verified": file.verified_with(),
    })
}

/// Which catalog an item came from and how it was read: the label, its provider, whether it is the type's own, what it is pinned at,
/// the revision the source read, and whether that is a fresh answer.
pub(crate) fn catalog_info(entry: &Entry, revision: Option<&str>, reading: &Reading) -> Value {
    json!({
        "label": entry.label,
        "provider": entry.provider,
        "origin": entry.origin.word(),
        "trust": entry.trust(),
        "pin": entry.pin,
        "revision": revision,
        "fetched": reading.fetched,
        "from": reading.from,
        "stale": reading.stale,
        "problem": reading.problem,
    })
}

fn item_json(item: &ModelItem, catalog: &Value) -> Value {
    json!({
        "id": item.id,
        "label": item.label,
        "by": item.by,
        "tags": item.tags,
        "license": item.license,
        "facts": item.facts.iter().map(|(label, value)| json!({"label": label, "value": value})).collect::<Vec<_>>(),
        "files": item.files.iter().map(file_json).collect::<Vec<_>>(),
        "engines": item.engines.iter().map(ItemEngine::to_value).collect::<Vec<_>>(),
        "verified": item.verified_with(),
        "catalog": catalog,
    })
}

fn release_json(release: &Release, catalog: &Value) -> Value {
    let platforms: serde_json::Map<String, Value> = release
        .platforms
        .iter()
        .map(|(name, platform)| {
            (
                name.clone(),
                json!({
                    "launch": platform.launch,
                    "parts": platform.parts.iter().map(|part| {
                        let mut shown = file_json(&part.file);
                        shown["path"] = json!(part.path);
                        shown["executable"] = json!(part.executable);
                        shown
                    }).collect::<Vec<_>>(),
                }),
            )
        })
        .collect();
    let verified = verified_with(
        release
            .platforms
            .values()
            .flat_map(|platform| platform.parts.iter())
            .map(|part| &part.file),
    );
    json!({
        "version": release.version,
        "channels": release.channels,
        "notices": release.notices,
        "platforms": platforms,
        "verified": verified,
        "catalog": catalog,
    })
}

/// The answer of `store/catalog` for a scope whose catalogs answer in a format of the host's: the catalog's own facts, and its items
/// or releases.
///
/// # Errors
///
/// Why the text is no document of the type's format.
pub(crate) fn view(kind: &TypeCatalogs, entry: &Entry, reading: &Reading) -> Result<Value, String> {
    match kind.format.as_deref() {
        Some(MODELS_FORMAT) => {
            let catalog = parse_models(&reading.text)?;
            let info = catalog_info(entry, catalog.revision.as_deref(), reading);
            Ok(json!({
                "type": kind.id,
                "label": entry.label,
                "provider": entry.provider,
                "format": MODELS_FORMAT,
                "revision": catalog.revision,
                "catalog": info,
                "items": catalog.items.iter().map(|item| item_json(item, &info)).collect::<Vec<_>>(),
            }))
        }
        Some(RELEASES_FORMAT) => {
            let catalog = parse_releases(&reading.text)?;
            let info = catalog_info(entry, None, reading);
            Ok(json!({
                "type": kind.id,
                "label": entry.label,
                "provider": entry.provider,
                "format": RELEASES_FORMAT,
                "channels": catalog.channels,
                "signed": catalog.signed.as_ref().map(|(by, over)| json!({"by": by, "over": over})),
                "catalog": info,
                "releases": catalog.releases.iter().map(|release| release_json(release, &info)).collect::<Vec<_>>(),
            }))
        }
        _ => Err(format!(
            "{} lists nothing in a format of the host's",
            kind.title
        )),
    }
}

#[cfg(test)]
#[path = "catalog_view_tests.rs"]
mod tests;
