//! Prevents the computer from idle-sleeping while an agent turn runs, when the Power setting `keep_awake` is on.
//!
//! Each in-flight `session/prompt` request holds one [`TurnGuard`]; transitioning from zero to one active turns acquires the
//! platform sleep inhibition, and transitioning back to zero releases it. Dropping the guard on response, cancel, error, or
//! disconnect guarantees no inhibition is leaked. The hold lasts exactly as long as a turn does, however long that is, and the
//! operating system lets it go when this process ends, however it ends.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "platform handles, locks and threads are std's"
)]

use std::sync::{Arc, Mutex, PoisonError};
use std::time::Duration;

use coqui_transport::{Channel, Failure};
use serde_json::Value;

/// A platform inhibition token kept alive until dropped.
pub(crate) trait Held: Send {}

/// Platform backend that acquires an idle-sleep inhibition handle while at least one turn is active.
pub(crate) trait Inhibitor: Send + Sync {
    fn acquire(&self) -> Result<Box<dyn Held>, String>;
}

#[derive(Default)]
struct State {
    active: usize,
    held: Option<Box<dyn Held>>,
}

/// Reference-counted manager that keeps the host from idle-sleeping while any agent turn is running.
pub(crate) struct KeepAwake {
    inhibitor: Option<Arc<dyn Inhibitor>>,
    state: Mutex<State>,
}

impl core::fmt::Debug for KeepAwake {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        let active = self.state.lock().map_or(0, |state| state.active);
        f.debug_struct("KeepAwake")
            .field("enabled", &self.inhibitor.is_some())
            .field("active_turns", &active)
            .finish()
    }
}

impl KeepAwake {
    /// No-op keep-awake manager when the setting is off.
    #[must_use]
    pub(crate) fn disabled() -> Arc<Self> {
        Arc::new(Self {
            inhibitor: None,
            state: Mutex::new(State::default()),
        })
    }

    /// Manager using the host platform's idle-sleep inhibitor when `enabled` is true, or [`Self::disabled`] otherwise.
    #[must_use]
    pub(crate) fn new(enabled: bool) -> Arc<Self> {
        if !enabled {
            return Self::disabled();
        }
        Self::with_inhibitor(platform_inhibitor())
    }

    /// Enabled manager backed by a custom [`Inhibitor`], used in tests.
    pub(crate) fn with_inhibitor(inhibitor: Arc<dyn Inhibitor>) -> Arc<Self> {
        Arc::new(Self {
            inhibitor: Some(inhibitor),
            state: Mutex::new(State::default()),
        })
    }

    /// Acquires one active-turn hold; releasing the returned [`TurnGuard`] decrements the refcount and releases the OS hold once
    /// the last concurrent turn ends.
    #[must_use]
    pub(crate) fn turn(self: &Arc<Self>) -> TurnGuard {
        let Some(inhibitor) = &self.inhibitor else {
            return TurnGuard { manager: None };
        };
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active = state.active.saturating_add(1);
        if state.active == 1 && state.held.is_none() {
            match inhibitor.acquire() {
                Ok(held) => state.held = Some(held),
                Err(why) => tracing::debug!("keep_awake could not inhibit idle sleep: {why}"),
            }
        }
        drop(state);
        TurnGuard {
            manager: Some(Arc::clone(self)),
        }
    }

    /// Number of currently active turns; used in tests.
    #[cfg(test)]
    pub(crate) fn active_turns(&self) -> usize {
        self.state
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .active
    }

    fn release(&self) {
        let mut state = self.state.lock().unwrap_or_else(PoisonError::into_inner);
        state.active = state.active.saturating_sub(1);
        let dropped = if state.active == 0 {
            state.held.take()
        } else {
            None
        };
        drop(state);
        drop(dropped);
    }
}

/// Guard held for the lifetime of a single active `session/prompt` request.
#[derive(Debug)]
pub(crate) struct TurnGuard {
    manager: Option<Arc<KeepAwake>>,
}

impl Drop for TurnGuard {
    fn drop(&mut self) {
        if let Some(manager) = self.manager.take() {
            manager.release();
        }
    }
}

/// [`Channel`] wrapper watching ACP JSON-RPC traffic on one connection for `session/prompt` requests (entering via
/// [`Channel::receive`] from the client) and their JSON-RPC responses (leaving via [`Channel::send`] to the client).
pub(crate) struct AwakeChannel<'a, C: ?Sized> {
    inner: &'a C,
    manager: Arc<KeepAwake>,
    in_flight: Mutex<Vec<(Value, TurnGuard)>>,
}

