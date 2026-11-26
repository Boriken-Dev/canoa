//! The sections of Settings the application draws itself, offered as providers of the `settings_section` kind the way an extension's
//! are: the application learns that General, Voice or Accounts exist from the host's list, and picks the widget for each by its id.
//! A section here has no form of its own, so nothing is asked of its provider but what it is called and where it sits.

use canoa::settings_section::{SectionInfo, SettingsSection};
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{Map, Provider, Value};

/// A section the application draws, with the heading it sits under and the words Settings' search finds it by.
macro_rules! built_in_sections {
    ($($provider:ident($id:literal, $title:literal, $group:literal, $icon:literal, $keywords:literal);)+) => {
        $(
            #[derive(Default, Provider)]
            #[provider(SettingsSection, id = $id, crate = ::coqui_acp_api::guatiao)]
            pub struct $provider;

            impl SettingsSection for $provider {
                fn describe(&self) -> Result<SectionInfo, ProviderError> {
                    let mut info = SectionInfo::new($id, $title);
                    info.group = Some($group.to_owned());
                    info.icon = Some($icon.to_owned());
                    info.keywords = Some(
                        $keywords
                            .replace("{config_file}", canoa::brand::CONFIG_FILE)
                            .replace("{program}", canoa::brand::PROGRAM),
                    );
                    Ok(info)
                }

                fn read(&self) -> Result<Value, ProviderError> {
                    Ok(Value::from(Map::new()))
                }

                fn write(&self, _values: Value) -> Result<(), ProviderError> {
                    Ok(())
                }
            }
        )+

        /// The sections in the order Settings lists them: a registry offers its providers by name, which is no order for people.
        pub(super) const ORDER: &[&str] = &[$($id),+];

        coqui_acp_api::guatiao::local_providers!(
            id = "canoa-settings-sections",
            version = env!("CARGO_PKG_VERSION"),
            providers = [$($provider),+]
        );
    };
}

built_in_sections! {
    General("general", "General", "App", "tune", "tray window startup network webtransport timeout sessions remembered folder workspace keep history");
    Appearance("appearance", "Appearance", "App", "palette", "look theme brightness dark light detail verbose density preview zoom scale larger smaller text size");
    Network("network", "Network", "App", "alt_route", "proxy route routes pac manual system direct socks http https no proxy bypass login certificate how traffic leaves default account");
    Notifications("notifications", "Notifications", "App", "notifications", "alerts permission question finished browser toast");
    Voice("voice", "Voice", "App", "mic", "dictation speech microphone input device language push to talk transcribe audio read aloud voice speak speaker playback tts text to speech");
    Shortcuts("shortcuts", "Shortcuts", "App", "keyboard", "keyboard keys chords hotkeys");
    Telemetry("telemetry", "Telemetry", "App", "cloud", "telemetry export traces metrics logs opentelemetry otlp share data privacy recorded exporter file");
    Sources("sources", "Sources", "Agents", "hub", "agents providers programs command install update version ready");
    Profiles("profiles", "Profiles", "Agents", "layers", "overlays layers model effort mode skills trusted directories project");
    Mcp("mcp", "MCP servers", "Agents", "extension", "tools servers mcp allowed per agent");
    Permissions("permissions", "Permissions", "Agents", "security", "bypass mode allow once remembered rules protected paths log revoke full access");
    Accounts("accounts", "Accounts", "Agents", "accounts", "login sign in api key subscription credentials add remove several");
    Shells("shells", "Shells", "Agents", "terminal", "bash git bash powershell pwsh wsl msys2 terminal shell command windows");
    Usage("usage", "Usage", "Agents", "speed", "limit warn percent quota");
    Schedules("schedules", "Scheduled tasks", "Agents", "schedule", "schedule timer cron daily weekly once repeat prompt run later automatic background recurring");
    Extensions("extensions", "Libraries", "Extensions", "extension", "extensions libraries library file source host loads load add by hand provider store types restart");
    CatalogSources("catalog_sources", "Catalog sources", "Store", "hub", "catalogs sources models releases default replace add off pin trust verified pinned unverified checksum digest mirror");
    Content("content", "Content", "Store", "hub", "content sources schemes uri file https git data generators cache try read list stat resolve documents ttl provider scheme");
    Advanced("advanced", "Advanced", "Store", "wrench", "configuration {config_file} tables keys web control native network delegate kb history worktrees tools environments openers limits advanced edit add remove");
    Providers("providers", "Providers", "Extensions", "wrench", "providers libraries kinds account harness vendor off disable switch turn disabled host restart");
    About("about", "About", "About", "info", "version {program} boriken licence");
}
