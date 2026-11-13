//! The host's own process starter, on Unix: a process the application starts, which starts the host's children and keeps them,
//! so that the application's own process handling cannot collect them. The host asks it over a socket, with the descriptors
//! of the child's standard streams riding on the request, and it says when a child has ended.
//!
//! A child stays uncollected in the starter until the host collects it (or lets go of it), and a signal to its process group is
//! sent only while it is uncollected, so the group's id is still the child's own.
//!
//! One frame is eight bytes (`kind`, the number of descriptors, two zeros, the length of the body as a `u32`), then the body.
//! The descriptors are sent with the first byte of the frame. The host sends `SPAWN` (with three descriptors: the child's
//! standard input, output and error), `KILL`, `KILL_GROUP` and `COLLECT`; the starter answers `SPAWNED`, `FAILED` and, once a
//! child has ended, `EXITED`.

use alloc::sync::Arc;
use alloc::vec::Vec;
use core::mem::MaybeUninit;
use std::collections::HashMap;
use std::ffi::OsStr;
use std::fs::{File, OpenOptions};
use std::io::{self, IoSlice, IoSliceMut, Read as _, Write as _};
use std::os::fd::{AsFd as _, BorrowedFd, OwnedFd};
use std::os::unix::ffi::OsStrExt as _;
use std::os::unix::net::UnixStream;
use std::os::unix::process::{CommandExt as _, ExitStatusExt as _};
use std::path::Path;
use std::process::{ChildStderr, ChildStdin, ChildStdout, ExitStatus};
use std::sync::mpsc::{self, SyncSender};
use std::sync::{Condvar, Mutex, MutexGuard, OnceLock, PoisonError};
use std::thread;

use rustix::net::{
    RecvAncillaryBuffer, RecvAncillaryMessage, RecvFlags, SendAncillaryBuffer,
    SendAncillaryMessage, SendFlags, recvmsg, sendmsg,
};

use super::{Child, Command, Inner, Stdio, lost};
use crate::cli::Program;

/// The command, after the program's own arguments, that runs [`serve`].
pub(crate) const SUBCOMMAND: &str = "spawner";

const HEADER: usize = 8;
/// The most a frame's body holds: a command line and an environment.
const MAX_BODY: usize = 64 * 1024 * 1024;

const SPAWN: u8 = 1;
const KILL: u8 = 2;
const KILL_GROUP: u8 = 3;
const COLLECT: u8 = 4;
const SPAWNED: u8 = 11;
const FAILED: u8 = 12;
const EXITED: u8 = 13;

fn lock<T>(mutex: &Mutex<T>) -> MutexGuard<'_, T> {
    mutex.lock().unwrap_or_else(PoisonError::into_inner)
}

fn wrong(what: &str) -> io::Error {
    io::Error::new(io::ErrorKind::InvalidData, what.to_owned())
}

struct Frame {
    kind: u8,
    body: Vec<u8>,
    fds: Vec<OwnedFd>,
}

