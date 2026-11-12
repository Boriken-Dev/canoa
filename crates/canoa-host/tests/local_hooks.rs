//! The scripted agent as a source of the host's own, with an overlay whose hook logs what it is told: which event fires
//! when, what the hook may add or ask, and what a hook that fails, goes too long or tries to grant does.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod hooked;
mod linked;
mod support;

use canoa_host::profile::PROFILE_META;
use hooked::{Allow, Arm, has, hooks, item, said, start};
use serde_json::{Value, json};

const START: &str = r#"{"context":"CTX-START"}"#;
const PROMPT: &str = r#"{"context":"CTX-PROMPT"}"#;

fn print(text: &str) -> Arm {
    Arm::Print(text.to_owned())
}

fn session_id(opened: &Value) -> String {
    opened["sessionId"]
        .as_str()
        .expect("a session id")
        .to_owned()
}

/// The user's turns the agent recorded in a session: the text of each.
fn heard(setup: &hooked::Setup, session: &str) -> Vec<String> {
    setup.agent_state(session)["history"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|said| said["role"] == "user")
        .filter_map(|said| said["text"].as_str().map(str::to_owned))
        .collect()
}

fn keys(payload: &Value) -> Vec<&str> {
    let mut keys: Vec<&str> = payload
        .as_object()
        .expect("an object")
        .keys()
        .map(String::as_str)
        .collect();
    keys.sort_unstable();
    keys
}

#[test]
fn hooks_are_off_until_allowed_and_the_report_says_so() {
    let setup = start(
        "",
        &[("session-start", print(START))],
        &Allow::Nothing,
        &["--system-prompt"],
    );
    let report = setup.initialize(false);
    let report = &report["_meta"][PROFILE_META];
    let off = item(report, "hooks.team");
    assert_eq!(off["state"], "off", "{report}");
    assert!(
        off["how"]
            .as_str()
            .is_some_and(|how| how.contains("hook-not-allowed")),
        "{report}"
    );
    let opened = setup.new_session(2).expect("a session opens");
    let session = session_id(&opened);
    assert!(
        setup.agent_state(&session)["system"].is_null(),
        "no hook ran"
    );
    assert!(hooked::logged(&setup.log).is_empty());
    setup.host.stop();
}

#[test]
fn hooks_allowed_at_another_digest_are_off_and_the_report_says_the_files_changed() {
    let setup = start(
        "",
        &[("session-start", print(START))],
        &Allow::Other,
        &["--system-prompt"],
    );
    let report = setup.initialize(false);
    let report = &report["_meta"][PROFILE_META];
    let off = item(report, "hooks.team");
    assert_eq!(off["state"], "off", "{report}");
    assert!(
        off["how"]
            .as_str()
            .is_some_and(|how| how.contains("another version")),
        "{report}"
    );
    let opened = setup.new_session(2).expect("a session opens");
    assert!(setup.agent_state(&session_id(&opened))["system"].is_null());
    assert!(hooked::logged(&setup.log).is_empty());
    setup.host.stop();
}

#[test]
fn session_start_for_a_new_session_adds_context_to_the_system_prompt_and_is_told_only_what_the_spec_says()
 {
    let setup = start(
        "",
        &[("session-start", print(START))],
        &Allow::Digest,
        &["--system-prompt"],
    );
    setup.initialize(false);
    let opened = setup.new_session(2).expect("a session opens");
    let session = session_id(&opened);
    let system = setup.agent_state(&session)["system"].to_string();
    assert!(system.contains("CTX-START"), "{system}");
    assert!(
        system.contains("session-start hook of the overlay team"),
        "the context is marked with its source, in a block of its own: {system}"
    );
    let events = setup.events("session-start", 1);
    let payload = &events[0];
    assert_eq!(
        keys(payload),
        ["cwd", "event", "harness", "reason", "session"],
        "{payload}"
    );
    assert_eq!(payload["reason"], "new");
    assert_eq!(payload["harness"], "claude-code");
    assert_eq!(payload["cwd"], json!(setup.cwd));
    let ours = payload["session"].as_str().expect("an id");
    assert!(
        ours != session && !ours.contains(&session),
        "never the agent's own id: {payload}"
    );
    let report = &opened["_meta"][PROFILE_META];
    assert_eq!(
        item(report, "hooks.team.session-start")["state"],
        "applied",
        "{report}"
    );
    setup.host.stop();
}

