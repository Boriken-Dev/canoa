//! Where this process's log goes while it hosts: every job that follows it, and the file `CANOA_HOST_LOG` names. The host's
//! `tracing` events come here, and so does this process's standard error, which agents inherit: a windowed application's
//! is nothing anybody reads, and an agent that writes there has to be heard and must never block.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "files, pipes and threads are std's"
)]

use std::fs::File;
use std::io::{BufRead as _, BufReader, Read as _, Write};
use std::sync::{Arc, LazyLock, Mutex, Once, PoisonError, Weak};

use tracing_subscriber::fmt::MakeWriter;

use crate::jobs::Job;
use crate::platform;

/// The longest line kept; a longer one is cut into pieces of this size.
const MAX_LINE: u64 = 16 * 1024;

static FOLLOWERS: Mutex<Vec<Weak<Job>>> = Mutex::new(Vec::new());
static FILE: LazyLock<Option<Mutex<File>>> = LazyLock::new(|| {
    let path = std::env::var_os("CANOA_HOST_LOG").filter(|path| !path.is_empty())?;
    File::options()
        .create(true)
        .append(true)
        .open(path)
        .ok()
        .map(Mutex::new)
});

/// `job` hears every line from now until it ends.
pub(crate) fn follow(job: &Arc<Job>) {
    FOLLOWERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push(Arc::downgrade(job));
}

/// One line of the log.
fn route(line: &str) {
    FOLLOWERS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .retain(|follower| {
            let Some(job) = follower.upgrade() else {
                return false;
            };
            if job.has_ended() {
                return false;
            }
            job.log(line);
            true
        });
    if let Some(file) = &*FILE {
        let mut file = file.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = writeln!(file, "{line}");
    }
}

/// What the `tracing` subscriber writes through.
#[derive(Clone, Copy, Debug)]
struct RouteWriter;

impl Write for RouteWriter {
    fn write(&mut self, bytes: &[u8]) -> std::io::Result<usize> {
        for line in String::from_utf8_lossy(bytes).lines() {
            route(line);
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

impl<'a> MakeWriter<'a> for RouteWriter {
    type Writer = Self;

    fn make_writer(&'a self) -> Self::Writer {
        *self
    }
}

fn filter() -> tracing_subscriber::EnvFilter {
    tracing_subscriber::EnvFilter::try_from_env("CANOA_LOG")
        .unwrap_or_else(|_| tracing_subscriber::EnvFilter::new("info"))
}

/// Sets this process up for hosting, once: the subscriber for any first command, and for a long one also its standard error,
/// a console for its children and broken pipes set aside. `peers` or `check` leave the application's alone.
pub(crate) fn install(long: bool) {
    static SUBSCRIBER: Once = Once::new();
    static HOSTING: Once = Once::new();
    SUBSCRIBER.call_once(|| {
        let _ = tracing_subscriber::fmt()
            .with_env_filter(filter())
            .with_writer(RouteWriter)
            .with_ansi(false)
            .with_target(false)
            .try_init();
    });
    if long {
        HOSTING.call_once(|| {
            platform::ignore_broken_pipes();
            // First: attaching to a console replaces the standard handles of a process started with none, as Explorer starts
            // an application, which would take standard error off the pipe.
            if let Err(error) = platform::hide_console_for_children() {
                tracing::warn!("an agent may open a window of its own: {error}");
            }
            if let Err(error) = capture_standard_error() {
                tracing::warn!("what agents write to standard error will not be logged: {error}");
            }
        });
    }
}

/// A host's own command logs on its standard error, which its host reads.
pub(crate) fn install_for_child() {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(filter())
        .with_writer(std::io::stderr)
        .with_ansi(false)
        .with_target(false)
        .try_init();
}

/// Puts this process's standard error on a pipe a thread reads for good.
fn capture_standard_error() -> std::io::Result<()> {
    let (reader, writer) = std::io::pipe()?;
    platform::set_standard_error(&writer)?;
    // It is this process's standard error from here on, for whatever starts or writes later.
    core::mem::forget(writer);
    std::thread::Builder::new()
        .name("canoa-stderr".to_owned())
        .spawn(move || {
            let mut reader = BufReader::new(reader);
            let mut line = Vec::new();
            loop {
                line.clear();
                match (&mut reader).take(MAX_LINE).read_until(b'\n', &mut line) {
                    Ok(0) | Err(_) => return,
                    Ok(_) => route(String::from_utf8_lossy(&line).trim_end()),
                }
            }
        })?;
    Ok(())
}
