//! What every [`ConfigStore`] does, written once as a check a provider's own tests run against a store it made, so a provider is
//! known by the same behaviour as every other.

use alloc::vec;

use serde_json::json;

use super::{Changes, ConfigStore, Key, Op, json_of, value_of};

fn held(store: &dyn ConfigStore, path: &[&str]) -> Option<serde_json::Value> {
    store
        .get(Key::new(path.iter().copied()))
        .expect("get")
        .value
        .as_ref()
        .and_then(json_of)
}

/// Runs the contract against `store`, which holds nothing under the table `contract` and whose writable layer may be written.
///
/// # Panics
///
/// At the first thing the store does that a store must not.
pub fn check(store: &dyn ConfigStore) {
    let info = store.describe().expect("describe");
    assert_eq!(info.problem(), None, "the description is usable");
    let own = info.writable().expect("one writable layer").to_owned();
    assert!(held(store, &["contract"]).is_none());

    // A value is set, read back with the layer it came from, and moves the revision.
    let before = store.revision().expect("revision");
    store
        .change(Changes::of(vec![Op::set(
            ["contract", "table", "key"],
            value_of(&json!("v")),
        )]))
        .expect("set");
    assert_eq!(held(store, &["contract", "table", "key"]), Some(json!("v")));
    let found = store
        .get(Key::new(["contract", "table", "key"]))
        .expect("get");
    assert_eq!(found.layer.as_deref(), Some(own.as_str()));
    assert_ne!(
        store.revision().expect("revision"),
        before,
        "a change moves the revision"
    );
    let snapshot = store.read().expect("read");
    assert_eq!(snapshot.revision, store.revision().expect("revision"));
    let layer = snapshot
        .layers
        .iter()
        .find(|layer| layer.id == own)
        .expect("the writable layer is read");
    let document = json_of(&layer.document).expect("a document");
    assert_eq!(document["contract"]["table"]["key"], json!("v"));

    // A table is set whole, a sibling stays, and a removal takes out one path.
    store
        .change(Changes::of(vec![
            Op::set(
                ["contract", "other"],
                value_of(&json!({"a": 1, "b": [1, 2]})),
            ),
            Op::remove(["contract", "table", "key"]),
        ]))
        .expect("two ops");
    assert!(held(store, &["contract", "table", "key"]).is_none());
    assert_eq!(
        held(store, &["contract", "other"]),
        Some(json!({"a": 1, "b": [1, 2]}))
    );

    // Removing what is not there is no error and changes nothing.
    let steady = store.revision().expect("revision");
    store
        .change(Changes::of(vec![Op::remove(["contract", "nowhere", "x"])]))
        .expect("absent removal");
    assert_eq!(store.revision().expect("revision"), steady);

    // All or none: a change that cannot be made in its second op leaves the first unmade.
    let refused = store.change(Changes::of(vec![
        Op::set(["contract", "atomic"], value_of(&json!(1))),
        Op::set(["contract", "other", "a", "deeper"], value_of(&json!(2))),
    ]));
    assert!(refused.is_err(), "a name that is a value is no table");
    assert!(
        held(store, &["contract", "atomic"]).is_none(),
        "none of it was made"
    );
    assert_eq!(store.revision().expect("revision"), steady);

    // A check answers what would be held and writes nothing.
    let mut dry = Changes::of(vec![Op::set(["contract", "dry"], value_of(&json!(true)))]);
    dry.check_only = true;
    let would = store.change(dry).expect("check");
    assert_eq!(
        json_of(&would.document).expect("a document")["contract"]["dry"],
        json!(true)
    );
    assert!(held(store, &["contract", "dry"]).is_none());
    assert_eq!(store.revision().expect("revision"), steady);

    // A write made from a revision the store has left is refused.
    let mut stale = Changes::of(vec![Op::set(["contract", "late"], value_of(&json!(1)))]);
    stale.expect_revision = Some(before);
    assert!(store.change(stale).is_err());
    let mut fresh = Changes::of(vec![Op::set(["contract", "late"], value_of(&json!(1)))]);
    fresh.expect_revision = Some(steady);
    store.change(fresh).expect("from the current revision");

    // A path names a key.
    assert!(store.get(Key::new::<&str>([])).is_err());
    assert!(
        store
            .change(Changes::of(vec![Op::remove::<&str>([])]))
            .is_err()
    );
    store
        .change(Changes::of(vec![Op::remove(["contract"])]))
        .expect("clean up");
    assert!(held(store, &["contract"]).is_none());
}
