//! `store/index`: every catalog and program store this host lists, as one normalized list answered from the index's cache.

use serde_json::{Value, json};

use super::{Caller, Control, DENIED, Grant, bad};
use crate::store::index::parse_sources;

type Outcome = Result<Value, (i32, String)>;

impl Control {
    /// `store/index` `{"refresh"?: bool, "icons"?: bool}` -> `{"schema", "sources": [...], "entries": [...]}`.
    ///
    /// Catalog entries are read with `overlay.read` and program entries with `store.programs`; a key holding one sees only
    /// that half, and one holding neither is refused. The sources are the ones canoa.cfg lists: a key names none, so the
    /// host fetches only places its owner chose. `refresh` forces the fetch, which is `overlay.write`'s.
    pub(super) fn store_index(&self, caller: &Caller, params: &Value) -> Outcome {
        let catalogs = caller.may(Grant::OverlayRead);
        let programs = caller.may(Grant::StorePrograms);
        if !catalogs && !programs {
            return Err((
                DENIED,
                "this key has neither the overlay.read nor the store.programs grant".to_owned(),
            ));
        }
        let flag = |key: &str| match params.get(key) {
            None | Some(Value::Null) => Ok(false),
            Some(Value::Bool(value)) => Ok(*value),
            Some(_) => Err(bad(&format!("{key} is true or false"))),
        };
        let refresh = flag("refresh")?;
        let icons = flag("icons")?;
        if refresh && catalogs && !caller.may(Grant::OverlayWrite) {
            return Err((
                DENIED,
                "refresh fetches sources, which takes the overlay.write grant".to_owned(),
            ));
        }
        let mut sources = Vec::new();
        let mut entries = Vec::new();
        if catalogs {
            let section = self
                .files
                .config_get(&json!({"section": "catalogs"}))
                .ok()
                .and_then(|answer| answer.get("value").cloned())
                .unwrap_or(Value::Null);
            let rows = parse_sources(&section);
            for (row, answer) in
                rows.iter()
                    .zip(self.index.catalogs(&self.overlays, &rows, refresh))
            {
                let mut shown = answer.source;
                let mut kept = answer.entries;
                if icons && let Ok(source) = row {
                    let commit = shown.get("commit").and_then(Value::as_str).unwrap_or("");
                    for entry in &mut kept {
                        let Some(file) = entry
                            .pointer("/icon/file")
                            .and_then(Value::as_str)
                            .map(str::to_owned)
                        else {
                            continue;
                        };
                        if let Some(bytes) = self.index.icon_bytes(source, commit, &file) {
                            entry["icon"]["data"] =
                                Value::String(data_encoding::BASE64.encode(&bytes));
                        }
                    }
                }
                shown["entries"] = json!(kept.len());
                sources.push(shown);
                entries.append(&mut kept);
            }
        }
        if programs {
            match self
                .stores()
                .and_then(|stores| stores.read("store/list", &json!({}), None))
            {
                Ok(listed) => {
                    let found = listed.get("stores").and_then(Value::as_array);
                    let mut count = 0;
                    for info in found.into_iter().flatten() {
                        entries.push(program_entry(info));
                        count += 1;
                    }
                    sources.push(json!({
                        "id": "programs",
                        "kind": "programs",
                        "stale": false,
                        "refreshing": false,
                        "entries": count,
                    }));
                }
                Err((_, why)) => sources.push(json!({
                    "id": "programs",
                    "kind": "programs",
                    "stale": true,
                    "refreshing": false,
                    "problem": why,
                    "entries": 0,
                })),
            }
        }
        Ok(json!({"schema": 1, "sources": sources, "entries": entries}))
    }
}

/// An agent program store as one entry: what the host knows of it with no network.
fn program_entry(info: &Value) -> Value {
    let text = |key: &str| info.get(key).and_then(Value::as_str);
    let program = text("program").unwrap_or_default();
    let provider = text("provider").unwrap_or_default();
    json!({
        "id": format!("programs:program:{provider}:{program}"),
        "kind": "program",
        "catalog": "programs",
        "name": text("title").unwrap_or(program),
        "summary": text("publisher").map(|publisher| format!("Published by {publisher}")),
        "source": provider,
        "version": Value::Null,
        "tags": [],
        "requirements": {
            "platforms": info.get("platforms").cloned().unwrap_or(json!([])),
            "serves": info.get("serves").cloned().unwrap_or(json!([])),
        },
        "pin": Value::Null,
        "contents": Value::Null,
        "icon": Value::Null,
        "homepage": info.get("homepage").cloned().unwrap_or(Value::Null),
        "program": program,
        "provider": provider,
        "channels": info.get("channels").cloned().unwrap_or(json!([])),
    })
}
