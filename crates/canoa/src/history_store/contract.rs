//! The contract every `history_store` provider passes.
//!
//! [`run`] takes a store with no schema, creates version 1 of it, and proves what the host relies on: typed rows come back as they
//! went in, a write is all or none, a key and a unique index refuse a duplicate, a page of a select does not move when rows are added,
//! every condition and every figure answers what it names, and what a table requires is required. A failure panics with what failed.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use super::schema::{self, EVENTS, SESSIONS, TURNS};
use super::{
    AggFn, Aggregate, Cell, Change, Cond, Figure, HistoryDb, Migration, Op, Order, Rows, Select,
    Step, StoreError, Update, Write, all_of,
};

type Row = BTreeMap<String, Cell>;

fn row(cells: Vec<(&str, Cell)>) -> Row {
    cells
        .into_iter()
        .map(|(name, cell)| (name.to_string(), cell))
        .collect()
}

fn insert(table: &str, row: Row) -> Change {
    Change::Insert {
        table: table.to_string(),
        row,
    }
}

fn session(id: &str, entry: &str, at: i64) -> Row {
    row(alloc::vec![
        ("id", Cell::text(id)),
        ("entry", Cell::text(entry)),
        ("agent_session", Cell::text(format!("agent-{id}"))),
        ("origin", Cell::text("recorded")),
        ("created_at", Cell::int(at)),
        ("updated_at", Cell::int(at)),
        ("header", Cell::text("{}")),
        ("turns", Cell::int(0)),
        ("events", Cell::int(0)),
        ("bytes", Cell::int(0)),
        ("open_turn", Cell::bool(false)),
    ])
}

fn event(session: &str, seq: i64, method: &str, text: Option<&str>) -> Row {
    row(alloc::vec![
        ("session_id", Cell::text(session)),
        ("seq", Cell::int(seq)),
        ("method", Cell::text(method)),
        ("bytes", Cell::int(10)),
        ("line", Cell::text(format!("{{\"seq\":{seq}}}"))),
        ("text", Cell::opt_text(text)),
    ])
}

fn turn(session: &str, no: i64, ended: Option<i64>, model: &str, tokens: i64) -> Row {
    row(alloc::vec![
        ("session_id", Cell::text(session)),
        ("no", Cell::int(no)),
        ("first_seq", Cell::int(1)),
        ("last_seq", Cell::int(2)),
        ("bytes", Cell::int(20)),
        ("ended_at", Cell::opt_int(ended)),
        ("model", Cell::text(model)),
        (
            "role",
            Cell::text(if no % 2 == 0 { "reviewer" } else { "coder" })
        ),
        ("tokens_input", Cell::int(tokens)),
        ("tokens_output", Cell::int(tokens / 2)),
        ("tags", Cell::json("{\"ai6:run\":\"r1\",\"acp:x\":1}")),
    ])
}

fn write(db: &dyn HistoryDb, ops: Vec<Change>) -> Result<u64, StoreError> {
    db.write(Write { ops })
        .map(|written| written.affected)
        .map_err(StoreError::from)
}

fn must(db: &dyn HistoryDb, ops: Vec<Change>) {
    if let Err(error) = write(db, ops) {
        panic!("a write the schema allows was refused: {error}");
    }
}

fn select(db: &dyn HistoryDb, select: Select) -> Rows {
    db.select(select).unwrap_or_else(|error| {
        panic!(
            "a select the schema allows was refused: {}",
            StoreError::from(error)
        )
    })
}

fn ints(rows: &Rows, column: &str) -> Vec<i64> {
    (0..rows.rows.len())
        .map(|at| {
            rows.get(at, column)
                .and_then(Cell::as_int)
                .unwrap_or_else(|| panic!("{column} of row {at} is not an integer"))
        })
        .collect()
}

fn texts(rows: &Rows, column: &str) -> Vec<String> {
    (0..rows.rows.len())
        .map(|at| {
            rows.get(at, column)
                .and_then(Cell::as_text)
                .unwrap_or_else(|| panic!("{column} of row {at} is not text"))
                .to_string()
        })
        .collect()
}

fn refused(result: Result<impl core::fmt::Debug, impl Into<StoreError>>, what: &str) -> StoreError {
    match result {
        Ok(value) => panic!("{what} was accepted: {value:?}"),
        Err(error) => error.into(),
    }
}

