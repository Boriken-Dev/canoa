//! The description of the kinds a library written in Python or JavaScript may serve (`agent_vendor`, `store_extension`, `model_store`,
//! `secret_provider`, `profile_item`, `catalog_source`, `settings_section`, `panel_tab`, `usage_reader`, `model_gateway` and
//! `account_provider`), and their models' schemas: what such a library is built against. Regenerate the files with
//! `CANOA_WRITE_KINDS=1 cargo test -p canoa --features store,vendor,secret,profile-item,settings-section,panel-tab,usage,gateway,environment --test script_kinds`.
#![cfg(all(
    feature = "store",
    feature = "vendor",
    feature = "secret",
    feature = "profile-item",
    feature = "settings-section",
    feature = "panel-tab",
    feature = "usage",
    feature = "gateway",
    feature = "environment",
    feature = "config-store",
    feature = "notifier"
))]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::path::PathBuf;

use canoa::config_store::{
    Changed, Changes, ConfigStoreVtable, Held as ConfigHeld, Key as ConfigKey, Layer, LayerInfo,
    Op, Snapshot, StoreInfo as ConfigStoreInfo,
};
use canoa::environment::{
    Call, CallAnswer, Capability, DirEntry, EnvironmentInfo, EnvironmentVtable, FileContent,
    ListDir, Listing, ReadFile, State as EnvironmentState, WriteFile,
};
use canoa::gateway::{GatewayInfo, GatewayLeaseVtable, GatewayRequest, ModelGatewayVtable};
use canoa::notifier::{
    Action, WithdrawRequest, Delivery, Notice, NotifierInfo, NotifierVtable, Reply, ReplyRequest,
    ReplyTo, SendRequest, SessionRef, Urgency,
};
use canoa::panel_tab::{PanelTabVtable, Shows, TabInfo, TailRequest};
use canoa::profile_item::{
    ApplyRequest, ClientCertificateHost, ItemEffect, ItemInfo, NetworkEffect, ProfileItemVtable,
    Setting, Variable,
};
use canoa::secret_provider::{Held, Keeping, Named, Names, ProviderInfo, SecretProviderVtable};
use canoa::settings_section::{SectionInfo, SettingsSectionVtable, Stored};
use canoa::store::extension::{
    CatalogWords, DefaultCatalog, ExtensionInfo, ExtensionType, StoreExtensionVtable,
};
use canoa::store::models::{
    Fact, ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelRow,
    ModelSettings, ModelStoreInfo, ModelStoreVtable, RowAction, RowEngine, RowVariant, Section,
    ServedModel, ServiceQuery, ServiceReport, SpeechRun, SpeechService, SpeechUse,
};
use canoa::store::source::{
    CatalogSourceProviderVtable, ReadRequest, SourceFetchVtable, SourceInfo,
};
use canoa::usage_reader::{
    Failure, ProcessMembersVtable, ReaderInfo, Reading, Retry, UsageReaderVtable,
    UsageReadingVtable,
};
use canoa::vendor::{
    AccessTarget, AgentVendorVtable, DeliveredFile, EntrySettingsRequest, GatewayRoute,
    HistoryAnswer, HistoryRequest, HookCommand, HookSupport, HookSupportRequest, HookTool,
    HooksDelivery, HooksRequest, LoginFile, ModeTitle, ModeWord, PluginDelivery, SessionFile,
    SessionListing, SessionsRequest, ShellAdvice, ShellRequest, SkillsDelivery, SkillsRequest,
    StandardTitle, ToolHook, Variable as VendorVariable, VendorInfo,
};
use coqui_acp_api::guatiao::Schema;
use coqui_acp_api::{
    AccountKeeperVtable, AccountKindInfo, AccountLoss, AccountModelsQuery, AccountOffer,
    AccountProviderInfo, AccountProviderVtable, AccountRenewal, AccountUsage, AccountUsageQuery,
    AccountWatchVtable, Bucket, EndpointSlot, LoginAnswer, LoginForm, LoginMethodInfo,
    LoginOutcome, LoginPrompterVtable, LoginRequest, ModelInfo, ModelListing, OpenUrl,
    ProcessSpawnerVtable, RenewReport, RenewRequest, SignedInCheck, SignedInQuery, SignedInState,
    SpawnedProcessVtable, UnknownKind, WatchRequest,
};
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
            ("HookTool", schema_of::<HookTool>()),
            ("ModeWord", schema_of::<ModeWord>()),
            ("ToolHook", schema_of::<ToolHook>()),
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
            ("CatalogWords", schema_of::<CatalogWords>()),
            ("DefaultCatalog", schema_of::<DefaultCatalog>()),
        ]),
    );
}