/// Sends one frame, its descriptors with its first byte.
fn send(socket: &UnixStream, kind: u8, body: &[u8], fds: &[BorrowedFd<'_>]) -> io::Result<()> {
    let length = u32::try_from(body.len()).map_err(|_| wrong("a frame too long"))?;
    let count = u8::try_from(fds.len()).map_err(|_| wrong("too many descriptors"))?;
    let mut message = Vec::with_capacity(HEADER + body.len());
    message.extend_from_slice(&[kind, count, 0, 0]);
    message.extend_from_slice(&length.to_le_bytes());
    message.extend_from_slice(body);
    let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
    let mut control = SendAncillaryBuffer::new(&mut space);
    if !fds.is_empty() && !control.push(SendAncillaryMessage::ScmRights(fds)) {
        return Err(wrong("too many descriptors"));
    }
    let sent = loop {
        match sendmsg(
            socket,
            &[IoSlice::new(&message)],
            &mut control,
            SendFlags::empty(),
        ) {
            Err(rustix::io::Errno::INTR) => {}
            other => break other.map_err(io::Error::from)?,
        }
    };
    let mut socket = socket;
    socket.write_all(&message[sent..])
}

/// The next frame, or `None` when the other side has closed the socket between frames.
fn receive(socket: &UnixStream) -> io::Result<Option<Frame>> {
    let mut header = [0_u8; HEADER];
    let mut have = 0;
    let mut fds: Vec<OwnedFd> = Vec::new();
    while have < HEADER {
        let mut space = [MaybeUninit::uninit(); rustix::cmsg_space!(ScmRights(3))];
        let mut control = RecvAncillaryBuffer::new(&mut space);
        let got = loop {
            match recvmsg(
                socket,
                &mut [IoSliceMut::new(&mut header[have..])],
                &mut control,
                RecvFlags::empty(),
            ) {
                Err(rustix::io::Errno::INTR) => {}
                other => break other.map_err(io::Error::from)?,
            }
        };
        for message in control.drain() {
            if let RecvAncillaryMessage::ScmRights(received) = message {
                fds.extend(received);
            }
        }
        if got.bytes == 0 {
            return if have == 0 && fds.is_empty() {
                Ok(None)
            } else {
                Err(io::ErrorKind::UnexpectedEof.into())
            };
        }
        have += got.bytes;
    }
    for fd in &fds {
        rustix::io::fcntl_setfd(fd, rustix::io::FdFlags::CLOEXEC)?;
    }
    let length = usize::try_from(u32::from_le_bytes([
        header[4], header[5], header[6], header[7],
    ]))
    .map_err(|_| wrong("a frame too long"))?;
    if length > MAX_BODY || usize::from(header[1]) != fds.len() {
        return Err(wrong("a frame that is not one"));
    }
    let mut body = vec![0_u8; length];
    let mut socket = socket;
    socket.read_exact(&mut body)?;
    Ok(Some(Frame {
        kind: header[0],
        body,
        fds,
    }))
}

fn put(out: &mut Vec<u8>, bytes: &[u8]) -> io::Result<()> {
    let length = u32::try_from(bytes.len()).map_err(|_| wrong("a field too long"))?;
    out.extend_from_slice(&length.to_le_bytes());
    out.extend_from_slice(bytes);
    Ok(())
}

/// Reads what [`put`] wrote.
struct Fields<'a>(&'a [u8]);

impl<'a> Fields<'a> {
    fn take(&mut self, len: usize) -> Option<&'a [u8]> {
        if self.0.len() < len {
            return None;
        }
        let (taken, rest) = self.0.split_at(len);
        self.0 = rest;
        Some(taken)
    }

    fn number(&mut self) -> Option<u32> {
        let bytes = self.take(4)?;
        Some(u32::from_le_bytes([bytes[0], bytes[1], bytes[2], bytes[3]]))
    }

    fn signed(&mut self) -> Option<i32> {
        self.number().map(u32::cast_signed)
    }

    fn byte(&mut self) -> Option<u8> {
        self.take(1).map(|byte| byte[0])
    }

    fn bytes(&mut self) -> Option<&'a [u8]> {
        let length = usize::try_from(self.number()?).ok()?;
        self.take(length)
    }
}

// ---------------------------------------------------------------------------------------------------------------------
// The starter's side.

/// What the starter knows of one child: whether the host has asked for it to be collected, and whether it has been.
struct Entry {
    life: Mutex<Life>,
    changed: Condvar,
}

struct Life {
    collect: bool,
    reaped: bool,
}

type Table = Arc<Mutex<HashMap<u32, Arc<Entry>>>>;

/// Runs the starter on standard input, which is the socket the host started it with: until the host closes it. Children it
/// started are left as they are then.
///
/// # Errors
///
/// The socket could not be read, or the host sent what is not a request.
pub fn serve() -> io::Result<()> {
    let socket = UnixStream::from(std::io::stdin().as_fd().try_clone_to_owned()?);
    run(&socket)
}