impl<'a, C: ?Sized> AwakeChannel<'a, C> {
    pub(crate) fn new(inner: &'a C, manager: Arc<KeepAwake>) -> Self {
        Self {
            inner,
            manager,
            in_flight: Mutex::new(Vec::new()),
        }
    }

    fn track_prompt(&self, message: &[u8]) {
        if self.manager.inhibitor.is_none() {
            return;
        }
        let Some(id) = prompt_request_id(message) else {
            return;
        };
        let mut active = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        active.push((id, self.manager.turn()));
    }

    fn finish_prompt(&self, message: &[u8]) {
        if self.manager.inhibitor.is_none() {
            return;
        }
        let Some(id) = response_id(message) else {
            return;
        };
        let mut active = self
            .in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        if let Some(pos) = active.iter().position(|(pending, _)| *pending == id) {
            active.swap_remove(pos);
        }
    }
}

impl<C: Channel + ?Sized> Channel for AwakeChannel<'_, C> {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        let sent = self.inner.send(message);
        self.finish_prompt(message);
        sent
    }

    fn receive(&self, timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        let received = self.inner.receive(timeout)?;
        if let Some(message) = &received {
            self.track_prompt(message);
        }
        Ok(received)
    }

    fn close(&self) {
        self.in_flight
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clear();
        self.inner.close();
    }

    fn set_max_message(&self, bytes: usize) {
        self.inner.set_max_message(bytes);
    }
}

fn prompt_request_id(message: &[u8]) -> Option<Value> {
    let parsed: Value = serde_json::from_slice(message).ok()?;
    if parsed.get("method").and_then(Value::as_str) != Some("session/prompt") {
        return None;
    }
    parsed.get("id").filter(|id| !id.is_null()).cloned()
}

fn response_id(message: &[u8]) -> Option<Value> {
    let parsed: Value = serde_json::from_slice(message).ok()?;
    if parsed.get("method").is_some() {
        return None;
    }
    if parsed.get("result").is_none() && parsed.get("error").is_none() {
        return None;
    }
    parsed.get("id").filter(|id| !id.is_null()).cloned()
}

fn platform_inhibitor() -> Arc<dyn Inhibitor> {
    #[cfg(windows)]
    {
        Arc::new(win32::WinInhibitor)
    }
    #[cfg(target_os = "macos")]
    {
        Arc::new(unix::ChildInhibitor::caffeinate())
    }
    #[cfg(all(unix, not(target_os = "macos")))]
    {
        Arc::new(unix::ChildInhibitor::systemd())
    }
    #[cfg(not(any(windows, unix)))]
    {
        Arc::new(NoopInhibitor)
    }
}

#[cfg(not(any(windows, unix)))]
struct NoopInhibitor;

#[cfg(not(any(windows, unix)))]
struct NoopHeld;

#[cfg(not(any(windows, unix)))]
impl Held for NoopHeld {}

#[cfg(not(any(windows, unix)))]
impl Inhibitor for NoopInhibitor {
    fn acquire(&self) -> Result<Box<dyn Held>, String> {
        Ok(Box::new(NoopHeld))
    }
}

#[cfg(unix)]
mod unix {
    use crate::spawn::{Child, Command, Stdio};

    use super::{Held, Inhibitor};

    pub(super) struct ChildInhibitor {
        kind: Kind,
    }

    #[derive(Clone, Copy)]
    enum Kind {
        #[cfg(target_os = "macos")]
        Caffeinate,
        #[cfg(not(target_os = "macos"))]
        Systemd,
    }

    impl ChildInhibitor {
        #[cfg(target_os = "macos")]
        pub(super) fn caffeinate() -> Self {
            Self {
                kind: Kind::Caffeinate,
            }
        }

        #[cfg(not(target_os = "macos"))]
        pub(super) fn systemd() -> Self {
            Self {
                kind: Kind::Systemd,
            }
        }
    }

    /// The inhibitor's process. On Linux it runs `cat` on a pipe this process holds the write end of, so the hold ends when the pipe
    /// closes: when the turn does, and when this process ends however it ends, with nothing left running. macOS's `caffeinate` is
    /// tied to this process by `-w`.
    struct HeldChild {
        child: Child,
    }