#[test]
fn the_committed_model_store_description_is_what_this_build_describes() {
    // The spawner every call but `describe` is handed is described beside it, as the other kinds that name it do.
    committed(
        "include/canoa.model_store.kinds.json",
        &document(&format!(
            "{},{},{}",
            ModelStoreVtable::description(),
            ProcessSpawnerVtable::description(),
            SpawnedProcessVtable::description()
        )),
    );
}

#[test]
fn the_committed_model_store_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.model_store.schemas.json",
        &schemas(vec![
            ("ModelStoreInfo", schema_of::<ModelStoreInfo>()),
            ("ModelSettings", schema_of::<ModelSettings>()),
            ("SpeechService", schema_of::<SpeechService>()),
            ("ModelQuery", schema_of::<ModelQuery>()),
            ("ModelList", schema_of::<ModelList>()),
            ("ModelRow", schema_of::<ModelRow>()),
            ("RowAction", schema_of::<RowAction>()),
            ("RowEngine", schema_of::<RowEngine>()),
            ("RowVariant", schema_of::<RowVariant>()),
            ("Section", schema_of::<Section>()),
            ("Fact", schema_of::<Fact>()),
            ("SpeechUse", schema_of::<SpeechUse>()),
            ("ModelAct", schema_of::<ModelAct>()),
            ("ModelActed", schema_of::<ModelActed>()),
            ("ModelKeep", schema_of::<ModelKeep>()),
            ("ModelKept", schema_of::<ModelKept>()),
            ("ServiceQuery", schema_of::<ServiceQuery>()),
            ("ServiceReport", schema_of::<ServiceReport>()),
            ("ServedModel", schema_of::<ServedModel>()),
            ("SpeechRun", schema_of::<SpeechRun>()),
        ]),
    );
}

#[test]
fn the_committed_config_store_description_is_what_this_build_describes() {
    committed(
        "include/canoa.config_store.kinds.json",
        &document(&ConfigStoreVtable::description()),
    );
}