fn run(socket: &UnixStream) -> io::Result<()> {
    let writer = Arc::new(Mutex::new(socket.try_clone()?));
    let table: Table = Arc::default();
    while let Some(frame) = receive(socket)? {
        match frame.kind {
            SPAWN => spawn(frame, &writer, &table)?,
            KILL | KILL_GROUP | COLLECT => {
                let pid = Fields(&frame.body)
                    .number()
                    .ok_or_else(|| wrong("a request with no process"))?;
                let entry = lock(&table).get(&pid).cloned();
                if let Some(entry) = entry {
                    signal(&entry, pid, frame.kind);
                }
            }
            _ => return Err(wrong("a request of a kind unknown")),
        }
    }
    Ok(())
}

/// What a request does to a child it names, while it is the starter's.
fn signal(entry: &Entry, pid: u32, kind: u8) {
    let mut life = lock(&entry.life);
    if kind == COLLECT {
        life.collect = true;
        entry.changed.notify_all();
        return;
    }
    let Some(pid) = i32::try_from(pid)
        .ok()
        .filter(|pid| *pid > 1)
        .and_then(rustix::process::Pid::from_raw)
    else {
        return;
    };
    // Not once it is collected: its id, and its group's, may be anyone's.
    if life.reaped {
        return;
    }
    let _ = if kind == KILL {
        rustix::process::kill_process(pid, rustix::process::Signal::KILL)
    } else {
        rustix::process::kill_process_group(pid, rustix::process::Signal::KILL)
    };
}

/// Starts the child a `SPAWN` frame describes and says so, or why not.
fn spawn(frame: Frame, writer: &Arc<Mutex<UnixStream>>, table: &Table) -> io::Result<()> {
    let mut fields = Fields(&frame.body);
    let request = fields
        .number()
        .ok_or_else(|| wrong("a spawn with no number"))?;
    let started = start(&mut fields, frame.fds);
    let mut said = Vec::new();
    said.extend_from_slice(&request.to_le_bytes());
    match started {
        Ok(pid) => {
            let entry = Arc::new(Entry {
                life: Mutex::new(Life {
                    collect: false,
                    reaped: false,
                }),
                changed: Condvar::new(),
            });
            // Before the host hears of it, so that nothing it then asks of it finds none.
            lock(table).insert(pid, Arc::clone(&entry));
            said.extend_from_slice(&pid.to_le_bytes());
            let watching = {
                let (writer, table) = (Arc::clone(writer), Arc::clone(table));
                thread::Builder::new()
                    .name("canoa-spawner-child".to_owned())
                    .spawn(move || watch(pid, &entry, &writer, &table))
            };
            if let Err(error) = watching {
                lock(table).remove(&pid);
                tracing::warn!("a child of the process starter cannot be watched: {error}");
            }
            send(&lock(writer), SPAWNED, &said, &[])
        }
        Err(error) => {
            said.extend_from_slice(&error.raw_os_error().unwrap_or(0).to_le_bytes());
            put(&mut said, error.to_string().as_bytes())?;
            send(&lock(writer), FAILED, &said, &[])
        }
    }
}

