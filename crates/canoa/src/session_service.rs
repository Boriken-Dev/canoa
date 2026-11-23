//! The `session_service` kind: something a library runs for the sessions of a host and hands to them.
//!
//! What a host gives its sessions beyond the agent itself (the person's mail, a knowledge index, the secrets an agent may use) is not
//! Canoa's to list: each is a provider of this kind. A provider describes itself (its id, what it gives a session) and the host asks
//! it three things: whether to give it to a connection's sessions now ([`SessionService::offered`]), to answer a call of one of its
//! tools ([`SessionService::call`], [`SessionService::resume`]), and to answer a control method of its own namespace
//! ([`SessionService::control`]). What a service gives a session is an open list:
//!
//! - an MCP server of `tools` the session is given, named `server`, with an optional skill that says when to use them;
//!
//! A change a tool would make that the person must allow is not asked by the provider: the call answers [`Outcome::Ask`] with what
//! the person is shown, the host puts it to the person on the client the session belongs to, and answers the provider with
//! [`SessionService::resume`].
//!
//! A service keeps nothing of the host's: its settings are a table of canoa.cfg, its secrets rest where the host keeps secrets, and
//! the other providers it uses (a mail service's backends) are the host's registries. It reaches them through the [`ServiceHost`]
//! its [`Context`] names, so one process can run several hosts, each with its own files.
//!
//! The models are plain data, the same in every language; [`ServiceHost`] is for a provider in the host's own process.

use alloc::string::String;
use alloc::sync::Arc;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::{ProviderError, Registry};
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `session_service` kind.
pub const KIND: &str = "session_service";

/// The most bytes of one skill file, so a provider cannot make a session's skills large.
pub const MAX_SKILL_BYTES: usize = 64 * 1024;

/// One file of a skill a service delivers with its tools.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SkillFile {
    /// Its path inside the skill, with `/` between folders (`SKILL.md`).
    pub path: String,
    /// What it holds, as text.
    pub text: String,
}

impl SkillFile {
    /// A skill file at `path` holding `text`.
    #[must_use]
    pub fn new(path: impl Into<String>, text: impl Into<String>) -> Self {
        Self {
            path: path.into(),
            text: text.into(),
        }
    }
}

/// One tool of the MCP server a service gives a session.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ToolInfo {
    /// The tool's name, as the agent calls it.
    pub name: String,
    /// What it does, for the agent.
    pub description: String,
    /// The JSON Schema of its arguments, as JSON text of an object.
    pub input_schema: String,
}

impl ToolInfo {
    /// A tool called `name` that `description` says, taking the arguments `input_schema` (JSON text) describes.
    #[must_use]
    pub fn new(
        name: impl Into<String>,
        description: impl Into<String>,
        input_schema: impl Into<String>,
    ) -> Self {
        Self {
            name: name.into(),
            description: description.into(),
            input_schema: input_schema.into(),
        }
    }
}

/// What a service is and what it gives a session, from [`SessionService::describe`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ServiceInfo {
    /// The id it is found by: a word of lower-case letters, digits and `_`, 1 to 32. It is the key of the service's table in canoa.cfg
    /// (`[host.services.ID]`) and the namespace of its control methods (`ID/accounts`).
    pub id: String,
    /// Its name for people.
    pub title: String,
    /// The name of the MCP server a session is given, a word of lower-case letters, digits, `-` and `_`; none when the service gives
    /// no tools.
    pub server: Option<String>,
    /// The tools of that server.
    #[map(default)]
    pub tools: Vec<ToolInfo>,
    /// The name of the skill delivered with the tools, a word like `server`; none for no skill.
    pub skill: Option<String>,
    /// The files of that skill, `SKILL.md` among them.
    #[map(default)]
    pub skill_files: Vec<SkillFile>,
}

