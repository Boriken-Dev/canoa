//! The profile items this host offers: each is a provider of the `profile_item` kind, the ones Canoa carries (the model, the effort,
//! the mode, the starting instructions, the access) and the bundled ones (a proxy, an identity, written as scripts) registered the way an
//! extension's are, and those the libraries of canoa.cfg's entries and extensions add. Nothing here names an item the host does not find in a
//! registry: a profile's values for an item no provider offers are kept and reported as not applied.
//!
//! A profile holds, for each item, a JSON object of the values the person gave the members of its schema. A session starts by asking each
//! item for its effect ([`ItemEffect`]): the host then applies the environment, the arguments, the values of the session's own
//! settings and the network the effects ask for, and reports each item and what it became.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::collections::BTreeMap;
use std::sync::{Arc, Mutex, PoisonError};

use canoa::profile_item::{
    ApplyRequest, ItemEffect, ItemInfo, NetworkEffect, ProfileItem, Setting, Variable,
    bad_value_error,
};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, ToValue};
use guatiao_intake::Form;
use serde_json::{Map, Value, json};

use super::{AccessSetting, Context, LayerKind, Profile};
use crate::config::Config;

type Library = fn(
    coqui_acp_api::guatiao::library::Host,
) -> Option<coqui_acp_api::guatiao::library::kind::LibraryParts>;

#[cfg(test)]
pub(crate) mod standin;
#[cfg(test)]
mod tests;

/// The origin of the items Canoa itself carries.
pub(crate) const BUILT_IN: &str = "built-in";
/// The origin of the items that ship with the host as extensions.
pub(crate) const BUNDLED: &str = "bundled";

/// The ids of the items that are the session's own settings, kept in a profile under their own names (`model = "..."`,
/// `access = { ... }`) instead of under `items`.
pub(crate) const SETTING_IDS: [&str; 5] = ["model", "effort", "mode", "context", "access"];

/// The most values one item may hold, so a profile cannot make a session's setup large.
const MAX_VALUES_BYTES: usize = 64 * 1024;

fn settings_of(key: &str, request: &ApplyRequest, note: &str) -> Result<ItemEffect, ProviderError> {
    let values = values_of(request)?;
    let mut effect = ItemEffect::none();
    if let Some(value) = values.get(key).and_then(Value::as_str) {
        effect.settings.push(Setting::new(key, value));
        effect.notes.push(note.to_owned());
    }
    Ok(effect)
}

/// The JSON object of `request`'s values.
pub(crate) fn values_of(request: &ApplyRequest) -> Result<Map<String, Value>, ProviderError> {
    match serde_json::from_str::<Value>(&request.values) {
        Ok(Value::Object(values)) => Ok(values),
        _ => Err(bad_value_error("the values are a JSON object")),
    }
}

