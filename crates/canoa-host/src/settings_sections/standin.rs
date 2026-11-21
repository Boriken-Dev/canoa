//! Stand-in extensions: libraries of their own that offer settings sections, as an extension of canoa.cfg would.

use std::sync::Mutex;

use canoa::settings_section::{SectionInfo, SettingsSection, Stored};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Secret, ToValue};
use guatiao_intake::Form;

use super::SectionCatalog;

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

/// What the region section holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[form(section(id = "where", label = "Where"))]
#[allow(dead_code, reason = "only its description is read")]
pub(crate) struct RegionValues {
    /// The region's name.
    #[schema(section = "where", title = "Name")]
    #[form(placeholder = "eu-west")]
    pub(crate) name: String,
    /// Which tier it runs.
    #[map(default = standard())]
    #[schema(section = "where", title = "Tier")]
    pub(crate) tier: Tier,
    /// What the region asks for to let a session in.
    #[schema(section = "where", title = "Token")]
    pub(crate) token: Option<Secret<String>>,
}

/// A section of its own, kept in canoa.cfg.
#[derive(Default, Debug, Provider)]
#[provider(SettingsSection, id = "region_section", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct RegionSection;

impl SettingsSection for RegionSection {
    fn describe(&self) -> Result<SectionInfo, ProviderError> {
        let mut info = SectionInfo::new("region", "Region");
        info.help = Some("Where the extension runs.".to_owned());
        info.icon = Some("map".to_owned());
        info.group = Some("Regions".to_owned());
        info.keywords = Some("tier cloud".to_owned());
        info.taking::<RegionValues>()
    }

    fn read(&self) -> Result<coqui_acp_api::guatiao::Value, ProviderError> {
        Ok(coqui_acp_api::guatiao::Value::from(
            coqui_acp_api::guatiao::Map::new(),
        ))
    }

    fn write(&self, _values: coqui_acp_api::guatiao::Value) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// What the cache group holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[allow(dead_code, reason = "only its description is read")]
pub(crate) struct CacheValues {
    /// Entries kept.
    #[schema(title = "Entries")]
    pub(crate) entries: Option<u32>,
}

/// The values the cache group keeps with the provider.
static CACHE: Mutex<String> = Mutex::new(String::new());

/// A group inside the General section, kept by the provider itself.
#[derive(Default, Debug, Provider)]
#[provider(SettingsSection, id = "cache_section", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct CacheSection;

impl SettingsSection for CacheSection {
    fn describe(&self) -> Result<SectionInfo, ProviderError> {
        let mut info = SectionInfo::new("cache", "Cache");
        info.under = Some("general".to_owned());
        info.stored = Stored::Provider;
        info.taking::<CacheValues>()
    }

    fn read(&self) -> Result<coqui_acp_api::guatiao::Value, ProviderError> {
        let held = CACHE.lock().expect("held").clone();
        if held.is_empty() {
            Ok(coqui_acp_api::guatiao::Value::from(
                coqui_acp_api::guatiao::Map::new(),
            ))
        } else {
            let parsed: serde_json::Value = serde_json::from_str(&held)
                .unwrap_or(serde_json::Value::Object(Default::default()));
            Ok(guatiao_serde::from_serde(parsed).expect("value"))
        }
    }

    fn write(&self, values: coqui_acp_api::guatiao::Value) -> Result<(), ProviderError> {
        let json: serde_json::Value = guatiao_serde::adapt::from_value(&values).expect("json");
        *CACHE.lock().expect("held") = json.to_string();
        Ok(())
    }
}

mod library {
    use super::{CacheSection, RegionSection};

    coqui_acp_api::guatiao::local_providers!(
        id = "regions",
        version = "1.0.0",
        providers = [RegionSection, CacheSection]
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

/// The sections of the stand-in extension.
pub(crate) fn with_extension() -> SectionCatalog {
    SectionCatalog::from_registries(vec![("regions".to_owned(), registry())])
}
