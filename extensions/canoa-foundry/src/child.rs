//! The Foundry Local SDK engine in a process of its own.
//!
//! The SDK loads a native library into whoever runs it, and that library crashes on some models and prompts (a segmentation fault
//! on a prompt of a couple of thousand tokens was measured), which in the host's own process would end the host and every
//! session it serves. So the engine runs in the host's own helper command ([`crate::run_helper`]), started through the spawner
//! the host handed over, which opens the engine and serves it over standard input and output ([`crate::worker`]). When that
//! process dies, the request under way fails with how it ended and the next one starts a new process, which loads the model again.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks, channels and threads are std's"
)]

use std::collections::BTreeMap;
use std::path::PathBuf;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use coqui_acp_api::guatiao::library::Object;
use coqui_acp_api::{ProcessSpawner, SpawnSpec, SpawnedProcess};
use serde_json::Value;

use crate::worker::{self, Hello};
use crate::{Cancel, Engine, EngineError, ModelInfo, lock_unless_cancelled};

/// How long the process has to open the engine and load the model: a large model takes a while to read from disk.
const START_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How long a process that was asked to stop its work for a cancelled request has to do it before it is ended instead: a model
/// stuck in native code does not look at a cancel.
const CANCEL_GRACE: Duration = Duration::from_secs(30);
/// How long a process has to end by itself when its input closes, before it is ended.
const END_LIMIT: Duration = Duration::from_secs(15);
/// How long one read of the process's output waits before it looks again at whether the process is still wanted.
const READ_WAIT_MS: u64 = 500;

/// How the process is started: the host's spawner, and the command line of the host's helper command.
#[derive(Clone)]
pub(crate) struct Launch {
    /// Starts programs where the host chooses.
    pub(crate) spawner: Arc<Object<dyn ProcessSpawner>>,
    /// The program and the arguments that come before the name of this library's helper.
    pub(crate) helper: Vec<String>,
}

/// What the process is started with: the engine's own settings.
#[derive(Debug, Clone, PartialEq, Eq)]
pub(crate) struct Spec {
    pub(crate) model: String,
    pub(crate) library: Option<PathBuf>,
    pub(crate) cache: Option<PathBuf>,
    /// The window to run the model with.
    pub(crate) window: Option<u64>,
    /// The folder the copies of the model it needs are made in.
    pub(crate) copies: Option<PathBuf>,
    /// The models folder of the host's own that the model is linked into.
    pub(crate) own_cache: Option<PathBuf>,
}

impl Spec {
    /// What follows the helper's name on the command line.
    pub(crate) fn args(&self) -> Vec<String> {
        let path = |path: &PathBuf| path.to_string_lossy().into_owned();
        let mut args = vec!["--model".to_owned(), self.model.clone()];
        if let Some(library) = &self.library {
            args.extend(["--library".to_owned(), path(library)]);
        }
        if let Some(cache) = &self.cache {
            args.extend(["--cache".to_owned(), path(cache)]);
        }
        if let Some(tokens) = self.window {
            args.extend(["--window".to_owned(), tokens.to_string()]);
        }
        if let Some(folder) = &self.copies {
            args.extend(["--copies".to_owned(), path(folder)]);
        }
        if let Some(folder) = &self.own_cache {
            args.extend(["--own-cache".to_owned(), path(folder)]);
        }
        args
    }
}

type Pending = Mutex<BTreeMap<u64, Sender<Result<Value, EngineError>>>>;

/// One process, from its start to its end.
struct Link {
    process: Object<dyn SpawnedProcess>,
    pending: Pending,
    alive: AtomicBool,
}

impl Link {
    /// Writes one line to the process; false when it is not there to read it.
    fn send(&self, line: &str) -> bool {
        self.process.write(format!("{line}\n").as_bytes()).is_ok()
    }

    fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.process.close_input();
        let _ = self.process.kill();
        let _ = self.process.wait(READ_WAIT_MS);
    }

    /// Closes the process's input, which is how it is asked to unload the model and end, and ends it if it has not.
    fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        let _ = self.process.close_input();
        let until = Instant::now() + END_LIMIT;
        loop {
            let ended = self.process.wait(50);
            if !matches!(ended, Ok(exit) if exit.running) {
                return;
            }
            if Instant::now() >= until {
                self.kill();
                return;
            }
        }
    }

    /// How the process ended, as far as it can be told now.
    fn how_it_ended(&self) -> String {
        match self.process.wait(2000) {
            Ok(exit) if !exit.running => match (exit.code, exit.signal) {
                (Some(code), _) => format!("it ended: exit code {code}"),
                (None, Some(signal)) => format!("it ended: signal {signal}"),
                (None, None) => "it ended".to_owned(),
            },
            _ => "it ended".to_owned(),
        }
    }
}

