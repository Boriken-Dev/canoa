//! The Foundry Local SDK engine in a process of its own.
//!
//! The SDK loads a native library into whoever runs it, and that library crashes on some models and prompts (a segmentation fault
//! on a prompt of a couple of thousand tokens was measured), which in the host's own process would end the host and every
//! session it serves. So the host starts `foundry-engine` (this program, through the same command that starts its other
//! commands), which opens the engine and serves it over standard input and output ([`canoa_foundry::worker`]). When that process
//! dies, the request under way fails with how it ended and the next one starts a new process, which loads the model again.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "pipes, locks and threads are std's"
)]

use std::collections::BTreeMap;
use std::ffi::OsString;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::PathBuf;
use std::process::ChildStdin;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::mpsc::{self, Sender};
use std::sync::{Arc, Mutex, PoisonError};
use std::time::{Duration, Instant};

use canoa_foundry::worker::{self, Hello};
use canoa_foundry::{Cancel, Engine, EngineError, ModelInfo, lock_unless_cancelled};
use serde_json::Value;

use crate::cli::Program;
use crate::spawn::{Child, Command, Stdio};

/// How long the process has to open the engine and load the model: a large model takes a while to read from disk.
const START_LIMIT: Duration = Duration::from_secs(10 * 60);
/// How long a process that was asked to stop its work for a cancelled request has to do it before it is ended instead: a model
/// stuck in native code does not look at a cancel.
const CANCEL_GRACE: Duration = Duration::from_secs(30);
/// How long a process has to end by itself when its input closes, before it is ended.
const END_LIMIT: Duration = Duration::from_secs(15);

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
    /// What follows `foundry-engine` on the command line.
    pub(crate) fn args(&self) -> Vec<OsString> {
        let mut args: Vec<OsString> = vec!["--model".into(), self.model.clone().into()];
        if let Some(library) = &self.library {
            args.extend(["--library".into(), library.clone().into_os_string()]);
        }
        if let Some(cache) = &self.cache {
            args.extend(["--cache".into(), cache.clone().into_os_string()]);
        }
        if let Some(tokens) = self.window {
            args.extend(["--window".into(), tokens.to_string().into()]);
        }
        if let Some(folder) = &self.copies {
            args.extend(["--copies".into(), folder.clone().into_os_string()]);
        }
        if let Some(folder) = &self.own_cache {
            args.extend(["--own-cache".into(), folder.clone().into_os_string()]);
        }
        args
    }
}

type Pending = Mutex<BTreeMap<u64, Sender<Result<Value, EngineError>>>>;

/// One process, from its start to its end.
struct Link {
    stdin: Mutex<Option<ChildStdin>>,
    child: Mutex<Child>,
    pending: Pending,
    alive: AtomicBool,
}

impl Link {
    /// Writes one line to the process; false when it is not there to read it.
    fn send(&self, line: &str) -> bool {
        let mut stdin = self.stdin.lock().unwrap_or_else(PoisonError::into_inner);
        stdin.as_mut().is_some_and(|stdin| {
            writeln!(stdin, "{line}")
                .and_then(|()| stdin.flush())
                .is_ok()
        })
    }

