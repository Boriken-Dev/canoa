//! The tables of canoa.cfg no dedicated page edits, described so that one generic editor can add, change and remove them: `config/advanced`
//! lists each with the guatiao schema of what it holds, the person's own values and what a lower layer gives, and `config/advanced_set`
//! writes or removes one table or entry through the configuration store.
//!
//! A table is listed here when the host reads it, so a key the host learns shows up in the editor with the one line that names it in
//! [`TABLES`], and `config::coverage_tests` fails when a key the host reads is in neither [`TABLES`], [`DEDICATED`] nor [`OWNED`].
#![allow(clippy::std_instead_of_alloc, reason = "the control channel is std's")]

use canoa::config_store::{Changes, Op, json_of, value_of};
use serde_json::{Map, Value, json};

use super::config::store_failed;
use super::{Files, Outcome, bad, refused};

/// How a table holds what it holds.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Shape {
    /// One table of settings: `[host.web]`.
    Flat,
    /// A table of entries, each a table of the same settings, named by its key: `[host.tools.NAME]`.
    Entries,
    /// An array of tables: `[[host.openers]]`.
    List,
    /// A table of names, each one text: `[harnesses]`.
    Pairs,
}

/// What a setting holds, which says how its form shows it.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum Kind {
    Bool,
    /// A whole number.
    Int,
    Text,
    /// A list of text, one to a line.
    Lines,
    /// Free-form: a table or a list the host reads whatever its keys, shown and edited as JSON.
    Json,
}

/// One table the generic editor offers.
#[derive(Debug)]
pub(crate) struct Table {
    /// Where it is in canoa.cfg.
    pub(crate) path: &'static [&'static str],
    pub(crate) shape: Shape,
    /// Every key an entry of it takes.
    pub(crate) fields: &'static [(&'static str, Kind)],
}

use Kind::{Bool, Int, Json, Lines, Text};

