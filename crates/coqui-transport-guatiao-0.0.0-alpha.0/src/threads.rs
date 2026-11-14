//! The threads a provider's library starts, so the library is not unloaded under them.
//!
//! A host unloads a library only when its `unload` slot answers `GUATIAO_OK`. A library built with
//! `guatiao::providers!(.., unload = coqui_transport_guatiao::threads::done)` answers busy while any
//! connection of its transports still has a thread running, and joins them once none has. The list is one per
//! image: a library and its host each have their own copy of this module.

use std::sync::{Mutex, PoisonError};
use std::thread::{Builder, JoinHandle};

use guatiao::Status;

static THREADS: Mutex<Vec<JoinHandle<()>>> = Mutex::new(Vec::new());

/// Starts `body` on a thread of its own, and remembers it for [`done`].
///
/// # Errors
///
/// The system's, when it cannot start the thread.
pub fn spawn(name: &str, body: impl FnOnce() + Send + 'static) -> std::io::Result<()> {
    let thread = Builder::new().name(name.into()).spawn(body)?;
    let mut threads = THREADS.lock().unwrap_or_else(PoisonError::into_inner);
    let (finished, running): (Vec<_>, Vec<_>) =
        threads.drain(..).partition(JoinHandle::is_finished);
    *threads = running;
    for done in finished {
        let _ = done.join();
    }
    threads.push(thread);
    Ok(())
}

/// A library's say in being unloaded: `GUATIAO_ERR_BUSY` while a thread this module started still runs, and
/// `GUATIAO_OK`, with every thread joined, once none does.
pub extern "C" fn done() -> Status {
    let mut threads = THREADS.lock().unwrap_or_else(PoisonError::into_inner);
    if threads.iter().any(|thread| !thread.is_finished()) {
        return Status::GUATIAO_ERR_BUSY;
    }
    for thread in threads.drain(..) {
        let _ = thread.join();
    }
    Status::GUATIAO_OK
}
