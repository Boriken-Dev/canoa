//! A command-line tool with no ACP of its own, presented as an ACP agent. Canoa.cfg says how a prompt goes in and how each
//! JSON line the tool writes becomes an ACP update, so no tool has code of its own here. What runs for each connection is
//! `canoa-host tool`, as a stdio agent: the host's scope and process handling apply to it as to any agent.

use alloc::collections::BTreeMap;
use alloc::format;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;
use core::fmt;
use core::time::Duration;
use std::io::{Read, Write};

use serde::{Deserialize, Serialize};
use serde_json::{Map, Value};

mod run;
pub(crate) mod template;

/// The key in `session/new`'s `_meta` under which the host lists what a person should know about this agent before using
/// it: what the tool does without asking, and whether it remembers earlier messages. Each is text for people.
pub const NOTICES_META_KEY: &str = "dev.boriken.canoa/notices";
/// How long a tool has to end a turn after its `cancel` line before its process is ended, and with it the session.
pub const CANCEL_GRACE: Duration = Duration::from_secs(10);
/// The most text one update carries; longer text goes as several.
pub const MAX_TEXT: usize = 1024 * 1024;
/// The most a title, a tool call's id or a failure's text takes, after control and format characters are removed.
pub const MAX_LABEL: usize = 4096;
/// Permission requests one process may have waiting; past it, a request is refused at once.
pub const MAX_ASKS: usize = 16;
/// The most an entry takes as JSON, since `serve` gives it to `canoa-host tool` on a command line, which Windows caps at
/// 32,767 characters.
pub const MAX_SPEC: usize = 16 * 1024;

/// How a tool keeps a conversation.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Runs {
    /// One process holds the session: it starts with `session/new`, and each prompt is a line on its standard input.
    Session,
    /// A process per prompt, which remembers nothing of earlier ones unless `resume_args` names the session it reported.
    Message,
}

/// What a rule makes of an event it matches.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Action {
    /// `agent_message_chunk` from `text`.
    Text,
    /// `agent_thought_chunk` from `text`.
    Thought,
    /// `tool_call` from `id` and `title`, in progress, with `kind` and `input` when given.
    Tool,
    /// `tool_call_update` for `id`, at `status`, with `output` as its text when given.
    ToolDone,
    /// Records `text` as the turn's failure: a turn that then ends is answered with that error.
    Failure,
    /// Ends the turn with `stop`, or with `error` as a failure.
    End,
    /// Records `id` as the tool's session, for `{session}` in `resume_args` and in the prompt line.
    Session,
    /// Asks the person, by `session/request_permission`, and writes `allow` or `reject` back to the tool.
    Ask,
    /// Writes `line` back to the tool at once, such as the answer to a question the host does not show.
    Reply,
    /// Nothing: the event is matched so that no later rule takes it.
    Ignore,
}

/// A tool call's state once it is done.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolStatus {
    /// It finished.
    Completed,
    /// It failed.
    Failed,
}

/// How a turn ended, in ACP's words.
#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
#[non_exhaustive]
pub enum Stop {
    /// The tool finished.
    EndTurn,
    /// The turn was cancelled.
    Cancelled,
    /// The model's output limit ended it.
    MaxTokens,
    /// The model refused.
    Refusal,
}

impl Stop {
    pub(crate) const fn as_str(self) -> &'static str {
        match self {
            Self::EndTurn => "end_turn",
            Self::Cancelled => "cancelled",
            Self::MaxTokens => "max_tokens",
            Self::Refusal => "refusal",
        }
    }
}

/// One rule: an event every `when` path of which holds its value is taken by the first such rule, in the file's order.
/// Text fields are templates (`{path}` is the event's value there); `allow`, `reject` and `line` are JSON whose strings are.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct Rule {
    /// Paths and the values they must hold; empty matches every event.
    #[serde(default)]
    pub when: Map<String, Value>,
    /// What the rule makes of the event.
    pub send: Action,
    /// The text of `text`, `thought` and `failure`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub text: Option<String>,
    /// The tool call's id for `tool` and `tool_done`; the tool's session for `session`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub id: Option<String>,
    /// The title of `tool` and `ask`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub title: Option<String>,
    /// A `tool`'s kind: one of ACP's, or a name in the entry's `kinds`; anything else is `other`.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub kind: Option<String>,
    /// A path whose value is a `tool`'s input, as it is.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub input: Option<String>,
    /// A `tool_done`'s status; `completed` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub status: Option<ToolStatus>,
    /// A `tool_done`'s output text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub output: Option<String>,
    /// How `end` ends the turn; `end_turn` by default.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub stop: Option<Stop>,
    /// An `end` that fails the turn with this text.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub error: Option<String>,
    /// What `ask` writes to the tool when the person allows.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub allow: Option<Value>,
    /// What `ask` writes when the person refuses, the request is cancelled, or it cannot be asked.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub reject: Option<Value>,
    /// What `reply` writes.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub line: Option<Value>,
}

