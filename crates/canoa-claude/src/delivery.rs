//! How Claude Code is given what the host adds to a session: the skills of a profile and the hooks that report to the host, each a
//! local plugin that the session request's `_meta` loads.

use canoa::vendor::{
    DeliveredFile, HookKind, HookSupport, HookSupportRequest, HooksDelivery, HooksRequest,
    PLUGIN_PATH, PluginDelivery, SkillsDelivery,
};
use coqui_acp_api::guatiao::{Map, ToValue as _, Value};

/// The plugin a profile's skills are given in: Claude Code names a plugin's skills `<plugin>:<skill>`.
const SKILLS_PLUGIN: &str = "canoa-profile";

/// The plugin the host's hooks are given in.
const HOOKS_PLUGIN: &str = "canoa-hooks";

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
        _ => None,
    }
}

/// Hooks as a local plugin whose commands run the host's forwarder; a moment Claude Code has no hook for is left out.
pub(crate) fn hooks_delivery(request: &HooksRequest) -> HooksDelivery {
    let mut hooks = serde_json::Map::new();
    for each in &request.commands {
        let Some(event) = hook_event(each.kind) else {
            continue;
        };
        hooks.insert(
            event.to_owned(),
            serde_json::json!([{"hooks": [{
                "type": "command",
                "shell": "bash",
                "command": each.command,
                "timeout": request.timeout_seconds.saturating_add(5),
            }]}]),
        );
    }
    HooksDelivery::new(
        vec![
            DeliveredFile::new(
                ".claude-plugin/plugin.json",
                manifest(
                    HOOKS_PLUGIN,
                    "Reports compaction and working directory changes to the host that runs this session",
                ),
            ),
            DeliveredFile::new(
                "hooks/hooks.json",
                serde_json::json!({ "hooks": hooks }).to_string(),
            ),
        ],
        PluginDelivery::new("", plugin_meta()),
    )
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
