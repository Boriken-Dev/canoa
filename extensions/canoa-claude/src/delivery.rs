//! How Claude Code is given what the host adds to a session: the skills of a profile and the hooks that report to the host, each a
//! local plugin that the session request's `_meta` loads.

use canoa::vendor::{
    DeliveredFile, HookKind, HookSupport, HookSupportRequest, HookTool, HooksDelivery,
    HooksRequest, ModeWord, PLUGIN_PATH, PluginDelivery, ShellFamily, SkillsDelivery, ToolHook,
};
use coqui_acp_api::guatiao::{Map, ToValue as _, Value};

/// The plugin a profile's skills are given in: Claude Code names a plugin's skills `<plugin>:<skill>`.
const SKILLS_PLUGIN: &str = "canoa-profile";

/// The plugin the host's hooks are given in.
const HOOKS_PLUGIN: &str = "canoa-hooks";

/// The plugin of the hook that runs a session's own tools in an environment: a name of its own, so a session can load it beside
/// the other.
const TOOLS_PLUGIN: &str = "canoa-environment";

/// Claude Code's tools that run a command, with the shell each command is written for.
const SHELL_TOOLS: [(&str, ShellFamily); 2] = [
    ("Bash", ShellFamily::Bash),
    ("PowerShell", ShellFamily::Powershell),
];

/// Claude Code's tools that act on nothing of the computer it runs on: tasks and subagents (whose own tools pass through the same
/// hook), questions to the person, plan mode, the web, a background command's output (which the forwarder prints), and the
/// resources of the session's MCP servers. Every tool not named here or below is refused.
const PASS_TOOLS: [&str; 18] = [
    "Task",
    "Agent",
    "TodoWrite",
    "TaskCreate",
    "TaskGet",
    "TaskList",
    "TaskUpdate",
    "TaskOutput",
    "TaskStop",
    "BashOutput",
    "KillShell",
    "KillBash",
    "AskUserQuestion",
    "EnterPlanMode",
    "ExitPlanMode",
    "WebFetch",
    "WebSearch",
    "ListMcpResourcesTool",
];

/// The beginning of the name of every tool of an MCP server the session was given.
const MCP_PREFIX: &str = "mcp__";

/// Claude Code's tools that read, write, edit, search or list files where it runs.
const FILE_TOOLS: [&str; 8] = [
    "Read",
    "Write",
    "Edit",
    "MultiEdit",
    "NotebookEdit",
    "Grep",
    "Glob",
    "LS",
];

/// What a session request gains to load the local plugin at the folder the host wrote.
fn plugin_meta() -> Value {
    let mut entry = Map::new();
    entry.set("type", Value::from("local"));
    entry.set("path", Value::from(PLUGIN_PATH));
    let mut options = Map::new();
    options.set(
        "plugins",
        vec![Value::from(entry)].to_value().expect("value"),
    );
    let mut claude = Map::new();
    claude.set("options", Value::from(options));
    let mut root = Map::new();
    root.set("claudeCode", Value::from(claude));
    Value::from(root)
}

fn manifest(name: &str, description: &str) -> String {
    serde_json::json!({"name": name, "version": "0.0.0", "description": description}).to_string()
}

/// Skills as a local plugin, which the request's `_meta` loads.
pub(crate) fn skills_delivery() -> SkillsDelivery {
    let mut delivery = SkillsDelivery::new(
        format!("{SKILLS_PLUGIN}/skills"),
        format!(
            "the Claude plugin {SKILLS_PLUGIN}, through the adapter's _meta, as {SKILLS_PLUGIN}:{{name}}"
        ),
    );
    delivery.files = vec![DeliveredFile::new(
        format!("{SKILLS_PLUGIN}/.claude-plugin/plugin.json"),
        manifest(
            SKILLS_PLUGIN,
            "The skills of the host's profile for this session",
        ),
    )];
    delivery.plugin = Some(PluginDelivery::new(SKILLS_PLUGIN, plugin_meta()));
    delivery
}

/// Claude Code's name for the moment a hook of `kind` reports.
fn hook_event(kind: HookKind) -> Option<&'static str> {
    match kind {
        HookKind::Start => Some("SessionStart"),
        HookKind::PreCompact => Some("PreCompact"),
        HookKind::Batch => Some("PostToolBatch"),
        HookKind::PreTool => Some("PreToolUse"),
        _ => None,
    }
}

