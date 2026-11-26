//! The `approver` kind: somewhere an agent's permission request can be answered from, besides the window that runs the session.
//!
//! A rule the person wrote ([`Authority::Policy`]) and a phone the person answers on ([`Authority::Person`]) are both providers of this
//! kind, the ones Canoa carries and the ones an extension adds alike. The host puts a request to each approver the person
//! configured and answers the agent with the first answer; an approver only ever says [`Choice::Allow`] or [`Choice::Reject`], and the host
//! maps that onto the options the agent offered (allow once, reject once), so an approver never names an option and never
//! answers "always".
//!
//! An approver that can answer at once ([`ApproverInfo::instant`]) does so from [`Approver::ask`]. One that has to wait for someone
//! answers [`Answer::waiting`] with a ticket, and the host polls [`Approver::wait`] with it until an answer, its own timeout, or
//! [`Approver::withdraw`] when the request was answered elsewhere. An instant approver answers `wait` with [`Answer::abstain`] and
//! `withdraw` with nothing.
//!
//! The text of a request is the agent's: the host strips control characters and cuts what it shows ([`Request::clean`]) before any
//! approver sees it. The models are plain data, the same in every language.

use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Schema, ToValue};

/// The name of the `approver` kind.
pub const KIND: &str = "approver";

/// The most characters of a request's title or command an approver is shown.
pub const MAX_TEXT: usize = 300;

/// The most paths of a request an approver is shown.
pub const MAX_PATHS: usize = 32;

/// Whose answer an approver's allow is.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Authority {
    /// The person answers (a phone): its allow stands for the person's own, so it applies to a protected place too.
    Person,
    /// A rule the person wrote answers: its allow never applies to a protected place, a request to widen the sandbox or a call the
    /// host cannot read, which go on to the person.
    Policy,
}

/// What an approver is, from [`Approver::describe`]. Cheap, and the same on every call.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct ApproverInfo {
    /// Its name for people.
    pub title: String,
    /// A line saying what it is.
    pub help: Option<String>,
    /// Whose answer its allow is.
    pub answers_as: Authority,
    /// Whether it answers from `ask` alone: an instant approver is asked first and in order, and never waits.
    pub instant: bool,
    /// Whether it could use the command and the files of a call (a rule that reads them). Only a statement: the host gives them to an
    /// approver when its entry says `detail = "command"`, and never because the provider asks, so one that says so is told nothing more
    /// and the host logs that its entry could widen it.
    #[map(default)]
    pub wants_detail: bool,
}

impl ApproverInfo {
    /// An approver called `title` whose answers are `answers_as`.
    #[must_use]
    pub fn new(title: impl Into<String>, answers_as: Authority, instant: bool) -> Self {
        Self {
            title: title.into(),
            help: None,
            answers_as,
            instant,
            wants_detail: false,
        }
    }

    /// Why this description cannot be used, or `None` when it can.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        self.title
            .trim()
            .is_empty()
            .then(|| String::from("an approver has no title"))
    }
}

/// The session a request is from.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct SessionRef {
    /// The host's own id for the session.
    pub id: String,
    /// What the person calls it, when it has a name.
    pub title: Option<String>,
    /// The agent it runs.
    pub agent: String,
}

impl SessionRef {
    /// The session `id` of `agent`.
    #[must_use]
    pub fn new(id: impl Into<String>, agent: impl Into<String>) -> Self {
        Self {
            id: id.into(),
            title: None,
            agent: agent.into(),
        }
    }
}

/// The tool call a request is about.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq, Default)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Tool {
    /// The agent's kind of call (`read`, `edit`, `execute`, ...).
    pub kind: String,
    /// The tool's own name.
    pub name: String,
    /// The call's title, on one line.
    pub title: String,
    /// The command, for an approver whose settings ask for the detail; absent otherwise.
    pub command: Option<String>,
    /// The files the call names, for an approver whose settings ask for the detail; empty otherwise.
    #[map(default)]
    pub paths: Vec<String>,
}

impl Tool {
    /// A call of `kind` to the tool `name`.
    #[must_use]
    pub fn new(kind: impl Into<String>, name: impl Into<String>, title: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
            title: title.into(),
            command: None,
            paths: Vec::new(),
        }
    }
}

/// An option the agent offered, without the id the host answers with.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct OptionRef {
    /// The agent's kind (`allow_once`, `allow_always`, `reject_once`, `reject_always`).
    pub kind: String,
    /// Its name for people.
    pub name: String,
}