impl Drop for Link {
    fn drop(&mut self) {
        if self.alive.load(Ordering::SeqCst) {
            self.stop();
        }
    }
}

/// The SDK engine, in a process the host can lose.
pub(crate) struct ChildEngine {
    launch: Launch,
    spec: Spec,
    info: ModelInfo,
    link: Mutex<Option<Arc<Link>>>,
    next: AtomicU64,
    /// How long a cancelled request has to stop before the process is ended.
    grace: Duration,
    /// One request at a time: a model answers one.
    one_at_a_time: Mutex<()>,
}

impl core::fmt::Debug for ChildEngine {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("ChildEngine")
            .field("model", &self.spec.model)
            .finish_non_exhaustive()
    }
}

impl ChildEngine {
    /// Starts the process and waits for it to say its model is loaded.
    ///
    /// # Errors
    ///
    /// The process could not start, or could not open the engine or load the model.
    pub(crate) fn open(launch: Launch, spec: Spec) -> Result<Self, EngineError> {
        let (link, info) = start(&launch, &spec)?;
        Ok(Self {
            launch,
            spec,
            info,
            link: Mutex::new(Some(link)),
            next: AtomicU64::new(1),
            grace: CANCEL_GRACE,
            one_at_a_time: Mutex::new(()),
        })
    }

    /// The process that is running, started again when the last one ended.
    fn link(&self) -> Result<Arc<Link>, EngineError> {
        let mut held = self.link.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(link) = held.as_ref()
            && link.alive.load(Ordering::SeqCst)
        {
            return Ok(Arc::clone(link));
        }
        tracing::info!(
            "{}: its model's process is started again, which loads the model",
            self.spec.model
        );
        let (link, _) = start(&self.launch, &self.spec)?;
        *held = Some(Arc::clone(&link));
        Ok(link)
    }
}

impl Drop for ChildEngine {
    fn drop(&mut self) {
        if let Some(link) = self
            .link
            .get_mut()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
        {
            link.stop();
        }
    }
}

/// Starts the helper and reads its first line.
fn start(launch: &Launch, spec: &Spec) -> Result<(Arc<Link>, ModelInfo), EngineError> {
    let Some((program, before)) = launch.helper.split_first() else {
        return Err(EngineError::Failed(
            "this host has no program of its own to run the model's process with".to_owned(),
        ));
    };
    let mut command = SpawnSpec::new(program.clone());
    for arg in before
        .iter()
        .map(String::as_str)
        .chain([crate::LIBRARY])
        .map(str::to_owned)
        .chain(spec.args())
    {
        command = command.arg(arg);
    }
    let process = launch.spawner.spawn(command).map_err(|error| {
        EngineError::Failed(format!(
            "the process that runs {} could not start: {}",
            spec.model,
            error.message()
        ))
    })?;
    let link = Arc::new(Link {
        process,
        pending: Mutex::new(BTreeMap::new()),
        alive: AtomicBool::new(true),
    });
    let (hello, said) = mpsc::channel();
    let reading = Arc::clone(&link);
    let name = spec.model.clone();
    std::thread::Builder::new()
        .name("canoa-foundry-engine".to_owned())
        .spawn(move || read(&reading, &hello, &name))
        .map_err(|error| EngineError::Failed(error.to_string()))?;
    let logging = Arc::clone(&link);
    let name = spec.model.clone();
    std::thread::Builder::new()
        .name("canoa-foundry-engine-log".to_owned())
        .spawn(move || log(&logging, &name))
        .map_err(|error| EngineError::Failed(error.to_string()))?;
    match said.recv_timeout(START_LIMIT) {
        Ok(Hello::Ready(info)) => Ok((link, info)),
        Ok(Hello::Failed(why)) => {
            link.kill();
            Err(EngineError::Failed(why))
        }
        Err(_) => {
            let how = link.how_it_ended();
            link.kill();
            Err(EngineError::Failed(format!(
                "the process that runs {} did not say it was ready: {how}",
                spec.model
            )))
        }
    }
}

/// Calls `line` with each line of what `read` gives, until it gives no more.
fn lines(mut read: impl FnMut(&mut [u8]) -> Option<usize>, mut line: impl FnMut(&str)) {
    let mut chunk = [0_u8; 8192];
    let mut held: Vec<u8> = Vec::new();
    while let Some(count) = read(&mut chunk) {
        held.extend_from_slice(&chunk[..count]);
        while let Some(end) = held.iter().position(|byte| *byte == b'\n') {
            let rest = held.split_off(end + 1);
            let text = String::from_utf8_lossy(&held[..end]).into_owned();
            held = rest;
            line(text.trim_end_matches('\r'));
        }
    }
    if !held.is_empty() {
        line(String::from_utf8_lossy(&held).trim_end_matches('\r'));
    }
}

