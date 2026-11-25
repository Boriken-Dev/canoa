//! A program store that registers a catalog of releases: its newest version and its plan come from the catalogs of its scope, the one
//! its extension registers (pinned, with a shipped snapshot) and canoa.cfg's entries that replace it, add another or turn it off.
//! The publisher is not asked at all: nothing touches the network.

use std::path::PathBuf;
use std::sync::Arc;

use canoa::store::extension::{CatalogWords, DefaultCatalog};
use canoa::store::recipe::{Recipe, UnpackStep};
use canoa::store::{MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use serde_json::{Value, json};

use super::*;
use crate::store::contract::{World, WorldTransport};
use crate::store::programs::{Options, Programs};
use crate::store::shelf::Shelf;

const SHA256: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";

/// A releases catalog of `versions`, each served for every platform the Store measures.
fn releases(versions: &[&str], stable: &str) -> String {
    let platforms: Value = ["windows", "linux"]
        .iter()
        .flat_map(|os| ["x86_64", "aarch64"].map(|arch| format!("{os}-{arch}")))
        .map(|key| {
            let suffix = if key.starts_with("windows") { ".exe" } else { "" };
            (
                key,
                json!({"launch": format!("feed{suffix}"), "parts": [
                    {"url": format!("https://feed.example/feed-1.0{suffix}"), "path": format!("feed{suffix}"),
                     "executable": true, "bytes": 5, "check": {"sha256": SHA256}}]}),
            )
        })
        .collect::<serde_json::Map<_, _>>()
        .into();
    json!({"format": "canoa.releases/1", "channels": {"stable": stable},
        "releases": versions.iter().map(|version| json!({"version": version, "channels": ["stable"],
            "notices": ["from a catalog"], "platforms": platforms})).collect::<Vec<_>>()})
    .to_string()
}

/// A program whose releases are all in its catalogs: the recipe says only what the catalogs cannot.
#[derive(Clone, Debug, Default)]
struct Feed;

impl Recipe for Feed {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("feed", "Feed", "Acme");
        info.channels = vec!["stable".to_owned()];
        info.endpoints = vec!["https://feed.example/".to_owned()];
        info.catalogs = Some(
            CatalogWords::new("source", "sources", "releases")
                .in_format("canoa.releases/1")
                .registering([DefaultCatalog::new("acme", "read")
                    .pinned("2.0.0", releases(&["2.0.0", "1.0.0"], "2.0.0"))]),
        );
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["feed-cli"]
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

    fn adjust(
        &self,
        mut release: canoa::store::recipe::Release,
        _platform: &Platform,
    ) -> Result<canoa::store::recipe::Release, ProviderError> {
        release.steps.push("adjusted by the recipe".to_owned());
        let _ = UnpackStep::new("zip", "x", 0, true);
        Ok(release)
    }
}

canoa::store_provider!(
    /// The program as a test has it.
    FeedStore, Feed, "feed_store"
);

coqui_acp_api::guatiao::local_providers!(
    id = "feed-library",
    version = env!("CARGO_PKG_VERSION"),
    providers = [FeedStore]
);

struct Fixture {
    root: PathBuf,
    stores: Stores,
}

impl Drop for Fixture {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.root);
    }
}