macro_rules! scalar_item {
    ($(#[$doc:meta])* $name:ident, $values:ident, $field:ident, $provider:literal, $id:literal, $title:literal, $help:literal, $label:literal, $note:literal) => {
        /// What the item holds: one text.
        #[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
        #[allow(dead_code, reason = "only its description is read")]
        pub struct $values {
            #[doc = $help]
            #[schema(title = $label)]
            pub $field: String,
        }

        $(#[$doc])*
        #[derive(Default, Provider)]
        #[provider(ProfileItem, id = $provider, crate = ::coqui_acp_api::guatiao)]
        pub struct $name;

        impl ProfileItem for $name {
            fn describe(&self) -> Result<ItemInfo, ProviderError> {
                let mut info = ItemInfo::new($id, $title);
                info.help = Some($help.to_owned());
                info.taking::<$values>()
            }

            fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError> {
                settings_of($id, &request, $note)
            }
        }
    };
}

scalar_item!(
    /// The model a session starts with.
    ModelItem, ModelValues, model, "model_item", "model", "Model",
    "The model the agent starts with; the agent's own when empty.", "Model", "sets the model the agent starts with"
);
scalar_item!(
    /// The effort a session starts with.
    EffortItem, EffortValues, effort, "effort_item", "effort", "Effort",
    "The effort, or thought level, the agent starts with.", "Effort", "sets the effort the agent starts with"
);
scalar_item!(
    /// The mode a session starts in.
    ModeItem, ModeValues, mode, "mode_item", "mode", "Mode",
    "The mode the agent starts in.", "Mode", "sets the mode the agent starts in"
);

/// What the starting instructions item holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "only its description is read")]
pub struct ContextValues {
    /// Text added to the session's initial context after every other layer's.
    #[schema(title = "Starting instructions")]
    #[form(widget = "textarea")]
    pub context: String,
}

/// The starting instructions.
#[derive(Default, Provider)]
#[provider(ProfileItem, id = "context_item", crate = ::coqui_acp_api::guatiao)]
pub struct ContextItem;

impl ProfileItem for ContextItem {
    fn describe(&self) -> Result<ItemInfo, ProviderError> {
        let mut info = ItemInfo::new("context", "Starting instructions");
        info.help = Some(
            "Text added to the session's initial context after every other layer's.".to_owned(),
        );
        info.taking::<ContextValues>()
    }

    fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError> {
        settings_of(
            "context",
            &request,
            "adds its text to the session's initial context",
        )
    }
}

/// How far a command the agent runs may reach.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(rename_all = "kebab-case")]
pub enum Sandbox {
    /// The person's own reach.
    None,
    /// Write only in the session's folders.
    Workspace,
    /// Change nothing.
    ReadOnly,
}

/// What is asked before the agent acts.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(rename_all = "snake_case")]
pub enum Approvals {
    /// Everything first.
    Ask,
    /// Before changes.
    Edits,
    /// Nothing.
    Never,
}

/// What the access item holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone, PartialEq, Eq)]
#[allow(dead_code, reason = "only its description is read")]
pub struct AccessValues {
    /// none: the person's own reach. workspace: write only in the session's folders. read-only: change nothing.
    #[schema(title = "Sandbox")]
    pub sandbox: Option<Sandbox>,
    /// Whether the agent's commands may use the network.
    #[schema(title = "Network")]
    pub network: Option<bool>,
    /// ask: everything first. edits: ask before changes. never: ask nothing.
    #[schema(title = "Approvals")]
    pub approvals: Option<Approvals>,
}

/// The access every agent is run with.
#[derive(Default, Provider)]
#[provider(ProfileItem, id = "access_item", crate = ::coqui_acp_api::guatiao)]
pub struct AccessItem;

impl ProfileItem for AccessItem {
    fn describe(&self) -> Result<ItemInfo, ProviderError> {
        let mut info = ItemInfo::new("access", "Access");
        info.help = Some(
            "What a command the agent runs may reach, whether it may use the network, and what is asked first: the same for every agent."
                .to_owned(),
        );
        info.taking::<AccessValues>()
    }

    fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError> {
        let values = values_of(&request)?;
        let mut effect = ItemEffect::none();
        for part in ["sandbox", "network", "approvals"] {
            match values.get(part) {
                Some(Value::String(text)) => effect
                    .settings
                    .push(Setting::new(format!("access.{part}"), text.clone())),
                Some(Value::Bool(yes)) => effect
                    .settings
                    .push(Setting::new(format!("access.{part}"), yes.to_string())),
                _ => {}
            }
        }
        if !effect.settings.is_empty() {
            effect
                .notes
                .push("sets the access every agent is run with".to_owned());
        }
        Ok(effect)
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-profile-items",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ModelItem, EffortItem, ModeItem, ContextItem, AccessItem]
);

/// A registry holding the library `library` as `name`.
fn registry_of(name: &str, library: Library) -> Result<Registry, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local(name, library)
        .map_err(|error| format!("{name}: {error}"))?;
    if loading.loaded().is_none() {
        return Err(format!("{name} was not accepted"));
    }
    Ok(registry)
}

/// One item a registry offers.
struct Entry {
    origin: String,
    registry: Arc<Registry>,
    key: String,
    info: ItemInfo,
}

impl Entry {
    /// Runs `body` with the provider's instance.
    fn with<T>(
        &self,
        body: impl FnOnce(&dyn ProfileItem) -> Result<T, ProviderError>,
    ) -> Result<T, String> {
        let offer = self
            .registry
            .offer::<dyn ProfileItem>(&self.key)
            .ok_or_else(|| format!("the provider {} is gone", self.key))?
            .map_err(|_| format!("{} is not a profile_item", self.key))?;
        let instance = offer
            .default_instance()
            .ok_or_else(|| format!("{} needs settings", self.key))?;
        body(instance).map_err(|error| error.to_string())
    }
}

/// The items this host offers, from every registry it was given.
pub(crate) struct ItemCatalog {
    entries: Vec<Entry>,
}

impl core::fmt::Debug for ItemCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| &entry.info.id))
            .finish()
    }
}

