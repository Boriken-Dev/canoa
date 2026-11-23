//! A link answers its requests as they are ready: one that waits on a slow source does not hold the others back.
//!
//! The Store here has one program whose `store/latest` waits until the test lets it go, which is what a publisher that does not
//! answer looks like to the host.

use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::time::Duration;

use canoa::store::recipe::{Recipe, Release};
use canoa::store::{InstallTools, LatestReport, MANAGER_UNKNOWN, Platform, StoreInfo, Updatable};
use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Value, json};

use super::*;
use crate::store::contract::{World, WorldTransport};
use crate::store::programs::{Options, Programs};
use crate::store::shelf::Shelf;

/// Whether the stand-in publisher answers yet.
static OPEN: (Mutex<bool>, Condvar) = (Mutex::new(false), Condvar::new());

fn let_go() {
    *OPEN.0.lock().unwrap_or_else(PoisonError::into_inner) = true;
    OPEN.1.notify_all();
}

#[derive(Clone, Debug, Default)]
struct Hangs;

impl Recipe for Hangs {
    fn info(&self) -> StoreInfo {
        StoreInfo::new("hangs", "Hangs", "Acme")
    }

    fn names(&self) -> &'static [&'static str] {
        &["hangs"]
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
        let mut open = OPEN.0.lock().unwrap_or_else(PoisonError::into_inner);
        while !*open {
            let (next, timeout) = OPEN
                .1
                .wait_timeout(open, Duration::from_secs(60))
                .unwrap_or_else(PoisonError::into_inner);
            open = next;
            if timeout.timed_out() {
                break;
            }
        }
        LatestReport::known("2.0.0")
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
    /// A program whose publisher does not answer until the test says.
    HangsStore, Hangs, "hangs_store"
);

coqui_acp_api::guatiao::local_providers!(
    id = "hangs-library",
    version = env!("CARGO_PKG_VERSION"),
    providers = [HangsStore]
);

fn control_over(dir: &Path) -> Control {
    let mut registry = Registry::new("canoa", env!("CARGO_PKG_VERSION"));
    registry
        .register_local("hangs-library", library)
        .expect("registered");
    let programs = Programs::new(Shelf::unchecked(dir.join("shelf")), Arc::new(registry))
        .transport(WorldTransport::new(World::new()))
        .options(Options {
            home: Some(dir.join("home")),
            ..Options::default()
        });
    let stores = Stores::new(Arc::new(programs), vec!["hangs_store".to_owned()])
        .with_home(Some(dir.join("home")));
    Control::open(
        None,
        DEFAULT_MAX_RECORDS,
        DEFAULT_MAX_RECORD_BYTES,
        Vec::new(),
    )
    .expect("opens")
    .with_stores(stores)
}

fn say<C: Channel>(client: &C, id: u32, method: &str, params: &Value) {
    let request = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
    client
        .send(&serde_json::to_vec(&request).expect("encodes"))
        .expect("sent");
}

fn heard<C: Channel>(client: &C, wait: Duration) -> Option<Value> {
    let line = client.receive(Some(wait)).ok().flatten()?;
    Some(serde_json::from_slice(&line).expect("JSON"))
}

#[test]
fn a_request_that_waits_on_a_slow_source_does_not_hold_the_others_back() {
    let dir = std::env::temp_dir().join(format!("canoa-concurrent-{}", std::process::id()));
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(dir.join("home")).expect("a home");
    let control = control_over(&dir);
    let stopped = Arc::new(AtomicBool::new(false));
    let (client, host) = crate::local::pair(MAX_REQUEST, stopped);
    std::thread::scope(|scope| {
        scope.spawn(|| serve(&control, &Caller::Owner, &host));
        say(
            &client,
            1,
            "store/latest",
            &json!({"provider": "hangs_store"}),
        );
        say(&client, 2, "ledger/list", &Value::Null);
        say(&client, 3, "store/list", &Value::Null);
        // The two quick requests are answered while the first still waits for its publisher.
        let mut early: Vec<Option<u64>> = (0..2)
            .map(|_| {
                heard(&client, Duration::from_secs(10)).and_then(|answer| answer["id"].as_u64())
            })
            .collect();
        let_go();
        let last =
            heard(&client, Duration::from_secs(20)).expect("the slow one is answered once let go");
        client.close();
        early.sort_unstable();
        assert_eq!(early, [Some(2), Some(3)], "answers before the release");
        assert_eq!(last["id"], 1, "{last}");
        assert_eq!(last["result"]["version"], "2.0.0", "{last}");
    });
    let _ = fs::remove_dir_all(&dir);
}
