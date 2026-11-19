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
use coqui_acp_api::guatiao::{FromValue, Schema, Status, ToValue};

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
            standards: Vec::new(),
            names: Vec::new(),
            markers: Vec::new(),
            modes: Vec::new(),
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

/// A shell a person chose, as an agent is asked to use it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ShellRequest {
    /// The shell program's path.
    pub path: String,
    /// The program's file name without its extension, in lower case (`bash`, `pwsh`).
    pub name: String,
    /// Its family: `bash`, `zsh`, `powershell`, `cmd`, or `other`.
    pub family: String,
    /// Whether it is the one the Windows Subsystem for Linux starts.
    pub wsl: bool,
    /// Whether the host runs on Windows.
    pub windows: bool,
}

impl ShellRequest {
    /// A request for the shell at `path`, called `name`, of `family`, with neither the subsystem nor Windows.
    #[must_use]
    pub fn new(
        path: impl Into<String>,
        name: impl Into<String>,
        family: impl Into<String>,
    ) -> Self {
        Self {
            path: path.into(),
            name: name.into(),
            family: family.into(),
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
                .sessions(SessionsRequest::new(10, 10))
                .expect("answers"),
            None
        );
        let shell = ShellRequest::new("/bin/bash", "bash", "bash");
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