fn events_where(session: &str, extra: Vec<Cond>) -> Select {
    let mut conds = alloc::vec![Cond::eq("session_id", Cell::text(session))];
    conds.extend(extra);
    Select::new(EVENTS, 1000)
        .filtered(all_of(conds))
        .ordered(alloc::vec![Order::asc("seq")])
}

/// Runs the contract on `db`, a store with no schema.
///
/// # Panics
///
/// When the store breaks a rule of the kind.
pub fn run(db: &dyn HistoryDb) {
    schema_and_migration(db);
    rows_and_conflicts(db);
    writes_are_atomic(db);
    conditions(db);
    paging(db);
    figures(db);
    counters_and_changes(db);
    requirements_and_refusals(db);
}

fn schema_and_migration(db: &dyn HistoryDb) {
    assert_eq!(
        db.schema().expect("schema").version,
        0,
        "a new store has no schema"
    );
    db.migrate(schema::v1()).expect("version 1 is created");
    let state = db.schema().expect("schema");
    assert_eq!(state.version, schema::VERSION);
    let names: Vec<&str> = state.tables.iter().map(|t| t.name.as_str()).collect();
    for table in schema::tables() {
        assert!(
            names.contains(&table.name.as_str()),
            "{} is missing",
            table.name
        );
        let held = state
            .tables
            .iter()
            .find(|t| t.name == table.name)
            .expect("table");
        assert_eq!(held.key, table.key, "{} key", table.name);
        assert_eq!(held.columns, table.columns, "{} columns", table.name);
    }
    let error = refused(db.migrate(schema::v1()), "a table that exists");
    assert!(matches!(error, StoreError::Conflict(_)), "{error:?}");
    let added = Migration {
        to_version: 2,
        steps: alloc::vec![Step::AddColumn {
            table: TURNS.to_string(),
            column: super::ColumnInfo::nullable("note", super::ColumnType::Text),
        }],
    };
    db.migrate(added).expect("a nullable column is added");
    let state = db.schema().expect("schema");
    assert_eq!(state.version, 2);
    assert!(
        state
            .tables
            .iter()
            .find(|t| t.name == TURNS)
            .and_then(|t| t.column("note"))
            .is_some()
    );
    let required = Migration {
        to_version: 3,
        steps: alloc::vec![Step::AddColumn {
            table: TURNS.to_string(),
            column: super::ColumnInfo::new("must", super::ColumnType::Text),
        }],
    };
    let error = refused(db.migrate(required), "a column that may not be null");
    assert!(matches!(error, StoreError::Bad(_)), "{error:?}");
    assert_eq!(
        db.schema().expect("schema").version,
        2,
        "a refused migration changes nothing"
    );
}

fn rows_and_conflicts(db: &dyn HistoryDb) {
    let before = db.revision().expect("revision");
    must(
        db,
        alloc::vec![
            insert(SESSIONS, session("s1", "claude", 1000)),
            insert(SESSIONS, session("s2", "claude", 2000)),
            insert(SESSIONS, session("s3", "codex", 3000)),
            insert(TURNS, turn("s1", 1, Some(10), "opus", 100)),
            insert(TURNS, turn("s1", 2, Some(20), "sonnet", 40)),
            insert(TURNS, turn("s1", 3, None, "sonnet", 60)),
        ],
    );
    assert_ne!(
        db.revision().expect("revision"),
        before,
        "a write moves the revision"
    );
    let turns = select(
        db,
        Select::new(TURNS, 10)
            .filtered(all_of(alloc::vec![Cond::eq("no", Cell::int(1))]))
            .ordered(alloc::vec![Order::asc("no")]),
    );
    assert_eq!(turns.rows.len(), 1);
    assert_eq!(turns.get(0, "model"), Some(&Cell::text("opus")));
    assert_eq!(turns.get(0, "tokens_input"), Some(&Cell::int(100)));
    assert_eq!(
        turns.get(0, "cost_micros"),
        Some(&Cell::Null),
        "a figure not reported is null"
    );
    assert_eq!(turns.get(0, "usage_known"), Some(&Cell::Null));
    let tags = turns.get(0, "tags").and_then(Cell::as_text).expect("tags");
    assert!(
        tags.contains("ai6:run") && tags.contains("acp:x"),
        "the open tags are kept: {tags}"
    );
    let error = refused(
        write(db, alloc::vec![insert(SESSIONS, session("s1", "x", 1))]),
        "a key that exists",
    );
    assert!(matches!(error, StoreError::Conflict(_)), "{error:?}");
    let mut same_agent = session("s9", "claude", 1);
    same_agent.insert("agent_session".to_string(), Cell::text("agent-s1"));
    let error = refused(
        write(db, alloc::vec![insert(SESSIONS, same_agent)]),
        "a unique index's duplicate",
    );
    assert!(matches!(error, StoreError::Conflict(_)), "{error:?}");
    let mut replaced = session("s1", "claude", 1000);
    replaced.insert("title".to_string(), Cell::text("renamed"));
    must(
        db,
        alloc::vec![Change::Upsert {
            table: SESSIONS.to_string(),
            row: replaced,
        }],
    );
    let found = select(
        db,
        Select::new(SESSIONS, 10).filtered(all_of(alloc::vec![Cond::eq("id", Cell::text("s1"))])),
    );
    assert_eq!(found.get(0, "title"), Some(&Cell::text("renamed")));
    assert_eq!(
        db.count(Select::new(SESSIONS, 1)).expect("count"),
        3,
        "an upsert of a key replaces"
    );
}

