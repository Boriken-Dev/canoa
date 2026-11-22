//! The `agent_vendor` kind: who an agent's vendor is, and how Canoa shows it.
//!
//! A harness provider and an account provider say what they do; neither says what the person calls the agent, which vendor
//! stamps its sessions, what its mark looks like or which credential standards are its own. A provider of this kind says
//! that, so the host and the application hold no list of agents: one that is loaded is described by its own library or source,
//! the bundled agents the same way as any other.
//!
//! The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::AccountGrant;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue, Value};

pub mod sessions;

/// The name of the `agent_vendor` kind.
pub const KIND: &str = "agent_vendor";

/// The most characters a mark, a look or a list of names may take, so a library cannot make a description large.
pub const MAX_DOCUMENT_TEXT: usize = 64 * 1024;

/// A credential standard an agent's vendor owns, and its name for people.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct StandardTitle {
    /// The standard's id, such as `anthropic-api`.
    pub id: String,
    /// What a person reads for it, such as `Anthropic API`.
    pub title: String,
}

impl StandardTitle {
    /// The standard `id`, called `title`.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

/// One mode an agent offers before it has run, for a client to show.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModeTitle {
    /// The mode's id, as the agent names it.
    pub id: String,
    /// What a person reads for it.
    pub title: String,
}

impl ModeTitle {
    /// The mode `id`, called `title`.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
        }
    }
}

/// Who an agent is, from [`AgentVendor::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct VendorInfo {
    /// The id the agent goes by: the id of its `acp_harness` provider when it has one (`claude`), else the lower-case name of
    /// its source. Every other provider and source of the agent names it this way.
    pub agent: String,
    /// Its name for people, such as `Claude`.
    pub title: String,
    /// The program's name for people, such as `Claude Code`; absent when it is the title.
    pub program: Option<String>,
    /// The vendor a session on it is stamped with (`AGENTS_VENDOR`), such as `anthropic`; absent for an agent with none.
    pub vendor: Option<String>,
    /// The agent's id in the standard registry of harnesses (`claude-code`); absent when the registry has none.
    pub aos: Option<String>,
    /// The variable the running model is read from in the agent's environment (`ANTHROPIC_MODEL`).
    pub model_variable: Option<String>,
    /// Where the agent puts a tool's own name in a tool call's `_meta`, as a JSON pointer below `_meta` (`/claudeCode/toolName`);
    /// absent for an agent whose tool call title is the name.
    pub tool_name: Option<String>,
    /// The folder below a project where the agent reads skills from (`.claude/skills`), for a source of skills to be searched there.
    pub skills_folder: Option<String>,
    /// The files of a project's context the agent loads for itself, as paths below a directory written with `/` (`AGENTS.md`), so the
    /// host does not give it the same text a second time. Empty for an agent that loads none of the standard names.
    #[map(default)]
    pub context_files: Vec<String>,
    /// The variable that names the folder the agent keeps its own files in (`CLAUDE_CONFIG_DIR`), which a person may point elsewhere.
    pub home_variable: Option<String>,
    /// That folder when the variable is not set, below the person's home (`.claude`).
    pub home_folder: Option<String>,
    /// The files in that folder the agent loads as the person's own instructions, as paths below it written with `/` (`CLAUDE.md`).
    #[map(default)]
    pub home_context_files: Vec<String>,
    /// The credential standards that are the vendor's own, with their names for people.
    #[map(default)]
    pub standards: Vec<StandardTitle>,
    /// The names the agent reports for itself in ACP's `initialize` (`agentInfo.name`), which tell its sessions apart.
    #[map(default)]
    pub names: Vec<String>,
    /// The keys of `initialize`'s `_meta` that are `true` for this agent and no other, for one that reports no `agentInfo` name.
    #[map(default)]
    pub markers: Vec<String>,
    /// The modes the agent is known to offer before it has run.
    #[map(default)]
    pub modes: Vec<ModeTitle>,
    /// The `https` addresses the agent's program reaches while it runs (`https://api.anthropic.com/`): its model API and login
    /// hosts. The host asks the system's proxy about each, to choose the proxy the program is started with.
    #[map(default)]
    pub endpoints: Vec<String>,
    /// Its mark as SVG text: `path` elements with a `d` and an optional `fill`, in the `viewBox`'s square. A path with no `fill`,
    /// or `currentColor`, is drawn in the text's colour. At most [`MAX_DOCUMENT_TEXT`] characters.
    pub mark: Option<String>,
    /// Its look as a JSON document with an `id`, a `label`, the agent's `vendor` and, for `light` and `dark`, colour tokens.
    pub look: Option<String>,
}

