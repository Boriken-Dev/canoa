//! The Models page of a type whose catalogs answer in `canoa.models/1`: the catalog its extension registers (pinned, with a shipped
//! snapshot, so nothing is fetched), canoa.cfg's entries that replace it, add another or turn it off, the badge each row carries
//! and the download of a row the catalog lists. The publisher is a world of documents; nothing touches the network.

use std::path::PathBuf;
use std::sync::Arc;

use canoa::store::extension::{
    CatalogWords, DefaultCatalog, ExtensionInfo, ExtensionType, StoreExtension,
};
use canoa::store::models::{
    ModelAct, ModelActed, ModelKeep, ModelKept, ModelList, ModelQuery, ModelStore, ModelStoreInfo,
    ServiceQuery, ServiceReport,
};
use canoa::store::recipe::{Recipe, Release};
use canoa::store::{InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_acp_api::guatiao::{Provider, Status};
use coqui_acp_api::{Environment, ProcessSpawner};
use serde_json::{Value, json};
use sha2::{Digest, Sha256};

use super::super::Failed;
use super::*;
use crate::control::files::UploadLimits;
use crate::store::contract::{World, WorldTransport};
use crate::store::programs::{Options, Programs};
use crate::store::shelf::Shelf;

const WEIGHTS: &[u8] = b"a stand-in model, a few bytes of it";
const ORIGIN: &str = "https://models.example/";

fn digest(bytes: &[u8]) -> String {
    data_encoding::HEXLOWER.encode(&Sha256::digest(bytes))
}

/// A catalog of two items: one with its digest, one with none.
fn snapshot() -> String {
    json!({"format": "canoa.models/1", "revision": "r1", "items": [
        {"id": "base", "label": "Base", "by": "acme", "tags": ["multilingual"], "license": "MIT",
         "files": [{"url": format!("{ORIGIN}base.bin"), "file": "base.bin", "bytes": WEIGHTS.len(),
                    "check": {"sha256": digest(WEIGHTS)}}]},
        {"id": "bare", "label": "Bare", "files": [{"url": format!("{ORIGIN}bare.bin"), "file": "bare.bin"}]},
    ]})
    .to_string()
}

/// A program that is nobody's but a test's.
#[derive(Clone, Debug, Default)]
struct Bare;

impl Recipe for Bare {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("stand", "Stand", "Acme");
        info.endpoints = vec![ORIGIN.to_owned()];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["stand-cli"]
    }

    fn dirs(&self) -> &'static [&'static str] {
        &[]
    }

    fn version_args(&self) -> &'static [&'static str] {
        &["--version"]
    }

    fn parse_version(&self, line: &str) -> Option<String> {
        Some(line.trim().to_owned())
    }

    fn classify(&self, _path: &str) -> (&'static str, Updatable) {
        (
            MANAGER_UNKNOWN,
            Updatable::No {
                why: "a test's".to_owned(),
            },
        )
    }

    fn launch_environment(&self) -> std::collections::BTreeMap<String, String> {
        std::collections::BTreeMap::new()
    }

    fn latest(&self, _channel: Option<&str>, _tools: &Object<dyn InstallTools>) -> LatestReport {
        LatestReport::unknown("a test's".to_owned())
    }

    fn release(
        &self,
        _version: Option<&str>,
        _channel: Option<&str>,
        _platform: &Platform,
        _tools: &Object<dyn InstallTools>,
    ) -> Result<Release, ProviderError> {
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "a test's program installs nothing",
        ))
    }
}

canoa::store_provider!(
    /// The program as a test has it.
    StandStore, Bare, "stand_store"
);

#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(StoreExtension, id = "stand_extension", crate = ::coqui_acp_api::guatiao)]
struct Extension;

impl StoreExtension for Extension {
    fn describe(&self) -> Result<ExtensionInfo, ProviderError> {
        let mut kind = ExtensionType::new("stand", "Stand", ["stand_store"]);
        kind.models = Some("stand_models".to_owned());
        kind.catalogs = Some(
            CatalogWords::new("source", "sources", "models")
                .in_format("canoa.models/1")
                .registering([DefaultCatalog::new("acme", "read").pinned("r1", snapshot())]),
        );
        Ok(ExtensionInfo::new("Stand", "Acme", vec![kind]))
    }
}

/// A `model_store` that lists nothing of its own: every row of the page comes from a catalog.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(ModelStore, id = "stand_models", crate = ::coqui_acp_api::guatiao)]
struct Models;

impl ModelStore for Models {
    fn describe(&self) -> Result<ModelStoreInfo, ProviderError> {
        Ok(ModelStoreInfo::new("Stand", "Acme", "Acme", "none"))
    }