    impl Drop for HeldChild {
        fn drop(&mut self) {
            drop(self.child.stdin.take());
            let _ = self.child.kill();
            let _ = self.child.wait();
        }
    }

    impl Held for HeldChild {}

    impl Inhibitor for ChildInhibitor {
        fn acquire(&self) -> Result<Box<dyn Held>, String> {
            let mut command = match self.kind {
                #[cfg(target_os = "macos")]
                Kind::Caffeinate => {
                    let mut cmd = Command::new("caffeinate");
                    let host = std::process::id().to_string();
                    cmd.args(["-i", "-w", host.as_str()]);
                    cmd
                }
                #[cfg(not(target_os = "macos"))]
                Kind::Systemd => {
                    let mut cmd = Command::new("systemd-inhibit");
                    cmd.args([
                        "--what=idle",
                        "--who=Canoa",
                        "--why=Agent turn running",
                        "--mode=block",
                        "cat",
                    ]);
                    cmd
                }
            };
            command
                .stdin(match self.kind {
                    #[cfg(target_os = "macos")]
                    Kind::Caffeinate => Stdio::Null,
                    #[cfg(not(target_os = "macos"))]
                    Kind::Systemd => Stdio::Piped,
                })
                .stdout(Stdio::Null)
                .stderr(Stdio::Null);
            let child = command.spawn().map_err(|error| error.to_string())?;
            Ok(Box::new(HeldChild { child }))
        }
    }
}

#[cfg(windows)]
#[allow(
    unsafe_code,
    reason = "SetThreadExecutionState has no safe form in std and runs on a dedicated thread so clear matches set"
)]
mod win32 {
    use std::sync::mpsc::{self, Sender};
    use std::thread::JoinHandle;

    use windows_sys::Win32::System::Power::{
        ES_CONTINUOUS, ES_SYSTEM_REQUIRED, SetThreadExecutionState,
    };

    use super::{Held, Inhibitor};

    pub(super) struct WinInhibitor;

    struct WinHeld {
        stop: Option<Sender<()>>,
        worker: Option<JoinHandle<()>>,
    }

    impl Held for WinHeld {}

    impl Drop for WinHeld {
        fn drop(&mut self) {
            if let Some(stop) = self.stop.take() {
                let _ = stop.send(());
            }
            if let Some(worker) = self.worker.take() {
                let _ = worker.join();
            }
        }
    }

