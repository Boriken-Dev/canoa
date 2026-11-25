//! Whether an installed overlay's source holds something newer than what is installed: one `ls-remote` of the ref the record
//! names, and, when the head moved, a fetch that compares the tree of the overlay's folder, so a commit elsewhere in a shared
//! repository is no update. What was found is kept in memory for a time and asked again in the background.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "clocks and threads are std's"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant, SystemTime, UNIX_EPOCH};

use serde_json::{Map, Value, json};

use super::{Outcome, Overlays, Trust, same_digest};
use crate::profile::{OverlaySource, full_commit_id};

/// The longest a look that failed waits before it is made again, whatever the time to live.
const RETRY: Duration = Duration::from_secs(60);

/// What a source's ref is: a branch, a tag, or a whole commit id.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Follows {
    Branch,
    Tag,
    Commit,
}

impl Follows {
    const fn name(self) -> &'static str {
        match self {
            Self::Branch => "branch",
            Self::Tag => "tag",
            Self::Commit => "commit",
        }
    }
}

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
enum State {
    /// What is installed is what the source holds.
    Current,
    /// The source holds something else.
    Available,
    /// The source names one commit, which never moves.
    Pinned,
    /// The source could not be asked.
    Unreachable,
    /// The record names a source this host does not ask.
    Unknown,
}

impl State {
    const fn name(self) -> &'static str {
        match self {
            Self::Current => "current",
            Self::Available => "available",
            Self::Pinned => "pinned",
            Self::Unreachable => "unreachable",
            Self::Unknown => "unknown",
        }
    }
}

/// One overlay as it was last looked at.
#[derive(Clone, Debug)]
struct Look {
    /// The record's source and commit the look was made for.
    source: String,
    installed: String,
    state: State,
    /// The commit the source's ref named, when it could be asked.
    head: Option<String>,
    follows: Option<Follows>,
    problem: Option<String>,
    /// Milliseconds since the epoch, for the client; and when, for the time to live.
    checked: u64,
    at: Instant,
}

impl Look {
    fn stale(&self, ttl: Duration) -> bool {
        let wait = if self.state == State::Unreachable {
            ttl.min(RETRY)
        } else {
            ttl
        };
        self.at.elapsed() >= wait
    }

    /// This look as it reads for the record that is installed now: what was installed since, when it is what the look found,
    /// is current; a look made for another source or another commit says nothing.
    fn for_record(&self, source: &str, installed: &str) -> Option<Self> {
        if self.source != source {
            return None;
        }
        if self.installed == installed {
            return Some(self.clone());
        }
        (self.state == State::Available && self.head.as_deref() == Some(installed)).then(|| Self {
            installed: installed.to_owned(),
            state: State::Current,
            ..self.clone()
        })
    }

    fn json(&self) -> Value {
        let mut out = json!({
            "state": self.state.name(),
            "installed": self.installed,
            "checked": self.checked,
        });
        if let Some(follows) = self.follows {
            out["follows"] = json!(follows.name());
        }
        if self.state == State::Available
            && let Some(head) = &self.head
        {
            out["latest"] = json!(head);
        }
        if let Some(problem) = &self.problem {
            out["problem"] = json!(problem);
        }
        out
    }
}

/// What the host kept of its looks.
#[derive(Debug, Default)]
pub(super) struct Kept {
    looks: Mutex<BTreeMap<String, Look>>,
    /// One look at a time; a request that finds one running answers from what is kept.
    one: Mutex<()>,
    /// Whether a background look has been started and has not ended.
    running: AtomicBool,
}

fn now_millis() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |since| {
            u64::try_from(since.as_millis()).unwrap_or(u64::MAX)
        })
}

/// What `record` names as its source and its commit.
fn identity(record: &Value) -> Option<(&str, &str)> {
    Some((
        record.get("source")?.as_str()?,
        record.get("commit")?.as_str()?,
    ))
}

impl Overlays {
    /// `overlay/updates` `{"refresh"?}` -> `{"overlays": {name: look}, "available", "checked", "stale", "refreshing", "checking"}`: whether the
    /// source of each installed overlay holds something other than what is installed, as last looked at. A look kept less than
    /// `ttl` ago answers with no network. Anything not looked at yet, or past `ttl`, is answered as it is (`checking` for an overlay
    /// never looked at) and looked at again on a thread of its own, so a page never waits on git; `refresh: true` looks at every
    /// source now and answers when done.
    pub(crate) fn updates(self: &Arc<Self>, params: &Value, ttl: Duration) -> Outcome {
        let refresh = match params.get("refresh") {
            None | Some(Value::Null) => false,
            Some(Value::Bool(on)) => *on,
            Some(_) => return Err(super::bad("refresh is true or false")),
        };
        if refresh {
            self.look_at_all(ttl, true)?;
        }
        let records = self.installed_records()?;
        let looks = self
            .updates
            .looks
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let mut found = Map::new();
        let (mut available, mut stale, mut checking, mut refreshing) = (0_u64, false, false, false);
        let mut checked: Option<u64> = None;
        for (name, record) in &records {
            let Some((source, installed)) = identity(record) else {
                continue;
            };
            match looks
                .get(name)
                .and_then(|look| look.for_record(source, installed))
            {
                Some(look) => {
                    refreshing |= look.stale(ttl);
                    stale |= look.state == State::Unreachable;
                    available += u64::from(look.state == State::Available);
                    checked = Some(checked.map_or(look.checked, |old| old.min(look.checked)));
                    found.insert(name.clone(), look.json());
                }
                None => {
                    checking = true;
                    found.insert(
                        name.clone(),
                        json!({"state": "checking", "installed": installed}),
                    );
                }
            }
        }
        drop(looks);
        if !refresh && (refreshing || checking) {
            self.look_in_background(ttl);
        }
        Ok(json!({
            "overlays": found,
            "available": available,
            "checked": checked,
            "stale": stale,
            "refreshing": refreshing,
            "checking": checking,
        }))
    }

