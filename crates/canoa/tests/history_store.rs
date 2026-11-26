//! The `history_store` kind: the contract on the in-memory store, and what a description and a migration carry.
#![cfg(feature = "history-store")]

use canoa::history_store::memory::{MemoryDb, MemoryStore};
use canoa::history_store::schema::{self, TURNS};
use canoa::history_store::{HistoryStore, OpenRequest, contract};

#[test]
fn the_memory_store_passes_the_contract() {
    contract::run(&MemoryDb::new());
}

#[test]
fn the_memory_store_is_opened_through_its_provider() {
    let provider = MemoryStore;
    let info = provider.describe().expect("described");
    assert_eq!(info.problem(), None);
    let db = provider
        .open(OpenRequest::new("{}", "unused"))
        .expect("opened");
    contract::run(&*db);
}

#[test]
fn a_turn_carries_every_property_of_the_turn_record() {
    let turns = schema::table(TURNS).expect("the turns table");
    for name in [
        "harness",
        "agent",
        "account",
        "model",
        "effort",
        "role",
        "pool",
        "member",
        "profile",
        "why",
        "moved_from",
        "usage_known",
        "tokens_input",
        "tokens_output",
        "tokens_cached",
        "cost_micros",
        "cost_currency",
        "duration_ms",
        "limited",
        "moved_on",
        "tags",
    ] {
        let column = turns.column(name).unwrap_or_else(|| panic!("turns.{name}"));
        assert!(column.nullable, "{name} is null when it was not reported");
    }
}
