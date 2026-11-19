//! The description of the kinds a library written in Python or JavaScript may serve (`agent_vendor`, `store_extension`,
//! `secret_provider`, `profile_item`, `catalog_source`, `settings_section` and `panel_tab`), and their models' schemas: what such a
//! library is built against. Regenerate the files with
//! `CANOA_WRITE_KINDS=1 cargo test -p canoa --features store,vendor,secret,profile-item,settings-section,panel-tab --test script_kinds`.
#![cfg(all(
    feature = "store",
    feature = "vendor",
    feature = "secret",
    feature = "profile-item",
    feature = "settings-section",
    feature = "panel-tab"
))]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::panel_tab::{PanelTabVtable, Shows, TabInfo, TailRequest};
use canoa::profile_item::{
    ApplyRequest, ClientCertificateHost, ItemEffect, ItemInfo, NetworkEffect, ProfileItemVtable,
    Setting, Variable,
};
use canoa::secret_provider::{Held, Keeping, Named, Names, ProviderInfo, SecretProviderVtable};
use canoa::settings_section::{SectionInfo, SettingsSectionVtable, Stored};
use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtensionVtable};
use canoa::store::source::{CatalogSourceProviderVtable, ReadRequest, SourceInfo};
use canoa::vendor::{
    AccessTarget, AgentVendorVtable, DeliveredFile, EntrySettingsRequest, GatewayRoute,
    HistoryAnswer, HistoryRequest, HookCommand, HookSupport, HookSupportRequest, HooksDelivery,
    HooksRequest, LoginFile, ModeTitle, PluginDelivery, SessionFile, SessionListing,
    SessionsRequest, ShellAdvice, ShellRequest, SkillsDelivery, SkillsRequest, StandardTitle,
    Variable as VendorVariable, VendorInfo,
};
use coqui_acp_api::guatiao::Schema;
use coqui_acp_api::{ProcessSpawnerVtable, SpawnedProcessVtable};
use guatiao_serde::Serializable;
use serde_json::Value as Json;

fn file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn pretty(value: &Json) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("written");
    text.push('\n');
    text
}

fn committed(path: &str, fresh: &Json) {
    let path = file(path);
    if std::env::var_os("CANOA_WRITE_KINDS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("directory");
        std::fs::write(&path, pretty(fresh)).expect("the file is written");
    }
    let committed = std::fs::read_to_string(&path).expect("the file is committed");
    // Compared as documents: another crate's `preserve_order` in the build changes the order a JSON object is written in.
    let committed: Json = serde_json::from_str(&committed).expect("the committed file is JSON");
    assert!(&committed == fresh, "regenerate {}", path.display());
}

fn schema_of<T: Schema>() -> Json {
    let value = T::schema().expect("a model describes itself");
    serde_json::to_value(Serializable::from(&value)).expect("a schema is JSON")
}

fn schemas(models: Vec<(&str, Json)>) -> Json {
    Json::Object(
        models
            .into_iter()
            .map(|(name, schema)| (name.to_owned(), schema))
            .collect(),
    )
}

fn document(description: &str) -> Json {
    serde_json::from_str(&format!("{{\"kinds\":[{description}]}}"))
        .expect("a kind's description is JSON")
}

#[test]
fn the_committed_vendor_description_is_what_this_build_describes() {
    committed(
        "include/canoa.vendor.kinds.json",
        &document(&AgentVendorVtable::description()),
    );
}

#[test]
fn the_committed_vendor_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.vendor.schemas.json",
        &schemas(vec![
            ("VendorInfo", schema_of::<VendorInfo>()),
            ("StandardTitle", schema_of::<StandardTitle>()),
            ("ModeTitle", schema_of::<ModeTitle>()),
            ("Variable", schema_of::<VendorVariable>()),
            ("LoginFile", schema_of::<LoginFile>()),
            ("ShellRequest", schema_of::<ShellRequest>()),
            ("ShellAdvice", schema_of::<ShellAdvice>()),
            ("SessionsRequest", schema_of::<SessionsRequest>()),
            ("SessionFile", schema_of::<SessionFile>()),
            ("SessionListing", schema_of::<SessionListing>()),
            ("HistoryRequest", schema_of::<HistoryRequest>()),
            ("HistoryAnswer", schema_of::<HistoryAnswer>()),
            ("DeliveredFile", schema_of::<DeliveredFile>()),
            ("PluginDelivery", schema_of::<PluginDelivery>()),
            ("SkillsRequest", schema_of::<SkillsRequest>()),
            ("SkillsDelivery", schema_of::<SkillsDelivery>()),
            ("HookCommand", schema_of::<HookCommand>()),
            ("HooksRequest", schema_of::<HooksRequest>()),
            ("HooksDelivery", schema_of::<HooksDelivery>()),
            ("HookSupportRequest", schema_of::<HookSupportRequest>()),
            ("HookSupport", schema_of::<HookSupport>()),
            ("AccessTarget", schema_of::<AccessTarget>()),
            ("GatewayRoute", schema_of::<GatewayRoute>()),
            ("EntrySettingsRequest", schema_of::<EntrySettingsRequest>()),
        ]),
    );
}

