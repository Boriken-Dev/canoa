//! The `history_store` kind: the contract on the in-memory store, and what a description and a migration carry.
#![cfg(feature = "history-store")]

use canoa::history_store::memory::{MemoryDb, MemoryStore};
use canoa::history_store::schema::{self, TURNS};
use canoa::history_store::{
    Aggregate, Compacted, HistoryDb, HistoryStore, Migration, OpenRequest, Rows, SchemaState,
    Select, Sql, Stats, Write, Written, contract,
};
use coqui_acp_api::guatiao::library::ProviderError;

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

/// A store that is the memory store except that a batch is applied one change at a time, so a failure part way keeps what came
/// before it.
struct NotAtomic(MemoryDb);

impl HistoryDb for NotAtomic {
    fn schema(&self) -> Result<SchemaState, ProviderError> {
        self.0.schema()
    }
    fn migrate(&self, migration: Migration) -> Result<(), ProviderError> {
        self.0.migrate(migration)
    }
    fn select(&self, select: Select) -> Result<Rows, ProviderError> {
        self.0.select(select)
    }
    fn count(&self, select: Select) -> Result<u64, ProviderError> {
        self.0.count(select)
    }
    fn aggregate(&self, aggregate: Aggregate) -> Result<Rows, ProviderError> {
        self.0.aggregate(aggregate)
    }
    fn write(&self, write: Write) -> Result<Written, ProviderError> {
        let mut affected = 0;
        for op in write.ops {
            affected += self.0.write(Write { ops: vec![op] })?.affected;
        }
        Ok(Written { affected })
    }
    fn sql(&self, sql: Sql) -> Result<Rows, ProviderError> {
        self.0.sql(sql)
    }
    fn revision(&self) -> Result<String, ProviderError> {
        self.0.revision()
    }
    fn stats(&self) -> Result<Stats, ProviderError> {
        self.0.stats()
    }
    fn compact(&self) -> Result<Compacted, ProviderError> {
        self.0.compact()
    }
    fn release(&self) -> Result<(), ProviderError> {
        self.0.release()
    }
}

#[test]
#[allow(clippy::std_instead_of_core, reason = "catch_unwind is std's")]
fn the_contract_catches_a_provider_whose_batch_is_not_atomic() {
    let broken = NotAtomic(MemoryDb::new());
    let caught = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| contract::run(&broken)))
        .expect_err("the contract passed a store that keeps half a batch");
    let said = caught
        .downcast_ref::<String>()
        .cloned()
        .or_else(|| caught.downcast_ref::<&str>().map(|s| (*s).to_owned()))
        .unwrap_or_default();
    assert!(
        said.contains("a batch with a failed op leaves nothing"),
        "{said}"
    );
}

#[test]
fn a_store_of_a_newer_schema_is_read_only_and_one_of_none_is_made() {
    use canoa::history_store::schema::{Fit, VERSION, fit};
    assert_eq!(fit(0), Fit::Empty);
    assert_eq!(fit(VERSION), Fit::Current);
    assert_eq!(fit(VERSION + 1), Fit::Newer(VERSION + 1));
    assert!(fit(0).writable() && fit(VERSION).writable());
    assert!(!fit(VERSION + 7).writable());
}
