//! The `history_store` kind: the contract outside providers (a library in Python or JavaScript, or another extension) are built against.
//! Regenerate the description files with `CANOA_HISTORY_WRITE_KINDS=1 cargo test -p canoa --features history-store --test history_store_kinds`.
#![cfg(feature = "history-store")]
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a test names what it uses from std"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;

use canoa::history_store::schema;
use canoa::history_store::{
    Access, AggFn, Aggregate, Cell, Change, ColumnInfo, ColumnType, Compacted, Cond, Figure,
    HistoryDbVtable, HistoryStoreVtable, IndexInfo, Limits, Migration, Op, Order, Rows,
    SchemaState, Select, Sql, Stats, Step, StoreInfo, TableInfo, Update, Write, Written,
};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};
use guatiao_serde::Serializable;
use serde_json::Value as Json;

fn file(path: &str) -> PathBuf {
    PathBuf::from(env!("CARGO_MANIFEST_DIR")).join(path)
}

fn pretty(value: &Json) -> String {
    let mut text = serde_json::to_string_pretty(value).expect("written");
    text.push('\n');
    text
}

fn committed(path: &str, fresh: &Json) {
    let path = file(path);
    if std::env::var_os("CANOA_HISTORY_WRITE_KINDS").is_some() {
        std::fs::create_dir_all(path.parent().expect("a directory")).expect("directory");
        std::fs::write(&path, pretty(fresh)).expect("the file is written");
    }
    let committed = std::fs::read_to_string(&path).expect("the file is committed");
    // Compared as documents: another crate's `preserve_order` in the build changes the order a JSON object is written in.
    let committed: Json = serde_json::from_str(&committed).expect("the committed file is JSON");
    assert!(&committed == fresh, "regenerate {}", path.display());
}

fn schema_of<T: Schema>() -> Json {
    let value = T::schema().expect("a model describes itself");
    serde_json::to_value(Serializable::from(&value)).expect("a schema is JSON")
}

#[test]
fn the_committed_description_is_what_this_build_describes() {
    let document: Json = serde_json::from_str(&format!(
        "{{\"kinds\":[{},{}]}}",
        HistoryStoreVtable::description(),
        HistoryDbVtable::description()
    ))
    .expect("a kind's description is JSON");
    committed("include/canoa.history_store.kinds.json", &document);
}

#[test]
fn the_committed_schemas_are_what_this_build_describes() {
    let schemas: serde_json::Map<String, Json> = [
        ("Cell", schema_of::<Cell>()),
        ("ColumnType", schema_of::<ColumnType>()),
        ("ColumnInfo", schema_of::<ColumnInfo>()),
        ("IndexInfo", schema_of::<IndexInfo>()),
        ("TableInfo", schema_of::<TableInfo>()),
        ("SchemaState", schema_of::<SchemaState>()),
        ("Step", schema_of::<Step>()),
        ("Migration", schema_of::<Migration>()),
        ("Op", schema_of::<Op>()),
        ("Cond", schema_of::<Cond>()),
        ("Order", schema_of::<Order>()),
        ("Select", schema_of::<Select>()),
        ("Rows", schema_of::<Rows>()),
        ("AggFn", schema_of::<AggFn>()),
        ("Figure", schema_of::<Figure>()),
        ("Aggregate", schema_of::<Aggregate>()),
        ("Update", schema_of::<Update>()),
        ("Change", schema_of::<Change>()),
        ("Write", schema_of::<Write>()),
        ("Written", schema_of::<Written>()),
        ("Sql", schema_of::<Sql>()),
        ("Stats", schema_of::<Stats>()),
        ("Compacted", schema_of::<Compacted>()),
        ("Access", schema_of::<Access>()),
        ("Limits", schema_of::<Limits>()),
        ("StoreInfo", schema_of::<StoreInfo>()),
        (
            "OpenRequest",
            schema_of::<canoa::history_store::OpenRequest>(),
        ),
    ]
    .into_iter()
    .map(|(name, schema)| (name.to_owned(), schema))
    .collect();
    committed(
        "include/canoa.history_store.schemas.json",
        &Json::Object(schemas),
    );
}

fn trip<T: ToValue + FromValue + PartialEq + core::fmt::Debug>(value: &T) {
    let carried = value.to_value().expect("a value");
    assert_eq!(&T::from_value(&carried).expect("read back"), value);
}

#[test]
fn every_model_survives_the_trip_through_a_value() {
    for cell in [
        Cell::Null,
        Cell::bool(true),
        Cell::int(-7),
        Cell::real(1.5),
        Cell::text("hi"),
        Cell::bytes(vec![0, 1, 255]),
        Cell::json("{\"a\":1}"),
    ] {
        trip(&cell);
    }
    let mut select = Select::new("events", 10).ordered(vec![Order::desc("seq")]);
    select.filter = vec![
        vec![
            Cond::eq("session_id", Cell::text("s")),
            Cond::any_of("method", vec![Cell::text("a"), Cell::text("b")]),
        ],
        vec![Cond::is_null("text")],
    ];
    select.after = Some("[]".to_owned());
    trip(&select);
    trip(&Write {
        ops: vec![
            Change::Insert {
                table: "t".to_owned(),
                row: BTreeMap::from([("a".to_owned(), Cell::int(1))]),
            },
            Change::Update {
                update: Update {
                    table: "t".to_owned(),
                    filter: Vec::new(),
                    set: BTreeMap::new(),
                    add: BTreeMap::from([("n".to_owned(), 2)]),
                },
            },
            Change::Delete {
                table: "t".to_owned(),
                filter: vec![vec![Cond::new("a", Op::Matches, Cell::text("x"))]],
            },
        ],
    });
    trip(&Aggregate {
        table: "turns".to_owned(),
        filter: Vec::new(),
        group_by: vec!["model".to_owned()],
        figures: vec![Figure::of(AggFn::Sum, Some("tokens_input"), "tokens")],
    });
    trip(&schema::v1());
    trip(&SchemaState {
        version: 1,
        tables: schema::tables(),
    });
    let mut info = StoreInfo::new("sqlite", "SQLite");
    info.capabilities = vec!["sql".to_owned()];
    info.dialect = Some("sqlite".to_owned());
    trip(&info);
}
