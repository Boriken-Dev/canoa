//! What a task may be handed besides its text: the delegating session's recent turns, or the files its tools touched. The host
//! keeps both as it relays the session, bounded, and builds what is handed over in one place, so what the person is shown
//! before it goes is the text the task is given.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and paths are std's"
)]

use std::collections::VecDeque;
use std::io::Read as _;
use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use coqui_a2a::error::{A2aError, ErrorKind};

use super::choice::Choice;
use super::{Engine, Session, lock, privilege};

/// The turns the host keeps of a session, in bytes, oldest dropped first.
const KEEP_TURN_BYTES: usize = 256 * 1024;
/// The files the host remembers a session touched, oldest dropped first.
const KEEP_FILES: usize = 64;

/// What a task is given besides its text.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(crate) enum Kind {
    /// Nothing: the task text is all.
    #[default]
    Task,
    /// The session's most recent turns.
    Messages,
    /// The files the session's tools touched.
    Files,
}

impl Kind {
    pub(crate) fn parse(name: &str) -> Option<Self> {
        match name {
            "task" => Some(Self::Task),
            "messages" => Some(Self::Messages),
            "files" => Some(Self::Files),
            _ => None,
        }
    }

    pub(crate) const fn name(self) -> &'static str {
        match self {
            Self::Task => "task",
            Self::Messages => "messages",
            Self::Files => "files",
        }
    }
}

#[derive(Debug)]
struct Turn {
    agent: bool,
    text: String,
}

/// The turns and touched files of one session, as the host saw them go by.
#[derive(Debug, Default)]
pub(crate) struct Record {
    turns: VecDeque<Turn>,
    bytes: usize,
    touched: Vec<PathBuf>,
}

impl Record {
    fn push(&mut self, agent: bool, text: &str) {
        if text.is_empty() {
            return;
        }
        match self.turns.back_mut() {
            Some(last) if last.agent == agent && agent => last.text.push_str(text),
            _ => self.turns.push_back(Turn {
                agent,
                text: text.to_owned(),
            }),
        }
        self.bytes += text.len();
        while self.bytes > KEEP_TURN_BYTES && self.turns.len() > 1 {
            if let Some(dropped) = self.turns.pop_front() {
                self.bytes -= dropped.text.len();
            }
        }
    }

    /// What the person sent in a prompt.
    pub(crate) fn person(&mut self, text: &str) {
        self.push(false, text);
    }

    /// A piece of what the agent said.
    pub(crate) fn agent(&mut self, text: &str) {
        self.push(true, text);
    }

    /// A file a tool of the agent read or wrote.
    pub(crate) fn touch(&mut self, path: PathBuf) {
        self.touched.retain(|held| held != &path);
        self.touched.push(path);
        if self.touched.len() > KEEP_FILES {
            self.touched.remove(0);
        }
    }
}

/// The text of the blocks of a `session/prompt`.
pub(crate) fn prompt_text(params: &Value) -> String {
    params
        .get("prompt")
        .and_then(Value::as_array)
        .into_iter()
        .flatten()
        .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
        .filter_map(|block| block.get("text").and_then(Value::as_str))
        .collect::<Vec<_>>()
        .join("\n")
}

/// Notes what a `session/update` of the agent says into `record`: its words, and the files its tool calls name.
pub(crate) fn note_update(record: &mut Record, cwd: &Path, params: &Value) {
    let Some(update) = params.get("update") else {
        return;
    };
    match update.get("sessionUpdate").and_then(Value::as_str) {
        Some("agent_message_chunk") => {
            if let Some(text) = update.pointer("/content/text").and_then(Value::as_str) {
                record.agent(text);
            }
        }
        Some("tool_call" | "tool_call_update") => {
            let mut named: Vec<&str> = Vec::new();
            for location in update
                .get("locations")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                named.extend(location.get("path").and_then(Value::as_str));
            }
            for content in update
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
            {
                if content.get("type").and_then(Value::as_str) == Some("diff") {
                    named.extend(content.get("path").and_then(Value::as_str));
                }
            }
            for key in ["file_path", "path"] {
                named.extend(
                    update
                        .pointer(&format!("/rawInput/{key}"))
                        .and_then(Value::as_str),
                );
            }
            for path in named.into_iter().filter(|path| !path.is_empty()) {
                let path = Path::new(path);
                record.touch(if path.is_absolute() {
                    path.to_path_buf()
                } else {
                    cwd.join(path)
                });
            }
        }
        _ => {}
    }
}

/// What would be handed over: the text, and what it is made of, for the person to read first.
#[derive(Clone, Debug)]
pub(crate) struct Bundle {
    pub(crate) kind: Kind,
    /// The block put before the task text.
    pub(crate) text: String,
    /// Each turn or file in it, by name, with its size in bytes.
    pub(crate) items: Vec<(String, usize)>,
    /// What was left out, and why.
    pub(crate) skipped: Vec<String>,
    /// Whether something was cut to fit the bound.
    pub(crate) truncated: bool,
}