impl VendorInfo {
    /// An agent `agent` called `title`, with nothing else said of it.
    #[must_use]
    pub fn new(agent: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            agent: agent.into(),
            title: title.into(),
            program: None,
            vendor: None,
            aos: None,
            model_variable: None,
            tool_name: None,
            skills_folder: None,
            context_files: Vec::new(),
            home_variable: None,
            home_folder: None,
            home_context_files: Vec::new(),
            standards: Vec::new(),
            names: Vec::new(),
            markers: Vec::new(),
            modes: Vec::new(),
            endpoints: Vec::new(),
            mark: None,
            look: None,
        }
    }

    /// Why this description cannot be used, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        let word = |text: &str| {
            !text.is_empty()
                && text.len() <= 64
                && text.bytes().all(|byte| {
                    byte.is_ascii_lowercase()
                        || byte.is_ascii_digit()
                        || byte == b'-'
                        || byte == b'_'
                })
        };
        if !word(&self.agent) {
            return Some(alloc::format!(
                "the agent id {:?} is not 1 to 64 lower-case letters, digits, hyphens and underscores",
                self.agent
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the agent {:?} has no title", self.agent));
        }
        for (what, text) in [("mark", &self.mark), ("look", &self.look)] {
            if text
                .as_ref()
                .is_some_and(|text| text.len() > MAX_DOCUMENT_TEXT)
            {
                return Some(alloc::format!(
                    "the {what} of {:?} is longer than {MAX_DOCUMENT_TEXT} characters",
                    self.agent
                ));
            }
        }
        None
    }
}

/// A variable of a program's environment.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Variable {
    /// Its name.
    pub name: String,
    /// Its value.
    pub value: String,
}

impl Variable {
    /// The variable `name` set to `value`.
    #[must_use]
    pub fn new(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            value: value.into(),
        }
    }
}

/// Where a kind of login lives in an account's directory, for a kind whose provider renews it by looking at that directory.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct LoginFile {
    /// The credential that holds the file's text.
    pub key: String,
    /// The file's name in the account's directory.
    pub file: String,
}

impl LoginFile {
    /// The login held as credential `key` and kept in the file `file`.
    #[must_use]
    pub fn new(key: impl Into<String>, file: impl Into<String>) -> Self {
        Self {
            key: key.into(),
            file: file.into(),
        }
    }
}

/// Which command-line family a shell belongs to.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum ShellFamily {
    /// POSIX Bourne-Again shell (`bash`).
    Bash,
    /// Z shell (`zsh`).
    Zsh,
    /// PowerShell (`powershell` or `pwsh`).
    Powershell,
    /// Windows Command Prompt (`cmd`).
    Cmd,
    /// Any other shell.
    Other,
}

impl ShellFamily {
    /// The family's canonical lower-case name.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Bash => "bash",
            Self::Zsh => "zsh",
            Self::Powershell => "powershell",
            Self::Cmd => "cmd",
            Self::Other => "other",
        }
    }
}

/// A shell a person chose, as an agent is asked to use it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ShellRequest {
    /// The shell program's path.
    pub path: String,
    /// The program's file name without its extension, in lower case (`bash`, `pwsh`).
    pub name: String,
    /// Its family.
    pub family: ShellFamily,
    /// Whether it is the one the Windows Subsystem for Linux starts.
    pub wsl: bool,
    /// Whether the host runs on Windows.
    pub windows: bool,
}

impl ShellRequest {
    /// A request for the shell at `path`, called `name`, of `family`, with neither the subsystem nor Windows.
    #[must_use]
    pub fn new(path: impl Into<String>, name: impl Into<String>, family: ShellFamily) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            family,
            wsl: false,
            windows: false,
        }
    }
}

/// How an agent is told to use a shell: the variables that make it, or why it cannot be told.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ShellAdvice {
    /// The variables that make the agent use the shell.
    #[map(default)]
    pub env: Vec<Variable>,
    /// Why the agent does not use it, in words, when it cannot be told.
    pub notice: Option<String>,
}

