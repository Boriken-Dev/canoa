//! The host's own sources: served over `Local` only, with no address, found where the system finds programs, and said to be
//! missing when they are not. One test in this file, since it sets the process's `PATH`.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    unsafe_code,
    reason = "tests name what they use from std, and a test sets the environment its host reads"
)]

mod linked;
mod support;

use linked::{Hosted, ask, open_session, said};
use serde_json::json;
use support::{cwd, example};

#[test]
fn the_hosts_own_sources_run_over_a_link_and_say_what_they_miss() {
    // A `grok` that is the scripted agent, on a PATH of its own; the rest are not installed.
    let bin = support::Scratch::new();
    let name = format!("grok{}", std::env::consts::EXE_SUFFIX);
    std::fs::copy(example("stdio_agent", false), bin.0.join(&name)).expect("copied");
    let path = std::env::join_paths([bin.0.clone()]).expect("a path");
    // SAFETY: the only test in this process, set before any thread of the host starts.
    unsafe { std::env::set_var("PATH", &path) };

    let host = Hosted::start_with("", &["--local-sources"]);
    let names = host.local.entries();
    assert_eq!(names, ["Claude", "Codex", "Antigravity", "Grok", "Pi"]);

    let grok = host.line_of("Grok").expect("printed");
    assert_eq!(grok["local"], true);
    assert_eq!(grok["missing"], json!([]));
    assert!(grok.get("url").is_none(), "no address: {grok}");
    let claude = host.line_of("Claude").expect("printed");
    assert_eq!(claude["missing"], json!(["claude"]));
    assert_eq!(claude["harness"], "claude");
    assert_eq!(
        host.line_of("Pi").expect("printed")["missing"],
        json!(["pi"])
    );

    let link = host.local.open("Grok").expect("a link");
    let session = open_session(&link, &cwd(&host.scratch));
    let (answer, updates) = ask(
        &link,
        3,
        "session/prompt",
        json!({"sessionId": session, "prompt": [{"type": "text", "text": "hello"}]}),
    )
    .expect("the turn is answered");
    assert_eq!(answer["stopReason"], "end_turn");
    assert_eq!(said(&updates), "echo: hello");
    drop(link);
    host.stop();
}
