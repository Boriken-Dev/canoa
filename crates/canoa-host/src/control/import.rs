//! `history/importable`, `history/import` and `history/imported`: the person's earlier Claude Code and Codex sessions, read from
//! those programs' own folders and brought into the host's histories.
//!
//! Nothing here runs unless the person asks. The folders are only ever read: a session is found by walking them (never by a name
//! a request or a file chose), read through the codec coqui-acp ships for it, and written as a history of the host's own, under
//! the id the session already has. What a history says is data, and the codecs and the history format hold it to their bounds.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "folders and files are std's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::Read as _;
use std::path::{Path, PathBuf};
use std::time::UNIX_EPOCH;

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
/// How much of a session's start is read to find its title.
const TITLE_WINDOW: u64 = 256 * 1024;
/// The longest title or folder shown, in characters.
const SHOWN: usize = 120;

/// Where each harness keeps its sessions on this computer; `None` for one with no folder to look in.
#[derive(Debug, Clone, Default)]
pub(crate) struct Homes {
    pub(crate) claude: Option<PathBuf>,
    pub(crate) codex: Option<PathBuf>,
}

impl Homes {
    /// Where the programs themselves look: `CLAUDE_CONFIG_DIR` or `.claude`, `CODEX_HOME` or `.codex`, in the user's home.
    pub(crate) fn of_this_computer() -> Self {
        let none = BTreeMap::new();
        Self {
            claude: coqui_acp_claude::config_dir(&none, false),
            codex: coqui_acp_codex::history::codex_home(&none, false),
        }
    }

