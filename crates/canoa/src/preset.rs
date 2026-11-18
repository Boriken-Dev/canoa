//! [`SourcePreset`]: how the host starts one coding agent it knows about when `canoa.cfg` does not name it.

/// What one built-in source is: a harness the host runs in process, or a program that speaks ACP on its standard streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourcePreset {
    /// The source's name, as a client lists it.
    pub name: &'static str,
    /// The id of the agent it starts, as its `agent_vendor` description names it (`claude`): empty for the lower-case name.
    pub agent: &'static str,
    /// The id of an `acp_harness` provider for a harness; `None` for a program, which `program` and `args` run.
    pub harness: Option<&'static str>,
    /// The program's file name, looked for on `PATH` and in `search`.
    pub program: &'static str,
    /// Arguments that make `program` speak ACP.
    pub args: &'static [&'static str],
    /// Other programs it needs, offered only when found.
    pub requires: &'static [&'static str],
    /// Where the program's own installer leaves it, tried when `PATH` has none: `~` is the home directory and `%NAME%` an
    /// environment variable.
    pub search: &'static [&'static str],
    /// The usage reader that reports this program's quota, when the host has one.
    pub usage: Option<&'static str>,
    /// The program store whose managed copy runs in place of a program found on the computer, when it has one.
    pub managed: Option<&'static str>,
}

impl SourcePreset {
    /// The id of the agent the source starts: [`agent`](Self::agent), else the source's name in lower case.
    #[must_use]
    pub fn agent_id(&self) -> alloc::string::String {
        if self.agent.is_empty() {
            self.name.to_lowercase()
        } else {
            self.agent.to_owned()
        }
    }

    /// A source called `name` that runs `program` with no arguments.
    #[must_use]
    pub const fn new(name: &'static str, program: &'static str) -> Self {
        Self {
            name,
            agent: "",
            harness: None,
            program,
            args: &[],
            requires: &[],
            search: &[],
            usage: None,
            managed: None,
        }
    }

    /// The agent it starts goes by `agent`, when that is not its name in lower case.
    #[must_use]
    pub const fn agent(mut self, agent: &'static str) -> Self {
        self.agent = agent;
        self
    }

    /// Run in process as the built-in harness `harness`.
    #[must_use]
    pub const fn harness(mut self, harness: &'static str) -> Self {
        self.harness = Some(harness);
        self
    }

    /// Started with `args`.
    #[must_use]
    pub const fn args(mut self, args: &'static [&'static str]) -> Self {
        self.args = args;
        self
    }

    /// Offered only when each of `requires` is found as well.
    #[must_use]
    pub const fn requires(mut self, requires: &'static [&'static str]) -> Self {
        self.requires = requires;
        self
    }

    /// Looked for in `search` when `PATH` has no copy.
    #[must_use]
    pub const fn search(mut self, search: &'static [&'static str]) -> Self {
        self.search = search;
        self
    }

    /// Its quota read by the usage reader `usage`.
    #[must_use]
    pub const fn usage(mut self, usage: &'static str) -> Self {
        self.usage = Some(usage);
        self
    }

    /// Run from the managed copy of the program store `store` when that has one.
    #[must_use]
    pub const fn managed(mut self, store: &'static str) -> Self {
        self.managed = Some(store);
        self
    }
}
