//! What the control channel says of a catalog read in one of the host's formats: its items (models) or releases (a program's
//! versions), each with the catalog it came from and how far its digest can be believed.
//!
//! An item whose files carry no digest is listed with the reason and no trust word, and is not installable unless the catalog's entry
//! says `trusted = true`; one whose files all carry a digest is `verified` (a publisher's), `pinned` (derived at a pinned commit) or,
//! when the entry is trusted and a file has none, `unverified`.

use canoa::store::catalog::{
    CatalogFile, FileCheck, MODELS_FORMAT, ModelItem, RELEASES_FORMAT, Release, Trust,
    parse_models, parse_releases,
};
use serde_json::{Value, json};

use super::catalog_run::Reading;
use super::catalogs::{Entry, TypeCatalogs};

/// A file's check as the control channel words it: `{"kind", "value"}`, or null when the catalog gives none.
fn check_json(check: Option<&FileCheck>) -> Value {
    match check {
        Some(FileCheck::Sha256(value)) => json!({"kind": "sha256", "value": value}),
        Some(FileCheck::Sha512(value)) => json!({"kind": "sha512", "value": value}),
        Some(FileCheck::GitBlob(value)) => json!({"kind": "git_blob", "value": value}),
        Some(FileCheck::Unverified(why)) => json!({"kind": "unverified", "value": why}),
        _ => Value::Null,
    }
}

fn file_json(file: &CatalogFile, trusted: bool) -> Value {
    let mut shown = json!({
        "url": file.url,
        "file": file.file,
        "bytes": file.bytes,
        "check": check_json(file.check.as_ref()),
        "derived": file.derived,
    });
    match file.trust(trusted) {
        Ok(trust) => shown["trust"] = json!(trust.word()),
        Err(why) => shown["problem"] = json!(why),
    }
    shown
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

/// The weakest trust in `trusts`, or the reason of the first file refused.
fn weakest(trusts: impl IntoIterator<Item = Result<Trust, String>>) -> Result<Trust, String> {
    let mut least = Trust::Verified;
    for trust in trusts {
        least = least.min(trust?);
    }
    Ok(least)
}

fn item_json(item: &ModelItem, entry: &Entry, catalog: &Value) -> Value {
    let mut shown = json!({
        "id": item.id,
        "label": item.label,
        "by": item.by,
        "tags": item.tags,
        "license": item.license,
        "facts": item.facts.iter().map(|(label, value)| json!({"label": label, "value": value})).collect::<Vec<_>>(),
        "files": item.files.iter().map(|file| file_json(file, entry.trusted)).collect::<Vec<_>>(),
        "catalog": catalog,
    });
    match item.trust(entry.trusted) {
        Ok(trust) => shown["trust"] = json!(trust.word()),
        Err(why) => shown["problem"] = json!(why),
    }
    shown
}

fn release_json(release: &Release, entry: &Entry, catalog: &Value) -> Value {
    let platforms: serde_json::Map<String, Value> = release
        .platforms
        .iter()
        .map(|(name, platform)| {
            (
                name.clone(),
                json!({
                    "launch": platform.launch,
                    "parts": platform.parts.iter().map(|part| {
                        let mut shown = file_json(&part.file, entry.trusted);
                        shown["path"] = json!(part.path);
                        shown["executable"] = json!(part.executable);
                        shown
                    }).collect::<Vec<_>>(),
                }),
            )
        })
        .collect();
    let trusts = release
        .platforms
        .values()
        .flat_map(|platform| platform.parts.iter())
        .map(|part| part.file.trust(entry.trusted));
    let mut shown = json!({
        "version": release.version,
        "channels": release.channels,
        "notices": release.notices,
        "platforms": platforms,
        "catalog": catalog,
    });
    match weakest(trusts) {
        Ok(trust) => shown["trust"] = json!(trust.word()),
        Err(why) => shown["problem"] = json!(why),
    }
    shown
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
                "items": catalog.items.iter().map(|item| item_json(item, entry, &info)).collect::<Vec<_>>(),
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
                "releases": catalog.releases.iter().map(|release| release_json(release, entry, &info)).collect::<Vec<_>>(),
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