/// Reads the process's lines until it ends, handing each answer to the request that waits for it. What is not one of the
/// protocol's lines is something a native library printed.
fn read(link: &Link, hello: &Sender<Hello>, model: &str) {
    let mut first = true;
    lines(
        |chunk| loop {
            match link.process.read(chunk, READ_WAIT_MS) {
                Ok(0) | Err(_) => return None,
                Ok(count) if count > 0 => return usize::try_from(count).ok(),
                // A timeout, or a link that is down: look again.
                Ok(_) => {}
            }
        },
        |line| {
            if first {
                if let Some(said) = worker::hello(line) {
                    first = false;
                    let _ = hello.send(said);
                }
                return;
            }
            let Some((id, outcome)) = worker::answer(line) else {
                return;
            };
            let waiting = link
                .pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            if let Some(waiting) = waiting {
                let _ = waiting.send(outcome);
            }
        },
    );
    link.alive.store(false, Ordering::SeqCst);
    let how = link.how_it_ended();
    tracing::warn!("{model}: the model's process ended: {how}");
    let waiting = std::mem::take(&mut *link.pending.lock().unwrap_or_else(PoisonError::into_inner));
    for (_, waiting) in waiting {
        let _ = waiting.send(Err(EngineError::Failed(format!(
            "the process that runs {model} stopped before it answered ({how}); the next request starts it again"
        ))));
    }
}

/// Logs what the process writes to its standard error, which the native library and the engine's own log use.
fn log(link: &Link, model: &str) {
    lines(
        |chunk| loop {
            match link.process.read_error(chunk, READ_WAIT_MS) {
                Ok(0) | Err(_) => return None,
                Ok(count) if count > 0 => return usize::try_from(count).ok(),
                Ok(_) => {}
            }
        },
        |line| tracing::info!("{model}: {line}"),
    );
}

impl Engine for ChildEngine {
    fn model(&self) -> Result<ModelInfo, EngineError> {
        Ok(self.info.clone())
    }

    fn models(&self) -> Vec<ModelInfo> {
        vec![self.info.clone()]
    }

