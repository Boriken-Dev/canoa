//! The `config_store` kind: where the host's configuration lives.
//!
//! canoa.cfg, a file in TOML, YAML or JSON that the person edits, is one provider of this kind; a configuration kept by a service, a
//! database or an organization's policy is another, and an extension adds one like any other kind. The host and the application never
//! open the file: they read what the provider holds and ask it to change a path, so where the configuration comes from can change
//! without either knowing.
//!
//! A store holds **layers**, lowest first. The host reads their merge (the rules of the merge are the host's, not the provider's), and
//! a change is made in the one layer that is [`LayerInfo::writable`], the person's own. A path that a lower layer names is shown with
//! the layer it comes from, and is removed there, not here.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue, Value};

/// The name of the `config_store` kind.
pub const KIND: &str = "config_store";

/// One layer of the configuration, lowest first in [`StoreInfo::layers`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct LayerInfo {
    /// Its id, which [`Layer`] and [`Held`] name it by: 1 to 64 letters, digits, `-`, `_` and `.`.
    pub id: String,
    /// What the person is told it is ("the installed canoa.cfg", "your canoa.cfg").
    pub label: String,
    /// Where it lives, for the person to go and change it there when it is not writable (a path, an address).
    pub origin: Option<String>,
    /// Whether [`ConfigStore::change`] writes it. Exactly one layer of a store is.
    #[map(default)]
    pub writable: bool,
}

impl LayerInfo {
    /// A layer called `id`.
    #[must_use]
    pub fn new(id: impl Into<String>, label: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            label: label.into(),
            origin: None,
            writable: false,
        }
    }
}

/// What a store is, from [`ConfigStore::describe`]. Cheap, and the same on every call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct StoreInfo {
    /// Its name for people.
    pub title: String,
    /// One line on what it is.
    pub description: Option<String>,
    /// The folder a relative path in the configuration is made from, when the store has one (the file's directory).
    pub base_dir: Option<String>,
    /// The file name the format of the configuration is told by (`canoa.cfg`), when it is a file.
    pub name: Option<String>,
    /// Its layers, lowest first.
    pub layers: Vec<LayerInfo>,
}

impl StoreInfo {
    /// A store called `title` with these layers.
    #[must_use]
    pub fn new(title: impl Into<String>, layers: Vec<LayerInfo>) -> Self {
        Self {
            title: title.into(),
            description: None,
            base_dir: None,
            name: None,
            layers,
        }
    }

    /// Why this description cannot be used, or `None` when it can: it has a title, each layer has a distinct id, and exactly one is
    /// writable.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if self.title.trim().is_empty() {
            return Some(String::from("a config store has no title"));
        }
        for (at, layer) in self.layers.iter().enumerate() {
            if let Some(why) = id_problem(&layer.id) {
                return Some(why);
            }
            if self.layers[..at].iter().any(|other| other.id == layer.id) {
                return Some(alloc::format!("two layers are called {:?}", layer.id));
            }
        }
        let writable = self.layers.iter().filter(|layer| layer.writable).count();
        (writable != 1).then(|| alloc::format!("{writable} layers are writable, and one must be"))
    }

    /// The id of the writable layer, if the description has exactly one.
    #[must_use]
    pub fn writable(&self) -> Option<&str> {
        let mut writable = self.layers.iter().filter(|layer| layer.writable);
        let first = writable.next()?;
        writable.next().is_none().then_some(first.id.as_str())
    }
}

/// Why `id` cannot name a layer, or `None` when it can.
#[must_use]
pub fn id_problem(id: &str) -> Option<String> {
    let ok = !id.is_empty()
        && id.len() <= 64
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || matches!(byte, b'-' | b'_' | b'.'));
    (!ok).then(|| alloc::format!("the layer id {id:?} is not 1 to 64 letters, digits, -, _ and ."))
}

/// One layer's tables, from [`ConfigStore::read`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Layer {
    /// The layer's id, as [`StoreInfo::layers`] names it.
    pub id: String,
    /// What it holds: a map of tables, as the host reads them. An absent layer (a file that does not exist yet) is an empty map.
    pub document: Value,
}

impl Layer {
    /// The layer `id` holding `document`.
    #[must_use]
    pub fn new(id: impl Into<String>, document: Value) -> Self {
        Self {
            id: id.into(),
            document,
        }
    }
}

/// Everything a store holds now, from [`ConfigStore::read`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Snapshot {
    /// What [`ConfigStore::revision`] answers while nothing changes.
    pub revision: String,
    /// Each layer, lowest first.
    pub layers: Vec<Layer>,
}

impl Snapshot {
    /// These layers at this revision.
    #[must_use]
    pub fn new(revision: impl Into<String>, layers: Vec<Layer>) -> Self {
        Self {
            revision: revision.into(),
            layers,
        }
    }
}

/// A path in the configuration: `["host", "agents", "claude"]`.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Key {
    /// The keys from the top, at least one.
    pub path: Vec<String>,
}

impl Key {
    /// This path.
    #[must_use]
    pub fn new<S: Into<String>>(path: impl IntoIterator<Item = S>) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
        }
    }
}

/// What a path holds, from [`ConfigStore::get`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Held {
    /// The value of the highest layer that names the path (a table is the highest layer's table whole), or absent.
    pub value: Option<Value>,
    /// The id of the layer it comes from, or absent when no layer names it.
    pub layer: Option<String>,
}

impl Held {
    /// `value`, from the layer `layer`.
    #[must_use]
    pub fn of(value: Value, layer: impl Into<String>) -> Self {
        Self {
            value: Some(value),
            layer: Some(layer.into()),
        }
    }