/// An item as the catalog lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Listed {
    /// Where its provider comes from: `built-in`, `bundled`, or the name of the entry or extension that loads it.
    pub(crate) origin: String,
    pub(crate) info: ItemInfo,
}

impl Listed {
    /// The item as `profile_items/list` answers.
    pub(crate) fn json(&self) -> Value {
        let described = |value: &Option<_>| match value {
            Some(value) => crate::forms::json_of(value).unwrap_or(Value::Null),
            None => Value::Null,
        };
        json!({
            "id": self.info.id, "title": self.info.title, "help": self.info.help,
            "origin": self.origin,
            "stored": if self.origin == BUILT_IN && SETTING_IDS.contains(&self.info.id.as_str()) { "setting" } else { "item" },
            "harnesses": self.info.harnesses,
            "schema": described(&self.info.schema), "form": described(&self.info.form),
        })
    }
}

impl ItemCatalog {
    /// The built-in and the bundled items alone, made once for each place the bundled files are kept.
    pub(crate) fn built_in() -> Arc<Self> {
        static KEPT: Mutex<Vec<(std::path::PathBuf, Arc<ItemCatalog>)>> = Mutex::new(Vec::new());
        let home = crate::scripts::root();
        let mut kept = KEPT.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((_, made)) = kept.iter().find(|(place, _)| *place == home) {
            return Arc::clone(made);
        }
        let made = Arc::new(Self::from_registries(Vec::new()));
        kept.push((home, Arc::clone(&made)));
        made
    }

    /// The built-in and the bundled items, then those `registries` offer, each registry as its origin names it. An item whose id an
    /// earlier one took, or that cannot be described, is left out and logged.
    pub(crate) fn from_registries(registries: Vec<(String, Registry)>) -> Self {
        let wrapped: Vec<(String, Arc<Registry>)> = registries
            .into_iter()
            .map(|(origin, registry)| (origin, Arc::new(registry)))
            .collect();
        Self::from_arcs(&wrapped)
    }

