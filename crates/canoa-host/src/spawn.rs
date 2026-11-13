//! Starting processes. Every process this host starts for itself (an agent, a provider, a tool, `git`) is started through
//! [`Command`], which does what std's does, with one addition for Unix: an application that runs the host inside its own process
//! can [`install`] a process of the host's own that starts them instead.
//!
//! Why: a program such as a Flutter application collects every child of its process that ends, so that the host's own wait on
//! one fails with "no child processes" and its status is lost, and a signal sent to a process id already collected may reach a
//! stranger. The starter is a process whose children are nobody else's. It keeps each child uncollected until the host collects
//! it, and ends a process group only while its leader is uncollected, so the group's id is still the child's.
//!
//! On Windows nothing is installed and every process is started as std starts it, as before.

use alloc::vec::Vec;
use std::ffi::{OsStr, OsString};
use std::io::{self, Read as _};
use std::path::{Path, PathBuf};
use std::process::{ChildStderr, ChildStdin, ChildStdout, ExitStatus, Output};
use std::thread;

#[cfg(unix)]
use coqui_acp_bridge::agent::Process;
use coqui_acp_bridge::agent::{Local, Spawned, Spawner};

use crate::cli::Program;

#[cfg(unix)]
mod apart;
#[cfg(unix)]
mod harness;

#[cfg(all(unix, test))]
pub(crate) use apart::SUBCOMMAND as SPAWNER_COMMAND;
#[cfg(unix)]
pub use apart::serve;

/// Where one of a process's standard streams goes.
#[derive(Debug, Default)]
pub enum Stdio {
    /// This process's own.
    #[default]
    Inherit,
    /// Nowhere: input that is at its end, output that is dropped.
    Null,
    /// A pipe to this process: [`Child::stdin`], [`Child::stdout`] or [`Child::stderr`] holds its end.
    Piped,
    /// The write end of a pipe the caller holds the other end of, for output: what a standard output and a standard error that
    /// are to be one stream are both given.
    Pipe(std::io::PipeWriter),
}

/// A process to start: std's `Command` with what this host uses of it. Nothing is inherited that a method has not asked for
/// beyond this process's environment and directory.
pub struct Command {
    program: OsString,
    args: Vec<OsString>,
    /// In the order given: a value set, or `None` for a variable removed.
    env: Vec<(OsString, Option<OsString>)>,
    /// Whether it starts with no environment but what `env` sets.
    clear_env: bool,
    dir: Option<PathBuf>,
    stdin: Stdio,
    stdout: Stdio,
    stderr: Stdio,
    own_group: bool,
    #[cfg(windows)]
    flags: u32,
}

impl Command {
    /// `program`, found as the operating system finds a command.
    pub fn new(program: impl AsRef<OsStr>) -> Self {
        Self {
            program: program.as_ref().to_owned(),
            args: Vec::new(),
            env: Vec::new(),
            clear_env: false,
            dir: None,
            stdin: Stdio::Inherit,
            stdout: Stdio::Inherit,
            stderr: Stdio::Inherit,
            own_group: false,
            #[cfg(windows)]
            flags: 0,
        }
    }

    /// Adds an argument.
    pub fn arg(&mut self, arg: impl AsRef<OsStr>) -> &mut Self {
        self.args.push(arg.as_ref().to_owned());
        self
    }

    /// Adds arguments.
    pub fn args<I, S>(&mut self, args: I) -> &mut Self
    where
        I: IntoIterator<Item = S>,
        S: AsRef<OsStr>,
    {
        self.args
            .extend(args.into_iter().map(|arg| arg.as_ref().to_owned()));
        self
    }

    /// Sets a variable of its environment.
    pub fn env(&mut self, key: impl AsRef<OsStr>, value: impl AsRef<OsStr>) -> &mut Self {
        self.env
            .push((key.as_ref().to_owned(), Some(value.as_ref().to_owned())));
        self
    }

    /// Sets variables of its environment.
    pub fn envs<I, K, V>(&mut self, vars: I) -> &mut Self
    where
        I: IntoIterator<Item = (K, V)>,
        K: AsRef<OsStr>,
        V: AsRef<OsStr>,
    {
        for (key, value) in vars {
            self.env(key, value);
        }
        self
    }

