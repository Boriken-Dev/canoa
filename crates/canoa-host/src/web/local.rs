//! What the add-in on the person's own computer needs of the host's web server, beyond its pages: a secret only this account
//! can read, the pane's tools, and the page that offers to start a host that is not running.
//!
//! Every route here answers only a request that is from this computer, names it by a loopback name, comes from a page of this very
//! origin and, with `[host.web] office_secret`, carries the home's secret. A program on this computer can send any header, so the
//! checks keep out pages of other origins and other computers; the secret is what keeps out another account on a shared one.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, threads, files and the clock are std's"
)]

use alloc::string::String;
use alloc::sync::{Arc, Weak};
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::io::{self, Write as _};
use std::path::Path;
use std::sync::mpsc;
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::time::Instant;

use serde_json::{Value, json};

use crate::local::{Link, LinkError};

/// The header the page sends its secret in.
pub(crate) const KEY_HEADER: &str = "x-canoa-office-key";
/// Where the task pane opens, polls, sends to and closes its tools' link.
pub(crate) const TOOLS_PATH: &str = "/office/tools";
/// Where the page that is shown while no host runs asks for one to be started.
pub(crate) const LAUNCH_PATH: &str = "/office/launch";
/// The Office applications that offer tools, which are the providers' names too.
pub(crate) const APPS: [&str; 3] = ["outlook", "word", "excel"];
/// The largest message either way: the same as the tool providers' door.
pub(crate) const MAX_MESSAGE: usize = crate::client_tools::MAX_MESSAGE;
/// How long a poll waits for something to say before it answers empty, so the pane asks again.
const POLL_WAIT: Duration = Duration::from_secs(20);
/// How long the host waits for a pane to ask again before it takes the pane to be closed.
const IDLE: Duration = Duration::from_secs(45);
/// How long registering takes.
const OPEN_WAIT: Duration = Duration::from_secs(5);
/// How often the host looks for panes that went quiet.
const REAP_EVERY: Duration = Duration::from_secs(5);
/// Messages one poll answers with at most.
const BATCH: usize = 64;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// The secret of this home's add-in: 32 random bytes in hexadecimal, in a file in the home, which the manifest rendered for this
/// computer carries and only this account can read.
pub(crate) struct Secret(String);

impl Secret {
    /// The secret in `dir`'s `secret` file, made when there is none (or the file holds something else).
    pub(crate) fn load(dir: &Path) -> io::Result<Self> {
        let file = dir.join("secret");
        let usable =
            |text: &str| text.len() == 64 && text.bytes().all(|byte| byte.is_ascii_hexdigit());
        match std::fs::read_to_string(&file) {
            Ok(text) if usable(text.trim()) => return Ok(Self(text.trim().to_ascii_lowercase())),
            Ok(_) => {}
            Err(error) if error.kind() == io::ErrorKind::NotFound => {}
            Err(error) => return Err(error),
        }
        std::fs::create_dir_all(dir)?;
        let mut bytes = [0_u8; 32];
        getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
        let text: String = bytes.iter().map(|byte| format!("{byte:02x}")).collect();
        let mut options = std::fs::OpenOptions::new();
        options.write(true).create(true).truncate(true);
        #[cfg(unix)]
        {
            use std::os::unix::fs::OpenOptionsExt as _;
            options.mode(0o600);
        }
        let mut written = options.open(&file)?;
        written.write_all(text.as_bytes())?;
        written.write_all(b"\n")?;
        Ok(Self(text))
    }

    /// Whether `given` is the secret, compared in time that does not say how much of it matched.
    pub(crate) fn permits(&self, given: Option<&str>) -> bool {
        let Some(given) = given else {
            return false;
        };
        let (own, other) = (self.0.as_bytes(), given.trim().as_bytes());
        let mut difference = u8::from(own.len() != other.len());
        for (index, byte) in own.iter().enumerate() {
            difference |= byte ^ other.get(index).copied().unwrap_or(0);
        }
        difference == 0
    }
}

/// One open pane: the link the host's tools hub is served on, and when its page last asked for something.
struct Pane {
    app: String,
    link: Link,
    seen: Mutex<Instant>,
}

/// How a pane's request ended: a status and the JSON it is answered with.
pub(crate) type Reply = (u16, Value);

fn ended() -> Reply {
    (410, json!({"ended": true}))
}

/// The tools of the Office applications that are open: each pane registers as a provider of the host's `@tools` and is carried by
/// the page's own requests, so no key is proven and no program is started. A pane that stops asking for [`IDLE`] is taken to be
/// closed, and its tools are gone from every session.
pub(crate) struct Tools {
    open: Box<dyn Fn() -> Option<Link> + Send + Sync>,
    panes: Mutex<HashMap<String, Arc<Pane>>>,
}

impl Tools {
    /// Tools whose providers are served on the links `open` makes.
    pub(crate) fn new(open: impl Fn() -> Option<Link> + Send + Sync + 'static) -> Arc<Self> {
        Arc::new(Self {
            open: Box::new(open),
            panes: Mutex::new(HashMap::new()),
        })
    }

