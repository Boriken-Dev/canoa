//! The descriptions of the kinds a library written in a script may serve, given to every source the host adds.
//!
//! A source that runs a language with no compiled tables (JavaScript, Lua, Rhai, WebAssembly) builds each provider's table from the
//! description of its kind, which only the host has: the document here is made from the very kinds the host is built with, so it
//! is never out of date, and the source reads it from a file the host keeps beside the other temporary files, named by its
//! content so that a host built with other kinds never reads this one's.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths and files are std's"
)]

use std::path::PathBuf;
use std::sync::OnceLock;

use canoa::panel_tab::PanelTabVtable;
use canoa::profile_item::ProfileItemVtable;
use canoa::secret_provider::SecretProviderVtable;
use canoa::settings_section::SettingsSectionVtable;
use canoa::speech::account::AccountSpeechVtable;
use canoa::speech::output::{SpeechOutputVtable, SpeechUtteranceVtable};
use canoa::speech::{SpeechProviderVtable, SpeechSessionVtable};
use canoa::store::extension::StoreExtensionVtable;
use canoa::store::models::ModelStoreVtable;
use canoa::store::source::CatalogSourceProviderVtable;
use canoa::store::{InstallPrompterVtable, InstallToolsVtable, ProgramStoreVtable};
use canoa::usage_reader::{ProcessMembersVtable, UsageReaderVtable, UsageReadingVtable};
use canoa::vendor::AgentVendorVtable;
use coqui_acp_api::{
    AccountKeeperVtable, AccountProviderVtable, AccountWatchVtable, AcpHarnessVtable,
    ChannelSinkVtable, ChannelVtable, LoginPrompterVtable, ProcessSpawnerVtable,
    SpawnedProcessVtable,
};
use coqui_acp_provider::{AgentProviderVtable, AgentSessionVtable};
use coqui_transport_guatiao::{TransportAcceptorVtable, TransportListenerVtable, TransportVtable};

/// Every kind a script library may serve, and each object kind their slots name, once, as a kinds document.
pub(crate) fn document() -> String {
    let kinds = [
        AgentVendorVtable::description(),
        StoreExtensionVtable::description(),
        ModelStoreVtable::description(),
        SecretProviderVtable::description(),
        ProfileItemVtable::description(),
        CatalogSourceProviderVtable::description(),
        UsageReaderVtable::description(),
        UsageReadingVtable::description(),
        ProcessMembersVtable::description(),
        SettingsSectionVtable::description(),
        PanelTabVtable::description(),
        ProgramStoreVtable::description(),
        InstallPrompterVtable::description(),
        InstallToolsVtable::description(),
        AccountSpeechVtable::description(),
        SpeechProviderVtable::description(),
        SpeechSessionVtable::description(),
        SpeechOutputVtable::description(),
        SpeechUtteranceVtable::description(),
        AcpHarnessVtable::description(),
        AccountProviderVtable::description(),
        AccountWatchVtable::description(),
        AccountKeeperVtable::description(),
        LoginPrompterVtable::description(),
        AgentProviderVtable::description(),
        AgentSessionVtable::description(),
        TransportVtable::description(),
        TransportAcceptorVtable::description(),
        TransportListenerVtable::description(),
        ChannelVtable::description(),
        ChannelSinkVtable::description(),
        ProcessSpawnerVtable::description(),
        SpawnedProcessVtable::description(),
    ];
    format!("{{\"kinds\":[{}]}}", kinds.join(","))
}

/// FNV-1a, 64 bits: a name that changes with the content.
fn fingerprint(bytes: &[u8]) -> u64 {
    bytes.iter().fold(0xcbf2_9ce4_8422_2325, |hash, byte| {
        (hash ^ u64::from(*byte)).wrapping_mul(0x0100_0000_01b3)
    })
}

/// The file holding [`document`], written when it is not there with this content.
///
/// # Errors
///
/// Why the file could not be made.
pub(crate) fn file() -> Result<PathBuf, String> {
    static FILE: OnceLock<Result<PathBuf, String>> = OnceLock::new();
    FILE.get_or_init(|| {
        let text = document();
        let directory = std::env::temp_dir().join("canoa-kinds");
        let path = directory.join(format!("kinds-{:016x}.json", fingerprint(text.as_bytes())));
        if std::fs::read(&path).is_ok_and(|held| held == text.as_bytes()) {
            return Ok(path);
        }
        std::fs::create_dir_all(&directory)
            .map_err(|error| format!("{}: {error}", directory.display()))?;
        // Written under a name of its own and moved into place, so a source starting at the same time never reads half of it.
        let partial = directory.join(format!(
            "kinds-{}-{}.part",
            std::process::id(),
            fingerprint(path.as_os_str().as_encoded_bytes())
        ));
        std::fs::write(&partial, &text)
            .map_err(|error| format!("{}: {error}", partial.display()))?;
        std::fs::rename(&partial, &path).map_err(|error| {
            let _ = std::fs::remove_file(&partial);
            format!("{}: {error}", path.display())
        })?;
        Ok(path)
    })
    .clone()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_document_names_every_kind_once_and_the_file_holds_it() {
        let document: serde_json::Value = serde_json::from_str(&document()).expect("JSON");
        let names: Vec<&str> = document["kinds"]
            .as_array()
            .expect("a list")
            .iter()
            .filter_map(|kind| kind["name"].as_str())
            .collect();
        let mut once = names.clone();
        once.sort_unstable();
        once.dedup();
        assert_eq!(once.len(), names.len(), "{names:?}");
        for needed in [
            "agent_vendor",
            "store_extension",
            "secret_provider",
            "profile_item",
            "catalog_source",
            "usage_reader",
            "settings_section",
            "panel_tab",
            "acp_harness",
            "transport",
        ] {
            assert!(names.contains(&needed), "{needed} in {names:?}");
        }
        let path = file().expect("written");
        assert_eq!(
            std::fs::read_to_string(path).expect("read"),
            super::document()
        );
    }
}
