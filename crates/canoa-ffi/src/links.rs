//! Links into a running host's entries: each is a connection inside this process, with no proof, which the application drives
//! by polling. No call blocks: two threads of the link's own move messages between its queues and the host.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "queues, locks and threads are std's"
)]

use std::collections::{BTreeMap, VecDeque};
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Condvar, LazyLock, Mutex, MutexGuard, PoisonError};
use std::time::Duration;

use canoa_host::{Link, LinkError, LocalError};
use serde_json::json;

use crate::exports::{
    STATUS_BUSY, STATUS_ENDED, STATUS_FAILED, STATUS_NO_ENTRY, STATUS_OK, STATUS_UNKNOWN_JOB,
};
use crate::jobs;

/// Messages of the host's a link holds unread; past it the host's sends wait on the application.
const MAX_INCOMING: usize = 1024;
/// Bytes of the application's a link holds unsent; past it a send is refused as busy.
const MAX_OUTGOING_BYTES: usize = 64 * 1024 * 1024;
/// How often a thread of a link looks up to see whether it was closed.
const POLL: Duration = Duration::from_millis(50);

#[derive(Default)]
struct Outgoing {
    messages: VecDeque<Vec<u8>>,
    bytes: usize,
}

#[derive(Default)]
struct Incoming {
    messages: VecDeque<String>,
    /// The host's end ended, or the link was closed.
    ended: bool,
}

struct Connection {
    link: Link,
    done: AtomicBool,
    outgoing: Mutex<Outgoing>,
    sendable: Condvar,
    incoming: Mutex<Incoming>,
    readable: Condvar,
}

fn locked<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

impl Connection {
    fn finish(&self) {
        self.done.store(true, Ordering::SeqCst);
        self.link.close();
        locked(&self.incoming).ended = true;
        self.readable.notify_all();
        self.sendable.notify_all();
    }

    /// Moves what the application sent to the host, until the link is done.
    fn write(&self) {
        let mut outgoing = locked(&self.outgoing);
        loop {
            if self.done.load(Ordering::SeqCst) {
                return;
            }
            if let Some(message) = outgoing.messages.pop_front() {
                outgoing.bytes -= message.len();
                drop(outgoing);
                if self.link.send(&message).is_err() {
                    return self.finish();
                }
                outgoing = locked(&self.outgoing);
                continue;
            }
            outgoing = self
                .sendable
                .wait_timeout(outgoing, POLL)
                .unwrap_or_else(PoisonError::into_inner)
                .0;
        }
    }

    /// Moves what the host sent to the application's queue, until the link is done.
    fn read(&self) {
        while !self.done.load(Ordering::SeqCst) {
            match self.link.receive(Some(POLL)) {
                Ok(Some(message)) => {
                    let text = String::from_utf8_lossy(&message).into_owned();
                    let mut incoming = locked(&self.incoming);
                    while incoming.messages.len() >= MAX_INCOMING
                        && !self.done.load(Ordering::SeqCst)
                    {
                        incoming = self
                            .readable
                            .wait_timeout(incoming, POLL)
                            .unwrap_or_else(PoisonError::into_inner)
                            .0;
                    }
                    incoming.messages.push_back(text);
                    drop(incoming);
                    self.readable.notify_all();
                }
                Err(LinkError::Timeout) => {}
                Ok(None) | Err(_) => return self.finish(),
            }
        }
    }
}

static LINKS: LazyLock<Mutex<BTreeMap<u64, Arc<Connection>>>> = LazyLock::new(Mutex::default);
/// 0 is what a failed open writes, so no link has it, and a number is never used twice.
static NEXT: AtomicU64 = AtomicU64::new(1);

fn link(number: u64) -> Option<Arc<Connection>> {
    locked(&LINKS).get(&number).cloned()
}

/// Opens a link to `entry` of the `serve` job `job` runs, and answers its number.
pub(crate) fn open(job: u64, entry: &str) -> Result<u64, i32> {
    let local = match jobs::local_of(job) {
        Err(()) => return Err(STATUS_UNKNOWN_JOB),
        // Not a `serve`, or one that has not bound its entries yet.
        Ok(None) => return Err(STATUS_NO_ENTRY),
        Ok(Some(local)) => local,
    };
    let link = local.open(entry).map_err(|error| match error {
        LocalError::Thread => STATUS_FAILED,
        _ => STATUS_NO_ENTRY,
    })?;
    let connection = Arc::new(Connection {
        link,
        done: AtomicBool::new(false),
        outgoing: Mutex::default(),
        sendable: Condvar::new(),
        incoming: Mutex::default(),
        readable: Condvar::new(),
    });
    for (name, run) in [
        ("canoa-link-write", Connection::write as fn(&Connection)),
        ("canoa-link-read", Connection::read),
    ] {
        let mine = Arc::clone(&connection);
        if std::thread::Builder::new()
            .name(name.to_owned())
            .spawn(move || run(&mine))
            .is_err()
        {
            connection.finish();
            return Err(STATUS_FAILED);
        }
    }
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    locked(&LINKS).insert(number, connection);
    Ok(number)
}

/// The secret the host of job `job` shares with this process for the links it opens: 32 bytes. [`STATUS_NO_ENTRY`] when the job is no
/// `serve` yet or its host could not mint one.
pub(crate) fn secret(job: u64) -> Result<[u8; 32], i32> {
    match jobs::local_of(job) {
        Err(()) => Err(STATUS_UNKNOWN_JOB),
        Ok(None) => Err(STATUS_NO_ENTRY),
        Ok(Some(local)) => local
            .link_secret()
            .map(|secret| *secret)
            .ok_or(STATUS_NO_ENTRY),
    }
}

/// Queues one message for the host; a status.
pub(crate) fn send(number: u64, message: &[u8]) -> i32 {
    let Some(connection) = link(number) else {
        return crate::exports::STATUS_UNKNOWN_LINK;
    };
    if connection.done.load(Ordering::SeqCst) {
        return STATUS_ENDED;
    }
    let mut outgoing = locked(&connection.outgoing);
    if outgoing.bytes.saturating_add(message.len()) > MAX_OUTGOING_BYTES {
        return STATUS_BUSY;
    }
    outgoing.bytes += message.len();
    outgoing.messages.push_back(message.to_vec());
    drop(outgoing);
    connection.sendable.notify_all();
    STATUS_OK
}

/// What the host has sent since the last read: `{"messages": [text...], "closed": bool}`; `closed` once nothing more will
/// come. `None` for a number that names no link.
pub(crate) fn read(number: u64) -> Option<String> {
    let connection = link(number)?;
    let mut incoming = locked(&connection.incoming);
    let text = json!({
        "messages": incoming.messages.drain(..).collect::<Vec<_>>(),
        "closed": incoming.ended,
    });
    drop(incoming);
    connection.readable.notify_all();
    Some(text.to_string())
}

/// Ends a link, and with it the entry's agent, and forgets it; whether the number named one.
pub(crate) fn close(number: u64) -> bool {
    let Some(connection) = locked(&LINKS).remove(&number) else {
        return false;
    };
    connection.finish();
    true
}