/// A `[host.tools.NAME]` entry: the program, how it keeps a conversation, how a prompt reaches it and what its events become.
#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
#[non_exhaustive]
pub struct ToolSpec {
    /// The program, found as the operating system finds a command.
    pub program: String,
    /// Its arguments. In `message` mode `{prompt}` and `{session}` in them are replaced.
    #[serde(default)]
    pub args: Vec<String>,
    /// How it keeps a conversation.
    pub runs: Runs,
    /// What the tool does without asking the person, in words for people; shown when a session opens.
    pub unasked: String,
    /// In `session` mode, the JSON line a prompt is written as; `{prompt}` is its text and `{session}` the tool's session.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub prompt: Option<Value>,
    /// In `session` mode, the JSON line that asks the tool to stop its turn. Without it, cancelling ends the process.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub cancel: Option<Value>,
    /// In `message` mode, arguments put before `args` once the tool has reported its session, with `{session}` replaced.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub resume_args: Vec<String>,
    /// Names a `kind` template may give, and the ACP tool kind each stands for.
    #[serde(default, skip_serializing_if = "BTreeMap::is_empty")]
    pub kinds: BTreeMap<String, String>,
    /// Sessions one connection may hold.
    #[serde(default = "default_max_sessions")]
    pub max_sessions: usize,
    /// The rules, in order.
    #[serde(default)]
    pub events: Vec<Rule>,
}

const fn default_max_sessions() -> usize {
    crate::config::DEFAULT_MAX_SESSIONS
}

/// ACP's tool kinds.
pub(crate) const TOOL_KINDS: [&str; 10] = [
    "read",
    "edit",
    "delete",
    "move",
    "search",
    "execute",
    "think",
    "fetch",
    "switch_mode",
    "other",
];

/// The fields a rule of each action may set; `when` and `send` are every rule's.
fn allowed(action: Action) -> (&'static [&'static str], &'static [&'static str]) {
    // (required, optional)
    match action {
        Action::Text | Action::Thought | Action::Failure => (&["text"], &[]),
        Action::Tool => (&["id", "title"], &["kind", "input"]),
        Action::ToolDone => (&["id"], &["status", "output"]),
        Action::End => (&[], &["stop", "error"]),
        Action::Session => (&["id"], &[]),
        Action::Ask => (&["title", "allow", "reject"], &[]),
        Action::Reply => (&["line"], &[]),
        Action::Ignore => (&[], &[]),
    }
}

impl Rule {
    /// The fields this rule sets, by name.
    fn set(&self) -> Vec<&'static str> {
        let fields: [(&'static str, bool); 11] = [
            ("text", self.text.is_some()),
            ("id", self.id.is_some()),
            ("title", self.title.is_some()),
            ("kind", self.kind.is_some()),
            ("input", self.input.is_some()),
            ("status", self.status.is_some()),
            ("output", self.output.is_some()),
            ("stop", self.stop.is_some()),
            ("error", self.error.is_some()),
            ("allow", self.allow.is_some()),
            ("reject", self.reject.is_some()),
        ];
        let mut set: Vec<&'static str> = fields
            .iter()
            .filter(|(_, present)| *present)
            .map(|(name, _)| *name)
            .collect();
        if self.line.is_some() {
            set.push("line");
        }
        set
    }

    fn check(&self, index: usize) -> Result<(), String> {
        let (required, optional) = allowed(self.send);
        let set = self.set();
        let name = serde_json::to_value(self.send)
            .ok()
            .and_then(|value| value.as_str().map(ToString::to_string))
            .unwrap_or_default();
        for field in required {
            if !set.contains(field) {
                return Err(format!("events[{index}]: a {name} rule needs {field}"));
            }
        }
        for field in &set {
            if !required.contains(field) && !optional.contains(field) {
                return Err(format!(
                    "events[{index}]: a {name} rule does not take {field}"
                ));
            }
        }
        if let Some(input) = &self.input
            && !template::is_path(input)
        {
            return Err(format!(
                "events[{index}]: input is a path, such as args.command"
            ));
        }
        if self.stop.is_some() && self.error.is_some() {
            return Err(format!(
                "events[{index}]: stop and error exclude each other"
            ));
        }
        if self.when.keys().any(|path| !template::is_path(path)) {
            return Err(format!(
                "events[{index}]: a when key is a path of letters, digits, _ and - steps joined by ."
            ));
        }
        Ok(())
    }
}

