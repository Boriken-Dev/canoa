//! One run of a hook's process: its payload on standard input, its output read up to a bound, a time limit, and every
//! process it started ended when it ends.

use alloc::vec::Vec;
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::io::{Read as _, Write as _};
use std::path::Path;
use std::process::{Child, Command, Stdio};
use std::time::Instant;

/// How often the run looks to see whether the process ended.
const POLL: Duration = Duration::from_millis(5);

/// What to run.
pub(super) struct Job<'a> {
    /// The interpreter, absolute.
    pub(super) program: &'a Path,
    /// Its arguments: the script, then the event.
    pub(super) args: Vec<OsString>,
    /// The overlay's root.
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
    /// Whether the process started and was held by its job.
    pub(super) started: bool,
    /// What it wrote to standard output, up to one byte past the bound.
    pub(super) stdout: Vec<u8>,
    /// Its exit status; `-1` when it was stopped or has none.
    pub(super) exit: i32,
    /// Whether it was stopped for taking too long.
    pub(super) timed_out: bool,
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
            seconds: 0,
        }
    }
}

/// A started process and what ends all of it.
struct Held {
    child: Child,
    #[cfg(windows)]
    job: Option<win32job::Job>,
}

impl Held {
    /// Ends the process and everything it started, and collects it.
    fn end(&mut self) {
        #[cfg(windows)]
        drop(self.job.take());
        #[cfg(unix)]
        self.end_group();
        let _ = self.child.kill();
        let _ = self.child.wait();
    }

    #[cfg(unix)]
    fn end_group(&self) {
        if let Some(group) = i32::try_from(self.child.id())
            .ok()
            .filter(|pid| *pid > 1)
            .and_then(rustix::process::Pid::from_raw)
        {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
    }

    /// The exit status once the process has ended, with what it started ended too.
    fn finished(&mut self) -> Option<i32> {
        // Where the system can say a process has ended without collecting it, its group is ended first: until it is
        // collected, nobody else can have its id. Elsewhere the group is ended just after, which the system does not
        // reuse an id for while any process is in it.
        #[cfg(any(target_os = "linux", target_os = "android"))]
        {
            let pid = i32::try_from(self.child.id())
                .ok()
                .and_then(rustix::process::Pid::from_raw)?;
            let ended = rustix::process::waitid(
                rustix::process::WaitId::Pid(pid),
                rustix::process::WaitIdOptions::EXITED
                    | rustix::process::WaitIdOptions::NOWAIT
                    | rustix::process::WaitIdOptions::NOHANG,
            )
            .is_ok_and(|status| status.is_some());
            if !ended {
                return None;
            }
            self.end_group();
        }
        let status = self.child.try_wait().ok().flatten()?;
        #[cfg(windows)]
        drop(self.job.take());
        #[cfg(all(unix, not(any(target_os = "linux", target_os = "android"))))]
        self.end_group();
        Some(status.code().unwrap_or(-1))
    }
}

/// Runs `job` to its end, its time limit or its output bound, whichever is first.
pub(super) fn run(job: &Job<'_>) -> Outcome {
    let mut command = Command::new(job.program);
    command
        .args(&job.args)
        .current_dir(job.cwd)
        .env_clear()
        .envs(job.env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // The agent's windows are the person's: a hook's console is not one of them.
    #[cfg(windows)]
    std::os::windows::process::CommandExt::creation_flags(&mut command, 0x0800_0000);
    // Its own group, so that ending it reaches what it starts.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
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
    #[cfg(windows)]
    let job_object = {
        use std::os::windows::io::AsRawHandle as _;
        let mut limits = win32job::ExtendedLimitInfo::new();
        limits.limit_kill_on_job_close();
        match win32job::Job::create_with_limit_info(&limits).and_then(|made| {
            made.assign_process(child.as_raw_handle() as isize)
                .map(|()| made)
        }) {
            Ok(made) => Some(made),
            Err(_) => {
                let _ = child.kill();
                let _ = child.wait();
                return Outcome::not_started();
            }
        }
    };
    let mut held = Held {
        child,
        #[cfg(windows)]
        job: job_object,
    };
    let over = AtomicBool::new(false);
    let max = job.max_output;
    let mut output = Vec::new();
    let mut exit = None;
    let mut timed_out = false;
    std::thread::scope(|scope| {
        // The payload is written on a thread of its own: a process that never reads it must not hold the run.
        scope.spawn(move || {
            let _ = stdin.write_all(job.input);
        });
        let reading = scope.spawn(|| {
            let mut kept = Vec::new();
            let mut buffer = [0_u8; 8192];
            while let Ok(read) = stdout.read(&mut buffer) {
                if read == 0 {
                    break;
                }
                let room = (max + 1).saturating_sub(kept.len());
                kept.extend_from_slice(&buffer[..read.min(room)]);
                if kept.len() > max {
                    over.store(true, Ordering::SeqCst);
                    break;
                }
            }
            kept
        });
        // Standard error is for the user's log and never reaches the agent: it is read so the process does not wait on it.
        scope.spawn(|| {
            let mut buffer = [0_u8; 8192];
            while stderr.read(&mut buffer).is_ok_and(|read| read > 0) {}
        });
        let until = started + job.timeout;
        loop {
            if let Some(code) = held.finished() {
                exit = Some(code);
                break;
            }
            if over.load(Ordering::SeqCst) {
                held.end();
                break;
            }
            if Instant::now() >= until {
                timed_out = true;
                held.end();
                break;
            }
            std::thread::sleep(POLL);
        }
        // Every process of the run is ended, so each pipe closes and the readers return.
        output = reading.join().unwrap_or_default();
    });
    Outcome {
        started: true,
        stdout: output,
        exit: exit.unwrap_or(-1),
        timed_out,
        seconds: started.elapsed().as_secs(),
    }
}
