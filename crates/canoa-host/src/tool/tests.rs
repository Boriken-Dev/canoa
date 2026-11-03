#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use serde_json::json;

use super::template::{lookup, matches, render, render_json};
use super::*;

fn spec(toml_text: &str) -> Result<ToolSpec, String> {
    let spec: ToolSpec = toml::from_str(toml_text).map_err(|error| error.to_string())?;
    spec.check()?;
    Ok(spec)
}

const SESSION: &str = r#"
program = "pi"
args = ["--mode", "rpc"]
runs = "session"
unasked = "read, bash, edit and write"
prompt = { type = "prompt", message = "{prompt}" }
"#;

#[test]
fn a_path_reaches_into_objects_and_lists() {
    let event = json!({"a": {"b": [{"c": "x"}, 2]}, "d": null});
    assert_eq!(lookup(&event, "a.b.0.c"), Some(&json!("x")));
    assert_eq!(lookup(&event, "a.b.1"), Some(&json!(2)));
    assert_eq!(lookup(&event, "a.b.2"), None);
    assert_eq!(lookup(&event, "a.x"), None);
    assert_eq!(lookup(&event, "a.b.c"), None, "a list takes only numbers");
}

#[test]
fn a_template_fills_paths_and_keeps_what_is_not_one() {
    let event = json!({"name": "bash", "args": {"command": "ls"}, "n": 3, "none": null});
    assert_eq!(render("{name}: {args.command}", &event), "bash: ls");
    assert_eq!(
        render("{n} and {none} and {missing}.", &event),
        "3 and  and ."
    );
    assert_eq!(render("{args}", &event), r#"{"command":"ls"}"#);
    assert_eq!(render(r#"{"a":1} {} {name"#, &event), r#"{"a":1} {} {name"#);
}

#[test]
fn a_json_template_keeps_a_whole_placeholders_type_and_never_parses_text() {
    let event = json!({"id": 7, "text": "\"},\"x\":{\"", "flag": true});
    let built = render_json(
        &json!({"type": "reply", "id": "{id}", "message": "say {text}", "ok": "{flag}", "gone": "{nothing}"}),
        &event,
    );
    assert_eq!(
        built,
        json!({"type": "reply", "id": 7, "message": "say \"},\"x\":{\"", "ok": true, "gone": null})
    );
    let keys = render_json(&json!({"{text}": 1}), &event);
    assert_eq!(keys, json!({"{text}": 1}), "keys stay as written");
}

#[test]
fn when_needs_every_path_to_hold_its_value() {
    let event = json!({"type": "message_update", "assistantMessageEvent": {"type": "text_delta"}});
    let when = |value: serde_json::Value| value.as_object().cloned().unwrap_or_default();
    assert!(matches(&when(json!({})), &event));
    assert!(matches(
        &when(json!({"type": "message_update", "assistantMessageEvent.type": "text_delta"})),
        &event
    ));
    assert!(!matches(
        &when(json!({"type": "message_update", "assistantMessageEvent.type": "thinking_delta"})),
        &event
    ));
    assert!(
        !matches(&when(json!({"missing": null})), &event),
        "a missing path is not null"
    );
}

#[test]
fn a_session_tool_needs_its_prompt_line() {
    spec(SESSION).expect("a session tool");
    let error =
        spec("program = \"pi\"\nruns = \"session\"\nunasked = \"x\"\n").expect_err("no prompt");
    assert!(error.contains("prompt"), "{error}");
    let error = spec(&format!(
        "{SESSION}resume_args = [\"--session\", \"{{session}}\"]\n"
    ))
    .expect_err("resume");
    assert!(error.contains("resume_args"), "{error}");
}

#[test]
fn a_tool_must_say_what_it_does_unasked() {
    let error =
        spec("program = \"pi\"\nruns = \"message\"\nunasked = \"  \"\n").expect_err("blank");
    assert!(error.contains("unasked"), "{error}");
    assert!(toml::from_str::<ToolSpec>("program = \"pi\"\nruns = \"message\"\n").is_err());
}

#[test]
fn a_message_tool_takes_no_prompt_line() {
    let error = spec(
        "program = \"pi\"\nruns = \"message\"\nunasked = \"x\"\nprompt = { message = \"{prompt}\" }\n",
    )
    .expect_err("prompt");
    assert!(error.contains("prompt and cancel"), "{error}");
}

#[test]
fn each_rule_takes_its_own_fields_only() {
    for (rule, expected) in [
        ("send = \"text\"", "needs text"),
        (
            "send = \"text\"\ntext = \"x\"\ntitle = \"y\"",
            "does not take title",
        ),
        ("send = \"tool\"\nid = \"{id}\"", "needs title"),
        (
            "send = \"ask\"\ntitle = \"t\"\nallow = { v = 1 }",
            "needs reject",
        ),
        (
            "send = \"end\"\nstop = \"cancelled\"\nerror = \"x\"",
            "exclude each other",
        ),
        (
            "send = \"tool\"\nid = \"i\"\ntitle = \"t\"\ninput = \"a b\"",
            "input is a path",
        ),
        ("send = \"reply\"", "needs line"),
    ] {
        let error = spec(&format!("{SESSION}[[events]]\n{rule}\n")).expect_err(rule);
        assert!(error.contains(expected), "{rule}: {error}");
    }
    assert!(
        spec(&format!("{SESSION}[[events]]\nsend = \"shout\"\n")).is_err(),
        "an action this host does not know"
    );
}

#[test]
fn kinds_name_acp_tool_kinds_only() {
    spec(&format!("{SESSION}[kinds]\nbash = \"execute\"\n")).expect("execute is a kind");
    let error = spec(&format!("{SESSION}[kinds]\nbash = \"shell\"\n")).expect_err("not a kind");
    assert!(error.contains("kinds"), "{error}");
}

#[test]
fn notices_say_what_runs_unasked_and_what_is_forgotten() {
    let session = spec(SESSION).expect("parses");
    let notices = session.notices("pi");
    assert_eq!(notices.len(), 1);
    assert!(
        notices[0].contains("read, bash, edit and write"),
        "{notices:?}"
    );
    let message =
        spec("program = \"t\"\nruns = \"message\"\nunasked = \"everything\"\n").expect("parses");
    let notices = message.notices("t");
    assert_eq!(notices.len(), 2);
    assert!(notices[1].contains("does not remember"), "{notices:?}");
    let resuming = spec(
        "program = \"t\"\nruns = \"message\"\nunasked = \"x\"\nresume_args = [\"--session\", \"{session}\"]\n",
    )
    .expect("parses");
    assert!(
        resuming.notices("t")[1].contains("continues"),
        "{:?}",
        resuming.notices("t")
    );
}

#[test]
fn the_prompt_goes_on_the_command_line_only_where_args_say() {
    let message = |args: &str| {
        spec(&format!(
            "program = \"t\"\nruns = \"message\"\nunasked = \"x\"\nargs = {args}\n"
        ))
        .expect("parses")
    };
    assert!(message("[\"--\", \"{prompt}\"]").prompt_in_args());
    assert!(message("[\"--message={prompt}\"]").prompt_in_args());
    assert!(!message("[\"--json\"]").prompt_in_args());
}

#[test]
fn an_entry_survives_its_trip_through_json() {
    let spec = spec(&format!(
        "{SESSION}cancel = {{ type = \"abort\" }}\n[kinds]\nbash = \"execute\"\n[[events]]\nwhen = {{ type = \"x\", \"a.b\" = true }}\nsend = \"text\"\ntext = \"{{a.c}}\"\n"
    ))
    .expect("parses");
    let json = serde_json::to_string(&spec).expect("JSON");
    let back: ToolSpec = serde_json::from_str(&json).expect("reads back");
    assert_eq!(back, spec);
}

#[test]
fn a_line_written_holds_no_character_a_reader_could_split_it_at() {
    let value = json!({"message": "one\u{2028}two\u{2029}three\r\nfour"});
    let line = super::run::line_of(&value);
    assert_eq!(line.last(), Some(&b'\n'));
    let body = &line[..line.len() - 1];
    let text = core::str::from_utf8(body).expect("UTF-8");
    assert!(
        !text.contains(['\u{2028}', '\u{2029}', '\n', '\r']),
        "{text}"
    );
    assert_eq!(
        serde_json::from_slice::<serde_json::Value>(body).expect("JSON"),
        value
    );
}

#[test]
fn an_entry_fits_a_command_line() {
    let error =
        spec(&format!("{SESSION}max_sessions = 2\n").replace("read, bash", r"read\u0000bash"))
            .expect_err("a NUL");
    assert!(error.contains("NUL"), "{error}");
    let long = "x".repeat(MAX_SPEC);
    let error = spec(&SESSION.replace("read, bash, edit and write", &long)).expect_err("too long");
    assert!(error.contains("bytes"), "{error}");
}

#[test]
fn a_prompt_alone_in_the_arguments_comes_after_a_double_dash() {
    let message = |args: &str| {
        spec(&format!(
            "program = \"t\"\nruns = \"message\"\nunasked = \"x\"\nargs = {args}\n"
        ))
    };
    let error = message("[\"--json\", \"{prompt}\"]").expect_err("could be read as an option");
    assert!(error.contains("--"), "{error}");
    message("[\"--json\", \"--\", \"{prompt}\"]").expect("after --");
    message("[\"--message={prompt}\"]").expect("inside an option");
}
