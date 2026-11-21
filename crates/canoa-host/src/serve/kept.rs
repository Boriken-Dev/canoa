//! A shared session's update log on disk, so that it survives the host's restart: the cursors clients hold stay theirs, and a
//! client that comes back with one is replayed what it missed in the order it happened.
//!
//! One file a session in `<home>/shared_sessions/`, readable by this user alone where the system has such a thing: a header line
//! naming the entry and the session, then one line for each update, exactly as the hub fanned it out. The hub appends to the
//! file as it logs and rewrites it, trimmed to what it still keeps, when the file has grown to twice that. A session the person
//! or the agent ended leaves no file; one the host's stopping interrupted, or a crash, leaves its file, which the next host
//! reads only for as long as the entry's idle time.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files and clocks are std's"
)]

use std::collections::BTreeMap;
use std::fs;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::time::{Duration, SystemTime};

use coqui_acp_hub::{Cursor, Hub, LogSnapshot};
use serde_json::{Value, json};
use sha2::{Digest as _, Sha256};

const MAGIC: &str = "canoa-shared-session-log";
const VERSION: u64 = 1;
/// How many lines past twice the log the file may hold before it is rewritten, so that a small log is not rewritten at each
/// update.
const SLACK: usize = 64;

/// Where the shared sessions' logs are kept.
#[derive(Debug)]
pub(super) struct Kept {
    dir: PathBuf,
}

impl Kept {
    pub(super) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    fn path(&self, entry: &str, session: &str) -> PathBuf {
        let mut digest = Sha256::new();
        digest.update(entry.as_bytes());
        digest.update([0]);
        digest.update(session.as_bytes());
        let name = data_encoding::HEXLOWER.encode(&digest.finalize());
        self.dir.join(format!("{name}.jsonl"))
    }

    /// What the host kept of the session `session` of `entry`, unless it is older than `idle` or is not that session's. A file
    /// that is too old is removed. An update the last write cut short, or anything after one that is not an update, is not part
    /// of it.
    pub(super) fn load(
        &self,
        entry: &str,
        session: &str,
        idle: Option<Duration>,
    ) -> Option<LogSnapshot> {
        let path = self.path(entry, session);
        let file = fs::File::open(&path).ok()?;
        if let Some(idle) = idle
            && age(&file).is_some_and(|age| age >= idle)
        {
            drop(file);
            let _ = fs::remove_file(&path);
            return None;
        }
        let mut lines = BufReader::new(file).lines();
        let header: Value = serde_json::from_str(&lines.next()?.ok()?).ok()?;
        if header["canoa"] != MAGIC
            || header["version"] != VERSION
            || header["entry"] != entry
            || header["session"] != session
        {
            return None;
        }
        let mut entries = Vec::new();
        for line in lines {
            let Ok(line) = line else { break };
            let Ok(message) = serde_json::from_str::<Value>(&line) else {
                break;
            };
            entries.push(message);
        }
        let head = entries
            .last()
            .and_then(|message| message.pointer("/params/_meta"))
            .and_then(|meta| meta.get(coqui_acp_hub::META_KEY))
            .and_then(|own| own.get("seq"))
            .and_then(Value::as_str)
            .and_then(|seq| seq.parse::<Cursor>().ok())?;
        Some(LogSnapshot::new(head, entries))
    }

    /// Starts keeping the log `hub` holds, replacing whatever was kept for its session.
    pub(super) fn begin(&self, entry: &str, hub: &Hub) -> std::io::Result<Keeper> {
        let path = self.path(entry, hub.session_id());
        let mut keeper = Keeper {
            path,
            entry: entry.to_owned(),
            session: hub.session_id().to_owned(),
            file: None,
            written: hub.head(),
            lines: 0,
            failed: false,
        };
        keeper.rewrite(hub)?;
        Ok(keeper)
    }

    /// Removes the logs of entries whose idle time has passed: `idles` is each shared entry's. A log of an entry it does not
    /// name is left alone.
    pub(super) fn sweep(&self, idles: &BTreeMap<String, Option<Duration>>) {
        let Ok(files) = fs::read_dir(&self.dir) else {
            return;
        };
        for file in files.flatten() {
            let path = file.path();
            let Ok(open) = fs::File::open(&path) else {
                continue;
            };
            let Some(Ok(first)) = BufReader::new(&open).lines().next() else {
                continue;
            };
            let Ok(header) = serde_json::from_str::<Value>(&first) else {
                continue;
            };
            let Some(Some(idle)) = header["entry"].as_str().and_then(|entry| idles.get(entry))
            else {
                continue;
            };
            if age(&open).is_some_and(|age| age >= *idle) {
                drop(open);
                let _ = fs::remove_file(&path);
            }
        }
    }
}

fn age(file: &fs::File) -> Option<Duration> {
    SystemTime::now()
        .duration_since(file.metadata().ok()?.modified().ok()?)
        .ok()
}

/// The file one live session's log is kept in.
#[derive(Debug)]
pub(super) struct Keeper {
    path: PathBuf,
    entry: String,
    session: String,
    file: Option<fs::File>,
    /// The newest update on disk.
    written: Cursor,
    lines: usize,
    /// A write failed: nothing more is tried, and the host's log said so once.
    failed: bool,
}