    /// Starts it with no environment but what [`Command::env`] and [`Command::envs`] set.
    pub fn env_clear(&mut self) -> &mut Self {
        self.clear_env = true;
        self.env.clear();
        self
    }

    /// Removes a variable from its environment.
    pub fn env_remove(&mut self, key: impl AsRef<OsStr>) -> &mut Self {
        self.env.push((key.as_ref().to_owned(), None));
        self
    }

    /// Sets its working directory; this process's when not set.
    pub fn current_dir(&mut self, dir: impl AsRef<Path>) -> &mut Self {
        self.dir = Some(dir.as_ref().to_owned());
        self
    }

    /// Where its standard input comes from; this process's by default.
    pub fn stdin(&mut self, how: Stdio) -> &mut Self {
        self.stdin = how;
        self
    }

    /// Where its standard output goes; this process's by default.
    pub fn stdout(&mut self, how: Stdio) -> &mut Self {
        self.stdout = how;
        self
    }

    /// Where its standard error goes; this process's by default.
    pub fn stderr(&mut self, how: Stdio) -> &mut Self {
        self.stderr = how;
        self
    }

    /// On Unix, starts it as the leader of a process group of its own, so that [`Child::end_group`] reaches what it starts.
    /// Nothing on Windows, where a job object does that.
    pub fn own_group(&mut self) -> &mut Self {
        self.own_group = true;
        self
    }

    /// The process creation flags of a Windows process.
    #[cfg(windows)]
    pub fn creation_flags(&mut self, flags: u32) -> &mut Self {
        self.flags = flags;
        self
    }

    /// Starts the process.
    ///
    /// # Errors
    ///
    /// The operating system could not start it, or the host's own process starter could not.
    pub fn spawn(&mut self) -> io::Result<Child> {
        #[cfg(unix)]
        if let Some(link) = apart::link()? {
            return link.spawn(self);
        }
        self.spawn_own()
    }

    /// Runs the process to its end and collects its output. Standard output and error are piped unless set otherwise, and
    /// standard input is [`Stdio::Null`] unless set otherwise, as std's.
    ///
    /// # Errors
    ///
    /// As [`Command::spawn`], or a stream could not be read.
    pub fn output(&mut self) -> io::Result<Output> {
        if matches!(self.stdin, Stdio::Inherit) {
            self.stdin = Stdio::Null;
        }
        if matches!(self.stdout, Stdio::Inherit) {
            self.stdout = Stdio::Piped;
        }
        if matches!(self.stderr, Stdio::Inherit) {
            self.stderr = Stdio::Piped;
        }
        let mut child = self.spawn()?;
        drop(child.stdin.take());
        let err = child.stderr.take().map(|mut err| {
            thread::spawn(move || {
                let mut bytes = Vec::new();
                let _ = err.read_to_end(&mut bytes);
                bytes
            })
        });
        let mut stdout = Vec::new();
        if let Some(mut out) = child.stdout.take() {
            out.read_to_end(&mut stdout)?;
        }
        let stderr = err.map_or_else(Vec::new, |reader| reader.join().unwrap_or_default());
        Ok(Output {
            status: child.wait()?,
            stdout,
            stderr,
        })
    }

    /// The process as a child of this one, started by std.
    fn spawn_own(&self) -> io::Result<Child> {
        let mut command = std::process::Command::new(&self.program);
        command.args(&self.args);
        if self.clear_env {
            command.env_clear();
        }
        for (key, value) in &self.env {
            match value {
                Some(value) => command.env(key, value),
                None => command.env_remove(key),
            };
        }
        if let Some(dir) = &self.dir {
            command.current_dir(dir);
        }
        command
            .stdin(own(&self.stdin)?)
            .stdout(own(&self.stdout)?)
            .stderr(own(&self.stderr)?);
        #[cfg(unix)]
        if self.own_group {
            std::os::unix::process::CommandExt::process_group(&mut command, 0);
        }
        #[cfg(windows)]
        std::os::windows::process::CommandExt::creation_flags(&mut command, self.flags);
        let mut child = command.spawn()?;
        Ok(Child {
            stdin: child.stdin.take(),
            stdout: child.stdout.take(),
            stderr: child.stderr.take(),
            inner: Inner::Own {
                child,
                collected: false,
            },
        })
    }