fn fixture(name: &str) -> Fixture {
    let root = std::env::temp_dir().join(format!("canoa-releases-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&root);
    let home = root.join("home");
    std::fs::create_dir_all(&home).expect("a home");
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("feed-library", library)
        .expect("registered");
    let programs = Programs::new(Shelf::unchecked(root.join("shelf")), Arc::new(registry))
        .transport(WorldTransport::new(World::new()))
        .options(Options {
            home: Some(home.clone()),
            ..Options::default()
        });
    let stores =
        Stores::new(Arc::new(programs), vec!["feed_store".to_owned()]).with_home(Some(home));
    Fixture { root, stores }
}

impl Fixture {
    /// The catalogs of the `feed` scope as canoa.cfg's `[store]` says them.
    fn with(&self, catalogs: &Value) {
        self.stores
            .trust_from(&json!({"feed": {"catalogs": catalogs}}));
    }

    fn call(&self, method: &str, params: &Value) -> Result<Value, Failed> {
        self.stores.read(method, params, Some("test"))
    }

    fn file(&self, name: &str, text: &str) -> String {
        let path = self.root.join(name);
        std::fs::write(&path, text).expect("a catalog file");
        path.to_string_lossy().into_owned()
    }
}

#[test]
fn the_newest_version_and_the_plan_come_from_the_default_catalog_the_extension_registers() {
    let host = fixture("default");
    host.with(&json!({}));
    let latest = host
        .call("store/latest", &json!({"provider": "feed_store"}))
        .expect("answered");
    assert_eq!(latest["known"], true, "{latest}");
    assert_eq!(latest["version"], "2.0.0");
    assert_eq!(latest["channel"], "stable");
    let plan = host
        .call(
            "store/plan",
            &json!({"provider": "feed_store", "action": "install"}),
        )
        .expect("planned");
    assert_eq!(plan["version"], "2.0.0", "{plan}");
    let download = &plan["downloads"][0];
    assert_eq!(download["check"]["kind"], "sha256");
    assert_eq!(download["check"]["value"], SHA256);
    assert!(
        download["url"]
            .as_str()
            .is_some_and(|url| url.starts_with("https://feed.example/")),
        "{plan}"
    );
    assert!(
        plan["steps"]
            .as_array()
            .is_some_and(|steps| steps.iter().any(|step| step == "adjusted by the recipe")),
        "the recipe's own step: {plan}"
    );
    let older = host
        .call(
            "store/plan",
            &json!({"provider": "feed_store", "action": "install", "version": "1.0.0"}),
        )
        .expect("a listed version");
    assert_eq!(older["version"], "1.0.0");
    let unlisted = host
        .call(
            "store/plan",
            &json!({"provider": "feed_store", "action": "install", "version": "0.5.0"}),
        )
        .expect_err("no catalog lists it");
    assert!(
        unlisted.1.contains("lists the release 0.5.0"),
        "{unlisted:?}"
    );
}

#[test]
fn an_entry_replaces_the_default_adds_a_catalog_and_turning_every_one_off_says_so() {
    let host = fixture("merge");
    let newer = host.file("newer.json", &releases(&["3.0.0"], "3.0.0"));
    // Replaced: the entry of the default's label is the catalog now.
    host.with(&json!({"acme": newer.clone()}));
    let latest = host
        .call(
            "store/latest",
            &json!({"provider": "feed_store", "refresh": true}),
        )
        .expect("answered");
    assert_eq!(latest["version"], "3.0.0", "{latest}");
    // Added: the default's channel answer stands, and the added catalog's versions can be installed.
    host.with(&json!({"mine": newer}));
    let plan = host
        .call(
            "store/plan",
            &json!({"provider": "feed_store", "action": "install", "version": "3.0.0"}),
        )
        .expect("a version the added catalog lists");
    assert_eq!(plan["version"], "3.0.0");
    let latest = host
        .call(
            "store/latest",
            &json!({"provider": "feed_store", "refresh": true}),
        )
        .expect("answered");
    assert_eq!(
        latest["version"], "2.0.0",
        "the first catalog names the channel: {latest}"
    );
    // Off: nothing is left to read, and why is the answer.
    host.with(&json!({"acme": false}));
    let off = host
        .call(
            "store/latest",
            &json!({"provider": "feed_store", "refresh": true}),
        )
        .expect("answered");
    assert_eq!(off["known"], false, "{off}");
    assert!(
        off["unknown_because"]
            .as_str()
            .is_some_and(|why| why.contains("turned off")),
        "{off}"
    );
}