    /// Looks for panes that went quiet until `stopped` is set or nothing holds the tools any more.
    pub(crate) fn watch(this: &Arc<Self>, stopped: &Arc<AtomicBool>) {
        let (weak, stopped): (Weak<Self>, _) = (Arc::downgrade(this), Arc::clone(stopped));
        let _ = std::thread::Builder::new()
            .name("canoa-office-tools".to_owned())
            .spawn(move || {
                while !stopped.load(Ordering::SeqCst) {
                    std::thread::sleep(REAP_EVERY);
                    let Some(tools) = weak.upgrade() else {
                        return;
                    };
                    tools.reap(IDLE);
                }
                if let Some(tools) = weak.upgrade() {
                    tools.reap(Duration::ZERO);
                }
            });
    }

    /// Closes every pane that has not asked for `idle`.
    pub(crate) fn reap(&self, idle: Duration) {
        let quiet: Vec<(String, Arc<Pane>)> = lock(&self.panes)
            .iter()
            .filter(|(_, pane)| lock(&pane.seen).elapsed() >= idle)
            .map(|(id, pane)| (id.clone(), Arc::clone(pane)))
            .collect();
        for (id, pane) in quiet {
            lock(&self.panes).remove(&id);
            pane.link.close();
            tracing::info!(app = %pane.app, "an Office pane stopped asking: its tools are gone");
        }
    }

    /// How many panes are open.
    #[cfg(test)]
    pub(crate) fn count(&self) -> usize {
        lock(&self.panes).len()
    }

    fn pane(&self, session: &str) -> Option<Arc<Pane>> {
        let pane = lock(&self.panes).get(session).cloned()?;
        *lock(&pane.seen) = Instant::now();
        Some(pane)
    }

    /// `POST /office/tools/open?app=word`: registers the application as a provider and answers `{"session"}`.
    pub(crate) fn open(&self, app: &str) -> Reply {
        if !APPS.contains(&app) {
            return (
                400,
                json!({"why": "an Office application that offers tools: outlook, word or excel"}),
            );
        }
        let Some(link) = (self.open)() else {
            return (503, json!({"why": "this host takes no tools now"}));
        };
        let registered = link
            .send(
                json!({"type": "register", "provider": app})
                    .to_string()
                    .as_bytes(),
            )
            .ok()
            .and_then(|()| link.receive(Some(OPEN_WAIT)).ok().flatten())
            .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
        let Some(answer) = registered else {
            link.close();
            return (503, json!({"why": "the host did not register the tools"}));
        };
        if answer["type"] != "registered" {
            link.close();
            let why = answer["why"]
                .as_str()
                .unwrap_or("the host refused the tools");
            return (409, json!({"why": why}));
        }
        let mut id = [0_u8; 16];
        if getrandom::fill(&mut id).is_err() {
            link.close();
            return (
                503,
                json!({"why": "the system has no randomness to name the session"}),
            );
        }
        let id: String = id.iter().map(|byte| format!("{byte:02x}")).collect();
        let pane = Arc::new(Pane {
            app: app.to_owned(),
            link,
            seen: Mutex::new(Instant::now()),
        });
        // A newer pane of the same application replaces the older, as the host does for a provider that returns.
        let replaced: Vec<Arc<Pane>> = {
            let mut panes = lock(&self.panes);
            let old: Vec<String> = panes
                .iter()
                .filter(|(_, held)| held.app == app)
                .map(|(key, _)| key.clone())
                .collect();
            let removed = old.iter().filter_map(|key| panes.remove(key)).collect();
            panes.insert(id.clone(), pane);
            removed
        };
        for old in replaced {
            old.link.close();
        }
        tracing::info!(app, "an Office pane offers its tools");
        (200, json!({"session": id}))
    }

    /// `GET /office/tools/poll?session=S`: what the host has for the pane, waiting for it a while. The pane asks again at once.
    pub(crate) fn poll(&self, session: &str) -> Reply {
        let Some(pane) = self.pane(session) else {
            return ended();
        };
        let mut messages = Vec::new();
        let mut wait = POLL_WAIT;
        while messages.len() < BATCH {
            match pane.link.receive(Some(wait)) {
                Ok(Some(bytes)) => {
                    if let Ok(message) = serde_json::from_slice::<Value>(&bytes) {
                        messages.push(message);
                    }
                    // What else is waiting goes with it; nothing more is waited for.
                    wait = Duration::from_millis(1);
                }
                Err(LinkError::Timeout) => break,
                Ok(None) | Err(_) => {
                    lock(&self.panes).remove(session);
                    pane.link.close();
                    if messages.is_empty() {
                        return ended();
                    }
                    break;
                }
            }
        }
        (200, json!({"messages": messages}))
    }