    /// This process's environment with this command's changes made, the way a child of it would see it.
    #[cfg(unix)]
    fn environment(&self) -> Vec<(OsString, OsString)> {
        let mut vars: Vec<(OsString, OsString)> = if self.clear_env {
            Vec::new()
        } else {
            std::env::vars_os().collect()
        };
        for (key, value) in &self.env {
            vars.retain(|(name, _)| name != key);
            if let Some(value) = value {
                vars.push((key.clone(), value.clone()));
            }
        }
        vars
    }
}

/// What the command is, without its environment: a variable may hold a secret.
impl core::fmt::Debug for Command {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Command")
            .field("program", &self.program)
            .field("args", &self.args)
            .field("dir", &self.dir)
            .field("stdin", &self.stdin)
            .field("stdout", &self.stdout)
            .field("stderr", &self.stderr)
            .field("own_group", &self.own_group)
            .finish_non_exhaustive()
    }
}

fn own(how: &Stdio) -> io::Result<std::process::Stdio> {
    Ok(match how {
        Stdio::Inherit => std::process::Stdio::inherit(),
        Stdio::Null => std::process::Stdio::null(),
        Stdio::Piped => std::process::Stdio::piped(),
        Stdio::Pipe(writer) => writer.try_clone()?.into(),
    })
}

/// A process started by [`Command::spawn`], and the ends of the pipes asked for.
#[derive(Debug)]
pub struct Child {
    /// Its standard input, when it was [`Stdio::Piped`] and has not been taken.
    pub stdin: Option<ChildStdin>,
    /// Its standard output, when it was [`Stdio::Piped`] and has not been taken.
    pub stdout: Option<ChildStdout>,
    /// Its standard error, when it was [`Stdio::Piped`] and has not been taken.
    pub stderr: Option<ChildStderr>,
    inner: Inner,
}

#[derive(Debug)]
enum Inner {
    /// A child of this process.
    Own {
        child: std::process::Child,
        #[cfg_attr(
            windows,
            allow(dead_code, reason = "only a Unix process group is ended")
        )]
        collected: bool,
    },
    /// A child of the host's own process starter.
    #[cfg(unix)]
    Apart(apart::Handle),
}

impl Child {
    /// Its process id.
    #[must_use]
    pub fn id(&self) -> u32 {
        match &self.inner {
            Inner::Own { child, .. } => child.id(),
            #[cfg(unix)]
            Inner::Apart(handle) => handle.id(),
        }
    }

    /// Whether it has ended and how, collecting it if so.
    ///
    /// # Errors
    ///
    /// The operating system, or the host's own process starter, could not say.
    pub fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.inner {
            Inner::Own { child, collected } => {
                let status = child.try_wait();
                // A status that cannot be read is taken as collected by someone else: its id may be anyone's.
                *collected |= !matches!(status, Ok(None));
                status
            }
            #[cfg(unix)]
            Inner::Apart(handle) => handle.try_wait(),
        }
    }

    /// Whether it has ended and how, leaving a child of the host's own process starter uncollected (a child of this process
    /// is collected, as by [`Child::try_wait`]), so that its id and its group's stay its own.
    ///
    /// # Errors
    ///
    /// As [`Child::try_wait`].
    pub fn peek(&mut self) -> io::Result<Option<ExitStatus>> {
        match &mut self.inner {
            #[cfg(unix)]
            Inner::Apart(handle) => handle.peek(),
            Inner::Own { child, collected } => peek_own(child, collected),
        }
    }

    /// Ends the process group of a process that has just ended, whether or not it has been collected: a group's id is not
    /// reused while a process is in it. Only for the moment after [`Child::peek`] or [`Child::try_wait`] has said it ended.
    /// Nothing on Windows.
    pub fn end_group_after_exit(&mut self) {
        match &mut self.inner {
            #[cfg(unix)]
            Inner::Own { child, .. } => {
                if let Some(group) = i32::try_from(child.id())
                    .ok()
                    .filter(|pid| *pid > 1)
                    .and_then(rustix::process::Pid::from_raw)
                {
                    let _ =
                        rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
                }
            }
            #[cfg(unix)]
            Inner::Apart(handle) => handle.end_group(),
            #[cfg(windows)]
            Inner::Own { .. } => {}
        }
    }

    /// Waits for it to end and collects it.
    ///
    /// # Errors
    ///
    /// As [`Child::try_wait`].
    pub fn wait(&mut self) -> io::Result<ExitStatus> {
        match &mut self.inner {
            Inner::Own { child, collected } => {
                let status = child.wait()?;
                *collected = true;
                Ok(status)
            }
            #[cfg(unix)]
            Inner::Apart(handle) => handle.wait(),
        }
    }

    /// Ends the process itself, with no signal it can catch. A process already ended is no error.
    ///
    /// # Errors
    ///
    /// The operating system, or the host's own process starter, could not.
    pub fn kill(&mut self) -> io::Result<()> {
        match &mut self.inner {
            Inner::Own { child, .. } => child.kill(),
            #[cfg(unix)]
            Inner::Apart(handle) => handle.kill(),
        }
    }

    /// Ends the process group a process started by [`Command::own_group`] leads, with no signal it can catch, unless the
    /// process has been collected: its group's id may be anyone's then. Nothing on Windows.
    pub fn end_group(&mut self) {
        match &mut self.inner {
            #[cfg(unix)]
            Inner::Own { child, collected } if !*collected => {
                if let Some(group) = i32::try_from(child.id())
                    .ok()
                    .filter(|pid| *pid > 1)
                    .and_then(rustix::process::Pid::from_raw)
                {
                    let _ =
                        rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
                }
            }
            #[cfg(unix)]
            Inner::Apart(handle) => handle.end_group(),
            Inner::Own { .. } => {}
        }
    }
}