#[test]
fn where_the_agent_takes_no_system_prompt_the_context_goes_with_the_first_prompt() {
    let setup = start("", &[("session-start", print(START))], &Allow::Digest, &[]);
    setup.initialize(false);
    let opened = setup.new_session(2).expect("a session opens");
    let session = session_id(&opened);
    assert!(setup.agent_state(&session)["system"].is_null());
    let (turn, _) = setup.turn(3, &session, "hello there", "yes");
    turn.expect("the turn ends");
    let heard = heard(&setup, &session);
    assert!(
        heard[0].starts_with("hello there"),
        "the user's words come first: {heard:?}"
    );
    assert!(heard[0].contains("CTX-START"), "{heard:?}");
    // Once: the second prompt carries nothing more.
    let (turn, _) = setup.turn(4, &session, "again", "yes");
    turn.expect("the turn ends");
    assert!(!heard_by(&setup, &session)[1].contains("CTX-START"));
    setup.host.stop();
}

#[test]
fn a_session_that_is_loaded_again_starts_with_the_reason_resume() {
    let setup = start("", &[], &Allow::Digest, &[]);
    setup.initialize(false);
    let opened = setup.new_session(2).expect("a session opens");
    let session = session_id(&opened);
    let (loaded, ..) = setup.request(
        3,
        "session/load",
        json!({"sessionId": session, "cwd": setup.cwd, "mcpServers": []}),
        None,
    );
    loaded.expect("the session loads");
    let events = setup.events("session-start", 2);
    let reasons: Vec<&str> = events
        .iter()
        .filter_map(|payload| payload["reason"].as_str())
        .collect();
    assert_eq!(reasons, ["new", "resume"], "{events:?}");
    assert_eq!(
        events[0]["session"], events[1]["session"],
        "a session loaded again is the same session to its hooks"
    );
    let (turn, _) = setup.turn(4, &session, "hello", "yes");
    turn.expect("the turn ends");
    assert_eq!(
        setup.events("turn-end", 1)[0]["session"],
        events[0]["session"]
    );
    setup.host.stop();
}

#[test]
fn a_session_start_hook_that_fails_closed_refuses_the_session() {
    let setup = start(
        &hooks("session-start = { on_failure = \"refuse\" }"),
        &[("session-start", Arm::Exit(3))],
        &Allow::Digest,
        &[],
    );
    setup.initialize(false);
    let refused = setup.new_session(2).expect_err("the session is refused");
    assert_eq!(refused["code"], -32000, "{refused}");
    assert!(
        !refused.to_string().contains("exit"),
        "the cause is for the log, not for a client: {refused}"
    );
    setup.host.stop();
}

#[test]
fn an_overlay_with_a_guard_that_is_not_allowed_is_not_activated() {
    let setup = start(
        &hooks("session-start = { on_failure = \"refuse\" }"),
        &[("session-start", Arm::Exit(0))],
        &Allow::Nothing,
        &[],
    );
    let report = setup.initialize(false);
    let report = &report["_meta"][PROFILE_META];
    let left = item(report, "overlay");
    assert_eq!(left["state"], "refused", "{report}");
    assert!(
        left["how"]
            .as_str()
            .is_some_and(|how| how.contains("fails closed")),
        "{report}"
    );
    assert!(!has(report, "hooks.team"), "{report}");
    setup.new_session(2).expect("a session opens");
    assert!(hooked::logged(&setup.log).is_empty());
    setup.host.stop();
}

#[test]
fn a_prompt_hook_adds_context_after_the_users_words() {
    let setup = start(
        &hooks("prompt = {}"),
        &[("prompt", print(PROMPT))],
        &Allow::Digest,
        &[],
    );
    setup.initialize(false);
    let session = session_id(&setup.new_session(2).expect("a session opens"));
    let (turn, _) = setup.turn(3, &session, "hello there", "yes");
    turn.expect("the turn ends");
    let heard = heard(&setup, &session);
    assert!(heard[0].starts_with("hello there"), "{heard:?}");
    assert!(heard[0].contains("CTX-PROMPT"), "{heard:?}");
    let events = setup.events("prompt", 1);
    let payload = &events[0];
    assert_eq!(
        keys(payload),
        ["cwd", "event", "harness", "prompt", "session"],
        "{payload}"
    );
    assert_eq!(payload["prompt"], "hello there");
    setup.host.stop();
}