    /// `POST /office/tools/send?session=S`: one message of the provider's, an answer or a notice, for the host.
    pub(crate) fn send(&self, session: &str, body: &[u8]) -> Reply {
        let Some(pane) = self.pane(session) else {
            return ended();
        };
        let Ok(message) = serde_json::from_slice::<Value>(body) else {
            return (400, json!({"why": "a JSON message"}));
        };
        if !matches!(message["type"].as_str(), Some("mcp" | "close")) {
            return (400, json!({"why": "a message of type mcp or close"}));
        }
        match pane.link.send(body) {
            Ok(()) => (200, json!({"ok": true})),
            Err(LinkError::TooLong) => (413, json!({"why": "the message is too long"})),
            Err(_) => {
                lock(&self.panes).remove(session);
                ended()
            }
        }
    }

    /// `POST /office/tools/close?session=S`: the pane is closing.
    pub(crate) fn close(&self, session: &str) -> Reply {
        if let Some(pane) = lock(&self.panes).remove(session) {
            pane.link.close();
            tracing::info!(app = %pane.app, "an Office pane closed: its tools are gone");
        }
        (200, json!({"ok": true}))
    }
}

/// What the page shown while no host runs asks for: a host started. The request is carried to whatever waits for it.
pub(crate) struct Launcher {
    asked: Mutex<mpsc::Sender<()>>,
}

impl Launcher {
    /// A launcher that tells `asked` each time the page asks.
    pub(crate) fn new(asked: mpsc::Sender<()>) -> Self {
        Self {
            asked: Mutex::new(asked),
        }
    }

    /// Whether the request could be passed on.
    pub(crate) fn ask(&self) -> bool {
        lock(&self.asked).send(()).is_ok()
    }
}

#[cfg(test)]
mod tests {
    use std::path::Path;

    use super::*;

    /// Tools whose providers are served by a hub of the test's own, as the host's are.
    fn tools() -> Arc<Tools> {
        let hub = crate::client_tools::Hub::new(crate::delegate::BridgeCommand::new(
            Path::new("canoa"),
            &[],
        ));
        Tools::new(move || {
            let (client, host) = crate::local::pair(MAX_MESSAGE, Arc::new(AtomicBool::new(false)));
            let hub = Arc::clone(&hub);
            std::thread::spawn(move || {
                hub.provide("web", &[crate::control::Grant::ToolsProvide].into(), &host);
                host.close();
            });
            Some(client)
        })
    }

    fn session(reply: &Reply) -> String {
        assert_eq!(reply.0, 200, "{}", reply.1);
        reply.1["session"].as_str().expect("a session").to_owned()
    }

    #[test]
    fn a_pane_that_goes_quiet_is_closed_and_a_newer_pane_of_its_application_replaces_it() {
        let tools = tools();
        assert_eq!(
            tools.open("powerpoint").0,
            400,
            "only the applications that offer tools"
        );
        let first = session(&tools.open("word"));
        assert_eq!(tools.count(), 1);
        tools.reap(Duration::from_secs(3600));
        assert_eq!(tools.count(), 1, "a pane that asked just now is not quiet");
        tools.reap(Duration::ZERO);
        assert_eq!(tools.count(), 0);
        assert_eq!(tools.poll(&first).0, 410, "the session is gone");
        assert_eq!(tools.send(&first, b"{}").0, 410);

        let older = session(&tools.open("word"));
        let newer = session(&tools.open("word"));
        let excel = session(&tools.open("excel"));
        assert_eq!(tools.count(), 2, "one pane an application");
        assert_eq!(tools.poll(&older).0, 410);
        assert_eq!(tools.send(&newer, b"not json").0, 400);
        assert_eq!(
            tools.send(&newer, br#"{"type":"register"}"#).0,
            400,
            "only an answer or a close is the pane's to say"
        );
        assert_eq!(
            tools.send(&newer, br#"{"type":"close","stream":"x"}"#).0,
            200
        );
        assert_eq!(tools.close(&excel).0, 200);
        assert_eq!(tools.count(), 1);
        assert_eq!(tools.poll(&excel).0, 410);
    }

    #[test]
    fn the_secret_is_made_once_and_read_again_and_only_itself_is_permitted() {
        let dir = std::env::temp_dir().join(format!("canoa-office-secret-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&dir);
        let first = Secret::load(&dir).expect("made");
        assert_eq!(first.0.len(), 64);
        let again = Secret::load(&dir).expect("read");
        assert_eq!(first.0, again.0, "the second load reads the file");
        assert!(first.permits(Some(&first.0)));
        assert!(
            first.permits(Some(&format!("{}\n", first.0))),
            "a trailing newline is not a difference"
        );
        assert!(!first.permits(None));
        assert!(!first.permits(Some("")));
        assert!(!first.permits(Some(&first.0[..63])));
        assert!(!first.permits(Some(&format!("{}0", first.0))));
        let mut other = first.0.clone();
        other.replace_range(0..1, if first.0.starts_with('a') { "b" } else { "a" });
        assert!(!first.permits(Some(&other)));
        std::fs::write(dir.join("secret"), "not a key").expect("written");
        assert_eq!(
            Secret::load(&dir).expect("made again").0.len(),
            64,
            "a file holding something else is replaced"
        );
        let _ = std::fs::remove_dir_all(&dir);
    }
}
