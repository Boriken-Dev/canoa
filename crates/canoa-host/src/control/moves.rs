//! `history/export` and `history/move`: a recorded session read back in the standard format, or carried to another source.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "paths are std's"
)]

use std::collections::BTreeMap;
use std::path::Path;
use std::sync::Arc;

use coqui_acp_api::guatiao::Status;
use coqui_acp_api::{CacheLifetime, SessionExport, SessionImport};
use coqui_acp_history::paging::{PageError, page};
use serde_json::{Value, json};

use super::account_move::export_from;
use super::{BAD_PARAMS, Control, NOT_FOUND, REFUSED, bad, text_param};
use crate::history::{self, Store, StoreError};
use crate::kind;

/// The most a page of an export holds, in bytes of history text.
pub(crate) const MAX_PAGE_BYTES: u64 = 4 * 1024 * 1024;
const DEFAULT_PAGE_BYTES: u64 = 1024 * 1024;

/// The recorded histories and the sources a session can move to.
pub(crate) struct Moves {
    pub(crate) store: Arc<Store>,
    pub(crate) targets: BTreeMap<String, Arc<kind::Harness>>,
    /// Where the harnesses keep the sessions the person may bring in.
    pub(crate) homes: super::import::Homes,
}

impl core::fmt::Debug for Moves {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Moves")
            .field("targets", &self.targets.keys().collect::<Vec<_>>())
            .finish_non_exhaustive()
    }
}

fn from_store(error: &StoreError) -> (i32, String) {
    match error {
        StoreError::Missing => (NOT_FOUND, error.to_string()),
        _ => (REFUSED, error.to_string()),
    }
}

pub(super) fn new_uuid() -> Result<String, (i32, String)> {
    let mut bytes = [0_u8; 16];
    getrandom::fill(&mut bytes).map_err(|e| (REFUSED, format!("no randomness: {e}")))?;
    bytes[6] = (bytes[6] & 0x0f) | 0x40;
    bytes[8] = (bytes[8] & 0x3f) | 0x80;
    let hex: String = data_encoding::HEXLOWER.encode(&bytes);
    Ok(format!(
        "{}-{}-{}-{}-{}",
        &hex[..8],
        &hex[8..12],
        &hex[12..16],
        &hex[16..20],
        &hex[20..]
    ))
}

impl Control {
    pub(super) fn history_call(
        &self,
        method: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        let moves = self
            .history
            .as_ref()
            .ok_or((REFUSED, StoreError::Off.to_string()))?;
        let entry = text_param(params, "entry")?.ok_or_else(|| bad("entry is needed"))?;
        let session = text_param(params, "session")?.ok_or_else(|| bad("session is needed"))?;
        if entry.len() > 256 || session.len() > 256 {
            return Err(bad("entry and session are at most 256 bytes"));
        }
        if method == "history/move" {
            return moves.moved_from(entry, session, params);
        }
        if !moves.store.recording() {
            return Err((REFUSED, StoreError::Off.to_string()));
        }
        let doc = moves
            .store
            .load(entry, session)
            .map_err(|e| from_store(&e))?;
        export(&doc, params)
    }

    /// `history/portable`: what each source that runs a harness can export and import.
    pub(super) fn history_portable(&self) -> Result<Value, (i32, String)> {
        let moves = self
            .history
            .as_ref()
            .ok_or((REFUSED, StoreError::Off.to_string()))?;
        Ok(json!({"entries": moves.portable()}))
    }
}

fn export(doc: &coqui_acp_history::Document, params: &Value) -> Result<Value, (i32, String)> {
    let max = match params.get("max_bytes") {
        None | Some(Value::Null) => DEFAULT_PAGE_BYTES,
        Some(value) => value
            .as_u64()
            .filter(|max| (1..=MAX_PAGE_BYTES).contains(max))
            .ok_or_else(|| bad(&format!("max_bytes is 1 to {MAX_PAGE_BYTES}")))?,
    };
    let after = text_param(params, "after")?;
    let text = doc
        .to_string()
        .map_err(|e| (REFUSED, format!("the history cannot be written: {e}")))?;
    match page(&text, after, max) {
        Ok(page) => {
            let mut out = json!({"id": doc.header.id, "text": page.text});
            if let Some(next) = page.next {
                out["next"] = Value::String(next);
            }
            Ok(out)
        }
        Err(PageError::LineTooLarge(size)) => Err((
            REFUSED,
            format!("line too large: {size} bytes, more than max_bytes {max}"),
        )),
        Err(other) => Err(bad(&other.to_string())),
    }
}

/// The probes' working directory: a place no session of the agent's is in, so asking it finds none.
fn probe_cwd() -> String {
    std::env::temp_dir().to_string_lossy().into_owned()
}

/// What `harness` can export and import, found by asking its provider about a session that cannot exist: a refusal for want of
/// support says no, anything else (a session it does not have, an id it does not take) says it can.
fn portability(harness: &kind::Harness) -> Value {
    let cwd = probe_cwd();
    let Ok(id) = new_uuid() else {
        return json!({"export": false, "import": false, "replay": false, "why": "no randomness to probe with"});
    };
    let unsupported = |status: Status| status == Status::GUATIAO_ERR_UNSUPPORTED;
    let connection = harness.connection(None, None, CacheLifetime::default());
    let why = match connection
        .harness
        .export_session(SessionExport::new(id, cwd.clone()), connection.spawner)
    {
        Err(error) if unsupported(error.status) => error.message().to_owned(),
        _ => return json!({"export": true, "import": true, "replay": false}),
    };
    // Only an id that is no UUID and a history with nothing in it: a provider that seeds sessions refuses it as a bad value.
    let connection = harness.connection(None, None, CacheLifetime::default());
    match connection.harness.import_session(
        SessionImport::new("not-a-session-id", cwd, ""),
        connection.spawner,
    ) {
        Err(error) if unsupported(error.status) => {
            json!({"export": false, "import": false, "replay": false, "why": error.message()})
        }
        _ => json!({"export": false, "import": true, "replay": true, "why": why}),
    }
}

