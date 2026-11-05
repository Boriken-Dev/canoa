//! A provider's process for one connection. ACP runs on a loopback socket of its own, which the process proves with a
//! one-time token given on its standard input, so nothing the provider's code writes to standard output or error reaches
//! the client: those go to this host's standard error, at most [`OUTPUT_LIMIT`] bytes per connection.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::net::{Ipv4Addr, SocketAddr};
use core::sync::atomic::{AtomicBool, AtomicUsize, Ordering};
use core::time::Duration;
use std::ffi::OsString;
use std::io::{self, BufRead as _, BufReader, ErrorKind, Read, Write as _};
use std::net::{Shutdown, TcpListener, TcpStream};
use std::process::{Child, ChildStderr, ChildStdin, ChildStdout, Command, Stdio};
use std::sync::{Mutex, PoisonError};
use std::thread;
use std::time::Instant;

use coqui_web::{Channel, Failure};

use crate::provider::HostLine;
use crate::serve::MAX_LINE;

/// What a provider's process may write to its standard output and error, together, for one connection; the rest is
/// dropped after a note in the log.
pub const OUTPUT_LIMIT: usize = 1024 * 1024;
/// How long a provider's process has to connect back and prove its token.
pub const START_LIMIT: Duration = Duration::from_secs(30);
/// Connections to the socket that fail the token, after which the process is not waited for.
const WRONG_TOKENS: usize = 8;
/// How long a connection has to send its token.
const TOKEN_LIMIT: Duration = Duration::from_secs(2);
/// Bytes of randomness in a token.
const TOKEN_BYTES: usize = 32;
/// How often each direction looks up to see whether the other has ended.
const POLL: Duration = Duration::from_millis(50);
/// How long a line may wait on a peer that has stopped taking them.
const PEER_LIMIT: Duration = Duration::from_secs(30);

/// A provider process that has started, and everything it started; [`Running::stop`] ends them.
pub(crate) struct Running {
    child: Mutex<Child>,
    #[cfg(windows)]
    job: Mutex<Option<win32job::Job>>,
    stopped: AtomicBool,
}

impl Running {
    /// Ends the process and what it started, by job object on Windows and by process group elsewhere, and collects it.
    pub(crate) fn stop(&self) {
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        if !self.stopped.swap(true, Ordering::SeqCst) {
            #[cfg(windows)]
            drop(
                self.job
                    .lock()
                    .unwrap_or_else(PoisonError::into_inner)
                    .take(),
            );
            // Before it is collected: until then its id, and its group's, cannot be anyone else's.
            #[cfg(unix)]
            if let Some(group) = i32::try_from(child.id())
                .ok()
                .filter(|pid| *pid > 1)
                .and_then(rustix::process::Pid::from_raw)
            {
                let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
            }
        }
        let _ = child.kill();
        let _ = child.wait();
    }

    fn ended(&self) -> bool {
        !matches!(
            self.child
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_wait(),
            Ok(None)
        )
    }
}

/// The socket a provider process will connect to, its token, and the command that starts it.
pub(crate) struct Prepared {
    listener: TcpListener,
    token: String,
    command: Command,
}

/// The standard streams of a provider process that has started.
pub(crate) struct Streams {
    stdin: ChildStdin,
    stdout: ChildStdout,
    stderr: ChildStderr,
}

/// A socket on loopback and a token for one connection's provider process, which `program args` will run as.
pub(crate) fn prepare(program: &OsString, args: &[OsString]) -> io::Result<Prepared> {
    let listener = TcpListener::bind(SocketAddr::from((Ipv4Addr::LOCALHOST, 0)))?;
    let address = listener.local_addr()?;
    let token = token()?;
    let mut command = Command::new(program);
    command
        .args(args)
        .arg("--connect")
        .arg(address.to_string())
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    // Its own group, so that stopping it reaches what it starts.
    #[cfg(unix)]
    std::os::unix::process::CommandExt::process_group(&mut command, 0);
    Ok(Prepared {
        listener,
        token,
        command,
    })
}