fn writes_are_atomic(db: &dyn HistoryDb) {
    let error = refused(
        write(
            db,
            alloc::vec![
                insert(SESSIONS, session("s4", "claude", 4000)),
                insert(SESSIONS, session("s1", "claude", 1)),
            ],
        ),
        "a batch with a conflict",
    );
    assert!(matches!(error, StoreError::Conflict(_)));
    let held = db
        .count(
            Select::new(SESSIONS, 1)
                .filtered(all_of(alloc::vec![Cond::eq("id", Cell::text("s4"))])),
        )
        .expect("count");
    assert_eq!(held, 0, "a batch with a failed op leaves nothing");
    let mut wrong = session("s5", "claude", 5000);
    wrong.insert("turns".to_string(), Cell::text("many"));
    let error = refused(
        write(db, alloc::vec![insert(SESSIONS, wrong)]),
        "a text in an integer column",
    );
    assert!(matches!(error, StoreError::Bad(_)), "{error:?}");
    let mut missing = session("s5", "claude", 5000);
    missing.remove("header");
    let error = refused(
        write(db, alloc::vec![insert(SESSIONS, missing)]),
        "a null in a column that may not be",
    );
    assert!(matches!(error, StoreError::Bad(_)), "{error:?}");
}

fn conditions(db: &dyn HistoryDb) {
    let ids = |filter: super::Filter| {
        texts(
            &select(
                db,
                Select::new(SESSIONS, 100)
                    .filtered(filter)
                    .ordered(alloc::vec![Order::asc("id")]),
            ),
            "id",
        )
    };
    let at = |op: Op, n: i64| all_of(alloc::vec![Cond::new("created_at", op, Cell::int(n))]);
    assert_eq!(ids(at(Op::Eq, 2000)), ["s2"]);
    assert_eq!(ids(at(Op::Ne, 2000)), ["s1", "s3"]);
    assert_eq!(ids(at(Op::Lt, 2000)), ["s1"]);
    assert_eq!(ids(at(Op::Le, 2000)), ["s1", "s2"]);
    assert_eq!(ids(at(Op::Gt, 2000)), ["s3"]);
    assert_eq!(ids(at(Op::Ge, 2000)), ["s2", "s3"]);
    assert_eq!(
        ids(all_of(alloc::vec![Cond::any_of(
            "id",
            alloc::vec![Cell::text("s1"), Cell::text("s3")]
        )])),
        ["s1", "s3"]
    );
    assert!(ids(all_of(alloc::vec![Cond::any_of("id", Vec::new())])).is_empty());
    assert_eq!(
        ids(all_of(alloc::vec![Cond::is_null("title")])),
        ["s2", "s3"]
    );
    assert_eq!(ids(all_of(alloc::vec![Cond::not_null("title")])), ["s1"]);
    assert_eq!(
        ids(all_of(alloc::vec![Cond::new(
            "title",
            Op::Prefix,
            Cell::text("ren")
        )])),
        ["s1"]
    );
    assert_eq!(
        ids(all_of(alloc::vec![Cond::new(
            "title",
            Op::Contains,
            Cell::text("nam")
        )])),
        ["s1"]
    );
    assert_eq!(
        ids(all_of(alloc::vec![Cond::new(
            "title",
            Op::Suffix,
            Cell::text("med")
        )])),
        ["s1"]
    );
    assert!(
        ids(all_of(alloc::vec![Cond::new(
            "title",
            Op::Contains,
            Cell::text("%")
        )]))
        .is_empty(),
        "a text test is literal: `%` is not a wildcard"
    );
    assert!(
        ids(all_of(alloc::vec![Cond::new(
            "title",
            Op::Contains,
            Cell::text("RENAMED")
        )]))
        .is_empty(),
        "a text test is case-sensitive"
    );
    let either = alloc::vec![
        alloc::vec![Cond::eq("entry", Cell::text("codex"))],
        alloc::vec![
            Cond::eq("entry", Cell::text("claude")),
            Cond::new("created_at", Op::Lt, Cell::int(1500)),
        ],
    ];
    assert_eq!(
        ids(either),
        ["s1", "s3"],
        "alternatives are ORed, conditions ANDed"
    );
    let ordered = select(
        db,
        Select::new(SESSIONS, 100).ordered(alloc::vec![Order::desc("created_at")]),
    );
    assert_eq!(texts(&ordered, "id"), ["s3", "s2", "s1"]);
    let chosen = select(
        db,
        Select {
            columns: alloc::vec!["id".to_string()],
            ..Select::new(SESSIONS, 1)
        },
    );
    assert_eq!(chosen.columns, ["id"]);
    assert_eq!(chosen.rows[0].len(), 1);
}

