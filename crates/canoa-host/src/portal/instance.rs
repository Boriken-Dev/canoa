//! One host per user: started when a user signs in and uses it, stopped when nobody has for a while.
//!
//! Each user's host has a state of its own, and no lock is held across a call to the system or a socket for longer than that
//! user's own start or stop: one user's slow host never delays another's.

use alloc::sync::Arc;
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use coqui_transport_pipe::{PipeName, ReadTimeout, Stream, WriteTimeout};

use super::signin::valid_user;
use crate::portal_link::{INTRODUCTION, entry_socket};
use crate::spawn::{Command, Stdio};

/// The longest answer the introduction socket is read for.
const MAX_ANSWER: usize = 64 * 1024;
/// How long a started host is asked again for its socket.
const POLL: Duration = Duration::from_millis(100);
/// How long one request to the introduction socket may take in all, however its answer arrives.
const CALL_TIME: Duration = Duration::from_secs(5);

/// Starts and stops one user's host. The portal calls it with names `valid_user` took, and a controller refuses any other.
pub trait Controller: Send + Sync {
    /// Starts the host of `user`; ends well when it already runs.
    ///
    /// # Errors
    ///
    /// When the user's host could not be started.
    fn start(&self, user: &str) -> io::Result<()>;
    /// Stops the host of `user`.
    ///
    /// # Errors
    ///
    /// When it could not be stopped.
    fn stop(&self, user: &str) -> io::Result<()>;
    /// The users whose host runs now.
    ///
    /// # Errors
    ///
    /// When the system could not say.
    fn running(&self) -> io::Result<Vec<String>>;
}

/// systemd's template unit, `canoa@USER.service`, started and stopped with `systemctl`. The portal runs as a service user with
/// no rights of its own, so `start` and `stop` are what a polkit rule or a root helper allows it for those units and nothing
/// more (`elevate` puts a command, such as `sudo -n`, before `systemctl`; `systemctl` may then be the helper, which takes
/// the same words).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Systemd {
    /// The `systemctl` program.
    pub systemctl: PathBuf,
    /// A command put before `systemctl` for `start` and `stop`; listing runs without it, and without `systemctl` being the
    /// helper, which takes only those two words.
    pub elevate: Vec<OsString>,
    /// The unit's name, `{user}` standing for the account.
    pub unit: String,
    /// The program that lists the running units, which needs no privilege.
    pub list: PathBuf,
    /// How long a call may take before it is ended.
    pub timeout: Duration,
}

impl Systemd {
    /// `systemctl` and the unit `canoa@{user}.service`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            systemctl: PathBuf::from("systemctl"),
            elevate: Vec::new(),
            unit: "canoa@{user}.service".to_owned(),
            list: PathBuf::from("systemctl"),
            timeout: Duration::from_secs(60),
        }
    }

    /// Runs `command` for at most `timeout`: its output, or an error after ending it.
    fn output(&self, command: &mut Command) -> io::Result<(bool, Vec<u8>, Vec<u8>)> {
        let mut child = command.spawn()?;
        let until = Instant::now() + self.timeout;
        let status = loop {
            if let Some(status) = child.try_wait()? {
                break status;
            }
            if Instant::now() >= until {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "systemctl took too long",
                ));
            }
            thread::sleep(Duration::from_millis(25));
        };
        let (mut out, mut err) = (Vec::new(), Vec::new());
        if let Some(mut stream) = child.stdout.take() {
            let _ = stream.read_to_end(&mut out);
        }
        if let Some(mut stream) = child.stderr.take() {
            let _ = stream.read_to_end(&mut err);
        }
        Ok((status.success(), out, err))
    }

    fn unit_of(&self, user: &str) -> io::Result<String> {
        if !valid_user(user) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a name a unit is made of",
            ));
        }
        Ok(self.unit.replace("{user}", user))
    }

    fn run(&self, verb: &str, unit: &str) -> io::Result<()> {
        let mut words = self.elevate.clone();
        words.push(self.systemctl.clone().into_os_string());
        let mut command = Command::new(&words[0]);
        command
            .args(&words[1..])
            .args([verb, "--no-ask-password", unit])
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::Null)
            .stdout(Stdio::Null)
            .stderr(Stdio::Piped);
        let (ok, _, stderr) = self.output(&mut command)?;
        if ok {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "systemctl {verb} {unit}: {}",
            String::from_utf8_lossy(&stderr).trim()
        )))
    }
}

impl Default for Systemd {
    fn default() -> Self {
        Self::new()
    }
}

impl Controller for Systemd {
    fn start(&self, user: &str) -> io::Result<()> {
        self.run("start", &self.unit_of(user)?)
    }

    fn stop(&self, user: &str) -> io::Result<()> {
        self.run("stop", &self.unit_of(user)?)
    }

