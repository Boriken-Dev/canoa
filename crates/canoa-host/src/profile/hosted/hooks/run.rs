//! One run of a hook's process: its payload on standard input, its output read up to a bound, and a time limit at which the
//! process itself is stopped. What it started is its own business, as with an agent's own hooks.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::sync::{Mutex, PoisonError};
use std::time::Instant;

use crate::spawn::{Child, Command, Stdio};

/// How often the run looks to see whether the process ended.
const POLL: Duration = Duration::from_millis(5);
/// How long a pipe held open past the hook's end is waited for.
const GRACE: Duration = Duration::from_millis(500);

/// What to run.
pub(super) struct Job<'a> {
    /// The interpreter, absolute.
    pub(super) program: &'a Path,
    /// Its arguments: the script, then the event.
    pub(super) args: Vec<OsString>,
    /// Where it runs: the session's folder.
    pub(super) cwd: &'a Path,
    /// The whole environment: nothing else is inherited.
    pub(super) env: &'a [(String, String)],
    /// The payload.
    pub(super) input: &'a [u8],
    /// The longest the run may take.
    pub(super) timeout: Duration,
    /// The most output read; one byte more is kept, so a caller can see it was passed.
    pub(super) max_output: usize,
}

/// How a run ended.
pub(super) struct Outcome {
    /// Whether the process started.
    pub(super) started: bool,
    /// What it wrote to standard output, up to one byte past the bound.
    pub(super) stdout: Vec<u8>,
    /// Its exit status; `-1` when it was stopped or has none.
    pub(super) exit: i32,
    /// Whether it was stopped for taking too long.
    pub(super) timed_out: bool,
    /// Whether how the process ended could not be read, so what it said is not known: it was ended, and this is no timeout
    /// and no pass.
    pub(super) unreadable: bool,
    /// Whole seconds it ran.
    pub(super) seconds: u64,
}

impl Outcome {
    fn not_started() -> Self {
        Self {
            started: false,
            stdout: Vec::new(),
            exit: -1,
            timed_out: false,
            unreadable: false,
            seconds: 0,
        }
    }
}

/// What a look at the process found.
enum Finished {
    Running,
    Exited(i32),
    /// It has ended, or its status was taken by someone else: how it ended cannot be read.
    Unreadable,
}

/// Whether the process has ended and how.
fn finished(child: &mut Child) -> Finished {
    match child.try_wait() {
        Ok(None) => Finished::Running,
        Ok(Some(status)) => Finished::Exited(status.code().unwrap_or(-1)),
        Err(_) => Finished::Unreadable,
    }
}

/// Stops the process, not what it started, and collects it.
fn end(child: &mut Child) {
    let _ = child.kill();
    let _ = child.wait();
}

/// Runs `job` to its end, its time limit or its output bound, whichever is first.
pub(super) fn run(job: &Job<'_>) -> Outcome {
    let mut command = Command::new(job.program);
    command
        .args(&job.args)
        .current_dir(job.cwd)
        .env_clear()
        .envs(job.env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::Piped)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Piped);
    // The agent's windows are the person's: a hook's console is not one of them.
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let Ok(mut child) = command.spawn() else {
        return Outcome::not_started();
    };
    let started = Instant::now();
    let streams = (child.stdin.take(), child.stdout.take(), child.stderr.take());
    let (Some(mut stdin), Some(mut stdout), Some(mut stderr)) = streams else {
        let _ = child.kill();
        let _ = child.wait();
        return Outcome::not_started();
    };
    let max = job.max_output;
    let over = Arc::new(AtomicBool::new(false));
    let closed = Arc::new(AtomicBool::new(false));
    let out = Arc::new(Mutex::new(Vec::new()));
    // Threads of their own that nothing waits on past a deadline: a process the hook started can hold a pipe open after the
    // hook has ended, and must not hold the relay with it.
    let input = job.input.to_vec();
    std::thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
    {
        let (over, closed, out) = (Arc::clone(&over), Arc::clone(&closed), Arc::clone(&out));
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            while let Ok(read) = stdout.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let mut kept = out.lock().unwrap_or_else(PoisonError::into_inner);
                let room = (max + 1).saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..read.min(room)]);
                if kept.len() > max {
                    over.store(true, Ordering::SeqCst);
                    break;
                }
            }
            closed.store(true, Ordering::SeqCst);
        });
    }
    {
        // Standard error never reaches the agent: it is read to its end so the process never waits on it.
        std::thread::spawn(move || {
            let mut buffer = [0_u8; 8192];
            while stderr.read(&mut buffer).is_ok_and(|read| read > 0) {}
        });
    }
    let until = started + job.timeout;
    let mut exit = None;
    let mut timed_out = false;
    let mut unreadable = false;
    loop {
        match finished(&mut child) {
            Finished::Exited(code) => {
                exit = Some(code);
                break;
            }
            Finished::Unreadable => {
                unreadable = true;
                end(&mut child);
                break;
            }
            Finished::Running => {}
        }
        if over.load(Ordering::SeqCst) {
            end(&mut child);
            break;
        }
        if Instant::now() >= until {
            timed_out = true;
            end(&mut child);
            break;
        }
        std::thread::sleep(POLL);
    }
    // What the process wrote is read to its end when the pipe closes; a process it left holding the pipe gets a short grace, no
    // more.
    let grace = Instant::now() + GRACE;
    while !closed.load(Ordering::SeqCst) && Instant::now() < grace {
        std::thread::sleep(POLL);
    }
    let take = |buffer: &Mutex<Vec<u8>>| {
        buffer
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    };
    Outcome {
        started: true,
        stdout: take(&out),
        exit: exit.unwrap_or(-1),
        timed_out,
        unreadable,
        seconds: started.elapsed().as_secs(),
    }
}
