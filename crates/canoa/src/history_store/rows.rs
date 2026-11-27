//! A session history in the standard format as rows of the schema, and back.
//!
//! [`to_rows`] reads a [`Document`] into one `sessions` row, a `turns` row per turn, an `events` row per line and the
//! `attachments` the lines name. [`document_text`] joins a stored header and the stored lines back into the text of the same
//! document, so what went in comes out byte for byte (the header's digest, if it had one, is not kept: the body it named may grow;
//! [`Document::seal`] makes one again for an export).

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString as _};
use alloc::vec::Vec;

use coqui_acp_history::{ATTACHMENT_META_KEY, Body, Document, HistoryError, Trigger, TurnRecord};
use serde_json::Value;

use super::schema::{ATTACHMENTS, EVENT_ATTACHMENTS, EVENTS, SESSIONS, TURNS};
use super::{Cell, Change, HistoryDb, StoreError};

type Row = BTreeMap<String, Cell>;

/// Where a history came from, which the standard format does not say.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Source {
    /// The host's entry (agent) the session ran under.
    pub entry: String,
    /// The agent's own id for the session.
    pub agent_session: String,
    /// `recorded`, `imported` or `moved`.
    pub origin: String,
    /// The adapter store the history was brought in from, when it was.
    pub imported_from: Option<String>,
    /// The time to use for what the history does not date, in milliseconds since the Unix epoch.
    pub now_ms: i64,
}

/// A history as rows.
#[derive(Debug, Clone, PartialEq)]
pub struct DocumentRows {
    /// The `sessions` row.
    pub session: Row,
    /// The `turns` rows, in order.
    pub turns: Vec<Row>,
    /// The `events` rows, in order.
    pub events: Vec<Row>,
    /// The `attachments` rows the lines name (known by digest only).
    pub attachments: Vec<Row>,
    /// The `event_attachments` rows.
    pub event_attachments: Vec<Row>,
}

impl DocumentRows {
    /// These rows for lines that follow `events` lines and `turns` turns the session already has: the line numbers, the turn numbers
    /// and the turns' line ranges move past them. The `sessions` row is left as it is.
    #[must_use]
    pub fn shifted(mut self, events: i64, turns: i64) -> Self {
        let bump = |row: &mut Row, column: &str, by: i64| {
            if let Some(n) = row.get(column).and_then(Cell::as_int) {
                row.insert(column.to_string(), Cell::int(n + by));
            }
        };
        for row in &mut self.events {
            bump(row, "seq", events);
            bump(row, "turn_no", turns);
        }
        for row in &mut self.turns {
            bump(row, "no", turns);
            bump(row, "first_seq", events);
            bump(row, "last_seq", events);
        }
        for row in &mut self.event_attachments {
            bump(row, "seq", events);
        }
        self
    }

    /// The writes that store this history: the session, then its turns, events and attachments, each as an `insert`.
    #[must_use]
    pub fn inserts(&self) -> Vec<Change> {
        let put = |table: &str, row: &Row| Change::Insert {
            table: table.to_string(),
            row: row.clone(),
        };
        let mut ops = Vec::new();
        ops.push(put(SESSIONS, &self.session));
        ops.extend(self.turns.iter().map(|r| put(TURNS, r)));
        ops.extend(self.events.iter().map(|r| put(EVENTS, r)));
        ops.extend(self.attachments.iter().map(|r| Change::Upsert {
            table: ATTACHMENTS.to_string(),
            row: r.clone(),
        }));
        ops.extend(
            self.event_attachments
                .iter()
                .map(|r| put(EVENT_ATTACHMENTS, r)),
        );
        ops
    }
}

fn millis(at: &str) -> Option<i64> {
    let parsed =
        time::OffsetDateTime::parse(at, &time::format_description::well_known::Rfc3339).ok()?;
    i64::try_from(parsed.unix_timestamp_nanos() / 1_000_000).ok()
}

fn row(cells: Vec<(&str, Cell)>) -> Row {
    cells.into_iter().map(|(k, v)| (k.to_string(), v)).collect()
}

fn text_blocks(blocks: &[Value]) -> Option<String> {
    let parts: Vec<&str> = blocks
        .iter()
        .filter(|b| b.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|b| b.get("text").and_then(Value::as_str))
        .collect();
    (!parts.is_empty()).then(|| parts.join("\n"))
}

