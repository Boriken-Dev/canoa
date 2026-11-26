//! Where events become notices: the host fires an event, the hub says who is told and delivers one notice to each notifier on a
//! thread of its own, so a session never waits on a slow service and a failing notifier never stops the next.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and locks are std's"
)]

use std::collections::VecDeque;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, Mutex, PoisonError};
use std::thread::JoinHandle;

use canoa::notifier::{Notice, SessionRef};
use serde_json::Value;

use super::desktop::{LinkState, Links};
use super::directory::{BUILT_IN, Directory, Offered};
use super::events;
use super::words::{self, Facts};

/// How many `(event, session, request)` triples are remembered to tell one request's notice from its repeat.
const REMEMBERED: usize = 1024;

/// Reads what `[notifications]` and the libraries say now.
pub(crate) type Source = Box<dyn Fn() -> Result<Arc<Directory>, String> + Send + Sync>;

/// What became of the notice for one notifier.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Delivered {
    /// The notifier's name.
    pub(crate) notifier: String,
    /// `Ok`, or one line saying why not.
    pub(crate) outcome: Result<(), String>,
}

/// What happened, for the hub to tell.
#[derive(Debug, Default)]
pub(crate) struct Occurrence {
    /// The session it is about.
    pub(crate) session: Option<SessionRef>,
    /// The id of the request it answers, so one request tells the person once.
    pub(crate) request: Option<String>,
    /// A figure the phrase carries.
    pub(crate) figure: Option<String>,
}

/// The host's way of telling the person.
pub(crate) struct Hub {
    /// Tells this hub's notices from another's: when it was made, in nanoseconds.
    id: u128,
    links: Arc<Links>,
    seen: Mutex<VecDeque<String>>,
    sequence: AtomicU64,
    source: Source,
}

impl core::fmt::Debug for Hub {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Hub")
            .field("id", &self.id)
            .finish_non_exhaustive()
    }
}

fn now() -> i64 {
    std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        })
}

impl Hub {
    /// A hub that reads what to do from `source` each time an event fires.
    pub(crate) fn new(source: Source) -> Arc<Self> {
        let links = Arc::new(Links::default());
        let id = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        Arc::new(Self {
            id,
            links,
            seen: Mutex::new(VecDeque::new()),
            sequence: AtomicU64::new(0),
            source,
        })
    }

    /// A hub over one directory that never changes.
    #[cfg(test)]
    pub(crate) fn fixed(directory: Directory) -> Arc<Self> {
        let directory = Arc::new(directory);
        Self::new(Box::new(move || Ok(Arc::clone(&directory))))
    }

    /// What `[notifications]` says now.
    pub(crate) fn directory(&self) -> Result<Arc<Directory>, String> {
        (self.source)()
    }

    /// Opens a window's link, which then receives the `desktop` notices.
    pub(crate) fn subscribe(&self, language: &str) -> Arc<LinkState> {
        let link = LinkState::new(language);
        self.links.add(&link);
        link
    }

    /// Closes a window's link: its notices stop and what it said it was looking at is forgotten.
    pub(crate) fn unsubscribe(&self, link: &Arc<LinkState>) {
        self.links.remove(link);
    }

    /// Whether any window is open on the host.
    pub(crate) fn has_window(&self) -> bool {
        !self.links.all().is_empty()
    }

    fn language(&self) -> String {
        self.links
            .all()
            .first()
            .map(|link| {
                link.language
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .clone()
            })
            .unwrap_or_else(|| "en".to_owned())
    }

    fn attended(&self, session: &str) -> bool {
        self.links.all().iter().any(|link| {
            link.attended
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .as_deref()
                == Some(session)
        })
    }

    fn first_time(&self, key: String) -> bool {
        let mut seen = self.seen.lock().unwrap_or_else(PoisonError::into_inner);
        if seen.contains(&key) {
            return false;
        }
        if seen.len() >= REMEMBERED {
            seen.pop_front();
        }
        seen.push_back(key);
        true
    }

