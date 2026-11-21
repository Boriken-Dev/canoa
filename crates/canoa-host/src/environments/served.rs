//! The standard tools of an agent served from an environment: the host, as the ACP client, answers the agent's `fs/read_text_file`,
//! `fs/write_text_file` and `terminal/*` calls from the environment, and advertises in `clientCapabilities` only what the
//! environment offers. A call for a capability the environment does not offer is refused naming it.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and locks are std's"
)]

use std::collections::BTreeMap;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};
use std::sync::{Arc, Mutex};

use canoa::environment::capability;
use coqui_acp_api::{ErrorOutput, SpawnSpec};
use serde_json::{Value, json};

use super::remote::Remote;
use super::{Env, lock};

/// JSON-RPC's code for a method the receiver does not have.
const METHOD_NOT_FOUND: i64 = -32601;
/// JSON-RPC's code for parameters the receiver will not take.
const INVALID_PARAMS: i64 = -32602;
/// The code of a call the environment refused or could not do.
const FAILED: i64 = -32000;
/// How much of a terminal's output is kept when the agent names no limit of its own.
const DEFAULT_OUTPUT_BYTES: usize = 1024 * 1024;

/// An error answer: its JSON-RPC code and its message.
pub(crate) type Refusal = (i64, String);

/// Whether `method` is one the host answers when it serves an agent's tools.
pub(crate) fn serves(method: &str) -> bool {
    matches!(
        method,
        "fs/read_text_file"
            | "fs/write_text_file"
            | "terminal/create"
            | "terminal/output"
            | "terminal/wait_for_exit"
            | "terminal/kill"
            | "terminal/release"
    )
}

/// What a terminal of the agent's has written so far.
#[derive(Default)]
pub(crate) struct Output {
    pub(super) bytes: Vec<u8>,
    pub(super) truncated: bool,
}

struct Terminal {
    process: Remote,
    output: Arc<Mutex<Output>>,
    /// The reader has seen the end of the output.
    finished: Arc<AtomicBool>,
}

/// The tools of one agent connection, served from one environment.
pub(crate) struct Served {
    env: Arc<Env>,
    terminals: Mutex<BTreeMap<String, Arc<Terminal>>>,
    next: AtomicU64,
}

impl core::fmt::Debug for Served {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Served")
            .field("env", &self.env.name())
            .finish_non_exhaustive()
    }
}

impl Served {
    /// The tools of an agent served from `env`.
    pub(crate) fn new(env: Arc<Env>) -> Self {
        Self {
            env,
            terminals: Mutex::new(BTreeMap::new()),
            next: AtomicU64::new(1),
        }
    }

    /// The environment.
    pub(crate) fn env(&self) -> &Arc<Env> {
        &self.env
    }

    /// What `clientCapabilities` says of the file and terminal methods: only what the environment offers.
    pub(crate) fn client_capabilities(&self) -> Value {
        let info = self.env.info();
        json!({
            "fs": {
                "readTextFile": info.offers(capability::FS_READ),
                "writeTextFile": info.offers(capability::FS_WRITE),
            },
            "terminal": info.offers(capability::EXEC),
        })
    }

    /// Answers one of the agent's calls: the result, or the refusal.
    ///
    /// # Errors
    ///
    /// The environment lacks the capability the call needs (named), or the call failed there.
    pub(crate) fn answer(&self, method: &str, params: &Value) -> Result<Value, Refusal> {
        match method {
            "fs/read_text_file" => self.read(params),
            "fs/write_text_file" => self.write(params),
            "terminal/create" => self.create(params),
            "terminal/output" => self.output(params),
            "terminal/wait_for_exit" => self.wait_for_exit(params),
            "terminal/kill" => {
                let (terminal, _) = self.terminal(params)?;
                let _ = terminal.process.kill();
                Ok(json!({}))
            }
            "terminal/release" => {
                let (terminal, id) = self.terminal(params)?;
                let _ = terminal.process.kill();
                lock(&self.terminals).remove(&id);
                Ok(json!({}))
            }
            _ => Err((METHOD_NOT_FOUND, format!("{method} is not served here"))),
        }
    }

    /// Ends every terminal the agent left running.
    pub(crate) fn end_all(&self) {
        let all = core::mem::take(&mut *lock(&self.terminals));
        for terminal in all.values() {
            let _ = terminal.process.kill();
        }
    }

    /// Maps the host's refusal words to an error answer: a missing capability is a method the client does not have.
    fn refused(why: String) -> Refusal {
        let code = if why.contains(" has no capability ") {
            METHOD_NOT_FOUND
        } else {
            FAILED
        };
        (code, why)
    }

    fn read(&self, params: &Value) -> Result<Value, Refusal> {
        let path = text(params, "path")?;
        let bytes = self.env.read(path).map_err(Self::refused)?;
        let content = String::from_utf8_lossy(&bytes).into_owned();
        let line = params.get("line").and_then(Value::as_u64);
        let limit = params.get("limit").and_then(Value::as_u64);
        let content = if line.is_some() || limit.is_some() {
            let from = usize::try_from(line.unwrap_or(1).saturating_sub(1)).unwrap_or(usize::MAX);
            let take = limit.map_or(usize::MAX, |limit| {
                usize::try_from(limit).unwrap_or(usize::MAX)
            });
            content
                .split_inclusive('\n')
                .skip(from)
                .take(take)
                .collect::<String>()
        } else {
            content
        };
        Ok(json!({ "content": content }))
    }

