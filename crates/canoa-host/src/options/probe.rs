//! A short session with an ACP program that lists its models and efforts only in `session/new`: it is started as a session
//! starts it, asked to initialize and open one session in a temporary directory, and ended. No prompt is ever sent, and
//! the session is closed first when the program offers `session/close`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "processes, files and pipes are std's"
)]

use alloc::string::{String, ToString};
use alloc::vec::Vec;
use core::time::Duration;
use std::ffi::OsString;
use std::io::{BufRead as _, BufReader, Write as _};
use std::path::{Path, PathBuf};
use std::sync::mpsc;
use std::time::Instant;

use coqui_acp_api::ModelInfo;
use serde_json::{Value, json};

use crate::spawn::{Child, Command, Stdio};

/// How long the program's `session/close` is waited for once the models are read.
const CLOSE_WAIT: Duration = Duration::from_secs(2);

/// What a program answered to `initialize` and `session/new`.
#[derive(Debug, PartialEq, Eq)]
pub(crate) struct Probed {
    /// The models, each with the efforts the session lists; empty when its session lists none.
    pub(crate) models: Vec<ModelInfo>,
}

/// Why a program could not be asked.
#[derive(Debug)]
pub(crate) enum Failure {
    /// The program is not there.
    NotInstalled(String),
    /// It is there and did not answer as asked.
    Failed(String),
}

impl Failure {
    pub(crate) fn text(&self) -> &str {
        match self {
            Self::NotInstalled(why) | Self::Failed(why) => why,
        }
    }
}

/// Asks `program args` for what its `session/new` lists, within `wait`.
pub(crate) fn ask(
    program: &OsString,
    args: &[OsString],
    environment: &[(OsString, OsString)],
    wait: Duration,
) -> Result<Probed, Failure> {
    let folder = Folder::make().map_err(|error| Failure::Failed(error.to_string()))?;
    let mut command = Command::new(program);
    command
        .args(args)
        .envs(environment.iter().map(|(key, value)| (key, value)))
        .current_dir(folder.path())
        .stdin(Stdio::Piped)
        .stdout(Stdio::Piped)
        .stderr(Stdio::Null)
        .own_group();
    #[cfg(windows)]
    command.creation_flags(0x0800_0000);
    let mut child = command.spawn().map_err(|error| {
        let text = format!(
            "{} could not be started: {error}",
            program.to_string_lossy()
        );
        if error.kind() == std::io::ErrorKind::NotFound {
            Failure::NotInstalled(text)
        } else {
            Failure::Failed(text)
        }
    })?;
    let result = converse(&mut child, folder.path(), wait);
    let _ = child.kill();
    child.end_group();
    let _ = child.wait();
    result.map_err(Failure::Failed)
}

fn converse(child: &mut Child, folder: &Path, wait: Duration) -> Result<Probed, String> {
    let mut stdin = child.stdin.take().ok_or("no standard input")?;
    let stdout = child.stdout.take().ok_or("no standard output")?;
    let (lines, heard) = mpsc::channel::<String>();
    std::thread::spawn(move || {
        for line in BufReader::new(stdout).lines() {
            let Ok(line) = line else { break };
            if lines.send(line).is_err() {
                break;
            }
        }
    });
    let deadline = Instant::now() + wait;
    let mut next = 0_u64;
    let mut call = |method: &str, params: Value, until: Instant| -> Result<Value, String> {
        next += 1;
        let id = next;
        let line = json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params});
        writeln!(stdin, "{line}")
            .and_then(|()| stdin.flush())
            .map_err(|error| format!("the program closed its input: {error}"))?;
        loop {
            let left = until.saturating_duration_since(Instant::now());
            let text = heard.recv_timeout(left).map_err(|error| match error {
                mpsc::RecvTimeoutError::Timeout => {
                    format!("{method} was not answered within {} s", wait.as_secs())
                }
                mpsc::RecvTimeoutError::Disconnected => {
                    format!("the program ended before it answered {method}")
                }
            })?;
            let Ok(message) = serde_json::from_str::<Value>(&text) else {
                continue;
            };
            if message.get("method").is_some() || message.get("id") != Some(&json!(id)) {
                // A request or a notice of its own: this probe takes nothing from the program and gives it no answer.
                continue;
            }
            if let Some(error) = message.get("error") {
                let why = error
                    .get("message")
                    .and_then(Value::as_str)
                    .unwrap_or("an error");
                return Err(format!("{method} failed: {why}"));
            }
            return Ok(message.get("result").cloned().unwrap_or(Value::Null));
        }
    };
    let initialized = call(
        "initialize",
        json!({
            "protocolVersion": 1,
            "clientCapabilities": {},
            "clientInfo": {"name": "canoa-choices", "version": env!("CARGO_PKG_VERSION")},
        }),
        deadline,
    )?;
    let opened = call(
        "session/new",
        json!({"cwd": folder.to_string_lossy(), "mcpServers": []}),
        deadline,
    )?;
    let models = models_of(&opened);
    let closes = initialized
        .pointer("/agentCapabilities/sessionCapabilities/close")
        .is_some_and(|close| !close.is_null());
    if closes && let Some(id) = opened.get("sessionId").and_then(Value::as_str) {
        let _ = call(
            "session/close",
            json!({"sessionId": id}),
            Instant::now() + CLOSE_WAIT,
        );
    }
    Ok(Probed { models })
}

