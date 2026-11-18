//! A stand-in extension: a library of its own that offers one item, as an extension of canoa.cfg would.

use canoa::profile_item::{
    ApplyRequest, ItemEffect, ItemInfo, ProfileItem, Setting, Variable, bad_value_error,
};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Secret, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

use super::{ItemCatalog, values_of};

/// A region's tier.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(rename_all = "snake_case")]
pub(crate) enum Tier {
    /// The usual.
    Standard,
    /// The quick one.
    Fast,
}

const fn standard() -> Tier {
    Tier::Standard
}

/// What the region item holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[form(
    section(id = "where", label = "Where"),
    section(id = "access", label = "Access")
)]
#[allow(dead_code, reason = "only its description is read")]
pub(crate) struct RegionValues {
    /// The region's name.
    #[schema(section = "where", title = "Name")]
    #[form(placeholder = "eu-west")]
    pub(crate) name: String,
    /// Hosts the region reaches.
    #[map(default)]
    #[schema(section = "where", title = "Hosts")]
    pub(crate) hosts: Vec<String>,
    /// Which tier it runs.
    #[map(default = standard())]
    #[schema(section = "where", title = "Tier")]
    pub(crate) tier: Tier,
    /// What the region asks for to let a session in.
    #[schema(section = "access", title = "Token")]
    pub(crate) token: Option<Secret<String>>,
}

/// Where the session runs: sets `REGION`, and for the fast tier the model.
#[derive(Default, Debug, Provider)]
#[provider(ProfileItem, id = "region_item", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct RegionItem;

impl ProfileItem for RegionItem {
    fn describe(&self) -> Result<ItemInfo, ProviderError> {
        let mut info = ItemInfo::new("region", "Region");
        info.help = Some("Where the session runs.".to_owned());
        info.harnesses = vec!["claude-code".to_owned()];
        info.taking::<RegionValues>()
    }

    fn apply(&self, request: ApplyRequest) -> Result<ItemEffect, ProviderError> {
        let values = values_of(&request)?;
        let name = values
            .get("name")
            .and_then(Value::as_str)
            .ok_or_else(|| bad_value_error("a region has a name"))?;
        let mut effect = ItemEffect::none();
        effect.env.push(Variable::new("REGION", name));
        if let Some(Value::Array(hosts)) = values.get("hosts") {
            let hosts: Vec<&str> = hosts.iter().filter_map(Value::as_str).collect();
            effect
                .env
                .push(Variable::new("REGION_HOSTS", hosts.join(",")));
        }
        if values.get("tier").and_then(Value::as_str) == Some("fast") {
            effect.settings.push(Setting::new("model", "fast-model"));
        }
        effect.notes.push("puts the session in a region".to_owned());
        Ok(effect)
    }
}

mod library {
    use super::RegionItem;

    coqui_acp_api::guatiao::local_providers!(
        id = "regions",
        version = "1.0.0",
        providers = [RegionItem]
    );
}

/// The registry of the stand-in extension.
pub(crate) fn registry() -> Registry {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("regions", library::library)
        .expect("registers");
    assert!(loading.loaded().is_some(), "the stand-in is accepted");
    registry
}

/// The built-in and bundled items and the stand-in extension's.
pub(crate) fn with_extension() -> ItemCatalog {
    ItemCatalog::from_registries(vec![("regions".to_owned(), registry())])
}