impl Prepared {
    /// Starts the process. On Windows it is in a job object before this returns, and so before it has its settings and
    /// can load anything that starts more.
    pub(crate) fn start(&mut self) -> io::Result<(Running, Streams)> {
        let mut child = self.command.spawn()?;
        let streams = match (child.stdin.take(), child.stdout.take(), child.stderr.take()) {
            (Some(stdin), Some(stdout), Some(stderr)) => Streams {
                stdin,
                stdout,
                stderr,
            },
            _ => {
                let _ = child.kill();
                let _ = child.wait();
                return Err(io::Error::other(
                    "the provider's standard streams are missing",
                ));
            }
        };
        #[cfg(windows)]
        let job = {
            use std::os::windows::io::AsRawHandle as _;
            let mut limits = win32job::ExtendedLimitInfo::new();
            limits.limit_kill_on_job_close();
            let job = win32job::Job::create_with_limit_info(&limits).and_then(|job| {
                job.assign_process(child.as_raw_handle() as isize)
                    .map(|()| job)
            });
            match job {
                Ok(job) => job,
                Err(error) => {
                    let _ = child.kill();
                    let _ = child.wait();
                    return Err(io::Error::other(error));
                }
            }
        };
        Ok((
            Running {
                child: Mutex::new(child),
                #[cfg(windows)]
                job: Mutex::new(Some(job)),
                stopped: AtomicBool::new(false),
            },
            streams,
        ))
    }
}

/// Serves one connection on `channel` with the process `running`, started from `prepared`, giving it the settings in
/// `line`, and says why it ended. The caller stops the process.
pub(crate) fn serve<C: Channel>(
    entry: &str,
    prepared: Prepared,
    running: &Running,
    streams: Streams,
    line: &HostLine,
    channel: &C,
) -> &'static str {
    let Streams {
        mut stdin,
        stdout,
        stderr,
    } = streams;
    let budget = Arc::new(AtomicUsize::new(OUTPUT_LIMIT));
    forward(entry, Box::new(stdout), Arc::clone(&budget));
    forward(entry, Box::new(stderr), budget);
    let mut line = line.clone();
    line.token = Some(prepared.token.clone());
    let mut bytes = line.to_line();
    bytes.push(b'\n');
    // The settings, then end of input: what the provider's code reads from standard input finds nothing.
    let given = stdin.write_all(&bytes).and_then(|()| stdin.flush());
    drop(stdin);
    if given.is_err() {
        return "the provider did not take its settings";
    }
    match accept(&prepared.listener, &prepared.token, running) {
        Some(stream) => {
            drop(prepared);
            relay(channel, &stream)
        }
        None => "the provider did not connect",
    }
}

/// A token of [`TOKEN_BYTES`] random bytes, as hexadecimal.
fn token() -> io::Result<String> {
    let mut bytes = [0_u8; TOKEN_BYTES];
    getrandom::fill(&mut bytes).map_err(|error| io::Error::other(error.to_string()))?;
    Ok(bytes.iter().map(|byte| format!("{byte:02x}")).collect())
}

/// Copies what the provider writes to this host's standard error until `budget` is spent, then reads and drops the rest,
/// so that a provider writing without end is never blocked by it.
fn forward(entry: &str, mut output: Box<dyn Read + Send>, budget: Arc<AtomicUsize>) {
    let entry = String::from(entry);
    let spawned = thread::Builder::new()
        .name("canoa-provider-output".to_owned())
        .spawn(move || {
            let mut buffer = vec![0_u8; 8192];
            loop {
                let read = match output.read(&mut buffer) {
                    Ok(0) | Err(_) => return,
                    Ok(read) => read,
                };
                let left = take(&budget, read);
                let kept = read.min(left);
                if kept > 0 {
                    let _ = std::io::stderr().lock().write_all(&buffer[..kept]);
                    if kept == left {
                        tracing::warn!(
                            entry,
                            "the provider wrote more than {OUTPUT_LIMIT} bytes; the rest is dropped"
                        );
                    }
                }
            }
        });
    if let Err(error) = spawned {
        tracing::warn!("the provider's output cannot be read: {error}");
    }
}

/// Takes up to `wanted` from `budget` and says what it held before.
fn take(budget: &AtomicUsize, wanted: usize) -> usize {
    let mut left = budget.load(Ordering::SeqCst);
    loop {
        let after = left.saturating_sub(wanted);
        match budget.compare_exchange_weak(left, after, Ordering::SeqCst, Ordering::SeqCst) {
            Ok(_) => return left,
            Err(now) => left = now,
        }
    }
}

/// The first connection that sends `token`, or `None` once the process has ended, the time is up, or too many failed.
fn accept(listener: &TcpListener, token: &str, running: &Running) -> Option<TcpStream> {
    listener.set_nonblocking(true).ok()?;
    let until = Instant::now() + START_LIMIT;
    let mut wrong = 0;
    while Instant::now() < until && wrong < WRONG_TOKENS {
        match listener.accept() {
            Ok((stream, _)) => {
                if proves(&stream, token) {
                    return Some(stream);
                }
                wrong += 1;
            }
            Err(error) if error.kind() == ErrorKind::WouldBlock => {
                if running.ended() {
                    return None;
                }
                thread::sleep(Duration::from_millis(10));
            }
            Err(_) => wrong += 1,
        }
    }
    None
}

