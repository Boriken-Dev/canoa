//! `history/importable`, `history/import` and `history/imported`: the person's earlier sessions with the agents whose providers
//! can find them, read from those programs' own folders and brought into the host's histories.
//!
//! Nothing here runs unless the person asks. The folders are only ever read: an agent's provider finds a session by walking them
//! (never by a name a request or a file chose) and reads it into the standard history, which the host writes as a history of its
//! own, under the id the session already has. What a history says is data, and the history format holds it to its bounds.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "folders and files are std's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::PathBuf;

use canoa::vendor::{HistoryRequest, SessionFile, SessionsRequest};
use coqui_acp_history::conversation::is_uuid;
use coqui_acp_history::{Body, Document};
use serde_json::{Value, json};

use super::moves::Moves;
use super::{REFUSED, bad, text_param};
use crate::history::{StoreError, uuid_of};

/// The key of a history's header `_meta` that says the host brought it in, and from where.
pub(crate) const IMPORT_KEY: &str = "dev.boriken.canoa/import";

/// Sessions one answer lists for a harness, newest first.
const MAX_LISTED: usize = 500;
/// Files one walk looks at for a harness.
const MAX_WALKED: usize = 20_000;
/// Sessions one `history/import` takes.
const MAX_IMPORTED: usize = 200;

/// Where each harness keeps its sessions on this computer: where each program itself looks, or, for a test, only the folders named.
#[derive(Debug, Clone)]
pub(crate) struct Homes(Option<BTreeMap<String, PathBuf>>);

impl Default for Homes {
    /// No folder for any harness.
    fn default() -> Self {
        Self(Some(BTreeMap::new()))
    }
}

impl Homes {
    /// Where the programs themselves look (each agent's provider says: its configuration folder, in the user's home).
    pub(crate) const fn of_this_computer() -> Self {
        Self(None)
    }

    /// Only these folders, and `home` for `harness` too.
    #[cfg(test)]
    pub(crate) fn with(mut self, harness: &str, home: PathBuf) -> Self {
        self.0
            .get_or_insert_with(BTreeMap::new)
            .insert(harness.to_owned(), home);
        self
    }

    /// The sessions `harness` kept, newest first, the newest `peek` read for a title and folder; empty for a harness with no folder
    /// here or no provider that finds sessions.
    fn listing(&self, harness: &str, peek: u64) -> Option<canoa::vendor::SessionListing> {
        let mut request = SessionsRequest::new(MAX_WALKED as u64, peek);
        match &self.0 {
            None => {}
            Some(homes) => request.home = Some(homes.get(harness)?.to_string_lossy().into_owned()),
        }
        crate::vendors::behave(harness, |vendor| vendor.sessions(request))?
            .ok()
            .flatten()
    }
}

/// Whether the host can bring in the sessions harness `harness`'s own program kept on this computer: whether its provider finds
/// them.
pub(crate) fn imports(harness: &str) -> bool {
    crate::vendors::behave(harness, |vendor| {
        vendor.sessions(SessionsRequest::new(0, 0))
    })
    .is_some_and(|answer| answer.ok().flatten().is_some())
}

fn harness_param(params: &Value) -> Result<(&str, &str), (i32, String)> {
    let harness = text_param(params, "harness")?.ok_or_else(|| bad("harness is needed"))?;
    if !imports(harness) {
        return Err(bad(
            "the harness is not one whose sessions can be brought in",
        ));
    }
    let entry = text_param(params, "entry")?.ok_or_else(|| bad("entry is needed"))?;
    if entry.is_empty() || entry.len() > 256 {
        return Err(bad("entry is 1 to 256 bytes"));
    }
    Ok((harness, entry))
}

impl Moves {
    /// The sessions of a harness the person could bring in: counts, titles and folders, never contents.
    pub(super) fn importable(&self, params: &Value) -> Result<Value, (i32, String)> {
        let (harness, entry) = harness_param(params)?;
        let all = self
            .homes
            .listing(harness, MAX_LISTED as u64)
            .map(|listing| listing.files)
            .unwrap_or_default();
        let found = all.len();
        let sessions: Vec<Value> = all
            .iter()
            .take(MAX_LISTED)
            .map(|each| {
                json!({
                    "id": each.id,
                    "title": each.title,
                    "cwd": each.cwd,
                    "bytes": each.bytes,
                    "updated": each.updated_ms,
                    "imported": self.store.has(entry, &uuid_of(entry, &each.id)),
                })
            })
            .collect();
        Ok(
            json!({"harness": params["harness"], "entry": entry, "found": found, "sessions": sessions}),
        )
    }

