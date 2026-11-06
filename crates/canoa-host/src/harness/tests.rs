use std::ffi::OsStr;

use serde_json::{Value, json};

use super::*;

fn args_of(launch: &coqui_acp_harness::Launch) -> Vec<String> {
    launch
        .args
        .iter()
        .map(|arg| arg.to_string_lossy().into_owned())
        .collect()
}

fn notice() -> coqui_acp_harness::acp::NoticeHook {
    Arc::new(|_: &str| {})
}

#[test]
fn claude_code_starts_with_what_the_options_say_and_its_environment_only_in_the_environment() {
    let mut options = AgentOptions::new(Harness::Claude);
    options.executable = Some(OsString::from("/opt/claude"));
    options.model = Some("claude-haiku-5-5".to_owned());
    options.effort = Some("low".to_owned());
    options.permission_mode = Some("default".to_owned());
    options.setting_sources = Some(Vec::new());
    options.strict_mcp_config = true;
    options.env = vec![("CANOA_TEST_TOKEN".into(), "never-on-a-command-line".into())];
    let launch = options.claude_launch().launch();
    assert_eq!(launch.program, OsStr::new("/opt/claude"));
    let args = args_of(&launch);
    for expected in [
        "--model=claude-haiku-5-5",
        "--effort=low",
        "--permission-mode=default",
        "--setting-sources=",
        "--strict-mcp-config",
        "--permission-prompts=host",
        "--permission-prompt-tool=stdio",
    ] {
        assert!(
            args.iter().any(|arg| arg == expected),
            "{expected}: {args:?}"
        );
    }
    assert!(
        !args
            .iter()
            .any(|arg| arg.contains("never-on-a-command-line")),
        "{args:?}"
    );
    assert_eq!(
        launch.env.get(OsStr::new("CANOA_TEST_TOKEN")),
        Some(&OsString::from("never-on-a-command-line"))
    );
    assert!(!format!("{launch:?}").contains("never-on-a-command-line"));
}

#[test]
fn claude_codes_defaults_are_the_translators_own() {
    let options = AgentOptions::new(Harness::Claude);
    let launch = options.claude_launch().launch();
    assert_eq!(launch.program, OsStr::new("claude"));
    let args = args_of(&launch);
    assert!(
        args.iter()
            .any(|arg| arg == "--permission-prompt-tool=stdio"),
        "{args:?}"
    );
    for absent in [
        "--model=",
        "--effort=",
        "--setting-sources=",
        "--permission-mode=",
    ] {
        assert!(
            !args.iter().any(|arg| arg.starts_with(absent)),
            "{absent}: {args:?}"
        );
    }
    assert!(!options.client_plugins, "plugins from a client are off");
    let mut off = AgentOptions::new(Harness::Claude);
    off.permission_prompts = false;
    let args = args_of(&off.claude_launch().launch());
    assert!(
        !args
            .iter()
            .any(|arg| arg.starts_with("--permission-prompt")),
        "{args:?}"
    );
}

#[test]
fn codex_starts_its_app_server_with_the_options_and_holds_a_resumed_thread_to_the_roots() {
    let mut options = AgentOptions::new(Harness::Codex);
    options.executable = Some(OsString::from("/opt/codex"));
    options.model = Some("gpt-5.6-luna".to_owned());
    options.effort = Some("low".to_owned());
    options.approval_policy = Some("on-request".to_owned());
    options.sandbox = Some("workspace-write".to_owned());
    options.resume_roots = vec![PathBuf::from("/work")];
    options.env = vec![("CODEX_HOME".into(), "/codex-home".into())];
    let launch = options.codex_launch();
    assert_eq!(launch.model.as_deref(), Some("gpt-5.6-luna"));
    assert_eq!(launch.effort.as_deref(), Some("low"));
    assert_eq!(launch.approval_policy.as_deref(), Some("on-request"));
    assert_eq!(launch.sandbox.as_deref(), Some("workspace-write"));
    assert_eq!(launch.working_roots, [PathBuf::from("/work")]);
    let process = launch.launch();
    assert_eq!(process.program, OsStr::new("/opt/codex"));
    assert_eq!(
        args_of(&process).first().map(String::as_str),
        Some("app-server")
    );
    assert_eq!(
        process.env.get(OsStr::new("CODEX_HOME")),
        Some(&OsString::from("/codex-home"))
    );
}

