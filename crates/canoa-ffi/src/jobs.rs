//! `canoa` commands running in this process. A job is one command on a thread of its own: what it prints and logs waits in
//! queues until read, and closing its input is what `--until-stdin-closes` waits on.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and processes are std's"
)]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::panic::{AssertUnwindSafe, catch_unwind};
use std::path::PathBuf;
use std::sync::atomic::{AtomicU64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use canoa_host::cli::{self, Cli, Front, Program};
use clap::Parser as _;
use serde::Deserialize;
use serde_json::json;

use crate::exports::{STATUS_BAD_ARGUMENT, STATUS_FAILED};
use crate::{log, platform};

/// The first argument that makes an application run as the host's own command when it starts itself.
pub const CHILD_FLAG: &str = "--canoa-child";

/// The most lines of each kind a job keeps unread; past it the oldest go, and `dropped` counts them.
const MAX_QUEUED: usize = 4096;
/// The status of a job whose command panicked.
const STATUS_JOB_PANICKED: i32 = 101;
/// The status of a command line that is not the host's own.
const STATUS_USAGE: i32 = 2;

/// What [`start`] reads.
#[derive(Debug, Deserialize)]
#[serde(deny_unknown_fields)]
struct Request {
    /// The command line, without the program's name.
    args: Vec<String>,
    /// The directory of the identity, trust store and lock; this user's own when absent.
    home: Option<PathBuf>,
    /// What `serve` calls itself to another host that finds its home held.
    holder: Option<String>,
    /// The program that runs `provider`, `tool` and `launch`; this application, with [`CHILD_FLAG`], when absent.
    program: Option<PathBuf>,
    /// What goes before the subcommand on `program`'s command line; only with `program`.
    program_args: Option<Vec<String>>,
}

#[derive(Default)]
struct Queues {
    out: VecDeque<String>,
    err: VecDeque<String>,
    dropped: u64,
    exit: Option<i32>,
}

#[derive(Default)]
struct Input {
    closed: bool,
    then: Option<Box<dyn FnOnce() + Send>>,
}

/// One command running or ended.
#[derive(Default)]
pub(crate) struct Job {
    queues: Mutex<Queues>,
    input: Mutex<Input>,
}

impl Job {
    fn push(&self, line: &str, queue: impl Fn(&mut Queues) -> &mut VecDeque<String>) {
        let mut queues = self.queues.lock().unwrap_or_else(PoisonError::into_inner);
        let held = queue(&mut queues);
        if held.len() == MAX_QUEUED {
            held.pop_front();
            queues.dropped += 1;
        }
        queue(&mut queues).push_back(line.to_owned());
    }

    /// One line of its log.
    pub(crate) fn log(&self, line: &str) {
        self.push(line, |queues| &mut queues.err);
    }

    pub(crate) fn has_ended(&self) -> bool {
        self.queues
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .exit
            .is_some()
    }

    fn end(&self, status: i32) {
        self.queues
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .exit = Some(status);
    }

    /// Closes its input and runs what waits on that, on a thread of its own, since ending agents takes time and the caller
    /// is an application's interface.
    fn close_input(&self) {
        let then = {
            let mut input = self.input.lock().unwrap_or_else(PoisonError::into_inner);
            input.closed = true;
            input.then.take()
        };
        if let Some(then) = then {
            run_apart(then);
        }
    }
}

/// Runs `then` on a thread of its own, or here should no thread start: what closes a job's input has to happen.
fn run_apart(then: Box<dyn FnOnce() + Send>) {
    let held = Arc::new(Mutex::new(Some(then)));
    let handed = Arc::clone(&held);
    let spawned = std::thread::Builder::new()
        .name("canoa-job-stop".to_owned())
        .spawn(move || {
            if let Some(then) = handed.lock().unwrap_or_else(PoisonError::into_inner).take() {
                then();
            }
        });
    if spawned.is_err()
        && let Some(then) = held.lock().unwrap_or_else(PoisonError::into_inner).take()
    {
        then();
    }
}

static JOBS: LazyLock<Mutex<BTreeMap<u64, Arc<Job>>>> = LazyLock::new(Mutex::default);
/// 0 is what a failed start writes, so no job has it, and a number is never used twice.
static NEXT: AtomicU64 = AtomicU64::new(1);

fn job(number: u64) -> Option<Arc<Job>> {
    JOBS.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .get(&number)
        .cloned()
}

/// The queues and input of one job, as the command sees them.
struct JobFront {
    job: Arc<Job>,
    home: Option<PathBuf>,
    holder: String,
    program: Option<Program>,
}

impl Front for JobFront {
    fn print(&self, line: &str) -> std::io::Result<()> {
        self.job.push(line, |queues| &mut queues.out);
        Ok(())
    }

    fn say(&self, line: &str) {
        self.job.log(line);
    }

    /// Whatever closes the input ends this job only: the process goes on, so `status` means nothing here.
    fn when_input_closes(
        &self,
        _status: u8,
        then: Box<dyn FnOnce() + Send>,
    ) -> std::io::Result<()> {
        let mut input = self
            .job
            .input
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if input.closed {
            drop(input);
            run_apart(then);
        } else {
            input.then = Some(then);
        }
        Ok(())
    }

    fn program(&self) -> std::io::Result<Program> {
        match &self.program {
            Some(program) => Ok(program.clone()),
            None => Ok(Program::new(
                std::env::current_exe()?,
                vec![OsString::from(CHILD_FLAG)],
            )),
        }
    }

    fn home(&self) -> Option<PathBuf> {
        self.home.clone()
    }

    fn holder(&self) -> &str {
        &self.holder
    }
}

/// Whether a command runs long enough that the process's log belongs in what it says.
fn follows_the_log(args: &[String]) -> bool {
    matches!(args.first().map(String::as_str), Some("serve" | "pair"))
}

/// Starts the command `request` names, and answers its number.
pub(crate) fn start(request: &[u8]) -> Result<u64, i32> {
    let request: Request = serde_json::from_slice(request).map_err(|_| STATUS_BAD_ARGUMENT)?;
    let program = match (request.program, request.program_args) {
        (Some(path), args) => Some(Program::new(
            path,
            args.unwrap_or_default()
                .into_iter()
                .map(OsString::from)
                .collect(),
        )),
        (None, Some(_)) => return Err(STATUS_BAD_ARGUMENT),
        (None, None) => None,
    };
    let job = Arc::new(Job::default());
    if follows_the_log(&request.args) {
        log::follow(&job);
    }
    let front = JobFront {
        job: Arc::clone(&job),
        home: request.home,
        holder: request
            .holder
            .unwrap_or_else(|| "an application".to_owned()),
        program,
    };
    let args = request.args;
    let number = NEXT.fetch_add(1, Ordering::Relaxed);
    // Registered first, so that a job ended before this returns can still be read.
    JOBS.lock()
        .unwrap_or_else(PoisonError::into_inner)
        .insert(number, job);
    let spawned = std::thread::Builder::new()
        .name("canoa-job".to_owned())
        .spawn(move || run_job(&args, &front));
    if spawned.is_err() {
        JOBS.lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&number);
        return Err(STATUS_FAILED);
    }
    Ok(number)
}

