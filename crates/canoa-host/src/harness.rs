//! Claude Code and Codex as ACP agents, through coqui-acp's translators of their own stdio protocols, with no Node: what
//! `canoa agent` serves on its standard streams, and what `serve` runs in this process for a harness entry.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "processes, paths and environments are std's"
)]

use std::ffi::OsString;
use std::path::PathBuf;
use std::sync::Arc;

use coqui_acp_claude::{ClaudeAgent, ClaudeLaunch};
use coqui_acp_codex::{CodexAgent, CodexLaunch};
use coqui_acp_harness::{Harness as Translator, Settings};

/// A coding agent's command line that the host serves as an ACP agent itself.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[non_exhaustive]
pub enum Harness {
    /// Claude Code, `claude`, over stream-json.
    Claude,
    /// Codex, `codex app-server`, over its JSON-RPC.
    Codex,
}

impl Harness {
    /// Its name in canoa.cfg and on `canoa agent`'s command line.
    #[must_use]
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Claude => "claude",
            Self::Codex => "codex",
        }
    }

    /// The harness of that name.
    #[must_use]
    pub fn named(name: &str) -> Option<Self> {
        match name {
            "claude" => Some(Self::Claude),
            "codex" => Some(Self::Codex),
            _ => None,
        }
    }
}

/// How one harness agent runs: what its translator starts the command line with, and what it takes from a client.
#[derive(Clone, Debug)]
pub(crate) struct AgentOptions {
    pub(crate) harness: Harness,
    /// The command line's program; its name looked up on `PATH` when `None`.
    pub(crate) executable: Option<OsString>,
    pub(crate) model: Option<String>,
    pub(crate) effort: Option<String>,
    /// Claude Code's `--permission-mode`.
    pub(crate) permission_mode: Option<String>,
    /// Claude Code's `--setting-sources`; `None` is its own default.
    pub(crate) setting_sources: Option<Vec<String>>,
    /// Claude Code's `--strict-mcp-config`.
    pub(crate) strict_mcp_config: bool,
    /// Codex's approval policy.
    pub(crate) approval_policy: Option<String>,
    /// Codex's sandbox.
    pub(crate) sandbox: Option<String>,
    /// Claude Code's permission prompts asked through ACP, rather than refused where nothing granted them.
    pub(crate) permission_prompts: bool,
    /// Claude Code plugins named in a session's `_meta`; their hooks and MCP servers run with nothing asked.
    pub(crate) client_plugins: bool,
    /// Directories a session's `cwd`, more directories and plugins must lie in; none takes any.
    pub(crate) working_roots: Vec<PathBuf>,
    /// Directories a resumed Codex thread's own directory must lie in; none takes any.
    pub(crate) resume_roots: Vec<PathBuf>,
    pub(crate) client_mcp_servers: bool,
    pub(crate) network_paths: bool,
    pub(crate) max_version: u8,
    pub(crate) max_sessions: usize,
    /// Added to the environment the command line inherits.
    pub(crate) env: Vec<(OsString, OsString)>,
}

impl AgentOptions {
    /// The translator's own defaults: prompts asked through ACP, no client plugins, any directory, ACP version 2 at most.
    pub(crate) const fn new(harness: Harness) -> Self {
        Self {
            harness,
            executable: None,
            model: None,
            effort: None,
            permission_mode: None,
            setting_sources: None,
            strict_mcp_config: false,
            approval_policy: None,
            sandbox: None,
            permission_prompts: true,
            client_plugins: false,
            working_roots: Vec::new(),
            resume_roots: Vec::new(),
            client_mcp_servers: true,
            network_paths: false,
            max_version: 2,
            max_sessions: crate::config::DEFAULT_MAX_SESSIONS,
            env: Vec::new(),
        }
    }

    /// The translator, ready to open sessions.
    pub(crate) fn agent(&self) -> Arc<dyn Translator> {
        match self.harness {
            Harness::Claude => Arc::new(
                ClaudeAgent::new()
                    .defaults(self.claude_launch())
                    .client_plugins(self.client_plugins),
            ),
            Harness::Codex => Arc::new(CodexAgent::new().defaults(self.codex_launch())),
        }
    }

    /// What every Claude Code session starts from, before a session's own parts are added.
    pub(crate) fn claude_launch(&self) -> ClaudeLaunch {
        let mut launch = ClaudeLaunch::new(".");
        if let Some(executable) = &self.executable {
            launch.executable.clone_from(executable);
        }
        launch.model.clone_from(&self.model);
        launch.effort.clone_from(&self.effort);
        launch.permission_mode.clone_from(&self.permission_mode);
        launch.setting_sources.clone_from(&self.setting_sources);
        launch.strict_mcp_config = self.strict_mcp_config;
        launch.permission_prompts = self.permission_prompts;
        launch.env.extend(self.env.iter().cloned());
        launch
    }

    /// What every Codex thread starts from, before a session's own parts are added.
    pub(crate) fn codex_launch(&self) -> CodexLaunch {
        let mut launch = CodexLaunch::new(".");
        if let Some(executable) = &self.executable {
            launch.executable.clone_from(executable);
        }
        launch.model.clone_from(&self.model);
        launch.effort.clone_from(&self.effort);
        launch.approval_policy.clone_from(&self.approval_policy);
        launch.sandbox.clone_from(&self.sandbox);
        launch.working_roots.clone_from(&self.resume_roots);
        launch.env.extend(self.env.iter().cloned());
        launch
    }

    /// What the ACP agent takes from a client, each notice and exit given to `on_notice`.
    pub(crate) fn settings(&self, on_notice: coqui_acp_harness::acp::NoticeHook) -> Settings {
        let mut settings = Settings::new();
        settings.max_version = self.max_version;
        settings.max_sessions = self.max_sessions;
        settings.working_roots.clone_from(&self.working_roots);
        settings.client_mcp_servers = self.client_mcp_servers;
        settings.network_paths = self.network_paths;
        settings.on_notice = Some(on_notice);
        settings
    }
}

#[cfg(test)]
mod tests;
