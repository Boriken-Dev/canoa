//! A session opened with `dev.boriken.canoa/ephemeral` is kept out of the host's list of running sessions, as it is kept out of
//! its record: a side chat answers and goes. A session opened without it is listed as before.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

mod linked;
mod support;

use canoa_host::EPHEMERAL_META;
use linked::{Hosted, ask, entry};
use serde_json::json;
use support::cwd;

#[test]
fn an_ephemeral_session_is_not_listed_as_running_and_an_ordinary_one_is() {
    let host = Hosted::start(&entry("fixture", "shared = true"));
    let control = host.local.open("@control").expect("the control channel");
    let dir = cwd(&host.scratch);

    let side = host.local.open("fixture").expect("a link");
    ask(
        &side,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialized");
    let (opened, _) = ask(
        &side,
        2,
        "session/new",
        json!({"cwd": dir, "mcpServers": [], "_meta": {EPHEMERAL_META: true}}),
    )
    .expect("opens");
    let hidden = opened["sessionId"].as_str().expect("an id").to_owned();
    ask(
        &side,
        3,
        "session/prompt",
        json!({"sessionId": hidden, "prompt": [{"type": "text", "text": "hi"}]}),
    )
    .expect("answered");

    let (listed, _) = ask(&control, 1, "sessions/running", json!({})).expect("listed");
    assert_eq!(
        listed["sessions"].as_array().map(Vec::len),
        Some(0),
        "{listed}"
    );
    let (active, _) = ask(&control, 2, "sessions/activity", json!({})).expect("listed");
    assert_eq!(
        active["sessions"].as_array().map(Vec::len),
        Some(0),
        "{active}"
    );

    let plain = host.local.open("fixture").expect("a link");
    ask(
        &plain,
        1,
        "initialize",
        json!({"protocolVersion": 1, "clientCapabilities": {}}),
    )
    .expect("initialized");
    let (opened, _) = ask(
        &plain,
        2,
        "session/new",
        json!({"cwd": dir, "mcpServers": []}),
    )
    .expect("opens");
    let shown = opened["sessionId"].as_str().expect("an id").to_owned();
    let (listed, _) = ask(&control, 3, "sessions/running", json!({})).expect("listed");
    assert_eq!(
        listed["sessions"].as_array().map(Vec::len),
        Some(1),
        "{listed}"
    );
    assert_eq!(listed["sessions"][0]["session"], shown.as_str());
    host.stop();
}