    fn list(
        &self,
        _query: ModelQuery,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelList, ProviderError> {
        Ok(ModelList::default())
    }

    fn act(
        &self,
        _request: ModelAct,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelActed, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "none"))
    }

    fn keep(
        &self,
        _request: ModelKeep,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ModelKept, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "none"))
    }

    fn service(
        &self,
        _query: ServiceQuery,
        _spawner: Object<dyn ProcessSpawner>,
    ) -> Result<ServiceReport, ProviderError> {
        Err(ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, "none"))
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "stand-library",
    version = env!("CARGO_PKG_VERSION"),
    providers = [StandStore, Extension, Models]
);

struct Fixture {
    root: PathBuf,
    stores: Stores,
    files: Files,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!(
        "canoa-model-catalogs-{name}-{}",
        std::process::id()
    ));
    let _ = std::fs::remove_dir_all(&root);
    let bin = root.join("bin");
    let home = root.join("home");
    std::fs::create_dir_all(&bin).expect("a program folder");
    std::fs::create_dir_all(&home).expect("a home");
    std::fs::write(
        bin.join(if cfg!(windows) {
            "stand-cli.exe"
        } else {
            "stand-cli"
        }),
        b"program",
    )
    .expect("a program");
    let options = Options {
        environment: [("PATH".to_owned(), bin.to_string_lossy().into_owned())]
            .into_iter()
            .collect::<Environment>(),
        home: Some(home.clone()),
        ..Options::default()
    };
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("stand-library", library)
        .expect("registered");
    let registry = Arc::new(registry);
    let mut world = World::new();
    world.put(format!("{ORIGIN}base.bin"), WEIGHTS);
    world.put(format!("{ORIGIN}bare.bin"), WEIGHTS);
    world.put("https://mirror.example/models/m.bin".to_owned(), WEIGHTS);
    let programs = Programs::new(Shelf::unchecked(root.join("shelf")), Arc::clone(&registry))
        .transport(WorldTransport::new(world))
        .options(options);
    let stores = Stores::new(Arc::new(programs), Vec::new())
        .with_included_extensions(&registry)
        .with_home(Some(home));
    let files = Files::new(
        Some(root.clone()),
        Vec::new(),
        Some(root.join("canoa.cfg")),
        None,
        UploadLimits::default(),
    );
    Fixture {
        root,
        stores,
        files,
    }
}

impl Fixture {
    /// canoa.cfg's `[store.stand.catalogs]` as the person writes it.
    fn catalogs(&self, table: &Value) {
        self.files
            .config_set(
                &json!({"section": "store", "value": {"stand": {"catalogs": table}}}),
                true,
            )
            .expect("written");
    }

    fn list(&self) -> Value {
        self.stores
            .models_read(&json!({"type": "stand"}), &self.files)
            .expect("listed")
    }

    fn row(&self, id: &str) -> Option<Value> {
        self.list()["models"]
            .as_array()
            .and_then(|all| all.iter().find(|row| row["id"] == id))
            .cloned()
    }

    fn act(&self, model: &str, action: &str) -> Result<Value, Failed> {
        self.stores.model_act(
            &json!({"type": "stand", "model": model, "action": action}),
            &self.files,
        )
    }

    /// A catalog of the person's own, a file the `read` source reads.
    fn file(&self, name: &str, text: &str) -> String {
        let path = self.root.join(name);
        std::fs::write(&path, text).expect("a catalog file");
        path.to_string_lossy().into_owned()
    }
}

#[test]
fn the_default_a_pinned_extension_registers_lists_its_items_from_the_shipped_snapshot_with_their_badges()
 {
    let host = fixture("default");
    let page = host.list();
    let labels: Vec<&str> = page["sources"]
        .as_array()
        .expect("sources")
        .iter()
        .filter_map(|source| source["label"].as_str())
        .collect();
    assert_eq!(labels, ["acme"]);
    assert_eq!(page["sources"][0]["from"], "snapshot");
    assert_eq!(page["sources"][0]["trust"], "pinned");
    let base = host.row("base").expect("a row");
    assert_eq!(base["catalog"]["label"], "acme");
    assert_eq!(base["catalog"]["origin"], "default");
    assert_eq!(base["catalog"]["revision"], "r1");
    assert_eq!(base["verified"], "SHA-256");
    assert_eq!(base["actions"][0]["steps"], json!(["fetch_files"]));
    assert_eq!(base["by"], "acme");
    let bare = host.row("bare").expect("a row");
    assert_eq!(bare["verified"], Value::Null, "no digest: not verified");
    assert_eq!(
        bare["actions"][0]["steps"],
        json!(["fetch_files"]),
        "and offered all the same"
    );
    assert_eq!(bare["state"], Value::Null);
}

