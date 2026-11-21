//! The terminals the host runs for the person: their own shell on a pseudo-terminal, reached over the control channel.
//!
//! A terminal belongs to the link that opened it. Its output goes to that link as `terminal/output` notifications as the shell
//! writes it, in chunks a read returns, with the host holding nothing beyond a bounded scrollback; when the link ends, so does the
//! shell and everything it started. Who may open one is decided by the control channel's `terminal` grant and nothing here.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, locks and processes are std's"
)]

use std::collections::{BTreeMap, VecDeque};
use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU32, AtomicUsize, Ordering};
use std::sync::mpsc::{Receiver, SyncSender, TrySendError, channel, sync_channel};
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::thread::{Builder, Scope};
use std::time::Duration;

use coqui_transport::{Channel, Failure};
use data_encoding::BASE64;
use serde_json::{Value, json};

#[cfg(unix)]
mod pty_unix;
#[cfg(unix)]
use pty_unix as pty;
#[cfg(windows)]
mod pty_windows;
#[cfg(windows)]
use pty_windows as pty;

/// The most terminals one link may have open.
pub(crate) const MAX_PER_LINK: usize = 8;
/// The most terminals this host runs at once, for every link.
pub(crate) const MAX_PER_HOST: usize = 32;
/// The most a read takes from the shell at once, which is also the most one `terminal/output` carries.
const CHUNK: usize = 16 * 1024;
/// How much of a terminal's latest output the host keeps for a view that was not there when it was written.
pub(crate) const SCROLLBACK: usize = 256 * 1024;
/// The most one `terminal/write` carries, in bytes before they are encoded.
pub(crate) const MAX_WRITE: usize = 16 * 1024;
/// The writes that wait for a shell that is not reading.
const QUEUED_WRITES: usize = 32;
/// The largest terminal either way, in cells.
const MAX_CELLS: u16 = 1000;
/// How long a finished terminal's reader waits for the shell's exit code.
const EXIT_WAIT: Duration = Duration::from_secs(10);

/// What ends, waits on and resizes a shell, on this computer or in an environment.
enum Shell {
    Local(pty::Process),
    Remote(crate::environments::remote::Remote),
}

impl Shell {
    fn wait(&self) -> i32 {
        match self {
            Self::Local(process) => process.wait(),
            Self::Remote(process) => process.wait_code(),
        }
    }

    fn end(&self) {
        match self {
            Self::Local(process) => process.end(),
            Self::Remote(process) => {
                let _ = process.kill();
            }
        }
    }

    fn resize(&self, size: Size) -> std::io::Result<()> {
        match self {
            Self::Local(process) => process.resize(size),
            Self::Remote(process) => process.resize(size.rows, size.cols),
        }
    }
}

/// What the shell reads.
enum Feed {
    Local(pty::Input),
    Remote(crate::environments::remote::Input),
}

impl Feed {
    fn write_all(&mut self, bytes: &[u8]) -> std::io::Result<()> {
        use std::io::Write as _;
        match self {
            Self::Local(input) => input.write_all(bytes),
            Self::Remote(input) => input.write_all(bytes),
        }
    }
}

/// What the shell wrote.
enum Drain {
    Local(pty::Output),
    Remote(crate::environments::remote::Output),
}

impl Drain {
    fn read(&mut self, buf: &mut [u8]) -> std::io::Result<usize> {
        use std::io::Read as _;
        match self {
            Self::Local(output) => output.read(buf),
            Self::Remote(output) => output.read(buf),
        }
    }
}

/// A terminal's size in character cells.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) struct Size {
    pub(crate) cols: u16,
    pub(crate) rows: u16,
}

/// Why a request was not done, which the control channel turns into its error codes.
#[derive(Debug)]
pub(crate) enum Fail {
    /// The request is not well formed.
    Bad(String),
    /// A limit, or something the computer would not do.
    Refused(String),
    /// No terminal has that id on this link.
    NotFound(String),
}

fn bad(what: &str) -> Fail {
    Fail::Bad(what.to_owned())
}

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

/// What every link's terminals share: which shell to start, where, and how many there are.
#[derive(Debug, Default)]
pub(crate) struct Terminals {
    shell: Option<String>,
    home: Option<PathBuf>,
    live: Arc<AtomicUsize>,
    /// Where shells are looked for; this computer when `None`.
    lookup: Option<crate::shells::Lookup>,
    /// The environments a request may open a shell in.
    environments: Option<Arc<crate::environments::Environments>>,
}