#[test]
fn a_prompt_hook_can_block_and_the_agent_never_hears_the_prompt() {
    let setup = start(
        &hooks("prompt = {}"),
        &[(
            "prompt",
            print(r#"{"decision":"block","reason":"not now"}"#),
        )],
        &Allow::Digest,
        &[],
    );
    setup.initialize(false);
    let session = session_id(&setup.new_session(2).expect("a session opens"));
    let (turn, seen) = setup.turn(3, &session, "hello there", "yes");
    assert_eq!(turn.expect("answered")["stopReason"], "refusal");
    let told = said(&seen);
    assert!(told.contains("not now") && told.contains("team"), "{told}");
    assert!(heard(&setup, &session).is_empty());
    setup.host.stop();
}

#[test]
fn a_prompt_hook_that_goes_wrong_blocks_when_it_fails_closed_and_otherwise_the_prompt_goes_on() {
    // Every cause: an exit status, too long, too much output, an attempt to grant, and output that is not an object.
    let causes: Vec<(&str, Arm, &str)> = vec![
        ("exit", Arm::Exit(3), "prompt = { on_failure = \"refuse\" }"),
        (
            "timeout",
            Arm::Sleep(30),
            "prompt = { on_failure = \"refuse\", timeout = 1 }",
        ),
        (
            "too large",
            Arm::Big(70_000),
            "prompt = { on_failure = \"refuse\" }",
        ),
        (
            "grant",
            print(r#"{"decision":"allow"}"#),
            "prompt = { on_failure = \"refuse\" }",
        ),
        (
            "not an object",
            print("[1]"),
            "prompt = { on_failure = \"refuse\" }",
        ),
    ];
    for (name, arm, table) in causes {
        let setup = start(&hooks(table), &[("prompt", arm)], &Allow::Digest, &[]);
        setup.initialize(false);
        let session = session_id(&setup.new_session(2).expect("a session opens"));
        let (turn, _) = setup.turn(3, &session, "hello there", "yes");
        assert_eq!(turn.expect("answered")["stopReason"], "refusal", "{name}");
        assert!(
            heard(&setup, &session).is_empty(),
            "{name}: the agent heard it"
        );
        setup.host.stop();
    }
    // Reporting is the default: the prompt goes on, and nothing the failed run said applies.
    let setup = start(
        &hooks("prompt = {}"),
        &[(
            "prompt",
            print(r#"{"context":"CTX-PROMPT","decision":"allow"}"#),
        )],
        &Allow::Digest,
        &[],
    );
    setup.initialize(false);
    let session = session_id(&setup.new_session(2).expect("a session opens"));
    let (turn, _) = setup.turn(3, &session, "hello there", "yes");
    assert_ne!(turn.expect("answered")["stopReason"], "refusal");
    let heard = heard(&setup, &session);
    assert!(
        heard[0].starts_with("hello there") && !heard[0].contains("CTX-PROMPT"),
        "{heard:?}"
    );
    setup.host.stop();
}

#[test]
fn turn_end_fires_when_the_agent_answers_the_prompt() {
    let setup = start("", &[], &Allow::Digest, &[]);
    setup.initialize(false);
    let session = session_id(&setup.new_session(2).expect("a session opens"));
    let (turn, _) = setup.turn(3, &session, "hello there", "yes");
    turn.expect("the turn ends");
    let events = setup.events("turn-end", 1);
    let payload = &events[0];
    assert_eq!(
        keys(payload),
        ["cwd", "event", "harness", "session"],
        "{payload}"
    );
    setup.host.stop();
}

#[test]
fn pre_tool_denies_a_call_the_agent_asks_about_and_the_client_is_never_asked() {
    let setup = start(
        &hooks("pre-tool = { match = [\"edit\"], on_failure = \"refuse\" }"),
        &[(
            "pre-tool",
            print(r#"{"decision":"deny","reason":"no edits"}"#),
        )],
        &Allow::Digest,
        &[],
    );
    setup.initialize(false);
    let session = session_id(&setup.new_session(2).expect("a session opens"));
    let (turn, seen) = setup.turn(3, &session, "Rename it", "yes");
    turn.expect("the turn ends");
    assert!(
        !seen
            .iter()
            .any(|message| message["method"] == "session/request_permission"),
        "the client was asked: {seen:?}"
    );
    assert!(
        seen.iter()
            .any(
                |message| message["params"]["update"]["sessionUpdate"] == "tool_call_update"
                    && message["params"]["update"]["status"] == "failed"
            ),
        "the call did not run: {seen:?}"
    );
    let events = setup.events("pre-tool", 1);
    let payload = &events[0];
    assert_eq!(
        keys(payload),
        ["cwd", "event", "harness", "session", "tool"],
        "{payload}"
    );
    assert_eq!(payload["tool"]["kind"], "edit");
    assert_eq!(payload["tool"]["name"], "Edit src/calc.js");
    assert_eq!(
        payload["tool"]["input"],
        json!({"file_path": "src/calc.js"})
    );
    setup.host.stop();
}

#[test]
fn a_pre_tool_hook_that_decides_nothing_or_asks_leaves_the_question_to_the_person() {
    for output in [
        "",
        r#"{"decision":"ask","reason":"look"}"#,
        r#"{"context":"x"}"#,
    ] {
        let arms = if output.is_empty() {
            vec![]
        } else {
            vec![("pre-tool", print(output))]
        };
        let setup = start(
            &hooks("pre-tool = { match = [\"edit\"] }"),
            &arms,
            &Allow::Digest,
            &[],
        );
        setup.initialize(false);
        let session = session_id(&setup.new_session(2).expect("a session opens"));
        let (turn, seen) = setup.turn(3, &session, "Rename it", "yes");
        turn.expect("the turn ends");
        assert!(
            seen.iter()
                .any(|message| message["method"] == "session/request_permission"),
            "{output}: the client is asked as it would be without the hook"
        );
        assert!(
            seen.iter()
                .any(|message| message["params"]["update"]["status"] == "completed"),
            "{output}: the person's yes ran it"
        );
        setup.host.stop();
    }
}

#[test]
fn a_pre_tool_hook_cannot_approve_a_call() {
    // `allow` is an attempt to grant: reported by default, a denial for a guard.
    for (table, denied) in [
        ("pre-tool = {}", false),
        ("pre-tool = { on_failure = \"refuse\" }", true),
    ] {
        let setup = start(
            &hooks(table),
            &[("pre-tool", print(r#"{"decision":"allow"}"#))],
            &Allow::Digest,
            &[],
        );
        setup.initialize(false);
        let session = session_id(&setup.new_session(2).expect("a session opens"));
        // The person says no: nothing a hook output made it run.
        let (turn, seen) = setup.turn(3, &session, "Rename it", "no");
        turn.expect("the turn ends");
        let asked = seen
            .iter()
            .any(|message| message["method"] == "session/request_permission");
        assert_eq!(asked, !denied, "{table}: {seen:?}");
        assert!(
            !seen
                .iter()
                .any(|message| message["params"]["update"]["status"] == "completed"),
            "{table}: it ran"
        );
        setup.host.stop();
    }
}

#[test]
fn post_tool_fires_when_a_call_ends_with_its_status() {
    let setup = start(&hooks("post-tool = {}"), &[], &Allow::Digest, &[]);
    setup.initialize(false);
    let session = session_id(&setup.new_session(2).expect("a session opens"));
    let (turn, _) = setup.turn(3, &session, "Rename it", "yes");
    turn.expect("the turn ends");
    let events = setup.events("post-tool", 1);
    let payload = &events[0];
    assert_eq!(
        keys(payload),
        ["cwd", "event", "harness", "session", "status", "tool"],
        "{payload}"
    );
    assert_eq!(payload["status"], "completed");
    assert_eq!(payload["tool"]["kind"], "edit");
    setup.host.stop();
}

#[test]
fn post_tool_context_goes_with_the_next_prompt() {
    let setup = start(
        &hooks("post-tool = {}"),
        &[("post-tool", print(r#"{"context":"CTX-AFTER"}"#))],
        &Allow::Digest,
        &[],
    );
    setup.initialize(false);
    let session = session_id(&setup.new_session(2).expect("a session opens"));
    let (turn, _) = setup.turn(3, &session, "Rename it", "yes");
    turn.expect("the turn ends");
    setup.events("post-tool", 1);
    let (turn, _) = setup.turn(4, &session, "and then", "yes");
    turn.expect("the turn ends");
    let heard = heard(&setup, &session);
    assert!(
        !heard[0].contains("CTX-AFTER") && heard[1].contains("CTX-AFTER"),
        "{heard:?}"
    );
    setup.host.stop();
}

#[test]
fn session_end_fires_when_a_session_is_closed_and_for_those_left_when_the_connection_ends() {
    let setup = start("", &[], &Allow::Digest, &[]);
    setup.initialize(false);
    let first = session_id(&setup.new_session(2).expect("a session opens"));
    let (closed, ..) = setup.request(3, "session/close", json!({"sessionId": first}), None);
    closed.expect("the session closes");
    let ends = setup.events("session-end", 1);
    assert_eq!(ends.len(), 1, "{ends:?}");
    assert_eq!(keys(&ends[0]), ["cwd", "event", "harness", "session"]);
    setup.new_session(4).expect("a second session opens");
    setup.host.stop();
    let ends = setup.events("session-end", 2);
    assert_eq!(
        ends.len(),
        2,
        "the second ended with the connection: {ends:?}"
    );
}

#[test]
fn what_canoa_cannot_see_is_reported_and_never_skipped_silently() {
    let setup = start(
        &hooks(
            "session-start = { match = [\"clear\"] }\npre-tool = { match = [\"execute\"] }\npre-compact = {}",
        ),
        &[],
        &Allow::Digest,
        &[],
    );
    let report = setup.initialize(false);
    let report = &report["_meta"][PROFILE_META];
    // The default hook is bound to `cd`, which ACP never shows, and a hook for `pre-compact` has nothing to fire it.
    for key in [
        "hooks.team.cd",
        "hooks.team.pre-compact",
        "hooks.team.session-start",
    ] {
        let each = item(report, key);
        assert_eq!(each["state"], "not supported", "{key}: {report}");
        assert!(
            each["how"]
                .as_str()
                .is_some_and(|how| how.contains("hook-unsupported")),
            "{key}"
        );
    }
    let partial = item(report, "hooks.team.pre-tool");
    assert_eq!(partial["state"], "applied");
    assert!(
        partial["how"]
            .as_str()
            .is_some_and(|how| how.contains("hook-partial")),
        "{partial}"
    );
    setup.host.stop();
}

#[test]
fn the_host_lists_the_hooks_it_asks_about_and_runs_only_those_the_person_allows() {
    let table = hooks("session-start = {}\nprompt = { on_failure = \"report\", timeout = 5 }");
    for (answer, runs) in [
        (Some(json!({"allow": [], "allowHooks": ["team"]})), true),
        (Some(json!({"allow": [], "allowHooks": ["other"]})), false),
        (Some(json!({"allow": []})), false),
        (None, false),
    ] {
        let setup = start(
            &table,
            &[("session-start", print(START))],
            &Allow::Nothing,
            &["--system-prompt"],
        );
        setup.initialize(true);
        let (opened, asked, _) = setup.request(
            2,
            "session/new",
            json!({"cwd": setup.cwd, "mcpServers": []}),
            answer.clone(),
        );
        let opened = opened.expect("a session opens");
        assert_eq!(asked.len(), 1, "asked once");
        let listed = &asked[0]["params"]["hooks"];
        assert_eq!(listed[0]["overlay"], "team", "{asked:?}");
        assert_eq!(listed[0]["digest"], json!(setup.digest));
        assert_eq!(listed[0]["failClosed"], false);
        let events: Vec<&str> = listed[0]["events"]
            .as_array()
            .expect("events")
            .iter()
            .filter_map(|each| each["event"].as_str())
            .collect();
        assert!(
            events.contains(&"session-start") && events.contains(&"prompt"),
            "{events:?}"
        );
        let pre_env = listed[0]["events"]
            .as_array()
            .expect("events")
            .iter()
            .find(|each| each["event"] == "pre-env")
            .expect("the default hook is bound to pre-env");
        assert_eq!(pre_env["fires"], false, "{pre_env}");
        assert!(
            pre_env["note"]
                .as_str()
                .is_some_and(|note| note.contains("canoa.cfg")),
            "{pre_env}"
        );
        let prompt = listed[0]["events"]
            .as_array()
            .expect("events")
            .iter()
            .find(|each| each["event"] == "prompt")
            .expect("the prompt hook");
        assert_eq!(prompt["timeout"], 5);
        assert_eq!(prompt["onFailure"], "report");
        assert_eq!(prompt["run"], hooked::script::file_name());
        let session = session_id(&opened);
        let ran = setup.agent_state(&session)["system"]
            .to_string()
            .contains("CTX-START");
        assert_eq!(ran, runs, "{answer:?}");
        let report = &opened["_meta"][PROFILE_META];
        assert_eq!(
            item(
                report,
                if runs {
                    "hooks.team.session-start"
                } else {
                    "hooks.team"
                }
            )["state"],
            if runs { "applied" } else { "off" },
            "{answer:?}: {report}"
        );
        setup.host.stop();
    }
}

#[test]
fn a_client_that_does_not_say_it_answers_is_never_asked_about_hooks() {
    let setup = start("", &[], &Allow::Nothing, &[]);
    setup.initialize(false);
    let (opened, asked, _) = setup.request(
        2,
        "session/new",
        json!({"cwd": setup.cwd, "mcpServers": []}),
        Some(json!({"allowHooks": ["team"]})),
    );
    opened.expect("a session opens");
    assert!(asked.is_empty());
    assert!(hooked::logged(&setup.log).is_empty());
    setup.host.stop();
}

fn heard_by(setup: &hooked::Setup, session: &str) -> Vec<String> {
    heard(setup, session)
}

#[test]
fn pre_env_fires_before_the_agent_starts_and_only_for_hooks_canoa_cfg_allows() {
    let setup = start("", &[], &Allow::Digest, &[]);
    // The host prepares a connection's profile from its first message.
    setup.initialize(false);
    let events = setup.events("pre-env", 1);
    assert_eq!(keys(&events[0]), ["cwd", "event", "harness"], "{events:?}");
    setup.host.stop();

    // Nobody can be asked before the agent starts, so a person's allowance for a session never reaches it.
    let setup = start("", &[], &Allow::Nothing, &[]);
    setup.initialize(true);
    let (opened, ..) = setup.request(
        2,
        "session/new",
        json!({"cwd": setup.cwd, "mcpServers": []}),
        Some(json!({"allow": [], "allowHooks": ["team"]})),
    );
    opened.expect("a session opens");
    setup.events("session-start", 1);
    assert!(
        hooked::logged(&setup.log)
            .iter()
            .all(|(event, _)| event != "pre-env")
    );
    setup.host.stop();
}

#[test]
fn a_hook_and_a_delegation_tool_share_one_connection_and_each_does_its_part() {
    let setup = start(
        "",
        &[("session-start", print(START))],
        &Allow::Digest,
        &["--system-prompt", "--mcp-http"],
    );
    // The client asks for the delegation tools in the same `initialize` that the hooks' report comes back on.
    let (init, _) = linked::ask(
        &setup.link,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}, "_meta": {canoa_host::DELEGATE_META: true}}),
    )
    .expect("initialize is answered");
    assert_eq!(
        item(&init["_meta"][PROFILE_META], "hooks.team.session-start")["state"],
        "applied",
        "{init}"
    );
    let opened = setup.new_session(2).expect("a session opens");
    let session = session_id(&opened);
    // The hook added its context to the session the host also gave the tool entry.
    let system = setup.agent_state(&session)["system"].to_string();
    assert!(system.contains("CTX-START"), "{system}");
    let ask_turn = |id: u64, text: &str| {
        let (answer, updates) = linked::ask(
            &setup.link,
            id,
            "session/prompt",
            json!({"sessionId": session, "prompt": [{"type": "text", "text": text}]}),
        )
        .expect("the turn is answered");
        assert_eq!(answer["stopReason"], "end_turn");
        linked::said(&updates)
    };
    let task: Value =
        serde_json::from_str(&ask_turn(3, "Delegate Scripted Say shared")).expect("a task id");
    let id = task["taskId"].as_str().expect("an id");
    let mut result = Value::Null;
    for turn in 4..40 {
        result =
            serde_json::from_str(&ask_turn(turn, &format!("Status {id} 5000"))).expect("a status");
        if result["state"] == "completed" {
            break;
        }
    }
    assert_eq!(result["state"], "completed", "{result}");
    assert_eq!(result["result"], "shared");
    setup.host.stop();
}