/// Hooks as a local plugin whose commands run the host's forwarder; a moment Claude Code has no hook for is left out.
///
/// A `PreToolUse` command runs before every tool (matcher `*`, "Match all" in Claude Code's hooks reference), and the host refuses
/// what [`tool_hook`] does not let pass or take over. Claude Code lets a call go on when its hook ends with any status but 0 or 2,
/// so the command is followed by `|| exit 2`: a forwarder that cannot start, or ends in error, refuses the call. Its timeout also
/// lets the call go on, so the forwarder answers within the request's `timeout_seconds`.
pub(crate) fn hooks_delivery(request: &HooksRequest) -> HooksDelivery {
    let mut hooks = serde_json::Map::new();
    let mut tools = false;
    for each in &request.commands {
        let Some(event) = hook_event(each.kind) else {
            continue;
        };
        let mut hook = serde_json::json!({
            "type": "command",
            "shell": "bash",
            "command": each.command,
            "timeout": request.timeout_seconds.saturating_add(5),
        });
        let mut entry = serde_json::Map::new();
        if each.kind == HookKind::PreTool {
            tools = true;
            hook["command"] = serde_json::json!(format!("{} || exit 2", each.command));
            // Claude Code's hooks reference: "To block the action instead, set \"onFailure\": \"block\" on a command ... hook"
            // (v2.1.295 and later), which makes a hook that cannot start, times out or exits with another status refuse the call.
            hook["onFailure"] = serde_json::json!("block");
            entry.insert("matcher".to_owned(), serde_json::json!("*"));
        }
        entry.insert("hooks".to_owned(), serde_json::json!([hook]));
        hooks.insert(event.to_owned(), serde_json::json!([entry]));
    }
    let (name, description) = if tools {
        (
            TOOLS_PLUGIN,
            "Runs this session's commands in the environment its tools are served from, and turns its file tools to that environment",
        )
    } else {
        (
            HOOKS_PLUGIN,
            "Reports compaction and working directory changes to the host that runs this session",
        )
    };
    HooksDelivery::new(
        vec![
            DeliveredFile::new(".claude-plugin/plugin.json", manifest(name, description)),
            DeliveredFile::new(
                "hooks/hooks.json",
                serde_json::json!({ "hooks": hooks }).to_string(),
            ),
        ],
        PluginDelivery::new("", plugin_meta()),
    )
}

/// How Claude Code's `PreToolUse` payload is read and answered. The rewritten input carries no `permissionDecision`, so Claude
/// Code's own rules and mode still decide whether the call runs, judging the command as rewritten.
pub(crate) fn tool_hook() -> ToolHook {
    let rewrite = serde_json::json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "updatedInput": {},
    }});
    let deny = serde_json::json!({"hookSpecificOutput": {
        "hookEventName": "PreToolUse",
        "permissionDecision": "deny",
        "permissionDecisionReason": "",
    }});
    let mut hook = ToolHook::new(
        SHELL_TOOLS
            .iter()
            .map(|(name, shell)| HookTool::shell(*name, "command", *shell))
            .chain(FILE_TOOLS.iter().map(|name| HookTool::files(*name)))
            .chain(PASS_TOOLS.iter().map(|name| HookTool::passes(*name)))
            .collect(),
        ("/tool_name", "/tool_input"),
        (
            deny.to_string(),
            "/hookSpecificOutput/permissionDecisionReason",
        ),
        (rewrite.to_string(), "/hookSpecificOutput/updatedInput"),
    );
    hook.pass_prefixes = vec![MCP_PREFIX.to_owned()];
    hook.mode_at = Some("/permission_mode".to_owned());
    hook.modes = [
        ("default", "manual"),
        ("acceptEdits", "accept_edits"),
        ("auto", "auto"),
        ("plan", "plan"),
        ("bypassPermissions", "bypass"),
        ("dontAsk", "dont_ask"),
    ]
    .into_iter()
    .map(|(word, mode)| ModeWord::new(word, mode))
    .collect();
    hook
}

