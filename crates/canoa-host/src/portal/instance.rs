//! One host per user: started when a user signs in and uses it, stopped when nobody has for a while.

use alloc::sync::Arc;
use core::time::Duration;
use std::collections::HashMap;
use std::ffi::OsString;
use std::io::{self, Read, Write};
use std::path::{Path, PathBuf};

use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use coqui_transport_pipe::{PipeName, ReadTimeout, Stream};

use super::signin::valid_user;
use crate::portal_link::{INTRODUCTION, entry_socket};
use crate::spawn::{Command, Stdio};

/// The longest answer the introduction socket is read for.
const MAX_ANSWER: u64 = 64 * 1024;
/// How long a started host is asked again for its socket.
const POLL: Duration = Duration::from_millis(100);
/// How long a request to the introduction socket may take.
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
/// no rights of its own, so `start` and `stop` are what a polkit rule allows it for `canoa@*.service` and nothing more
/// (`elevate` is the other way: a command that runs `systemctl` as root, such as `sudo -n`).
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Systemd {
    /// The `systemctl` program.
    pub systemctl: PathBuf,
    /// A command put before `systemctl` for `start` and `stop`; listing runs without it.
    pub elevate: Vec<OsString>,
    /// The unit's name, `{user}` standing for the account.
    pub unit: String,
}

