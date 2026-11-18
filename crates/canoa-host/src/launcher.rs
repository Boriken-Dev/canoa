//! `canoa office-launcher`: a small program that stays resident and holds the add-in's address while no host runs, so that a pane
//! opened with Canoa stopped finds a page that says so and offers to start it, in place of the client's error page.
//!
//! It is a separate program, not part of the host, because the add-in's address is fixed in the manifest and a task pane that
//! cannot load its first page has no script of its own to start anything with; a `canoa:` link handler would be reached from a pane
//! that is already open, and not from one that failed to open. It never holds the address while a host does: a host takes the home
//! before it binds, the launcher looks at the home every poll and lets go of the address within one, and a host that starts in that
//! moment waits for the address a few seconds ([`crate::web::WebServer::bind`]).
//!
//! The page asks `POST /office/launch`, the launcher lets go of the address and starts the host the home's `native install` recorded
//! (the program and arguments the browser extension's relay starts as well), detached from the launcher.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and channels are std's"
)]

use core::time::Duration;
use std::fs::OpenOptions;
use std::path::Path;
use std::process::{Command, Stdio};
use std::sync::mpsc;
use std::thread;
use std::time::Instant;

use crate::config::WebConfig;
use crate::home::{Home, LockError};
use crate::native::Launch;
use crate::web::{Launcher, Secret, WebError, WebServer};

pub(crate) mod login;

/// How often the launcher looks at whether a host holds the home, and at whether the page asked.
const POLL: Duration = Duration::from_millis(500);
/// How many looks in a row find no host before the launcher takes the address, so that a host restarting does not meet it.
const TAKE_AFTER: u32 = 4;
/// How long a host started on request is waited for before the launcher offers again.
const START_WAIT: Duration = Duration::from_secs(60);

/// What the launcher does and how fast, apart from the host it watches and starts.
pub(crate) struct Pace {
    pub(crate) poll: Duration,
    pub(crate) take_after: u32,
    pub(crate) start_wait: Duration,
}

impl Default for Pace {
    fn default() -> Self {
        Self {
            poll: POLL,
            take_after: TAKE_AFTER,
            start_wait: START_WAIT,
        }
    }
}

/// Whether a host holds `home` now.
pub(crate) fn host_runs(home: &Home) -> bool {
    matches!(home.probe(), Err(LockError::Held { .. }))
}

/// Holds the add-in's address for as long as no host holds the home, and starts one when the page asks. Returns only when
/// `stopped` says so or the address cannot be used at all.
///
/// # Errors
///
/// Why the address cannot be used, when the reason is not that another program has it.
pub(crate) fn run(
    web: &WebConfig,
    secret_dir: &std::path::Path,
    runs: &dyn Fn() -> bool,
    start: &dyn Fn() -> Result<u32, String>,
    say: &dyn Fn(&str),
    stopped: &dyn Fn() -> bool,
    pace: &Pace,
) -> Result<(), String> {
    let mut quiet = 0_u32;
    while !stopped() {
        if runs() {
            quiet = 0;
            thread::sleep(pace.poll);
            continue;
        }
        quiet += 1;
        if quiet < pace.take_after {
            thread::sleep(pace.poll);
            continue;
        }
        let secret = if web.office_secret {
            Some(Secret::load(secret_dir).map_err(|error| error.to_string())?)
        } else {
            None
        };
        let (asked, waiting) = mpsc::channel();
        let server = match WebServer::bind_launcher(web, secret, Launcher::new(asked)) {
            Ok(server) => server,
            // Another program has the address, which is not the launcher's to take back.
            Err(WebError::Listen { .. }) => {
                quiet = 0;
                thread::sleep(pace.poll);
                continue;
            }
            Err(error) => return Err(error.to_string()),
        };
        say(&format!(
            "no Canoa runs: offering to start it at {}",
            server.url()
        ));
        let stopper = server.stopper();
        let serving = thread::spawn(move || server.run());
        let mut asked_to_start = false;
        loop {
            match waiting.recv_timeout(pace.poll) {
                Ok(()) => {
                    asked_to_start = true;
                    break;
                }
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if runs() || stopped() {
                        break;
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => break,
            }
        }
        stopper.stop();
        let _ = serving.join();
        quiet = 0;
        if asked_to_start {
            match start() {
                Ok(id) => say(&format!("started Canoa (process {id})")),
                Err(why) => say(&why),
            }
            let until = Instant::now() + pace.start_wait;
            while !runs() && Instant::now() < until && !stopped() {
                thread::sleep(pace.poll);
            }
        }
    }
    Ok(())
}

/// Starts `command` (the program, then its arguments) again with no console and no input, its output appended to `log`, and
/// returns its process id: the launcher that `--detach` leaves running, while the console program that started it ends. A
/// console window of a program started at sign-in stays for as long as it runs, so the one that stays is started without one.
pub(crate) fn start_hidden(command: &[String], log: &Path) -> Result<u32, String> {
    let (program, args) = command.split_first().ok_or("no program to start")?;
    if let Some(dir) = log.parent() {
        std::fs::create_dir_all(dir)
            .map_err(|error| format!("{} could not be made: {error}", dir.display()))?;
    }
    let file = OpenOptions::new()
        .create(true)
        .append(true)
        .open(log)
        .map_err(|error| format!("{} could not be opened: {error}", log.display()))?;
    let errors = file
        .try_clone()
        .map_err(|error| format!("{} could not be shared: {error}", log.display()))?;
    let mut child = Command::new(program);
    child
        .args(args)
        .stdin(Stdio::null())
        .stdout(file)
        .stderr(errors);
    leave_console(&mut child);
    child
        .spawn()
        .map(|child| child.id())
        .map_err(|error| format!("the launcher could not be started: {error}"))
}

/// No console at all (`DETACHED_PROCESS`; `CREATE_NO_WINDOW` is ignored beside it), and a process group of its own, so that
/// what ends the starter's console does not reach it.
#[cfg(windows)]
fn leave_console(command: &mut Command) {
    use std::os::windows::process::CommandExt as _;
    const DETACHED_PROCESS: u32 = 0x0000_0008;
    const CREATE_NEW_PROCESS_GROUP: u32 = 0x0000_0200;
    command.creation_flags(DETACHED_PROCESS | CREATE_NEW_PROCESS_GROUP);
}

#[cfg(unix)]
fn leave_console(command: &mut Command) {
    use std::os::unix::process::CommandExt as _;
    command.process_group(0);
}

/// The program `home` records for starting Canoa, else `program` with `serve`.
pub(crate) fn launch_of(
    home: &Home,
    program: &crate::cli::Program,
    config: Option<&str>,
) -> Launch {
    if let Ok(recorded) = Launch::read(home) {
        return recorded;
    }
    let mut args: Vec<String> = program
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect();
    args.push("serve".to_owned());
    if let Some(config) = config {
        args.push("--config".to_owned());
        args.push(config.to_owned());
    }
    Launch {
        program: program.path.clone(),
        args,
    }
}

#[cfg(test)]
mod tests;
