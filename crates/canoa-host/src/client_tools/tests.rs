use std::path::Path;

use super::*;
use crate::local;

fn hub() -> Arc<Hub> {
    Hub::new(BridgeCommand::new(Path::new("canoa"), &[]))
}

fn who() -> Who {
    Who {
        agent: "Claude".to_owned(),
        via: Via::App,
    }
}

fn using() -> BTreeSet<Grant> {
    [Grant::ToolsUse].into()
}

fn providing() -> BTreeSet<Grant> {
    [Grant::ToolsProvide].into()
}

/// A connection of a provider with its stand-in's end, served on a thread of its own.
fn connect(
    hub: &Arc<Hub>,
    key: &str,
    grants: BTreeSet<Grant>,
) -> (local::Link, std::thread::JoinHandle<()>) {
    let stopped = Arc::new(AtomicBool::new(false));
    let (client, host) = local::pair(MAX_MESSAGE, stopped);
    let (hub, key) = (Arc::clone(hub), key.to_owned());
    let thread = std::thread::spawn(move || {
        hub.provide(&key, &grants, &host);
        drop(host);
    });
    (client, thread)
}

fn say(link: &local::Link, message: &Value) {
    link.send(message.to_string().as_bytes()).expect("sent");
}

fn hear(link: &local::Link) -> Value {
    let bytes = link
        .receive(Some(Duration::from_secs(5)))
        .expect("a message")
        .expect("not ended");
    serde_json::from_slice(&bytes).expect("json")
}

fn register(link: &local::Link, provider: &str) -> Value {
    say(link, &json!({"type": "register", "provider": provider}));
    hear(link)
}

fn initialize() -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 1, "method": "initialize", "params": {"protocolVersion": 1}}))
        .expect("json")
}

fn session_new(id: u64) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": "session/new",
        "params": {"cwd": "/work", "mcpServers": []}}))
    .expect("json")
}

fn answered(id: u64, result: &Value) -> Vec<u8> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": result})).expect("json")
}

fn servers(message: &[u8]) -> Vec<Value> {
    let value: Value = serde_json::from_slice(message).expect("json");
    value["params"]["mcpServers"]
        .as_array()
        .cloned()
        .unwrap_or_default()
}

/// A link that has been through `initialize` with an agent that takes HTTP servers, and a provider that has been seen.
fn ready(hub: &Arc<Hub>, grants: Option<&BTreeSet<Grant>>) -> Arc<Link> {
    let link = hub.for_connection(grants, who()).expect("a link");
    let _ = link.client_said(initialize());
    link.agent_said(&answered(
        1,
        &json!({"protocolVersion": 1, "agentCapabilities": {"mcpCapabilities": {"http": true}}}),
    ));
    link
}

#[test]
fn a_providers_name_is_a_plain_lowercase_word() {
    for good in ["browser", "office", "word_2", "a-b"] {
        assert!(plain_name(good), "{good}");
    }
    for bad in [
        "",
        "Browser",
        "1browser",
        "a b",
        "a/b",
        "@tools",
        &"x".repeat(33),
    ] {
        assert!(!plain_name(bad), "{bad}");
    }
}

#[test]
fn only_a_key_that_holds_tools_use_is_given_the_tools_and_a_task_never_is() {
    let hub = hub();
    assert!(
        hub.for_connection(None, who()).is_none(),
        "a task another agent handed out"
    );
    assert!(hub.for_connection(Some(&BTreeSet::new()), who()).is_none());
    assert!(
        hub.for_connection(Some(&providing()), who()).is_none(),
        "offering is not using"
    );
    assert!(hub.for_connection(Some(&using()), who()).is_some());
}

#[test]
fn a_key_without_tools_provide_is_refused_and_nothing_is_registered() {
    let hub = hub();
    let (link, thread) = connect(&hub, "k", using());
    let refused = hear(&link);
    assert_eq!(refused["type"], "refused");
    thread.join().expect("ends");
    assert!(hub.expected().is_empty());
}

#[test]
fn a_first_message_that_does_not_register_ends_the_connection() {
    let hub = hub();
    let (link, thread) = connect(&hub, "k", providing());
    for bad in [
        json!({"type": "open"}),
        json!({"type": "register", "provider": "Bad Name"}),
    ] {
        let (link, thread) = connect(&hub, "k", providing());
        say(&link, &bad);
        assert_eq!(hear(&link)["type"], "refused", "{bad}");
        thread.join().expect("ends");
    }
    drop(link);
    thread.join().expect("a link that says nothing ends");
    assert!(hub.expected().is_empty());
}