impl Terminals {
    /// `shell` is `[host] shell`; `home` is where a terminal starts when its request names no folder.
    pub(crate) fn new(shell: Option<String>, home: Option<PathBuf>) -> Self {
        Self {
            shell,
            home,
            live: Arc::new(AtomicUsize::new(0)),
            lookup: None,
            environments: None,
        }
    }

    /// These terminals able to open a shell in an environment of `environments`, which a request names with `environment`.
    pub(crate) fn in_environments(
        mut self,
        environments: Arc<crate::environments::Environments>,
    ) -> Self {
        self.environments = Some(environments);
        self
    }

    /// These terminals looking for the shells an id names in `lookup`, in place of this computer.
    #[cfg(test)]
    pub(crate) fn finding(mut self, lookup: crate::shells::Lookup) -> Self {
        self.lookup = Some(lookup);
        self
    }

    /// The shell to start: the one a request names (an id `shell/list` gives), else the one canoa.cfg names (an id, or a
    /// program), else the person's own. On Windows it is an absolute path: a bare name is looked for on `PATH` and never in the
    /// folder the host happens to be in, where a cloned repository could have put one.
    fn program(&self, asked: Option<&str>) -> Result<OsString, String> {
        let by_id = |id: &str| {
            let lookup = self
                .lookup
                .clone()
                .unwrap_or_else(crate::shells::Lookup::here);
            let found = crate::shells::detect(&lookup);
            found
                .iter()
                .find(|shell| shell.id == id)
                .map(crate::shells::program)
        };
        if let Some(id) = asked {
            return by_id(id).ok_or_else(|| {
                format!("this host found no shell {id}: shell/list names the ones it has")
            });
        }
        if let Some(program) = self.shell.as_deref().and_then(by_id) {
            return Ok(program);
        }
        #[cfg(windows)]
        {
            if let Some(shell) = &self.shell {
                return resolve(std::ffi::OsStr::new(shell))
                    .map(PathBuf::into_os_string)
                    .ok_or_else(|| format!("the shell {shell} was not found on PATH"));
            }
            for name in ["pwsh.exe", "powershell.exe"] {
                if let Some(found) = resolve(std::ffi::OsStr::new(name)) {
                    return Ok(found.into_os_string());
                }
            }
            std::env::var_os("COMSPEC")
                .map(PathBuf::from)
                .filter(|comspec| comspec.is_absolute())
                .or_else(|| resolve(std::ffi::OsStr::new("cmd.exe")))
                .map(PathBuf::into_os_string)
                .ok_or_else(|| "no shell was found: name one in [host] shell".to_owned())
        }
        #[cfg(not(windows))]
        {
            Ok(self
                .shell
                .as_ref()
                .map(OsString::from)
                .or_else(|| std::env::var_os("SHELL").filter(|shell| !shell.is_empty()))
                .unwrap_or_else(|| OsString::from("/bin/sh")))
        }
    }

    fn start_in(&self) -> Option<PathBuf> {
        self.home
            .clone()
            .or_else(|| std::env::current_dir().ok())
            .filter(|dir| dir.is_dir())
    }

    /// How many terminals run now.
    #[cfg(test)]
    pub(crate) fn running(&self) -> usize {
        self.live.load(Ordering::SeqCst)
    }
}

/// The program `name` stands for as an absolute path: itself when it is one, else the first match in a folder of `PATH` that is
/// itself absolute (so never the working folder), with `.exe` added when it has no extension. `None` when there is none.
#[cfg(windows)]
fn resolve(name: &std::ffi::OsStr) -> Option<PathBuf> {
    let with_extension = |path: PathBuf| {
        if path.extension().is_some() {
            return path.is_file().then_some(path);
        }
        let exe = path.with_extension("exe");
        exe.is_file().then_some(exe)
    };
    let path = std::path::Path::new(name);
    if path.is_absolute() {
        return with_extension(path.to_path_buf());
    }
    if path.components().count() != 1 {
        return None;
    }
    std::env::split_paths(&std::env::var_os("PATH")?)
        .filter(|dir| dir.is_absolute())
        .find_map(|dir| with_extension(dir.join(name)))
}

/// One place of the host's `max` terminals, given back when it is dropped.
struct Slot(Arc<AtomicUsize>);