    /// As [`Self::from_registries`], sharing cached `Arc<Registry>` handles.
    pub(crate) fn from_arcs(registries: &[(String, Arc<Registry>)]) -> Self {
        let mut found: Vec<(String, Arc<Registry>)> = Vec::new();
        match registry_of("canoa-profile-items", library as Library) {
            Ok(registry) => found.push((BUILT_IN.to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("the built-in profile items are not offered: {why}"),
        }
        match super::bundled::registry() {
            Ok(registry) => found.push((BUNDLED.to_owned(), Arc::new(registry))),
            Err(why) => tracing::warn!("the bundled profile items are not offered: {why}"),
        }
        found.extend(registries.iter().cloned());
        let mut entries: Vec<Entry> = Vec::new();
        for (origin, registry) in found {
            let keys: Vec<String> = registry
                .offers::<dyn ProfileItem>()
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn ProfileItem>(&key) else {
                    continue;
                };
                let Some(info) = offer
                    .default_instance()
                    .and_then(|item| item.describe().ok())
                else {
                    tracing::warn!(origin, provider = key, "a profile item cannot be described");
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "a profile item is not offered: {problem}"
                    );
                    continue;
                }
                if entries.iter().any(|each| each.info.id == info.id) {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "the profile item {} is already offered",
                        info.id
                    );
                    continue;
                }
                entries.push(Entry {
                    origin: origin.clone(),
                    registry: Arc::clone(&registry),
                    key,
                    info,
                });
            }
        }
        Self { entries }
    }

    /// The built-in and bundled items, and those the libraries of `config`'s entries and extensions add. Each library is opened as the
    /// host opens it for `providers/list`: an extension whose pin no longer matches is left out.
    pub(crate) fn of(config: &Config) -> Self {
        Self::from_arcs(&crate::control::providers::loaded(config))
    }

    /// The items offered, in the order they were found.
    pub(crate) fn list(&self) -> Vec<Listed> {
        self.entries
            .iter()
            .map(|entry| Listed {
                origin: entry.origin.clone(),
                info: entry.info.clone(),
            })
            .collect()
    }

    /// Applies every item of `values` (the profile's values by item id) for a session on `harness`, in the order the profile holds
    /// them. An item the catalog has no provider for, or that does not apply to the harness, or whose values do not fit its form, or
    /// whose provider refuses, is [`Outcome::Not`] with the reason; the profile keeps its values either way.
    pub(crate) fn apply_all(
        &self,
        values: &indexmap::IndexMap<String, Map<String, Value>>,
        harness: Option<&str>,
    ) -> Vec<Applied> {
        values
            .iter()
            .map(|(id, held)| self.apply(id, harness, held))
            .collect()
    }

    fn apply(&self, id: &str, harness: Option<&str>, values: &Map<String, Value>) -> Applied {
        let Some(entry) = self.entries.iter().find(|entry| entry.info.id == id) else {
            return Applied {
                id: id.to_owned(),
                title: id.to_owned(),
                origin: None,
                outcome: Outcome::Not(
                    "no provider offers this item, so its values are kept and not applied"
                        .to_owned(),
                ),
            };
        };
        let outcome = Self::run(entry, harness, values);
        Applied {
            id: id.to_owned(),
            title: entry.info.title.clone(),
            origin: Some(entry.origin.clone()),
            outcome,
        }
    }

    fn run(entry: &Entry, harness: Option<&str>, values: &Map<String, Value>) -> Outcome {
        let info = &entry.info;
        if !harness.map_or(info.harnesses.is_empty(), |harness| {
            info.applies_to(harness)
        }) {
            return Outcome::Not(format!(
                "it applies to {} only, and this session is not on one",
                info.harnesses.join(", ")
            ));
        }
        if let Err(why) = check(info, values) {
            return Outcome::Not(why);
        }
        let Ok(text) = serde_json::to_string(values) else {
            return Outcome::Not("its values cannot be read".to_owned());
        };
        if text.len() > MAX_VALUES_BYTES {
            return Outcome::Not(format!(
                "its values are longer than {MAX_VALUES_BYTES} bytes"
            ));
        }
        let request = ApplyRequest::new(harness.unwrap_or(""), text);
        match entry.with(|item| item.apply(request)) {
            Ok(effect) => match effect.problem() {
                None => Outcome::Done(effect),
                Some(problem) => Outcome::Not(format!("its provider answered wrongly: {problem}")),
            },
            Err(why) => Outcome::Not(format!("its provider refused: {why}")),
        }
    }
}

/// Whether `values` fit the schema `info` takes: every key a member, every value of its member's type, every required member there.
pub(crate) fn check(info: &ItemInfo, values: &Map<String, Value>) -> Result<(), String> {
    crate::forms::check(info.schema.as_ref(), values).map_err(|why| format!("{}: {why}", info.id))
}

/// What one item became for one session.
#[derive(Debug, Clone, PartialEq)]
pub(crate) struct Applied {
    pub(crate) id: String,
    pub(crate) title: String,
    /// Where its provider is from; `None` when none offers it.
    pub(crate) origin: Option<String>,
    pub(crate) outcome: Outcome,
}

/// The effect of an item, or why it has none.
#[derive(Debug, Clone, PartialEq)]
#[allow(
    clippy::large_enum_variant,
    reason = "a session holds a handful, each read once"
)]
pub(crate) enum Outcome {
    Done(ItemEffect),
    Not(String),
}

impl Applied {
    /// What the item became, in words, with no value in them: the variables by name, the settings by key, the network by mode.
    pub(crate) fn how(&self) -> String {
        let Outcome::Done(effect) = &self.outcome else {
            return String::new();
        };
        let mut parts: Vec<String> = effect.notes.clone();
        if !effect.env.is_empty() {
            let names: Vec<&str> = effect.env.iter().map(|each| each.name.as_str()).collect();
            parts.push(format!("gives the agent's programs {}", names.join(", ")));
        }
        if !effect.args.is_empty() {
            parts.push(format!(
                "adds {} argument(s) to the program's command line",
                effect.args.len()
            ));
        }
        if let Some(network) = &effect.network {
            parts.push(format!(
                "sets how its programs reach the network: {}",
                network.mode
            ));
        }
        if parts.is_empty() {
            "sets nothing for this session".to_owned()
        } else {
            parts.join("; ")
        }
    }
}