/// What a person would read of a line: a prompt's text, an agent or user message, a compaction's summary; not tool output, not thinking.
fn visible_text(body: &Body) -> Option<String> {
    match body {
        Body::Prompt { prompt } => text_blocks(prompt),
        Body::Update { update } => {
            let kind = update.get("sessionUpdate").and_then(Value::as_str)?;
            if !matches!(kind, "agent_message_chunk" | "user_message_chunk") {
                return None;
            }
            let content = update.get("content")?;
            (content.get("type").and_then(Value::as_str) == Some("text"))
                .then(|| {
                    content
                        .get("text")
                        .and_then(Value::as_str)
                        .map(str::to_string)
                })
                .flatten()
        }
        Body::Compaction { summary, .. } => summary.clone(),
        _ => None,
    }
}

fn kind_of(body: &Body) -> Option<String> {
    match body {
        Body::Update { update } => update
            .get("sessionUpdate")
            .and_then(Value::as_str)
            .map(str::to_string),
        Body::Compaction { trigger, .. } => trigger.map(|t| {
            match t {
                Trigger::Auto => "auto",
                _ => "manual",
            }
            .to_string()
        }),
        _ => None,
    }
}

fn call_id(body: &Body) -> Option<String> {
    match body {
        Body::Update { update } => update
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(str::to_string),
        Body::PermissionRequest { tool_call, .. } => tool_call
            .get("toolCallId")
            .and_then(Value::as_str)
            .map(str::to_string),
        Body::PermissionAnswer { tool_call_id, .. } => Some(tool_call_id.clone()),
        _ => None,
    }
}

fn attachments_of(body: &Body) -> Vec<(String, i64)> {
    let mut found = Vec::new();
    let mut look = |block: &Value| {
        let meta = block.get("_meta").and_then(|m| m.get(ATTACHMENT_META_KEY));
        if let (Some(sha), Some(bytes)) = (
            meta.and_then(|m| m.get("sha256")).and_then(Value::as_str),
            meta.and_then(|m| m.get("bytes")).and_then(Value::as_i64),
        ) {
            found.push((sha.to_string(), bytes));
        }
    };
    match body {
        Body::Prompt { prompt } => prompt.iter().for_each(&mut look),
        Body::Update { update } => {
            if let Some(content) = update.get("content") {
                look(content);
            }
        }
        _ => {}
    }
    found
}

fn opt(value: Option<&str>) -> Cell {
    Cell::opt_text(value)
}