impl ServiceInfo {
    /// A service that gives nothing yet.
    #[must_use]
    pub fn new(id: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: title.into(),
            server: None,
            tools: Vec::new(),
            skill: None,
            skill_files: Vec::new(),
        }
    }

    /// Why this provider cannot be offered, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        if !is_id(&self.id) {
            return Some(alloc::format!(
                "the service id {:?} is not 1 to 32 lower-case letters, digits and _",
                self.id
            ));
        }
        if self.title.trim().is_empty() {
            return Some(alloc::format!("the service {:?} has no title", self.id));
        }
        if let Some(server) = &self.server
            && !is_name(server)
        {
            return Some(alloc::format!(
                "the server name {server:?} of the service {:?} is not 1 to 32 lower-case letters, digits, - and _",
                self.id
            ));
        }
        if self.server.is_none() && !self.tools.is_empty() {
            return Some(alloc::format!(
                "the service {:?} has tools and no server to carry them",
                self.id
            ));
        }
        let mut seen: Vec<&str> = Vec::new();
        for tool in &self.tools {
            if tool.name.trim().is_empty() || seen.contains(&tool.name.as_str()) {
                return Some(alloc::format!(
                    "the service {:?} names a tool twice or with no name",
                    self.id
                ));
            }
            seen.push(&tool.name);
            if serde_json::from_str::<serde_json::Value>(&tool.input_schema)
                .map_or(true, |schema| !schema.is_object())
            {
                return Some(alloc::format!(
                    "the input schema of the tool {:?} is not a JSON object",
                    tool.name
                ));
            }
        }
        if let Some(skill) = &self.skill {
            if !is_name(skill) {
                return Some(alloc::format!(
                    "the skill name {skill:?} of the service {:?} is not 1 to 32 lower-case letters, digits, - and _",
                    self.id
                ));
            }
            if !self.skill_files.iter().any(|file| file.path == "SKILL.md") {
                return Some(alloc::format!("the skill {skill:?} has no SKILL.md"));
            }
        }
        for file in &self.skill_files {
            if file.text.len() > MAX_SKILL_BYTES
                || file.path.is_empty()
                || file.path.starts_with('/')
                || file
                    .path
                    .split('/')
                    .any(|part| part.is_empty() || part == "..")
            {
                return Some(alloc::format!(
                    "the skill file {:?} of the service {:?} has a path that leaves the skill or is too large",
                    file.path,
                    self.id
                ));
            }
        }
        None
    }
}

/// Whether `text` is a service id: 1 to 32 lower-case letters, digits and `_`.
#[must_use]
pub fn is_id(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text
            .bytes()
            .all(|byte| byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_')
}

fn is_name(text: &str) -> bool {
    !text.is_empty()
        && text.len() <= 32
        && text.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

/// Which host a call is for: the name [`register`] gave its [`ServiceHost`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Context {
    /// The host's handle, which [`host`] turns into its [`ServiceHost`].
    pub host: String,
}

impl Context {
    /// The context of the host registered as `host`.
    #[must_use]
    pub fn new(host: impl Into<String>) -> Self {
        Self { host: host.into() }
    }
}

/// A call of one tool.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ToolCall {
    /// The tool's name.
    pub tool: String,
    /// Its arguments, as JSON text.
    pub arguments: String,
}

impl ToolCall {
    /// A call of `tool` with `arguments` (JSON text).
    #[must_use]
    pub fn new(tool: impl Into<String>, arguments: impl Into<String>) -> Self {
        Self {
            tool: tool.into(),
            arguments: arguments.into(),
        }
    }
}

/// What the person answered to a question a tool put.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Answer {
    /// The person said yes.
    Allowed,
    /// Nobody was asked: the person put the session in a mode that asks for nothing.
    Silent,
    /// The person said no.
    Declined,
}

/// A call that goes on once the person answered: [`SessionService::resume`].
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Resume {
    /// The tool's name.
    pub tool: String,
    /// The `state` of the [`Question`] the person was asked.
    pub state: String,
    /// What they answered.
    pub answer: Answer,
}

impl Resume {
    /// The call of `tool` that `state` kept, answered with `answer`.
    #[must_use]
    pub fn new(tool: impl Into<String>, state: impl Into<String>, answer: Answer) -> Self {
        Self {
            tool: tool.into(),
            state: state.into(),
            answer,
        }
    }
}

/// What the person is asked before a tool goes on.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Question {
    /// The change in a line.
    pub title: String,
    /// The change in plain words, as the person reads it.
    pub text: String,
    /// What the question is bound to (a digest of the change), so an answer cannot be taken for another's.
    pub digest: String,
    /// What the provider needs to go on, handed back by [`SessionService::resume`]; the host keeps it and reads none of it.
    pub state: String,
}

impl Question {
    /// A question titled `title`, saying `text`, bound to `digest`, which goes on from `state`.
    #[must_use]
    pub fn new(
        title: impl Into<String>,
        text: impl Into<String>,
        digest: impl Into<String>,
        state: impl Into<String>,
    ) -> Self {
        Self {
            title: title.into(),
            text: text.into(),
            digest: digest.into(),
            state: state.into(),
        }
    }
}

/// How a tool call went: the answer, or a question the person must answer first.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Outcome {
    /// What `tools/call` answers, as JSON text (`{"content": [...], "isError": false}`); none while `ask` is set.
    pub result: Option<String>,
    /// What to say in the session, in the person's view, besides the result.
    pub note: Option<String>,
    /// A question to put to the person before the call goes on.
    pub ask: Option<Question>,
}

impl Outcome {
    /// A finished call that answered `result` (JSON text).
    #[must_use]
    pub fn done(result: impl Into<String>) -> Self {
        Self {
            result: Some(result.into()),
            note: None,
            ask: None,
        }
    }