/// Starts the command in `fields` with `fds` for its standard streams and gives its id.
fn start(fields: &mut Fields<'_>, mut fds: Vec<OwnedFd>) -> io::Result<u32> {
    let malformed = || wrong("a spawn that is not one");
    let flags = fields.byte().ok_or_else(malformed)?;
    let program = fields.bytes().ok_or_else(malformed)?;
    let dir = fields.bytes().ok_or_else(malformed)?;
    let mut command = std::process::Command::new(OsStr::from_bytes(program));
    for _ in 0..fields.number().ok_or_else(malformed)? {
        command.arg(OsStr::from_bytes(fields.bytes().ok_or_else(malformed)?));
    }
    command.env_clear();
    for _ in 0..fields.number().ok_or_else(malformed)? {
        let name = fields.bytes().ok_or_else(malformed)?;
        let value = fields.bytes().ok_or_else(malformed)?;
        command.env(OsStr::from_bytes(name), OsStr::from_bytes(value));
    }
    if flags & HAS_DIR != 0 {
        command.current_dir(OsStr::from_bytes(dir));
    }
    if flags & OWN_GROUP != 0 {
        command.process_group(0);
    }
    let [stdin, stdout, stderr] = <[OwnedFd; 3]>::try_from(core::mem::take(&mut fds))
        .map_err(|_| wrong("a spawn without its three streams"))?;
    command
        .stdin(std::process::Stdio::from(stdin))
        .stdout(std::process::Stdio::from(stdout))
        .stderr(std::process::Stdio::from(stderr));
    let child = command.spawn()?;
    let pid = child.id();
    // The command holds the child's ends of its pipes: let go of them, or the host never sees them end.
    drop(command);
    // Its end is the starter's to wait on, below, and std's handle does nothing when it goes.
    drop(child);
    Ok(pid)
}

const OWN_GROUP: u8 = 1;
const HAS_DIR: u8 = 2;

/// Waits for the child to end, says so, and collects it once the host asks.
fn watch(pid: u32, entry: &Entry, writer: &Mutex<UnixStream>, table: &Table) {
    let status = ended(pid);
    let mut said = Vec::new();
    said.extend_from_slice(&pid.to_le_bytes());
    said.extend_from_slice(&status.to_le_bytes());
    let told = send(&lock(writer), EXITED, &said, &[]);
    let mut life = lock(&entry.life);
    while !life.collect && told.is_ok() {
        life = entry
            .changed
            .wait(life)
            .unwrap_or_else(PoisonError::into_inner);
    }
    if let Some(pid) = i32::try_from(pid)
        .ok()
        .and_then(rustix::process::Pid::from_raw)
    {
        let _ = rustix::process::waitpid(Some(pid), rustix::process::WaitOptions::empty());
    }
    life.reaped = true;
    drop(life);
    lock(table).remove(&pid);
}

/// Waits for the child to end, leaving it uncollected, and gives its wait status.
fn ended(pid: u32) -> i32 {
    use rustix::process::{Pid, WaitId, WaitIdOptions, waitid};
    let Some(pid) = i32::try_from(pid).ok().and_then(Pid::from_raw) else {
        return UNKNOWN;
    };
    loop {
        match waitid(
            WaitId::Pid(pid),
            WaitIdOptions::EXITED | WaitIdOptions::NOWAIT,
        ) {
            Err(rustix::io::Errno::INTR) => {}
            // A wait status: the exit code in the second byte, or the signal in the low seven bits.
            Ok(Some(info)) => {
                return match info.exit_status() {
                    Some(code) => (code & 0xff) << 8,
                    None => info.terminating_signal().unwrap_or(0) & 0x7f,
                };
            }
            _ => return UNKNOWN,
        }
    }
}

/// The status of a child the starter could not wait on: exited with 255.
const UNKNOWN: i32 = 0xff << 8;

// ---------------------------------------------------------------------------------------------------------------------
// The host's side.

/// How a child of the starter stands, as far as the host has heard.
#[derive(Clone, Copy)]
enum State {
    Running,
    Exited(i32),
    /// The starter ended with the child running: nothing more will be heard of it.
    Lost,
}

struct Fate {
    state: Mutex<State>,
    changed: Condvar,
}

impl Fate {
    fn set(&self, state: State) {
        *lock(&self.state) = state;
        self.changed.notify_all();
    }
}

type Reply = SyncSender<io::Result<(u32, Arc<Fate>)>>;

struct Book {
    next: u32,
    pending: HashMap<u32, Reply>,
    children: HashMap<u32, Arc<Fate>>,
    ended: bool,
}

/// The host's side of a connection to a starter.
pub(super) struct Link {
    writer: Mutex<UnixStream>,
    book: Mutex<Book>,
}

