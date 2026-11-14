//! What a harness's own hooks tell the host: Claude Code runs `canoa hook-forward` for the moments ACP does not show
//! (compaction, a working directory that changed), and the host, which stays the one place that runs an overlay's hooks,
//! runs them and answers.
//!
//! The forwarder is a program of the host's own that prints nothing to the harness: it reports on a loopback channel only
//! this connection's host reads, and exits with the status the host's answer or the hook's fail mode gives. What an
//! overlay's hook prints never reaches the harness. The channel's address and secret are in a file of a directory made for
//! one session, never on a command line, and the host answers a request only if it carries that secret and the answer
//! carries the request's nonce.
//!
//! The threat is another local user, and another session's agent, writing to the channel: a request is bounded in size and
//! rate, a secret is good for one session, and nothing the harness sends past a few named fields is passed on.

use alloc::collections::{BTreeMap, VecDeque};
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::{IpAddr, Ipv4Addr, SocketAddr};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::io::{BufRead as _, Read as _, Write as _};
use std::net::{TcpListener, TcpStream};
use std::path::Path;
use std::sync::mpsc::{Sender, channel};
use std::sync::{Mutex, MutexGuard, PoisonError};
use std::thread::JoinHandle;
use std::time::Instant;

use serde_json::{Map, Value, json};

use super::super::Written;

/// The file in a session's directory that holds the channel's address and secret.
pub(super) const CHANNEL_FILE: &str = "channel.json";
/// The subcommand the forwarder runs as.
pub(crate) const COMMAND: &str = "hook-forward";
/// The longest request line the host reads. What the harness's payload adds is cut to named fields before it is sent.
const MAX_REQUEST: usize = 8 * 1024;
/// The longest answer line the forwarder reads.
const MAX_ANSWER: usize = 1024;
/// The longest a payload from the harness is read, and the longest each field kept from it.
const MAX_PAYLOAD: u64 = 1024 * 1024;
const MAX_FIELD: usize = 2048;
/// How long a request may take to arrive, once connected.
const READ_LIMIT: Duration = Duration::from_secs(3);
/// How long a hook may take to answer: an event's hooks may run for [`coqui_aos::Settings`]' bound, and a margin.
pub(super) const ANSWER_LIMIT: Duration = Duration::from_secs(130);
/// How long the forwarder waits to connect.
const CONNECT_LIMIT: Duration = Duration::from_secs(2);
/// Requests waiting for the host, and handled at once.
const MAX_QUEUED: usize = 64;
const MAX_HANDLERS: usize = 16;
/// Secrets the host holds at once: one for each session it has a channel for.
pub(super) const MAX_SECRETS: usize = 256;
/// Requests a secret may make in a window: more than a person's compactions and sessions' starts can come to.
const RATE: usize = 30;
const WINDOW: Duration = Duration::from_secs(10);
/// Refused attempts in a window before each further one waits, and how long.
const BAD_FREE: usize = 20;
const BAD_WAIT: Duration = Duration::from_secs(1);
/// How often the accepting thread looks for a connection or for its stop.
const POLL: Duration = Duration::from_millis(20);

/// What a harness's hook reports.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(super) enum Kind {
    /// A session started, resumed, was compacted into or cleared: `source` says which.
    Start,
    /// A compaction is about to run: `trigger` says whose it is.
    PreCompact,
    /// A batch of tool calls ended: `cwd` is where the session now works.
    Batch,
}

impl Kind {
    pub(super) const fn as_str(self) -> &'static str {
        match self {
            Self::Start => "start",
            Self::PreCompact => "pre-compact",
            Self::Batch => "batch",
        }
    }

    pub(super) fn named(name: &str) -> Option<Self> {
        [Self::Start, Self::PreCompact, Self::Batch]
            .into_iter()
            .find(|kind| kind.as_str() == name)
    }
}

/// What the host answers a forwarded report.
#[derive(Clone, Copy, Debug, Default, PartialEq, Eq)]
pub(super) struct Verdict {
    /// The harness is to stop what it was about to do: only a compaction can be stopped.
    pub(super) block: bool,
}