#[cfg(windows)]
impl std::os::windows::io::AsRawHandle for Child {
    fn as_raw_handle(&self) -> std::os::windows::io::RawHandle {
        let Inner::Own { child, .. } = &self.inner;
        child.as_raw_handle()
    }
}

/// Whether a child of this process has ended, leaving it uncollected where the system can say so (Linux): until it is collected
/// its id, and its group's, cannot be anyone else's. Elsewhere it is collected, as by `try_wait`.
fn peek_own(
    child: &mut std::process::Child,
    collected: &mut bool,
) -> io::Result<Option<ExitStatus>> {
    #[cfg(any(target_os = "linux", target_os = "android"))]
    if !*collected {
        use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
        use std::os::unix::process::ExitStatusExt as _;
        let pid = i32::try_from(child.id())
            .ok()
            .and_then(Pid::from_raw)
            .ok_or_else(|| io::Error::other("not a process id"))?;
        return match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT | WaitIdOptions::NOHANG,
        ) {
            Ok(info) => Ok(info.map(|info| ExitStatus::from_raw(raw_status(&info)))),
            Err(error) => {
                *collected = true;
                Err(error.into())
            }
        };
    }
    let status = child.try_wait();
    *collected |= !matches!(status, Ok(None));
    status
}

/// A wait status for what `waitid` reported: the exit code in the second byte, or the signal in the low seven bits.
#[cfg(unix)]
fn raw_status(info: &rustix::process::WaitIdStatus) -> i32 {
    match info.exit_status() {
        Some(code) => (code & 0xff) << 8,
        None => info.terminating_signal().unwrap_or(0) & 0x7f,
    }
}

/// Starts the agents a [`coqui_acp_bridge::agent::Agent`] runs: as children of the host's own process starter when one is
/// installed, and as this process's own otherwise.
#[derive(Clone, Copy, Debug, Default)]
pub struct Agents;

impl Spawner for Agents {
    fn spawn(&self, program: &OsStr, args: &[OsString]) -> io::Result<Spawned> {
        #[cfg(unix)]
        if let Some(link) = apart::link()? {
            let mut command = Command::new(program);
            command
                .args(args)
                .stdin(Stdio::Piped)
                .stdout(Stdio::Piped)
                .stderr(Stdio::Inherit)
                .own_group();
            let mut child = link.spawn(&command)?;
            let input = child
                .stdin
                .take()
                .ok_or_else(|| io::Error::other("no standard input"))?;
            let output = child
                .stdout
                .take()
                .ok_or_else(|| io::Error::other("no standard output"))?;
            return Ok(Spawned {
                process: Box::new(Held(child)),
                input: Box::new(input),
                output: Box::new(output),
            });
        }
        Local.spawn(program, args)
    }
}