impl OptionRef {
    /// An option of `kind` that people know as `name`.
    #[must_use]
    pub fn new(kind: impl Into<String>, name: impl Into<String>) -> Self {
        Self {
            kind: kind.into(),
            name: name.into(),
        }
    }
}

/// A permission request, as an approver is shown it.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Request {
    /// The host's id for this request, which names it in a reply.
    pub id: String,
    /// The session it is from.
    pub session: SessionRef,
    /// The call it is about.
    pub tool: Tool,
    /// What the agent offered.
    #[map(default)]
    pub options: Vec<OptionRef>,
    /// When the host stops waiting for this approver, in Unix seconds; absent when it waits until the session ends.
    pub deadline: Option<i64>,
    /// The host's own verdict: the call names a place the person protects, asks to widen the sandbox, or is not one the host can
    /// read. A [`Authority::Policy`] approver must not allow it.
    pub protected: bool,
}

impl Request {
    /// A request `id` for `tool` in `session`.
    #[must_use]
    pub fn new(id: impl Into<String>, session: SessionRef, tool: Tool) -> Self {
        Self {
            id: id.into(),
            session,
            tool,
            options: Vec::new(),
            deadline: None,
            protected: false,
        }
    }

    /// This request with every text the agent wrote made safe to show: control, direction and zero-width characters removed, the title
    /// and the command cut at [`MAX_TEXT`] characters and the paths at [`MAX_PATHS`].
    #[must_use]
    pub fn clean(mut self) -> Self {
        self.tool.kind = clean_text(&self.tool.kind, 64);
        self.tool.name = clean_text(&self.tool.name, 256);
        self.tool.title = clean_text(&self.tool.title, MAX_TEXT);
        self.tool.command = self
            .tool
            .command
            .map(|command| clean_text(&command, MAX_TEXT));
        self.tool.paths = self
            .tool
            .paths
            .iter()
            .take(MAX_PATHS)
            .map(|path| clean_text(path, 512))
            .collect();
        self.session.title = self.session.title.map(|title| clean_text(&title, 120));
        self
    }
}

/// `text` on one line with the characters that could pass for something else removed, cut at `max` characters.
fn clean_text(text: &str, max: usize) -> String {
    let line = text.split(['\r', '\n']).next().unwrap_or_default();
    line.chars()
        .filter(|c| !unsafe_char(*c))
        .take(max)
        .collect()
}

fn unsafe_char(c: char) -> bool {
    c.is_control()
        || matches!(
            c,
            '\u{200b}'..='\u{200f}'
                | '\u{202a}'..='\u{202e}'
                | '\u{2060}'..='\u{2064}'
                | '\u{2066}'..='\u{2069}'
                | '\u{feff}'
        )
}

/// What an approver says about a request.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Choice {
    /// Allow this call once.
    Allow,
    /// Refuse this call once.
    Reject,
}

/// Where an approver is with a request.
#[derive(ToValue, FromValue, Schema, Debug, Clone, Copy, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao, rename_all = "snake_case")]
pub enum Step {
    /// It has answered: [`Answer::choice`] says how.
    Answered,
    /// It waits for someone: [`Answer::ticket`] names the wait.
    Waiting,
    /// It has no answer and will not give one: a rule no request matched, a notice that could not be sent, a ticket that is gone.
    Abstain,
}

/// An approver's answer to [`Approver::ask`], or to [`Approver::wait`] (where [`Step::Abstain`] means the ticket is gone).
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Answer {
    /// Where the approver is.
    pub step: Step,
    /// For [`Step::Answered`], what it said.
    pub choice: Option<Choice>,
    /// For [`Step::Waiting`] from `ask`, what `wait` and `withdraw` name the wait by.
    pub ticket: Option<String>,
    /// One line for the host's log, never shown to the agent and never holding the request's text.
    pub note: Option<String>,
}

impl Answer {
    fn of(step: Step) -> Self {
        Self {
            step,
            choice: None,
            ticket: None,
            note: None,
        }
    }

    /// Allow this call once.
    #[must_use]
    pub fn allow() -> Self {
        Self {
            choice: Some(Choice::Allow),
            ..Self::of(Step::Answered)
        }
    }

    /// Refuse this call once.
    #[must_use]
    pub fn reject() -> Self {
        Self {
            choice: Some(Choice::Reject),
            ..Self::of(Step::Answered)
        }
    }