impl ShellAdvice {
    /// Advice that sets the variable `name` to `value`.
    #[must_use]
    pub fn sets(name: impl Into<String>, value: impl Into<String>) -> Self {
        Self {
            env: alloc::vec![Variable::new(name, value)],
            notice: None,
        }
    }

    /// Advice that says only that the agent cannot be told, for `why`.
    #[must_use]
    pub fn cannot(why: impl Into<String>) -> Self {
        Self {
            env: Vec::new(),
            notice: Some(why.into()),
        }
    }
}

/// A request for the sessions the agent's own program kept on this computer.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SessionsRequest {
    /// The folder the program keeps them in, or none for where the program itself looks.
    pub home: Option<String>,
    /// The most files to look at.
    pub limit: u64,
    /// How many of the newest files are read for their title and folder.
    pub peek: u64,
}

impl SessionsRequest {
    /// A request for at most `limit` sessions from where the program looks, the newest `peek` of them read for a title and folder.
    #[must_use]
    pub const fn new(limit: u64, peek: u64) -> Self {
        Self {
            home: None,
            limit,
            peek,
        }
    }
}

/// One session file an agent's program kept.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SessionFile {
    /// The session's id as the program names it.
    pub id: String,
    /// The file's path.
    pub path: String,
    /// Its size in bytes.
    pub bytes: u64,
    /// When it last changed, in milliseconds since the Unix epoch.
    pub updated_ms: u64,
    /// The first thing the person said in it, tidy and short.
    pub title: Option<String>,
    /// The folder the session worked in.
    pub cwd: Option<String>,
}

impl SessionFile {
    /// A session `id` in the file `path` of `bytes` bytes, changed at `updated_ms`.
    #[must_use]
    pub fn new(
        id: impl Into<String>,
        path: impl Into<String>,
        bytes: u64,
        updated_ms: u64,
    ) -> Self {
        Self {
            id: id.into(),
            path: path.into(),
            bytes,
            updated_ms,
            title: None,
            cwd: None,
        }
    }
}

/// The sessions an agent's program kept, from [`AgentVendor::sessions`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SessionListing {
    /// The folder they were read from, when there is one.
    pub home: Option<String>,
    /// The most bytes of a session file [`AgentVendor::session_history`] takes.
    pub max_bytes: u64,
    /// The session files found, the newest first.
    #[map(default)]
    pub files: Vec<SessionFile>,
}

impl SessionListing {
    /// A listing of `files` from `home`, whose files may hold `max_bytes` each.
    #[must_use]
    pub const fn new(home: Option<String>, max_bytes: u64, files: Vec<SessionFile>) -> Self {
        Self {
            home,
            max_bytes,
            files,
        }
    }
}

/// A session file to turn into a history.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HistoryRequest {
    /// The file's text.
    pub text: String,
    /// The id the history's session is given (a UUID).
    pub session: String,
}

impl HistoryRequest {
    /// A request to read `text` as the session `session`.
    #[must_use]
    pub fn new(text: impl Into<String>, session: impl Into<String>) -> Self {
        Self {
            text: text.into(),
            session: session.into(),
        }
    }
}

/// One session's history, from [`AgentVendor::session_history`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HistoryAnswer {
    /// The history in the standard session-history format: its JSON lines.
    pub history: String,
    /// The first thing the person said in the session, tidy and short.
    pub title: Option<String>,
}

impl HistoryAnswer {
    /// The history `history` of a session first spoken to as `title`.
    #[must_use]
    pub fn new(history: impl Into<String>, title: Option<String>) -> Self {
        Self {
            history: history.into(),
            title,
        }
    }
}

/// A file an agent asks the host to write, at a plain relative path (`/`-separated, no `.` or `..`) below the folder it is written in.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct DeliveredFile {
    /// The file's path below the folder.
    pub path: String,
    /// Its text.
    pub text: String,
}

impl DeliveredFile {
    /// The file `path` holding `text`.
    #[must_use]
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            text: text.into(),
        }
    }
}

/// The placeholder in a [`PluginDelivery::meta`] that the host replaces with the folder's absolute path.
pub const PLUGIN_PATH: &str = "{path}";