    fn running(&self) -> io::Result<Vec<String>> {
        let pattern = self.unit.replace("{user}", "*");
        let mut command = Command::new(&self.list);
        command
            .args(["list-units", "--plain", "--no-legend", "--state=active"])
            .arg(&pattern)
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::Null)
            .stdout(Stdio::Piped)
            .stderr(Stdio::Null);
        let (_, stdout, _) = self.output(&mut command)?;
        let (before, after) = self.unit.split_once("{user}").unwrap_or((&self.unit, ""));
        Ok(String::from_utf8_lossy(&stdout)
            .lines()
            .filter_map(|line| {
                let unit = line.split_whitespace().next()?;
                let user = unit.strip_prefix(before)?.strip_suffix(after)?;
                valid_user(user).then(|| user.to_owned())
            })
            .collect())
    }
}

/// Connects to `path`, a socket the portal dials for a person whose user id is `uid`, and says it only when the system says the
/// process that made the socket runs as that user. The person's own folder holds the path and so can make a link in it to
/// another person's socket; whoever is there is that other person's host, and is turned away here.
///
/// # Errors
///
/// When the socket cannot be reached, or was made by another user.
pub fn dial(path: &Path, uid: u32) -> io::Result<Stream> {
    let name = PipeName::at(path).map_err(|error| io::Error::other(error.to_string()))?;
    let stream = Stream::connect(&name).map_err(|error| io::Error::other(error.to_string()))?;
    let made_by = stream
        .peer()?
        .user
        .and_then(|user| user.parse::<u32>().ok());
    if made_by != Some(uid) {
        return Err(io::Error::new(
            io::ErrorKind::PermissionDenied,
            "the socket was not made by the person it was dialed for",
        ));
    }
    Ok(stream)
}

/// What the portal knows of one user's host.
#[derive(Default)]
struct Slot {
    relays: AtomicUsize,
    /// Whether the host is up, as far as the portal knows: counted against `max_hosts`.
    up: AtomicBool,
    last: Mutex<Option<Instant>>,
    /// Held while this user's host is started or stopped, and nowhere else.
    op: Mutex<()>,
}

/// What the portal knows of each user's host: who is using it, and since when nobody has.
pub struct Instances {
    controller: Arc<dyn Controller>,
    dir: PathBuf,
    idle: Duration,
    start_wait: Duration,
    max_relays: usize,
    max_hosts: usize,
    /// Hosts up or being started: one count, so that starts by different people cannot together pass `max_hosts`.
    hosts: AtomicUsize,
    slots: Mutex<HashMap<String, Arc<Slot>>>,
}

impl core::fmt::Debug for Instances {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Instances").finish_non_exhaustive()
    }
}

/// A user's host in use: the instance is not stopped for being idle while one lives.
#[derive(Debug)]
pub struct Lease {
    slot: Arc<Slot>,
}

impl core::fmt::Debug for Slot {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Slot").finish_non_exhaustive()
    }
}

impl Drop for Lease {
    fn drop(&mut self) {
        self.slot.relays.fetch_sub(1, Ordering::SeqCst);
        *self
            .slot
            .last
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }
}

