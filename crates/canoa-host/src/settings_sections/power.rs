//! The Power group of General: whether the host keeps this computer awake while an agent turn runs. The application draws it from
//! the form this provider describes, and its value is `[settings.power] keep_awake` in canoa.cfg, which the host reads when it starts.

use canoa::settings_section::{SectionInfo, SettingsSection};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{FromValue, Map, Provider, Schema, ToValue, Value};
use guatiao_intake::Form;

/// What the Power group holds.
#[derive(ToValue, FromValue, Schema, Form, Debug, Clone)]
#[allow(dead_code, reason = "only its description is read")]
pub(crate) struct PowerValues {
    /// Stops this computer from idling to sleep while an agent turn runs, and lets it sleep again when the turn ends. Takes effect
    /// when the host starts.
    #[map(default)]
    #[schema(title = "Keep this computer awake while a turn runs")]
    pub(crate) keep_awake: bool,
}

/// A group inside the General section, kept in canoa.cfg.
#[derive(Default, Debug, Provider)]
#[provider(SettingsSection, id = "power_section", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct PowerSection;

impl SettingsSection for PowerSection {
    fn describe(&self) -> Result<SectionInfo, ProviderError> {
        let mut info = SectionInfo::new("power", "Power");
        info.under = Some("general".to_owned());
        info.keywords = Some("sleep idle awake suspend caffeinate inhibit".to_owned());
        info.taking::<PowerValues>()
    }

    fn read(&self) -> Result<Value, ProviderError> {
        Ok(Value::from(Map::new()))
    }

    fn write(&self, _values: Value) -> Result<(), ProviderError> {
        Ok(())
    }
}

mod library {
    use super::PowerSection;

    coqui_acp_api::guatiao::local_providers!(
        id = "canoa-host-settings",
        version = env!("CARGO_PKG_VERSION"),
        providers = [PowerSection]
    );
}

/// The registry of the sections the host offers beyond the application's own pages.
pub(super) fn registry() -> Result<Registry, String> {
    let mut registry = Registry::new(canoa::brand::PROGRAM, env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa-host-settings", library::library)
        .map_err(|error| format!("canoa-host-settings: {error}"))?;
    if loading.loaded().is_none() {
        return Err("canoa-host-settings was not accepted".to_owned());
    }
    Ok(registry)
}
