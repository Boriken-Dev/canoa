//! The kinds of tab the application's panel can open, as the providers of this host offer them: each is a provider of the `panel_tab`
//! kind, the ones the host carries (a terminal, its own log) registered the way an extension's are, and those the libraries of
//! canoa.cfg's entries and extensions add. Nothing here names a kind: the application opens what a provider says of its tab.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the catalog is the host's"
)]

use std::sync::Arc;

use canoa::panel_tab::{PanelTab, Shows, TabInfo, TailRequest};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use serde_json::{Value, json};

use crate::config::Config;

#[cfg(test)]
pub(crate) mod standin;

/// The origin of the kinds the host itself carries.
pub(crate) const BUILT_IN: &str = "built-in";

/// A shell on the host.
#[derive(Default, Provider)]
#[provider(PanelTab, id = "terminal_tab", crate = ::coqui_acp_api::guatiao)]
pub struct TerminalTab;

impl PanelTab for TerminalTab {
    fn describe(&self) -> Result<TabInfo, ProviderError> {
        let mut info = TabInfo::new("terminal", "Terminal", Shows::Terminal);
        info.icon = Some("terminal".to_owned());
        info.help = Some("A shell on this host, in the session's folder.".to_owned());
        Ok(info)
    }

    fn tail(&self, _request: TailRequest) -> Result<String, ProviderError> {
        Ok(String::new())
    }
}

/// The host's own log.
#[derive(Default, Provider)]
#[provider(PanelTab, id = "trace_log_tab", crate = ::coqui_acp_api::guatiao)]
pub struct LogsTab;

impl PanelTab for LogsTab {
    fn describe(&self) -> Result<TabInfo, ProviderError> {
        let mut info = TabInfo::new("logs", "Logs", Shows::Log);
        info.icon = Some("article".to_owned());
        info.help = Some("What this host logged lately.".to_owned());
        Ok(info)
    }

    fn tail(&self, request: TailRequest) -> Result<String, ProviderError> {
        let lines = request.lines.min(canoa::panel_tab::MAX_TAIL_LINES) as usize;
        Ok(crate::host_log::tail(lines))
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-panel-tabs",
    version = env!("CARGO_PKG_VERSION"),
    providers = [TerminalTab, LogsTab]
);

/// One kind a registry offers.
struct Entry {
    origin: String,
    registry: Arc<Registry>,
    key: String,
    info: TabInfo,
}

impl Entry {
    fn with<T>(
        &self,
        body: impl FnOnce(&dyn PanelTab) -> Result<T, ProviderError>,
    ) -> Result<T, String> {
        let offer = self
            .registry
            .offer::<dyn PanelTab>(&self.key)
            .ok_or_else(|| format!("the provider {} is gone", self.key))?
            .map_err(|_| format!("{} is not a panel_tab", self.key))?;
        let instance = offer
            .default_instance()
            .ok_or_else(|| format!("{} needs settings", self.key))?;
        body(instance).map_err(|error| error.to_string())
    }
}

/// The kinds of tab this host offers, from every registry it was given.
pub(crate) struct TabCatalog {
    entries: Vec<Entry>,
}

impl core::fmt::Debug for TabCatalog {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_list()
            .entries(self.entries.iter().map(|entry| &entry.info.id))
            .finish()
    }
}

fn built_in() -> Result<Registry, String> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("canoa-panel-tabs", library)
        .map_err(|error| format!("canoa-panel-tabs: {error}"))?;
    if loading.loaded().is_none() {
        return Err("canoa-panel-tabs was not accepted".to_owned());
    }
    Ok(registry)
}

impl TabCatalog {
    /// The built-in kinds, then those `registries` offer, each registry as its origin names it. A kind whose id an earlier one took,
    /// or that cannot be described, is left out and logged.
    pub(crate) fn from_registries(registries: Vec<(String, Registry)>) -> Self {
        let mut found: Vec<(String, Registry)> = Vec::new();
        match built_in() {
            Ok(registry) => found.push((BUILT_IN.to_owned(), registry)),
            Err(why) => tracing::warn!("the built-in panel tabs are not offered: {why}"),
        }
        found.extend(registries);
        let mut entries: Vec<Entry> = Vec::new();
        for (origin, registry) in found {
            let registry = Arc::new(registry);
            let keys: Vec<String> = registry
                .offers::<dyn PanelTab>()
                .filter_map(|offer| offer.key().map(str::to_owned))
                .collect();
            for key in keys {
                let Some(Ok(offer)) = registry.offer::<dyn PanelTab>(&key) else {
                    continue;
                };
                let Some(info) = offer.default_instance().and_then(|tab| tab.describe().ok())
                else {
                    tracing::warn!(origin, provider = key, "a panel tab cannot be described");
                    continue;
                };
                if let Some(problem) = info.problem() {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "a panel tab is not offered: {problem}"
                    );
                    continue;
                }
                if entries.iter().any(|each| each.info.id == info.id) {
                    tracing::warn!(
                        origin,
                        provider = key,
                        "the panel tab {} is already offered",
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

    /// The built-in kinds and those the libraries of `config`'s entries and extensions add.
    pub(crate) fn of(config: &Config) -> Self {
        Self::from_registries(crate::control::providers::loaded(config))
    }

    /// The kinds as `panel_tabs/list` answers: each with its `id`, `title`, `help`, `icon`, `shows` (`terminal` or `log`), `shell`
    /// and `origin` (`built-in`, or the name of the entry or extension that loads it).
    pub(crate) fn list(&self) -> Value {
        let tabs: Vec<Value> = self
            .entries
            .iter()
            .map(|entry| {
                let info = &entry.info;
                json!({
                    "id": info.id, "title": info.title, "help": info.help, "icon": info.icon,
                    "shows": match info.shows { Shows::Terminal => "terminal", Shows::Log => "log" },
                    "shell": info.shell, "origin": entry.origin,
                })
            })
            .collect();
        json!({ "tabs": tabs })
    }

    /// The latest `lines` lines of the log kind `id`; an error in words for a kind that is none, shows no log, or whose provider
    /// refuses.
    pub(crate) fn tail(&self, id: &str, lines: u32) -> Result<String, String> {
        let entry = self
            .entries
            .iter()
            .find(|entry| entry.info.id == id)
            .ok_or_else(|| format!("this host has no panel tab {id}"))?;
        if entry.info.shows != Shows::Log {
            return Err(format!("the panel tab {id} shows no log"));
        }
        let lines = lines.clamp(1, canoa::panel_tab::MAX_TAIL_LINES);
        entry.with(|tab| tab.tail(TailRequest::new(lines)))
    }
}