impl Keeper {
    /// Writes what the hub logged since the last call.
    pub(super) fn sync(&mut self, hub: &Hub) {
        if self.failed {
            return;
        }
        let mut text = String::new();
        let mut added = 0;
        for message in hub.log_after(self.written) {
            text.push_str(&message.to_string());
            text.push('\n');
            added += 1;
        }
        if added == 0 {
            return;
        }
        let result = if self.lines + added > 2 * hub.log_len() + SLACK {
            self.rewrite(hub)
        } else {
            match &mut self.file {
                Some(file) => file.write_all(text.as_bytes()),
                None => Err(std::io::Error::other("the file is not open")),
            }
            .map(|()| {
                self.written = hub.head();
                self.lines += added;
            })
        };
        if let Err(error) = result {
            self.failed = true;
            tracing::warn!(
                entry = self.entry,
                session = self.session,
                "the log of a shared session could not be written, so it will not survive a restart: {error}"
            );
        }
    }

    /// Writes the whole log the hub holds to a new file and puts it in the old one's place.
    fn rewrite(&mut self, hub: &Hub) -> std::io::Result<()> {
        self.file = None;
        if let Some(dir) = self.path.parent() {
            crate::control::private_dir(dir)?;
        }
        let snapshot = hub.log_snapshot();
        let mut text = json!({
            "canoa": MAGIC,
            "version": VERSION,
            "entry": self.entry,
            "session": self.session,
        })
        .to_string();
        text.push('\n');
        for message in &snapshot.entries {
            text.push_str(&message.to_string());
            text.push('\n');
        }
        crate::replace::write_file(&self.path, text.as_bytes(), crate::replace::PRIVATE)?;
        self.file = Some(fs::OpenOptions::new().append(true).open(&self.path)?);
        self.written = snapshot.head;
        self.lines = snapshot.entries.len();
        Ok(())
    }

    /// Ends the log: the session is over for good, and nothing of it is kept.
    pub(super) fn remove(self) {
        drop(self.file);
        let _ = fs::remove_file(&self.path);
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coqui_acp_hub::Client;

    fn files_in(dir: &std::path::Path) -> usize {
        fs::read_dir(dir).map_or(0, |files| files.flatten().count())
    }

    fn scratch(name: &str) -> PathBuf {
        let dir = std::env::temp_dir().join(format!("canoa-kept-{}-{name}", std::process::id()));
        let _ = fs::remove_dir_all(&dir);
        dir
    }

    fn update(text: &str) -> Value {
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s",
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})
    }

    fn hub_with(updates: usize) -> Hub {
        let mut hub = Hub::builder("s").build();
        hub.connect(Client::new("a")).expect("connect");
        for n in 0..updates {
            let _ = hub.from_agent(&update(&format!("t{n}")));
        }
        hub
    }

    #[test]
    fn what_a_hub_logged_comes_back_as_it_was_and_a_cut_line_is_dropped() {
        let dir = scratch("roundtrip");
        let kept = Kept::new(dir.clone());
        let mut hub = hub_with(3);
        let mut keeper = kept.begin("e", &hub).expect("begins");
        let _ = hub.from_agent(&update("later"));
        keeper.sync(&hub);
        let loaded = kept.load("e", "s", None).expect("kept");
        assert_eq!(loaded, hub.log_snapshot());
        assert!(kept.load("other", "s", None).is_none());
        assert!(kept.load("e", "other", None).is_none());

        // A write the host died in the middle of.
        let mut file = fs::OpenOptions::new()
            .append(true)
            .open(kept.path("e", "s"))
            .expect("opens");
        file.write_all(b"{\"jsonrpc\": \"2.0\", \"meth")
            .expect("writes");
        assert_eq!(kept.load("e", "s", None), Some(hub.log_snapshot()));
        keeper.remove();
        assert!(kept.load("e", "s", None).is_none());
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_file_is_rewritten_when_it_has_grown_past_the_log_and_an_old_one_is_not_read() {
        let dir = scratch("compact");
        let kept = Kept::new(dir.clone());
        let mut hub = Hub::builder("s").max_log_entries(2).build();
        let mut keeper = kept.begin("e", &hub).expect("begins");
        for n in 0..200 {
            let _ = hub.from_agent(&update(&format!("t{n}")));
            keeper.sync(&hub);
        }
        let lines = fs::read_to_string(kept.path("e", "s"))
            .expect("read")
            .lines()
            .count();
        assert!(lines < 2 * 2 + SLACK + 3, "{lines} lines for 2 updates");
        // What was kept, given to a hub with the same bound, is what the hub held.
        let loaded = kept.load("e", "s", None).expect("kept");
        let again = Hub::builder("s")
            .max_log_entries(2)
            .restore_log(loaded)
            .expect("restores")
            .build();
        assert!(again.log_snapshot() == hub.log_snapshot());

        assert!(kept.load("e", "s", Some(Duration::ZERO)).is_none());
        assert_eq!(files_in(&dir), 0, "an old file is removed");
        let _ = fs::remove_dir_all(dir);
    }

    #[test]
    fn a_sweep_removes_what_an_entrys_idle_time_has_passed_and_nothing_else() {
        let dir = scratch("sweep");
        let kept = Kept::new(dir.clone());
        let hub = hub_with(1);
        let _a = kept.begin("old", &hub).expect("begins");
        let _b = kept.begin("long", &hub).expect("begins");
        let _c = kept.begin("unknown", &hub).expect("begins");
        let mut idles = BTreeMap::new();
        idles.insert("old".to_owned(), Some(Duration::ZERO));
        idles.insert("long".to_owned(), Some(Duration::from_secs(86_400)));
        kept.sweep(&idles);
        assert!(kept.load("old", "s", None).is_none());
        assert!(kept.load("long", "s", None).is_some());
        assert!(kept.load("unknown", "s", None).is_some());
        let _ = fs::remove_dir_all(dir);
    }
}