/// An agent's process, started by the host's own process starter.
#[cfg(unix)]
#[derive(Debug)]
struct Held(Child);

#[cfg(unix)]
impl Process for Held {
    fn id(&self) -> u32 {
        self.0.id()
    }

    fn peek(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.peek()
    }

    fn end_group(&mut self) {
        self.0.end_group();
    }

    fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        self.0.try_wait()
    }

    fn kill(&mut self) -> io::Result<()> {
        self.0.kill()
    }

    fn wait(&mut self) -> io::Result<ExitStatus> {
        self.0.wait()
    }
}

/// Has every process this host starts from now on started by a process of the host's own, which `program` runs with `spawner`
/// after its arguments, so that none is a child of the application's own process. A later call names the program a starter
/// not yet started is run by, and the process is started at the first child's start, and again if it has ended. Does nothing on Windows.
pub fn install(program: Program) {
    #[cfg(unix)]
    {
        apart::install(program, true);
        harness::use_starter();
    }
    #[cfg(windows)]
    let _ = program;
}

/// Has `prepare` change the command that starts the process starter, before it runs. For an application that has to do to it what
/// std cannot, such as putting a signal's disposition back to the default (an ignored one outlives `exec`). Nothing on Windows.
pub fn prepare_starter(prepare: fn(&mut std::process::Command)) {
    #[cfg(unix)]
    apart::prepare_with(prepare);
    #[cfg(windows)]
    let _ = prepare;
}

/// As [`install`], for a program the host chose for itself rather than one it was told to use: it is the starter's program
/// unless one has been named, and a later [`install`] replaces it.
pub fn install_default(program: Program) {
    #[cfg(unix)]
    {
        apart::install(program, false);
        harness::use_starter();
    }
    #[cfg(windows)]
    let _ = program;
}