    impl Inhibitor for WinInhibitor {
        fn acquire(&self) -> Result<Box<dyn Held>, String> {
            let (ready_tx, ready_rx) = mpsc::channel::<Result<(), String>>();
            let (stop_tx, stop_rx) = mpsc::channel::<()>();
            let worker = std::thread::Builder::new()
                .name("canoa-keep-awake".to_owned())
                .spawn(move || {
                    // SAFETY: SetThreadExecutionState takes only bitflags and alters only this dedicated thread's execution state.
                    let prev =
                        unsafe { SetThreadExecutionState(ES_CONTINUOUS | ES_SYSTEM_REQUIRED) };
                    if prev == 0 {
                        let _ = ready_tx.send(Err("SetThreadExecutionState failed".to_owned()));
                        return;
                    }
                    let _ = ready_tx.send(Ok(()));
                    let _ = stop_rx.recv();
                    // SAFETY: clearing ES_SYSTEM_REQUIRED on the same thread that set it restores normal idle sleep.
                    unsafe {
                        SetThreadExecutionState(ES_CONTINUOUS);
                    }
                })
                .map_err(|error| error.to_string())?;
            match ready_rx.recv() {
                Ok(Ok(())) => Ok(Box::new(WinHeld {
                    stop: Some(stop_tx),
                    worker: Some(worker),
                })),
                Ok(Err(why)) => {
                    let _ = worker.join();
                    Err(why)
                }
                Err(_) => {
                    let _ = worker.join();
                    Err("keep-awake worker stopped before setting execution state".to_owned())
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use alloc::collections::VecDeque;
    use core::sync::atomic::{AtomicUsize, Ordering};

    use super::*;

    struct TestChannel {
        inbound: Mutex<VecDeque<Vec<u8>>>,
        outbound: Mutex<Vec<Vec<u8>>>,
    }

    impl TestChannel {
        fn new(inbound: Vec<&[u8]>) -> Self {
            Self {
                inbound: Mutex::new(inbound.into_iter().map(<[u8]>::to_vec).collect()),
                outbound: Mutex::new(Vec::new()),
            }
        }
    }

    impl Channel for TestChannel {
        fn send(&self, message: &[u8]) -> Result<(), Failure> {
            self.outbound
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .push(message.to_vec());
            Ok(())
        }

        fn receive(&self, _timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
            Ok(self
                .inbound
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .pop_front())
        }

        fn close(&self) {}

        fn set_max_message(&self, _bytes: usize) {}
    }

    struct FakeInhibitor {
        acquires: AtomicUsize,
        releases: Arc<AtomicUsize>,
    }

    impl FakeInhibitor {
        fn new() -> Arc<Self> {
            Arc::new(Self {
                acquires: AtomicUsize::new(0),
                releases: Arc::new(AtomicUsize::new(0)),
            })
        }
    }

    struct FakeHeld(Arc<AtomicUsize>);

    impl Held for FakeHeld {}

    impl Drop for FakeHeld {
        fn drop(&mut self) {
            self.0.fetch_add(1, Ordering::SeqCst);
        }
    }

    impl Inhibitor for FakeInhibitor {
        fn acquire(&self) -> Result<Box<dyn Held>, String> {
            self.acquires.fetch_add(1, Ordering::SeqCst);
            Ok(Box::new(FakeHeld(Arc::clone(&self.releases))))
        }
    }

    #[test]
    fn disabled_manager_never_inhibits() {
        let manager = KeepAwake::new(false);
        let guard = manager.turn();
        assert_eq!(manager.active_turns(), 0);
        drop(guard);
        assert_eq!(manager.active_turns(), 0);
    }

    #[test]
    fn concurrent_turns_hold_once_until_last_finishes() {
        let fake = FakeInhibitor::new();
        let manager = KeepAwake::with_inhibitor(Arc::clone(&fake) as Arc<dyn Inhibitor>);
        let g1 = manager.turn();
        let g2 = manager.turn();
        assert_eq!(manager.active_turns(), 2);
        assert_eq!(fake.acquires.load(Ordering::SeqCst), 1);
        assert_eq!(fake.releases.load(Ordering::SeqCst), 0);

        drop(g1);
        assert_eq!(manager.active_turns(), 1);
        assert_eq!(fake.releases.load(Ordering::SeqCst), 0);

        drop(g2);
        assert_eq!(manager.active_turns(), 0);
        assert_eq!(fake.releases.load(Ordering::SeqCst), 1);
    }

    #[test]
    fn channel_tracks_prompt_lifecycle_and_releases_on_drop() {
        let fake = FakeInhibitor::new();
        let manager = KeepAwake::with_inhibitor(Arc::clone(&fake) as Arc<dyn Inhibitor>);
        let server = TestChannel::new(vec![
            br#"{"jsonrpc":"2.0","id":7,"method":"session/prompt","params":{"sessionId":"s","prompt":[]}}"#,
            br#"{"jsonrpc":"2.0","id":8,"method":"session/prompt","params":{}}"#,
        ]);
        let awake = AwakeChannel::new(&server, Arc::clone(&manager));

        assert!(
            awake
                .receive(Some(Duration::from_secs(1)))
                .unwrap()
                .is_some()
        );
        assert_eq!(manager.active_turns(), 1);
        assert_eq!(fake.acquires.load(Ordering::SeqCst), 1);

        // Unrelated agent notification does not end the turn.
        awake
            .send(br#"{"jsonrpc":"2.0","method":"session/update","params":{}}"#)
            .unwrap();
        assert_eq!(manager.active_turns(), 1);

        // Prompt answer ends the turn.
        awake
            .send(br#"{"jsonrpc":"2.0","id":7,"result":{"stopReason":"end_turn"}}"#)
            .unwrap();
        assert_eq!(manager.active_turns(), 0);
        assert_eq!(fake.releases.load(Ordering::SeqCst), 1);

        // Disconnect mid-turn still releases.
        assert!(
            awake
                .receive(Some(Duration::from_secs(1)))
                .unwrap()
                .is_some()
        );
        assert_eq!(manager.active_turns(), 1);
        drop(awake);
        assert_eq!(manager.active_turns(), 0);
        assert_eq!(fake.releases.load(Ordering::SeqCst), 2);
    }
}