#[test]
fn what_a_host_narrows_reaches_the_acp_agents_settings() {
    let mut options = AgentOptions::new(Harness::Codex);
    options.working_roots = vec![PathBuf::from("/work")];
    options.client_mcp_servers = false;
    options.network_paths = true;
    options.max_version = 1;
    options.max_sessions = 3;
    let settings = options.settings(notice());
    assert_eq!(settings.working_roots, [PathBuf::from("/work")]);
    assert!(!settings.client_mcp_servers);
    assert!(settings.network_paths);
    assert_eq!((settings.max_version, settings.max_sessions), (1, 3));
    assert!(settings.on_notice.is_some());
    let defaults = AgentOptions::new(Harness::Claude).settings(notice());
    assert!(defaults.working_roots.is_empty() && defaults.client_mcp_servers);
    assert!(!defaults.network_paths);
    assert_eq!(defaults.max_version, 2);
}

#[test]
fn a_harness_is_named_as_canoa_cfg_and_the_command_line_name_it() {
    for harness in [Harness::Claude, Harness::Codex] {
        assert_eq!(Harness::named(harness.as_str()), Some(harness));
    }
    assert_eq!(Harness::named("Claude"), None);
    assert_eq!(Harness::named("gemini"), None);
}

#[test]
fn its_debug_names_the_environment_and_never_shows_a_value() {
    let mut options = AgentOptions::new(Harness::Claude);
    options.env = vec![("TOKEN".into(), "never-shown".into())];
    let shown = format!("{options:?}");
    assert!(shown.contains("TOKEN"), "{shown}");
    assert!(!shown.contains("never-shown"), "{shown}");
}

#[test]
fn a_message_end_keeps_what_was_streamed_into_it() {
    for kind in ["agent_message", "user_message", "agent_thought"] {
        let end = json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s",
            "update": {"sessionUpdate": kind, "messageId": "m", "content": []}}});
        let kept: Value =
            serde_json::from_slice(&kept_content(end.to_string().into_bytes())).expect("JSON");
        assert_eq!(
            kept["params"]["update"],
            json!({"sessionUpdate": kind, "messageId": "m"}),
            "{kind}: a patch with no content leaves the message's content as it is"
        );
    }
    for untouched in [
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s",
            "update": {"sessionUpdate": "agent_message", "messageId": "m", "content": [{"type": "text", "text": "x"}]}}}),
        json!({"jsonrpc": "2.0", "method": "session/update", "params": {"sessionId": "s",
            "update": {"sessionUpdate": "tool_call_update", "toolCallId": "t", "content": []}}}),
        json!({"jsonrpc": "2.0", "id": 3, "result": {"content": []}}),
    ] {
        let line = untouched.to_string().into_bytes();
        assert_eq!(kept_content(line.clone()), line, "{untouched}");
    }
}

#[test]
fn what_canoa_agent_writes_keeps_message_ends_from_clearing_their_text() {
    let mut written = Vec::new();
    {
        let mut out = KeptContent::new(&mut written);
        let end = b"{\"method\":\"session/update\",\"params\":{\"update\":{\"sessionUpdate\":\"agent_message\",\"messageId\":\"m\",\"content\":[]}}}\n";
        out.write_all(&end[..20]).expect("written");
        out.write_all(&end[20..]).expect("written");
        out.write_all(b"{\"id\":1,\"result\":{}}\n")
            .expect("written");
        out.flush().expect("flushed");
    }
    let lines: Vec<Value> = written
        .split(|byte| *byte == b'\n')
        .filter(|line| !line.is_empty())
        .map(|line| serde_json::from_slice(line).expect("one JSON line"))
        .collect();
    assert_eq!(lines.len(), 2);
    assert!(
        lines[0]["params"]["update"].get("content").is_none(),
        "{}",
        lines[0]
    );
    assert_eq!(lines[1], json!({"id": 1, "result": {}}));
}