impl Slot {
    fn take(live: &Arc<AtomicUsize>) -> Option<Self> {
        let before = live.fetch_add(1, Ordering::SeqCst);
        if before >= MAX_PER_HOST {
            live.fetch_sub(1, Ordering::SeqCst);
            None
        } else {
            Some(Self(Arc::clone(live)))
        }
    }
}

impl Drop for Slot {
    fn drop(&mut self) {
        self.0.fetch_sub(1, Ordering::SeqCst);
    }
}

/// The latest output, and how much there has been in all.
#[derive(Default)]
struct Scroll {
    bytes: VecDeque<u8>,
    /// Bytes written by the shell since it started: the position just after the last one kept.
    end: u64,
}

impl Scroll {
    fn keep(&mut self, chunk: &[u8]) -> u64 {
        self.bytes.extend(chunk);
        let over = self.bytes.len().saturating_sub(SCROLLBACK);
        self.bytes.drain(..over);
        self.end += chunk.len() as u64;
        self.end
    }
}

/// What a terminal's threads and its link share.
struct Terminal {
    process: Arc<Shell>,
    scroll: Mutex<Scroll>,
    /// The person closed it, so its end is not announced.
    quiet: AtomicBool,
    ended: AtomicBool,
}

/// A terminal as its link holds it. Dropping `input` is what lets the thread that writes to the shell end.
struct Entry {
    terminal: Arc<Terminal>,
    input: SyncSender<Vec<u8>>,
}

/// The terminals one link has open, and the threads that carry their output.
pub(crate) struct Link<'scope, 'env, C: Channel> {
    scope: &'scope Scope<'scope, 'env>,
    channel: &'env C,
    host: &'env Terminals,
    open: Mutex<BTreeMap<u32, Entry>>,
    next: AtomicU32,
}

impl<C: Channel> Drop for Link<'_, '_, C> {
    fn drop(&mut self) {
        // However the link's loop ended, the shells end: the scope it runs in waits for their threads.
        self.close_all();
    }
}

impl<C: Channel> core::fmt::Debug for Link<'_, '_, C> {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Link").finish_non_exhaustive()
    }
}

