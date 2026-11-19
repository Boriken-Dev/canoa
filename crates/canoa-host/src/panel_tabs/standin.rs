//! A stand-in extension: a library of its own that offers two kinds of tab, as an extension of canoa.cfg would.

use canoa::panel_tab::{PanelTab, Shows, TabInfo, TailRequest};
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{ProviderError, Registry};

use super::TabCatalog;

/// A shell of its own choice.
#[derive(Default, Debug, Provider)]
#[provider(PanelTab, id = "dev_shell", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct DevShell;

impl PanelTab for DevShell {
    fn describe(&self) -> Result<TabInfo, ProviderError> {
        let mut info = TabInfo::new("devshell", "Dev shell", Shows::Terminal);
        info.shell = Some("git-bash".to_owned());
        info.icon = Some("code".to_owned());
        Ok(info)
    }

    fn tail(&self, _request: TailRequest) -> Result<String, ProviderError> {
        Ok(String::new())
    }
}

/// A log of its own.
#[derive(Default, Debug, Provider)]
#[provider(PanelTab, id = "build_log", crate = ::coqui_acp_api::guatiao)]
pub(crate) struct BuildLog;

impl PanelTab for BuildLog {
    fn describe(&self) -> Result<TabInfo, ProviderError> {
        Ok(TabInfo::new("buildlog", "Build log", Shows::Log))
    }

    fn tail(&self, request: TailRequest) -> Result<String, ProviderError> {
        let lines: Vec<String> = (1..=3).map(|n| format!("step {n}")).collect();
        let from = lines.len().saturating_sub(request.lines as usize);
        Ok(lines[from..]
            .iter()
            .map(|line| format!("{line}\n"))
            .collect())
    }
}

mod library {
    use super::{BuildLog, DevShell};

    coqui_acp_api::guatiao::local_providers!(
        id = "builds",
        version = "1.0.0",
        providers = [DevShell, BuildLog]
    );
}

/// The registry of the stand-in extension.
pub(crate) fn registry() -> Registry {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    let loading = registry
        .register_local("builds", library::library)
        .expect("registers");
    assert!(loading.loaded().is_some(), "the stand-in is accepted");
    registry
}

/// The built-in kinds and the stand-in extension's.
pub(crate) fn with_extension() -> TabCatalog {
    TabCatalog::from_registries(vec![("builds".to_owned(), registry())])
}