/// One report, as the host holds it until it has run the event's hooks.
#[derive(Debug)]
pub(super) struct Forwarded {
    /// Canoa's id for the session the secret belongs to.
    pub(super) session: String,
    pub(super) kind: Kind,
    /// `source` of a start, `trigger` of a compaction; `cwd` of any.
    pub(super) source: Option<String>,
    pub(super) cwd: Option<String>,
    reply: Sender<Verdict>,
}

impl Forwarded {
    /// Answers the forwarder.
    pub(super) fn answer(self, verdict: Verdict) {
        let _ = self.reply.send(verdict);
    }
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What the accepting thread and its handlers share with the host.
#[derive(Default)]
struct Shared {
    /// Each session's secret, and when its recent requests came.
    secrets: Mutex<BTreeMap<String, Holder>>,
    queue: Mutex<VecDeque<Forwarded>>,
    handlers: AtomicUsize,
    /// When recent refused attempts came.
    refused: Mutex<VecDeque<Instant>>,
}

struct Holder {
    session: String,
    recent: VecDeque<Instant>,
}

/// The host's end of the channel: a loopback listener, and the secrets of the sessions it speaks for.
pub(super) struct Forward {
    shared: Arc<Shared>,
    addr: SocketAddr,
    stop: Arc<AtomicBool>,
    thread: Option<JoinHandle<()>>,
}

impl core::fmt::Debug for Forward {
    /// Never a secret.
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Forward")
            .field("addr", &self.addr)
            .finish_non_exhaustive()
    }
}