impl Systemd {
    /// `systemctl` and the unit `canoa@{user}.service`.
    #[must_use]
    pub fn new() -> Self {
        Self {
            systemctl: PathBuf::from("systemctl"),
            elevate: Vec::new(),
            unit: "canoa@{user}.service".to_owned(),
        }
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

    fn run(&self, verb: &str, unit: &str, elevated: bool) -> io::Result<()> {
        let mut words = if elevated {
            self.elevate.clone()
        } else {
            Vec::new()
        };
        words.push(self.systemctl.clone().into_os_string());
        let mut command = Command::new(&words[0]);
        command
            .args(&words[1..])
            .args([verb, unit])
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::Null)
            .stdout(Stdio::Null)
            .stderr(Stdio::Piped);
        let output = command.output()?;
        if output.status.success() {
            return Ok(());
        }
        Err(io::Error::other(format!(
            "systemctl {verb} {unit}: {}",
            String::from_utf8_lossy(&output.stderr).trim()
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
        self.run("start", &self.unit_of(user)?, true)
    }

    fn stop(&self, user: &str) -> io::Result<()> {
        self.run("stop", &self.unit_of(user)?, true)
    }

    fn running(&self) -> io::Result<Vec<String>> {
        let pattern = self.unit.replace("{user}", "*");
        let output = Command::new(&self.systemctl)
            .args(["list-units", "--plain", "--no-legend", "--state=active"])
            .arg(&pattern)
            .env("PATH", "/usr/local/bin:/usr/bin:/bin:/usr/sbin:/sbin")
            .stdin(Stdio::Null)
            .stderr(Stdio::Null)
            .output()?;
        let (before, after) = self.unit.split_once("{user}").unwrap_or((&self.unit, ""));
        Ok(String::from_utf8_lossy(&output.stdout)
            .lines()
            .filter_map(|line| {
                let unit = line.split_whitespace().next()?;
                let user = unit.strip_prefix(before)?.strip_suffix(after)?;
                valid_user(user).then(|| user.to_owned())
            })
            .collect())
    }
}

#[derive(Default)]
struct Slot {
    relays: usize,
    last: Option<Instant>,
}

/// What the portal knows of each user's host: who is using it, and since when nobody has.
pub struct Instances {
    controller: Arc<dyn Controller>,
    dir: PathBuf,
    idle: Duration,
    start_wait: Duration,
    max_relays: usize,
    slots: Mutex<HashMap<String, Slot>>,
    /// Held while a user's host is started, so that two requests start it once.
    starting: Mutex<()>,
}

impl core::fmt::Debug for Instances {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Instances").finish_non_exhaustive()
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

/// A user's host in use: the instance is not stopped for being idle while one lives.
#[derive(Debug)]
pub struct Lease {
    instances: Arc<Instances>,
    user: String,
}

impl Drop for Lease {
    fn drop(&mut self) {
        let mut slots = self
            .instances
            .slots
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(slot) = slots.get_mut(&self.user) {
            slot.relays = slot.relays.saturating_sub(1);
            slot.last = Some(Instant::now());
        }
    }
}

impl Instances {
    /// Hosts started by `controller` whose sockets are under `dir/USER`, stopped after `idle` with nobody using them, given
    /// `start_wait` to start, at most `max_relays` open for a user.
    #[must_use]
    pub fn new(
        controller: Arc<dyn Controller>,
        dir: impl Into<PathBuf>,
        idle: Duration,
        start_wait: Duration,
        max_relays: usize,
    ) -> Arc<Self> {
        Arc::new(Self {
            controller,
            dir: dir.into(),
            idle,
            start_wait,
            max_relays,
            slots: Mutex::new(HashMap::new()),
            starting: Mutex::new(()),
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

    /// The hosts that already run are counted as used just now, so a portal that started again stops them in time.
    pub fn adopt(&self) {
        let Ok(users) = self.controller.running() else {
            return;
        };
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        for user in users {
            slots.entry(user).or_default().last = Some(Instant::now());
        }
    }

    /// Makes sure `user`'s host runs and answers, starting it when it does not, and counts one more use of it until the lease
    /// is dropped.
    ///
    /// # Errors
    ///
    /// When the user has too many open already, or the host could not be started or did not answer in time.
    pub fn acquire(self: &Arc<Self>, user: &str, uid: u32) -> io::Result<Lease> {
        if !valid_user(user) {
            return Err(io::Error::new(
                io::ErrorKind::InvalidInput,
                "not a user name",
            ));
        }
        {
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            let slot = slots.entry(user.to_owned()).or_default();
            if slot.relays >= self.max_relays {
                return Err(io::Error::other("too many connections open for this user"));
            }
            slot.relays += 1;
        }
        let lease = Lease {
            instances: Arc::clone(self),
            user: user.to_owned(),
        };
        self.ready(user, uid)?;
        Ok(lease)
    }

    /// Counts a use of `user`'s host that is over in a moment, so that it is not stopped for being idle yet.
    pub fn touch(&self, user: &str) {
        let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
        slots.entry(user.to_owned()).or_default().last = Some(Instant::now());
    }

    fn ready(&self, user: &str, uid: u32) -> io::Result<()> {
        let index = r#"{"op":"index"}"#;
        match self.ask(user, uid, index) {
            Ok(_) => return Ok(()),
            // A socket another user made is not a host to start over.
            Err(error) if error.kind() == io::ErrorKind::PermissionDenied => return Err(error),
            Err(_) => {}
        }
        let _starting = self.starting.lock().unwrap_or_else(PoisonError::into_inner);
        if self.ask(user, uid, index).is_ok() {
            return Ok(());
        }
        self.controller.start(user)?;
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

    /// One request to `user`'s introduction socket: a line of JSON, answered with a line of JSON.
    ///
    /// # Errors
    ///
    /// When the socket cannot be reached, was made by another user, or does not answer with JSON.
    pub fn ask(&self, user: &str, uid: u32, request: &str) -> io::Result<serde_json::Value> {
        let stream = dial(&self.dir_of(user).join(INTRODUCTION), uid)?;
        let mut stream = &stream;
        stream.set_read_timeout(Some(CALL_TIME))?;
        stream.write_all(request.as_bytes())?;
        stream.write_all(b"\n")?;
        let mut answer = Vec::new();
        stream.take(MAX_ANSWER).read_to_end(&mut answer)?;
        serde_json::from_slice(&answer).map_err(io::Error::other)
    }

    /// Stops each host nobody has used for `idle`; returns who was stopped.
    pub fn sweep(&self) -> Vec<String> {
        let now = Instant::now();
        let due: Vec<String> = {
            let slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            slots
                .iter()
                .filter(|(_, slot)| {
                    slot.relays == 0
                        && slot
                            .last
                            .is_some_and(|last| now.duration_since(last) >= self.idle)
                })
                .map(|(user, _)| user.clone())
                .collect()
        };
        let mut stopped = Vec::new();
        for user in due {
            // A relay that came while this one was deciding keeps the host.
            let mut slots = self.slots.lock().unwrap_or_else(PoisonError::into_inner);
            if slots.get(&user).is_some_and(|slot| slot.relays > 0) {
                continue;
            }
            match self.controller.stop(&user) {
                Ok(()) => {
                    slots.remove(&user);
                    stopped.push(user);
                }
                Err(error) => {
                    tracing::warn!("the idle host of {user} could not be stopped: {error}")
                }
            }
        }
        stopped
    }

    /// The directory of the sockets, for a log line.
    #[must_use]
    pub fn root(&self) -> &Path {
        &self.dir
    }
}