/// A folder the host wrote that an agent loads as a plugin, and how a session request says so.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct PluginDelivery {
    /// The plugin's folder below the folder the host wrote, `/`-separated; empty for that folder itself.
    pub folder: String,
    /// What the session request's `_meta` gains, as an object: it is merged into the request's, objects key by key and lists by
    /// appending. A string that is exactly [`PLUGIN_PATH`] becomes the folder's absolute path.
    pub meta: Value,
}

impl PluginDelivery {
    /// The plugin in `folder`, loaded by a session whose `_meta` gains `meta`.
    #[must_use]
    pub fn new(folder: impl Into<String>, meta: Value) -> Self {
        Self {
            folder: folder.into(),
            meta,
        }
    }
}

/// What an agent said it takes, when the host asks how it takes skills.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SkillsRequest {
    /// Whether the agent's `initialize` says it takes ACP's `additionalDirectories`.
    pub additional_directories: bool,
}

impl SkillsRequest {
    /// A request for an agent that takes `additional_directories` or not.
    #[must_use]
    pub const fn new(additional_directories: bool) -> Self {
        Self {
            additional_directories,
        }
    }
}

/// How an agent is given the skills of a profile.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SkillsDelivery {
    /// The folder, below the one the host writes, that holds a skill's own folder: a skill `name` is written at
    /// `<folder>/<name>/...`.
    pub folder: String,
    /// The files written beside the skills, such as a plugin's manifest.
    #[map(default)]
    pub files: Vec<DeliveredFile>,
    /// The folder the agent loads as a plugin, when it takes skills in one.
    pub plugin: Option<PluginDelivery>,
    /// Whether the folder the host wrote is given to the session as an additional directory.
    #[map(default)]
    pub directory: bool,
    /// What the report says of each skill, in words; `{name}` is the skill's name.
    pub how: String,
}

impl SkillsDelivery {
    /// Skills written under `folder` and reported as `how`, given to the agent by neither a plugin nor a directory yet.
    #[must_use]
    pub fn new(folder: impl Into<String>, how: impl Into<String>) -> Self {
        Self {
            folder: folder.into(),
            files: Vec::new(),
            plugin: None,
            directory: false,
            how: how.into(),
        }
    }
}

/// The moments of a session that only an agent's own hooks show the host.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub enum HookKind {
    /// A session started again or was compacted into or cleared.
    #[map(rename = "start")]
    Start,
    /// A compaction is about to run, which the command's failure stops.
    #[map(rename = "pre-compact")]
    PreCompact,
    /// A batch of tool calls ended.
    #[map(rename = "batch")]
    Batch,
    /// One of the agent's own tools is about to run (a tool [`ToolHook::tools`] names): the command reads the agent's payload on
    /// standard input and prints the answer the agent reads on standard output. A command that fails, or exits with any status but
    /// 0, refuses the call.
    #[map(rename = "pre-tool")]
    PreTool,
}

/// What one of an agent's own tools does on the computer the agent runs on.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
#[non_exhaustive]
pub enum ToolRole {
    /// Runs a shell command, which [`HookTool::command`] names in its input.
    Shell,
    /// Reads, writes, edits, searches or lists files.
    Files,
}

/// One of an agent's own tools that the host takes over when the session's tools are served from an environment.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HookTool {
    /// Its name, as the agent's payload gives it.
    pub name: String,
    /// What it does.
    pub role: ToolRole,
    /// For a shell: the key of its input that holds the command.
    pub command: Option<String>,
    /// For a shell: the shell the command is written for.
    pub shell: Option<ShellFamily>,
}

impl HookTool {
    /// A tool that runs commands written for `shell`, held at `command` in its input.
    #[must_use]
    pub fn shell(name: impl Into<String>, command: impl Into<String>, shell: ShellFamily) -> Self {
        Self {
            name: name.into(),
            role: ToolRole::Shell,
            command: Some(command.into()),
            shell: Some(shell),
        }
    }

    /// A tool that works on files.
    #[must_use]
    pub fn files(name: impl Into<String>) -> Self {
        Self {
            name: name.into(),
            role: ToolRole::Files,
            command: None,
            shell: None,
        }
    }
}

/// One of the agent's own words for a permission mode, and the mode it is.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ModeWord {
    /// The word the agent's payload carries (`dontAsk`).
    pub word: String,
    /// The mode: `manual` (asks), `accept_edits`, `auto`, `plan`, `bypass`, or `dont_ask` (refuses what no rule allows).
    pub mode: String,
}

