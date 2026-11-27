//! A pool's turn read from real account providers against a stand-in for the provider's usage endpoint, which answers each login
//! by its own token: two accounts of one pool, one with its allowance used up.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::io::{Read as _, Write as _};
use std::net::TcpListener;
use std::path::PathBuf;
use std::sync::Arc;
use std::sync::atomic::{AtomicU64, Ordering};

use coqui_acp_api::{CacheLifetime, Credentials};
use serde_json::{Value, json};

use super::candidates;
use crate::accounts::Accounts;
use crate::accounts::store::FileStore;
use crate::config::Config;
use crate::options::limits::Endpoints;
use crate::options::{harness_of, offering};
use crate::pools::policy;
use coqui_acp_pool::{Ask, SharedMemory, Standing, Why};

pub(crate) struct Dir(pub(crate) PathBuf);

impl Dir {
    pub(crate) fn new() -> Self {
        static NEXT: AtomicU64 = AtomicU64::new(0);
        let dir = std::env::temp_dir().join(format!(
            "canoa-pool-turn-{}-{}",
            std::process::id(),
            NEXT.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir_all(&dir).expect("a directory");
        Self(dir)
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// A usage endpoint on loopback that answers the document of the token the request carries.
pub(crate) fn standin(documents: Vec<(&'static str, Value)>) -> String {
    let listener = TcpListener::bind("127.0.0.1:0").expect("binds");
    let address = format!("http://{}", listener.local_addr().expect("an address"));
    std::thread::spawn(move || {
        for stream in listener.incoming() {
            let Ok(mut stream) = stream else { return };
            let documents = documents.clone();
            std::thread::spawn(move || {
                let mut head = Vec::new();
                let mut byte = [0_u8; 1];
                while !head.ends_with(b"\r\n\r\n") {
                    if stream.read(&mut byte).unwrap_or(0) == 0 {
                        return;
                    }
                    if head.is_empty() && !byte[0].is_ascii_alphabetic() {
                        return;
                    }
                    head.push(byte[0]);
                }
                let head = String::from_utf8_lossy(&head).into_owned();
                let body = documents
                    .iter()
                    .find(|(token, _)| head.contains(&format!("Bearer {token}\r\n")))
                    .map_or_else(|| json!({}), |(_, document)| document.clone())
                    .to_string();
                let _ = write!(
                    stream,
                    "HTTP/1.1 200 X\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n{body}",
                    body.len()
                );
            });
        }
    });
    address
}

pub(crate) fn login_of(token: &str) -> String {
    json!({"claudeAiOauth": {
        "accessToken": token,
        "refreshToken": format!("{token}-refresh"),
        "expiresAt": 4_102_444_800_000_i64,
        "scopes": ["user:inference", "user:profile"],
    }})
    .to_string()
}

const CONFIG: &str = "[host.agents.Claude]\nharness = \"claude\"\naccounts = [\"both\"]\n\
    [host.accounts.work]\nkind = \"claude-login\"\nid = \"id-work\"\n\
    [host.accounts.work2]\nkind = \"claude-login\"\nid = \"id-work2\"\n\
    [host.pools.both]\nstandard = \"claude-code\"\naccounts = [\"work\", \"work2\"]\n";

/// The pool's accounts as candidates, read against the stand-in: `work` reports `work_doc` and `work2` reports `work2_doc`.
fn read(
    work_doc: Value,
    work2_doc: Value,
    model: Option<&str>,
    current: Option<&str>,
) -> Vec<Standing> {
    let dir = Dir::new();
    let root = dir.0.join("accounts");
    let store = Accounts::with_store(root.clone(), Box::new(FileStore::new(root))).expect("opens");
    for (id, token) in [("id-work", "tok-work"), ("id-work2", "tok-work2")] {
        store
            .store_login(
                id,
                &Credentials::from_iter([("credentials_file", login_of(token))]),
                None,
            )
            .expect("stored");
    }
    let address = standin(vec![("tok-work", work_doc), ("tok-work2", work2_doc)]);
    let config = Config::from_str_in(CONFIG, std::path::Path::new("")).expect("parses");
    let entry = config.entry("Claude").expect("an entry");
    let built = harness_of(entry, Arc::default()).expect("a provider");
    let built = offering(&config, entry, built, &Arc::clone(&store), true).expect("offered");
    let pool = built.pool_named("both", true).expect("the pool").clone();
    let names: Vec<String> = pool
        .usable(true)
        .map(|member| member.name.clone())
        .collect();
    let endpoints = Endpoints::default().with("claude", address);
    candidates(
        &config,
        &built,
        &names,
        (model, current),
        (&[], &endpoints),
        true,
    )
}

fn turn<'a>(current: Option<&'a str>, model: Option<&'a str>, now: i64) -> Ask<'a> {
    let mut ask = Ask::new(now).lifetime(CacheLifetime::Long);
    if let Some(current) = current {
        ask = ask.current(current).idle(std::time::Duration::from_secs(5));
    }
    if let Some(model) = model {
        ask = ask.model(model);
    }
    ask
}

/// The account `ask` is given by the pool of `work` and `work2`, in a memory of the test's own.
fn decide(
    memory: &SharedMemory,
    standings: &[Standing],
    ask: &Ask<'_>,
) -> Result<coqui_acp_pool::Chosen, String> {
    let names = ["work".to_owned(), "work2".to_owned()];
    policy::decide_with(memory, "both", &names, standings, ask)
}

const NOW: i64 = 4_000_000_000;

#[test]
fn an_account_whose_provider_reports_its_allowance_used_up_is_left_for_the_one_with_room() {
    let out = json!({"five_hour": {"utilization": 100.0, "resets_at": "2099-01-01T05:00:00Z"}});
    let room = json!({"five_hour": {"utilization": 12.5, "resets_at": "2099-01-01T05:00:00Z"}});
    let accounts = read(out, room, None, None);
    assert_eq!(accounts.len(), 2);
    assert!(
        accounts.iter().all(|each| each.unavailable.is_none()),
        "{accounts:?}"
    );
    // A session that starts goes to the one with room, and a session on the other moves to it and says why.
    let memory = SharedMemory::new();
    let start = decide(&memory, &accounts, &turn(None, None, NOW)).expect("room");
    assert_eq!((start.member.as_str(), start.why), ("work2", Why::Start));
    let on_work = decide(&memory, &accounts, &turn(Some("work"), None, NOW)).expect("room");
    assert_eq!(
        (on_work.member.as_str(), on_work.moved, on_work.why),
        ("work2", true, Why::Exhausted)
    );
    assert_eq!(
        on_work.left.as_deref(),
        Some("Session (5 hours) is used up")
    );
}

#[test]
fn every_account_out_of_usage_refuses_the_turn_and_names_when_the_soonest_refills() {
    let soon = json!({"five_hour": {"utilization": 100.0, "resets_at": "2099-01-01T05:00:00Z"}});
    let later = json!({"seven_day": {"utilization": 100.0, "resets_at": "2099-01-05T00:00:00Z"}});
    let accounts = read(soon, later, None, None);
    let why =
        decide(&SharedMemory::new(), &accounts, &turn(None, None, NOW)).expect_err("none has room");
    assert!(why.contains("pool both") && why.contains("work2"), "{why}");
    assert!(why.contains("2099-01-01 05:00 UTC"), "{why}");
}

#[test]
fn a_bucket_for_one_model_stops_the_account_for_that_model_only() {
    let sonnet_out = json!({
        "five_hour": {"utilization": 10.0, "resets_at": "2099-01-01T05:00:00Z"},
        "seven_day_sonnet": {"utilization": 100.0, "resets_at": "2099-01-05T00:00:00Z"},
    });
    let room = json!({"five_hour": {"utilization": 10.0, "resets_at": "2099-01-01T05:00:00Z"}});
    let accounts = read(sonnet_out, room, None, None);
    let memory = SharedMemory::new();
    let on = |model: Option<&str>| {
        decide(&memory, &accounts, &turn(None, model, NOW))
            .expect("room")
            .member
    };
    assert_eq!(on(Some("sonnet")), "work2");
    // For another model the first account has room: it is the next in turn after the one that just took the sonnet turn.
    assert_eq!(on(Some("opus")), "work");
}
