//! The Store answers from what it keeps: a stand-in publisher counts its requests, so a second list that reaches it would show.

use std::sync::atomic::AtomicUsize;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use canoa::store::recipe::{Recipe, Release};
use canoa::store::{InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Value, json};

use super::*;
use crate::store::caches::{DEFAULT_CATALOG_TTL, DEFAULT_MODELS_TTL, StoreCaches};
use crate::store::contract::{World, WorldTransport};
use crate::store::programs::{Options, Programs};
use crate::store::shelf::Shelf;

/// The requests the stand-in publisher has had.
static ASKED: AtomicUsize = AtomicUsize::new(0);
/// The tests share the one publisher, so they run one after another.
static ONE_AT_A_TIME: Mutex<()> = Mutex::new(());

fn asked() -> usize {
    ASKED.load(Ordering::SeqCst)
}

#[derive(Clone, Debug, Default)]
struct Counts;

impl Recipe for Counts {
    fn info(&self) -> StoreInfo {
        StoreInfo::new("counts", "Counts", "Acme")
    }

    fn names(&self) -> &'static [&'static str] {
        &["counts"]
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

    /// Says a version that is the number of this request, so which answer a client got shows.
    fn latest(&self, _channel: Option<&str>, _tools: &Object<dyn InstallTools>) -> LatestReport {
        let n = ASKED.fetch_add(1, Ordering::SeqCst) + 1;
        LatestReport::known(format!("{n}.0.0"))
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
    /// A program whose publisher counts the requests it gets.
    CountsStore, Counts, "counts_store"
);

coqui_acp_api::guatiao::local_providers!(
    id = "counts-library",
    version = env!("CARGO_PKG_VERSION"),
    providers = [CountsStore]
);

fn control_over(dir: &Path, caches: StoreCaches) -> Arc<Control> {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("counts-library", library)
        .expect("registered");
    let programs = Programs::new(Shelf::unchecked(dir.join("shelf")), Arc::new(registry))
        .transport(WorldTransport::new(World::new()))
        .options(Options {
            home: Some(dir.join("home")),
            ..Options::default()
        });
    let caches = Arc::new(caches);
    let stores = Stores::new(Arc::new(programs), vec!["counts_store".to_owned()])
        .with_home(Some(dir.join("home")))
        .with_caches(Arc::clone(&caches));
    Arc::new(
        Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens")
        .with_caches(caches)
        .with_stores(stores),
    )
}

fn caches(latest: Duration, refresh_every: Duration) -> StoreCaches {
    StoreCaches::new(
        None,
        latest,
        DEFAULT_MODELS_TTL,
        DEFAULT_CATALOG_TTL,
        refresh_every,
    )
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-store-cache-{name}-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("home")).expect("a home");
    dir
}

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

fn latest(control: &Control, refresh: bool) -> Value {
    ask(
        control,
        "store/latest",
        &json!({"provider": "counts_store", "refresh": refresh}),
    )
}

fn wait_for(what: &str, mut done: impl FnMut() -> bool) {
    for _ in 0..300 {
        if done() {
            return;
        }
        std::thread::sleep(Duration::from_millis(10));
    }
    panic!("{what}");
}

#[test]
fn a_second_list_is_answered_from_the_cache_and_a_refresh_asks_the_publisher_again() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    ASKED.store(0, Ordering::SeqCst);
    let dir = folder("second");
    let control = control_over(&dir, caches(Duration::from_secs(3600), Duration::ZERO));

    // Nothing is kept yet, so the overview says nothing of the newest version and asks nobody.
    let first = ask(&control, "store/overview", &json!({}));
    let row = &first["stores"][0];
    assert_eq!(row["provider"], "counts_store");
    assert!(row.get("latest").is_none(), "{row}");
    assert!(row["report"]["copies"].is_array(), "{row}");
    assert_eq!(asked(), 0, "an overview asks no publisher");

    assert_eq!(latest(&control, false)["version"], "1.0.0");
    assert_eq!(asked(), 1);
    let again = latest(&control, false);
    assert_eq!(again["version"], "1.0.0", "{again}");
    assert_eq!(again["stale"], false);
    assert_eq!(asked(), 1, "the second list reached no publisher");

    let second = ask(&control, "store/overview", &json!({}));
    assert_eq!(second["stores"][0]["latest"]["version"], "1.0.0");
    assert_eq!(second["stores"][0]["latest"]["stale"], false);
    assert_eq!(asked(), 1);

    assert_eq!(latest(&control, true)["version"], "2.0.0");
    assert_eq!(asked(), 2);
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_stale_version_is_served_at_once_and_the_publisher_is_asked_once() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    ASKED.store(0, Ordering::SeqCst);
    let dir = folder("stale");
    let control = control_over(&dir, caches(Duration::from_millis(80), Duration::ZERO));
    assert_eq!(latest(&control, false)["version"], "1.0.0");
    std::thread::sleep(Duration::from_millis(160));
    for _ in 0..3 {
        let stale = latest(&control, false);
        assert_eq!(stale["version"], "1.0.0", "the old one, at once: {stale}");
    }
    wait_for("the publisher is asked again", || {
        control
            .caches
            .latest
            .peek("counts_store", None)
            .is_some_and(|kept| kept["version"] == "2.0.0")
    });
    assert_eq!(asked(), 2, "once, however many asked meanwhile");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn the_timer_asks_again_with_no_request_while_a_client_is_connected_and_not_otherwise() {
    let _one = ONE_AT_A_TIME.lock().unwrap_or_else(PoisonError::into_inner);
    ASKED.store(0, Ordering::SeqCst);
    let dir = folder("timer");
    // The interval is the test's own, a few milliseconds, never minutes.
    let control = control_over(
        &dir,
        caches(Duration::from_millis(60), Duration::from_millis(80)),
    );
    let timer = control.keep_fresh().expect("a timer");
    assert_eq!(latest(&control, false)["version"], "1.0.0");
    std::thread::sleep(Duration::from_millis(500));
    assert_eq!(asked(), 1, "no client is connected, so nothing is renewed");

    let stopped = Arc::new(AtomicBool::new(false));
    let (client, host) = crate::local::pair(MAX_REQUEST, stopped);
    std::thread::scope(|scope| {
        scope.spawn(|| serve(&control, &Caller::Owner, &host));
        wait_for("the timer renews with no request", || asked() >= 2);
        assert_eq!(
            control
                .caches
                .latest
                .peek("counts_store", None)
                .expect("kept")["version"]
                .as_str()
                .map(|version| version.ends_with(".0.0")),
            Some(true)
        );
        client.close();
    });
    wait_for("the link is gone", || {
        control.clients.load(Ordering::SeqCst) == 0
    });
    std::thread::sleep(Duration::from_millis(200));
    let after = asked();
    std::thread::sleep(Duration::from_millis(400));
    assert_eq!(asked(), after, "nothing is renewed once the client is gone");

    drop(control);
    timer.join().expect("the timer ends with the host");
    let _ = fs::remove_dir_all(&dir);
}

#[test]
fn a_timer_of_zero_starts_none() {
    let dir = folder("off");
    let control = control_over(&dir, caches(Duration::from_secs(1), Duration::ZERO));
    assert!(control.keep_fresh().is_none());
    let _ = fs::remove_dir_all(&dir);
}