impl Moves {
    /// What each source that runs a harness can export and import, by the name of its entry.
    pub(super) fn portable(&self) -> Value {
        Value::Object(
            self.targets
                .iter()
                .map(|(name, harness)| (name.clone(), portability(harness)))
                .collect(),
        )
    }

    /// `history/move`: the session as this host recorded it, else as its own agent exports it (the entry's provider, asked for
    /// the session in `cwd`, which the move then needs from the caller), carried to the source `to`.
    pub(super) fn moved_from(
        &self,
        entry: &str,
        session: &str,
        params: &Value,
    ) -> Result<Value, (i32, String)> {
        let recorded = if self.store.recording() {
            match self.store.load(entry, session) {
                Ok(doc) => Some(doc),
                Err(StoreError::Missing) => None,
                Err(other) => return Err(from_store(&other)),
            }
        } else {
            None
        };
        match recorded {
            Some(doc) => self.moved(&doc, params, "record"),
            None => {
                let doc = self.exported(entry, session, params)?;
                self.moved(&doc, params, "provider")
            }
        }
    }

    /// The history of `session` as the agent behind `entry` exports it.
    fn exported(
        &self,
        entry: &str,
        session: &str,
        params: &Value,
    ) -> Result<coqui_acp_history::Document, (i32, String)> {
        let gone = || (NOT_FOUND, StoreError::Missing.to_string());
        let harness = self.targets.get(entry).ok_or_else(gone)?;
        let cwd = text_param(params, "cwd")?.ok_or_else(|| {
            bad("cwd is needed: the host has no record of this session, so its agent is asked for it where it ran")
        })?;
        if cwd.chars().any(char::is_control) {
            return Err(bad("cwd is an absolute path"));
        }
        let text = match export_from(harness, None, session, cwd) {
            Ok(text) => text,
            Err(Err(refused)) => return Err(refused),
            Err(Ok((status, _))) if status == Status::GUATIAO_ERR_NOT_FOUND => return Err(gone()),
            Err(Ok((_, why))) => {
                return Err((
                    REFUSED,
                    format!(
                        "{entry} cannot export this conversation, and the host has no record of it: {why}"
                    ),
                ));
            }
        };
        text.parse()
            .map_err(|error: coqui_acp_history::HistoryError| {
                (
                    REFUSED,
                    format!("the exported history cannot be read: {error}"),
                )
            })
    }

    fn moved(
        &self,
        doc: &coqui_acp_history::Document,
        params: &Value,
        via: &str,
    ) -> Result<Value, (i32, String)> {
        let to = text_param(params, "to")?.ok_or_else(|| bad("to is needed"))?;
        let target = self.targets.get(to).ok_or_else(|| {
            (
                NOT_FOUND,
                "this host has no such source to move to".to_owned(),
            )
        })?;
        let cwd = text_param(params, "cwd")?
            .map(str::to_owned)
            .or_else(|| doc.header.cwd.clone())
            .ok_or_else(|| bad("cwd is needed: the history does not say where it ran"))?;
        let path = crate::share::plain(Path::new(&cwd));
        if !path.is_absolute() || cwd.chars().any(char::is_control) {
            return Err(bad("cwd is an absolute path"));
        }
        if !target.roots().is_empty()
            && !target
                .roots()
                .iter()
                .any(|root| path.starts_with(crate::share::plain(Path::new(root))))
        {
            return Err(bad("that source serves only under its working roots"));
        }
        let session = new_uuid()?;
        let moved = history::reidentified(doc, &session);
        let text = moved
            .to_string()
            .map_err(|e| (REFUSED, format!("the history cannot be written: {e}")))?;
        let connection = target.connection(None, None, CacheLifetime::default());
        let report = connection
            .harness
            .import_session(
                SessionImport::new(session.clone(), cwd.clone(), text),
                connection.spawner,
            )
            .map_err(|error| {
                let code = if error.status == Status::GUATIAO_ERR_BAD_VALUE {
                    BAD_PARAMS
                } else {
                    REFUSED
                };
                (
                    code,
                    format!("{to} did not take the history: {}", error.message()),
                )
            })?;
        // The session answers to its own history from here on, so it can be moved or exported again.
        let recorded = self.store.seed(to, &session, &moved);
        if let Err(error) = &recorded {
            tracing::warn!("a moved session's history was not kept: {error}");
        }
        let losses: Vec<Value> = report
            .losses
            .iter()
            .map(|loss| json!({"kind": loss.kind, "count": loss.count, "detail": loss.detail}))
            .collect();
        Ok(json!({
            "session": session,
            "entry": to,
            "cwd": cwd,
            "replayed": report.replayed(),
            "compacted": report.compacted,
            "seededFrom": report.seeded_from.as_ref().map(|point| point.line),
            "recorded": recorded.is_ok(),
            "via": via,
            "losses": losses,
        }))
    }
}
