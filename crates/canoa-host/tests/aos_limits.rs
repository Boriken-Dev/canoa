//! What a hostile overlay's text cannot do to the host: grow a value without bound, nest expressions past the stack, or
//! make a pattern take forever; and what counts as this computer when an MCP server is reached over plain `http`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::time::{Duration, Instant};

use canoa_host::aos::{self, Environment, Facts, Weakening};
use serde_json::{Map, json};

fn lookup_in<'a>(
    env: &'a Environment<'_>,
) -> impl Fn(&str) -> Result<Option<String>, aos::AosError> + 'a {
    move |name: &str| env.lookup(name)
}

#[test]
fn a_value_that_doubles_line_after_line_is_refused_before_it_is_large() {
    let mut text = String::from("A0=xxxxxxxxxxxxxxxxxxxxxxxxxxxxxxxx\n");
    for line in 1..48 {
        let before = line - 1;
        text.push_str(&format!("A{line}=${{A{before}}}${{A{before}}}\n"));
    }
    let mut env = Environment::new(false);
    let started = Instant::now();
    let error = aos::read_dotenv(text.as_bytes(), &mut env).expect_err("refused");
    assert_eq!(error.code, "too-large", "{error}");
    assert!(started.elapsed() < Duration::from_secs(5));
}

#[test]
fn expressions_nested_past_the_limit_are_refused_not_followed_down_the_stack() {
    let depth = 200_000;
    let text = "${U:-".repeat(depth) + "x" + &"}".repeat(depth);
    let env = Environment::new(false);
    let error = aos::substitute(&text, &lookup_in(&env)).expect_err("refused");
    assert_eq!(error.code, "too-deep", "{error}");
}

#[test]
fn a_pattern_that_would_take_forever_is_refused_quickly() {
    let mut env = Environment::new(false);
    env.set("A", &"a".repeat(20_000), false);
    let started = Instant::now();
    let error = aos::substitute("${A//*z/}", &lookup_in(&env)).expect_err("refused");
    assert_eq!(error.code, "too-complex", "{error}");
    assert!(
        started.elapsed() < Duration::from_secs(5),
        "{:?}",
        started.elapsed()
    );
    // An ordinary replacement still works.
    env.set("P", "/usr/bin:/bin", false);
    assert_eq!(
        aos::substitute("${P//:/ }", &lookup_in(&env)).expect("substituted"),
        "/usr/bin /bin"
    );
}

#[test]
fn only_a_loopback_address_or_localhost_is_this_computer_for_plain_http() {
    let manifest = |url: &str| -> Map<String, serde_json::Value> {
        json!({"mcp": {"servers": [{"name": "s", "transport": "streamable-http", "url": url}]}})
            .as_object()
            .expect("a mapping")
            .clone()
    };
    let insecure = |url: &str| {
        let mut found = Vec::new();
        let _ = aos::grants_report(&manifest(url), &Facts::new("linux"), &mut found);
        found.contains(&Weakening::new("insecure-transport", "s"))
    };
    for remote in [
        "http://127.0.0.1.attacker.example/mcp",
        "http://127.evil.example:80/x",
        "http://localhost.attacker.example/",
        "http://127.0.0.256/x",
    ] {
        assert!(insecure(remote), "{remote} is another host");
    }
    for local in [
        "http://127.0.0.1:9000/x",
        "http://127.1.2.3/x",
        "http://localhost/x",
        "http://LOCALHOST:8080/",
        "http://[::1]:7000/x",
    ] {
        assert!(!insecure(local), "{local} is this computer");
    }
}

#[test]
fn a_harness_hint_that_is_not_one_plain_line_is_not_applied() {
    use canoa_host::aos::TreeEntry;
    use canoa_host::profile::{OverlayInput, overlay_layer};
    let manifest = "[harness.claude-code]\nmodel = \"claude\\nfake: row\"\neffort = \"high\"\n";
    let tree = [TreeEntry {
        components: vec![b"overlay.cfg".to_vec()],
        kind: aos::EntryKind::File,
        data: manifest.as_bytes().to_vec(),
    }];
    let mut input = OverlayInput::new("o", "/ov/o", "linux", &tree);
    input.harness = Some("claude-code");
    let layer = overlay_layer(&input).expect("applies");
    assert_eq!(
        layer.profile.model, None,
        "a model holding a line break is not applied"
    );
    assert_eq!(layer.profile.effort.as_deref(), Some("high"));
    let not_applied = &layer.aos.as_ref().expect("an AOS layer").not_applied;
    assert!(
        not_applied
            .iter()
            .any(|item| item.starts_with("harness.claude-code.model")),
        "{not_applied:?}"
    );
}
