//! An ACP agent served in this process over one connection's channel: each message the client sends is one line of the
//! agent's input, each line the agent writes one message to the client. Nothing runs as a child of its own here; what the
//! agent starts is its own to end, and it ends it when its input does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, channels and streams are std's"
)]

use std::io::{self, Read, Write};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use coqui_acp_harness::{Harness as Translator, Settings};
use coqui_web::{Channel, Failure};

/// How often each direction looks up to see whether the other has ended.
const POLL: Duration = Duration::from_millis(50);
/// How long a message may wait on a full queue to the client, after which the client has stopped taking them.
const PEER_LIMIT: Duration = Duration::from_secs(30);
/// Lines of the agent's waiting for the client; past it the agent waits too.
const QUEUED: usize = 256;
/// How long what the agent says after its input ended may still reach the client.
const LAST_WORDS: Duration = Duration::from_secs(1);
/// How long [`InProcess::stop`] waits for the agent to have closed its sessions.
const STOP_WAIT: Duration = Duration::from_secs(10);

/// Why a connection to an agent in this process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// The client closed in order.
    PeerClosed,
    /// The channel failed or the client stopped taking messages.
    Failed,
    /// A message held a line break, or a line of the agent's was over the limit.
    Protocol,
    /// The host stopped it.
    Stopped,
    /// The agent's input was refused by the agent itself.
    AgentFailed,
}

/// One agent running in this process, which the host can stop from another thread.
#[derive(Debug, Default)]
pub(crate) struct InProcess {
    stop: AtomicBool,
    done: Mutex<bool>,
    ended: Condvar,
}

impl InProcess {
    pub(crate) fn new() -> Arc<Self> {
        Arc::new(Self::default())
    }

    /// Ends its input, so it closes every session and what each started; returns once it has, or after [`STOP_WAIT`].
    pub(crate) fn stop(&self) {
        self.stop.store(true, Ordering::SeqCst);
        let done = self.done.lock().unwrap_or_else(PoisonError::into_inner);
        let _ = self
            .ended
            .wait_timeout_while(done, STOP_WAIT, |done| !*done)
            .unwrap_or_else(PoisonError::into_inner);
    }

    fn stopping(&self) -> bool {
        self.stop.load(Ordering::SeqCst)
    }

    fn finish(&self) {
        *self.done.lock().unwrap_or_else(PoisonError::into_inner) = true;
        self.ended.notify_all();
    }
}

/// The client's messages as the agent's input, one line each, ending when the client or the host does.
struct FromClient<'a, C: Channel + ?Sized> {
    channel: &'a C,
    control: &'a InProcess,
    /// Set once the other direction has given up, so that reading stops too.
    gone: &'a AtomicBool,
    line: Vec<u8>,
    at: usize,
    end: Option<End>,
}

impl<C: Channel + ?Sized> FromClient<'_, C> {
    /// The next line, or `false` at the end.
    fn fill(&mut self) -> bool {
        while self.end.is_none() {
            if self.control.stopping() {
                self.end = Some(End::Stopped);
                break;
            }
            if self.gone.load(Ordering::SeqCst) {
                self.end = Some(End::Failed);
                break;
            }
            match self.channel.receive(Some(POLL)) {
                Ok(Some(message)) => {
                    // One message is one line to the agent: a line break inside would forge a second.
                    if message.iter().any(|byte| matches!(byte, b'\n' | b'\r')) {
                        self.end = Some(End::Protocol);
                        break;
                    }
                    self.line = message;
                    self.line.push(b'\n');
                    self.at = 0;
                    return true;
                }
                Ok(None) => self.end = Some(End::PeerClosed),
                // A refused message reads as a timeout: nothing reached the agent.
                Err(Failure::Timeout) => {}
                Err(_) => self.end = Some(End::Failed),
            }
        }
        false
    }
}

impl<C: Channel + ?Sized> Read for FromClient<'_, C> {
    fn read(&mut self, buffer: &mut [u8]) -> io::Result<usize> {
        if self.at == self.line.len() && !self.fill() {
            return Ok(0);
        }
        let count = buffer.len().min(self.line.len() - self.at);
        buffer[..count].copy_from_slice(&self.line[self.at..self.at + count]);
        self.at += count;
        Ok(count)
    }
}

/// The agent's output, cut into lines for the client.
struct ToClient {
    lines: SyncSender<Vec<u8>>,
    partial: Vec<u8>,
    max_line: usize,
}

impl Write for ToClient {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.partial.extend_from_slice(bytes);
        while let Some(at) = self.partial.iter().position(|byte| *byte == b'\n') {
            let mut line: Vec<u8> = self.partial.drain(..=at).collect();
            line.pop();
            if line.last() == Some(&b'\r') {
                line.pop();
            }
            if line.is_empty() {
                continue;
            }
            self.lines
                .send(line)
                .map_err(|_| io::Error::from(io::ErrorKind::BrokenPipe))?;
        }
        if self.partial.len() > self.max_line {
            return Err(io::Error::new(
                io::ErrorKind::InvalidData,
                "a line over the limit",
            ));
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Sends one line, waiting on a full queue, so the agent feels a slow client as back-pressure.
fn deliver<C: Channel + ?Sized>(channel: &C, line: &[u8]) -> Result<(), End> {
    let until = Instant::now() + PEER_LIMIT;
    loop {
        match channel.send(line) {
            Ok(()) => return Ok(()),
            Err(Failure::Protocol) => return Err(End::Protocol),
            Err(Failure::NoProgress) if Instant::now() < until => thread::sleep(POLL),
            Err(_) => return Err(End::Failed),
        }
    }
}

/// Serves `agent` on `channel` until the client closes, the channel fails or `control` is stopped; then the agent's
/// sessions are closed, and with them everything they started.
pub(crate) fn serve<C: Channel + ?Sized>(
    channel: &C,
    agent: Arc<dyn Translator>,
    settings: Settings,
    max_line: usize,
    control: &InProcess,
) -> End {
    let (lines, queued) = mpsc::sync_channel::<Vec<u8>>(QUEUED);
    let gone = AtomicBool::new(false);
    let finished = AtomicBool::new(false);
    let end = thread::scope(|scope| {
        let gone = &gone;
        let finished = &finished;
        let outbound = scope.spawn(move || {
            let mut last = None;
            loop {
                match queued.recv_timeout(POLL) {
                    Ok(line) => {
                        if let Err(end) = deliver(channel, &line) {
                            gone.store(true, Ordering::SeqCst);
                            return Some(end);
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return None,
                    Err(RecvTimeoutError::Timeout) if finished.load(Ordering::SeqCst) => {
                        // The agent's threads may still say a last word after its input ended.
                        let since = *last.get_or_insert_with(Instant::now);
                        if since.elapsed() >= LAST_WORDS {
                            return None;
                        }
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        });
        let mut input = FromClient {
            channel,
            control,
            gone,
            line: Vec::new(),
            at: 0,
            end: None,
        };
        let output = ToClient {
            lines,
            partial: Vec::new(),
            max_line,
        };
        let served = coqui_acp_harness::serve(agent, settings, &mut input, output);
        finished.store(true, Ordering::SeqCst);
        let outbound = outbound.join().unwrap_or(Some(End::Failed));
        match (served, input.end, outbound) {
            (Err(_), _, _) => End::AgentFailed,
            (Ok(()), _, Some(end)) => end,
            (Ok(()), Some(end), None) => end,
            (Ok(()), None, None) => End::PeerClosed,
        }
    });
    control.finish();
    end
}

#[cfg(test)]
mod tests;