impl Bundle {
    /// The bundle as the person's client and the task's metadata tell it. The text is the person's to read; the metadata
    /// carries only its sizes.
    pub(crate) fn describe(&self, with_text: bool) -> Value {
        let mut out = json!({
            "kind": self.kind.name(),
            "bytes": self.text.len(),
            "items": self.items.iter().map(|(name, bytes)| json!({"name": name, "bytes": bytes})).collect::<Vec<_>>(),
            "skipped": self.skipped,
            "truncated": self.truncated,
        });
        if with_text {
            out["text"] = json!(self.text);
        }
        out
    }

    /// `task` as the child is given it: the task first, then the context handed over with it.
    pub(crate) fn with_task(&self, task: &str) -> String {
        format!(
            "{task}\n\n---\nHanded over with this task:\n\n{}",
            self.text
        )
    }
}

/// The last `limit` bytes of `text`, from a character boundary.
fn tail(text: &str, limit: usize) -> &str {
    let mut start = text.len().saturating_sub(limit);
    while !text.is_char_boundary(start) {
        start += 1;
    }
    &text[start..]
}

/// What a task is handed besides its text, or `None` when its kind is `task`. The person's own start names the preview they were
/// shown, and the task is given exactly that. An agent's call builds it now, and only when the person allowed the kind in the
/// session's delegation setup. What stays on this machine is never built for a task that crosses machines.
///
/// # Errors
///
/// A kind that is not allowed, a preview that is not held, or nothing to hand over; each says which.
pub(crate) fn attach(
    engine: &Engine,
    session: &Session,
    choice: &Choice,
    crossing: bool,
) -> Result<Option<Bundle>, A2aError> {
    let kind = choice.context;
    if kind == Kind::Task {
        return Ok(None);
    }
    let refuse = |what: String| A2aError::new(ErrorKind::InvalidParams, what);
    if crossing {
        return Err(refuse(format!(
            "context \"{}\" stays on this machine: a task that crosses machines is given its text only",
            kind.name()
        )));
    }
    if let Some(id) = &choice.context_id {
        let held = lock(&session.prepared)
            .iter()
            .find(|(held, _)| held == id)
            .map(|(_, bundle)| bundle.clone());
        return match held {
            Some(bundle) if bundle.kind == kind => Ok(Some(bundle)),
            _ => Err(refuse(format!(
                "no preview of context \"{}\" was shown for this task: ask for it again",
                kind.name()
            ))),
        };
    }
    if !lock(&session.setup).context.contains(&kind) {
        return Err(refuse(format!(
            "context \"{}\" is not available: the person has not let this session hand it on (its delegation setup)",
            kind.name()
        )));
    }
    build(session, kind, engine.config.context_bytes)
        .map(Some)
        .map_err(refuse)
}

/// Builds what `kind` hands over from what the host kept of `session`, within `limit` bytes.
///
/// # Errors
///
/// There is nothing to hand over, or the kind is `task`.
pub(crate) fn build(session: &Session, kind: Kind, limit: usize) -> Result<Bundle, String> {
    let record = lock(&session.record);
    let mut bundle = Bundle {
        kind,
        text: String::new(),
        items: Vec::new(),
        skipped: Vec::new(),
        truncated: false,
    };
    match kind {
        Kind::Task => return Err("context \"task\" hands over nothing besides the task".to_owned()),
        Kind::Messages => {
            let mut left = limit;
            let mut taken: Vec<(bool, String)> = Vec::new();
            for turn in record.turns.iter().rev() {
                if left == 0 {
                    bundle.truncated = true;
                    break;
                }
                let part = tail(&turn.text, left);
                bundle.truncated |= part.len() < turn.text.len();
                left -= part.len();
                taken.push((turn.agent, part.to_owned()));
            }
            taken.reverse();
            for (agent, text) in &taken {
                let who = if *agent { "agent" } else { "person" };
                bundle.items.push((who.to_owned(), text.len()));
                bundle.text.push_str(&format!("[{who}]\n{text}\n\n"));
            }
            if taken.is_empty() {
                return Err("this conversation has no turns to hand over yet".to_owned());
            }
            bundle.text = format!(
                "The conversation of the agent that handed you this task, its most recent turns:\n\n{}",
                bundle.text.trim_end()
            );
        }
        Kind::Files => {
            let roots = session
                .conn()
                .map(|conn| conn.roots().to_vec())
                .unwrap_or_default();
            let mut left = limit;
            let mut body = String::new();
            for path in record.touched.iter().rev() {
                let shown = path.to_string_lossy().into_owned();
                let inside = path
                    .parent()
                    .and_then(Path::to_str)
                    .ok_or_else(|| "a path has no folder".to_owned())
                    .and_then(|folder| privilege::folder(&session.cwd, Some(folder), &roots));
                if inside.is_err() {
                    bundle
                        .skipped
                        .push(format!("{shown}: outside this session's folder"));
                    continue;
                }
                if left == 0 {
                    bundle.truncated = true;
                    bundle.skipped.push(format!("{shown}: no room left"));
                    continue;
                }
                match read_text(path, left) {
                    Ok((text, cut)) => {
                        left -= text.len();
                        bundle.truncated |= cut;
                        bundle.items.push((shown.clone(), text.len()));
                        body.push_str(&format!("File {shown}:\n{text}\n\n"));
                    }
                    Err(why) => bundle.skipped.push(format!("{shown}: {why}")),
                }
            }
            if bundle.items.is_empty() {
                return Err(if bundle.skipped.is_empty() {
                    "no file has been touched in this conversation yet".to_owned()
                } else {
                    format!("no file can be handed over: {}", bundle.skipped.join("; "))
                });
            }
            bundle.text = format!(
                "Files the agent that handed you this task read or wrote, the newest first:\n\n{}",
                body.trim_end()
            );
        }
    }
    Ok(bundle)
}

