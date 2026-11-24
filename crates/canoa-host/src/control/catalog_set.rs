//! The two switches the Settings pages Sources and Providers write: `store/catalog_set` (a catalog of a Store type: replace a default
//! by its label, add one, turn one off or on, or take an entry out of canoa.cfg) and `providers/switch` (a provider the libraries
//! register: off or on in `[host.disabled]`). Each is one read and one write of canoa.cfg under the lock every write takes, so it
//! never takes another writer's change out; nothing here changes what a page could not write with `config/set`.

use serde_json::{Map, Value, json};

use super::{Caller, Control, DENIED, Grant, REFUSED, bad, text_param};
use crate::store::catalogs::{self, READ};

type Outcome = Result<Value, (i32, String)>;

impl Control {
    /// What a type's catalogs list is what canoa.cfg says now: nothing kept from before it is answered after a write of it.
    pub(super) fn forget_listings(&self) {
        if let Ok(stores) = self.stores() {
            stores.forget_listings();
        }
    }

    /// `store/catalog_set` `{"type", "label", "entry"}` -> `{"ok": true, "type", "label", "catalogs": [row, ...]}`, where `entry` is
    /// what `[store.<type>.catalogs.<label>]` becomes: `false` turns the catalog of that label off (a default stays listed, off),
    /// a string or a table (`{provider?, ...fields, pin?, trusted?, ttl?, keys?}`) replaces the default of that label or adds one,
    /// and null takes the entry out of canoa.cfg (a default of that label is on again). A catalog that runs a program, or reads
    /// through an extension's source, takes the `hosting` grant, as writing it with `config/set` does. Needs `config.write`.
    pub(super) fn store_catalog_set(&self, caller: &Caller, params: &Value) -> Outcome {
        let kind = text_param(params, "type")?.ok_or_else(|| bad("type is needed"))?;
        let label = text_param(params, "label")?.ok_or_else(|| bad("label is needed"))?;
        let entry = params.get("entry").cloned().unwrap_or(Value::Null);
        if !matches!(
            entry,
            Value::Null | Value::Bool(false) | Value::String(_) | Value::Object(_)
        ) {
            return Err(bad(
                "entry is false, a source, a table naming a provider and its fields, or null",
            ));
        }
        let scopes = {
            let mut scopes = catalogs::built_in_types();
            if let Ok(stores) = self.stores() {
                scopes.extend(stores.catalog_types());
            }
            scopes
        };
        let scope = scopes
            .iter()
            .find(|scope| scope.id == kind)
            .ok_or_else(|| {
                (
                    super::NOT_FOUND,
                    format!("this host has no Store type {kind:?} that reads catalogs"),
                )
            })?;
        if scope
            .fixed
            .is_some_and(|(fixed, _)| catalogs::same_label(fixed, label))
        {
            return Err((
                REFUSED,
                format!("{label} is the host's own catalog and cannot be changed"),
            ));
        }
        if !caller.may(Grant::Hosting)
            && entry
                .get("provider")
                .and_then(Value::as_str)
                .is_some_and(|provider| provider != READ)
        {
            return Err((
                DENIED,
                "a catalog that runs a program or reads through an extension's source takes the hosting grant"
                    .to_owned(),
            ));
        }
        // The entry is read as canoa.cfg would read it, so what is refused here is what the host would refuse at its next start.
        if !entry.is_null() {
            let probe = json!({ label: entry.clone() });
            if let Some(Err((_, why))) = catalogs::read_all(&probe).into_iter().next() {
                return Err(bad(&why));
            }
        }
        self.files
            .config_update("store", caller.may(Grant::ConfigWriteSessions), |current| {
                let mut store = current.as_object().cloned().unwrap_or_default();
                let mut own = store
                    .get(kind)
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                let mut table: Map<String, Value> = own
                    .get("catalogs")
                    .and_then(Value::as_object)
                    .cloned()
                    .unwrap_or_default();
                // A label is the same label whatever its case: the entry the file holds under it is the one replaced.
                let held = table
                    .keys()
                    .find(|each| catalogs::same_label(each, label))
                    .cloned();
                if let Some(held) = held {
                    table.remove(&held);
                }
                if !entry.is_null() {
                    table.insert(label.to_owned(), entry.clone());
                }
                if table.is_empty() {
                    own.remove("catalogs");
                } else {
                    own.insert("catalogs".to_owned(), Value::Object(table));
                }
                if own.is_empty() {
                    store.remove(kind);
                } else {
                    store.insert(kind.to_owned(), Value::Object(own));
                }
                Ok(if store.is_empty() {
                    Value::Null
                } else {
                    Value::Object(store)
                })
            })?;
        self.forget_listings();
        let store = self.store_section();
        Ok(json!({
            "ok": true,
            "type": kind,
            "label": label,
            "catalogs": catalogs::rows_of(scope, catalogs::table_of(&store, kind)),
        }))
    }

    /// `providers/switch` `{"kind", "id", "off"}` -> `{"ok": true, "kind", "id", "off", "restart": true, "disabled": {...}}`: puts
    /// the provider `id` of the kind `kind` in `[host.disabled]`, or takes it out. The host reads `[host]` when it starts, so the
    /// switch is in force after the host next starts. A provider no loaded library offers cannot be turned off, since there is
    /// nothing to turn off (an entry the file already holds for one can be turned on). Needs `hosting`.
    pub(super) fn providers_switch(&self, params: &Value) -> Outcome {
        let kind = text_param(params, "kind")?.ok_or_else(|| bad("kind is needed"))?;
        let id = text_param(params, "id")?.ok_or_else(|| bad("id is needed"))?;
        let off = match params.get("off") {
            Some(Value::Bool(off)) => *off,
            _ => return Err(bad("off is true or false")),
        };
        if off {
            let config = self.loaded_config()?;
            let known = super::providers::pairs(&config);
            if !known.iter().any(|(k, i)| k == kind && i == id) {
                let ids: Vec<&str> = known
                    .iter()
                    .filter(|(k, _)| k == kind)
                    .map(|(_, i)| i.as_str())
                    .collect();
                return Err((
                    REFUSED,
                    if ids.is_empty() {
                        format!("no loaded library offers a provider of the kind {kind:?}")
                    } else {
                        format!(
                            "no loaded library offers {id:?} as a {kind}; the providers of that kind are {}",
                            ids.join(", ")
                        )
                    },
                ));
            }
        }
        let mut now = Value::Null;
        self.files.hidden_update(super::files::HOST, |current| {
            let mut host = current.as_object().cloned().unwrap_or_default();
            let mut table: Map<String, Value> = host
                .get("disabled")
                .and_then(Value::as_object)
                .cloned()
                .unwrap_or_default();
            let mut ids: Vec<String> = table
                .get(kind)
                .and_then(Value::as_array)
                .map(|each| {
                    each.iter()
                        .filter_map(|id| id.as_str().map(str::to_owned))
                        .collect()
                })
                .unwrap_or_default();
            ids.retain(|each| each != id);
            if off {
                ids.push(id.to_owned());
            }
            if ids.is_empty() {
                table.remove(kind);
            } else {
                table.insert(kind.to_owned(), json!(ids));
            }
            now = Value::Object(table.clone());
            if table.is_empty() {
                host.remove("disabled");
            } else {
                host.insert("disabled".to_owned(), Value::Object(table));
            }
            Ok(Value::Object(host))
        })?;
        Ok(
            json!({"ok": true, "kind": kind, "id": id, "off": off, "restart": true, "disabled": now}),
        )
    }
}