    /// A call that waits for the person to answer `question`.
    #[must_use]
    pub fn asking(question: Question) -> Self {
        Self {
            result: None,
            note: None,
            ask: Some(question),
        }
    }

    /// This outcome with `note` said in the session.
    #[must_use]
    pub fn noting(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }
}

/// A control method of the service's namespace.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ControlCall {
    /// The method without the namespace (`accounts` for `mail/accounts`).
    pub method: String,
    /// Its parameters, as JSON text.
    pub params: String,
}

impl ControlCall {
    /// A call of `method` with `params` (JSON text).
    #[must_use]
    pub fn new(method: impl Into<String>, params: impl Into<String>) -> Self {
        Self {
            method: method.into(),
            params: params.into(),
        }
    }
}

/// Why a control method did not answer.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Refusal {
    /// The parameters are not what the method takes.
    BadParams,
    /// What the method names is not there.
    NotFound,
    /// The method understood and would not.
    Refused,
}

/// A control method's answer: a result as JSON text, or why there is none.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ControlReply {
    /// The result, as JSON text.
    pub result: Option<String>,
    /// Why there is none.
    pub refusal: Option<Refusal>,
    /// What to tell the person when there is none.
    pub message: Option<String>,
}

impl ControlReply {
    /// An answer, `result` being JSON text.
    #[must_use]
    pub fn ok(result: impl Into<String>) -> Self {
        Self {
            result: Some(result.into()),
            refusal: None,
            message: None,
        }
    }

    /// A refusal for `why`, said as `message`.
    #[must_use]
    pub fn refused(why: Refusal, message: impl Into<String>) -> Self {
        Self {
            result: None,
            refusal: Some(why),
            message: Some(message.into()),
        }
    }
}

/// A service a library runs for the sessions of a host.
///
/// `describe` answers the same on every call with nothing run. The other methods are asked of the host's own process, one call at a
/// time per tool call, and may take as long as a call to a server does.
#[coqui_acp_api::guatiao::kind(name = "session_service", crate = ::coqui_acp_api::guatiao)]
pub trait SessionService: Send + Sync {
    /// What this service is and what it gives a session.
    fn describe(&self) -> Result<ServiceInfo, ProviderError>;

    /// Whether the sessions of a connection that may have it are given the service now: a mail service with no account to read has
    /// nothing to give.
    fn offered(&self, context: Context) -> Result<bool, ProviderError>;

    /// Runs a tool, or says what the person must allow first.
    fn call(&self, context: Context, call: ToolCall) -> Result<Outcome, ProviderError>;

    /// Goes on with a tool the person was asked about.
    fn resume(&self, context: Context, resume: Resume) -> Result<Outcome, ProviderError>;

    /// Answers one control method of the service's namespace.
    fn control(&self, context: Context, call: ControlCall) -> Result<ControlReply, ProviderError>;
}

/// What a provider in the host's process reaches of its host: the host's own files, secrets and providers.
///
/// A provider is handed a [`Context`], not a host, so one process can run several (a test's, each with files of its own): [`host`]
/// finds the one the context names.
pub trait ServiceHost: Send + Sync {
    /// The table of canoa.cfg that is `service`'s own, as the text of a JSON object whose members keep the file's order; `{}` when
    /// the file has none.
    ///
    /// # Errors
    ///
    /// Why canoa.cfg cannot be read.
    fn config(&self, service: &str) -> Result<String, String>;

    /// Replaces the table `path` names inside `service`'s own table (the whole table when `path` is empty) with `table`, the text of
    /// a JSON object, or takes it out for `None`, after checking that the file would still be read. The rest of the file, its
    /// comments included, is left as it is.
    ///
    /// # Errors
    ///
    /// Why canoa.cfg cannot be changed; nothing is then.
    fn set_config(&self, service: &str, path: &[&str], table: Option<&str>) -> Result<(), String>;

    /// The secret `key` the host keeps under `id`, or `None` when none is kept.
    ///
    /// # Errors
    ///
    /// Why the host's store cannot be read, or has none.
    fn secret(&self, id: &str, key: &str) -> Result<Option<String>, String>;

    /// Keeps `value` as the secret `key` under `id`.
    ///
    /// # Errors
    ///
    /// Why the host cannot keep it.
    fn put_secret(&self, id: &str, key: &str, value: &str) -> Result<(), String>;

    /// Forgets every secret kept under `id`.
    ///
    /// # Errors
    ///
    /// Why the host cannot forget them.
    fn forget_secrets(&self, id: &str) -> Result<(), String>;

    /// A line saying where secrets are kept when that is worth the person knowing (a file store, not the system's), or `None`.
    fn secrets_note(&self) -> Option<String>;