    fn chat(&self, request: &Value, cancel: &Cancel) -> Result<Value, EngineError> {
        let _one = lock_unless_cancelled(&self.one_at_a_time, cancel)?;
        let link = self.link()?;
        let id = self.next.fetch_add(1, Ordering::SeqCst);
        let (sender, answer) = mpsc::channel();
        link.pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .insert(id, sender);
        if !link.send(&worker::chat_line(id, request)) {
            link.pending
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .remove(&id);
            return Err(EngineError::Failed(format!(
                "the process that runs {} is gone; the next request starts it again",
                self.spec.model
            )));
        }
        let told = Arc::clone(&link);
        cancel.on_cancel(move || {
            told.send(&worker::cancel_line(id));
        });
        let mut cancelled = None;
        loop {
            match answer.recv_timeout(Duration::from_millis(100)) {
                Ok(outcome) => return outcome,
                Err(mpsc::RecvTimeoutError::Timeout) => {
                    if cancel.is_cancelled() {
                        let since = *cancelled.get_or_insert_with(Instant::now);
                        if since.elapsed() > self.grace {
                            tracing::warn!(
                                "{}: the model's process did not stop its work in {} s: it is ended",
                                self.spec.model,
                                self.grace.as_secs()
                            );
                            link.kill();
                            return Err(EngineError::Cancelled);
                        }
                    }
                }
                Err(mpsc::RecvTimeoutError::Disconnected) => {
                    return Err(EngineError::Failed(
                        "the process that runs the model ended without answering".to_owned(),
                    ));
                }
            }
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use coqui_acp_harness::LocalSpawner;
    use serde_json::json;

    fn standin() -> String {
        let exe = std::env::current_exe().expect("test path");
        let dir = exe.parent().expect("deps").parent().expect("debug");
        let name = if cfg!(windows) {
            "engine_standin.exe"
        } else {
            "engine_standin"
        };
        let path = dir.join("examples").join(name);
        assert!(
            path.is_file(),
            "build the examples first: cargo build -p canoa-foundry --examples ({})",
            path.display()
        );
        path.to_string_lossy().into_owned()
    }

    fn launch(before: &[&str]) -> Launch {
        let mut helper = vec![standin()];
        helper.extend(before.iter().map(|each| (*each).to_owned()));
        Launch {
            spawner: Arc::new(LocalSpawner::new().into_object()),
            helper,
        }
    }

    fn spec() -> Spec {
        Spec {
            model: "stand-in".to_owned(),
            library: None,
            cache: None,
            window: None,
            copies: None,
            own_cache: None,
        }
    }

    fn open() -> ChildEngine {
        ChildEngine::open(launch(&[]), spec()).expect("opens")
    }

    #[test]
    fn the_engine_says_what_its_process_runs_and_answers_through_it() {
        let engine = open();
        assert_eq!(
            engine.model().expect("a model"),
            ModelInfo::new("stand-in").tools(true).context(8192)
        );
        let said = engine
            .chat(&json!({"messages": []}), &Cancel::new())
            .expect("answers");
        assert_eq!(said["echo"], json!({"messages": []}));
    }

    #[test]
    fn a_host_with_no_helper_command_says_so() {
        let launch = Launch {
            spawner: Arc::new(LocalSpawner::new().into_object()),
            helper: Vec::new(),
        };
        let refused = ChildEngine::open(launch, spec()).expect_err("refused");
        assert!(
            refused.to_string().contains("no program of its own"),
            "{refused}"
        );
    }

    #[test]
    fn a_process_that_cannot_open_its_engine_says_why() {
        let refused = ChildEngine::open(launch(&["--fail-open"]), spec()).expect_err("refused");
        assert!(
            refused.to_string().contains("no native library"),
            "{refused}"
        );
    }

    #[test]
    fn a_process_that_crashes_fails_its_request_and_the_next_one_starts_another() {
        let engine = open();
        let first = engine
            .chat(&json!({"n": 1}), &Cancel::new())
            .expect("answers")["pid"]
            .clone();
        let lost = engine
            .chat(&json!({"crash": true}), &Cancel::new())
            .expect_err("the process ended");
        assert!(
            matches!(&lost, EngineError::Failed(why) if why.contains("stopped before it answered")),
            "{lost:?}"
        );
        let after = engine
            .chat(&json!({"n": 2}), &Cancel::new())
            .expect("a new process answers");
        assert_ne!(after["pid"], first, "another process");
    }

    #[test]
    fn a_cancelled_request_stops_in_the_process_and_leaves_it_ready_for_the_next() {
        let engine = Arc::new(open());
        let first = engine
            .chat(&json!({"n": 1}), &Cancel::new())
            .expect("answers")["pid"]
            .clone();
        let cancel = Cancel::new();
        let waiting = {
            let (engine, cancel) = (Arc::clone(&engine), cancel.clone());
            std::thread::spawn(move || engine.chat(&json!({"wait": true}), &cancel))
        };
        std::thread::sleep(Duration::from_millis(300));
        cancel.cancel();
        assert_eq!(waiting.join().expect("ends"), Err(EngineError::Cancelled));
        let after = engine
            .chat(&json!({"n": 2}), &Cancel::new())
            .expect("answers");
        assert_eq!(after["pid"], first, "the same process: nothing was lost");
    }

    #[test]
    fn a_process_that_does_not_stop_when_asked_is_ended_after_its_grace() {
        let mut engine = open();
        engine.grace = Duration::from_millis(300);
        let engine = Arc::new(engine);
        let first = engine
            .chat(&json!({"n": 1}), &Cancel::new())
            .expect("answers")["pid"]
            .clone();
        let cancel = Cancel::new();
        let deaf = {
            let (engine, cancel) = (Arc::clone(&engine), cancel.clone());
            std::thread::spawn(move || engine.chat(&json!({"deaf": true}), &cancel))
        };
        std::thread::sleep(Duration::from_millis(300));
        cancel.cancel();
        let started = Instant::now();
        assert_eq!(deaf.join().expect("ends"), Err(EngineError::Cancelled));
        assert!(started.elapsed() < Duration::from_secs(20));
        let after = engine
            .chat(&json!({"n": 2}), &Cancel::new())
            .expect("a new process answers");
        assert_ne!(after["pid"], first);
    }

    #[test]
    fn the_command_line_carries_what_the_engine_is_opened_with() {
        let mut spec = spec();
        spec.library = Some(PathBuf::from("lib"));
        spec.window = Some(16384);
        spec.copies = Some(PathBuf::from("copies"));
        spec.own_cache = Some(PathBuf::from("own"));
        assert_eq!(
            spec.args(),
            [
                "--model",
                "stand-in",
                "--library",
                "lib",
                "--window",
                "16384",
                "--copies",
                "copies",
                "--own-cache",
                "own"
            ]
        );
    }
}