/// The command to its end. Everything it holds, a home's lock included, is given up before the job says it ended.
fn run_job(args: &[String], front: &JobFront) {
    let status = catch_unwind(AssertUnwindSafe(|| run_command(args, front))).unwrap_or_else(|p| {
        core::mem::forget(p);
        front.say("canoa: the command panicked");
        STATUS_JOB_PANICKED
    });
    front.job.end(status);
}

fn run_command(args: &[String], front: &JobFront) -> i32 {
    log::install(follows_the_log(args));
    let line = std::iter::once("canoa".to_owned()).chain(args.iter().cloned());
    let parsed = match Cli::try_parse_from(line) {
        Ok(parsed) => parsed,
        Err(error) => {
            let text = error.to_string();
            let wrong = error.use_stderr();
            for line in text.lines() {
                if wrong {
                    front.say(line);
                } else {
                    let _ = front.print(line);
                }
            }
            return if wrong { STATUS_USAGE } else { 0 };
        }
    };
    match cli::run(parsed, front) {
        Ok(()) => 0,
        Err(failed) => {
            front.say(&format!("canoa: {}", failed.why));
            i32::from(failed.status)
        }
    }
}

/// What a job has said since the last read; `None` for a number that names no job.
pub(crate) fn read(number: u64) -> Option<String> {
    let job = job(number)?;
    let mut queues = job.queues.lock().unwrap_or_else(PoisonError::into_inner);
    let text = json!({
        "out": queues.out.drain(..).collect::<Vec<_>>(),
        "err": queues.err.drain(..).collect::<Vec<_>>(),
        "dropped": core::mem::take(&mut queues.dropped),
        "exit": queues.exit,
    });
    Some(text.to_string())
}

/// Closes a job's input; whether the number names one.
pub(crate) fn stop(number: u64) -> bool {
    let Some(job) = job(number) else {
        return false;
    };
    job.close_input();
    true
}

/// Closes a job's input and forgets it; whether the number named one.
pub(crate) fn free(number: u64) -> bool {
    let job = JOBS
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .remove(&number);
    let Some(job) = job else {
        return false;
    };
    job.close_input();
    true
}

/// A front for a command a host started for itself: its own standard streams, and nothing else it may ask for.
struct ChildFront;

impl Front for ChildFront {
    fn print(&self, line: &str) -> std::io::Result<()> {
        use std::io::Write as _;
        let mut out = std::io::stdout().lock();
        writeln!(out, "{line}").and_then(|()| out.flush())
    }

    fn say(&self, line: &str) {
        eprintln!("{line}");
    }

    fn when_input_closes(
        &self,
        _status: u8,
        _then: Box<dyn FnOnce() + Send>,
    ) -> std::io::Result<()> {
        Err(std::io::Error::other(
            "a host's own command waits on nothing",
        ))
    }

    fn program(&self) -> std::io::Result<Program> {
        Err(std::io::Error::other("a host's own command starts no host"))
    }

    fn home(&self) -> Option<PathBuf> {
        None
    }

    fn holder(&self) -> &str {
        "canoa"
    }
}

/// See [`crate::exports::canoa_ffi_child_main`].
pub(crate) fn child_main() -> i32 {
    platform::share_parents_console();
    log::install_for_child();
    let args = std::env::args_os().skip(2);
    let line = std::iter::once(OsString::from("canoa")).chain(args);
    let parsed = match Cli::try_parse_from(line) {
        Ok(parsed) if parsed.is_a_hosts_own() => parsed,
        Ok(_) => {
            eprintln!("canoa: {CHILD_FLAG} takes only provider, tool or launch");
            return STATUS_USAGE;
        }
        Err(error) => {
            eprintln!("canoa: {error}");
            return if error.use_stderr() { STATUS_USAGE } else { 0 };
        }
    };
    match cli::run(parsed, &ChildFront) {
        Ok(()) => 0,
        Err(failed) => {
            eprintln!("canoa: {}", failed.why);
            i32::from(failed.status)
        }
    }
}