    /// Nothing held.
    #[must_use]
    pub fn none() -> Self {
        Self {
            value: None,
            layer: None,
        }
    }
}

/// One change: `value` is set at `path` in the writable layer (tables are made on the way), or, absent, the path is removed.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Op {
    /// The keys from the top, at least one.
    pub path: Vec<String>,
    /// The value, or absent to remove.
    pub value: Option<Value>,
}

impl Op {
    /// Sets `value` at `path`.
    #[must_use]
    pub fn set<S: Into<String>>(path: impl IntoIterator<Item = S>, value: Value) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
            value: Some(value),
        }
    }

    /// Removes `path`.
    #[must_use]
    pub fn remove<S: Into<String>>(path: impl IntoIterator<Item = S>) -> Self {
        Self {
            path: path.into_iter().map(Into::into).collect(),
            value: None,
        }
    }
}

/// A change to ask [`ConfigStore::change`] for: every op, or none.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Changes {
    /// The ops, in order.
    pub ops: Vec<Op>,
    /// The revision these ops were made from: the change is refused when the store is at another now, so a writer never overwrites
    /// what changed after it read.
    pub expect_revision: Option<String>,
    /// Answer what the writable layer would hold and write nothing.
    #[map(default)]
    pub check_only: bool,
}

impl Changes {
    /// These ops, written.
    #[must_use]
    pub fn of(ops: Vec<Op>) -> Self {
        Self {
            ops,
            expect_revision: None,
            check_only: false,
        }
    }
}

/// What [`ConfigStore::change`] did.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq)]
#[map(crate = ::coqui_acp_api::guatiao, deny_unknown_fields)]
#[non_exhaustive]
pub struct Changed {
    /// The store's revision after the change (the one it had, for a check).
    pub revision: String,
    /// What the writable layer holds after the change, or would hold for a check: the host reads it before it lets a change stand.
    pub document: Value,
}

impl Changed {
    /// A change that left the store at `revision`, with `document` in the writable layer.
    #[must_use]
    pub fn new(revision: impl Into<String>, document: Value) -> Self {
        Self {
            revision: revision.into(),
            document,
        }
    }
}

/// Where the configuration lives. One instance is one store.
///
/// A path is refused, with `BAD_VALUE`, when it is empty, when a name on it is a value and not a table, or when a lower layer names
/// the path and so the writable layer cannot take it out (a removal reaches only the writable layer: what a lower layer holds is
/// shown, never silently hidden). A store that cannot be reached answers `GONE` ([`crate::status::unavailable`]); one asked for
/// what it cannot do answers `UNSUPPORTED` ([`crate::status::unsupported`]).
#[coqui_acp_api::guatiao::kind(name = "config_store", crate = ::coqui_acp_api::guatiao)]
pub trait ConfigStore: Send + Sync {
    /// What it is and what layers it has. Cheap, and the same on every call.
    fn describe(&self) -> Result<StoreInfo, ProviderError>;

    /// Every layer's tables now.
    fn read(&self) -> Result<Snapshot, ProviderError>;

    /// What one path holds, merged over the layers with the host's rules, and the layer it comes from.
    fn get(&self, key: Key) -> Result<Held, ProviderError>;

    /// Makes `changes` in the writable layer, all or none, keeping what a person wrote that the change does not touch (a file keeps
    /// its comments and its order). `BUSY` ([`crate::status::changed`]) when `expect_revision` is not the store's.
    fn change(&self, changes: Changes) -> Result<Changed, ProviderError>;

    /// A token that differs whenever anything in the store did: the change notification. A host that holds a copy reads again when
    /// it moves.
    fn revision(&self) -> Result<String, ProviderError>;
}

/// Why `path` cannot be a path in the configuration, or `None` when it can.
#[must_use]
pub fn path_problem(path: &[String]) -> Option<String> {
    if path.is_empty() {
        return Some(String::from("a path names at least one key"));
    }
    path.iter()
        .any(|key| key.is_empty() || key.chars().any(char::is_control))
        .then(|| String::from("a key of a path is empty or holds a control character"))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_description_needs_exactly_one_writable_layer() {
        let mut own = LayerInfo::new("own", "yours");
        own.writable = true;
        let low = LayerInfo::new("installed", "installed");
        assert_eq!(
            StoreInfo::new("canoa.cfg", alloc::vec![low.clone(), own.clone()]).problem(),
            None
        );
        assert!(
            StoreInfo::new("canoa.cfg", alloc::vec![low.clone()])
                .problem()
                .is_some()
        );
        let mut two = own.clone();
        two.id = "two".into();
        assert!(
            StoreInfo::new("canoa.cfg", alloc::vec![own.clone(), two])
                .problem()
                .is_some()
        );
        assert!(
            StoreInfo::new("canoa.cfg", alloc::vec![own.clone(), own])
                .problem()
                .is_some()
        );
    }

    #[test]
    fn a_path_names_at_least_one_key() {
        assert!(path_problem(&[]).is_some());
        assert!(path_problem(&["".into()]).is_some());
        assert!(path_problem(&["host".into(), "agents".into()]).is_none());
    }
}

pub mod contract;
mod memory;

pub use memory::MemoryStore;

/// A JSON value as the Guatiao value a store's models hold.
///
/// # Panics
///
/// Never: JSON has nothing a Guatiao value cannot hold.
#[must_use]
pub fn value_of(json: &serde_json::Value) -> Value {
    guatiao_serde::from_serde(json.clone()).expect("JSON is a value")
}

/// A Guatiao value as JSON, or `None` for one JSON cannot hold (bytes).
#[must_use]
pub fn json_of(value: &Value) -> Option<serde_json::Value> {
    guatiao_serde::adapt::from_value::<serde_json::Value>(value).ok()
}
