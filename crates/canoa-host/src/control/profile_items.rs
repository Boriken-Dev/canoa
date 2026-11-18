//! `profile_items/list`: the things a profile may set, as the providers this host loaded say of them. Nothing here names an item: a
//! row is one `profile_item` provider the host carries, the bundled extensions offer, or an entry's or extension's library of
//! canoa.cfg adds, so the app draws a form for each from what its provider says of it.

use serde_json::{Value, json};

use crate::config::Config;
use crate::profile::items::{ItemCatalog, Listed};

/// `{"items": [...]}`: each with its `id`, `title`, `help`, `origin` (`built-in`, `bundled`, or the name of the entry or extension that
/// loads it), `stored` (`setting` for one of the session's own settings, which a profile keeps under its own name, else `item`),
/// the `harnesses` it applies to (none is every one) and its form's `fields` (`name`, `label`, `shape`, `required`, `default`,
/// `help`, `choices`).
pub(crate) fn list(config: &Config) -> Value {
    of(&ItemCatalog::of(config))
}

/// The answer for `catalog`.
pub(crate) fn of(catalog: &ItemCatalog) -> Value {
    json!({ "items": catalog.list().iter().map(Listed::json).collect::<Vec<_>>() })
}

#[cfg(test)]
mod tests;