impl<'scope, 'env: 'scope, C: Channel> Link<'scope, 'env, C> {
    /// A link with no terminals, whose threads run in `scope`, which must not end before [`Link::close_all`] has been called.
    pub(crate) fn new(
        scope: &'scope Scope<'scope, 'env>,
        channel: &'env C,
        host: &'env Terminals,
    ) -> Self {
        Self {
            scope,
            channel,
            host,
            open: Mutex::new(BTreeMap::new()),
            next: AtomicU32::new(1),
        }
    }

    /// Answers one `terminal/*` request.
    pub(crate) fn call(&self, method: &str, params: &Value) -> Result<Value, Fail> {
        match method {
            "terminal/open" => self.open(params),
            "terminal/write" => self.write(params),
            "terminal/resize" => self.resize(params),
            "terminal/scrollback" => self.scrollback(params),
            "terminal/close" => self.close(params),
            _ => Err(bad("this host has no such terminal method")),
        }
    }

    /// Whether a shell of this link is still running, which keeps the link from being ended for its silence.
    pub(crate) fn any_running(&self) -> bool {
        lock(&self.open)
            .values()
            .any(|entry| !entry.terminal.ended.load(Ordering::SeqCst))
    }

    /// Ends every terminal of this link and what each started. The link's threads end with them.
    pub(crate) fn close_all(&self) {
        let entries = core::mem::take(&mut *lock(&self.open));
        for entry in entries.into_values() {
            entry.terminal.quiet.store(true, Ordering::SeqCst);
            entry.terminal.process.end();
        }
    }

    fn open(&self, params: &Value) -> Result<Value, Fail> {
        let size = size_of(params)?;
        if let Some(name) = text(params, "environment")?
            && name != crate::environments::HERE
        {
            return self.open_in(name, params, size);
        }
        let cwd = match text(params, "cwd")? {
            Some(path) => {
                // A share is an ordinary folder, written plain; a device namespace is no folder.
                let path = crate::share::plain(std::path::Path::new(path)).into_owned();
                if !path.is_absolute() {
                    return Err(bad("cwd is an absolute path"));
                }
                if !path.is_dir() {
                    return Err(bad("cwd is not a folder on this computer"));
                }
                path
            }
            None => self
                .host
                .start_in()
                .ok_or_else(|| bad("this host knows no folder to start in: name a cwd"))?,
        };
        let mut open = lock(&self.open);
        if open
            .values()
            .filter(|e| !e.terminal.ended.load(Ordering::SeqCst))
            .count()
            >= MAX_PER_LINK
        {
            return Err(Fail::Refused(format!(
                "a link may have at most {MAX_PER_LINK} terminals open: close one"
            )));
        }
        // Ended ones stay until closed, so that their scrollback can be read; that is bounded too.
        if open.len() >= MAX_PER_LINK * 4 {
            return Err(Fail::Refused(
                "a link may hold at most 32 terminals, ended ones included: close some".to_owned(),
            ));
        }
        let slot = Slot::take(&self.host.live).ok_or_else(|| {
            Fail::Refused(format!(
                "this host runs at most {MAX_PER_HOST} terminals at once"
            ))
        })?;
        let program = self
            .host
            .program(text(params, "shell")?)
            .map_err(Fail::Refused)?;
        let started = pty::open(&program, &cwd, size).map_err(|error| {
            Fail::Refused(format!(
                "the shell {} could not be started: {error}",
                program.to_string_lossy()
            ))
        })?;
        let pty::Pty {
            input,
            output,
            process,
        } = started;
        let (input, output) = (Feed::Local(input), Drain::Local(output));
        let terminal = Arc::new(Terminal {
            process: Arc::new(Shell::Local(process)),
            scroll: Mutex::new(Scroll::default()),
            quiet: AtomicBool::new(false),
            ended: AtomicBool::new(false),
        });
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (typed, to_shell) = sync_channel::<Vec<u8>>(QUEUED_WRITES);
        let (exit_code, exit_code_rx) = channel::<i32>();
        let spawned = self.spawn_threads(
            id,
            &terminal,
            (input, to_shell),
            (output, exit_code, exit_code_rx),
            slot,
        );
        if let Err(error) = spawned {
            terminal.process.end();
            return Err(Fail::Refused(format!(
                "the terminal's threads could not start: {error}"
            )));
        }
        open.insert(
            id,
            Entry {
                terminal,
                input: typed,
            },
        );
        let mut opened =
            json!({"id": id, "shell": program.to_string_lossy(), "cwd": cwd.to_string_lossy()});
        // cmd.exe refuses a folder on a share as its working directory and starts in Windows' own folder instead: say so, the
        // terminal is not moved by this host.
        if cfg!(windows)
            && crate::share::is_share(&cwd)
            && std::path::Path::new(&program)
                .file_stem()
                .is_some_and(|stem| stem.eq_ignore_ascii_case("cmd"))
        {
            opened["notice"] = json!(
                "cmd.exe cannot start in a folder on a share and opened in its own folder; use PowerShell or run pushd there"
            );
        }
        Ok(opened)
    }

    /// Opens a shell in the environment `name`, through its spawner: its programs, its folders and its shells, as the environment
    /// says them. The host checks none of them against this computer's.
    fn open_in(&self, name: &str, params: &Value, size: Size) -> Result<Value, Fail> {
        let environments = self
            .host
            .environments
            .as_ref()
            .ok_or_else(|| Fail::Refused("this host has no environments".to_owned()))?;
        let env = environments.get(name).map_err(Fail::Refused)?;
        let info = env
            .needs(canoa::environment::capability::TERMINAL)
            .map_err(Fail::Refused)?;
        let spawner = env.terminal_spawner().map_err(Fail::Refused)?;
        let program = match text(params, "shell")? {
            Some(shell) => shell.to_owned(),
            None => info
                .capability(canoa::environment::capability::TERMINAL)
                .and_then(|each| each.facts.get("shell").cloned())
                .unwrap_or_else(|| {
                    if info.os == "windows" {
                        "cmd.exe"
                    } else {
                        "/bin/sh"
                    }
                    .to_owned()
                }),
        };
        let mut spec = coqui_acp_api::SpawnSpec::new(program.clone())
            .terminal(coqui_acp_api::TerminalSize::new(size.rows, size.cols));
        if let Some(cwd) = text(params, "cwd")? {
            spec = spec.cwd(cwd);
        }
        let mut open = lock(&self.open);
        if open
            .iter()
            .filter(|(_, e)| !e.terminal.ended.load(Ordering::SeqCst))
            .count()
            >= MAX_PER_LINK
        {
            return Err(Fail::Refused(format!(
                "a link may have at most {MAX_PER_LINK} terminals open: close one"
            )));
        }
        let slot = Slot::take(&self.host.live).ok_or_else(|| {
            Fail::Refused(format!(
                "this host runs at most {MAX_PER_HOST} terminals at once"
            ))
        })?;
        let started = spawner.spawn(spec).map_err(|error| {
            Fail::Refused(format!(
                "the shell {program} could not be started in {name}: {error}"
            ))
        })?;
        let process = crate::environments::remote::Remote::new(started);
        let terminal = Arc::new(Terminal {
            process: Arc::new(Shell::Remote(process.clone())),
            scroll: Mutex::new(Scroll::default()),
            quiet: AtomicBool::new(false),
            ended: AtomicBool::new(false),
        });
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (typed, to_shell) = sync_channel::<Vec<u8>>(QUEUED_WRITES);
        let (exit_code, exit_code_rx) = channel::<i32>();
        let spawned = self.spawn_threads(
            id,
            &terminal,
            (Feed::Remote(process.input()), to_shell),
            (Drain::Remote(process.output()), exit_code, exit_code_rx),
            slot,
        );
        if let Err(error) = spawned {
            terminal.process.end();
            return Err(Fail::Refused(format!(
                "the terminal's threads could not start: {error}"
            )));
        }
        open.insert(
            id,
            Entry {
                terminal,
                input: typed,
            },
        );
        Ok(json!({"id": id, "shell": program, "environment": name, "cwd": text(params, "cwd")?}))
    }

    fn spawn_threads(
        &self,
        id: u32,
        terminal: &Arc<Terminal>,
        (mut input, to_shell): (Feed, Receiver<Vec<u8>>),
        (mut output, exit_code, exit_code_rx): (Drain, std::sync::mpsc::Sender<i32>, Receiver<i32>),
        slot: Slot,
    ) -> std::io::Result<()> {
        // The shell's input: one thread, so that a shell that does not read holds up only its own terminal.
        Builder::new()
            .name(format!("canoa-terminal-{id}-in"))
            .spawn_scoped(self.scope, move || {
                for bytes in to_shell {
                    if input.write_all(&bytes).is_err() {
                        break;
                    }
                }
            })?;
        // Whether the shell has ended, and with what.
        let process = Arc::clone(&terminal.process);
        Builder::new()
            .name(format!("canoa-terminal-{id}-wait"))
            .spawn_scoped(self.scope, move || {
                let _ = exit_code.send(process.wait());
            })?;
        // Its output, passed on as it is written.
        let terminal = Arc::clone(terminal);
        let channel = self.channel;
        Builder::new()
            .name(format!("canoa-terminal-{id}-out"))
            .spawn_scoped(self.scope, move || {
                let _slot = slot;
                let mut buf = vec![0_u8; CHUNK];
                // A link that cannot take more is no reason to stop reading: the console has to be drained to close.
                let mut sending = true;
                while let Ok(n) = output.read(&mut buf)
                    && n > 0
                {
                    let end = lock(&terminal.scroll).keep(&buf[..n]);
                    if sending {
                        sending = notify(
                            channel,
                            "terminal/output",
                            &json!({"id": id, "data": BASE64.encode(&buf[..n]), "end": end}),
                            &terminal.quiet,
                        );
                        if !sending {
                            // Nobody is listening any more: the shell is not left running for no one.
                            terminal.process.end();
                        }
                    }
                }
                terminal.ended.store(true, Ordering::SeqCst);
                // Whatever ended the reading, the shell and what it started go with it.
                terminal.process.end();
                let code = exit_code_rx.recv_timeout(EXIT_WAIT).unwrap_or(-1);
                if sending && !terminal.quiet.load(Ordering::SeqCst) {
                    notify(
                        channel,
                        "terminal/exit",
                        &json!({"id": id, "code": code}),
                        &terminal.quiet,
                    );
                }
            })?;
        Ok(())
    }

    fn entry(&self, params: &Value) -> Result<(Arc<Terminal>, SyncSender<Vec<u8>>), Fail> {
        let id = params
            .get("id")
            .and_then(Value::as_u64)
            .and_then(|id| u32::try_from(id).ok())
            .ok_or_else(|| bad("id is the number terminal/open answered"))?;
        lock(&self.open)
            .get(&id)
            .map(|entry| (Arc::clone(&entry.terminal), entry.input.clone()))
            .ok_or_else(|| Fail::NotFound("this link has no such terminal".to_owned()))
    }

    fn write(&self, params: &Value) -> Result<Value, Fail> {
        let (terminal, input) = self.entry(params)?;
        let data = text(params, "data")?
            .ok_or_else(|| bad("data is needed: base64 of the bytes typed"))?;
        if data.len() > MAX_WRITE.div_ceil(3) * 4 {
            return Err(bad(&format!("data is at most {MAX_WRITE} bytes")));
        }
        let bytes = BASE64
            .decode(data.as_bytes())
            .map_err(|_| bad("data is base64"))?;
        if bytes.len() > MAX_WRITE {
            return Err(bad(&format!("data is at most {MAX_WRITE} bytes")));
        }
        if terminal.ended.load(Ordering::SeqCst) {
            return Err(Fail::Refused("the terminal has ended".to_owned()));
        }
        match input.try_send(bytes) {
            Ok(()) => Ok(json!({})),
            Err(TrySendError::Full(_)) => Err(Fail::Refused(
                "the shell is not reading what was typed: wait and send again".to_owned(),
            )),
            Err(TrySendError::Disconnected(_)) => {
                Err(Fail::Refused("the terminal has ended".to_owned()))
            }
        }
    }

    fn resize(&self, params: &Value) -> Result<Value, Fail> {
        let (terminal, _) = self.entry(params)?;
        let size = size_of(params)?;
        terminal.process.resize(size).map_err(|error| {
            Fail::Refused(format!("the terminal could not be resized: {error}"))
        })?;
        Ok(json!({}))
    }

    fn scrollback(&self, params: &Value) -> Result<Value, Fail> {
        let (terminal, _) = self.entry(params)?;
        let scroll = lock(&terminal.scroll);
        let bytes: Vec<u8> = scroll.bytes.iter().copied().collect();
        Ok(json!({"data": BASE64.encode(&bytes), "end": scroll.end}))
    }

    fn close(&self, params: &Value) -> Result<Value, Fail> {
        let (terminal, _) = self.entry(params)?;
        let id = params.get("id").and_then(Value::as_u64).unwrap_or(0);
        terminal.quiet.store(true, Ordering::SeqCst);
        terminal.process.end();
        if let Ok(id) = u32::try_from(id) {
            lock(&self.open).remove(&id);
        }
        Ok(json!({"closed": true}))
    }
}