/// What a failure of the starter says of the child it lost.
#[cfg(unix)]
fn lost() -> io::Error {
    io::Error::other("the host's process starter has ended")
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn a_command_shows_what_it_runs_and_never_its_environment() {
        let mut command = Command::new("tool");
        command.arg("--flag").env("API_TOKEN", "hunter2-secret");
        let shown = format!("{command:?}");
        assert!(
            shown.contains("tool") && shown.contains("--flag"),
            "{shown}"
        );
        assert!(
            !shown.contains("hunter2") && !shown.contains("API_TOKEN"),
            "{shown}"
        );
    }

    /// Every process the host starts for itself goes through [`Command`], so that none is a child of an application that
    /// collects its own children. A use of std's is one that was left behind, unless it is one of these, which are the host's
    /// own commands (they run as a process of their own, whose children are its own) or Windows-only.
    /// The coqui-acp crates this host runs in its own process start processes too, and each has a seam for a host's own starter. A
    /// use of std's `Command` in one of them is allowed where it is the fallback behind that seam, or Windows' own: a new one is
    /// a process left to start as a child of the application.
    #[test]
    #[cfg(unix)]
    fn the_crates_run_in_process_start_no_process_by_std_but_behind_their_seam() {
        // (file, what shows it is behind the seam or Windows-only)
        const BEHIND: [(&str, &str); 6] = [
            ("coqui-acp-secrets/src/run.rs", "STARTER"),
            ("coqui-acp-harness/src/process.rs", "STARTER"),
            ("coqui-acp-bridge/src/agent.rs", "Spawner"),
            ("coqui-acp-bridge/src/standing.rs", "icacls"),
            ("coqui-acp-pi/src/gate.rs", "icacls"),
            ("coqui-acp-harness/src/antigravity/ports.rs", "netstat"),
        ];
        const CRATES: [&str; 9] = [
            "coqui-acp",
            "coqui-acp-secrets",
            "coqui-acp-harness",
            "coqui-acp-bridge",
            "coqui-acp-claude",
            "coqui-acp-codex",
            "coqui-acp-pi",
            "coqui-acp-provider",
            "coqui-acp-history",
        ];
        fn scan(
            dir: &Path,
            root: &Path,
            name: &str,
            found: &mut Vec<String>,
            seen: &mut Vec<(String, String)>,
        ) {
            for entry in std::fs::read_dir(dir).expect("a directory") {
                let path = entry.expect("an entry").path();
                if path.is_dir() {
                    scan(&path, root, name, found, seen);
                    continue;
                }
                if path.extension().is_none_or(|ext| ext != "rs") {
                    continue;
                }
                let relative = path
                    .strip_prefix(root)
                    .expect("below the root")
                    .to_string_lossy()
                    .replace('\\', "/");
                let text = std::fs::read_to_string(&path).expect("a source file");
                let uses_std = text.lines().any(|line| {
                    !line.trim_start().starts_with("//")
                        && (line.contains("std::process::Command")
                            || (line.contains("use std::process::") && line.contains("Command")))
                });
                if uses_std {
                    seen.push((format!("{name}/{relative}"), text));
                    found.push(format!("{name}/{relative}"));
                }
            }
        }
        // Only what this system builds is on disk, so only it is asked for.
        let rustc = std::process::Command::new("rustc")
            .arg("-vV")
            .output()
            .expect("rustc runs");
        let host = String::from_utf8_lossy(&rustc.stdout)
            .lines()
            .find_map(|line| line.strip_prefix("host: ").map(str::to_owned))
            .expect("rustc names its host");
        let cargo = std::env::var_os("CARGO").unwrap_or_else(|| "cargo".into());
        let listed = std::process::Command::new(cargo)
            .args([
                "metadata",
                "--format-version",
                "1",
                "--offline",
                "--filter-platform",
            ])
            .arg(host)
            .current_dir(env!("CARGO_MANIFEST_DIR"))
            .output()
            .expect("cargo runs");
        assert!(
            listed.status.success(),
            "{}",
            String::from_utf8_lossy(&listed.stderr)
        );
        let metadata: serde_json::Value = serde_json::from_slice(&listed.stdout).expect("JSON");
        let mut found = Vec::new();
        let mut seen = Vec::new();
        for name in CRATES {
            let manifest = metadata["packages"]
                .as_array()
                .expect("packages")
                .iter()
                .find(|package| package["name"] == name)
                .and_then(|package| package["manifest_path"].as_str())
                .unwrap_or_else(|| panic!("{name} is in the dependency graph"));
            let src = Path::new(manifest)
                .parent()
                .expect("a crate directory")
                .join("src");
            scan(&src, &src, name, &mut found, &mut seen);
        }
        for (file, text) in &seen {
            let Some((_, shows)) = BEHIND.iter().find(|(allowed, _)| allowed == file) else {
                continue;
            };
            assert!(
                text.contains(shows),
                "{file} uses std's Command, and nothing in it is the seam ({shows})"
            );
        }
        let stray: Vec<&String> = found
            .iter()
            .filter(|file| !BEHIND.iter().any(|(allowed, _)| allowed == file))
            .collect();
        assert!(
            stray.is_empty(),
            "std starts a process in a crate this host runs in process: {stray:#?}"
        );
    }

    #[test]
    fn no_process_is_started_by_std_outside_this_module() {
        const OWN: [&str; 2] = ["profile/launch.rs", "accounts/private.rs"];
        fn scan(dir: &Path, root: &Path, found: &mut Vec<String>) {
            for entry in std::fs::read_dir(dir).expect("a directory") {
                let path = entry.expect("an entry").path();
                let relative = path
                    .strip_prefix(root)
                    .expect("below the root")
                    .to_string_lossy()
                    .replace('\\', "/");
                if path.is_dir() {
                    if relative != "spawn" {
                        scan(&path, root, found);
                    }
                    continue;
                }
                let own = relative == "spawn.rs"
                    || relative.ends_with("tests.rs")
                    || relative.contains("/tests/")
                    || OWN.contains(&relative.as_str());
                if own || !relative.ends_with(".rs") {
                    continue;
                }
                let text = std::fs::read_to_string(&path).expect("a source file");
                for (number, line) in text.lines().enumerate() {
                    let uses_std = line.contains("std::process::Command")
                        || (line.contains("use std::process::") && line.contains("Command"));
                    if uses_std && !line.trim_start().starts_with("//") {
                        found.push(format!("{relative}:{}: {}", number + 1, line.trim()));
                    }
                }
            }
        }
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("src");
        let mut found = Vec::new();
        scan(&root, &root, &mut found);
        assert!(found.is_empty(), "std starts a process here: {found:#?}");
    }
}