#[test]
fn the_committed_config_store_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.config_store.schemas.json",
        &schemas(vec![
            ("StoreInfo", schema_of::<ConfigStoreInfo>()),
            ("LayerInfo", schema_of::<LayerInfo>()),
            ("Snapshot", schema_of::<Snapshot>()),
            ("Layer", schema_of::<Layer>()),
            ("Key", schema_of::<ConfigKey>()),
            ("Held", schema_of::<ConfigHeld>()),
            ("Changes", schema_of::<Changes>()),
            ("Op", schema_of::<Op>()),
            ("Changed", schema_of::<Changed>()),
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
    // The spawner and the fetch a source's `read` is handed are described beside it, as the other kinds that name them do.
    committed(
        "include/canoa.catalog_source.kinds.json",
        &document(&format!(
            "{},{},{},{}",
            CatalogSourceProviderVtable::description(),
            SourceFetchVtable::description(),
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
fn the_committed_notifier_description_is_what_this_build_describes() {
    committed(
        "include/canoa.notifier.kinds.json",
        &document(&NotifierVtable::description()),
    );
}

#[test]
fn the_committed_notifier_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.notifier.schemas.json",
        &schemas(vec![
            ("NotifierInfo", schema_of::<NotifierInfo>()),
            ("SendRequest", schema_of::<SendRequest>()),
            ("Notice", schema_of::<Notice>()),
            ("SessionRef", schema_of::<SessionRef>()),
            ("Action", schema_of::<Action>()),
            ("ReplyTo", schema_of::<ReplyTo>()),
            ("Urgency", schema_of::<Urgency>()),
            ("Delivery", schema_of::<Delivery>()),
            ("ReplyRequest", schema_of::<ReplyRequest>()),
            ("Reply", schema_of::<Reply>()),
            ("WithdrawRequest", schema_of::<WithdrawRequest>()),
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

#[test]
fn the_committed_usage_description_is_what_this_build_describes() {
    // The host's process table, which `open` is handed, and the reading it answers with are described beside the reader.
    committed(
        "include/canoa.usage.kinds.json",
        &document(&format!(
            "{},{},{}",
            UsageReaderVtable::description(),
            UsageReadingVtable::description(),
            ProcessMembersVtable::description()
        )),
    );
}

#[test]
fn the_committed_usage_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.usage.schemas.json",
        &schemas(vec![
            ("ReaderInfo", schema_of::<ReaderInfo>()),
            ("Reading", schema_of::<Reading>()),
            ("Failure", schema_of::<Failure>()),
            ("Retry", schema_of::<Retry>()),
        ]),
    );
}

#[test]
fn the_committed_gateway_description_is_what_this_build_describes() {
    committed(
        "include/canoa.gateway.kinds.json",
        &document(&format!(
            "{},{},{},{}",
            ModelGatewayVtable::description(),
            GatewayLeaseVtable::description(),
            ProcessSpawnerVtable::description(),
            SpawnedProcessVtable::description()
        )),
    );
}

#[test]
fn the_committed_gateway_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.gateway.schemas.json",
        &schemas(vec![
            ("GatewayInfo", schema_of::<GatewayInfo>()),
            ("GatewayRequest", schema_of::<GatewayRequest>()),
        ]),
    );
}

#[test]
fn the_committed_account_provider_description_is_what_this_build_describes() {
    committed(
        "include/canoa.account_provider.kinds.json",
        &document(&format!(
            "{},{},{},{},{},{}",
            AccountProviderVtable::description(),
            AccountWatchVtable::description(),
            AccountKeeperVtable::description(),
            LoginPrompterVtable::description(),
            ProcessSpawnerVtable::description(),
            SpawnedProcessVtable::description()
        )),
    );
}

#[test]
fn the_committed_account_provider_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.account_provider.schemas.json",
        &schemas(vec![
            ("AccountProviderInfo", schema_of::<AccountProviderInfo>()),
            ("AccountOffer", schema_of::<AccountOffer>()),
            ("AccountKindInfo", schema_of::<AccountKindInfo>()),
            ("EndpointSlot", schema_of::<EndpointSlot>()),
            ("LoginMethodInfo", schema_of::<LoginMethodInfo>()),
            ("LoginRequest", schema_of::<LoginRequest>()),
            ("LoginOutcome", schema_of::<LoginOutcome>()),
            ("LoginForm", schema_of::<LoginForm>()),
            ("LoginAnswer", schema_of::<LoginAnswer>()),
            ("OpenUrl", schema_of::<OpenUrl>()),
            ("RenewRequest", schema_of::<RenewRequest>()),
            ("RenewReport", schema_of::<RenewReport>()),
            ("WatchRequest", schema_of::<WatchRequest>()),
            ("AccountRenewal", schema_of::<AccountRenewal>()),
            ("AccountLoss", schema_of::<AccountLoss>()),
            ("AccountUsageQuery", schema_of::<AccountUsageQuery>()),
            ("AccountUsage", schema_of::<AccountUsage>()),
            ("Bucket", schema_of::<Bucket>()),
            ("AccountModelsQuery", schema_of::<AccountModelsQuery>()),
            ("ModelListing", schema_of::<ModelListing>()),
            ("ModelInfo", schema_of::<ModelInfo>()),
            ("UnknownKind", schema_of::<UnknownKind>()),
            ("SignedInQuery", schema_of::<SignedInQuery>()),
            ("SignedInState", schema_of::<SignedInState>()),
            ("SignedInCheck", schema_of::<SignedInCheck>()),
        ]),
    );
}

#[test]
fn the_committed_environment_description_is_what_this_build_describes() {
    committed(
        "include/canoa.environment.kinds.json",
        &document(&EnvironmentVtable::description()),
    );
}

#[test]
fn the_committed_environment_schemas_are_what_this_build_describes() {
    committed(
        "include/canoa.environment.schemas.json",
        &schemas(vec![
            ("EnvironmentInfo", schema_of::<EnvironmentInfo>()),
            ("Capability", schema_of::<Capability>()),
            ("State", schema_of::<EnvironmentState>()),
            ("ReadFile", schema_of::<ReadFile>()),
            ("FileContent", schema_of::<FileContent>()),
            ("WriteFile", schema_of::<WriteFile>()),
            ("ListDir", schema_of::<ListDir>()),
            ("Listing", schema_of::<Listing>()),
            ("DirEntry", schema_of::<DirEntry>()),
            ("Call", schema_of::<Call>()),
            ("CallAnswer", schema_of::<CallAnswer>()),
        ]),
    );
}