impl Link {
    /// Runs on `socket`, whose other end the starter has, and listens for what it says on a thread of its own. `process` is
    /// the starter's, which that thread collects when the socket ends.
    fn over(socket: UnixStream, process: Option<std::process::Child>) -> io::Result<Arc<Self>> {
        let listening = socket.try_clone()?;
        let link = Arc::new(Self {
            writer: Mutex::new(socket),
            book: Mutex::new(Book {
                next: 0,
                pending: HashMap::new(),
                children: HashMap::new(),
                ended: false,
            }),
        });
        let mine = Arc::clone(&link);
        thread::Builder::new()
            .name("canoa-spawner-link".to_owned())
            .spawn(move || {
                while let Ok(Some(frame)) = receive(&listening) {
                    mine.heard(&frame);
                }
                mine.end();
                if let Some(mut process) = process {
                    let _ = process.wait();
                }
            })?;
        Ok(link)
    }

    /// Starts the starter: `program`'s command line with `spawner`, its standard input the socket.
    fn start(program: &Program) -> io::Result<Arc<Self>> {
        let (host, starter) = UnixStream::pair()?;
        let mut command = std::process::Command::new(&program.path);
        command
            .args(&program.args)
            .arg(SUBCOMMAND)
            .stdin(std::process::Stdio::from(OwnedFd::from(starter)))
            .stdout(std::process::Stdio::null())
            .stderr(std::process::Stdio::inherit());
        let process = command.spawn()?;
        // The command holds the starter's end: with it open here, the host would never see the starter end.
        drop(command);
        Self::over(host, Some(process))
    }

    fn heard(&self, frame: &Frame) {
        let mut fields = Fields(&frame.body);
        match frame.kind {
            SPAWNED | FAILED => {
                let Some(request) = fields.number() else {
                    return;
                };
                let reply = lock(&self.book).pending.remove(&request);
                let Some(reply) = reply else {
                    return;
                };
                let answer = if frame.kind == SPAWNED {
                    fields.number().map(|pid| {
                        let fate = Arc::new(Fate {
                            state: Mutex::new(State::Running),
                            changed: Condvar::new(),
                        });
                        // Before the caller has the id, so that an end heard at once is not lost.
                        lock(&self.book).children.insert(pid, Arc::clone(&fate));
                        (pid, fate)
                    })
                } else {
                    None
                };
                let _ = reply.send(answer.ok_or_else(|| {
                    let code = fields.signed().unwrap_or(0);
                    let text = String::from_utf8_lossy(fields.bytes().unwrap_or_default());
                    if code == 0 {
                        io::Error::other(text.into_owned())
                    } else {
                        io::Error::from_raw_os_error(code)
                    }
                }));
            }
            EXITED => {
                if let (Some(pid), Some(status)) = (fields.number(), fields.signed()) {
                    let fate = lock(&self.book).children.get(&pid).cloned();
                    if let Some(fate) = fate {
                        fate.set(State::Exited(status));
                    }
                }
            }
            _ => {}
        }
    }

    /// The starter is gone: no child will be heard of again, and no spawn answered.
    fn end(&self) {
        let (pending, children) = {
            let mut book = lock(&self.book);
            book.ended = true;
            (
                core::mem::take(&mut book.pending),
                core::mem::take(&mut book.children),
            )
        };
        for reply in pending.into_values() {
            let _ = reply.send(Err(lost()));
        }
        for fate in children.into_values() {
            let mut state = lock(&fate.state);
            if matches!(*state, State::Running) {
                *state = State::Lost;
            }
            drop(state);
            fate.changed.notify_all();
        }
    }

    fn has_ended(&self) -> bool {
        lock(&self.book).ended
    }

    /// Asks the starter to do `kind` to the child `pid`.
    fn ask(&self, kind: u8, pid: u32) -> io::Result<()> {
        if self.has_ended() {
            return Err(lost());
        }
        send(&lock(&self.writer), kind, &pid.to_le_bytes(), &[])
    }