/// Whether `stream` sends `token` and a line break within [`TOKEN_LIMIT`].
fn proves(stream: &TcpStream, token: &str) -> bool {
    if stream.set_nonblocking(false).is_err() || stream.set_read_timeout(Some(TOKEN_LIMIT)).is_err()
    {
        return false;
    }
    let mut sent = Vec::new();
    let limit = u64::try_from(token.len() + 1).unwrap_or(u64::MAX);
    if BufReader::new(stream.take(limit))
        .read_until(b'\n', &mut sent)
        .is_err()
        || sent.pop() != Some(b'\n')
        || sent.len() != token.len()
    {
        return false;
    }
    // Compared in time that does not depend on where the texts differ.
    sent.iter()
        .zip(token.as_bytes())
        .fold(0_u8, |differ, (a, b)| differ | (a ^ b))
        == 0
}

/// Passes lines between `channel` and the provider's socket until either ends, and says which.
fn relay<C: Channel>(channel: &C, stream: &TcpStream) -> &'static str {
    if stream.set_read_timeout(Some(POLL)).is_err()
        || stream.set_write_timeout(Some(PEER_LIMIT)).is_err()
    {
        return "the provider's socket could not be set up";
    }
    let _ = stream.set_nodelay(true);
    let stop = AtomicBool::new(false);
    thread::scope(|scope| {
        let inbound = scope.spawn(|| {
            let end = to_provider(channel, stream, &stop);
            stop.store(true, Ordering::SeqCst);
            let _ = stream.shutdown(Shutdown::Both);
            end
        });
        let outbound = from_provider(channel, stream, &stop);
        stop.store(true, Ordering::SeqCst);
        let _ = stream.shutdown(Shutdown::Both);
        let inbound = inbound.join().unwrap_or("the relay failed");
        outbound.unwrap_or(inbound)
    })
}

fn to_provider<C: Channel>(channel: &C, mut stream: &TcpStream, stop: &AtomicBool) -> &'static str {
    while !stop.load(Ordering::SeqCst) {
        match channel.receive(Some(POLL)) {
            Ok(Some(mut message)) => {
                // One message is one line to the provider: a raw line break inside would forge a second one.
                if message.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
                    return "a client message held a line break";
                }
                message.push(b'\n');
                if stream.write_all(&message).is_err() {
                    return "the provider stopped reading";
                }
            }
            Ok(None) => return "the client closed",
            Err(Failure::Timeout) => {}
            Err(_) => return "the client's link failed",
        }
    }
    "the provider ended"
}

/// `None` when the other direction ended first.
fn from_provider<C: Channel>(
    channel: &C,
    stream: &TcpStream,
    stop: &AtomicBool,
) -> Option<&'static str> {
    let mut reader = BufReader::new(stream);
    let mut line = Vec::new();
    while !stop.load(Ordering::SeqCst) {
        let buffer = match reader.fill_buf() {
            Ok([]) => return Some("the provider ended"),
            Ok(buffer) => buffer,
            Err(error) if matches!(error.kind(), ErrorKind::WouldBlock | ErrorKind::TimedOut) => {
                continue;
            }
            // A process that ends with the socket open resets it on some systems.
            Err(error)
                if matches!(
                    error.kind(),
                    ErrorKind::ConnectionReset | ErrorKind::ConnectionAborted
                ) =>
            {
                return Some("the provider ended");
            }
            Err(_) => return Some("the provider's socket failed"),
        };
        let (taken, complete) = match buffer.iter().position(|byte| *byte == b'\n') {
            Some(at) => (at + 1, true),
            None => (buffer.len(), false),
        };
        line.extend_from_slice(&buffer[..taken]);
        reader.consume(taken);
        if line.len() > MAX_LINE + 2 {
            return Some("the provider sent a line over the limit");
        }
        if complete {
            while matches!(line.last(), Some(b'\n' | b'\r')) {
                line.pop();
            }
            if !line.is_empty() && !deliver(channel, &line, stop) {
                return Some("the client stopped taking messages");
            }
            line.clear();
        }
    }
    None
}

/// Sends one line, waiting on a full queue, so the provider feels a slow client as back-pressure.
fn deliver<C: Channel>(channel: &C, line: &[u8], stop: &AtomicBool) -> bool {
    let until = Instant::now() + PEER_LIMIT;
    loop {
        match channel.send(line) {
            Ok(()) => return true,
            Err(Failure::NoProgress) if Instant::now() < until && !stop.load(Ordering::SeqCst) => {
                thread::sleep(POLL);
            }
            Err(_) => return false,
        }
    }
}