/// Reads `document` into rows.
///
/// # Errors
///
/// A line the format refuses, or a header that cannot be written.
pub fn to_rows(document: &Document, source: &Source) -> Result<DocumentRows, StoreError> {
    let fail = |e: HistoryError| {
        StoreError::Bad(format!("the history is not in the standard format: {e}"))
    };
    let summary = document.check().map_err(fail)?;
    let mut header = document.header.clone();
    header.digest = None;
    let header_text = header.to_line().map_err(fail)?;
    let id = header.id.clone();

    let mut events = Vec::new();
    let mut turns: Vec<Row> = Vec::new();
    let mut attachments: BTreeMap<String, Row> = BTreeMap::new();
    let mut event_attachments = Vec::new();
    let mut open: Option<usize> = None;
    // The header line and each line with its line feed: what the history takes written out.
    let mut total_bytes = i64::try_from(header_text.len() + 1).unwrap_or(i64::MAX);
    let mut last_at: Option<i64> = None;

    for (at, line) in document.lines.iter().enumerate() {
        let seq = i64::try_from(at + 1).unwrap_or(i64::MAX);
        let json = line.to_json().map_err(fail)?;
        let bytes = i64::try_from(json.len()).unwrap_or(i64::MAX);
        total_bytes += bytes + 1;
        let at_ms = line.at.as_deref().and_then(millis);
        if at_ms.is_some() {
            last_at = at_ms;
        }
        if matches!(line.body, Body::Prompt { .. }) && open.is_none() {
            turns.push(row(alloc::vec![
                ("session_id", Cell::text(&id)),
                (
                    "no",
                    Cell::int(i64::try_from(turns.len() + 1).unwrap_or(i64::MAX))
                ),
                ("first_seq", Cell::int(seq)),
                ("last_seq", Cell::int(seq)),
                ("bytes", Cell::int(0)),
                ("started_at", Cell::opt_int(at_ms)),
                ("ended_at", Cell::Null),
                ("stop", Cell::Null),
            ]));
            open = Some(turns.len() - 1);
        }
        let turn_no = open.map(|i| i64::try_from(i + 1).unwrap_or(i64::MAX));
        if let Some(i) = open {
            let turn = &mut turns[i];
            turn.insert("last_seq".into(), Cell::int(seq));
            let so_far = turn.get("bytes").and_then(Cell::as_int).unwrap_or(0);
            turn.insert("bytes".into(), Cell::int(so_far + bytes + 1));
            if let Body::TurnEnd { stop, .. } = &line.body {
                turn.insert("ended_at".into(), Cell::opt_int(at_ms));
                turn.insert("stop".into(), Cell::text(stop.name()));
                open = None;
            }
        }
        for (sha, size) in attachments_of(&line.body) {
            attachments.entry(sha.clone()).or_insert_with(|| {
                row(alloc::vec![
                    ("sha256", Cell::text(&sha)),
                    ("bytes", Cell::int(size)),
                ])
            });
            event_attachments.push(row(alloc::vec![
                ("session_id", Cell::text(&id)),
                ("seq", Cell::int(seq)),
                ("sha256", Cell::text(sha)),
            ]));
        }
        events.push(row(alloc::vec![
            ("session_id", Cell::text(&id)),
            ("seq", Cell::int(seq)),
            ("method", Cell::text(line.body.method())),
            ("kind", Cell::opt_text(kind_of(&line.body).as_deref())),
            ("turn_no", Cell::opt_int(turn_no)),
            ("call_id", Cell::opt_text(call_id(&line.body).as_deref())),
            ("at", Cell::opt_text(line.at.as_deref())),
            ("at_ms", Cell::opt_int(at_ms)),
            ("bytes", Cell::int(bytes)),
            ("line", Cell::text(json)),
            ("text", Cell::opt_text(visible_text(&line.body).as_deref())),
        ]));
    }

    let created = header.created_at.as_deref().and_then(millis);
    let session = row(alloc::vec![
        ("id", Cell::text(&id)),
        ("entry", Cell::text(&source.entry)),
        ("agent_session", Cell::text(&source.agent_session)),
        ("harness", opt(Some(&header.origin.harness))),
        ("origin", Cell::text(&source.origin)),
        ("imported_from", opt(source.imported_from.as_deref())),
        ("cwd", opt(header.cwd.as_deref())),
        ("model", opt(header.model.as_deref())),
        ("mode", opt(header.mode.as_deref())),
        ("effort", opt(header.effort.as_deref())),
        (
            "acp_version",
            Cell::opt_int(header.acp_version.and_then(|v| i64::try_from(v).ok())),
        ),
        ("created_at", Cell::int(created.unwrap_or(source.now_ms))),
        (
            "updated_at",
            Cell::int(last_at.or(created).unwrap_or(source.now_ms))
        ),
        ("header", Cell::text(header_text)),
        (
            "redacted",
            header.redacted.as_ref().map_or(Cell::Null, |r| {
                Cell::json(format!(
                    "{{\"rules\":{},\"replaced\":{}}}",
                    r.rules, r.replaced
                ))
            }),
        ),
        (
            "turns",
            Cell::int(i64::try_from(summary.turns).unwrap_or(i64::MAX))
        ),
        (
            "events",
            Cell::int(i64::try_from(events.len()).unwrap_or(i64::MAX))
        ),
        ("bytes", Cell::int(total_bytes)),
        ("open_turn", Cell::bool(summary.open_turn)),
        ("last_stop", opt(summary.last_stop.map(|s| s.name()))),
    ]);
    Ok(DocumentRows {
        session,
        turns,
        events,
        attachments: attachments.into_values().collect(),
        event_attachments,
    })
}

/// The text of the history a stored `header` and its stored `lines` (each the line's JSON, in `seq` order) make.
#[must_use]
pub fn document_text<'a>(header: &str, lines: impl IntoIterator<Item = &'a str>) -> String {
    let mut text = String::from(header);
    text.push('\n');
    for line in lines {
        text.push_str(line);
        text.push('\n');
    }
    text
}

/// The [`Document`] a stored `header` and its stored `lines` make.
///
/// # Errors
///
/// A line the format refuses.
pub fn to_document<'a>(
    header: &str,
    lines: impl IntoIterator<Item = &'a str>,
) -> Result<Document, HistoryError> {
    document_text(header, lines).parse()
}

/// The history of the session `id`, read from the rows of `db`: the stored header, then every stored line in order.
///
/// # Errors
///
/// The store refuses a read, or what it holds is not a history of the format.
pub fn read_document(db: &dyn HistoryDb, id: &str) -> Result<Option<Document>, StoreError> {
    use super::{Cond, Order, Select, all_of};
    let found = db
        .select(
            Select::new(SESSIONS, 1).filtered(all_of(alloc::vec![Cond::eq("id", Cell::text(id))])),
        )
        .map_err(StoreError::from)?;
    let Some(header) = found
        .get(0, "header")
        .and_then(Cell::as_text)
        .map(str::to_owned)
    else {
        return Ok(None);
    };
    let mut lines: Vec<String> = Vec::new();
    let mut page = Select::new(EVENTS, 1000)
        .filtered(all_of(alloc::vec![Cond::eq("session_id", Cell::text(id))]))
        .ordered(alloc::vec![Order::asc("seq")]);
    page.columns = alloc::vec!["line".to_string()];
    loop {
        let rows = db.select(page.clone()).map_err(StoreError::from)?;
        lines.extend(
            rows.rows
                .iter()
                .filter_map(|row| row.first().and_then(Cell::as_text).map(str::to_owned)),
        );
        match rows.next {
            Some(next) => page.after = Some(next),
            None => break,
        }
    }
    to_document(&header, lines.iter().map(String::as_str))
        .map(Some)
        .map_err(|e| StoreError::Corrupt(format!("a stored history is not in the format: {e}")))
}