impl ModeWord {
    /// `word` means `mode`.
    #[must_use]
    pub fn new(word: impl Into<String>, mode: impl Into<String>) -> Self {
        Self {
            word: word.into(),
            mode: mode.into(),
        }
    }
}

/// How the host reads the payload of a [`HookKind::PreTool`] hook and answers it in the agent's own terms. Pointers are JSON
/// pointers; the answers are JSON documents into which the host puts a reason or a rewritten input.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ToolHook {
    /// The tools the hook runs before; no other tool of the agent's is taken over.
    pub tools: Vec<HookTool>,
    /// Where the payload names the tool.
    pub name_at: String,
    /// Where the payload holds the tool's input, an object.
    pub input_at: String,
    /// Where the payload says the agent's permission mode, when it does.
    pub mode_at: Option<String>,
    /// The agent's words for its modes.
    pub modes: Vec<ModeWord>,
    /// The answer that refuses the call, as JSON text.
    pub deny: String,
    /// Where the reason goes in [`Self::deny`].
    pub deny_reason_at: String,
    /// The answer that changes the call's input and decides nothing, so the agent's own rules and mode still decide whether it
    /// runs, as JSON text.
    pub rewrite: String,
    /// Where the new input goes in [`Self::rewrite`].
    pub rewrite_input_at: String,
}

impl ToolHook {
    /// A hook before `tools`, whose payload names the tool and holds its input at the two pointers of `payload`, answered with
    /// `deny` and `rewrite` (each its JSON text and where the reason, or the input, goes); no mode is read.
    #[must_use]
    pub fn new(
        tools: Vec<HookTool>,
        payload: (&str, &str),
        deny: (String, &str),
        rewrite: (String, &str),
    ) -> Self {
        Self {
            tools,
            name_at: payload.0.to_owned(),
            input_at: payload.1.to_owned(),
            mode_at: None,
            modes: Vec::new(),
            deny: deny.0,
            deny_reason_at: deny.1.to_owned(),
            rewrite: rewrite.0,
            rewrite_input_at: rewrite.1.to_owned(),
        }
    }
}

/// One command an agent's hook runs.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HookCommand {
    /// What it reports: `start` (a session started again or was compacted into or cleared), `pre-compact` (a compaction is about
    /// to run, which the command's failure stops), or `batch` (a batch of tool calls ended).
    pub kind: HookKind,
    /// The command line, quoted for a POSIX shell, that reports it to the host.
    pub command: String,
}

impl HookCommand {
    /// The command `command` for the moment `kind`.
    #[must_use]
    pub fn new(kind: HookKind, command: impl Into<String>) -> Self {
        Self {
            kind,
            command: command.into(),
        }
    }
}

/// The hooks the host asks an agent to run.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HooksRequest {
    /// One command for each moment to be reported.
    pub commands: Vec<HookCommand>,
    /// The longest, in seconds, the host may take to answer one.
    pub timeout_seconds: u64,
}

impl HooksRequest {
    /// A request for `commands`, each given `timeout_seconds` to answer.
    #[must_use]
    pub const fn new(commands: Vec<HookCommand>, timeout_seconds: u64) -> Self {
        Self {
            commands,
            timeout_seconds,
        }
    }
}

/// How an agent is given hooks that report to the host: the files of a plugin, and how a session request loads it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HooksDelivery {
    /// The files written, below the folder the host made for the session.
    pub files: Vec<DeliveredFile>,
    /// How the session request loads them.
    pub plugin: PluginDelivery,
}

impl HooksDelivery {
    /// Hooks written as `files` and loaded as `plugin`.
    #[must_use]
    pub const fn new(files: Vec<DeliveredFile>, plugin: PluginDelivery) -> Self {
        Self { files, plugin }
    }
}

/// An event only an agent's own hooks can show, as the host asks whether they do.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HookSupportRequest {
    /// The event: `cd` (the working directory changed), `pre-compact` or `session-start`.
    pub event: String,
    /// For `session-start`, why a session starts that ACP does not show: `compact` or `clear`.
    pub reason: Option<String>,
}