    /// Looks, on a thread of its own, at every source whose last look is past `ttl`; one such thread at a time.
    pub(crate) fn look_in_background(self: &Arc<Self>, ttl: Duration) {
        if self.root.is_none()
            || self
                .updates
                .running
                .compare_exchange(false, true, Ordering::SeqCst, Ordering::SeqCst)
                .is_err()
        {
            return;
        }
        let overlays = Arc::clone(self);
        let started = std::thread::Builder::new()
            .name("overlay-updates".to_owned())
            .spawn({
                let overlays = Arc::clone(&overlays);
                move || {
                    let _ = overlays.look_at_all(ttl, false);
                    overlays.updates.running.store(false, Ordering::SeqCst);
                }
            });
        if started.is_err() {
            overlays.updates.running.store(false, Ordering::SeqCst);
        }
    }

    /// Looks, on this thread, at every installed overlay whose last look is past `ttl` (all of them when `force`), and forgets
    /// the looks at overlays that are gone.
    pub(crate) fn look_at_all(&self, ttl: Duration, force: bool) -> Result<(), (i32, String)> {
        let _one = self
            .updates
            .one
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        let records = self.installed_records()?;
        for (name, record) in &records {
            let Some((source, installed)) = identity(record) else {
                continue;
            };
            let kept = self
                .updates
                .looks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .get(name)
                .cloned();
            let current = kept
                .as_ref()
                .and_then(|look| look.for_record(source, installed));
            let next = match current {
                Some(look) if !force && !look.stale(ttl) => look,
                prior => self.look(name, record, prior.as_ref()),
            };
            self.updates
                .looks
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .insert(name.clone(), next);
        }
        self.updates
            .looks
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .retain(|name, _| records.contains_key(name));
        Ok(())
    }

    fn installed_records(&self) -> Result<BTreeMap<String, Value>, (i32, String)> {
        let answer = self.installed(&json!({}))?;
        Ok(answer
            .get("overlays")
            .and_then(Value::as_object)
            .map(|found| {
                found
                    .iter()
                    .map(|(name, record)| (name.clone(), record.clone()))
                    .collect()
            })
            .unwrap_or_default())
    }

    /// Asks `record`'s source what its ref names now. An overlay chosen from a catalog is asked as `overlay/check` fetches it,
    /// with none of the person's git settings; one the person named is asked as they would from a shell.
    fn look(&self, name: &str, record: &Value, prior: Option<&Look>) -> Look {
        let (source, installed) = identity(record).unwrap_or_default();
        let mut look = Look {
            source: source.to_owned(),
            installed: installed.to_owned(),
            state: State::Unknown,
            head: None,
            follows: None,
            problem: None,
            checked: now_millis(),
            at: Instant::now(),
        };
        let Ok(OverlaySource::Git {
            repo,
            reference,
            path,
        }) = OverlaySource::parse_git(source)
        else {
            look.problem = Some("its source is not one this host asks".to_owned());
            return look;
        };
        if full_commit_id(&reference) {
            look.state = State::Pinned;
            look.follows = Some(Follows::Commit);
            return look;
        }
        let trust = if record.get("catalog").is_some_and(Value::is_string) {
            Trust::Isolated
        } else {
            Trust::Own
        };
        let (follows, head) = match self.remote_ref(trust, &repo, &reference) {
            Ok(found) => found,
            Err((_, why)) => {
                look.state = State::Unreachable;
                look.problem = Some(why);
                return look;
            }
        };
        look.follows = Some(follows);
        look.head = Some(head.clone());
        look.state = if head == installed {
            State::Current
        } else if let Some(known) = prior.filter(|prior| prior.head.as_deref() == Some(&head)) {
            // The head is the one already compared with what is installed.
            known.state
        } else {
            self.compare(trust, name, record, &repo, &reference, &path)
        };
        look
    }

    /// Whether the folder the source names holds other files at its head than the installed copy's digest: a head that moved with
    /// the overlay's folder as it was is no update. A fetch that fails leaves it an update, since the head did move.
    fn compare(
        &self,
        trust: Trust,
        name: &str,
        record: &Value,
        repo: &str,
        reference: &str,
        path: &str,
    ) -> State {
        let Some(digest) = record.get("digest").and_then(Value::as_str) else {
            return State::Available;
        };
        let gate = self.gate(name);
        let _one = gate.lock().unwrap_or_else(PoisonError::into_inner);
        let Ok(cache) = self.cache(name) else {
            return State::Available;
        };
        if self.init_cache(&cache).is_err() {
            return State::Available;
        }
        let Ok(commit) = self.fetch(trust, &cache, repo, reference) else {
            return State::Available;
        };
        match self.list(&cache, &commit, path) {
            Ok((_, _, listed, _)) if same_digest(&listed, digest) => State::Current,
            _ => State::Available,
        }
    }
}
