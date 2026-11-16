//! [`SourcePreset`]: how the host starts one coding agent it knows about when `canoa.cfg` does not name it.

/// What one built-in source is: a harness the host runs in process, or a program that speaks ACP on its standard streams.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
#[non_exhaustive]
pub struct SourcePreset {
    /// The source's name, as a client lists it.
    pub name: &'static str,
    /// `claude`, `codex` or `pi` for a built-in harness; `None` for a program, which `program` and `args` run.
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
    /// A source called `name` that runs `program` with no arguments.
    #[must_use]
    pub const fn new(name: &'static str, program: &'static str) -> Self {
        Self {
            name,
            harness: None,
            program,
            args: &[],
            requires: &[],
            search: &[],
            usage: None,
            managed: None,
        }
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