#[test]
fn another_key_cannot_take_a_name_a_provider_holds_but_the_same_key_replaces_itself() {
    let hub = hub();
    let (first, first_thread) = connect(&hub, "key-a", providing());
    assert_eq!(register(&first, "browser")["type"], "registered");

    let (other, other_thread) = connect(&hub, "key-b", providing());
    assert_eq!(register(&other, "browser")["type"], "refused");
    other_thread.join().expect("ends");

    let (again, again_thread) = connect(&hub, "key-a", providing());
    assert_eq!(register(&again, "browser")["type"], "registered");
    // The first connection was replaced, and ends.
    first_thread.join().expect("the replaced connection ends");
    drop(again);
    again_thread.join().expect("ends");
    assert_eq!(
        hub.expected(),
        ["browser"],
        "the name is remembered after the provider left"
    );
    drop(first);
}

#[test]
fn a_session_request_is_given_one_server_for_each_provider_the_host_has_seen() {
    let hub = hub();
    let link = ready(&hub, Some(&using()));
    let untouched = link.client_said(session_new(2));
    assert!(
        servers(&untouched).is_empty(),
        "no provider has connected yet"
    );

    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let given = servers(&link.client_said(session_new(3)));
    assert_eq!(given.len(), 1, "{given:?}");
    assert_eq!(given[0]["name"], "canoa-browser");
    assert_eq!(given[0]["type"], "http");
    let bearer = given[0]["headers"][0]["value"].as_str().expect("a bearer");
    assert!(
        bearer.starts_with("Bearer ") && bearer.len() > 40,
        "{bearer}"
    );
    assert!(
        given[0]["url"]
            .as_str()
            .expect("a url")
            .starts_with("http://127.0.0.1:")
    );
    drop(provider);
    thread.join().expect("ends");
    // Seen once, so a session that starts after the browser closed still gets the server.
    let later = servers(&link.client_said(session_new(4)));
    assert_eq!(later.len(), 1);
    link.close();
}

#[test]
fn an_agent_that_takes_only_stdio_is_given_the_bridge_with_the_address_and_bearer_in_its_environment()
 {
    let hub = hub();
    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let link = hub.for_connection(Some(&using()), who()).expect("a link");
    let _ = link.client_said(initialize());
    link.agent_said(&answered(1, &json!({"protocolVersion": 1})));
    let given = servers(&link.client_said(session_new(2)));
    assert_eq!(given[0]["command"], "canoa");
    assert_eq!(given[0]["args"], json!(["mcp-bridge"]));
    let names: Vec<&str> = given[0]["env"]
        .as_array()
        .expect("env")
        .iter()
        .filter_map(|item| item["name"].as_str())
        .collect();
    assert_eq!(
        names,
        [
            crate::delegate::bridge::URL_VAR,
            crate::delegate::bridge::TOKEN_VAR
        ]
    );
    link.close();
    drop(provider);
    thread.join().expect("ends");
}

fn page_request(id: u64, own: Option<bool>) -> Vec<u8> {
    let mut site = json!({"site": "example.com"});
    if let Some(own) = own {
        site["own"] = json!(own);
    }
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "method": "session/new",
        "params": {"cwd": "/", "mcpServers": [], "_meta": {SITE_META: site}}}))
    .expect("json")
}

#[test]
fn a_session_a_page_started_is_given_no_server_and_one_the_persons_own_pages_started_is() {
    let hub = hub();
    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let link = ready(&hub, Some(&using()));
    // What the scope passes on has lost the mark; the peek read it before.
    let forwarded = |id: u64| session_new(id);
    link.client_raw(&page_request(2, None));
    assert!(
        servers(&link.client_said(forwarded(2))).is_empty(),
        "a page's session"
    );
    link.client_raw(&page_request(3, Some(false)));
    assert!(
        servers(&link.client_said(forwarded(3))).is_empty(),
        "a page that says it is not own"
    );
    link.client_raw(&page_request(4, Some(true)));
    assert_eq!(
        servers(&link.client_said(forwarded(4))).len(),
        1,
        "the extension's own pages"
    );
    // A request nobody marked is not a page's.
    assert_eq!(servers(&link.client_said(forwarded(5))).len(), 1);
    // The mark belongs to its request alone.
    assert_eq!(servers(&link.client_said(forwarded(2))).len(), 1);
    link.close();
    drop(provider);
    thread.join().expect("ends");
}

#[test]
fn a_page_reopening_a_session_is_marked_too() {
    let hub = hub();
    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let link = ready(&hub, Some(&using()));
    let raw = serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 7, "method": "session/load",
        "params": {"sessionId": "s", "cwd": "/", "mcpServers": [], "_meta": {SITE_META: {"site": "a.example"}}}}))
    .expect("json");
    link.client_raw(&raw);
    let forwarded =
        serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": 7, "method": "session/load",
        "params": {"sessionId": "s", "cwd": "/", "mcpServers": []}}))
        .expect("json");
    assert!(servers(&link.client_said(forwarded)).is_empty());
    link.close();
    drop(provider);
    thread.join().expect("ends");
}