/// Every table the generic editor offers, with each key the host reads in it.
pub(crate) const TABLES: &[Table] = &[
    Table {
        path: &["harnesses"],
        shape: Shape::Pairs,
        fields: &[("value", Text)],
    },
    Table {
        path: &["assist_harness"],
        shape: Shape::Entries,
        fields: &[
            ("source", Text),
            ("title", Text),
            ("profile", Text),
            ("account", Text),
            ("model", Text),
            ("effort", Text),
        ],
    },
    Table {
        path: &["assist_pool"],
        shape: Shape::Flat,
        fields: &[("idle_seconds", Int), ("max_sessions", Int)],
    },
    Table {
        path: &["host"],
        shape: Shape::Flat,
        fields: &[
            ("origins", Lines),
            ("shell", Text),
            ("max_message_bytes", Int),
        ],
    },
    Table {
        path: &["host", "web"],
        shape: Shape::Flat,
        fields: &[
            ("dir", Text),
            ("listen", Text),
            ("cert", Text),
            ("key", Text),
            ("hosts", Lines),
            ("mode", Text),
            ("connect", Lines),
            ("office", Bool),
            ("office_frames", Lines),
            ("office_local", Bool),
            ("office_secret", Bool),
            ("max_connections", Int),
            ("max_connections_per_address", Int),
            ("connection_seconds", Int),
            ("max_request_bytes", Int),
            ("max_requests", Int),
            ("request_seconds", Int),
            ("idle_seconds", Int),
            ("send_seconds", Int),
            ("immutable_seconds", Int),
        ],
    },
    Table {
        path: &["host", "control"],
        shape: Shape::Flat,
        fields: &[
            ("listen", Text),
            ("grants", Json),
            ("max_records", Int),
            ("max_record_bytes", Int),
            ("max_upload_bytes", Int),
            ("max_upload_total_bytes", Int),
            ("max_uploads", Int),
        ],
    },
    Table {
        path: &["host", "native"],
        shape: Shape::Flat,
        fields: &[("enabled", Bool), ("control", Lines)],
    },
    Table {
        path: &["host", "network"],
        shape: Shape::Flat,
        fields: &[
            ("proxy", Text),
            ("proxy_user", Text),
            ("proxy_password_env", Text),
            ("no_proxy", Lines),
            ("negotiate", Bool),
            ("client_certificate", Text),
            ("ca_file", Text),
            ("git_tls", Text),
            ("endpoints", Lines),
            ("proxy_idle_seconds", Int),
        ],
    },
    Table {
        path: &["host", "store"],
        shape: Shape::Flat,
        fields: &[
            ("protected", Lines),
            ("credential_variables", Lines),
            ("skill_folders", Lines),
            ("index_ttl", Int),
            ("index_cache_bytes", Int),
            ("latest_ttl", Int),
            ("models_ttl", Int),
            ("catalog_ttl", Int),
            ("refresh_every", Int),
            ("connect_timeout", Int),
            ("lookup_timeout", Int),
            ("tools", Bool),
            ("verify_copies", Bool),
        ],
    },
    Table {
        path: &["host", "content"],
        shape: Shape::Flat,
        fields: &[("ttl", Int), ("document_bytes", Int)],
    },
    Table {
        path: &["host", "delegate"],
        shape: Shape::Flat,
        fields: &[
            ("enabled", Bool),
            ("sources", Lines),
            ("max_children", Int),
            ("max_running", Int),
            ("max_depth", Int),
            ("a2a", Lines),
            ("task_bytes", Int),
            ("result_bytes", Int),
            ("history_bytes", Int),
            ("context_bytes", Int),
            ("wait_seconds", Int),
            ("dial_seconds", Int),
        ],
    },
    Table {
        path: &["host", "kb"],
        shape: Shape::Flat,
        fields: &[
            ("tools", Bool),
            ("chunk_bytes", Int),
            ("overlap_bytes", Int),
            ("retry_seconds", Int),
            ("query_cache", Int),
        ],
    },
    Table {
        path: &["host", "kb", "embedder"],
        shape: Shape::Flat,
        fields: &[
            ("use", Text),
            ("model", Text),
            ("dimensions", Int),
            ("document_prefix", Text),
            ("query_prefix", Text),
        ],
    },
    Table {
        path: &["host", "kb", "ranker"],
        shape: Shape::Flat,
        fields: &[
            ("use", Text),
            ("model", Text),
            ("candidates", Int),
            ("timeout_seconds", Int),
            ("passage_chars", Int),
        ],
    },
    Table {
        path: &["host", "kb", "search"],
        shape: Shape::Flat,
        fields: &[("use", Text)],
    },
    Table {
        path: &["host", "mcp_list"],
        shape: Shape::Flat,
        fields: &[
            ("query_seconds", Int),
            ("max_items_per_server", Int),
            ("max_reply_bytes", Int),
        ],
    },
    Table {
        path: &["host", "usage"],
        shape: Shape::Flat,
        fields: &[("keep_days", Int)],
    },
    Table {
        path: &["host", "worktrees"],
        shape: Shape::Flat,
        fields: &[("max_per_repository", Int), ("git_seconds", Int)],
    },
    Table {
        path: &["host", "history"],
        shape: Shape::Flat,
        fields: &[
            ("record", Bool),
            ("max_session_bytes", Int),
            ("max_sessions", Int),
            ("max_total_bytes", Int),
            ("store", Text),
            ("store_settings", Json),
        ],
    },
    Table {
        path: &["host", "tools"],
        shape: Shape::Entries,
        fields: &[
            ("program", Text),
            ("args", Lines),
            ("runs", Text),
            ("unasked", Text),
            ("prompt", Json),
            ("cancel", Json),
            ("resume_args", Lines),
            ("kinds", Json),
            ("max_sessions", Int),
            ("events", Json),
        ],
    },
    Table {
        path: &["host", "environments"],
        shape: Shape::Entries,
        fields: &[
            ("kind", Text),
            ("library", Text),
            ("source", Text),
            ("source_settings", Json),
            ("import_search", Text),
            ("exec_seconds", Int),
            ("output_bytes", Int),
            ("scrollback_bytes", Int),
            ("max_tool_sessions", Int),
            ("max_prepared", Int),
            ("prepared_seconds", Int),
            ("hook_read_ms", Int),
            ("hook_handlers", Int),
            ("hook_seconds", Int),
            ("settings", Json),
        ],
    },
    Table {
        path: &["host", "openers"],
        shape: Shape::List,
        fields: &[
            ("id", Text),
            ("label", Text),
            ("program", Text),
            ("args", Lines),
            ("item", Bool),
        ],
    },
];