    fn kill(&self) {
        self.alive.store(false, Ordering::SeqCst);
        drop(
            self.stdin
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        let mut child = self.child.lock().unwrap_or_else(PoisonError::into_inner);
        child.end_group();
        let _ = child.kill();
        let _ = child.wait();
    }

    /// Closes the process's input, which is how it is asked to unload the model and end, and ends it if it has not.
    fn stop(&self) {
        self.alive.store(false, Ordering::SeqCst);
        drop(
            self.stdin
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .take(),
        );
        let until = Instant::now() + END_LIMIT;
        loop {
            let ended = self
                .child
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_wait();
            if !matches!(ended, Ok(None)) {
                return;
            }
            if Instant::now() >= until {
                self.kill();
                return;
            }
            std::thread::sleep(Duration::from_millis(50));
        }
    }

    /// How the process ended, as far as it can be told now.
    fn how_it_ended(&self) -> String {
        let until = Instant::now() + Duration::from_secs(2);
        loop {
            let status = self
                .child
                .lock()
                .unwrap_or_else(PoisonError::into_inner)
                .try_wait();
            match status {
                Ok(Some(status)) => return format!("it ended: {status}"),
                Ok(None) if Instant::now() < until => {
                    std::thread::sleep(Duration::from_millis(25));
                }
                _ => return "it ended".to_owned(),
            }
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
    program: Program,
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
    pub(crate) fn open(program: Program, spec: Spec) -> Result<Self, EngineError> {
        let (link, info) = start(&program, &spec)?;
        Ok(Self {
            program,
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
        let (link, _) = start(&self.program, &self.spec)?;
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

/// Starts `foundry-engine` and reads its first line.
fn start(program: &Program, spec: &Spec) -> Result<(Arc<Link>, ModelInfo), EngineError> {
    let mut command = Command::new(&program.path);
    command
        .args(&program.args)
        .arg("foundry-engine")
        .args(spec.args())
        .stdin(Stdio::Piped)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Inherit);
    // Its own group, so that ending it reaches what the native library started.
    command.own_group();
    let mut child = command.spawn().map_err(|error| {
        EngineError::Failed(format!(
            "the process that runs {} could not start: {error}",
            spec.model
        ))
    })?;
    let (Some(stdin), Some(stdout)) = (child.stdin.take(), child.stdout.take()) else {
        let _ = child.kill();
        let _ = child.wait();
        return Err(EngineError::Failed(
            "the model's process has no standard streams".to_owned(),
        ));
    };
    let link = Arc::new(Link {
        stdin: Mutex::new(Some(stdin)),
        child: Mutex::new(child),
        pending: Mutex::new(BTreeMap::new()),
        alive: AtomicBool::new(true),
    });
    let (hello, said) = mpsc::channel();
    let reading = Arc::clone(&link);
    let name = spec.model.clone();
    std::thread::Builder::new()
        .name("canoa-foundry-engine".to_owned())
        .spawn(move || read(&reading, stdout, &hello, &name))
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

/// Reads the process's lines until it ends, handing each answer to the request that waits for it. What is not one of the
/// protocol's lines is something a native library printed.
fn read(link: &Link, stdout: std::process::ChildStdout, hello: &Sender<Hello>, model: &str) {
    let mut first = true;
    for line in BufReader::new(stdout).lines() {
        let Ok(line) = line else { break };
        if first {
            if let Some(said) = worker::hello(&line) {
                first = false;
                let _ = hello.send(said);
            }
            continue;
        }
        let Some((id, outcome)) = worker::answer(&line) else {
            continue;
        };
        let waiting = link
            .pending
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id);
        if let Some(waiting) = waiting {
            let _ = waiting.send(outcome);
        }
    }
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
    use serde_json::json;

    fn standin() -> PathBuf {
        let exe = std::env::current_exe().expect("test path");
        let dir = exe.parent().expect("deps").parent().expect("debug");
        let name = if cfg!(windows) {
            "foundry_engine_standin.exe"
        } else {
            "foundry_engine_standin"
        };
        let path = dir.join("examples").join(name);
        assert!(
            path.is_file(),
            "build the examples first: cargo build -p canoa-host --examples ({})",
            path.display()
        );
        path
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
        ChildEngine::open(Program::new(standin(), Vec::new()), spec()).expect("opens")
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
    fn a_process_that_cannot_open_its_engine_says_why() {
        let program = Program::new(standin(), vec![OsString::from("--fail-open")]);
        let refused = ChildEngine::open(program, spec()).expect_err("refused");
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
        let engine = std::sync::Arc::new(open());
        let first = engine
            .chat(&json!({"n": 1}), &Cancel::new())
            .expect("answers")["pid"]
            .clone();
        let cancel = Cancel::new();
        let waiting = {
            let (engine, cancel) = (std::sync::Arc::clone(&engine), cancel.clone());
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
        let engine = std::sync::Arc::new(engine);
        let first = engine
            .chat(&json!({"n": 1}), &Cancel::new())
            .expect("answers")["pid"]
            .clone();
        let cancel = Cancel::new();
        let deaf = {
            let (engine, cancel) = (std::sync::Arc::clone(&engine), cancel.clone());
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
        let args: Vec<String> = spec
            .args()
            .iter()
            .map(|arg| arg.to_string_lossy().into_owned())
            .collect();
        assert_eq!(
            args,
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
