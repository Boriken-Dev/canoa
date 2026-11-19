//! The description of the `agent_vendor`, `store_extension` and `secret_provider` kinds, and their models' schemas: what a library
//! written in Python or JavaScript is built against. Regenerate the files with
//! `CANOA_WRITE_KINDS=1 cargo test -p canoa --features store,vendor,secret --test script_kinds`.
#![cfg(all(feature = "store", feature = "vendor", feature = "secret"))]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::secret_provider::{Held, Keeping, Named, Names, ProviderInfo, SecretProviderVtable};
use canoa::store::extension::{ExtensionInfo, ExtensionType, StoreExtensionVtable};
use canoa::vendor::{
    AccessTarget, AgentVendorVtable, DeliveredFile, EntrySettingsRequest, GatewayRoute,
    HistoryAnswer, HistoryRequest, HookCommand, HookSupport, HookSupportRequest, HooksDelivery,
    HooksRequest, LoginFile, ModeTitle, PluginDelivery, SessionFile, SessionListing,
    SessionsRequest, ShellAdvice, ShellRequest, SkillsDelivery, SkillsRequest, StandardTitle,
    Variable, VendorInfo,
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
            ("Variable", schema_of::<Variable>()),
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