    /// The registries of providers this host loaded: the libraries it carries and the ones canoa.cfg adds.
    fn registries(&self) -> Vec<Arc<Registry>>;

    /// A new id for a secret: a random UUID in lower-case hex.
    ///
    /// # Errors
    ///
    /// When the system has no randomness.
    fn new_id(&self) -> Result<String, String>;
}

type Hosts = std::sync::Mutex<alloc::collections::BTreeMap<String, Arc<dyn ServiceHost>>>;

static HOSTS: Hosts = std::sync::Mutex::new(alloc::collections::BTreeMap::new());

fn hosts()
-> std::sync::MutexGuard<'static, alloc::collections::BTreeMap<String, Arc<dyn ServiceHost>>> {
    HOSTS
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner)
}

/// A host made known to the providers of its process, until this is dropped.
#[derive(Debug)]
pub struct Registration {
    handle: String,
}

impl Registration {
    /// The context calls to providers carry for this host.
    #[must_use]
    pub fn context(&self) -> Context {
        Context::new(self.handle.clone())
    }
}

impl Drop for Registration {
    fn drop(&mut self) {
        hosts().remove(&self.handle);
    }
}

/// Makes `host` known to this process's providers under a handle of its own.
#[must_use]
pub fn register(host: Arc<dyn ServiceHost>) -> Registration {
    static NEXT: core::sync::atomic::AtomicU64 = core::sync::atomic::AtomicU64::new(1);
    let handle = alloc::format!(
        "host-{}",
        NEXT.fetch_add(1, core::sync::atomic::Ordering::Relaxed)
    );
    hosts().insert(handle.clone(), host);
    Registration { handle }
}

/// The host `context` names, or `None` when it is gone.
#[must_use]
pub fn host(context: &Context) -> Option<Arc<dyn ServiceHost>> {
    hosts().get(&context.host).cloned()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn tool() -> ToolInfo {
        ToolInfo::new("mail_list", "Lists.", "{\"type\":\"object\"}")
    }

    #[test]
    fn a_service_with_a_word_for_an_id_a_server_and_tools_is_usable() {
        let mut info = ServiceInfo::new("mail", "Mail");
        assert_eq!(info.problem(), None);
        info.server = Some("canoa-mail".into());
        info.tools.push(tool());
        info.skill = Some("canoa-mail".into());
        info.skill_files.push(SkillFile::new("SKILL.md", "text"));
        assert_eq!(info.problem(), None);
    }

    #[test]
    fn a_service_that_is_no_word_or_has_tools_without_a_server_a_repeated_tool_or_a_skill_that_leaves_its_folder_is_refused()
     {
        for id in ["", "Mail", "a b", &"a".repeat(33)] {
            assert!(ServiceInfo::new(id, "T").problem().is_some(), "{id:?}");
        }
        assert!(ServiceInfo::new("a", " ").problem().is_some());
        let mut info = ServiceInfo::new("a", "A");
        info.tools.push(tool());
        assert!(info.problem().is_some_and(|why| why.contains("no server")));
        info.server = Some("a".into());
        info.tools.push(tool());
        assert!(info.problem().is_some_and(|why| why.contains("twice")));
        info.tools.pop();
        info.tools[0].input_schema = "[]".into();
        assert!(
            info.problem()
                .is_some_and(|why| why.contains("not a JSON object"))
        );
        let mut info = ServiceInfo::new("a", "A");
        info.skill = Some("s".into());
        assert!(info.problem().is_some_and(|why| why.contains("SKILL.md")));
        info.skill_files.push(SkillFile::new("../SKILL.md", ""));
        assert!(info.problem().is_some());
    }

    #[test]
    fn a_registered_host_is_found_by_its_context_and_gone_with_its_registration() {
        struct None_;
        impl ServiceHost for None_ {
            fn config(&self, _: &str) -> Result<String, String> {
                Ok("{}".into())
            }
            fn set_config(&self, _: &str, _: &[&str], _: Option<&str>) -> Result<(), String> {
                Ok(())
            }
            fn secret(&self, _: &str, _: &str) -> Result<Option<String>, String> {
                Ok(None)
            }
            fn put_secret(&self, _: &str, _: &str, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn forget_secrets(&self, _: &str) -> Result<(), String> {
                Ok(())
            }
            fn secrets_note(&self) -> Option<String> {
                None
            }
            fn registries(&self) -> Vec<Arc<Registry>> {
                Vec::new()
            }
            fn new_id(&self) -> Result<String, String> {
                Ok("id".into())
            }
        }
        let registration = register(Arc::new(None_));
        let context = registration.context();
        assert!(host(&context).is_some());
        let other = register(Arc::new(None_));
        assert_ne!(other.context(), context);
        drop(registration);
        assert!(host(&context).is_none());
        assert!(host(&other.context()).is_some());
    }
}