#[test]
fn the_committed_extension_description_is_what_this_build_describes() {
    committed(
        "include/canoa.store_extension.kinds.json",
        &document(&StoreExtensionVtable::description()),
    );
}

#[test]
fn the_committed_extension_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.store_extension.schemas.json",
        &schemas(vec![
            ("ExtensionInfo", schema_of::<ExtensionInfo>()),
            ("ExtensionType", schema_of::<ExtensionType>()),
        ]),
    );
}

#[test]
fn the_committed_secret_description_is_what_this_build_describes() {
    // The spawner every call but `describe` is handed is described beside it, as the other kinds that name it do.
    committed(
        "include/canoa.secret.kinds.json",
        &document(&format!(
            "{},{},{}",
            SecretProviderVtable::description(),
            ProcessSpawnerVtable::description(),
            SpawnedProcessVtable::description()
        )),
    );
}

#[test]
fn the_committed_secret_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.secret.schemas.json",
        &schemas(vec![
            ("ProviderInfo", schema_of::<ProviderInfo>()),
            ("Named", schema_of::<Named>()),
            ("Keeping", schema_of::<Keeping>()),
            ("Held", schema_of::<Held>()),
            ("Names", schema_of::<Names>()),
        ]),
    );
}

#[test]
fn the_committed_profile_item_description_is_what_this_build_describes() {
    committed(
        "include/canoa.profile_item.kinds.json",
        &document(&ProfileItemVtable::description()),
    );
}

#[test]
fn the_committed_profile_item_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.profile_item.schemas.json",
        &schemas(vec![
            ("ItemInfo", schema_of::<ItemInfo>()),
            ("ApplyRequest", schema_of::<ApplyRequest>()),
            ("ItemEffect", schema_of::<ItemEffect>()),
            ("Variable", schema_of::<Variable>()),
            ("Setting", schema_of::<Setting>()),
            ("NetworkEffect", schema_of::<NetworkEffect>()),
            (
                "ClientCertificateHost",
                schema_of::<ClientCertificateHost>(),
            ),
        ]),
    );
}

#[test]
fn the_committed_catalog_source_description_is_what_this_build_describes() {
    // The spawner a source's `read` is handed is described beside it, as the other kinds that name it do.
    committed(
        "include/canoa.catalog_source.kinds.json",
        &document(&format!(
            "{},{},{}",
            CatalogSourceProviderVtable::description(),
            ProcessSpawnerVtable::description(),
            SpawnedProcessVtable::description()
        )),
    );
}

#[test]
fn the_committed_catalog_source_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.catalog_source.schemas.json",
        &schemas(vec![
            ("SourceInfo", schema_of::<SourceInfo>()),
            ("ReadRequest", schema_of::<ReadRequest>()),
        ]),
    );
}

#[test]
fn the_committed_settings_section_description_is_what_this_build_describes() {
    committed(
        "include/canoa.settings_section.kinds.json",
        &document(&SettingsSectionVtable::description()),
    );
}

#[test]
fn the_committed_settings_section_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.settings_section.schemas.json",
        &schemas(vec![
            ("SectionInfo", schema_of::<SectionInfo>()),
            ("Stored", schema_of::<Stored>()),
        ]),
    );
}

#[test]
fn the_committed_panel_tab_description_is_what_this_build_describes() {
    committed(
        "include/canoa.panel_tab.kinds.json",
        &document(&PanelTabVtable::description()),
    );
}

#[test]
fn the_committed_panel_tab_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.panel_tab.schemas.json",
        &schemas(vec![
            ("TabInfo", schema_of::<TabInfo>()),
            ("TailRequest", schema_of::<TailRequest>()),
            ("Shows", schema_of::<Shows>()),
        ]),
    );
}