/// The session's own layer the settings of `applied` make, and what could not be taken, by key.
pub(crate) fn session_layer(applied: &[Applied]) -> (Profile, Vec<(String, String)>) {
    let mut layer = Profile::default();
    let mut refused = Vec::new();
    for item in applied {
        let Outcome::Done(effect) = &item.outcome else {
            continue;
        };
        for setting in &effect.settings {
            if let Err(why) = take(&mut layer, setting) {
                refused.push((setting.key.clone(), why));
            }
        }
    }
    (layer, refused)
}

fn take(layer: &mut Profile, setting: &Setting) -> Result<(), String> {
    let mut table = toml::Table::new();
    match setting.key.as_str() {
        "model" | "effort" | "mode" => {
            table.insert(
                setting.key.clone(),
                toml::Value::String(setting.value.clone()),
            );
        }
        "context" => {
            layer
                .contexts
                .retain(|each| !matches!(each, Context::Text(_)));
            layer.contexts.push(Context::Text(setting.value.clone()));
            return Ok(());
        }
        "access.sandbox" | "access.approvals" => {
            let part = setting.key.trim_start_matches("access.");
            let mut access = toml::Table::new();
            access.insert(part.to_owned(), toml::Value::String(setting.value.clone()));
            table.insert("access".to_owned(), toml::Value::Table(access));
        }
        "access.network" => {
            let yes = setting
                .value
                .parse::<bool>()
                .map_err(|_| "access.network is true or false".to_owned())?;
            let mut access = toml::Table::new();
            access.insert("network".to_owned(), toml::Value::Boolean(yes));
            table.insert("access".to_owned(), toml::Value::Table(access));
        }
        other => return Err(format!("{other} is not a setting a session has")),
    }
    let own = Profile::from_table(table, LayerKind::Session).map_err(|error| error.to_string())?;
    layer.model = own.model.or(layer.model.take());
    layer.effort = own.effort.or(layer.effort.take());
    layer.mode = own.mode.or(layer.mode.take());
    layer.access = AccessSetting {
        sandbox: own.access.sandbox.or(layer.access.sandbox),
        network: own.access.network.or(layer.access.network),
        approvals: own.access.approvals.or(layer.access.approvals),
    };
    Ok(())
}

/// What the effects of `applied` ask the session's network to be: the last item that asks.
pub(crate) fn network_of(applied: &[Applied]) -> Option<&NetworkEffect> {
    applied.iter().rev().find_map(|item| match &item.outcome {
        Outcome::Done(effect) => effect.network.as_ref(),
        Outcome::Not(_) => None,
    })
}

/// The variables of `applied`, in the order the items were applied; a name an earlier item set keeps its first value.
pub(crate) fn variables_of(applied: &[Applied]) -> Vec<(&str, &Variable)> {
    let mut seen: BTreeMap<String, ()> = BTreeMap::new();
    let mut out = Vec::new();
    for item in applied {
        if let Outcome::Done(effect) = &item.outcome {
            for variable in &effect.env {
                let key = if cfg!(windows) {
                    variable.name.to_ascii_uppercase()
                } else {
                    variable.name.clone()
                };
                if seen.insert(key, ()).is_none() {
                    out.push((item.id.as_str(), variable));
                }
            }
        }
    }
    out
}

/// The items a host offers, held by the profiles it serves: the built-in and bundled items alone until the host has opened its
/// extensions' libraries.
#[derive(Clone, Default)]
pub struct ItemSet(Option<Arc<ItemCatalog>>);

impl core::fmt::Debug for ItemSet {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.write_str("ItemSet")
    }
}

impl PartialEq for ItemSet {
    fn eq(&self, other: &Self) -> bool {
        match (&self.0, &other.0) {
            (None, None) => true,
            (Some(a), Some(b)) => Arc::ptr_eq(a, b),
            _ => false,
        }
    }
}

impl Eq for ItemSet {}

impl ItemSet {
    /// The items `config`'s libraries offer, with the built-in and bundled ones.
    pub(crate) fn of(config: &Config) -> Self {
        Self(Some(Arc::new(ItemCatalog::of(config))))
    }

    /// The items of `catalog`.
    #[cfg(test)]
    pub(crate) fn with(catalog: ItemCatalog) -> Self {
        Self(Some(Arc::new(catalog)))
    }

    /// The catalog these are.
    pub(crate) fn catalog(&self) -> Arc<ItemCatalog> {
        self.0.clone().unwrap_or_else(ItemCatalog::built_in)
    }
}