    /// Starts `command` in the starter.
    pub(super) fn spawn(self: &Arc<Self>, command: &Command) -> io::Result<Child> {
        let plumbing = Plumbing::of(command)?;
        let dir = command.dir.clone().or_else(|| std::env::current_dir().ok());
        let (reply, answer) = mpsc::sync_channel(1);
        let request = {
            let mut book = lock(&self.book);
            if book.ended {
                return Err(lost());
            }
            book.next = book.next.wrapping_add(1);
            let request = book.next;
            book.pending.insert(request, reply);
            request
        };
        let sent = encode(request, command, dir.as_deref()).and_then(|body| {
            let [stdin, stdout, stderr] = &plumbing.for_child;
            send(
                &lock(&self.writer),
                SPAWN,
                &body,
                &[stdin.as_fd(), stdout.as_fd(), stderr.as_fd()],
            )
        });
        // Its ends of the pipes are the starter's now, and held here they would keep each pipe from ending.
        let Plumbing {
            for_child,
            stdin,
            stdout,
            stderr,
        } = plumbing;
        drop(for_child);
        if let Err(error) = sent {
            lock(&self.book).pending.remove(&request);
            return Err(error);
        }
        let (pid, fate) = answer.recv().map_err(|_| lost())??;
        Ok(Child {
            stdin,
            stdout,
            stderr,
            inner: Inner::Apart(Handle {
                link: Arc::clone(self),
                pid,
                fate,
                collected: false,
            }),
        })
    }
}

/// A `SPAWN` request's body.
fn encode(request: u32, command: &Command, dir: Option<&Path>) -> io::Result<Vec<u8>> {
    let mut body = Vec::new();
    body.extend_from_slice(&request.to_le_bytes());
    body.push(
        if command.own_group { OWN_GROUP } else { 0 } | if dir.is_some() { HAS_DIR } else { 0 },
    );
    put(&mut body, command.program.as_bytes())?;
    put(
        &mut body,
        dir.map_or(&[][..], |dir| dir.as_os_str().as_bytes()),
    )?;
    body.extend_from_slice(
        &u32::try_from(command.args.len())
            .map_err(|_| wrong("too many arguments"))?
            .to_le_bytes(),
    );
    for arg in &command.args {
        put(&mut body, arg.as_bytes())?;
    }
    let environment = command.environment();
    body.extend_from_slice(
        &u32::try_from(environment.len())
            .map_err(|_| wrong("too many variables"))?
            .to_le_bytes(),
    );
    for (name, value) in &environment {
        put(&mut body, name.as_bytes())?;
        put(&mut body, value.as_bytes())?;
    }
    Ok(body)
}

/// The three standard streams of a child about to start: the ends the starter takes, and the host's own ends of the pipes.
struct Plumbing {
    for_child: [OwnedFd; 3],
    stdin: Option<ChildStdin>,
    stdout: Option<ChildStdout>,
    stderr: Option<ChildStderr>,
}

impl Plumbing {
    fn of(command: &Command) -> io::Result<Self> {
        let (child_in, stdin) = match command.stdin {
            Stdio::Piped => {
                let (read, write) = io::pipe()?;
                (
                    OwnedFd::from(read),
                    Some(ChildStdin::from(OwnedFd::from(write))),
                )
            }
            Stdio::Null => (OwnedFd::from(File::open("/dev/null")?), None),
            Stdio::Inherit => (inherit(std::io::stdin().as_fd())?, None),
        };
        let (child_out, stdout) = match command.stdout {
            Stdio::Piped => {
                let (read, write) = io::pipe()?;
                (
                    OwnedFd::from(write),
                    Some(ChildStdout::from(OwnedFd::from(read))),
                )
            }
            Stdio::Null => (null_for_writing()?, None),
            Stdio::Inherit => (inherit(std::io::stdout().as_fd())?, None),
        };
        let (child_err, stderr) = match command.stderr {
            Stdio::Piped => {
                let (read, write) = io::pipe()?;
                (
                    OwnedFd::from(write),
                    Some(ChildStderr::from(OwnedFd::from(read))),
                )
            }
            Stdio::Null => (null_for_writing()?, None),
            Stdio::Inherit => (inherit(std::io::stderr().as_fd())?, None),
        };
        Ok(Self {
            for_child: [child_in, child_out, child_err],
            stdin,
            stdout,
            stderr,
        })
    }
}