    /// Still waiting, under `ticket`.
    #[must_use]
    pub fn waiting(ticket: impl Into<String>) -> Self {
        Self {
            ticket: Some(ticket.into()),
            ..Self::of(Step::Waiting)
        }
    }

    /// No answer, for the reason `why`.
    #[must_use]
    pub fn abstain(why: impl Into<String>) -> Self {
        Self {
            note: Some(why.into()),
            ..Self::of(Step::Abstain)
        }
    }

    /// This answer with a line for the log.
    #[must_use]
    pub fn noted(mut self, note: impl Into<String>) -> Self {
        self.note = Some(note.into());
        self
    }

    /// Why this answer cannot be used, or `None` when it can: an answer names its choice, a wait its ticket.
    #[must_use]
    pub fn problem(&self) -> Option<String> {
        match self.step {
            Step::Answered if self.choice.is_none() => {
                Some(String::from("an answer names no choice"))
            }
            Step::Waiting if self.ticket.as_deref().is_none_or(str::is_empty) => {
                Some(String::from("a wait names no ticket"))
            }
            _ => None,
        }
    }
}

/// A wait to look at again.
#[derive(ToValue, FromValue, Schema, Debug, Clone, PartialEq, Eq)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[non_exhaustive]
pub struct Ticket {
    /// What [`Approver::ask`] gave.
    pub ticket: String,
    /// The most milliseconds [`Approver::wait`] may take to answer: the host's poll interval, at most 5000.
    pub wait_ms: u64,
}

impl Ticket {
    /// The wait `ticket`, looked at for up to `wait_ms` milliseconds.
    #[must_use]
    pub fn new(ticket: impl Into<String>, wait_ms: u64) -> Self {
        Self {
            ticket: ticket.into(),
            wait_ms,
        }
    }
}

/// The most milliseconds a [`Approver::wait`] is asked to take.
pub const MAX_WAIT_MS: u64 = 5000;

/// An approver. One instance is one configuration of one provider (an entry of `[permissions.approvers]`); its settings are the
/// provider's own schema.
///
/// An error from `ask` or `wait` is an abstain: the host logs it and goes on to the others. It never answers.
#[coqui_acp_api::guatiao::kind(name = "approver", crate = ::coqui_acp_api::guatiao)]
pub trait Approver: Send + Sync {
    /// What it is. Cheap, and the same on every call.
    fn describe(&self) -> Result<ApproverInfo, ProviderError>;

    /// Puts `request` to it, and returns at once: an answer, a ticket to wait on, or an abstain.
    fn ask(&self, request: Request) -> Result<Answer, ProviderError>;

    /// Looks at a wait it gave a ticket for, for up to `wait` milliseconds: still waiting, an answer, or an abstain for a ticket that
    /// is gone. An instant approver answers an abstain.
    fn wait(&self, wait: Ticket) -> Result<Answer, ProviderError>;

    /// Ends a wait that is no longer needed (the request was answered elsewhere, or the turn ended); ending one that is gone is no
    /// error.
    fn withdraw(&self, ticket: Ticket) -> Result<(), ProviderError>;
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_agents_text_is_made_safe_before_an_approver_sees_it() {
        let mut tool = Tool::new("execute", "Bash\u{202e}", "run it\u{200b}\nsecond line");
        tool.command = Some("a".repeat(1000));
        tool.paths = (0..100).map(|n| alloc::format!("/f{n}\u{7}")).collect();
        let request = Request::new("r1", SessionRef::new("s", "claude"), tool).clean();
        assert_eq!(request.tool.name, "Bash");
        assert_eq!(request.tool.title, "run it");
        assert_eq!(
            request.tool.command.map(|c| c.chars().count()),
            Some(MAX_TEXT)
        );
        assert_eq!(request.tool.paths.len(), MAX_PATHS);
        assert_eq!(request.tool.paths[1], "/f1");
    }

    #[test]
    fn an_answer_names_what_its_step_needs() {
        assert_eq!(Answer::allow().problem(), None);
        assert_eq!(Answer::waiting("t").problem(), None);
        assert_eq!(Answer::abstain("no rule").problem(), None);
        let mut bad = Answer::allow();
        bad.choice = None;
        assert!(bad.problem().is_some());
        assert!(Answer::waiting("").problem().is_some());
    }
}