impl HookSupportRequest {
    /// A request about `event`, and for a start `reason`.
    #[must_use]
    pub fn new(event: impl Into<String>, reason: Option<String>) -> Self {
        Self {
            event: event.into(),
            reason,
        }
    }
}

/// What an agent's own hooks do for one event.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct HookSupport {
    /// Whether the event fires.
    pub fires: bool,
    /// How it fires, or why it cannot, in words.
    pub text: String,
}

impl HookSupport {
    /// An event that fires, as `text` says.
    #[must_use]
    pub fn fires(text: impl Into<String>) -> Self {
        Self {
            fires: true,
            text: text.into(),
        }
    }

    /// An event that cannot fire, for `text`.
    #[must_use]
    pub fn cannot(text: impl Into<String>) -> Self {
        Self {
            fires: false,
            text: text.into(),
        }
    }
}

/// How the table of words for the access (sandbox, network, approvals) reaches an agent.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct AccessTarget {
    /// The agent's id in the standard registry of harnesses, which names its table of words.
    pub harness: String,
    /// Whether a program the host starts itself is given the access in its arguments, variables and session `_meta`; false when
    /// the agent takes it in each session's request.
    pub at_launch: bool,
}

impl AccessTarget {
    /// The table of `harness`, applied when the program starts (`at_launch`) or when each session does.
    #[must_use]
    pub fn new(harness: impl Into<String>, at_launch: bool) -> Self {
        Self {
            harness: harness.into(),
            at_launch,
        }
    }
}

/// Where an agent takes the address and bearer of a gateway the host runs for a session, by an account kind of its own.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct GatewayRoute {
    /// The agent's own account kind that takes an address and a secret.
    pub kind: String,
    /// The key of that kind's credentials that holds the bearer.
    pub credential: String,
    /// The key of its settings that holds the address; absent when the agent reads it from a file in its folder
    /// ([`AgentVendor::gateway_files`]).
    pub address: Option<String>,
    /// What follows the gateway's address in the address the agent is given (`/v1`).
    #[map(default)]
    pub suffix: String,
    /// Settings of that kind the account fixes, each a key and its value.
    #[map(default)]
    pub fixed: Vec<Variable>,
}

impl GatewayRoute {
    /// A route through the kind `kind`, whose bearer is the credential `credential`, with no address setting, suffix or fixed setting.
    #[must_use]
    pub fn new(kind: impl Into<String>, credential: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            credential: credential.into(),
            address: None,
            suffix: String::new(),
            fixed: Vec::new(),
        }
    }
}

/// What the host knows that an agent's entry settings may depend on.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct EntrySettingsRequest {
    /// The launch settings of the copy of the agent the Store installed, each a name and its text; empty when there is none. A setting
    /// that is an absolute path names a file inside that copy.
    #[map(default)]
    pub managed: Vec<Variable>,
    /// The places the person's configuration says a session may not read or change, when it says any; absent when it says nothing.
    pub protected: Option<Vec<String>>,
}

impl EntrySettingsRequest {
    /// A request with no managed copy and nothing said of protected places.
    #[must_use]
    pub const fn new() -> Self {
        Self {
            managed: Vec::new(),
            protected: None,
        }
    }
}

impl Default for EntrySettingsRequest {
    fn default() -> Self {
        Self::new()
    }
}

/// What a shell is said to do for an agent that has no word of its own.
pub const NO_SHELL_SETTING: &str = "this agent documents no setting for its shell";

/// Describes an agent, and answers what Canoa does for it beyond its description. `describe` is the same answer on every call, with
/// nothing run and no connection opened; every other method has a default that says the agent has nothing to add.
#[coqui_acp_api::guatiao::kind(name = "agent_vendor", crate = ::coqui_acp_api::guatiao)]
pub trait AgentVendor: Send + Sync {
    /// Who the agent is.
    fn describe(&self) -> Result<VendorInfo, ProviderError>;

    /// The variables that holding the account `grant` describes sets in the agent's environment and that hold no credential (its
    /// configuration directory, its endpoint), so a profile's layers can read them before a session starts. The default: none.
    fn account_variables(&self, grant: AccountGrant) -> Result<Vec<Variable>, ProviderError> {
        drop(grant);
        Ok(Vec::new())
    }

    /// Where a login of the account kind `kind` lives in the account's directory, when the agent's provider renews it by looking
    /// at the directory. The default: nowhere.
    fn login_file(&self, kind: String) -> Result<Option<LoginFile>, ProviderError> {
        drop(kind);
        Ok(None)
    }