/// The keys of a table the host reads that a dedicated page of the application edits, so the generic editor repeats them only as a
/// convenience. `(table, keys)`; `config::coverage_tests` checks every key the host reads is here, in [`TABLES`] or in [`OWNED`].
#[cfg(test)]
pub(crate) const DEDICATED: &[(&str, &[&str])] = &[
    (
        "(top level)",
        &[
            "host",
            "usage",
            "permissions",
            "settings",
            "agents",
            "profile",
            "overlays",
            "skill_pins",
            "loadouts",
            "defaults",
            "profiles",
            "secrets",
            // The knowledge search page edits the server it keeps; `[assist.OPERATION]` is edited as JSON until its own page.
            "assist",
            "sdk",
            "assist_harness",
            "assist_pool",
            // Settings' Notifications page edits the routes of each event and the notifier entries.
            "notifications",
        ],
    ),
    (
        "host",
        &[
            "agents",
            "providers",
            "extensions",
            "profiles",
            "accounts",
            "pools",
            "disabled",
            "services",
        ],
    ),
    ("settings", &["power"]),
    ("settings.power", &["keep_awake"]),
    ("permissions", &["protected"]),
    ("usage", &["limit_phrases"]),
    ("notifications", &["notifiers"]),
    (
        "notifications.notifiers.NAME",
        &[
            "provider",
            "library",
            "source",
            "source_settings",
            "import_search",
            "settings",
        ],
    ),
    (
        "host.agents.NAME",
        &[
            "program",
            "args",
            "harness",
            "library",
            "source",
            "source_settings",
            "import_search",
            "settings",
            "account_settings",
            "executable",
            "setting_sources",
            "cache_lifetime",
            "shell",
            "profiles",
            "accounts",
            "usage",
            "usage_settings",
            "title",
            "shared",
            "environment",
            "shared_idle_seconds",
            "shared_permission_window_seconds",
            "shared_keep_log",
            "shared_log_entries",
            "shared_log_bytes",
            "listen",
            "transport",
            "transport_library",
            "transport_source",
            "transport_source_settings",
            "transport_import_search",
            "transport_settings",
            "working_roots",
            "worktrees",
            "peers",
            "max_peers",
        ],
    ),
    (
        "host.providers.NAME",
        &[
            "library",
            "source",
            "source_settings",
            "import_search",
            "settings",
            "provider",
            "max_sessions",
            "listen",
            "transport",
            "transport_library",
            "transport_source",
            "transport_source_settings",
            "transport_import_search",
            "transport_settings",
            "working_roots",
            "peers",
            "max_peers",
        ],
    ),
    ("host.accounts.NAME", &["kind", "id", "settings"]),
    ("host.pools.NAME", &["standard", "accounts"]),
    (
        "host.extensions.NAME",
        &[
            "library",
            "source",
            "source_settings",
            "import_search",
            "provider",
            "pin",
            "sha256",
            "root",
        ],
    ),
];

/// The keys the host reads that no page edits because the host or the Store writes them, with why.
#[cfg(test)]
pub(crate) const OWNED: &[(&str, &str, &str)] = &[
    (
        "host.extensions.NAME",
        "pin",
        "the Store writes the pin when it installs the extension",
    ),
    (
        "host.extensions.NAME",
        "root",
        "the Store writes the folder the pin covers",
    ),
    (
        "host.extensions.NAME",
        "sha256",
        "the Store writes the library's digest",
    ),
    (
        "host.accounts.NAME",
        "id",
        "the host makes the id of an account and its stored credential is kept under it",
    ),
];

