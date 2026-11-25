use std::collections::BTreeMap;
use std::io::Cursor;
use std::path::PathBuf;
use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use canoa::store::source::{CatalogSourceProvider, ReadRequest, SourceFetch, SourceInfo};
use coqui_acp_api::ProcessSpawner;
use coqui_acp_api::guatiao::Provider;
use coqui_acp_api::guatiao::library::{Object, ProviderError, Registry};
use serde_json::{Map, Value, json};

use super::{Run, kept_file};
use crate::store::catalogs::{Entry, Origin, TypeCatalogs, merge};
use crate::store::sources::{Fetching, Sources};
use crate::store::tools::{Reply, ToolsLimits, Transport};

/// What the stand-in source answers for each label, how often it was asked, and which labels it fails for.
static ANSWERS: Mutex<BTreeMap<String, String>> = Mutex::new(BTreeMap::new());
static ASKED: Mutex<BTreeMap<String, usize>> = Mutex::new(BTreeMap::new());
static DOWN: Mutex<Vec<String>> = Mutex::new(Vec::new());

fn answer(label: &str, text: &str) {
    ANSWERS
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .insert(label.to_owned(), text.to_owned());
}

fn asked(label: &str) -> usize {
    ASKED
        .lock()
        .unwrap_or_else(|e| e.into_inner())
        .get(label)
        .copied()
        .unwrap_or(0)
}

fn down(label: &str, is_down: bool) {
    let mut down = DOWN.lock().unwrap_or_else(|e| e.into_inner());
    down.retain(|each| each != label);
    if is_down {
        down.push(label.to_owned());
    }
}

/// A source that answers what the test set for the label, and counts its reads.
#[derive(Default, Provider)]
#[provider(CatalogSourceProvider, id = "models_standin", crate = ::coqui_acp_api::guatiao)]
struct Standin;

impl CatalogSourceProvider for Standin {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        Ok(SourceInfo::new("models_standin", "A stand-in"))
    }

    fn read(
        &self,
        request: ReadRequest,
        _spawner: Object<dyn ProcessSpawner>,
        _fetch: Object<dyn SourceFetch>,
    ) -> Result<String, ProviderError> {
        *ASKED
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .entry(request.label.clone())
            .or_default() += 1;
        if DOWN
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .contains(&request.label)
        {
            return Err(canoa::status::internal("the stand-in is down"));
        }
        ANSWERS
            .lock()
            .unwrap_or_else(|e| e.into_inner())
            .get(&request.label)
            .cloned()
            .ok_or_else(|| canoa::status::internal("nothing set for this label"))
    }
}

/// A source that reads the document its entry names, through the fetch it is handed, and says what it got.
#[derive(Default, Provider)]
#[provider(CatalogSourceProvider, id = "hub_standin", crate = ::coqui_acp_api::guatiao)]
struct Hub;

impl CatalogSourceProvider for Hub {
    fn describe(&self) -> Result<SourceInfo, ProviderError> {
        Ok(SourceInfo::new("hub_standin", "A hub").fetching(["https://hub.example/api/"]))
    }