fn null_for_writing() -> io::Result<OwnedFd> {
    Ok(OwnedFd::from(
        OpenOptions::new().write(true).open("/dev/null")?,
    ))
}

/// A copy of this process's own stream, or nothing when it has none, as a process with its streams closed has.
fn inherit(own: BorrowedFd<'_>) -> io::Result<OwnedFd> {
    own.try_clone_to_owned().or_else(|_| {
        OpenOptions::new()
            .read(true)
            .write(true)
            .open("/dev/null")
            .map(OwnedFd::from)
    })
}

/// A child of the starter, as far as the host knows.
pub(super) struct Handle {
    link: Arc<Link>,
    pid: u32,
    fate: Arc<Fate>,
    collected: bool,
}

impl core::fmt::Debug for Handle {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Handle")
            .field("pid", &self.pid)
            .finish_non_exhaustive()
    }
}

impl Handle {
    pub(super) const fn id(&self) -> u32 {
        self.pid
    }

    fn state(&self) -> State {
        *lock(&self.fate.state)
    }

    /// Lets the starter collect the child, which it does once the child has ended.
    fn collect(&mut self) {
        if !self.collected {
            self.collected = true;
            lock(&self.link.book).children.remove(&self.pid);
            let _ = self.link.ask(COLLECT, self.pid);
        }
    }

    pub(super) fn peek(&mut self) -> io::Result<Option<ExitStatus>> {
        match self.state() {
            State::Running => Ok(None),
            State::Exited(status) => Ok(Some(ExitStatus::from_raw(status))),
            State::Lost => Err(lost()),
        }
    }

    pub(super) fn try_wait(&mut self) -> io::Result<Option<ExitStatus>> {
        let status = self.peek()?;
        if status.is_some() {
            self.collect();
        }
        Ok(status)
    }

    pub(super) fn wait(&mut self) -> io::Result<ExitStatus> {
        let mut state = lock(&self.fate.state);
        while matches!(*state, State::Running) {
            state = self
                .fate
                .changed
                .wait(state)
                .unwrap_or_else(PoisonError::into_inner);
        }
        drop(state);
        let status = self.peek()?;
        self.collect();
        status.ok_or_else(lost)
    }

    pub(super) fn kill(&mut self) -> io::Result<()> {
        if self.collected {
            return Ok(());
        }
        self.link.ask(KILL, self.pid)
    }

    pub(super) fn end_group(&mut self) {
        if !self.collected {
            let _ = self.link.ask(KILL_GROUP, self.pid);
        }
    }
}

impl Drop for Handle {
    fn drop(&mut self) {
        self.collect();
    }
}

struct Starter {
    program: Program,
    link: Mutex<Option<Arc<Link>>>,
}

static STARTER: OnceLock<Starter> = OnceLock::new();

pub(super) fn install(program: Program) {
    let _ = STARTER.set(Starter {
        program,
        link: Mutex::new(None),
    });
}

/// The connection to the starter, started if need be, or `None` when none is installed.
pub(super) fn link() -> io::Result<Option<Arc<Link>>> {
    let Some(starter) = STARTER.get() else {
        return Ok(None);
    };
    let mut held = lock(&starter.link);
    if let Some(link) = held.as_ref().filter(|link| !link.has_ended()) {
        return Ok(Some(Arc::clone(link)));
    }
    let link = Link::start(&starter.program)?;
    *held = Some(Arc::clone(&link));
    Ok(Some(link))
}

#[cfg(test)]
mod tests;