/// Up to `limit` bytes of a text file, and whether more was left.
fn read_text(path: &Path, limit: usize) -> Result<(String, bool), String> {
    let file = std::fs::File::open(path).map_err(|error| format!("cannot be read: {error}"))?;
    if !file.metadata().is_ok_and(|meta| meta.is_file()) {
        return Err("is not a file".to_owned());
    }
    let mut bytes = Vec::new();
    file.take(u64::try_from(limit).unwrap_or(u64::MAX).saturating_add(4))
        .read_to_end(&mut bytes)
        .map_err(|error| format!("cannot be read: {error}"))?;
    let cut = bytes.len() > limit;
    bytes.truncate(limit);
    match String::from_utf8(bytes) {
        Ok(text) => Ok((text, cut)),
        // A cut inside a character is the bound's, not the file's.
        Err(error) if cut && error.utf8_error().error_len().is_none() => {
            let valid = error.utf8_error().valid_up_to();
            let mut bytes = error.into_bytes();
            bytes.truncate(valid);
            Ok((
                String::from_utf8(bytes).map_err(|error| error.to_string())?,
                true,
            ))
        }
        Err(_) => Err("is not text".to_owned()),
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_record_keeps_turns_in_order_joins_the_agents_pieces_and_notes_each_file_once() {
        let mut record = Record::default();
        record.person("fix it");
        record.agent("Looking");
        record.agent(" now");
        record.person("and?");
        assert_eq!(record.turns.len(), 3);
        assert_eq!(record.turns[1].text, "Looking now");
        let cwd = Path::new("/work");
        for update in [
            json!({"sessionUpdate": "tool_call", "locations": [{"path": "a.rs"}]}),
            json!({"sessionUpdate": "tool_call_update", "content": [{"type": "diff", "path": "b.rs"}]}),
            json!({"sessionUpdate": "tool_call_update", "locations": [{"path": "a.rs"}]}),
            json!({"sessionUpdate": "tool_call", "rawInput": {"file_path": "c.rs"}}),
        ] {
            note_update(&mut record, cwd, &json!({"update": update}));
        }
        assert_eq!(
            record.touched,
            [cwd.join("b.rs"), cwd.join("a.rs"), cwd.join("c.rs")]
        );
    }

    #[test]
    fn a_prompts_text_blocks_are_joined_and_others_left_out() {
        let params = json!({"prompt": [{"type": "text", "text": "a"}, {"type": "image"}, {"type": "text", "text": "b"}]});
        assert_eq!(prompt_text(&params), "a\nb");
    }

    #[test]
    fn a_cut_lands_on_a_character_and_is_the_newest_end_of_a_turn() {
        assert_eq!(tail("abc\u{e9}\u{e9}", 3), "\u{e9}");
        assert_eq!(tail("abc", 10), "abc");
    }

    #[test]
    fn a_file_is_read_as_text_up_to_the_bound_and_says_when_it_cut() {
        let dir = std::env::temp_dir().join(format!("canoa-context-{}", std::process::id()));
        std::fs::create_dir_all(&dir).expect("a directory");
        let path = dir.join("a.txt");
        std::fs::write(&path, "ab\u{e9}cd").expect("written");
        assert_eq!(
            read_text(&path, 100).expect("read"),
            ("ab\u{e9}cd".to_owned(), false)
        );
        // The bound falls inside the two-byte character: it is dropped whole.
        assert_eq!(read_text(&path, 3).expect("read"), ("ab".to_owned(), true));
        let binary = dir.join("b.bin");
        std::fs::write(&binary, [0xff, 0xfe, 0x00]).expect("written");
        assert!(read_text(&binary, 100).is_err());
        assert!(read_text(&dir, 100).is_err());
        let _ = std::fs::remove_file(&path);
        let _ = std::fs::remove_file(&binary);
        let _ = std::fs::remove_dir(&dir);
    }
}