    fn read(
        &self,
        request: ReadRequest,
        _spawner: Object<dyn ProcessSpawner>,
        fetch: Object<dyn SourceFetch>,
    ) -> Result<String, ProviderError> {
        let fields: Value = serde_json::from_str(&request.fields).unwrap_or(Value::Null);
        let url = if fields["reach"] == "elsewhere" {
            "https://elsewhere.example/models.json".to_owned()
        } else {
            fields["url"]
                .as_str()
                .unwrap_or("https://hub.example/api/models.json")
                .to_owned()
        };
        fetch.get_text(&url, 1 << 20)
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-catalog-run-standins",
    version = "0",
    providers = [Standin, Hub]
);

/// A world of documents by address, which counts what it is asked.
#[derive(Debug, Default)]
struct World {
    docs: Mutex<BTreeMap<String, Vec<u8>>>,
    asked: AtomicUsize,
}

impl Transport for World {
    fn get(&self, url: &str, _limits: &ToolsLimits) -> Result<Reply, String> {
        self.asked.fetch_add(1, Ordering::SeqCst);
        let docs = self.docs.lock().map_err(|_| "poisoned".to_owned())?;
        let (status, bytes) = docs
            .get(url)
            .map_or((404, Vec::new()), |bytes| (200, bytes.clone()));
        Ok(Reply {
            status,
            location: None,
            body: Box::new(Cursor::new(bytes)),
        })
    }
}

fn sources(world: &Arc<World>) -> Sources {
    let mut registry = Registry::new("canoa", "0");
    let loading = registry
        .register_local("standins", library)
        .expect("registers");
    assert!(loading.loaded().is_some());
    let mut sources = Sources::built_in().fetching(Fetching::through(
        Arc::clone(world) as Arc<dyn Transport>,
        Duration::from_secs(1),
        Duration::from_secs(1),
    ));
    sources
        .add(Some("standins"), Arc::new(registry))
        .expect("added");
    sources
}

const SHA: &str = "60ed5bc3dd14eea856493d334349b405782ddcaf0028d4b5df4088345fba2efe";

fn models(revision: &str, ids: &[&str]) -> String {
    let items: Vec<Value> = ids
        .iter()
        .map(|id| {
            json!({"id": id, "label": id, "files": [{
                "url": format!("https://hub.example/{id}.bin"), "file": format!("{id}.bin"),
                "check": {"sha256": SHA}}]})
        })
        .collect();
    json!({"format": "canoa.models/1", "revision": revision, "items": items}).to_string()
}

fn kind() -> TypeCatalogs {
    TypeCatalogs {
        id: "whisper".to_owned(),
        title: "Whisper".to_owned(),
        noun: "source".to_owned(),
        plural: "sources".to_owned(),
        lists: "models".to_owned(),
        defaults: Vec::new(),
        fixed: None,
        format: Some("canoa.models/1".to_owned()),
    }
}

fn entry(label: &str, provider: &str) -> Entry {
    let table = json!({label: {"provider": provider}});
    let merged = merge(&kind(), &table);
    merged
        .into_iter()
        .find_map(|each| each.parsed.ok())
        .expect("an entry")
}

fn folder(name: &str) -> PathBuf {
    let dir = std::env::temp_dir().join(format!("canoa-catalog-run-{name}-{}", std::process::id()));
    let _ = std::fs::remove_dir_all(&dir);
    std::fs::create_dir_all(&dir).expect("a folder");
    dir
}

fn run(dir: &std::path::Path, world: &Arc<World>, ttl: u64) -> Run {
    Run {
        dir: Some(dir.to_path_buf()),
        ttl: Duration::from_secs(ttl),
        sources: sources(world),
    }
}

#[test]
fn a_catalog_is_asked_once_while_it_is_fresh_and_again_when_it_is_stale_or_refreshed() {
    let dir = folder("fresh");
    let world = Arc::new(World::default());
    answer("fresh-a", &models("r1", &["base"]));
    let entry = entry("fresh-a", "models_standin");
    let reader = run(&dir, &world, 3600);
    let first = reader.read(&kind(), &entry, false).expect("read");
    assert_eq!(first.from, "fetched");
    let again = reader.read(&kind(), &entry, false).expect("kept");
    assert_eq!(
        (again.from, again.text.as_str()),
        ("cache", first.text.as_str())
    );
    assert_eq!(asked("fresh-a"), 1, "a fresh answer asks nobody");
    assert!(kept_file(&dir, "whisper", "fresh-a").is_file());

    answer("fresh-a", &models("r2", &["base", "small"]));
    let refreshed = reader.read(&kind(), &entry, true).expect("refreshed");
    assert!(refreshed.text.contains("small"), "a refresh asks again");
    let stale = run(&dir, &world, 0);
    assert_eq!(
        stale.read(&kind(), &entry, false).expect("read").from,
        "fetched"
    );
    assert_eq!(asked("fresh-a"), 3);
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_source_that_fails_leaves_the_last_answer_with_the_reason_and_none_leaves_the_error() {
    let dir = folder("failing");
    let world = Arc::new(World::default());
    answer("fail-a", &models("r1", &["base"]));
    let entry = entry("fail-a", "models_standin");
    let reader = run(&dir, &world, 0);
    reader.read(&kind(), &entry, false).expect("read once");
    down("fail-a", true);
    let kept = reader
        .read(&kind(), &entry, false)
        .expect("the last good one");
    assert!(kept.stale && kept.from == "cache");
    assert!(
        kept.problem
            .is_some_and(|why| why.contains("the stand-in is down"))
    );
    assert!(kept.text.contains("base"));

    let never = entry_named("fail-never");
    down("fail-never", true);
    let none = run(&folder("failing-none"), &world, 0).read(&kind(), &never, false);
    assert!(none.is_err_and(|why| why.contains("the stand-in is down")));
    let _ = std::fs::remove_dir_all(dir);
}

fn entry_named(label: &str) -> Entry {
    entry(label, "models_standin")
}

#[test]
fn a_pinned_default_answers_with_its_shipped_snapshot_and_asks_nobody() {
    let dir = folder("snapshot");
    let world = Arc::new(World::default());
    answer("shipped", &models("live", &["live"]));
    let mut pinned = entry("shipped", "models_standin");
    pinned.origin = Origin::Default;
    pinned.pin = Some("5359861c".to_owned());
    pinned.snapshot = Some(models("5359861c", &["base"]));
    let reader = run(&dir, &world, 0);
    let first = reader.read(&kind(), &pinned, false).expect("the snapshot");
    assert_eq!((first.from, first.fetched), ("snapshot", 0));
    assert!(first.text.contains("5359861c") && !first.stale);
    assert_eq!(asked("shipped"), 0, "a pinned default needs no network");
    assert_eq!(
        reader.peek(&kind(), &pinned),
        json!({"fetched": 0, "from": "snapshot"})
    );

    let live = reader
        .read(&kind(), &pinned, true)
        .expect("asked on request");
    assert_eq!(live.from, "fetched");
    assert_eq!(asked("shipped"), 1);
    let after = reader.read(&kind(), &pinned, false).expect("kept");
    assert_eq!(
        after.from, "cache",
        "a pinned catalog never goes stale by time"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_digest_pin_refuses_a_catalog_that_is_not_the_one_it_names_and_keeps_nothing() {
    use sha2::Digest as _;

    let dir = folder("digest");
    let world = Arc::new(World::default());
    let text = models("r1", &["base"]);
    answer("digest-a", &text);
    let mut pinned = entry("digest-a", "models_standin");
    pinned.pin = Some(format!("sha256:{}", "00".repeat(32)));
    let reader = run(&dir, &world, 3600);
    let refused = reader.read(&kind(), &pinned, false);
    assert!(refused.is_err_and(|why| why.contains("its pin names")));
    assert!(!kept_file(&dir, "whisper", "digest-a").exists());

    let hex = data_encoding::HEXLOWER.encode(&sha2::Sha256::digest(text.as_bytes()));
    pinned.pin = Some(format!("sha256:{hex}"));
    assert_eq!(
        reader
            .read(&kind(), &pinned, false)
            .expect("the pinned one")
            .from,
        "fetched"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn what_is_not_a_document_of_the_format_is_refused_and_a_replaced_entry_does_not_read_the_old_one()
{
    let dir = folder("format");
    let world = Arc::new(World::default());
    answer(
        "format-a",
        "{\"format\":\"canoa.releases/1\",\"releases\":[]}",
    );
    let reader = run(&dir, &world, 3600);
    let wrong = reader.read(&kind(), &entry("format-a", "models_standin"), false);
    assert!(wrong.is_err_and(|why| why.contains("canoa.releases/1")));

    answer("format-b", &models("r1", &["base"]));
    let mut mine = entry("format-b", "models_standin");
    reader.read(&kind(), &mine, false).expect("read");
    mine.fields
        .insert("note".to_owned(), json!("another entry now"));
    answer("format-b", &models("r2", &["small"]));
    let again = reader.read(&kind(), &mine, false).expect("read again");
    assert!(
        again.text.contains("small") && again.from == "fetched",
        "what an entry says is part of what was kept for it"
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_source_fetches_only_from_its_endpoints_and_the_address_its_entry_names() {
    let world = Arc::new(World::default());
    {
        let mut docs = world.docs.lock().expect("docs");
        docs.insert(
            "https://hub.example/api/models.json".to_owned(),
            models("hub", &["base"]).into_bytes(),
        );
        docs.insert(
            "https://mirror.example/own/models.json".to_owned(),
            models("mirror", &["small"]).into_bytes(),
        );
        docs.insert(
            "https://elsewhere.example/models.json".to_owned(),
            models("elsewhere", &["x"]).into_bytes(),
        );
    }
    let dir = folder("fetch");
    let reader = run(&dir, &world, 3600);
    let own = entry("hub-own", "hub_standin");
    assert!(
        reader
            .read(&kind(), &own, false)
            .expect("its declared endpoint")
            .text
            .contains("\"hub\"")
    );

    let mut table = Map::new();
    table.insert("provider".to_owned(), json!("hub_standin"));
    table.insert(
        "url".to_owned(),
        json!("https://mirror.example/own/models.json"),
    );
    let mirror = merge(&kind(), &json!({"mirror": Value::Object(table)}))
        .into_iter()
        .find_map(|each| each.parsed.ok())
        .expect("an entry");
    assert!(
        reader
            .read(&kind(), &mirror, false)
            .expect("the address the entry names")
            .text
            .contains("mirror")
    );

    // An address that is neither the source's own nor written into the entry is out of reach, and nothing is asked of it.
    let stray = merge(
        &kind(),
        &json!({"stray": {"provider": "hub_standin", "reach": "elsewhere"}}),
    )
    .into_iter()
    .find_map(|each| each.parsed.ok())
    .expect("an entry");
    let before = world.asked.load(Ordering::SeqCst);
    let refused = run(&folder("fetch-2"), &world, 3600).read(&kind(), &stray, false);
    assert!(refused.is_err(), "{refused:?}");
    assert_eq!(world.asked.load(Ordering::SeqCst), before);

    // A bare address is read by the host from that address alone.
    let bare = merge(
        &kind(),
        &json!({"bare": "https://elsewhere.example/models.json"}),
    )
    .into_iter()
    .find_map(|each| each.parsed.ok())
    .expect("an entry");
    assert!(
        run(&folder("fetch-3"), &world, 3600)
            .read(&kind(), &bare, false)
            .expect("read by the host")
            .text
            .contains("elsewhere")
    );
    let _ = std::fs::remove_dir_all(dir);
}

#[test]
fn a_kept_catalog_past_its_time_is_asked_again_by_the_renewal_and_one_never_read_is_not() {
    let dir = folder("renewal");
    let world = Arc::new(World::default());
    answer("renew-a", &models("r1", &["base"]));
    let entry = entry("renew-a", "models_standin");
    // A time of zero makes whatever is kept past it.
    let reader = run(&dir, &world, 0);
    reader.renew(&kind(), &entry);
    assert_eq!(
        asked("renew-a"),
        0,
        "nothing was ever read, so nothing is asked"
    );
    reader.read(&kind(), &entry, false).expect("read once");
    assert_eq!(asked("renew-a"), 1);

    answer("renew-a", &models("r2", &["base", "small"]));
    reader.renew(&kind(), &entry);
    assert_eq!(asked("renew-a"), 2, "asked with no request for the catalog");
    let now = run(&dir, &world, 3600)
        .read(&kind(), &entry, false)
        .expect("kept");
    assert!(
        now.text.contains("small"),
        "the renewal kept the new answer"
    );

    // A source that fails leaves what was kept, and the next read says why.
    down("renew-a", true);
    reader.renew(&kind(), &entry);
    let failed = run(&dir, &world, 3600)
        .read(&kind(), &entry, false)
        .expect("kept");
    assert!(failed.text.contains("small"), "the last good answer stands");
    let _ = std::fs::remove_dir_all(dir);
}