#[test]
fn ending_a_session_or_the_connection_ends_its_bearer_and_tells_the_provider() {
    let hub = hub();
    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let link = ready(&hub, Some(&using()));
    let given = servers(&link.client_said(session_new(2)));
    assert_eq!(given.len(), 1);
    link.agent_said(&answered(2, &json!({"sessionId": "one"})));
    assert_eq!(hub.tokens.len(), 1);
    let closing = serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": 3, "method": "session/close",
        "params": {"sessionId": "one"}}),
    )
    .expect("json");
    let _ = link.client_said(closing);
    assert_eq!(hub.tokens.len(), 0, "its bearer stopped working");

    // A session the agent refused has no use for one.
    let _ = link.client_said(session_new(4));
    assert_eq!(hub.tokens.len(), 1);
    link.agent_said(
        &serde_json::to_vec(
            &json!({"jsonrpc": "2.0", "id": 4, "error": {"code": -1, "message": "no"}}),
        )
        .expect("json"),
    );
    assert_eq!(hub.tokens.len(), 0);

    let _ = link.client_said(session_new(5));
    link.agent_said(&answered(5, &json!({"sessionId": "two"})));
    assert_eq!(hub.tokens.len(), 1);
    link.close();
    assert_eq!(hub.tokens.len(), 0, "the connection ended every bearer");
    drop(provider);
    thread.join().expect("ends");
}

#[test]
fn a_provider_hears_a_stream_open_forwards_what_the_agent_asks_and_answers_it() {
    let hub = hub();
    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let link = ready(&hub, Some(&using()));
    let _ = link.client_said(session_new(2));
    let access = lock(&hub.tokens.0)
        .values()
        .next()
        .cloned()
        .expect("a bearer");
    let asked = json!({"jsonrpc": "2.0", "id": 9, "method": "tools/list"});
    let asking = {
        let (hub, access) = (Arc::clone(&hub), Arc::clone(&access));
        std::thread::spawn(move || hub.answer(&access, &asked))
    };
    let opened = hear(&provider);
    assert_eq!(opened["type"], "open");
    assert_eq!(opened["session"], json!({"agent": "Claude", "via": "app"}));
    let stream = opened["stream"].as_str().expect("a stream").to_owned();
    let forwarded = hear(&provider);
    assert_eq!(forwarded["message"]["method"], "tools/list");
    assert_eq!(forwarded["stream"], stream);
    // An answer on a stream this connection never opened is ignored; the right one is delivered.
    say(
        &provider,
        &json!({"type": "mcp", "stream": "other", "message": {"jsonrpc": "2.0", "id": 9, "result": {"tools": [{"name": "wrong"}]}}}),
    );
    say(
        &provider,
        &json!({"type": "mcp", "stream": stream, "message": {"jsonrpc": "2.0", "id": 9, "result": {"tools": []}}}),
    );
    let answer = asking.join().expect("answers").expect("an answer");
    assert_eq!(answer["result"]["tools"], json!([]));
    link.close();
    assert_eq!(hear(&provider), json!({"type": "close", "stream": stream}));
    drop(provider);
    thread.join().expect("ends");
}

#[test]
fn no_connected_provider_gives_an_empty_list_and_a_call_says_why() {
    let hub = hub();
    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let link = ready(&hub, Some(&using()));
    let _ = link.client_said(session_new(2));
    drop(provider);
    thread.join().expect("ends");
    let access = lock(&hub.tokens.0)
        .values()
        .next()
        .cloned()
        .expect("a bearer");
    let listed = hub
        .answer(
            &access,
            &json!({"jsonrpc": "2.0", "id": 1, "method": "tools/list"}),
        )
        .expect("answered");
    assert_eq!(listed["result"]["tools"], json!([]));
    let called = hub
        .answer(
            &access,
            &json!({"jsonrpc": "2.0", "id": 2, "method": "tools/call", "params": {"name": "browser_read_page", "arguments": {}}}),
        )
        .expect("answered");
    assert_eq!(called["result"]["isError"], true);
    assert!(
        called["result"]["content"][0]["text"]
            .as_str()
            .expect("text")
            .contains("not connected")
    );
    let unknown = hub
        .answer(
            &access,
            &json!({"jsonrpc": "2.0", "id": 3, "method": "resources/list"}),
        )
        .expect("answered");
    assert_eq!(unknown["error"]["code"], -32601);
    assert!(
        hub.answer(
            &access,
            &json!({"jsonrpc": "2.0", "method": "notifications/initialized"})
        )
        .is_none()
    );
    link.close();
}

#[test]
fn a_provider_leaving_tells_every_session_its_list_changed() {
    let hub = hub();
    let (provider, thread) = connect(&hub, "k", providing());
    register(&provider, "browser");
    let link = ready(&hub, Some(&using()));
    let _ = link.client_said(session_new(2));
    let access = lock(&hub.tokens.0)
        .values()
        .next()
        .cloned()
        .expect("a bearer");
    assert!(lock(&access.notes).is_empty());
    drop(provider);
    thread.join().expect("ends");
    let Note::Message(told) = access.next_note() else {
        panic!("the session was told");
    };
    assert!(told.contains("notifications/tools/list_changed"), "{told}");
    link.close();
    assert!(matches!(access.next_note(), Note::Closed));
}
