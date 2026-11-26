//! `content/stat`, `content/list`, `content/read`, `content/resolve` and `content/cache`, through the control channel.

use std::path::PathBuf;
use std::sync::Arc;

use serde_json::json;

use super::overlays_tests::{ask, dir, key};
use super::*;
use crate::store::programs::Programs;
use crate::store::shelf::Shelf;
use crate::store::sources::Sources;

struct Rig {
    tmp: PathBuf,
}

impl Rig {
    fn new(name: &str) -> Self {
        Self { tmp: dir(name) }
    }

    fn control(&self, cache_bytes: Option<u64>) -> Control {
        let mut sources = Sources::built_in().content_in(
            Some(&self.tmp.join("content")),
            None,
            cache_bytes,
            std::time::Duration::from_secs(3600),
        );
        sources
            .add(Some("demo_ext"), crate::content::tests::demo_registry())
            .expect("added");
        let programs = Programs::new(
            Shelf::unchecked(self.tmp.join("programs")),
            crate::store::registry::built_in().expect("the built-in stores"),
        );
        Control::open(
            None,
            DEFAULT_MAX_RECORDS,
            DEFAULT_MAX_RECORD_BYTES,
            Vec::new(),
        )
        .expect("opens")
        .with_stores(Stores::new(Arc::new(programs), Vec::new()).with_sources(sources))
    }
}

impl Drop for Rig {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.tmp);
    }
}

#[test]
fn a_document_is_stat_ed_listed_read_and_resolved_through_its_scheme() {
    let rig = Rig::new("content-read");
    let control = rig.control(None);
    let owner = Caller::Owner;

    let read = ask(
        &control,
        &owner,
        "content/read",
        json!({"uri": "data:,hello%20there"}),
    );
    assert_eq!(read["result"]["text"], "hello there", "{read}");
    assert_eq!(read["result"]["from"], "fetched");
    assert_eq!(read["result"]["truncated"], false);
    // A data URI is immutable, so what was kept never goes stale; the second read is the cache's.
    let again = ask(
        &control,
        &owner,
        "content/read",
        json!({"uri": "data:,hello%20there"}),
    );
    assert_eq!(again["result"]["from"], "cache");
    let fresh = ask(
        &control,
        &owner,
        "content/read",
        json!({"uri": "data:,hello%20there", "refresh": true}),
    );
    assert_eq!(fresh["result"]["from"], "fetched");
    // The preview is cut where the caller says, and says it was.
    let cut = ask(
        &control,
        &owner,
        "content/read",
        json!({"uri": "data:,hello%20there", "max_bytes": 5}),
    );
    assert_eq!(cut["result"]["text"], "hello");
    assert_eq!(cut["result"]["truncated"], true);

    let stat = ask(
        &control,
        &owner,
        "content/stat",
        json!({"uri": "data:,abc"}),
    );
    assert_eq!(stat["result"]["stat"]["size"], 3, "{stat}");
    // The digest of a `data:` URI is derived, so no publisher states one.
    assert_eq!(stat["result"]["badge"], serde_json::Value::Null);
    assert_eq!(
        stat["result"]["stat"]["digests"][0]["provenance"],
        "derived"
    );

    let listed = ask(&control, &owner, "content/list", json!({"uri": "demo://x"}));
    let names: Vec<&str> = listed["result"]["entries"]
        .as_array()
        .expect("entries")
        .iter()
        .filter_map(|entry| entry["path"].as_str())
        .collect();
    assert_eq!(names, ["a.txt", "sub"], "{listed}");
    let resolved = ask(
        &control,
        &owner,
        "content/resolve",
        json!({"uri": "demo://x"}),
    );
    assert_eq!(resolved["result"]["revision"], "abc");
    assert_eq!(resolved["result"]["immutable"], true);

    let nobody = ask(&control, &owner, "content/read", json!({"uri": "hf://o/n"}));
    assert!(
        nobody["error"]["message"]
            .as_str()
            .is_some_and(|why| why.contains("no provider offers hf:")),
        "{nobody}"
    );
    let missing = ask(&control, &owner, "content/read", json!({}));
    assert!(missing["error"].is_object());
}

#[test]
fn the_cache_says_what_it_keeps_is_cut_to_its_size_and_is_cleared_on_request() {
    let rig = Rig::new("content-cache");
    let control = rig.control(Some(150));
    let owner = Caller::Owner;
    let empty = ask(&control, &owner, "content/cache", json!({}));
    assert_eq!(empty["result"]["entries"], 0);
    assert_eq!(empty["result"]["max_bytes"], 150);

    let read = |text: &str| {
        ask(
            &control,
            &owner,
            "content/read",
            json!({"uri": format!("data:,{text}")}),
        )
    };
    read("one");
    let kept = ask(&control, &owner, "content/cache", json!({}));
    assert_eq!(kept["result"]["entries"], 1, "{kept}");
    // Each kept document takes its header and its body, so a third pushes the oldest out of the 150 bytes.
    for text in ["two", "three"] {
        std::thread::sleep(std::time::Duration::from_millis(50));
        read(text);
    }
    let cut = ask(&control, &owner, "content/cache", json!({}));
    assert!(
        cut["result"]["bytes"]
            .as_u64()
            .is_some_and(|bytes| bytes <= 150),
        "{cut}"
    );
    assert_eq!(cut["result"]["entries"], 2, "{cut}");

    let cleared = ask(&control, &owner, "content/cache", json!({"clear": true}));
    assert_eq!(cleared["result"]["entries"], 0);
    assert!(cleared["result"]["cleared"].as_u64().is_some());
}

#[test]
fn reading_a_uri_takes_the_hosting_grant() {
    let rig = Rig::new("content-read-grant");
    let control = rig.control(None);
    let host = key("sha256:aa", &[Grant::Hosting]);
    assert!(ask(&control, &host, "content/read", json!({"uri": "data:,x"}))["result"].is_object());
    for method in [
        "content/read",
        "content/stat",
        "content/list",
        "content/resolve",
        "content/cache",
    ] {
        let none = key("sha256:bb", &[Grant::ConfigRead, Grant::OverlayRead]);
        assert_eq!(
            ask(&control, &none, method, json!({"uri": "data:,x"}))["error"]["code"],
            DENIED,
            "{method}"
        );
    }
}