    /// How the agent is told to use `shell`. The default says the agent documents no setting for its shell.
    fn shell_advice(&self, shell: ShellRequest) -> Result<ShellAdvice, ProviderError> {
        drop(shell);
        Ok(ShellAdvice::cannot(NO_SHELL_SETTING))
    }

    /// Why the agent does not take the starting instructions of a profile, in words, when it does not. The default: it does not
    /// say it does.
    fn context_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    /// Why the agent does not take the skills of a profile, in words, when it does not. The default: it does not say.
    fn skills_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    /// Why the agent starts none of the MCP servers a session request gives it, in words, when it starts none. The default: it
    /// starts them as ACP says.
    fn mcp_not_taken(&self) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    /// What the report adds to an MCP server the agent is given, in words, where the agent does something with it that the person
    /// should know; `{name}` is the server's name. The default: nothing.
    fn mcp_note(&self) -> Result<Option<String>, ProviderError> {
        Ok(None)
    }

    /// How the agent is given the skills of a profile, for an agent that says what it takes in `request`; `None` when it cannot be.
    /// The default: it cannot.
    fn skills_delivery(
        &self,
        request: SkillsRequest,
    ) -> Result<Option<SkillsDelivery>, ProviderError> {
        let _ = request;
        Ok(None)
    }

    /// How the agent runs hooks that report to the host: the moments of `request` that only the agent's own hooks show, as the
    /// files of a plugin. `None` when the agent's own hooks cannot be given. The default: they cannot.
    fn hooks_delivery(
        &self,
        request: HooksRequest,
    ) -> Result<Option<HooksDelivery>, ProviderError> {
        drop(request);
        Ok(None)
    }

    /// What the agent's own hooks do for an event ACP does not show; `None` when the agent says nothing of it, which the host
    /// reports as an event no hook of the agent's reaches. The default: nothing.
    fn hook_support(
        &self,
        request: HookSupportRequest,
    ) -> Result<Option<HookSupport>, ProviderError> {
        drop(request);
        Ok(None)
    }

    /// How the agent's own file and shell tools are taken over by a [`HookKind::PreTool`] hook, so that they act in the
    /// environment a session's tools are served from; `None` for an agent whose tools cannot be taken over. The default: they
    /// cannot.
    fn tool_hook(&self) -> Result<Option<ToolHook>, ProviderError> {
        Ok(None)
    }

    /// How the agent is given a gateway the host runs for a session (the address and bearer of a local model's server), when it can be:
    /// the account kind of its own that takes them. The default: it cannot.
    fn gateway_route(&self) -> Result<Option<GatewayRoute>, ProviderError> {
        Ok(None)
    }

    /// The files the agent reads from its own folder to use a gateway at `address` (what [`GatewayRoute::address`] leaves out, and what
    /// a gateway cannot serve turned off). The default: none.
    fn gateway_files(&self, address: String) -> Result<Vec<DeliveredFile>, ProviderError> {
        drop(address);
        Ok(Vec::new())
    }

    /// The settings of the agent's own harness provider that the host's source for it is started with, as an object (a managed
    /// copy's program, the person's protected places), or `None` when there are none to add. The default: none.
    fn entry_settings(
        &self,
        request: EntrySettingsRequest,
    ) -> Result<Option<Value>, ProviderError> {
        drop(request);
        Ok(None)
    }

    /// The table of words that says what became of the access a profile sets, for the agent the host starts or serves; `None` for an
    /// agent no table covers, which is started as its entry says. The default: none.
    fn access_target(&self) -> Result<Option<AccessTarget>, ProviderError> {
        Ok(None)
    }

    /// The sessions the agent's own program kept on this computer, found and read as the program lays them out; `None` for an agent
    /// whose sessions Canoa cannot bring in.
    fn sessions(&self, request: SessionsRequest) -> Result<Option<SessionListing>, ProviderError> {
        drop(request);
        Ok(None)
    }

