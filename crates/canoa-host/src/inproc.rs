//! An `acp_harness` provider served in this process over one connection's channel: each message the client sends goes to
//! the provider's channel, each message the provider hands the host's sink goes to the client. Nothing runs as a child of its
//! own here; the provider starts its program through the spawner, and kills it when the connection closes.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads, channels and streams are std's"
)]

use std::panic::{AssertUnwindSafe, catch_unwind};
use std::sync::atomic::{AtomicBool, Ordering};
use std::sync::mpsc::{self, RecvTimeoutError, SyncSender, TrySendError};
use std::sync::{Arc, Condvar, Mutex, PoisonError};
use std::thread;
use std::time::{Duration, Instant};

use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::{
    AccountKeeper, AccountLoss, AccountRenewal, AcpHarness, ChannelSink, ProcessSpawner,
    StartRequest,
};
use coqui_acp_harness::LocalSpawner;
use coqui_web_channel::{Channel, Failure};

/// How often each direction looks up to see whether the other has ended.
const POLL: Duration = Duration::from_millis(50);
/// How long a message may wait on a full queue to the client, after which the client has stopped taking them.
const PEER_LIMIT: Duration = Duration::from_secs(30);
/// Messages of the provider's waiting for the client; past it the provider waits too.
const QUEUED: usize = 256;
/// How long a connection waits for the provider to say it is closed, which is after it has closed its sessions.
const STOP_WAIT: Duration = Duration::from_secs(10);

/// Why a connection to a provider in this process ended.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub(crate) enum End {
    /// The client closed in order.
    PeerClosed,
    /// The channel failed or the client stopped taking messages.
    Failed,
    /// A message of the provider's was over the limit.
    Protocol,
    /// The host stopped it.
    Stopped,
    /// The provider ended the connection with a failure, or could not start one.
    AgentFailed,
}