#[test]
fn a_row_the_catalog_lists_is_downloaded_checked_and_removed() {
    let host = fixture("install");
    host.act("base", "fetch_files").expect("installed");
    let file = host
        .root
        .join("shelf")
        .join("stand")
        .join("models")
        .join("base.bin");
    assert_eq!(std::fs::read(&file).expect("the file"), WEIGHTS);
    let after = host.row("base").expect("a row");
    assert_eq!(after["state"], "installed");
    assert_eq!(after["actions"][0]["steps"], json!(["delete_files"]));
    host.act("base", "delete_files").expect("removed");
    assert!(!file.exists());
}

#[test]
fn an_entry_replaces_the_default_adds_a_catalog_or_turns_the_default_off() {
    let host = fixture("merge");
    let mine = host.file(
        "mine.json",
        &json!({"format": "canoa.models/1", "items": [
            {"id": "base", "label": "My base", "files": [{"url": format!("{ORIGIN}base.bin"), "file": "mine.bin"}]},
            {"id": "extra", "label": "Extra", "files": [{"url": format!("{ORIGIN}base.bin"), "file": "extra.bin",
                "check": {"sha256": digest(WEIGHTS)}}]},
        ]})
        .to_string(),
    );

    // Added: a second catalog, whose `base` is shown beside the default's; its file has no digest, so it is not verified.
    host.catalogs(&json!({"mine": mine.clone()}));
    assert_eq!(
        host.row("base").expect("the default's")["catalog"]["label"],
        "acme"
    );
    let second = host
        .row("base@mine")
        .expect("the added catalog's, by its label");
    assert_eq!(second["catalog"]["origin"], "added");
    assert_eq!(second["verified"], Value::Null);
    assert_eq!(host.row("extra").expect("extra")["verified"], "SHA-256");
    host.act("base@mine", "fetch_files")
        .expect("offered and taken, as not verified");
    assert!(host.root.join("shelf/stand/models/mine.bin").is_file());

    // Replaced: an entry with the default's label is the catalog now, and the default's rows are gone.
    host.catalogs(&json!({"acme": mine.clone()}));
    assert!(host.row("bare").is_none(), "the default was replaced whole");
    assert_eq!(
        host.row("base").expect("a row")["catalog"]["origin"],
        "replaced"
    );

    // Off: the default is listed, off, and its items are gone.
    host.catalogs(&json!({"acme": false}));
    let page = host.list();
    assert_eq!(page["models"], json!([]));
    assert_eq!(page["sources"][0]["label"], "acme");
    assert_eq!(page["sources"][0]["enabled"], false);
}

#[test]
fn a_catalog_that_cannot_be_read_is_listed_with_the_reason_and_the_others_stay() {
    let host = fixture("broken");
    host.catalogs(&json!({"gone": host.root.join("missing.json").to_string_lossy()}));
    let page = host.list();
    assert!(host.row("base").is_some(), "the default stays");
    let gone = page["sources"]
        .as_array()
        .and_then(|all| all.iter().find(|source| source["label"] == "gone"))
        .expect("listed");
    assert!(gone["problem"].is_string(), "{gone}");
}

#[test]
fn a_file_outside_the_places_the_type_downloads_from_is_refused_until_an_entry_names_them() {
    let host = fixture("endpoints");
    let mirrored = host.file(
        "mirror.json",
        &json!({"format": "canoa.models/1", "items": [
            {"id": "mirrored", "label": "Mirrored", "files": [{"url": "https://mirror.example/models/m.bin", "file": "m.bin",
                "check": {"sha256": digest(WEIGHTS)}}]},
        ]})
        .to_string(),
    );
    host.catalogs(&json!({"mirror": {"provider": "read", "source": mirrored.clone()}}));
    let refused = host
        .act("mirrored", "fetch_files")
        .expect_err("not one of the type's places");
    assert!(
        refused.1.contains("none of the places") && refused.1.contains("endpoints"),
        "{refused:?}"
    );
    assert!(!host.root.join("shelf/stand/models/m.bin").exists());

    host.catalogs(
        &json!({"mirror": {"provider": "read", "source": mirrored.clone(),
        "endpoints": ["https://mirror.example/models/"]}}),
    );
    assert_eq!(
        host.list()["sources"][1]["endpoints"],
        json!(["https://mirror.example/models/"])
    );
    host.act("mirrored", "fetch_files")
        .expect("installed from the mirror");
    assert_eq!(
        std::fs::read(host.root.join("shelf/stand/models/m.bin")).expect("the file"),
        WEIGHTS
    );
}