/// A key as the person reads it: `max_per_repository` is "Max per repository".
fn title_of(key: &str) -> String {
    let mut text = key.replace('_', " ");
    if let Some(first) = text.get_mut(..1) {
        first.make_ascii_uppercase();
    }
    text
}

fn table_title(table: &Table) -> String {
    format!("[{}]", table.path.join("."))
}

/// The guatiao schema of one entry of `table`: one member per key, in the order [`TABLES`] gives them.
fn schema_of(table: &Table) -> Value {
    use coqui_acp_api::guatiao::schema::{FieldBuilder, KindBuilder, SchemaBuilder};
    let mut schema = SchemaBuilder::new().title(&table_title(table));
    for (key, kind) in table.fields {
        let shown = match kind {
            Kind::Bool => KindBuilder::bool(),
            Kind::Int => KindBuilder::int_bounds(Some(0), None),
            Kind::Text | Kind::Json => KindBuilder::string(),
            Kind::Lines => KindBuilder::list(KindBuilder::string()),
        };
        let mut field = FieldBuilder::new(key, shown).title(&title_of(key));
        if *kind == Kind::Json {
            field = field.option("x-json", true);
        }
        schema = schema.field(field);
    }
    schema
        .finish()
        .ok()
        .and_then(|value| json_of(&value))
        .unwrap_or(Value::Null)
}

/// Whether `value` is something a setting of `kind` holds.
fn fits(kind: Kind, value: &Value) -> bool {
    match kind {
        Kind::Bool => value.is_boolean(),
        Kind::Int => value.is_i64() || value.is_u64(),
        Kind::Text => value.is_string(),
        Kind::Lines => value
            .as_array()
            .is_some_and(|items| items.iter().all(Value::is_string)),
        Kind::Json => value.is_object() || value.is_array(),
    }
}

impl Table {
    /// The table `path` is, or the entry of one: `path` is a table's whole path, or for a table of entries its path and a name.
    fn named(path: &[String]) -> Option<(&'static Table, Option<String>)> {
        TABLES.iter().find_map(|table| {
            let same = |len: usize| {
                path.len() == len
                    && table
                        .path
                        .iter()
                        .zip(path)
                        .all(|(want, have)| *want == have)
            };
            match table.shape {
                Shape::Entries | Shape::Pairs if same(table.path.len() + 1) => {
                    Some((table, path.last().cloned()))
                }
                Shape::Flat | Shape::List if same(table.path.len()) => Some((table, None)),
                _ => None,
            }
        })
    }

    fn check(&self, value: &Value) -> Result<(), String> {
        let check_entry = |entry: &Value| -> Result<(), String> {
            let Value::Object(map) = entry else {
                return Err(format!("{} holds tables", table_title(self)));
            };
            for (key, held) in map {
                let Some((_, kind)) = self.fields.iter().find(|(name, _)| name == key) else {
                    return Err(format!("{} has no setting {key}", table_title(self)));
                };
                if !fits(*kind, held) {
                    return Err(format!(
                        "{key} in {} is not what that setting holds",
                        table_title(self)
                    ));
                }
            }
            Ok(())
        };
        match (self.shape, value) {
            (Shape::Pairs, Value::String(_)) => Ok(()),
            (Shape::Pairs, _) => Err(format!("{} holds text", table_title(self))),
            (Shape::List, Value::Array(rows)) => rows.iter().try_for_each(check_entry),
            (Shape::List, _) => Err(format!("{} is a list of tables", table_title(self))),
            (_, entry) => check_entry(entry),
        }
    }
}

fn lookup<'a>(root: &'a Value, path: &[&str]) -> Option<&'a Value> {
    path.iter().try_fold(root, |here, key| here.get(key))
}