impl ToolSpec {
    /// Checks what serde cannot: each mode's fields, each rule's, the kinds named. The text names a setting, never a value.
    ///
    /// # Errors
    ///
    /// What is wrong, in words of the schema's.
    pub fn check(&self) -> Result<(), String> {
        if self.program.is_empty() {
            return Err("program is empty".to_owned());
        }
        if self.unasked.trim().is_empty() {
            return Err(
                "unasked must say what the tool does without asking, even if that is nothing"
                    .to_owned(),
            );
        }
        if self.max_sessions == 0 {
            return Err("max_sessions must be at least 1".to_owned());
        }
        match self.runs {
            Runs::Session => {
                if !matches!(self.prompt, Some(Value::Object(_))) {
                    return Err(
                        "a session tool needs prompt, a table written as one JSON line".to_owned(),
                    );
                }
                if self
                    .cancel
                    .as_ref()
                    .is_some_and(|cancel| !cancel.is_object())
                {
                    return Err("cancel is a table, written as one JSON line".to_owned());
                }
                if !self.resume_args.is_empty() {
                    return Err("resume_args go with runs = \"message\"".to_owned());
                }
            }
            Runs::Message => {
                if self.prompt.is_some() || self.cancel.is_some() {
                    return Err("prompt and cancel go with runs = \"session\"".to_owned());
                }
                // A prompt beginning with - would otherwise be read as an option of the tool's.
                for (at, arg) in self.args.iter().enumerate() {
                    if arg == "{prompt}" && (at == 0 || self.args[at - 1] != "--") {
                        return Err(
                            "an argument that is {prompt} alone comes right after --, or the prompt goes on standard input"
                                .to_owned(),
                        );
                    }
                }
            }
        }
        let json = serde_json::to_string(self).unwrap_or_default();
        if json.contains("\\u0000") {
            return Err(
                "a setting holds a NUL character, which no command line can carry".to_owned(),
            );
        }
        if json.len() > MAX_SPEC {
            return Err(format!(
                "the entry is over {MAX_SPEC} bytes as JSON, more than its command line can carry"
            ));
        }
        for (name, kind) in &self.kinds {
            if name.is_empty() || !TOOL_KINDS.contains(&kind.as_str()) {
                return Err(format!(
                    "kinds: each value is one of ACP's tool kinds: {}",
                    TOOL_KINDS.join(", ")
                ));
            }
        }
        for (index, rule) in self.events.iter().enumerate() {
            rule.check(index)?;
        }
        Ok(())
    }

    /// Whether the prompt goes on the command line, as `{prompt}` in `args`, rather than on standard input.
    pub(crate) fn prompt_in_args(&self) -> bool {
        self.args
            .iter()
            .any(|arg| template::placeholders(arg).contains(&"prompt"))
    }

    /// What a person should know about this agent before using it, as `session/new` lists it under [`NOTICES_META_KEY`].
    #[must_use]
    pub fn notices(&self, name: &str) -> Vec<String> {
        let mut notices = vec![format!(
            "{name} is a command-line tool, not an ACP agent. What it does without asking you: {}",
            self.unasked.trim()
        )];
        if self.runs == Runs::Message {
            notices.push(if self.resume_args.is_empty() {
                format!("Each message runs {name} anew: it does not remember your earlier messages.")
            } else {
                format!(
                    "Each message runs {name} anew, which continues the session it reported, if it reported one."
                )
            });
        }
        notices
    }
}

/// Why a tool entry could not be served.
#[derive(Debug)]
#[non_exhaustive]
pub enum ToolError {
    /// The entry is not a tool entry of this version; what is wrong, in the schema's words.
    Spec(String),
    /// Standard input or output failed.
    Transport(std::io::Error),
}

impl fmt::Display for ToolError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Spec(why) => write!(f, "the tool entry is not usable: {why}"),
            Self::Transport(error) => write!(f, "standard input and output failed: {error}"),
        }
    }
}

impl core::error::Error for ToolError {
    fn source(&self) -> Option<&(dyn core::error::Error + 'static)> {
        match self {
            Self::Transport(error) => Some(error),
            Self::Spec(_) => None,
        }
    }
}

/// Serves the tool entry `name` as an ACP agent on `input` and `output` until `input` ends, then ends every process it
/// started. Each session's processes run in its `cwd`, with this process's environment.
///
/// # Errors
///
/// [`ToolError::Spec`] when `spec` does not pass [`ToolSpec::check`], [`ToolError::Transport`] when `input` cannot be read.
pub fn serve(
    spec: ToolSpec,
    name: &str,
    input: impl Read,
    output: impl Write + Send + 'static,
) -> Result<(), ToolError> {
    spec.check().map_err(ToolError::Spec)?;
    run::serve(Arc::new(spec), name, input, output).map_err(ToolError::Transport)
}

/// [`serve`] on this process's standard input and output.
///
/// # Errors
///
/// As [`serve`].
pub fn serve_stdio(spec: ToolSpec, name: &str) -> Result<(), ToolError> {
    serve(spec, name, std::io::stdin().lock(), std::io::stdout())
}

#[cfg(test)]
mod tests;
