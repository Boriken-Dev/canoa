//! The `config_store` kind: the contract every store meets, run on the in-memory one, and what a layer below the writable one means.
#![cfg(feature = "config-store")]

use canoa::config_store::{
    Changes, ConfigStore, Key, MemoryStore, Op, StoreInfo, contract, json_of, value_of,
};
use serde_json::json;

#[test]
fn the_memory_store_meets_the_contract() {
    contract::check(&MemoryStore::new());
}

#[test]
fn a_lower_layer_is_read_and_shown_with_its_label_and_never_removed_from_above() {
    let store = MemoryStore::new().with_lower(
        "installed",
        "the installed canoa.cfg",
        json!({"host": {"agents": {"claude": {"harness": "claude"}}}}),
    );
    contract::check(&store);
    let found = store
        .get(Key::new(["host", "agents", "claude"]))
        .expect("get");
    assert_eq!(found.layer.as_deref(), Some("installed"));
    let refused = store
        .change(Changes::of(vec![Op::remove(["host", "agents", "claude"])]))
        .expect_err("the writable layer does not hold it");
    assert!(
        refused.to_string().contains("the installed canoa.cfg"),
        "the refusal says where it comes from: {refused}"
    );
    // The writable layer's own value wins and is what a removal then takes out.
    store
        .change(Changes::of(vec![Op::set(
            ["host", "agents", "claude"],
            value_of(&json!({"harness": "claude", "title": "mine"})),
        )]))
        .expect("an own entry");
    let found = store
        .get(Key::new(["host", "agents", "claude"]))
        .expect("get");
    assert_eq!(found.layer.as_deref(), Some("own"));
    assert_eq!(
        json_of(&found.value.expect("value")).expect("json")["title"],
        "mine"
    );
}

#[test]
fn a_description_with_no_writable_layer_is_refused() {
    assert!(StoreInfo::new("t", Vec::new()).problem().is_some());
}
