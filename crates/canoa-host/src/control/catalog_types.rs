//! `store/catalogs`: for each Store type that reads catalogs, what it calls them, the catalogs canoa.cfg lists under it and the
//! catalog sources whose form can add one. Each type answers from its own table, `[store.<type>.catalogs]`, so a catalog added
//! under one type is never listed under another.

use serde_json::{Value, json};

use super::{Caller, Control, DENIED, Grant, bad};
use crate::store::catalog_run::Run;
use crate::store::catalogs::{TypeCatalogs, built_in_types, merge, row_of, table_of};

type Outcome = Result<Value, (i32, String)>;

impl Control {
    /// `store/catalogs` `{"type"?: string}` -> `{"schema": 1, "types": [{"type", "title", "words": {"noun", "plural", "lists"},
    /// "default"?: {"label", "source"}, "format"?, "catalogs": [{"label", "provider", "fields", "source"?, "default", "origin", "enabled", "pin"?, "ttl"?, "keys"?, "endpoints"?, "trust"? (`pinned`), "snapshot"?, "fetched"?, "from"?, "problem"?}],
    /// "sources": [{"id", "title", "help"?, "extension"?, "types", "schema"?, "form"?}]}]}`, the schema and the form a guatiao schema and a
    /// guatiao-intake form document. With `type` only that type is answered.
    /// Needs `overlay.read`, as the catalogs it lists do.
    pub(super) fn store_catalogs(&self, caller: &Caller, params: &Value) -> Outcome {
        if !caller.may(Grant::OverlayRead) {
            return Err((
                DENIED,
                "this key does not hold the overlay.read grant".to_owned(),
            ));
        }
        let wanted = match params.get("type") {
            None | Some(Value::Null) => None,
            Some(Value::String(kind)) => Some(kind.as_str()),
            Some(_) => return Err(bad("type is the id of a Store type")),
        };
        let store = self.store_section();
        let mut kinds = built_in_types();
        let mut run = None;
        if let Ok(stores) = self.stores() {
            kinds.extend(stores.catalog_types());
            run = Some(stores.catalog_run());
        }
        let sources = self.catalog_sources();
        let mut types = Vec::new();
        for kind in kinds
            .iter()
            .filter(|kind| wanted.is_none_or(|id| id == kind.id))
        {
            types.push(answer_for(kind, &store, &sources, run.as_ref()));
        }
        if let Some(id) = wanted
            && types.is_empty()
        {
            return Err((
                super::NOT_FOUND,
                format!("this host has no Store type {id:?} that reads catalogs"),
            ));
        }
        Ok(json!({"schema": 1, "types": types}))
    }
}

fn answer_for(
    kind: &TypeCatalogs,
    store: &Value,
    sources: &crate::store::sources::Sources,
    run: Option<&Run>,
) -> Value {
    let offered: Vec<Value> = sources
        .list()
        .filter(|(info, _)| info.serves(&kind.id))
        .map(|(info, extension)| {
            json!({
                "id": info.id,
                "title": info.title,
                "help": info.help,
                "extension": extension,
                "types": info.types,
                "schema": info.schema.as_ref().and_then(|schema| crate::forms::json_of(schema).ok()),
                "form": info.form.as_ref().and_then(|form| crate::forms::json_of(form).ok()),
            })
        })
        .collect();
    let mut offered = offered;
    // The one that reads what the type reads itself is first, then the rest as their ids sort.
    offered.sort_by_key(|each| {
        let id = each["id"].as_str().unwrap_or_default().to_owned();
        (id != crate::store::catalogs::READ, id)
    });
    let mut row = json!({
        "type": kind.id,
        "title": kind.title,
        "words": {"noun": kind.noun, "plural": kind.plural, "lists": kind.lists},
        "default": kind.defaults.first().map(|default| json!({"label": default.label, "source": default.fields.get("source")})),
        "format": kind.format,
        "catalogs": merge(kind, table_of(store, &kind.id))
            .iter()
            .map(|merged| {
                let mut row = row_of(kind, merged);
                if let (Some(run), Ok(entry)) = (run, &merged.parsed)
                    && kind.format.is_some()
                    && entry.enabled
                {
                    // What was last read, with nothing asked: the page is opened without waiting on any source.
                    let seen = run.peek(kind, entry);
                    row["fetched"] = seen["fetched"].clone();
                    row["from"] = seen["from"].clone();
                }
                row
            })
            .collect::<Vec<_>>(),
        "sources": offered,
    });
    if kind.id == "skills" {
        row["skills_search"] = json!(crate::control::skill_search::DEFAULT_BASE);
    }
    row
}