fn paging(db: &dyn HistoryDb) {
    let mut ops = Vec::new();
    for seq in 1..=25 {
        ops.push(insert(
            EVENTS,
            event("s1", seq, "session/update", Some("text")),
        ));
    }
    must(db, ops);
    let page = Select::new(EVENTS, 10)
        .filtered(all_of(alloc::vec![Cond::eq(
            "session_id",
            Cell::text("s1")
        )]))
        .ordered(alloc::vec![Order::desc("seq")]);
    let mut seen = Vec::new();
    let mut after = None;
    let mut pages = 0;
    loop {
        let rows = select(
            db,
            Select {
                after: after.clone(),
                ..page.clone()
            },
        );
        seen.extend(ints(&rows, "seq"));
        pages += 1;
        if pages == 1 {
            // Lines are added while the reader is part way through: they sort before the cursor and are not met again.
            must(
                db,
                alloc::vec![
                    insert(EVENTS, event("s1", 26, "session/update", None)),
                    insert(EVENTS, event("s1", 27, "session/update", None)),
                ],
            );
        }
        match rows.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    let expected: Vec<i64> = (1..=25).rev().collect();
    assert_eq!(
        seen, expected,
        "a page boundary does not move when rows are added"
    );
    assert_eq!(pages, 3);
    let error = refused(
        db.select(Select {
            after: Some("[]".to_string()),
            ..page.clone()
        }),
        "a cursor of another select",
    );
    assert!(matches!(error, StoreError::Bad(_)));
    // Paging by a column that holds nulls: they sort lowest, so they come last descending.
    let mut by_end = Vec::new();
    let mut after = None;
    let ended = Select::new(TURNS, 2)
        .filtered(all_of(alloc::vec![Cond::eq(
            "session_id",
            Cell::text("s1")
        )]))
        .ordered(alloc::vec![Order::desc("ended_at")]);
    loop {
        let rows = select(
            db,
            Select {
                after: after.clone(),
                ..ended.clone()
            },
        );
        by_end.extend(ints(&rows, "no"));
        match rows.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(by_end, [2, 1, 3], "descending, a null is last");
    let mut by_end = Vec::new();
    let mut after = None;
    let ascending = Select {
        order: alloc::vec![Order::asc("ended_at")],
        ..ended
    };
    loop {
        let rows = select(
            db,
            Select {
                after: after.clone(),
                ..ascending.clone()
            },
        );
        by_end.extend(ints(&rows, "no"));
        match rows.next {
            Some(next) => after = Some(next),
            None => break,
        }
    }
    assert_eq!(by_end, [3, 1, 2], "ascending, a null is first");
}

fn figures(db: &dyn HistoryDb) {
    let by_model = db
        .aggregate(Aggregate {
            table: TURNS.to_string(),
            filter: Vec::new(),
            group_by: alloc::vec!["model".to_string()],
            figures: alloc::vec![
                Figure::of(AggFn::Count, None, "turns"),
                Figure::of(AggFn::Sum, Some("tokens_input"), "tokens"),
                Figure::of(AggFn::Min, Some("ended_at"), "first"),
                Figure::of(AggFn::Max, Some("ended_at"), "last"),
                Figure::of(AggFn::Count, Some("ended_at"), "ended"),
            ],
        })
        .expect("aggregate");
    assert_eq!(texts(&by_model, "model"), ["opus", "sonnet"]);
    assert_eq!(ints(&by_model, "turns"), [1, 2]);
    assert_eq!(ints(&by_model, "tokens"), [100, 100]);
    assert_eq!(ints(&by_model, "first"), [10, 20]);
    assert_eq!(ints(&by_model, "last"), [10, 20]);
    assert_eq!(
        ints(&by_model, "ended"),
        [1, 1],
        "a count of a column skips nulls"
    );
    let all = db
        .aggregate(Aggregate {
            table: TURNS.to_string(),
            filter: all_of(alloc::vec![Cond::eq("role", Cell::text("nobody"))]),
            group_by: Vec::new(),
            figures: alloc::vec![
                Figure::of(AggFn::Count, None, "n"),
                Figure::of(AggFn::Sum, Some("tokens_input"), "tokens"),
            ],
        })
        .expect("aggregate");
    assert_eq!(
        all.rows.len(),
        1,
        "figures with no group are one row, even over no rows"
    );
    assert_eq!(all.get(0, "n"), Some(&Cell::int(0)));
    assert_eq!(all.get(0, "tokens"), Some(&Cell::Null));
    let by_role = db
        .aggregate(Aggregate {
            table: TURNS.to_string(),
            filter: Vec::new(),
            group_by: alloc::vec!["role".to_string(), "model".to_string()],
            figures: alloc::vec![Figure::of(AggFn::Sum, Some("tokens_output"), "out")],
        })
        .expect("aggregate");
    assert_eq!(by_role.rows.len(), 3, "role and model together");
    let error = refused(
        db.aggregate(Aggregate {
            table: TURNS.to_string(),
            filter: Vec::new(),
            group_by: Vec::new(),
            figures: alloc::vec![Figure::of(AggFn::Sum, Some("model"), "bad")],
        }),
        "the sum of text",
    );
    assert!(matches!(error, StoreError::Bad(_)));
}

fn counters_and_changes(db: &dyn HistoryDb) {
    let by = |n: i64| Change::Update {
        update: Update {
            table: SESSIONS.to_string(),
            filter: all_of(alloc::vec![Cond::eq("id", Cell::text("s2"))]),
            set: row(alloc::vec![("title", Cell::text("counted"))]),
            add: BTreeMap::from([("events".to_string(), n), ("bytes".to_string(), 10 * n)]),
        },
    };
    let affected = write(db, alloc::vec![by(3), by(2), by(-1)]).expect("updates");
    assert_eq!(affected, 3);
    let found = select(
        db,
        Select::new(SESSIONS, 1).filtered(all_of(alloc::vec![Cond::eq("id", Cell::text("s2"))])),
    );
    assert_eq!(
        found.get(0, "events"),
        Some(&Cell::int(4)),
        "a counter moves in place"
    );
    assert_eq!(found.get(0, "bytes"), Some(&Cell::int(40)));
    assert_eq!(found.get(0, "title"), Some(&Cell::text("counted")));
    let error = refused(
        write(
            db,
            alloc::vec![Change::Update {
                update: Update {
                    table: SESSIONS.to_string(),
                    filter: Vec::new(),
                    set: row(alloc::vec![("header", Cell::Null)]),
                    add: BTreeMap::new(),
                },
            }],
        ),
        "a null set in a column that may not be",
    );
    assert!(matches!(error, StoreError::Bad(_)));
    let removed = write(
        db,
        alloc::vec![Change::Delete {
            table: EVENTS.to_string(),
            filter: all_of(alloc::vec![
                Cond::eq("session_id", Cell::text("s1")),
                Cond::new("seq", Op::Gt, Cell::int(25)),
            ]),
        }],
    )
    .expect("delete");
    assert_eq!(removed, 2);
    assert_eq!(db.count(Select::new(EVENTS, 1)).expect("count"), 25);
    let stats = db.stats().expect("stats");
    assert_eq!(stats.rows.get(EVENTS), Some(&25));
}

fn requirements_and_refusals(db: &dyn HistoryDb) {
    let error = refused(
        db.select(Select::new(EVENTS, 10)),
        "a select of events with no session",
    );
    assert!(matches!(error, StoreError::Bad(_)));
    let error = refused(
        db.select(Select::new(EVENTS, 10).filtered(alloc::vec![
            alloc::vec![Cond::eq("session_id", Cell::text("s1"))],
            alloc::vec![Cond::eq("method", Cell::text("session/update"))],
        ])),
        "an alternative that does not test the session",
    );
    assert!(matches!(error, StoreError::Bad(_)));
    assert_eq!(
        select(
            db,
            events_where("s1", alloc::vec![Cond::new("seq", Op::Le, Cell::int(3))])
        )
        .rows
        .len(),
        3
    );
    let error = refused(db.select(Select::new("nothing", 1)), "an unknown table");
    assert!(matches!(error, StoreError::Bad(_)));
    let error = refused(
        db.select(
            Select::new(SESSIONS, 1)
                .filtered(all_of(alloc::vec![Cond::eq("nothing", Cell::int(1))])),
        ),
        "an unknown column",
    );
    assert!(matches!(error, StoreError::Bad(_)));
    let error = refused(
        db.select(
            Select::new(SESSIONS, 1)
                .filtered(all_of(alloc::vec![Cond::eq("created_at", Cell::text("1"))])),
        ),
        "a text compared with an integer column",
    );
    assert!(matches!(error, StoreError::Bad(_)));
    let error = refused(
        db.select(Select::new(SESSIONS, 0)),
        "a select with no limit",
    );
    assert!(matches!(error, StoreError::Bad(_)));
    db.close().expect("close");
}

/// Runs the document half of the contract on `db`, which holds schema version 1 (a store made with [`schema::v1`], or one that is the
/// canonical schema by nature): a history is stored as rows and read back as the same text, grows by lines and counters, and is
/// removed with everything that hangs off it.
///
/// # Panics
///
/// When the store breaks a rule of the kind.
pub fn run_documents(db: &dyn HistoryDb) {
    use coqui_acp_history::{Document, Header, Line, Origin, StopReason};

    use super::rows::{Source, read_document, to_rows};

    let id = "5f0f6d6e-3f3c-4f1a-8a52-1c1e6f0b9a77";
    let source = Source {
        entry: "claude".to_string(),
        agent_session: "a1".to_string(),
        origin: "recorded".to_string(),
        imported_from: None,
        now_ms: 1000,
    };
    let say = |text: &str| {
        Line::update(serde_json::json!({
            "sessionUpdate": "agent_message_chunk",
            "content": {"type": "text", "text": text},
        }))
    };
    let end = || Line::turn_end(coqui_acp_history::Stop::Acp(StopReason::EndTurn));
    let mut document = Document::new(Header::new(id, Origin::new("claude")));
    document.lines = alloc::vec![Line::text_prompt("hi"), say("ok"), end()];
    let rows = to_rows(&document, &source).expect("rows");
    must(db, rows.inserts());
    let back = read_document(db, id)
        .expect("read")
        .expect("the history is there");
    assert_eq!(back, document, "a history comes back as it went in");
    assert_eq!(
        back.to_string().expect("text"),
        document.to_string().expect("text"),
        "byte for byte"
    );
    assert!(
        read_document(db, "no-such-session")
            .expect("read")
            .is_none()
    );

    // A second turn arrives as lines and counters.
    document
        .lines
        .extend([Line::text_prompt("again"), say("sure"), end()]);
    let grown = to_rows(&document, &source).expect("rows");
    let mut ops: Vec<Change> = grown.events[3..]
        .iter()
        .map(|event| insert(EVENTS, event.clone()))
        .collect();
    ops.push(insert(TURNS, grown.turns[1].clone()));
    ops.push(Change::Update {
        update: Update {
            table: SESSIONS.to_string(),
            filter: all_of(alloc::vec![Cond::eq("id", Cell::text(id))]),
            set: BTreeMap::new(),
            add: BTreeMap::from([("events".to_string(), 3), ("turns".to_string(), 1)]),
        },
    });
    must(db, ops);
    assert_eq!(
        read_document(db, id).expect("read").expect("there"),
        document,
        "a history grows by lines"
    );
    let session = select(
        db,
        Select::new(SESSIONS, 1).filtered(all_of(alloc::vec![Cond::eq("id", Cell::text(id))])),
    );
    assert_eq!(session.get(0, "events"), Some(&Cell::int(6)));
    assert_eq!(session.get(0, "turns"), Some(&Cell::int(2)));
    let hits = select(
        db,
        events_where(
            id,
            alloc::vec![Cond::new("text", Op::Contains, Cell::text("sure"))],
        ),
    );
    assert_eq!(
        hits.rows.len(),
        1,
        "what an agent said is found by its text"
    );

    // Removing a history removes everything that hangs off it.
    let removal = |table: &str, column: &str| Change::Delete {
        table: table.to_string(),
        filter: all_of(alloc::vec![Cond::eq(column, Cell::text(id))]),
    };
    must(
        db,
        alloc::vec![
            removal(EVENTS, "session_id"),
            removal(TURNS, "session_id"),
            removal(SESSIONS, "id"),
        ],
    );
    assert!(
        read_document(db, id).expect("read").is_none(),
        "the history is gone"
    );
    assert_eq!(
        db.count(
            Select::new(TURNS, 1)
                .filtered(all_of(alloc::vec![Cond::eq("session_id", Cell::text(id))]))
        )
        .expect("count"),
        0
    );
}

/// Runs the search half of the contract on `db`, which holds schema version 1 and has the capability `search`: a condition `matches`
/// finds the lines whose text holds every word of the query, whole words, whatever the case, and follows the text as it changes.
///
/// # Panics
///
/// When the store breaks a rule of the kind.
pub fn run_search(db: &dyn HistoryDb) {
    let id = "6a0f6d6e-3f3c-4f1a-8a52-1c1e6f0b9a78";
    let mut session = session(id, "claude", 1);
    session.insert("agent_session".to_string(), Cell::text("search"));
    must(
        db,
        alloc::vec![
            insert(SESSIONS, session),
            insert(
                EVENTS,
                event(id, 1, "session/prompt", Some("Refactor the Parser module"))
            ),
            insert(
                EVENTS,
                event(
                    id,
                    2,
                    "session/update",
                    Some("Done: the parser now streams tokens")
                )
            ),
            insert(EVENTS, event(id, 3, "session/update", None)),
        ],
    );
    let found = |query: &str| {
        ints(
            &select(
                db,
                events_where(
                    id,
                    alloc::vec![Cond::new("text", Op::Matches, Cell::text(query))],
                ),
            ),
            "seq",
        )
    };
    assert_eq!(found("parser"), [1, 2], "whole words, any case");
    assert_eq!(found("PARSER tokens"), [2], "every word of the query");
    assert!(found("pars").is_empty(), "a word is not a prefix");
    assert!(found("   ").is_empty(), "no words match nothing");
    must(
        db,
        alloc::vec![Change::Update {
            update: Update {
                table: EVENTS.to_string(),
                filter: all_of(alloc::vec![
                    Cond::eq("session_id", Cell::text(id)),
                    Cond::eq("seq", Cell::int(1)),
                ]),
                set: row(alloc::vec![("text", Cell::text("Rename the lexer"))]),
                add: BTreeMap::new(),
            },
        }],
    );
    assert_eq!(found("parser"), [2], "the index follows an update");
    assert_eq!(found("lexer"), [1]);
    must(
        db,
        alloc::vec![Change::Delete {
            table: EVENTS.to_string(),
            filter: all_of(alloc::vec![
                Cond::eq("session_id", Cell::text(id)),
                Cond::eq("seq", Cell::int(2)),
            ]),
        }],
    );
    assert!(found("parser").is_empty(), "the index follows a delete");
}