    fn of(&self, harness: Harness) -> Option<&Path> {
        match harness {
            Harness::Claude => self.claude.as_deref(),
            Harness::Codex => self.codex.as_deref(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Harness {
    Claude,
    Codex,
}

impl Harness {
    fn named(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// One session found in a harness's folder.
struct Found {
    id: String,
    path: PathBuf,
    bytes: u64,
    updated_ms: u64,
}

/// Text a person will be shown that a file chose: control and bidirectional characters out, runs of space as one, cut to [`SHOWN`].
fn tidy(text: &str) -> String {
    let mut out = String::new();
    let mut gap = false;
    for ch in text.chars() {
        let hidden = ch.is_control()
            || matches!(ch, '\u{200b}'..='\u{200f}' | '\u{202a}'..='\u{202e}' | '\u{2060}'..='\u{206f}' | '\u{feff}');
        if hidden || ch.is_whitespace() {
            gap = !out.is_empty();
            continue;
        }
        if gap {
            out.push(' ');
            gap = false;
        }
        out.push(ch);
        if out.chars().count() >= SHOWN {
            break;
        }
    }
    out
}

fn is_dir(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_dir())
}

fn is_file(entry: &fs::DirEntry) -> bool {
    entry.file_type().is_ok_and(|kind| kind.is_file())
}

fn children(path: &Path) -> impl Iterator<Item = fs::DirEntry> {
    fs::read_dir(path).into_iter().flatten().flatten()
}

fn found(entry: &fs::DirEntry, id: String) -> Option<Found> {
    let meta = entry.metadata().ok()?;
    Some(Found {
        id,
        path: entry.path(),
        bytes: meta.len(),
        updated_ms: meta
            .modified()
            .ok()
            .and_then(|at| at.duration_since(UNIX_EPOCH).ok())
            .map_or(0, |since| u64::try_from(since.as_millis()).unwrap_or(0)),
    })
}

/// Every session file of `harness` under `home`, by the layout the program keeps; a link is not followed.
fn walk(harness: Harness, home: &Path) -> Vec<Found> {
    let mut out = Vec::new();
    match harness {
        Harness::Claude => {
            for project in children(&home.join("projects")).filter(is_dir) {
                for file in children(&project.path()).filter(is_file) {
                    let name = file.file_name().to_string_lossy().into_owned();
                    if let Some(id) = name.strip_suffix(".jsonl")
                        && is_uuid(id)
                        && let Some(each) = found(&file, id.to_owned())
                    {
                        out.push(each);
                    }
                    if out.len() >= MAX_WALKED {
                        return out;
                    }
                }
            }
        }
        Harness::Codex => {
            for year in children(&home.join("sessions")).filter(is_dir) {
                for month in children(&year.path()).filter(is_dir) {
                    for day in children(&month.path()).filter(is_dir) {
                        for file in children(&day.path()).filter(is_file) {
                            let name = file.file_name().to_string_lossy().into_owned();
                            // `rollout-<time>-<id>.jsonl`: the id is the last 36 characters before the extension.
                            let id = name
                                .strip_prefix("rollout-")
                                .and_then(|rest| rest.strip_suffix(".jsonl"))
                                .filter(|rest| rest.len() > 36)
                                .map(|rest| &rest[rest.len() - 36..]);
                            if let Some(id) = id
                                && rest_is_dash(&name, id)
                                && is_uuid(id)
                                && let Some(each) = found(&file, id.to_owned())
                            {
                                out.push(each);
                            }
                            if out.len() >= MAX_WALKED {
                                return out;
                            }
                        }
                    }
                }
            }
        }
    }
    out
}

/// Whether the character before `id` in `name` is the dash that joins it to the time.
fn rest_is_dash(name: &str, id: &str) -> bool {
    name.strip_suffix(".jsonl")
        .and_then(|stem| stem.strip_suffix(id))
        .is_some_and(|front| front.ends_with('-'))
}

/// The first thing the person said and the folder it was said in, from the start of a session file.
fn peek(harness: Harness, path: &Path) -> (Option<String>, Option<String>) {
    let Ok(file) = fs::File::open(path) else {
        return (None, None);
    };
    let mut head = Vec::new();
    if file.take(TITLE_WINDOW).read_to_end(&mut head).is_err() {
        return (None, None);
    }
    let text = String::from_utf8_lossy(&head);
    let mut title = None;
    let mut cwd = None;
    for line in text
        .lines()
        .filter(|line| line.trim_start().starts_with('{'))
    {
        let Ok(value) = serde_json::from_str::<Value>(line) else {
            continue;
        };
        if cwd.is_none() {
            let place = match harness {
                Harness::Claude => value.get("cwd"),
                Harness::Codex => value.get("payload").and_then(|payload| payload.get("cwd")),
            };
            cwd = place
                .and_then(Value::as_str)
                .map(tidy)
                .filter(|cwd| !cwd.is_empty());
        }
        if title.is_none() {
            title = prompt_text(harness, &value)
                .map(|said| tidy(&said))
                .filter(|said| !said.is_empty());
        }
        if title.is_some() && cwd.is_some() {
            break;
        }
    }
    (title, cwd)
}

/// The text of a line that is the person speaking, not something the program put in their place.
fn prompt_text(harness: Harness, value: &Value) -> Option<String> {
    let text = match harness {
        Harness::Claude => {
            if value.get("type")?.as_str()? != "user"
                || value.get("isMeta").and_then(Value::as_bool) == Some(true)
                || value.get("isSidechain").and_then(Value::as_bool) == Some(true)
            {
                return None;
            }
            let content = value.get("message")?.get("content")?;
            match content {
                Value::String(text) => text.clone(),
                Value::Array(blocks) => blocks
                    .iter()
                    .find_map(|block| block.get("text").and_then(Value::as_str))?
                    .to_owned(),
                _ => return None,
            }
        }
        Harness::Codex => {
            let payload = value.get("payload")?;
            if value.get("type")?.as_str()? != "response_item"
                || payload.get("type")?.as_str()? != "message"
                || payload.get("role")?.as_str()? != "user"
            {
                return None;
            }
            payload
                .get("content")?
                .as_array()?
                .iter()
                .find_map(|block| block.get("text").and_then(Value::as_str))?
                .to_owned()
        }
    };
    // Commands, caveats and injected instructions are the program's own, not the person's words.
    let trimmed = text.trim_start();
    (!trimmed.is_empty() && !trimmed.starts_with('<') && !trimmed.starts_with("# AGENTS.md"))
        .then_some(text)
}

fn harness_param(params: &Value) -> Result<(Harness, &str), (i32, String)> {
    let name = text_param(params, "harness")?.ok_or_else(|| bad("harness is needed"))?;
    let harness = Harness::named(name).ok_or_else(|| bad("harness is claude or codex"))?;
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
        let mut all = self
            .homes
            .of(harness)
            .map(|home| walk(harness, home))
            .unwrap_or_default();
        all.sort_by(|a, b| {
            b.updated_ms
                .cmp(&a.updated_ms)
                .then_with(|| a.id.cmp(&b.id))
        });
        let found = all.len();
        let sessions: Vec<Value> = all
            .iter()
            .take(MAX_LISTED)
            .map(|each| {
                let (title, cwd) = peek(harness, &each.path);
                json!({
                    "id": each.id,
                    "title": title,
                    "cwd": cwd,
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
        let all = self
            .homes
            .of(harness)
            .map(|home| walk(harness, home))
            .unwrap_or_default();
        let (mut imported, mut refused) = (Vec::new(), Vec::new());
        for id in ids {
            let outcome = all
                .iter()
                .find(|each| each.id == id)
                .ok_or_else(|| "no such session is in its folder".to_owned())
                .and_then(|each| self.bring(harness, entry, each));
            match outcome {
                Ok(done) => imported.push(done),
                Err(why) => refused.push(json!({"id": id, "why": why})),
            }
        }
        Ok(json!({"imported": imported, "refused": refused}))
    }

    fn bring(&self, harness: Harness, entry: &str, each: &Found) -> Result<Value, String> {
        let session = uuid_of(entry, &each.id);
        if self.store.has(entry, &session) {
            return Err("it is already imported".to_owned());
        }
        let file =
            fs::File::open(&each.path).map_err(|_| "its file could not be read".to_owned())?;
        let limit = match harness {
            Harness::Claude => coqui_acp_claude::history::DEFAULT_MAX_BYTES,
            Harness::Codex => coqui_acp_codex::history::DEFAULT_MAX_BYTES,
        };
        let mut bytes = Vec::new();
        file.take(limit.saturating_add(1))
            .read_to_end(&mut bytes)
            .map_err(|_| "its file could not be read".to_owned())?;
        if bytes.len() as u64 > limit {
            return Err(format!("it is larger than {limit} bytes"));
        }
        let text = String::from_utf8_lossy(&bytes);
        let mut doc: Document = match harness {
            Harness::Claude => coqui_acp_claude::history::export(&text, &session)
                .map_err(|error| error.to_string())?,
            Harness::Codex => coqui_acp_codex::history::export(&text, &session)
                .map_err(|error| error.to_string())?,
        };
        if !doc
            .lines
            .iter()
            .any(|line| matches!(line.body, Body::Prompt { .. }))
        {
            return Err("it holds no conversation".to_owned());
        }
        let skipped = text
            .lines()
            .filter(|line| !line.trim().is_empty())
            .filter(|line| {
                !(line.trim_start().starts_with('{') && serde_json::from_str::<Value>(line).is_ok())
            })
            .count();
        let (title, _) = peek(harness, &each.path);
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