/// What Claude Code's hooks give for the events ACP does not show.
pub(crate) fn hook_support(request: &HookSupportRequest) -> Option<HookSupport> {
    match (request.event.as_str(), request.reason.as_deref()) {
        ("cd", _) => Some(HookSupport::fires(
            "run by Canoa when Claude Code reports a new working directory after a batch of tool calls (a plugin's hook: the event of its own is not given to plugins)",
        )),
        ("pre-compact", _) => Some(HookSupport::fires(
            "run by Canoa before Claude Code compacts, for the person's /compact (a plugin's PreCompact hook): block stops it, context is not applied",
        )),
        ("session-start", Some("compact")) => Some(HookSupport::fires(
            "run by Canoa when Claude Code starts a session again after compacting (a plugin's SessionStart hook): context goes in the next prompt",
        )),
        ("session-start", Some("clear")) => Some(HookSupport::cannot(
            "over ACP the person's /clear ends Claude Code's session and the adapter's, so no session starts after it",
        )),
        _ => None,
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use canoa::vendor::{HookCommand, HookKind};
    use coqui_acp_api::guatiao::FromValue as _;

    #[test]
    fn the_hooks_plugin_runs_each_command_for_the_moment_it_reports() {
        assert!(HookKind::from_value(&Value::from("nothing")).is_err());
        let request = HooksRequest::new(
            vec![
                HookCommand::new(HookKind::PreCompact, "report pre-compact"),
                HookCommand::new(HookKind::Batch, "report batch"),
            ],
            130,
        );
        let delivery = hooks_delivery(&request);
        let hooks = delivery
            .files
            .iter()
            .find(|file| file.path == "hooks/hooks.json")
            .expect("the hooks file");
        let hooks: serde_json::Value = serde_json::from_str(&hooks.text).expect("JSON");
        assert_eq!(
            hooks["hooks"]["PreCompact"][0]["hooks"][0]["command"],
            "report pre-compact"
        );
        assert_eq!(
            hooks["hooks"]["PostToolBatch"][0]["hooks"][0]["timeout"],
            135
        );
        assert_eq!(hooks["hooks"].as_object().expect("an object").len(), 2);
        assert_eq!(
            hooks["hooks"]["PreCompact"][0]["hooks"][0]["shell"], "bash",
            "the commands are written for bash"
        );
        assert_eq!(delivery.plugin.folder, "");
    }

    fn hooks_file(delivery: &HooksDelivery) -> serde_json::Value {
        let file = delivery
            .files
            .iter()
            .find(|file| file.path == "hooks/hooks.json")
            .expect("the hooks file");
        serde_json::from_str(&file.text).expect("JSON")
    }

    #[test]
    fn the_pre_tool_hook_runs_before_every_shell_and_file_tool_and_refuses_when_it_fails() {
        let delivery = hooks_delivery(&HooksRequest::new(
            vec![HookCommand::new(
                HookKind::PreTool,
                "\"canoa\" hook-forward",
            )],
            40,
        ));
        let hooks = hooks_file(&delivery);
        let entry = &hooks["hooks"]["PreToolUse"][0];
        assert_eq!(
            entry["matcher"], "*",
            "every tool, so none acts on this computer unseen"
        );
        let hook = tool_hook();
        let names: Vec<&str> = hook.tools.iter().map(|tool| tool.name.as_str()).collect();
        for name in [
            "Bash",
            "PowerShell",
            "Read",
            "Write",
            "Edit",
            "MultiEdit",
            "NotebookEdit",
            "Grep",
            "Glob",
            "LS",
        ] {
            assert!(names.contains(&name), "{name}");
        }
        assert_eq!(
            entry["hooks"][0]["command"],
            "\"canoa\" hook-forward || exit 2"
        );
        assert_eq!(entry["hooks"][0]["timeout"], 45);
        assert_eq!(
            entry["hooks"][0]["onFailure"], "block",
            "a failed hook refuses the call"
        );
        let manifest = delivery
            .files
            .iter()
            .find(|file| file.path == ".claude-plugin/plugin.json")
            .expect("a manifest");
        assert!(
            manifest.text.contains("canoa-environment"),
            "{}",
            manifest.text
        );
        // A tool that acts on this computer and is neither a shell nor a file tool is not let through.
        for refused in ["LSP", "EnterWorktree", "ExitWorktree", "Monitor", "Skill"] {
            assert!(!names.contains(&refused), "{refused}");
        }
        assert_eq!(hook.pass_prefixes, ["mcp__"]);
    }

    #[test]
    fn the_tool_description_decides_nothing_when_it_rewrites_and_tells_dont_ask_from_bypass() {
        let hook = tool_hook();
        let rewrite: serde_json::Value = serde_json::from_str(&hook.rewrite).expect("JSON");
        assert!(rewrite.pointer(&hook.rewrite_input_at).is_some());
        assert!(
            rewrite
                .pointer("/hookSpecificOutput/permissionDecision")
                .is_none(),
            "a rewrite leaves the decision to Claude Code"
        );
        let deny: serde_json::Value = serde_json::from_str(&hook.deny).expect("JSON");
        assert_eq!(deny["hookSpecificOutput"]["permissionDecision"], "deny");
        assert!(deny.pointer(&hook.deny_reason_at).is_some());
        let mode = |word: &str| {
            hook.modes
                .iter()
                .find(|each| each.word == word)
                .map(|each| each.mode.clone())
        };
        assert_eq!(mode("dontAsk").as_deref(), Some("dont_ask"));
        assert_eq!(mode("bypassPermissions").as_deref(), Some("bypass"));
        assert_eq!(mode("default").as_deref(), Some("manual"));
    }

    #[test]
    fn the_skills_are_a_plugin_named_for_them() {
        let delivery = skills_delivery();
        assert_eq!(delivery.folder, "canoa-profile/skills");
        let plugin = delivery.plugin.expect("a plugin");
        assert_eq!(plugin.folder, "canoa-profile");
        assert_eq!(plugin.meta, plugin_meta());
        assert!(delivery.how.contains("canoa-profile:{name}"));
        assert!(!delivery.directory);
    }

    #[test]
    fn a_clear_is_one_no_hook_reaches() {
        let request = HookSupportRequest::new("session-start", Some("clear".to_owned()));
        assert!(!hook_support(&request).expect("an answer").fires);
        assert_eq!(
            hook_support(&HookSupportRequest::new("session-start", None)),
            None
        );
    }
}