/// A random 128-bit value in hexadecimal.
fn random_hex() -> std::io::Result<String> {
    let mut random = [0_u8; 16];
    getrandom::fill(&mut random).map_err(|error| std::io::Error::other(error.to_string()))?;
    Ok(random.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Whether two strings are the same, taking as long for any two of the same length.
fn same(a: &str, b: &str) -> bool {
    a.len() == b.len()
        && a.bytes()
            .zip(b.bytes())
            .fold(0_u8, |all, (a, b)| all | (a ^ b))
            == 0
}

impl Forward {
    /// Listens on a loopback port of the system's choosing.
    pub(super) fn open() -> std::io::Result<Self> {
        let listener = TcpListener::bind(SocketAddr::new(IpAddr::V4(Ipv4Addr::LOCALHOST), 0))?;
        listener.set_nonblocking(true)?;
        let addr = listener.local_addr()?;
        let shared = Arc::new(Shared::default());
        let stop = Arc::new(AtomicBool::new(false));
        let thread = {
            let (shared, stop) = (Arc::clone(&shared), Arc::clone(&stop));
            std::thread::Builder::new()
                .name("canoa-hook-channel".to_owned())
                .spawn(move || accept(&listener, &shared, &stop))?
        };
        Ok(Self {
            shared,
            addr,
            stop,
            thread: Some(thread),
        })
    }

    /// Gives `session` a secret and the file that holds it: `fail` says, by [`Kind`], whether a forwarder that is not
    /// answered is to stop what the harness was about to do. The secret is good until [`Self::revoke`] or the end of the
    /// connection.
    pub(super) fn enroll(
        &self,
        session: &str,
        fail: &BTreeMap<&'static str, bool>,
    ) -> std::io::Result<Vec<u8>> {
        let mut secrets = lock(&self.shared.secrets);
        if secrets.len() >= MAX_SECRETS {
            return Err(std::io::Error::other("too many sessions have a channel"));
        }
        let secret = random_hex()?;
        secrets.insert(
            secret.clone(),
            Holder {
                session: session.to_owned(),
                recent: VecDeque::new(),
            },
        );
        let file = json!({
            "addr": self.addr.to_string(),
            "secret": &secret,
            "refuse": fail.iter().filter(|(_, refuse)| **refuse).map(|(kind, _)| *kind).collect::<Vec<_>>(),
        });
        Ok(file.to_string().into_bytes())
    }

    /// Ends every secret `session` holds.
    pub(super) fn revoke(&self, session: &str) {
        lock(&self.shared.secrets).retain(|_, holder| holder.session != session);
        // What it reported and nobody has run yet is not run: its forwarder is answered by the queue's end.
        lock(&self.shared.queue).retain(|each| each.session != session);
    }

    /// Everything reported since the last call, oldest first.
    pub(super) fn take(&self) -> Vec<Forwarded> {
        lock(&self.shared.queue).drain(..).collect()
    }
}

impl Drop for Forward {
    fn drop(&mut self) {
        self.stop.store(true, Ordering::SeqCst);
        lock(&self.shared.secrets).clear();
        // Dropped with their senders, the waiting handlers end without an answer.
        lock(&self.shared.queue).clear();
        if let Some(thread) = self.thread.take() {
            let _ = thread.join();
        }
    }
}

/// The accepting thread: one handler thread for each connection, up to a bound, until `stop`.
fn accept(listener: &TcpListener, shared: &Arc<Shared>, stop: &AtomicBool) {
    while !stop.load(Ordering::SeqCst) {
        match listener.accept() {
            Ok((stream, from)) => {
                // A connection past the bound is dropped, not queued: its forwarder fails as its fail mode says.
                if !from.ip().is_loopback()
                    || stream.set_nonblocking(false).is_err()
                    || shared.handlers.fetch_add(1, Ordering::SeqCst) >= MAX_HANDLERS
                {
                    if from.ip().is_loopback() {
                        shared.handlers.fetch_sub(1, Ordering::SeqCst);
                    }
                    continue;
                }
                let own = Arc::clone(shared);
                let spawned = std::thread::Builder::new()
                    .name("canoa-hook-request".to_owned())
                    .spawn(move || {
                        handle(&stream, &own);
                        own.handlers.fetch_sub(1, Ordering::SeqCst);
                    });
                if spawned.is_err() {
                    shared.handlers.fetch_sub(1, Ordering::SeqCst);
                }
            }
            Err(_) => std::thread::sleep(POLL),
        }
    }
}

/// A refused attempt: past [`BAD_FREE`] in a window, each one waits before it is dropped, so a guesser makes no more than
/// a few attempts a second whatever it opens.
fn note_refused(shared: &Shared) {
    let wait = {
        let mut refused = lock(&shared.refused);
        let now = Instant::now();
        while refused
            .front()
            .is_some_and(|at| now.duration_since(*at) > WINDOW)
        {
            refused.pop_front();
        }
        refused.push_back(now);
        if refused.len() > 4 * BAD_FREE {
            refused.pop_front();
        }
        refused.len() > BAD_FREE
    };
    if wait {
        std::thread::sleep(BAD_WAIT);
    }
}

/// One request. Anything that is not exactly one well-formed request with a good secret is dropped with no answer.
fn handle(stream: &TcpStream, shared: &Shared) {
    let _ = stream.set_read_timeout(Some(READ_LIMIT));
    let _ = stream.set_write_timeout(Some(READ_LIMIT));
    let mut line = Vec::new();
    let limit = u64::try_from(MAX_REQUEST + 1).unwrap_or(u64::MAX);
    let mut reader = std::io::BufReader::new(stream).take(limit);
    if reader.read_until(b'\n', &mut line).is_err() || line.len() > MAX_REQUEST {
        return note_refused(shared);
    }
    let Ok(request) = serde_json::from_slice::<Value>(&line) else {
        return note_refused(shared);
    };
    let text = |key: &str| request.get(key).and_then(Value::as_str);
    let (Some(secret), Some(nonce), Some(kind)) = (
        text("secret"),
        text("nonce").filter(|nonce| (8..=64).contains(&nonce.len())),
        text("kind").and_then(Kind::named),
    ) else {
        return note_refused(shared);
    };
    let field = |key: &str| {
        request
            .get("fields")
            .and_then(|fields| fields.get(key))
            .and_then(Value::as_str)
            .filter(|value| value.len() <= MAX_FIELD)
            .map(str::to_owned)
    };
    let receiver = {
        let mut secrets = lock(&shared.secrets);
        // Every secret is compared, so how long it takes does not say how many bytes of one were right.
        let mut found = None;
        for (held, holder) in secrets.iter_mut() {
            if same(held, secret) {
                found = Some(holder);
            }
        }
        let Some(holder) = found else {
            drop(secrets);
            return note_refused(shared);
        };
        let now = Instant::now();
        while holder
            .recent
            .front()
            .is_some_and(|at| now.duration_since(*at) > WINDOW)
        {
            holder.recent.pop_front();
        }
        if holder.recent.len() >= RATE {
            return;
        }
        holder.recent.push_back(now);
        let (sender, receiver) = channel();
        let session = holder.session.clone();
        let mut queue = lock(&shared.queue);
        if queue.len() >= MAX_QUEUED {
            return;
        }
        queue.push_back(Forwarded {
            session,
            kind,
            source: field("source"),
            cwd: field("cwd"),
            reply: sender,
        });
        receiver
    };
    let Ok(verdict) = receiver.recv_timeout(ANSWER_LIMIT) else {
        return;
    };
    let answer = json!({"nonce": nonce, "block": verdict.block});
    let mut out = answer.to_string().into_bytes();
    out.push(b'\n');
    let mut writer = stream;
    let _ = writer.write_all(&out);
}

/// The characters a path may hold to be put inside double quotes in a command Claude Code runs through a shell.
fn quotable(text: &str) -> bool {
    !text.chars().any(|c| {
        c.is_control() || matches!(c, '"' | '$' | '`' | '\\' | '!' | '%' | '\'' | '&' | ';')
    })
}

/// `program [before] hook-forward --channel FILE KIND`, quoted for a shell: `None` when a path holds a character that
/// cannot be quoted safely, so no hook is written rather than one that runs something else.
pub(super) fn command_for(
    program: &str,
    before: &[String],
    channel: &Path,
    kind: Kind,
) -> Option<String> {
    let slashed = |text: &str| text.replace('\\', "/");
    let mut parts = Vec::new();
    for each in core::iter::once(program).chain(before.iter().map(String::as_str)) {
        let each = slashed(each);
        if !quotable(&each) {
            return None;
        }
        parts.push(format!("\"{each}\""));
    }
    parts.push(COMMAND.to_owned());
    parts.push("--channel".to_owned());
    let channel = slashed(&channel.to_string_lossy());
    if !quotable(&channel) {
        return None;
    }
    parts.push(format!("\"{channel}\""));
    parts.push(kind.as_str().to_owned());
    Some(parts.join(" "))
}

/// The `hooks` object of the plugin for the kinds in `commands`.
pub(super) fn hooks_json(commands: &[(Kind, String)]) -> Value {
    let event = |kind: Kind| match kind {
        Kind::Start => "SessionStart",
        Kind::PreCompact => "PreCompact",
        Kind::Batch => "PostToolBatch",
    };
    let mut hooks = Map::new();
    for (kind, command) in commands {
        hooks.insert(
            event(*kind).to_owned(),
            json!([{"hooks": [{
                "type": "command",
                "command": command,
                "timeout": ANSWER_LIMIT.as_secs() + 5,
            }]}]),
        );
    }
    json!({ "hooks": hooks })
}

/// Writes the session's plugin under `under`: the channel file and, made from the directory's own name, the hooks that run
/// the forwarder. `None` when a command cannot be quoted.
pub(super) fn write_plugin(
    under: &Path,
    program: &str,
    before: &[String],
    kinds: &[Kind],
    channel: Vec<u8>,
) -> std::io::Result<Option<Written>> {
    let mut files = BTreeMap::new();
    files.insert(CHANNEL_FILE.to_owned(), channel);
    files.insert(
        ".claude-plugin/plugin.json".to_owned(),
        serde_json::to_vec(&json!({
            "name": "canoa-hooks",
            "version": "0.0.0",
            "description": "Reports compaction and working directory changes to the host that runs this session",
        }))
        .unwrap_or_default(),
    );
    let written = Written::create(under, &files)?;
    let at = written.root().join(CHANNEL_FILE);
    let mut commands = Vec::new();
    for kind in kinds {
        let Some(command) = command_for(program, before, &at, *kind) else {
            return Ok(None);
        };
        commands.push((*kind, command));
    }
    written.add(
        "hooks/hooks.json",
        &serde_json::to_vec(&hooks_json(&commands)).unwrap_or_default(),
    )?;
    Ok(Some(written))
}

/// What `canoa hook-forward` does: reads the harness's payload from standard input, reports its named fields to the host
/// on the channel in `channel`, and returns whether the harness is to stop what it was about to do. It prints nothing.
///
/// # Errors
///
/// A message when the host did not answer and the channel's fail mode for `kind` is to stop; otherwise a forwarder that
/// is not answered, or the host's channel being gone, is no failure.
pub(crate) fn forward(
    kind: &str,
    channel: &Path,
    mut payload: impl std::io::Read,
) -> Result<bool, String> {
    let Some(kind) = Kind::named(kind) else {
        return Err("an event this forwarder does not know".to_owned());
    };
    let read = std::fs::metadata(channel)
        .ok()
        .filter(|meta| meta.len() <= 4096)
        .and_then(|_| std::fs::read(channel).ok())
        .and_then(|bytes| serde_json::from_slice::<Value>(&bytes).ok());
    // A session whose directory is gone has ended: there is nobody to answer and nothing to stop.
    let Some(read) = read else {
        return Ok(false);
    };
    let refuse = read
        .get("refuse")
        .and_then(Value::as_array)
        .is_some_and(|kinds| {
            kinds
                .iter()
                .any(|each| each.as_str() == Some(kind.as_str()))
        });
    let stop = |why: &str| {
        if refuse {
            Err(format!(
                "an overlay's hook that guards this could not be asked, so {why}"
            ))
        } else {
            Ok(false)
        }
    };
    let (Some(addr), Some(secret)) = (
        read.get("addr")
            .and_then(Value::as_str)
            .and_then(|addr| addr.parse::<SocketAddr>().ok())
            .filter(|addr| addr.ip().is_loopback()),
        read.get("secret").and_then(Value::as_str),
    ) else {
        return stop("it was stopped");
    };
    let mut bytes = Vec::new();
    let _ = (&mut payload).take(MAX_PAYLOAD).read_to_end(&mut bytes);
    let given: Value = serde_json::from_slice(&bytes).unwrap_or(Value::Null);
    let mut fields = Map::new();
    for (from, to) in [("source", "source"), ("trigger", "source"), ("cwd", "cwd")] {
        if let Some(value) = given
            .get(from)
            .and_then(Value::as_str)
            .filter(|value| value.len() <= MAX_FIELD)
        {
            fields.insert(to.to_owned(), json!(value));
        }
    }
    let nonce = random_hex().map_err(|error| error.to_string())?;
    let request =
        json!({"secret": secret, "nonce": nonce, "kind": kind.as_str(), "fields": fields});
    let answered = ask(addr, &request, &nonce);
    match answered {
        Some(verdict) => Ok(verdict.block),
        None => stop("it was stopped"),
    }
}

/// The host's answer to `request`, when it came, was one line within bounds and echoed `nonce`.
fn ask(addr: SocketAddr, request: &Value, nonce: &str) -> Option<Verdict> {
    let mut stream = TcpStream::connect_timeout(&addr, CONNECT_LIMIT).ok()?;
    stream
        .set_read_timeout(Some(ANSWER_LIMIT + CONNECT_LIMIT))
        .ok()?;
    stream.set_write_timeout(Some(CONNECT_LIMIT)).ok()?;
    let mut line = request.to_string().into_bytes();
    line.push(b'\n');
    stream.write_all(&line).ok()?;
    let mut answer = Vec::new();
    std::io::BufReader::new(&stream)
        .take(u64::try_from(MAX_ANSWER + 1).unwrap_or(u64::MAX))
        .read_until(b'\n', &mut answer)
        .ok()?;
    if answer.len() > MAX_ANSWER {
        return None;
    }
    let answer: Value = serde_json::from_slice(&answer).ok()?;
    if answer.get("nonce")?.as_str()? != nonce {
        return None;
    }
    Some(Verdict {
        block: answer.get("block")?.as_bool()?,
    })
}

#[cfg(test)]
mod tests;