impl Instances {
    /// Hosts started by `controller` whose sockets are under `dir/USER`, stopped after `idle` with nobody using them, given
    /// `start_wait` to start, at most `max_relays` open for a user and `max_hosts` running.
    #[must_use]
    pub fn new(
        controller: Arc<dyn Controller>,
        dir: impl Into<PathBuf>,
        idle: Duration,
        start_wait: Duration,
        max_relays: usize,
        max_hosts: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            controller,
            dir: dir.into(),
            idle,
            start_wait,
            max_relays,
            max_hosts,
            hosts: AtomicUsize::new(0),
            slots: Mutex::new(HashMap::new()),
        })
    }

    /// The directory of `user`'s sockets.
    #[must_use]
    pub fn dir_of(&self, user: &str) -> PathBuf {
        self.dir.join(user)
    }

    /// The socket of `user`'s entry `name`.
    #[must_use]
    pub fn entry_socket(&self, user: &str, name: &str) -> PathBuf {
        entry_socket(&self.dir_of(user), name)
    }

    fn slot(&self, user: &str) -> Arc<Slot> {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        Arc::clone(slots.entry(user.to_owned()).or_default())
    }

    /// The hosts that already run are counted as used just now, so a portal that started again stops them in time.
    pub fn adopt(&self) {
        let Ok(users) = self.controller.running() else {
            return;
        };
        for user in users {
            let slot = self.slot(&user);
            self.mark_up(&slot);
            *slot.last.lock().unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
        }
    }

    /// Counts the host as up, once.
    fn mark_up(&self, slot: &Slot) {
        if !slot.up.swap(true, Ordering::SeqCst) {
            self.hosts.fetch_add(1, Ordering::SeqCst);
        }
    }

    /// Counts the host as down, once.
    fn mark_down(&self, slot: &Slot) {
        if slot.up.swap(false, Ordering::SeqCst) {
            self.hosts.fetch_sub(1, Ordering::SeqCst);
        }
    }

    /// Makes sure `user`'s host runs and answers, starting it when it does not, and counts one more use of it until the lease
    /// is dropped.
    ///
    /// # Errors
    ///
    /// When the user has too many open already, too many hosts run, or the host could not be started or did not answer in time.
    pub fn acquire(&self, user: &str, uid: u32) -> io::Result<Lease> {
        if !valid_user(user) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a user name",
            ));
        }
        let slot = self.slot(user);
        if slot.relays.fetch_add(1, Ordering::SeqCst) >= self.max_relays {
            slot.relays.fetch_sub(1, Ordering::SeqCst);
            return Err(io::Error::other("too many connections open for this user"));
        }
        let lease = Lease {
            slot: Arc::clone(&slot),
        };
        self.ready(user, uid, &slot)?;
        Ok(lease)
    }

    /// Counts a use of `user`'s host that is over in a moment, so that it is not stopped for being idle yet.
    pub fn touch(&self, user: &str) {
        *self
            .slot(user)
            .last
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(Instant::now());
    }

    fn ready(&self, user: &str, uid: u32, slot: &Slot) -> io::Result<()> {
        let index = r#"{"op":"index"}"#;
        // Only this user's own start or stop waits on this.
        let _own = slot.op.lock().unwrap_or_else(PoisonError::into_inner);
        // Taken before the first question, so a stop that is under way ends before this user is told the host is there.
        match self.ask(user, uid, index) {
            Ok(_) => {
                self.mark_up(slot);
                return Ok(());
            }
            // A socket another user made is not a host to start over.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
            Err(_) => {}
        }
        // The place is taken before the start, in the one count every person's start goes through.
        if !slot.up.load(Ordering::SeqCst) {
            if self.hosts.fetch_add(1, Ordering::SeqCst) >= self.max_hosts {
                self.hosts.fetch_sub(1, Ordering::SeqCst);
                return Err(io::Error::other("too many hosts are running"));
            }
            slot.up.store(true, Ordering::SeqCst);
        }
        if let Err(error) = self.controller.start(user) {
            self.mark_down(slot);
            return Err(error);
        }
        let until = Instant::now() + self.start_wait;
        loop {
            match self.ask(user, uid, index) {
                Ok(_) => return Ok(()),
                Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
                Err(_) => {}
            }
            if Instant::now() >= until {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the host did not answer after it was started",
                ));
            }
            thread::sleep(POLL);
        }
    }

    /// One request to `user`'s introduction socket: a line of JSON, answered with a line of JSON, all within a few seconds
    /// however the other end paces its bytes.
    ///
    /// # Errors
    ///
    /// When the socket cannot be reached, was made by another user, or does not answer with JSON in time.
    pub fn ask(&self, user: &str, uid: u32, request: &str) -> io::Result<serde_json::Value> {
        let stream = dial(&self.dir_of(user).join(INTRODUCTION), uid)?;
        let deadline = Instant::now() + CALL_TIME;
        let mut stream = &stream;
        stream.set_write_timeout(Some(CALL_TIME))?;
        stream.write_all(request.as_bytes())?;
        stream.write_all(b"\n")?;
        let mut answer = Vec::new();
        let mut chunk = [0_u8; 4096];
        loop {
            let left = deadline.saturating_duration_since(Instant::now());
            if left.is_zero() {
                return Err(io::Error::new(
                    io::ErrorKind::TimedOut,
                    "the host answered too slowly",
                ));
            }
            stream.set_read_timeout(Some(left))?;
            match stream.read(&mut chunk) {
                Ok(0) => break,
                Ok(read) => {
                    answer.extend_from_slice(&chunk[..read]);
                    if answer.len() > MAX_ANSWER {
                        return Err(io::Error::other("the host's answer is too long"));
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => {}
                Err(error) => return Err(error),
            }
        }
        serde_json::from_slice(&answer).map_err(io::Error::other)
    }

    /// Stops each host nobody has used for `idle`; returns who was stopped.
    pub fn sweep(&self) -> Vec<String> {
        let now = Instant::now();
        let candidates: Vec<(String, Arc<Slot>)> = {
            let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            slots
                .iter()
                .map(|(user, slot)| (user.clone(), Arc::clone(slot)))
                .collect()
        };
        let mut stopped = Vec::new();
        for (user, slot) in candidates {
            let idle = |slot: &Slot| {
                slot.relays.load(Ordering::SeqCst) == 0
                    && slot
                        .last
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .is_some_and(|last| now.duration_since(last) >= self.idle)
            };
            if !idle(&slot) {
                continue;
            }
            // A user whose host is being started or stopped is not idle; the system is asked with no lock but this user's.
            let Ok(_own) = slot.op.try_lock() else {
                continue;
            };
            // A relay that came while this one was deciding keeps the host.
            if !idle(&slot) {
                continue;
            }
            match self.controller.stop(&user) {
                Ok(()) => {
                    self.mark_down(&slot);
                    *slot.last.lock().unwrap_or_else(PoisonError::into_inner) = None;
                    stopped.push(user);
                }
                Err(error) => {
                    tracing::warn!("the idle host of {user} could not be stopped: {error}")
                }
            }
        }
        stopped
    }
}