    /// Tells the person about `event`. Nothing is sent for an event the catalog lacks, for a request already told, or for a session
    /// a window shows the person right now. The answer is the thread that resolves who is told and delivers; the host drops it, a test
    /// joins it.
    pub(crate) fn fire(
        self: &Arc<Self>,
        event: &str,
        occurrence: Occurrence,
    ) -> Option<JoinHandle<Vec<Delivered>>> {
        let Some(known) = events::find(event) else {
            tracing::warn!("an event no catalog has was fired: {event}");
            return None;
        };
        if let Some(request) = &occurrence.request {
            let session = occurrence
                .session
                .as_ref()
                .map_or("", |each| each.id.as_str());
            if !self.first_time(format!("{event}\u{0}{session}\u{0}{request}")) {
                return None;
            }
        }
        if let Some(session) = &occurrence.session
            && self.attended(&session.id)
        {
            tracing::debug!(event, "not told: the person is looking at the session");
            return None;
        }
        let facts = Facts {
            session: occurrence.session,
            figure: occurrence.figure,
        };
        let (title, body) = words::notice(event, &self.language(), &facts);
        let number = self.sequence.fetch_add(1, Ordering::Relaxed);
        let at = now();
        let mut notice = Notice::new(
            format!("{at:x}-{}-{number}", self.id),
            event,
            title,
            body,
            at,
        );
        notice.session = facts.session;
        notice.urgency = known.urgency;
        let hub = Arc::clone(self);
        Some(std::thread::spawn(move || hub.deliver(&notice)))
    }

    /// Delivers `notice` through `offered`: the host's own provider (the windows) is told by the host that owns the windows.
    fn dispatch(
        links: &Links,
        offered: &Offered,
        settings: serde_json::Map<String, Value>,
        notice: Notice,
    ) -> Result<(), String> {
        if offered.origin == BUILT_IN {
            return super::desktop::push(links, &notice);
        }
        offered
            .send(Value::Object(settings).to_string(), notice)
            .map(|_| ())
    }

    fn deliver(&self, notice: &Notice) -> Vec<Delivered> {
        let directory = match self.directory() {
            Ok(directory) => directory,
            Err(why) => {
                tracing::warn!("notices go where the catalog says, canoa.cfg was not read: {why}");
                return Vec::new();
            }
        };
        let mut sending: Vec<(String, JoinHandle<Result<(), String>>)> = Vec::new();
        let mut done: Vec<Delivered> = Vec::new();
        for name in directory.route(&notice.event) {
            let resolved = match directory.resolve(&name) {
                Ok(resolved) => resolved,
                Err(why) => {
                    tracing::warn!(notifier = name, "a notice was not sent: {why}");
                    done.push(Delivered {
                        notifier: name,
                        outcome: Err(why),
                    });
                    continue;
                }
            };
            if resolved
                .offered
                .info
                .events
                .as_ref()
                .is_some_and(|takes| !takes.contains(&notice.event))
            {
                continue;
            }
            let settings = resolved.settings;
            let links = Arc::clone(&self.links);
            let directory = Arc::clone(&directory);
            let notice = notice.clone();
            let notifier = name.clone();
            sending.push((
                name,
                std::thread::spawn(move || {
                    let outcome = directory.resolve(&notifier).and_then(|resolved| {
                        Self::dispatch(&links, resolved.offered, settings, notice)
                    });
                    if let Err(why) = &outcome {
                        tracing::warn!(notifier, "a notice was not delivered: {why}");
                    }
                    outcome
                }),
            ));
        }
        for (notifier, handle) in sending {
            let outcome = handle
                .join()
                .unwrap_or_else(|_| Err("the notifier panicked".to_owned()));
            done.push(Delivered { notifier, outcome });
        }
        done
    }

    /// Sends a notice of the words "Test" to the notifier `name`, no matter what routes say.
    ///
    /// # Errors
    ///
    /// One line saying why it could not be delivered.
    pub(crate) fn test(&self, name: &str) -> Result<(), String> {
        let directory = self.directory()?;
        let resolved = directory.resolve(name)?;
        let (title, body) = if self.language().starts_with("es") {
            ("Canoa", "Prueba")
        } else {
            ("Canoa", "Test")
        };
        let mut notice = Notice::new(format!("test-{}", self.id), "test", title, body, now());
        notice.urgency = canoa::notifier::Urgency::Low;
        Self::dispatch(&self.links, resolved.offered, resolved.settings, notice)
    }
}