/// The models a `session/new` answer lists, each with the efforts the session lists beside them.
///
/// The model option of `configOptions` (category `model`) wins over the older `models` block; the efforts are the options
/// of the `thought_level` (or `effort`) category, the same for every model since the session names no other.
pub(crate) fn models_of(opened: &Value) -> Vec<ModelInfo> {
    let options = opened.get("configOptions").and_then(Value::as_array);
    let by_category = |names: &[&str]| {
        options.and_then(|options| {
            options.iter().find(|option| {
                option
                    .get("category")
                    .and_then(Value::as_str)
                    .is_some_and(|category| names.contains(&category))
                    || option
                        .get("id")
                        .and_then(Value::as_str)
                        .is_some_and(|id| names.contains(&id))
            })
        })
    };
    let choices = |option: &Value| -> Vec<(String, Option<String>, Option<String>)> {
        option
            .get("options")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|choice| {
                Some((
                    choice.get("value")?.as_str()?.to_owned(),
                    choice
                        .get("name")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                    choice
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                ))
            })
            .collect()
    };
    let current = |option: &Value| {
        option
            .get("currentValue")
            .and_then(Value::as_str)
            .map(str::to_owned)
    };
    let (efforts, default_effort) = by_category(&["thought_level", "effort"])
        .map(|option| {
            (
                choices(option)
                    .into_iter()
                    .map(|(value, ..)| value)
                    .collect::<Vec<_>>(),
                current(option),
            )
        })
        .unwrap_or_default();
    let (listed, default_model) = if let Some(option) = by_category(&["model"]) {
        (choices(option), current(option))
    } else if let Some(block) = opened.get("models") {
        let listed = block
            .get("availableModels")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|model| {
                Some((
                    model.get("modelId")?.as_str()?.to_owned(),
                    model.get("name").and_then(Value::as_str).map(str::to_owned),
                    model
                        .get("description")
                        .and_then(Value::as_str)
                        .map(str::to_owned),
                ))
            })
            .collect();
        let default = block
            .get("currentModelId")
            .and_then(Value::as_str)
            .map(str::to_owned);
        (listed, default)
    } else {
        return Vec::new();
    };
    listed
        .into_iter()
        .map(|(id, name, description)| {
            let mut info = ModelInfo::new(id.clone());
            info.name = name;
            info.description = description;
            info.efforts.clone_from(&efforts);
            info.default_effort.clone_from(&default_effort);
            info.is_default = default_model.as_deref() == Some(id.as_str());
            info
        })
        .collect()
}

/// A directory of its own for the probe's session, removed when it goes.
struct Folder(PathBuf);

impl Folder {
    fn make() -> std::io::Result<Self> {
        let nanos = std::time::SystemTime::now()
            .duration_since(std::time::UNIX_EPOCH)
            .map_or(0, |since| since.as_nanos());
        let path =
            std::env::temp_dir().join(format!("canoa-choices-{}-{nanos}", std::process::id()));
        std::fs::create_dir(&path)?;
        Ok(Self(path))
    }

    fn path(&self) -> &Path {
        &self.0
    }
}

impl Drop for Folder {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn config_options_give_models_and_efforts() {
        let opened = json!({"sessionId": "s", "configOptions": [
            {"id": "model", "category": "model", "currentValue": "b", "options": [
                {"value": "a", "name": "A"}, {"value": "b", "name": "B"}]},
            {"id": "thought_level", "category": "thought_level", "currentValue": "medium", "options": [
                {"value": "low"}, {"value": "medium"}]},
        ]});
        let models = models_of(&opened);
        assert_eq!(models.len(), 2);
        assert_eq!(models[0].efforts, ["low", "medium"]);
        assert_eq!(models[1].default_effort.as_deref(), Some("medium"));
        assert!(models[1].is_default && !models[0].is_default);
    }

    #[test]
    fn the_older_models_block_still_lists() {
        let opened = json!({"sessionId": "s", "models": {
            "availableModels": [{"modelId": "m1", "name": "One"}], "currentModelId": "m1"}});
        let models = models_of(&opened);
        assert_eq!(models[0].id, "m1");
        assert!(models[0].is_default && models[0].efforts.is_empty());
    }

    #[test]
    fn a_session_that_lists_none_lists_none() {
        assert!(models_of(&json!({"sessionId": "s", "modes": {}})).is_empty());
    }
}