/// How long to wait before offering a notification again to a link whose queue is full.
const RETRY: Duration = Duration::from_millis(50);

/// Sends one notification and says whether the link took it. A link whose queue is full is waited for, so a shell that writes
/// faster than the link carries is slowed and not ended; the answer is `false` only once the link has ended or `stop` is set.
fn notify<C: Channel>(channel: &C, method: &str, params: &Value, stop: &AtomicBool) -> bool {
    let Ok(bytes) =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "method": method, "params": params}))
    else {
        return false;
    };
    loop {
        match channel.send(&bytes) {
            Ok(()) => return true,
            Err(Failure::NoProgress | Failure::Timeout)
                if !channel.is_closed() && !stop.load(Ordering::SeqCst) =>
            {
                std::thread::sleep(RETRY);
            }
            Err(_) => return false,
        }
    }
}

fn text<'a>(params: &'a Value, key: &str) -> Result<Option<&'a str>, Fail> {
    match params.get(key) {
        None | Some(Value::Null) => Ok(None),
        Some(Value::String(text)) => Ok(Some(text)),
        Some(_) => Err(bad(&format!("{key} is text"))),
    }
}

/// `cols` and `rows`, 80 by 24 when absent.
fn size_of(params: &Value) -> Result<Size, Fail> {
    let cell = |key: &str, default: u16| -> Result<u16, Fail> {
        match params.get(key) {
            None | Some(Value::Null) => Ok(default),
            Some(value) => value
                .as_u64()
                .and_then(|n| u16::try_from(n).ok())
                .filter(|n| (1..=MAX_CELLS).contains(n))
                .ok_or_else(|| bad(&format!("{key} is 1 to {MAX_CELLS}"))),
        }
    };
    Ok(Size {
        cols: cell("cols", 80)?,
        rows: cell("rows", 24)?,
    })
}

#[cfg(test)]
mod tests;