impl Files {
    /// `config/layers`: every layer of the configuration, lowest first, each with the document it holds, which one is written, and the
    /// store's revision. What an application that shows where a value comes from reads, whether or not a host serves it.
    pub(crate) fn config_layers(&self) -> Outcome {
        let store = self.store()?;
        let info = store.describe().map_err(store_failed)?;
        let snapshot = store.read().map_err(store_failed)?;
        let layers: Vec<Value> = info
            .layers
            .iter()
            .map(|layer| {
                let document = snapshot
                    .layers
                    .iter()
                    .find(|read| read.id == layer.id)
                    .and_then(|read| json_of(&read.document))
                    .unwrap_or_else(|| json!({}));
                json!({
                    "id": layer.id,
                    "label": layer.label,
                    "origin": layer.origin,
                    "writable": layer.writable,
                    "document": document,
                })
            })
            .collect();
        Ok(json!({
            "name": info.name,
            "base_dir": info.base_dir,
            "revision": snapshot.revision,
            "layers": layers,
        }))
    }

    /// `config/change` `{ops: [{path, value?}], check?, revision?}`: makes the ops in the person's own layer, all or none, through the
    /// configuration store: a `value` sets the path, none takes it out. With `check` nothing is written; a `revision` the store has left
    /// is refused with `-32004`.
    pub(crate) fn config_change(&self, params: &Value) -> Outcome {
        let _one = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let store = self.store()?;
        let ops: Vec<Op> = params
            .get("ops")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("ops is a list of {path, value}"))?
            .iter()
            .map(|op| {
                let path: Vec<String> = op
                    .get("path")
                    .and_then(Value::as_array)
                    .map(|keys| {
                        keys.iter()
                            .filter_map(Value::as_str)
                            .map(str::to_owned)
                            .collect()
                    })
                    .filter(|keys: &Vec<String>| !keys.is_empty())
                    .ok_or_else(|| bad("path is the keys from the top, as a list of text"))?;
                Ok(match op.get("value").filter(|value| !value.is_null()) {
                    Some(value) => Op::set(path.iter().map(String::as_str), value_of(value)),
                    None => Op::remove(path.iter().map(String::as_str)),
                })
            })
            .collect::<Result<_, (i32, String)>>()?;
        let mut changes = Changes::of(ops);
        changes.check_only = params.get("check").and_then(Value::as_bool) == Some(true);
        changes.expect_revision = params
            .get("revision")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let done = store.change(changes).map_err(|error| {
            let status = error.to_string();
            (
                if status.contains("changed since") {
                    super::CHANGED
                } else {
                    super::REFUSED
                },
                status,
            )
        })?;
        Ok(json!({"revision": done.revision, "restart": true}))
    }

    /// `config/advanced`: every table the generic editor offers, with its schema, what the person's own file holds and what a lower layer
    /// gives (`inherited`, each with the label of the layer it comes from).
    pub(crate) fn advanced_list(&self) -> Outcome {
        let store = self.store()?;
        let info = store.describe().map_err(store_failed)?;
        let own = info.writable().unwrap_or_default().to_owned();
        let snapshot = store.read().map_err(store_failed)?;
        let layer_label = |id: &str| {
            info.layers
                .iter()
                .find(|layer| layer.id == id)
                .map_or_else(|| id.to_owned(), |layer| layer.label.clone())
        };
        let documents: Vec<(String, Value)> = snapshot
            .layers
            .iter()
            .map(|layer| {
                (
                    layer.id.clone(),
                    json_of(&layer.document).unwrap_or_else(|| json!({})),
                )
            })
            .collect();
        let mut tables = Vec::new();
        for table in TABLES {
            let mut mine = Value::Null;
            let mut inherited = Map::new();
            for (id, document) in &documents {
                let Some(held) = lookup(document, table.path) else {
                    continue;
                };
                if *id == own {
                    mine = held.clone();
                    continue;
                }
                let from = layer_label(id);
                match (table.shape, held) {
                    (Shape::Entries | Shape::Pairs, Value::Object(entries)) => {
                        for (name, entry) in entries {
                            inherited.insert(name.clone(), json!({"layer": from, "value": entry}));
                        }
                    }
                    (Shape::Flat, Value::Object(keys)) => {
                        for (key, value) in keys {
                            inherited.insert(key.clone(), json!({"layer": from, "value": value}));
                        }
                    }
                    (_, whole) => {
                        inherited.insert(String::new(), json!({"layer": from, "value": whole}));
                    }
                }
            }
            // What the person's own layer names takes the place of what a lower layer gave for the same entry.
            if let (Shape::Entries | Shape::Pairs | Shape::Flat, Value::Object(own_keys)) =
                (table.shape, &mine)
            {
                inherited.retain(|key, _| !own_keys.contains_key(key));
            }
            tables.push(json!({
                "path": table.path,
                "title": table_title(table),
                "shape": match table.shape {
                    Shape::Flat => "flat",
                    Shape::Entries => "entries",
                    Shape::List => "list",
                    Shape::Pairs => "pairs",
                },
                "schema": schema_of(table),
                "own": mine,
                "inherited": inherited,
            }));
        }
        Ok(json!({"tables": tables, "revision": snapshot.revision, "restart": true}))
    }

    /// `config/advanced_set` `{path, value, check?, revision?}`: makes the table at `path` (or, for a table of entries, the entry) hold
    /// `value` in the person's own file, or takes it out when `value` is null. With `check` nothing is written and the answer is whether
    /// the host would read the result. A `revision` the file has left is refused.
    pub(crate) fn advanced_set(&self, params: &Value) -> Outcome {
        let _one = self
            .writing
            .lock()
            .unwrap_or_else(std::sync::PoisonError::into_inner);
        let store = self.store()?;
        let path: Vec<String> = params
            .get("path")
            .and_then(Value::as_array)
            .map(|keys| {
                keys.iter()
                    .filter_map(Value::as_str)
                    .map(str::to_owned)
                    .collect()
            })
            .filter(|keys: &Vec<String>| !keys.is_empty())
            .ok_or_else(|| bad("path is the keys from the top, as a list of text"))?;
        let Some((table, _)) = Table::named(&path) else {
            return Err((
                super::super::DENIED,
                format!(
                    "{} is not a table the generic editor offers",
                    path.join(".")
                ),
            ));
        };
        let value = params.get("value").unwrap_or(&Value::Null);
        let keys: Vec<&str> = path.iter().map(String::as_str).collect();
        let ops = if value.is_null() && !(table.shape == Shape::Flat && keys == ["host"]) {
            vec![Op::remove(keys)]
        } else {
            let value = if value.is_null() { &json!({}) } else { value };
            table.check(value).map_err(|why| refused(&why))?;
            match (table.shape, value) {
                // A table of settings is written key by key, so what its table holds besides the settings it names (the agents under
                // `[host]`) stays, and a setting left out of `value` is taken out.
                (Shape::Flat, Value::Object(given)) => {
                    let mine = serde_json::to_value(self.own_tables()?).unwrap_or(Value::Null);
                    table
                        .fields
                        .iter()
                        .filter_map(|(key, _)| {
                            let mut at = keys.clone();
                            at.push(key);
                            match given.get(*key) {
                                Some(held) => Some(Op::set(at, value_of(held))),
                                // What a lower layer gives is not the person's file's to take out.
                                None if lookup(&mine, &at).is_some() => Some(Op::remove(at)),
                                None => None,
                            }
                        })
                        .collect()
                }
                _ => vec![Op::set(keys, value_of(value))],
            }
        };
        let mut changes = Changes::of(ops);
        changes.check_only = params.get("check").and_then(Value::as_bool) == Some(true);
        changes.expect_revision = params
            .get("revision")
            .and_then(Value::as_str)
            .map(str::to_owned);
        let done = store.change(changes).map_err(|error| {
            let status = error.to_string();
            (
                if status.contains("changed since") {
                    super::CHANGED
                } else {
                    super::REFUSED
                },
                status,
            )
        })?;
        Ok(json!({"path": path, "revision": done.revision, "restart": true}))
    }
}
