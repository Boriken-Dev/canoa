use serde_json::json;

use super::*;

fn run(id: &str) -> AgentRun {
    AgentRun {
        id: id.to_owned(),
        executable: None,
        settings: Map::new(),
        access: Some(coqui_acp_harness::Access::STANDARD.to_json()),
        access_asked: false,
        roots: Vec::new(),
        lifetime: CacheLifetime::Default,
        max_message_bytes: crate::config::DEFAULT_MAX_MESSAGE_BYTES,
    }
}

#[test]
fn the_variable_naming_an_agents_program_is_made_from_its_id() {
    assert_eq!(executable_variable("claude"), "COQUI_ACP_CLAUDE_EXECUTABLE");
    assert_eq!(executable_variable("codex"), "COQUI_ACP_CODEX_EXECUTABLE");
    assert_eq!(executable_variable("a-b"), "COQUI_ACP_A_B_EXECUTABLE");
}

#[test]
fn an_agent_is_given_the_standard_access_and_every_mode_where_its_provider_declares_them() {
    let registry = crate::kind::registry(None).expect("bundled");
    for id in ["claude", "codex"] {
        let mut asked = run(id);
        asked.executable = Some(OsString::from("/opt/agent"));
        let settings = asked.provider_settings(&registry).expect("settings");
        let map = settings.as_map();
        assert_eq!(map["executable"], "/opt/agent", "{id}");
        assert_eq!(
            map["access"],
            coqui_acp_harness::Access::STANDARD.to_json(),
            "{id}"
        );
        assert_eq!(map["allow_bypass"], true, "{id}");
        assert!(
            map["modes"]
                .as_array()
                .is_some_and(|modes| !modes.is_empty())
        );
    }
}

#[test]
fn what_the_command_line_sets_is_not_replaced() {
    let registry = crate::kind::registry(None).expect("bundled");
    let mut asked = run("claude");
    asked.settings.insert("modes".to_owned(), json!(["plan"]));
    asked
        .settings
        .insert("allow_bypass".to_owned(), json!(false));
    asked
        .settings
        .insert("setting_sources".to_owned(), json!([]));
    let settings = asked.provider_settings(&registry).expect("settings");
    assert_eq!(settings.as_map()["modes"], json!(["plan"]));
    assert_eq!(settings.as_map()["allow_bypass"], false);
    assert_eq!(settings.as_map()["setting_sources"], json!([]));
}

#[test]
fn an_agent_that_takes_no_access_is_refused_one_only_when_the_command_line_asked_for_it() {
    let registry = crate::kind::registry(None).expect("bundled");
    // An id no bundled provider declares anything for takes none of the keys, so the standard access is not passed...
    let quiet = run("nobody");
    let settings = quiet.provider_settings(&registry).expect("settings");
    assert!(!settings.as_map().contains_key("access"));
    // ...and a command line that asked for an access is told so.
    let mut asked = run("nobody");
    asked.access_asked = true;
    assert!(
        asked
            .provider_settings(&registry)
            .expect_err("no access")
            .contains("takes no access")
    );
}

#[test]
fn an_agent_the_host_does_not_carry_is_a_usage_error_naming_it() {
    let refused = serve_stdio(&run("nobody")).expect_err("nothing offers it");
    assert!(refused.usage, "{refused:?}");
    assert!(refused.why.contains("nobody"), "{}", refused.why);
}

#[test]
fn a_setting_the_provider_does_not_declare_is_a_usage_error() {
    let mut asked = run("claude");
    asked
        .settings
        .insert("no_such_setting".to_owned(), json!(true));
    asked.executable = Some(OsString::from("none"));
    let refused = serve_stdio(&asked).expect_err("the schema refuses it");
    assert!(refused.usage, "{refused:?}");
}