    /// The standard session history of one session file's text. The default refuses: the agent has none to give.
    fn session_history(&self, request: HistoryRequest) -> Result<HistoryAnswer, ProviderError> {
        drop(request);
        Err(ProviderError::new(
            Status::GUATIAO_ERR_UNSUPPORTED,
            "this agent's sessions cannot be brought in",
        ))
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    /// An agent that says only who it is.
    struct Plain;

    impl AgentVendor for Plain {
        fn describe(&self) -> Result<VendorInfo, ProviderError> {
            Ok(VendorInfo::new("plain", "Plain"))
        }
    }

    #[test]
    fn an_agent_that_says_only_who_it_is_has_nothing_to_add() {
        let plain = Plain;
        let grant = AccountGrant::new("a", "k");
        assert_eq!(plain.account_variables(grant).expect("answers"), []);
        assert_eq!(plain.login_file("k".to_owned()).expect("answers"), None);
        assert_eq!(plain.context_not_taken().expect("answers"), None);
        assert_eq!(plain.skills_not_taken().expect("answers"), None);
        assert_eq!(
            plain
                .skills_delivery(SkillsRequest::new(true))
                .expect("answers"),
            None
        );
        assert_eq!(
            plain
                .hooks_delivery(HooksRequest::new(Vec::new(), 1))
                .expect("answers"),
            None
        );
        assert_eq!(
            plain
                .hook_support(HookSupportRequest::new("cd", None))
                .expect("answers"),
            None
        );
        assert_eq!(plain.access_target().expect("answers"), None);
        assert_eq!(
            plain
                .entry_settings(EntrySettingsRequest::new())
                .expect("answers"),
            None
        );
        assert_eq!(plain.gateway_route().expect("answers"), None);
        assert_eq!(
            plain.gateway_files("http://x".to_owned()).expect("answers"),
            []
        );
        assert_eq!(plain.mcp_not_taken().expect("answers"), None);
        assert_eq!(plain.mcp_note().expect("answers"), None);
        assert_eq!(
            plain
                .sessions(SessionsRequest::new(10, 10))
                .expect("answers"),
            None
        );
        let shell = ShellRequest::new("/bin/bash", "bash", ShellFamily::Bash);
        assert_eq!(
            plain.shell_advice(shell).expect("answers"),
            ShellAdvice::cannot(NO_SHELL_SETTING)
        );
        let refused = plain
            .session_history(HistoryRequest::new("{}", "id"))
            .expect_err("it has no sessions");
        assert_eq!(refused.status, Status::GUATIAO_ERR_UNSUPPORTED);
    }

    #[test]
    fn a_listing_is_newest_first_and_only_the_newest_are_peeked() {
        let mut older = SessionFile::new("a", "/x/a", 1, 10);
        older.title = None;
        let files = alloc::vec![
            older,
            SessionFile::new("b", "/x/b", 1, 20),
            SessionFile::new("c", "/x/c", 1, 20)
        ];
        let request = SessionsRequest::new(10, 2);
        let listed = sessions::listing(files, &request, |path| (Some(path.to_owned()), None));
        let ids: Vec<&str> = listed.iter().map(|each| each.id.as_str()).collect();
        assert_eq!(ids, ["b", "c", "a"], "newest first, ties by id");
        assert_eq!(listed[0].title.as_deref(), Some("/x/b"));
        assert_eq!(listed[1].title.as_deref(), Some("/x/c"));
        assert_eq!(listed[2].title, None, "past the peek");
    }

    #[test]
    fn a_description_with_a_word_for_an_id_and_a_title_is_usable() {
        assert_eq!(VendorInfo::new("claude", "Claude").problem(), None);
        assert_eq!(VendorInfo::new("my_agent-2", "Mine").problem(), None);
    }

    #[test]
    fn an_id_that_is_no_word_or_a_missing_title_or_a_huge_document_is_refused() {
        for agent in ["", "Claude", "a b", "a/b", &"a".repeat(65)] {
            assert!(
                VendorInfo::new(agent, "T").problem().is_some(),
                "{agent:?} is no id"
            );
        }
        assert!(VendorInfo::new("a", "  ").problem().is_some());
        let mut info = VendorInfo::new("a", "A");
        info.mark = Some("x".repeat(MAX_DOCUMENT_TEXT + 1));
        assert!(info.problem().is_some_and(|why| why.contains("mark")));
        info.mark = None;
        info.look = Some("x".repeat(MAX_DOCUMENT_TEXT + 1));
        assert!(info.problem().is_some_and(|why| why.contains("look")));
    }
}