    fn write(&self, params: &Value) -> Result<Value, Refusal> {
        let path = text(params, "path")?;
        let content = text(params, "content")?;
        self.env
            .write(path, content.as_bytes().to_vec())
            .map_err(Self::refused)?;
        Ok(Value::Null)
    }

    fn create(&self, params: &Value) -> Result<Value, Refusal> {
        let command = text(params, "command")?;
        let mut spec = SpawnSpec::new(command);
        if let Some(args) = params.get("args").and_then(Value::as_array) {
            for arg in args {
                spec = spec.arg(
                    arg.as_str()
                        .ok_or((INVALID_PARAMS, "args are text".to_owned()))?,
                );
            }
        }
        if let Some(cwd) = params.get("cwd").and_then(Value::as_str) {
            spec = spec.cwd(cwd);
        }
        if let Some(env) = params.get("env").and_then(Value::as_array) {
            let variables: coqui_acp_api::Environment = env
                .iter()
                .filter_map(|each| {
                    Some((
                        each.get("name")?.as_str()?.to_owned(),
                        each.get("value")?.as_str()?.to_owned(),
                    ))
                })
                .collect();
            spec = spec.environment(variables);
        }
        let limit = params
            .get("outputByteLimit")
            .and_then(Value::as_u64)
            .map_or(DEFAULT_OUTPUT_BYTES, |limit| {
                usize::try_from(limit).unwrap_or(usize::MAX)
            });
        spec = spec.error_output(ErrorOutput::Merged);
        let process = Remote::new(self.env.start(spec).map_err(Self::refused)?);
        let output = Arc::new(Mutex::new(Output {
            bytes: Vec::new(),
            truncated: false,
        }));
        let finished = Arc::new(AtomicBool::new(false));
        let (reader, kept, done) = (process.output(), Arc::clone(&output), Arc::clone(&finished));
        std::thread::Builder::new()
            .name("canoa-environment-terminal".to_owned())
            .spawn(move || collect(reader, &kept, limit, &done))
            .map_err(|error| (FAILED, format!("the output could not be read: {error}")))?;
        let id = format!("env-term-{}", self.next.fetch_add(1, Ordering::SeqCst));
        lock(&self.terminals).insert(
            id.clone(),
            Arc::new(Terminal {
                process,
                output,
                finished,
            }),
        );
        Ok(json!({ "terminalId": id }))
    }

    fn terminal(&self, params: &Value) -> Result<(Arc<Terminal>, String), Refusal> {
        let id = text(params, "terminalId")?.to_owned();
        lock(&self.terminals)
            .get(&id)
            .map(|terminal| (Arc::clone(terminal), id))
            .ok_or((INVALID_PARAMS, "no such terminal".to_owned()))
    }

    fn output(&self, params: &Value) -> Result<Value, Refusal> {
        let (terminal, _) = self.terminal(params)?;
        let (text, truncated) = {
            let held = lock(&terminal.output);
            (
                String::from_utf8_lossy(&held.bytes).into_owned(),
                held.truncated,
            )
        };
        let exit = terminal
            .process
            .exit(0)
            .map_err(|error| (FAILED, error.to_string()))?;
        let mut answer = json!({ "output": text, "truncated": truncated });
        if !exit.running {
            answer["exitStatus"] = json!({ "exitCode": exit.code, "signal": exit.signal });
        }
        Ok(answer)
    }

    fn wait_for_exit(&self, params: &Value) -> Result<Value, Refusal> {
        let (terminal, _) = self.terminal(params)?;
        loop {
            let exit = terminal
                .process
                .exit(200)
                .map_err(|error| (FAILED, error.to_string()))?;
            if !exit.running {
                // The output is complete once the reader has seen its end: the agent reads it next.
                while !terminal.finished.load(Ordering::SeqCst) {
                    std::thread::sleep(core::time::Duration::from_millis(10));
                }
                return Ok(json!({ "exitCode": exit.code, "signal": exit.signal }));
            }
        }
    }
}

fn text<'a>(params: &'a Value, key: &str) -> Result<&'a str, Refusal> {
    params
        .get(key)
        .and_then(Value::as_str)
        .ok_or((INVALID_PARAMS, format!("{key} is text")))
}

/// Reads `reader` to its end into `into`, keeping the latest `limit` bytes.
pub(super) fn collect(
    mut reader: impl std::io::Read,
    into: &Mutex<Output>,
    limit: usize,
    done: &AtomicBool,
) {
    let mut buf = [0_u8; 8192];
    while let Ok(n) = reader.read(&mut buf)
        && n > 0
    {
        let mut held = lock(into);
        held.bytes.extend_from_slice(&buf[..n]);
        if held.bytes.len() > limit {
            let mut cut = held.bytes.len() - limit;
            // A cut never starts inside a character.
            while cut < held.bytes.len() && (held.bytes[cut] & 0xC0) == 0x80 {
                cut += 1;
            }
            held.bytes.drain(..cut);
            held.truncated = true;
        }
    }
    done.store(true, Ordering::SeqCst);
}