    /// Brings the named sessions into the host's histories. A session that cannot come is named with why, and the rest still do.
    pub(super) fn import(&self, params: &Value) -> Result<Value, (i32, String)> {
        let (harness, entry) = harness_param(params)?;
        let ids: Vec<&str> = params
            .get("ids")
            .and_then(Value::as_array)
            .ok_or_else(|| bad("ids is a list"))?
            .iter()
            .map(|id| {
                id.as_str()
                    .filter(|id| is_uuid(id))
                    .ok_or_else(|| bad("an id is a UUID"))
            })
            .collect::<Result<_, _>>()?;
        if ids.len() > MAX_IMPORTED {
            return Err(bad(&format!("at most {MAX_IMPORTED} sessions at a time")));
        }
        if !self.store.recording() {
            return Err((REFUSED, StoreError::Off.to_string()));
        }
        let listing = self.homes.listing(harness, 0);
        let limit = listing.as_ref().map_or(0, |listing| listing.max_bytes);
        let all = listing.map(|listing| listing.files).unwrap_or_default();
        let (mut imported, mut refused) = (Vec::new(), Vec::new());
        for id in ids {
            let outcome = all
                .iter()
                .find(|each| each.id == id)
                .ok_or_else(|| "no such session is in its folder".to_owned())
                .and_then(|each| self.bring(harness, entry, each, limit));
            match outcome {
                Ok(done) => imported.push(done),
                Err(why) => refused.push(json!({"id": id, "why": why})),
            }
        }
        Ok(json!({"imported": imported, "refused": refused}))
    }

    fn bring(
        &self,
        harness: &str,
        entry: &str,
        each: &SessionFile,
        limit: u64,
    ) -> Result<Value, String> {
        let session = uuid_of(entry, &each.id);
        if self.store.has(entry, &session) {
            return Err("it is already imported".to_owned());
        }
        let file =
            fs::File::open(&each.path).map_err(|_| "its file could not be read".to_owned())?;
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| "its file could not be read".to_owned())?;
        if bytes.len() as u64 > limit {
            return Err(format!("it is larger than {limit} bytes"));
        }
        let text = String::from_utf8_lossy(&bytes).into_owned();
        let skipped = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter(|line| {
                !(line.trim_start().starts_with('{') && serde_json::from_str::<Value>(line).is_ok())
            })
            .count();
        let answer = crate::vendors::behave(harness, |vendor| {
            vendor.session_history(HistoryRequest::new(text, &session))
        })
        .ok_or_else(|| "its sessions cannot be brought in".to_owned())?
        .map_err(|error| error.message().to_owned())?;
        let mut doc =
            Document::from_slice(answer.history.as_bytes()).map_err(|error| error.to_string())?;
        if !doc
            .lines
            .iter()
            .any(|line| matches!(line.body, Body::Prompt { .. }))
        {
            return Err("it holds no conversation".to_owned());
        }
        let title = answer.title;
        let mut mark = json!({"entry": entry, "session": session});
        if let Some(title) = &title {
            mark["title"] = Value::String(title.clone());
        }
        let mut meta = doc.header.meta.take().unwrap_or_else(|| json!({}));
        if let Some(object) = meta.as_object_mut() {
            object.insert(IMPORT_KEY.to_owned(), mark);
            doc.header.meta = Some(meta);
        }
        let turns = doc
            .lines
            .iter()
            .filter(|line| matches!(line.body, Body::Prompt { .. }))
            .count();
        self.store
            .seed(entry, &session, &doc)
            .map_err(|error| error.to_string())?;
        Ok(json!({
            "id": each.id,
            "session": session,
            "title": title,
            "turns": turns,
            "notCarried": {"lines": skipped},
        }))
    }

    /// The sessions imported under `entry`, newest first.
    pub(super) fn imported(&self, params: &Value) -> Result<Value, (i32, String)> {
        let entry = text_param(params, "entry")?.ok_or_else(|| bad("entry is needed"))?;
        if entry.len() > 256 {
            return Err(bad("entry is at most 256 bytes"));
        }
        if !self.store.recording() {
            return Err((REFUSED, StoreError::Off.to_string()));
        }
        let sessions: Vec<Value> = self
            .store
            .imported(entry)
            .into_iter()
            .map(|each| json!({"session": each.session, "title": each.title, "cwd": each.cwd, "at": each.at}))
            .collect();
        Ok(json!({"entry": entry, "sessions": sessions}))
    }
}

impl super::Control {
    pub(super) fn import_call(&self, method: &str, params: &Value) -> Result<Value, (i32, String)> {
        let moves = self
            .history
            .as_ref()
            .ok_or_else(|| (REFUSED, StoreError::Off.to_string()))?;
        match method {
            "history/importable" => moves.importable(params),
            "history/import" => moves.import(params),
            _ => moves.imported(params),
        }
    }
}
