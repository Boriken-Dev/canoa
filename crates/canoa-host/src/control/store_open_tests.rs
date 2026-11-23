//! One Store open, counted at a publisher: a stand-in on a loopback address says the newest version in a 4000-byte release list with
//! an `ETag`, and counts the connections, requests and bytes it gets from the host's real HTTP client.

use std::sync::{Arc, OnceLock};
use std::time::Duration;

use canoa::store::recipe::{Recipe, Release};
use canoa::store::{InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use coqui_syshttp::tls::TlsBuilder;
use serde_json::{Value, json};

use super::*;
use crate::store::caches::{
    DEFAULT_CATALOG_TTL, DEFAULT_CONNECT_TIMEOUT, DEFAULT_LOOKUP_TIMEOUT, DEFAULT_MODELS_TTL,
    StoreCaches,
};
use crate::store::http_tests::{Answer, Origin};
use crate::store::programs::{Options, Programs};
use crate::store::shelf::Shelf;
use crate::store::tools::HttpTransport;

/// Where the stand-in publisher is, for the recipe.
static ORIGIN: OnceLock<String> = OnceLock::new();

#[derive(Clone, Debug, Default)]
struct Reads;

impl Recipe for Reads {
    fn info(&self) -> StoreInfo {
        let mut info = StoreInfo::new("reads", "Reads", "Acme");
        info.endpoints = vec![ORIGIN.get().cloned().unwrap_or_default()];
        info
    }

    fn names(&self) -> &'static [&'static str] {
        &["reads"]
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

    /// Reads the publisher's release list, as a store that lists a publisher's releases does.
    fn latest(&self, _channel: Option<&str>, tools: &Object<dyn InstallTools>) -> LatestReport {
        let url = format!("{}/releases", ORIGIN.get().cloned().unwrap_or_default());
        match tools.fetch_text(&url, 1 << 20) {
            Ok(text) => LatestReport::known(&text[..5]),
            Err(error) => LatestReport::unknown(error.message().to_owned()),
        }
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
    /// A program whose newest version is read from a publisher's release list.
    ReadsStore, Reads, "reads_store"
);

coqui_acp_api::guatiao::local_providers!(
    id = "reads-library",
    version = env!("CARGO_PKG_VERSION"),
    providers = [ReadsStore]
);

fn ask(control: &Control, method: &str, params: &Value) -> Value {
    let request = json!({"jsonrpc": "2.0", "id": 1, "method": method, "params": params});
    let line = control
        .handle(
            &Caller::Owner,
            &serde_json::to_vec(&request).expect("encodes"),
        )
        .expect("answered");
    let answer: Value = serde_json::from_slice(&line).expect("JSON");
    assert!(answer.get("error").is_none(), "{method}: {answer}");
    answer["result"].clone()
}

/// What the app does to open the Store: one overview, then the newest version of each program the host keeps no fresh answer for.
fn open_the_store(control: &Control, refresh: bool) -> Value {
    let overview = ask(control, "store/overview", &json!({}));
    let kept_fresh = overview["stores"][0]["latest"]["stale"] == json!(false);
    if refresh || !kept_fresh {
        return ask(
            control,
            "store/latest",
            &json!({"provider": "reads_store", "refresh": refresh}),
        );
    }
    overview["stores"][0]["latest"].clone()
}

#[test]
fn opening_the_store_reaches_a_publisher_once_and_costs_no_body_when_it_has_not_changed() {
    let mut document = b"1.2.3".to_vec();
    document.resize(4000, b' ');
    let origin = Origin::start(move |asked| {
        if asked.if_none_match.as_deref() == Some("\"r1\"") {
            Answer {
                status: 304,
                etag: Some("\"r1\"".to_owned()),
                body: Vec::new(),
                delay: Duration::ZERO,
            }
        } else {
            Answer {
                status: 200,
                etag: Some("\"r1\"".to_owned()),
                body: document.clone(),
                delay: Duration::ZERO,
            }
        }
    });
    ORIGIN.set(origin.url.clone()).expect("one test");

    let dir = std::env::temp_dir().join(format!("canoa-store-open-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(dir.join("home")).expect("a home");
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("reads-library", library)
        .expect("registered");
    let network = Arc::new(canoa::network::Network::with(
        crate::network::tests::direct(),
        TlsBuilder::default(),
    ));
    let programs = Programs::new(Shelf::unchecked(dir.join("shelf")), Arc::new(registry))
        .transport(Arc::new(HttpTransport::on(network)))
        .options(Options {
            home: Some(dir.join("home")),
            loopback_http: true,
            ..Options::default()
        });
    // A time to live short enough that the third open finds the kept version stale.
    let caches = Arc::new(
        StoreCaches::new(
            Some(dir.join("cache")),
            Duration::from_millis(300),
            DEFAULT_MODELS_TTL,
            DEFAULT_CATALOG_TTL,
            Duration::ZERO,
        )
        .with_lookups(DEFAULT_CONNECT_TIMEOUT, DEFAULT_LOOKUP_TIMEOUT),
    );
    let stores = Stores::new(Arc::new(programs), vec!["reads_store".to_owned()])
        .with_home(Some(dir.join("home")))
        .with_caches(Arc::clone(&caches));
    let control = Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_caches(caches)
    .with_stores(stores);

    let mut table = Vec::new();
    let mut count = |what: &str| {
        table.push(format!(
            "{what}: {} requests, {} body bytes, {} connections",
            origin.asked(),
            origin.sent(),
            origin.opened()
        ));
    };
    // The first open reads the release list.
    assert_eq!(open_the_store(&control, false)["version"], "1.2.3");
    count("open 1 (nothing kept)");
    assert_eq!(
        (origin.asked(), origin.sent(), origin.opened()),
        (1, 4000, 1)
    );
    // The second finds what was kept, fresh, and asks nobody.
    assert_eq!(open_the_store(&control, false)["version"], "1.2.3");
    count("open 2 (kept, fresh)");
    assert_eq!((origin.asked(), origin.sent()), (1, 4000));
    // The third finds it stale, and the publisher says it has not changed.
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(open_the_store(&control, true)["version"], "1.2.3");
    count("open 3 (stale, asked again)");
    assert_eq!(
        (origin.asked(), origin.sent(), origin.opened()),
        (2, 4000, 1),
        "one request, no body, the same connection"
    );
    eprintln!(
        "a Store open, counted at the publisher:\n{}",
        table.join("\n")
    );
    let _ = std::fs::remove_dir_all(&dir);
}