/// One connection running in this process, which the host can stop from another thread.
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

    /// Ends the connection, so the provider closes every session and what each started, without waiting for that.
    pub(crate) fn signal(&self) {
        self.stop.store(true, Ordering::SeqCst);
    }

    /// As [`InProcess::signal`], then returns once the provider has closed them, or after [`STOP_WAIT`].
    pub(crate) fn stop(&self) {
        self.signal();
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

/// Sends one line, waiting on a full queue, so the agent feels a slow client as back-pressure; a host that is stopping
/// waits for no client.
fn deliver<C: Channel + ?Sized>(channel: &C, line: &[u8], control: &InProcess) -> Result<(), End> {
    let until = Instant::now() + PEER_LIMIT;
    loop {
        match channel.send(line) {
            Ok(()) => return Ok(()),
            Err(Failure::Protocol) => return Err(End::Protocol),
            Err(Failure::NoProgress) if control.stopping() => return Err(End::Stopped),
            Err(Failure::NoProgress) if Instant::now() < until => thread::sleep(POLL),
            Err(_) => return Err(End::Failed),
        }
    }
}

/// What the provider has said about its end of a connection, by `closed`.
#[derive(Default)]
struct Ending {
    closed: Mutex<Option<(u32, String)>>,
    changed: Condvar,
}

impl Ending {
    fn said(&self) -> Option<(u32, String)> {
        self.closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    /// Waits up to `limit` for the provider's `closed`.
    fn wait(&self, limit: Duration) -> Option<(u32, String)> {
        let held = self.closed.lock().unwrap_or_else(PoisonError::into_inner);
        let (held, _) = self
            .changed
            .wait_timeout_while(held, limit, |closed| closed.is_none())
            .unwrap_or_else(PoisonError::into_inner);
        held.clone()
    }
}

/// The provider's messages, queued for the client. A full queue holds the provider back until the client takes more.
struct ToClientSink {
    lines: Option<SyncSender<Vec<u8>>>,
    gone: Arc<AtomicBool>,
    oversize: Arc<AtomicBool>,
    ending: Arc<Ending>,
    max_line: usize,
}

fn kind_gone(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_GONE, why)
}

impl ChannelSink for ToClientSink {
    fn receive(&mut self, bytes: &[u8]) -> Result<(), ProviderError> {
        if bytes.len() > self.max_line {
            self.oversize.store(true, Ordering::SeqCst);
            return Err(ProviderError::new(
                Status::GUATIAO_ERR_BAD_VALUE,
                "a message over the limit",
            ));
        }
        let Some(lines) = &self.lines else {
            return Err(kind_gone("the connection is closed"));
        };
        let mut line = bytes.to_vec();
        loop {
            match lines.try_send(line) {
                Ok(()) => return Ok(()),
                Err(TrySendError::Full(back)) => {
                    if self.gone.load(Ordering::SeqCst) {
                        return Err(kind_gone("the client is gone"));
                    }
                    line = back;
                    thread::sleep(Duration::from_millis(2));
                }
                Err(TrySendError::Disconnected(_)) => return Err(kind_gone("the client is gone")),
            }
        }
    }

    fn closed(&mut self, status: u32, reason: &str) {
        // The queue ends once what it holds is delivered.
        self.lines = None;
        *self
            .ending
            .closed
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some((status, reason.to_owned()));
        self.ending.changed.notify_all();
    }
}

/// The host holds no account for a connection, so there is nothing to renew or lose.
struct NoAccounts;

impl AccountKeeper for NoAccounts {
    fn renewed(&self, _renewal: AccountRenewal) -> Result<(), ProviderError> {
        Ok(())
    }

    fn lost(&self, _loss: AccountLoss) -> Result<(), ProviderError> {
        Ok(())
    }
}

/// Serves one connection with an `acp_harness` provider: each message the client sends goes to the provider's channel,
/// each message the provider's sink gets goes to the client, until the client closes, the channel fails, the provider ends
/// the connection or `control` is stopped. The provider starts its program through this machine's spawner, and kills what
/// it started when the connection ends; this returns once it has said so, or after [`STOP_WAIT`].
pub(crate) fn serve_kind<C: Channel + ?Sized>(
    channel: &C,
    harness: &dyn AcpHarness,
    request: StartRequest,
    max_line: usize,
    control: &InProcess,
) -> End {
    let (lines, queued) = mpsc::sync_channel::<Vec<u8>>(QUEUED);
    let gone = Arc::new(AtomicBool::new(false));
    let oversize = Arc::new(AtomicBool::new(false));
    let ending = Arc::new(Ending::default());
    let sink = ToClientSink {
        lines: Some(lines),
        gone: Arc::clone(&gone),
        oversize: Arc::clone(&oversize),
        ending: Arc::clone(&ending),
        max_line,
    }
    .into_object();
    let started = harness.start(
        request,
        LocalSpawner::new().into_object(),
        sink,
        NoAccounts.into_object(),
    );
    let provider = match started {
        Ok(provider) => provider,
        Err(error) => {
            tracing::warn!(
                "the harness provider did not start a connection: {}",
                error.message()
            );
            control.finish();
            return End::AgentFailed;
        }
    };
    let finished = AtomicBool::new(false);
    let end = thread::scope(|scope| {
        let (gone, finished) = (&*gone, &finished);
        let outbound = scope.spawn(move || {
            loop {
                match queued.recv_timeout(POLL) {
                    Ok(line) => {
                        if let Err(end) = deliver(channel, &line, control) {
                            gone.store(true, Ordering::SeqCst);
                            return Some(end);
                        }
                    }
                    Err(RecvTimeoutError::Disconnected) => return None,
                    Err(RecvTimeoutError::Timeout) if finished.load(Ordering::SeqCst) => {
                        return None;
                    }
                    Err(RecvTimeoutError::Timeout) => {}
                }
            }
        });
        // Anything on the thread that reads the client may panic; the connection then ends as any other, rather than leaving
        // the delivering thread and the provider waiting.
        let (host_end, close_with) = catch_unwind(AssertUnwindSafe(|| {
            loop {
                if control.stopping() {
                    break (End::Stopped, Some(Status::GUATIAO_OK));
                }
                if gone.load(Ordering::SeqCst) {
                    break (End::Failed, Some(Status::GUATIAO_ERR_GONE));
                }
                if ending.said().is_some() {
                    break (End::AgentFailed, None);
                }
                match channel.receive(Some(POLL)) {
                    Ok(Some(message)) => {
                        if provider.send(&message).is_err() {
                            // It ended the connection, or refused the message: what it says is in `closed`.
                            break (End::AgentFailed, None);
                        }
                    }
                    Ok(None) => break (End::PeerClosed, Some(Status::GUATIAO_OK)),
                    Err(Failure::Timeout) => {}
                    Err(_) => break (End::Failed, Some(Status::GUATIAO_ERR_GONE)),
                }
            }
        }))
        .unwrap_or_else(|panic| {
            core::mem::forget(panic);
            (End::Failed, Some(Status::GUATIAO_ERR_GONE))
        });
        if let Some(status) = close_with {
            provider.close(u32::from(status), "");
        }
        let said = ending.wait(STOP_WAIT);
        finished.store(true, Ordering::SeqCst);
        let delivered = outbound.join().unwrap_or(Some(End::Failed));
        // A provider that ended the connection by itself says how in `closed`; one the host closed echoes the host's.
        match (host_end, said, delivered) {
            (End::AgentFailed, Some((status, reason)), _) => {
                if status == u32::from(Status::GUATIAO_OK) {
                    End::PeerClosed
                } else {
                    tracing::info!(
                        "the harness provider ended the connection with status {status}: {reason}"
                    );
                    if oversize.load(Ordering::SeqCst) {
                        End::Protocol
                    } else {
                        End::AgentFailed
                    }
                }
            }
            (End::AgentFailed, None, _) => End::AgentFailed,
            (_, _, Some(end)) => end,
            (end, _, None) => end,
        }
    });
    drop(provider);
    control.finish();
    end
}

#[cfg(test)]
mod tests;