/// The columns of a `turns` row that a [`TurnRecord`] fills: who served the turn (harness, agent, account, model, effort, role, pool,
/// member, profile), how it came to be there (`why`, `moved_from`, `usage_known`), what it used, how long it ran and how it ended,
/// with the open `tags` as one JSON object. A figure the record does not know is null, never zero.
#[must_use]
pub fn turn_cells(record: &TurnRecord) -> Vec<(&'static str, Cell)> {
    let count =
        |value: Option<u64>| Cell::opt_int(value.map(|n| i64::try_from(n).unwrap_or(i64::MAX)));
    let outcome = record.outcome.as_ref();
    let tags = if record.tags.is_empty() {
        Cell::Null
    } else {
        Cell::json(serde_json::to_string(&record.tags).unwrap_or_default())
    };
    alloc::vec![
        ("recorded_at", Cell::int(record.at.saturating_mul(1000))),
        ("harness", Cell::opt_text(record.harness.as_deref())),
        ("agent", Cell::opt_text(record.agent.as_deref())),
        ("account", Cell::opt_text(record.account.as_deref())),
        ("model", Cell::opt_text(record.model.as_deref())),
        ("effort", Cell::opt_text(record.effort.as_deref())),
        ("role", Cell::opt_text(record.role.as_deref())),
        ("pool", Cell::opt_text(record.pool.as_deref())),
        ("member", Cell::opt_text(record.member.as_deref())),
        ("profile", Cell::opt_text(record.profile.as_deref())),
        ("why", Cell::opt_text(record.why.as_deref())),
        ("moved_from", Cell::opt_text(record.moved_from.as_deref())),
        (
            "usage_known",
            record.usage_known.map_or(Cell::Null, Cell::bool)
        ),
        ("tokens_input", count(record.tokens.and_then(|t| t.input))),
        ("tokens_output", count(record.tokens.and_then(|t| t.output))),
        ("tokens_cached", count(record.tokens.and_then(|t| t.cached))),
        ("cost_micros", count(record.cost.as_ref().map(|c| c.micros))),
        (
            "cost_currency",
            Cell::opt_text(record.cost.as_ref().map(|c| c.currency.as_str())),
        ),
        ("duration_ms", count(record.duration_ms)),
        (
            "limited",
            outcome.map_or(Cell::Null, |o| Cell::bool(o.limit))
        ),
        (
            "moved_on",
            outcome.map_or(Cell::Null, |o| Cell::bool(o.moved_on))
        ),
        ("tags", tags),
    ]
}

#[cfg(test)]
mod turn_tests {
    use coqui_acp_history::{Cost, Tokens, TurnOutcome, TurnRecord};

    use super::*;
    use crate::history_store::schema;

    #[test]
    fn every_column_a_record_fills_is_a_nullable_column_of_the_turns_table() {
        let table = schema::table(schema::TURNS).expect("the table");
        let mut record = TurnRecord::new(1_800_000_000);
        record.cost = Some(Cost::new(2500, "USD"));
        record.outcome = Some(TurnOutcome::default());
        for (column, _) in turn_cells(&record) {
            let found = table
                .columns
                .iter()
                .find(|each| each.name == column)
                .unwrap_or_else(|| panic!("the turns table has no column {column}"));
            assert!(found.nullable, "{column} may be unknown");
        }
    }

    #[test]
    fn what_the_record_does_not_say_is_null_and_never_zero() {
        let mut record = TurnRecord::new(1_800_000_000);
        let mut tokens = Tokens::new();
        tokens.output = Some(0);
        record.tokens = Some(tokens);
        let cells: BTreeMap<_, _> = turn_cells(&record).into_iter().collect();
        assert_eq!(cells["tokens_input"], Cell::Null);
        assert_eq!(
            cells["tokens_output"],
            Cell::int(0),
            "a reported zero is kept"
        );
        assert_eq!(cells["cost_micros"], Cell::Null);
        assert_eq!(cells["limited"], Cell::Null);
        assert_eq!(cells["tags"], Cell::Null);
    }
}
