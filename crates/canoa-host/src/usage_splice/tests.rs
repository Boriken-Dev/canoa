#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use core::time::Duration;
use std::cell::RefCell;
use std::sync::Arc;

use coqui_acp_api::usage::Bucket;

use super::*;

const AGY: &str = "Agent execution terminated due to error.\nError: HTTP 429 Too Many Requests\n{ \"error\": { \"code\": 429, \"message\": \"Resource has been exhausted (e.g. check quota).\", \"status\": \"RESOURCE_EXHAUSTED\" } }";
const GATEWAY: &str =
    "Rate limit exceeded for api_key: tokens. Limit reset at: 2099-01-01T00:00:00Z";
const PROVIDER: &str =
    "litellm.RateLimitError: RateLimitError: OpenAIException - Rate limit reached for gpt";
const CAPACITY: &str = "UNAVAILABLE (code 503): No capacity available for model gemini";
/// How long a test waits for an event it needs; a loaded machine is slow, a wait that ends sooner than this is not a bound.
const PATIENCE: Duration = Duration::from_secs(300);
/// 2099-01-01T00:00:00Z.
const FAR: i64 = 4_070_908_800;

fn splice(phrases: Option<Vec<&str>>) -> Splice {
    let phrases = phrases.map(|phrases| phrases.into_iter().map(str::to_owned).collect());
    Splice::new(Arc::new(Inference::new(phrases)), Some("work".to_owned()))
}

fn bytes(value: &Value) -> Vec<u8> {
    serde_json::to_vec(value).expect("JSON")
}

/// What the client is sent when the agent says `message`.
fn from_agent(splice: &Splice, message: &Value) -> Vec<Value> {
    let sent = RefCell::new(Vec::new());
    splice
        .relay(&bytes(message), |out| {
            sent.borrow_mut()
                .push(serde_json::from_slice(out).expect("JSON"));
            Ok(())
        })
        .expect("sent");
    sent.into_inner()
}

fn read_client(splice: &Splice, message: &Value) -> Option<Value> {
    match splice.read_client(&bytes(message)) {
        Client::Answer(answer) => Some(serde_json::from_slice(&answer).expect("JSON")),
        Client::Forward => None,
        Client::Held => panic!("an answer that waits for a reading"),
    }
}

fn initialize(splice: &Splice, notify: bool, answer: Value) -> Value {
    let capabilities = json!({"_meta": {USAGE_CAPABILITY: {"notify": notify}}});
    assert!(
        read_client(
            splice,
            &json!({"id": 0, "method": "initialize",
                "params": {"protocolVersion": 1, "clientCapabilities": capabilities}})
        )
        .is_none()
    );
    let mut sent = from_agent(splice, &json!({"id": 0, "result": answer}));
    sent.remove(0)
}

fn plain_answer() -> Value {
    json!({"protocolVersion": 1, "agentCapabilities": {"loadSession": true}})
}

fn prompt(splice: &Splice, id: u64, session: &str) {
    assert!(
        read_client(
            splice,
            &json!({"id": id, "method": "session/prompt",
                "params": {"sessionId": session, "prompt": [{"type": "text", "text": "go"}]}})
        )
        .is_none()
    );
}

fn failed(id: u64, message: &str) -> Value {
    json!({"id": id, "error": {"code": -32603, "message": message}})
}

/// The report the host answers a usage request with. A reading older than the cache makes the host hold the request until
/// a new reading ends, so the answer is then collected as the connection would send it.
fn report_of(splice: &Splice) -> Value {
    let request = json!({"id": 99, "method": USAGE_METHOD, "params": {}});
    match splice.read_client(&bytes(&request)) {
        Client::Answer(answer) => {
            serde_json::from_slice::<Value>(&answer).expect("JSON")["result"].clone()
        }
        Client::Forward => panic!("the host answers"),
        Client::Held => {
            let answer = RefCell::new(None);
            let until = std::time::Instant::now() + PATIENCE;
            while answer.borrow().is_none() && std::time::Instant::now() < until {
                splice.tick(|message| {
                    let message: Value = serde_json::from_slice(message).expect("JSON");
                    if message["id"] == 99 {
                        *answer.borrow_mut() = Some(message["result"].clone());
                    }
                    Ok(())
                });
                std::thread::sleep(Duration::from_millis(20));
            }
            answer.into_inner().expect("the held request is answered")
        }
    }
}

#[test]
fn an_agent_with_no_usage_capability_is_given_one_with_the_source_inferred() {
    let splice = splice(None);
    let answered = initialize(&splice, false, plain_answer());
    let capability = &answered["result"]["agentCapabilities"]["_meta"][USAGE_CAPABILITY];
    assert_eq!(capability["version"], 1, "{answered}");
    assert_eq!(capability["sources"], json!(["inferred"]), "{answered}");
    assert_eq!(answered["result"]["agentCapabilities"]["loadSession"], true);
    let v2 = self::splice(None);
    let capabilities = json!({"_meta": {}});
    assert!(
        read_client(
            &v2,
            &json!({"id": 0, "method": "initialize",
        "params": {"protocolVersion": 2, "capabilities": capabilities}})
        )
        .is_none()
    );
    let sent = from_agent(
        &v2,
        &json!({"id": 0, "result": {"protocolVersion": 2, "capabilities": {}}}),
    );
    assert_eq!(
        sent[0]["result"]["capabilities"]["_meta"][USAGE_CAPABILITY]["sources"],
        json!(["inferred"]),
        "{sent:?}"
    );
}

#[test]
fn an_agent_that_advertises_usage_passes_untouched() {
    let splice = splice(None);
    let theirs = json!({"protocolVersion": 1, "agentCapabilities": {"_meta": {USAGE_CAPABILITY: {"version": 1, "sources": ["reported"]}}}});
    let answered = initialize(&splice, true, theirs.clone());
    assert_eq!(answered["result"], theirs);
    // Its request goes on to it, its failed turn is not read and its notice is not added to.
    assert!(
        read_client(
            &splice,
            &json!({"id": 5, "method": USAGE_METHOD, "params": {}})
        )
        .is_none()
    );
    prompt(&splice, 6, "s1");
    let sent = from_agent(&splice, &failed(6, AGY));
    assert_eq!(sent.len(), 1, "{sent:?}");
    assert_eq!(splice.inference.report(0, None).buckets.len(), 0);
}

#[test]
fn a_failed_turn_with_agys_words_makes_an_exhausted_inferred_bucket() {
    let splice = splice(None);
    initialize(&splice, false, plain_answer());
    let empty = report_of(&splice);
    assert_eq!(empty["supported"], true);
    assert_eq!(
        empty["buckets"],
        json!([]),
        "nothing yet is not unsupported"
    );
    assert_eq!(empty["revision"], 0);
    prompt(&splice, 1, "s1");
    let sent = from_agent(&splice, &failed(1, AGY));
    assert_eq!(
        sent,
        vec![failed(1, AGY)],
        "the error reaches the client as it came"
    );
    let report = report_of(&splice);
    assert_eq!(report["account"], "work", "{report}");
    assert_eq!(report["sources"], json!(["inferred"]));
    assert_eq!(report["revision"], 1);
    let bucket = &report["buckets"][0];
    assert_eq!(bucket["id"], "inferred", "{report}");
    assert_eq!(bucket["status"], "exhausted");
    assert_eq!(bucket["source"], "inferred");
    assert_eq!(bucket["evidence"], "Resource has been exhausted");
    let seen = report["observed_at"].as_i64().expect("a time");
    assert!((unix_now() - seen).abs() < 60, "the host's clock: {report}");
}

#[test]
fn litellms_texts_read_a_reset_from_the_gateway_and_none_from_the_provider() {
    let gateway = splice(None);
    initialize(&gateway, false, plain_answer());
    prompt(&gateway, 1, "s1");
    from_agent(
        &gateway,
        &json!({"id": 1, "error": {"code": 429, "message": "Rate limit exceeded", "data": {"detail": GATEWAY}}}),
    );
    let report = report_of(&gateway);
    assert_eq!(report["limit"]["resets_at"], FAR, "{report}");
    assert_eq!(report["buckets"][0]["refill"]["resets_at"], FAR, "{report}");
    let provider = self::splice(None);
    initialize(&provider, false, plain_answer());
    prompt(&provider, 1, "s1");
    from_agent(&provider, &failed(1, PROVIDER));
    let report = report_of(&provider);
    assert_eq!(report["buckets"][0]["status"], "exhausted", "{report}");
    assert_eq!(
        report["buckets"][0]["refill"]["kind"], "unknown",
        "{report}"
    );
}

#[test]
fn a_capacity_error_is_not_a_limit() {
    let splice = splice(None);
    initialize(&splice, true, plain_answer());
    prompt(&splice, 1, "s1");
    assert_eq!(from_agent(&splice, &failed(1, CAPACITY)).len(), 1);
    assert_eq!(report_of(&splice)["buckets"], json!([]));
}

#[test]
fn a_refusal_is_read_from_what_the_agent_said_in_the_turn() {
    let splice = splice(None);
    initialize(&splice, false, plain_answer());
    prompt(&splice, 1, "s1");
    let chunk = |text: &str| {
        json!({"method": "session/update", "params": {"sessionId": "s1",
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})
    };
    from_agent(&splice, &chunk("You have hit your "));
    from_agent(&splice, &chunk("usage limit. Retry after 2 hours."));
    from_agent(
        &splice,
        &json!({"id": 1, "result": {"stopReason": "refusal"}}),
    );
    let report = report_of(&splice);
    assert_eq!(report["buckets"][0]["evidence"], "usage limit", "{report}");
    assert_eq!(report["limit"]["retry_after"], json!([7200, 0]), "{report}");
    // Words said in another turn are not carried over.
    prompt(&splice, 2, "s1");
    from_agent(
        &splice,
        &json!({"id": 2, "result": {"stopReason": "refusal"}}),
    );
    assert_eq!(report_of(&splice)["revision"], 1);
}

#[test]
fn the_client_that_asked_is_told_when_the_report_changes_and_the_one_that_did_not_is_not() {
    let entry = Arc::new(Inference::default());
    let asking = Splice::new(Arc::clone(&entry), Some("work".to_owned()));
    let quiet = Splice::new(Arc::clone(&entry), Some("work".to_owned()));
    initialize(&asking, true, plain_answer());
    initialize(&quiet, false, plain_answer());
    prompt(&asking, 1, "s1");
    let sent = from_agent(&asking, &failed(1, AGY));
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(sent[1]["method"], USAGE_NOTIFICATION);
    assert_eq!(sent[1]["params"]["report"]["account"], "work");
    assert_eq!(sent[1]["params"]["report"]["revision"], 1);
    // The same turn failing again changes nothing.
    prompt(&asking, 2, "s1");
    assert_eq!(from_agent(&asking, &failed(2, AGY)).len(), 1);
    // The other connection of the entry is told with its next message, once, and only if it asked.
    let chunk = json!({"method": "session/update", "params": {"sessionId": "s2", "update": {"sessionUpdate": "plan"}}});
    assert_eq!(from_agent(&quiet, &chunk).len(), 1);
    let sent = from_agent(&asking, &chunk);
    assert_eq!(sent.len(), 1, "told already: {sent:?}");
}

#[test]
fn a_connection_that_joins_a_limited_entry_is_told_at_once() {
    let entry = Arc::new(Inference::default());
    assert!(entry.observe(AGY, unix_now()));
    let late = Splice::new(entry, None);
    let answered = initialize(&late, true, plain_answer());
    assert!(answered["result"].is_object());
    let again = from_agent(
        &late,
        &json!({"method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "plan"}}}),
    );
    assert_eq!(again.len(), 1, "the notice went with the initialize answer");
}

#[test]
fn with_no_phrases_the_host_stays_out_of_the_way() {
    let splice = splice(Some(Vec::new()));
    let answered = initialize(&splice, true, plain_answer());
    assert_eq!(answered["result"], plain_answer(), "no capability is added");
    assert!(
        read_client(
            &splice,
            &json!({"id": 3, "method": USAGE_METHOD, "params": {}})
        )
        .is_none()
    );
    prompt(&splice, 1, "s1");
    assert_eq!(from_agent(&splice, &failed(1, AGY)).len(), 1);
    assert_eq!(splice.inference.report(0, None).buckets.len(), 0);
}

#[test]
fn a_list_of_phrases_replaces_the_defaults() {
    let splice = splice(Some(vec!["out of credits"]));
    initialize(&splice, false, plain_answer());
    prompt(&splice, 1, "s1");
    from_agent(&splice, &failed(1, AGY));
    assert_eq!(
        report_of(&splice)["buckets"],
        json!([]),
        "a default phrase no longer counts"
    );
    prompt(&splice, 2, "s1");
    from_agent(&splice, &failed(2, "You are out of credits"));
    assert_eq!(
        report_of(&splice)["buckets"][0]["evidence"],
        "out of credits"
    );
}

#[test]
fn a_message_the_channel_could_not_take_is_read_when_it_is_offered_again() {
    let splice = splice(None);
    let capabilities = json!({"_meta": {USAGE_CAPABILITY: {"notify": false}}});
    assert!(
        read_client(
            &splice,
            &json!({"id": 0, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": capabilities}})
        )
        .is_none()
    );
    let answer = bytes(&json!({"id": 0, "result": plain_answer()}));
    assert!(matches!(
        splice.relay(&answer, |_| Err(Failure::NoProgress)),
        Err(Failure::NoProgress)
    ));
    let sent = RefCell::new(Vec::new());
    splice
        .relay(&answer, |out| {
            sent.borrow_mut()
                .push(serde_json::from_slice::<Value>(out).expect("JSON"));
            Ok(())
        })
        .expect("sent");
    assert_eq!(
        sent.borrow()[0]["result"]["agentCapabilities"]["_meta"][USAGE_CAPABILITY]["sources"],
        json!(["inferred"])
    );
}

#[test]
fn a_hit_lapses_at_its_reset_or_its_retry_after_and_otherwise_with_a_good_turn() {
    let reset = Inference::default();
    assert!(reset.observe(GATEWAY, 1_000));
    assert_eq!(reset.report(FAR - 1, None).buckets.len(), 1);
    let later = reset.report(FAR, None);
    assert_eq!(later.buckets.len(), 0, "the reset passed");
    assert_eq!(later.revision, 2, "and that was a change");
    // A good turn does not clear a hit whose reset has not passed.
    let held = Inference::default();
    held.observe(GATEWAY, 1_000);
    assert!(!held.succeeded(2_000));
    assert_eq!(held.report(2_000, None).buckets.len(), 1);

    let waiting = Inference::default();
    waiting.observe("Error 429. Retry after 90 seconds", 1_000);
    assert_eq!(waiting.report(1_089, None).buckets.len(), 1);
    assert_eq!(waiting.report(1_090, None).buckets.len(), 0);

    let unknown = Inference::default();
    unknown.observe("Error 429", 1_000);
    assert_eq!(
        unknown.report(9_999_999, None).buckets.len(),
        1,
        "no clock ends it"
    );
    assert!(unknown.succeeded(1_001));
    assert_eq!(unknown.report(1_001, None).buckets.len(), 0);
    assert!(!unknown.succeeded(1_002), "nothing left to clear");
}

#[test]
fn a_repeat_changes_nothing_but_a_new_wait_moves_the_end() {
    let unknown = Inference::default();
    assert!(unknown.observe("Error 429", 1_000));
    assert!(!unknown.observe("Error 429", 1_005));
    assert_eq!(unknown.report(1_005, None).revision, 1);
    let waiting = Inference::default();
    assert!(waiting.observe("Error 429. Retry after 60 seconds", 1_000));
    assert!(waiting.observe("Error 429. Retry after 60 seconds", 1_030));
    assert_eq!(
        waiting.report(1_089, None).buckets.len(),
        1,
        "counted from the later sighting"
    );
}

#[test]
fn a_reset_already_past_in_the_text_is_no_limit() {
    let inference = Inference::default();
    assert!(!inference.observe(
        "Error 429, limit reset at 2001-01-01T00:00:00Z",
        1_700_000_000
    ));
    assert_eq!(inference.report(1_700_000_000, None).revision, 0);
}

#[test]
fn the_lapse_of_a_hit_is_told_to_a_client_that_asked() {
    let entry = Arc::new(Inference::default());
    let seen = unix_now();
    assert!(entry.observe("Error 429. Retry after 1 second", seen));
    let splice = Splice::new(Arc::clone(&entry), None);
    let chunk = json!({"method": "session/update", "params": {"sessionId": "s", "update": {"sessionUpdate": "plan"}}});
    let first = initialize(&splice, true, plain_answer());
    assert!(first["result"].is_object());
    while unix_now() <= seen + 1 {
        std::thread::sleep(std::time::Duration::from_millis(100));
    }
    let sent = from_agent(&splice, &chunk);
    assert_eq!(sent.len(), 2, "{sent:?}");
    assert_eq!(
        sent[1]["params"]["report"]["buckets"],
        json!([]),
        "the hit lapsed"
    );
}

fn usage_notice() -> Value {
    json!({"jsonrpc": "2.0", "method": USAGE_NOTIFICATION, "params": {"report": {
        "supported": true, "sources": ["reported"], "revision": 9, "observed_at": 1, "buckets": []}}})
}

#[test]
fn the_agents_own_usage_notice_is_dropped_while_the_host_speaks_for_it() {
    let splice = splice(None);
    initialize(&splice, true, plain_answer());
    assert!(from_agent(&splice, &usage_notice()).is_empty());
    // An answer of the agent's to a request the host answered itself is dropped too.
    let asked = read_client(
        &splice,
        &json!({"id": 41, "method": USAGE_METHOD, "params": {}}),
    );
    assert!(asked.is_some());
    let forged =
        json!({"id": 41, "result": {"supported": true, "buckets": [], "sources": ["reported"]}});
    assert!(from_agent(&splice, &forged).is_empty());
    // The client may use the id again for a request that does go to the agent.
    assert!(
        read_client(
            &splice,
            &json!({"id": 41, "method": "session/new", "params": {}})
        )
        .is_none()
    );
    assert_eq!(
        from_agent(&splice, &json!({"id": 41, "result": {"sessionId": "s"}})).len(),
        1
    );
}

#[test]
fn an_agent_that_advertises_usage_may_send_its_notices() {
    let splice = splice(None);
    let theirs = json!({"protocolVersion": 1, "agentCapabilities": {"_meta": {USAGE_CAPABILITY: {"version": 1}}}});
    initialize(&splice, true, theirs);
    assert_eq!(from_agent(&splice, &usage_notice()).len(), 1);
}

#[test]
fn a_usage_request_before_the_initialize_answer_goes_to_the_agent() {
    let splice = splice(None);
    let capabilities = json!({"_meta": {USAGE_CAPABILITY: {"notify": false}}});
    assert!(
        read_client(
            &splice,
            &json!({"id": 0, "method": "initialize",
        "params": {"protocolVersion": 1, "clientCapabilities": capabilities}})
        )
        .is_none()
    );
    assert!(
        read_client(
            &splice,
            &json!({"id": 1, "method": USAGE_METHOD, "params": {}})
        )
        .is_none()
    );
}

#[test]
fn an_initialize_that_failed_leaves_the_host_out_of_it() {
    for answer in [
        json!({"id": 0, "error": {"code": -32603, "message": "no"}}),
        json!({"id": 0}),
    ] {
        let splice = splice(None);
        let capabilities = json!({"_meta": {USAGE_CAPABILITY: {"notify": true}}});
        assert!(
            read_client(
                &splice,
                &json!({"id": 0, "method": "initialize",
            "params": {"protocolVersion": 1, "clientCapabilities": capabilities}})
            )
            .is_none()
        );
        assert_eq!(from_agent(&splice, &answer).len(), 1);
        assert!(lock(&splice.conn).initializes.is_empty(), "{answer}");
        assert!(
            read_client(
                &splice,
                &json!({"id": 1, "method": USAGE_METHOD, "params": {}})
            )
            .is_none()
        );
        prompt(&splice, 2, "s1");
        assert!(lock(&splice.conn).prompts.is_empty());
    }
}

#[test]
fn an_initialize_answer_with_a_nested_method_key_is_still_read() {
    let splice = splice(None);
    let mut answer = plain_answer();
    answer["agentInfo"] = json!({"name": "x", "method": "y"});
    let sent = initialize(&splice, false, answer);
    assert_eq!(
        sent["result"]["agentCapabilities"]["_meta"][USAGE_CAPABILITY]["sources"],
        json!(["inferred"]),
        "{sent}"
    );
}

#[test]
fn words_said_in_a_session_with_no_open_turn_are_not_kept() {
    let splice = splice(None);
    initialize(&splice, false, plain_answer());
    let chunk = |session: &str, text: &str| {
        json!({"method": "session/update", "params": {"sessionId": session,
            "update": {"sessionUpdate": "agent_message_chunk", "content": {"type": "text", "text": text}}}})
    };
    prompt(&splice, 1, "b");
    // A replay of session a during b's turn.
    from_agent(&splice, &chunk("a", "You have hit your usage limit."));
    from_agent(
        &splice,
        &json!({"id": 1, "result": {"stopReason": "end_turn"}}),
    );
    prompt(&splice, 2, "a");
    from_agent(
        &splice,
        &json!({"id": 2, "result": {"stopReason": "refusal"}}),
    );
    assert_eq!(report_of(&splice)["buckets"], json!([]));
}

fn quota_entry(state_dir: Option<std::path::PathBuf>, private: bool) -> Quota {
    Quota::new(Antigravity {
        program: "canoa-test-no-such-agy".into(),
        args: Vec::new(),
        state_dir,
        private,
        per_connection: false,
        starter: Some(plain_starter()),
    })
}

/// A starter that records nothing: the program is started as it is.
fn plain_starter() -> Starter {
    Starter(Arc::new(|program, args| {
        let agent = Arc::new(coqui_acp_bridge::agent::Agent::spawn(
            program,
            args,
            crate::serve::MAX_LINE,
        )?);
        Ok(Some((agent, Box::new(|| {}))))
    }))
}

#[test]
fn an_entry_that_reads_a_quota_advertises_reported_and_inferred_and_needs_a_session() {
    let inference = Arc::new(Inference::new(None).with_quota(quota_entry(None, false)));
    let splice = Splice::new(inference, Some("work".to_owned()));
    let answered = initialize(&splice, false, plain_answer());
    let capability = &answered["result"]["agentCapabilities"]["_meta"][USAGE_CAPABILITY];
    assert_eq!(
        capability["sources"],
        json!(["reported", "inferred"]),
        "{answered}"
    );
    assert_eq!(capability["needs_session"], true, "{answered}");
}

#[test]
fn read_buckets_and_an_inferred_hit_are_one_report_with_a_revision_that_only_grows() {
    let quota = quota_entry(None, false);
    let inference = Inference::new(None).with_quota(quota);
    let before = inference.report(1_000, Some("work"));
    assert_eq!(before.buckets.len(), 0);
    assert_eq!(before.sources, [Source::Reported, Source::Inferred]);
    let quota = inference.quota.as_ref().expect("a quota");
    quota.apply(vec![Bucket::new("agy:model-a")]);
    let read = inference.report(1_000, Some("work"));
    assert_eq!(read.buckets.len(), 1);
    assert!(read.revision > before.revision);
    assert!(inference.observe(AGY, 1_000));
    let both = inference.report(1_000, Some("work"));
    let ids: Vec<&str> = both
        .buckets
        .iter()
        .map(|bucket| bucket.id.as_str())
        .collect();
    assert_eq!(ids, ["agy:model-a", "inferred"]);
    assert!(both.limit.is_some());
    assert!(both.revision > read.revision);
    assert_eq!(both.account.as_deref(), Some("work"));
    // A new reading replaces the buckets it read and leaves the inferred one.
    quota.apply(vec![Bucket::new("agy:model-b")]);
    let replaced = inference.report(1_000, Some("work"));
    let ids: Vec<&str> = replaced
        .buckets
        .iter()
        .map(|bucket| bucket.id.as_str())
        .collect();
    assert_eq!(ids, ["agy:model-b", "inferred"]);
    assert_eq!(
        inference.revision(1_000, Some(quota.as_ref())),
        replaced.revision
    );
}

#[test]
fn a_private_agent_is_started_only_where_allowed_one_at_a_time_and_not_often() {
    let state = std::env::temp_dir().join(format!("canoa-private-test-{}", std::process::id()));
    let done = || {};
    assert!(!Arc::new(quota_entry(None, true)).read_privately(done));
    assert!(!Arc::new(quota_entry(Some(state.clone()), false)).read_privately(done));
    let quota = Arc::new(quota_entry(Some(state.clone()), true));
    let (ended, wait) = std::sync::mpsc::channel();
    assert!(quota.read_privately(move || {
        let _ = ended.send(());
    }));
    // The program does not exist, so the attempt ends at once; and a second is refused for being too soon either way.
    wait.recv_timeout(PATIENCE)
        .expect("the attempt ends and says so");
    assert!(!quota.read_privately(done));
    let _ = std::fs::remove_dir_all(&state);
}

/// Serves the recorded `GetUserStatus` answer on a loopback port of this test process, so a reading finds it among the
/// listeners of the process it is told is the agent.
fn serve_status() {
    use std::io::{Read as _, Write as _};
    let body = std::fs::read(concat!(
        env!("CARGO_MANIFEST_DIR"),
        "/tests/fixtures/agy/getuserstatus.json"
    ))
    .expect("the fixture");
    let listener = std::net::TcpListener::bind("127.0.0.1:0").expect("a port");
    std::thread::spawn(move || {
        for mut stream in listener.incoming().flatten() {
            let body = body.clone();
            std::thread::spawn(move || {
                // The answer goes once the request's head is in; what follows it is drained, or closing the socket would
                // reset it under the reader.
                let mut request = Vec::new();
                let mut byte = [0_u8; 1];
                while !request.ends_with(b"\r\n\r\n")
                    && stream.read(&mut byte).is_ok_and(|read| read == 1)
                {
                    request.push(byte[0]);
                }
                let head = format!(
                    "HTTP/1.1 200 OK\r\nContent-Type: application/json\r\nContent-Length: {}\r\nConnection: close\r\n\r\n",
                    body.len()
                );
                let _ = stream.write_all(head.as_bytes());
                let _ = stream.write_all(&body);
                let _ = stream.set_read_timeout(Some(Duration::from_millis(200)));
                let mut rest = [0_u8; 256];
                while stream.read(&mut rest).is_ok_and(|read| read > 0) {}
            });
        }
    });
}

#[test]
fn a_session_opening_reads_the_quota_and_the_client_that_asked_is_told() {
    serve_status();
    let inference = Arc::new(Inference::new(None).with_quota(quota_entry(None, false)));
    let splice =
        Splice::new(Arc::clone(&inference), Some("work".to_owned())).agent(std::process::id());
    initialize(&splice, true, plain_answer());
    assert_eq!(report_of(&splice)["buckets"], json!([]));
    assert!(
        read_client(
            &splice,
            &json!({"id": 5, "method": "session/new", "params": {"cwd": "/work"}})
        )
        .is_none()
    );
    from_agent(
        &splice,
        &json!({"id": 5, "result": {"sessionId": "s", "models": {"availableModels": [
            {"modelId": "gemini-4-argon-medium", "name": "Gemini 4 Argon (Medium)"}]}}}),
    );
    let told = RefCell::new(Vec::<Value>::new());
    let until = std::time::Instant::now() + PATIENCE;
    while !told.borrow().iter().any(|message| {
        message["params"]["report"]["buckets"]
            .as_array()
            .is_some_and(|buckets| !buckets.is_empty())
    }) && std::time::Instant::now() < until
    {
        splice.tick(|message| {
            told.borrow_mut()
                .push(serde_json::from_slice(message).expect("JSON"));
            Ok(())
        });
        std::thread::sleep(Duration::from_millis(50));
    }
    let told = told.into_inner();
    let notice = told
        .iter()
        .find(|message| message["method"] == USAGE_NOTIFICATION)
        .expect("a notice");
    let report = &notice["params"]["report"];
    assert_eq!(report["account"], "work", "{report}");
    assert!(
        report["buckets"].as_array().is_some_and(|b| b.len() >= 2),
        "{report}"
    );
    assert!(!report.to_string().contains("<EMAIL>"));
    // The reading is current, so a request is answered at once from it.
    let asked = report_of(&splice);
    assert_eq!(asked["buckets"], report["buckets"]);
}

fn per_connection_entry() -> Arc<Inference> {
    Arc::new(Inference::new(None).with_antigravity(Antigravity {
        program: "canoa-test-no-such-agy".into(),
        args: Vec::new(),
        state_dir: None,
        private: false,
        per_connection: true,
        starter: None,
    }))
}

#[test]
fn connections_of_an_entry_with_profiles_never_answer_from_each_others_readings() {
    serve_status();
    let inference = per_connection_entry();
    let first =
        Splice::new(Arc::clone(&inference), Some("work".to_owned())).agent(std::process::id());
    let second = Splice::new(Arc::clone(&inference), Some("work".to_owned()));
    initialize(&first, true, plain_answer());
    initialize(&second, true, plain_answer());
    read_client(
        &first,
        &json!({"id": 5, "method": "session/new", "params": {"cwd": "/work"}}),
    );
    from_agent(&first, &json!({"id": 5, "result": {"sessionId": "s"}}));
    let until = std::time::Instant::now() + PATIENCE;
    let told = std::cell::Cell::new(false);
    while !told.get() && std::time::Instant::now() < until {
        first.tick(|message| {
            let notice: Value = serde_json::from_slice(message).expect("JSON");
            told.set(told.get() || notice["params"]["report"]["buckets"] != json!([]));
            Ok(())
        });
        std::thread::sleep(Duration::from_millis(100));
    }
    assert!(
        report_of(&first)["buckets"]
            .as_array()
            .is_some_and(|buckets| buckets.len() >= 2)
    );
    // The first connection's reading is current, and says nothing to the second.
    assert_eq!(report_of(&second)["buckets"], json!([]));
    assert_eq!(report_of(&second)["revision"], 0);
}

#[test]
fn notices_from_two_threads_go_in_order_and_the_told_revision_only_grows() {
    let inference = Arc::new(Inference::new(None));
    let splice = Arc::new(Splice::new(Arc::clone(&inference), Some("work".to_owned())));
    initialize(&splice, true, plain_answer());
    let sent = Arc::new(Mutex::new(Vec::<u64>::new()));
    let done = Arc::new(core::sync::atomic::AtomicBool::new(false));
    let moving = {
        let (inference, sent) = (Arc::clone(&inference), Arc::clone(&sent));
        std::thread::spawn(move || {
            for at in 0..200_i64 {
                // A new retry-after each time is a change, which the report numbers.
                let text = format!("Error 429 Retry-After: {}", 1_000 + at);
                inference.observe(&text, 1_000);
                if at == 0 {
                    // The readers have told the first revision before the rest move, whatever the machine's load.
                    let until = std::time::Instant::now() + PATIENCE;
                    while lock(&sent).is_empty() && std::time::Instant::now() < until {
                        std::thread::sleep(Duration::from_millis(1));
                    }
                }
                std::thread::sleep(Duration::from_micros(200));
            }
        })
    };
    let readers: Vec<_> = (0..4)
        .map(|_| {
            let (splice, sent, done) = (Arc::clone(&splice), Arc::clone(&sent), Arc::clone(&done));
            std::thread::spawn(move || {
                // One tick more after the changes ended, so the last revision is told.
                let mut last = false;
                while !last {
                    last = done.load(core::sync::atomic::Ordering::SeqCst);
                    splice.tick(|message| {
                        let notice: Value = serde_json::from_slice(message).expect("JSON");
                        let revision = notice["params"]["report"]["revision"]
                            .as_u64()
                            .expect("a revision");
                        // A send that takes a moment is where an older notice would overtake a newer one.
                        std::thread::sleep(Duration::from_micros(300));
                        lock(&sent).push(revision);
                        Ok(())
                    });
                }
            })
        })
        .collect();
    moving.join().expect("the changes end");
    done.store(true, core::sync::atomic::Ordering::SeqCst);
    for reader in readers {
        reader.join().expect("a reader ends");
    }
    let sent = lock(&sent);
    assert!(sent.len() > 1, "{sent:?}");
    assert!(
        sent.windows(2).all(|pair| pair[0] < pair[1]),
        "notices are in order and none repeats: {sent:?}"
    );
}

#[test]
fn a_panic_in_a_reading_frees_the_connection_and_answers_the_request_that_waited() {
    let inference = Arc::new(Inference::new(None).with_quota(quota_entry(None, false)));
    let quota = inference.quota.clone().expect("a quota");
    let link = Arc::new(Link::default());
    let reading = Arc::new(Reading::new(
        Arc::clone(&inference),
        quota,
        Arc::clone(&link),
        Some("work".to_owned()),
        std::process::id(),
    ));
    reading.wait_for(json!(7));
    assert!(reading.spawn(|_| panic!("a reading that goes wrong")));
    let until = std::time::Instant::now() + PATIENCE;
    while link.take().is_empty() && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    // Free again: another job starts.
    let (ran, done) = std::sync::mpsc::channel();
    let until = std::time::Instant::now() + PATIENCE;
    let mut tries = 0;
    while done.try_recv().is_err() && std::time::Instant::now() < until {
        let ran = ran.clone();
        if reading.spawn(move |_| {
            let _ = ran.send(());
        }) {
            tries += 1;
        }
        std::thread::sleep(Duration::from_millis(50));
    }
    assert!(tries >= 1, "the connection took no further reading");
}

#[test]
fn a_panic_in_a_private_reading_still_answers_who_asked() {
    let state = std::env::temp_dir().join(format!("canoa-panic-test-{}", std::process::id()));
    let quota = Arc::new(Quota::new(Antigravity {
        program: "x".into(),
        args: Vec::new(),
        state_dir: Some(state.clone()),
        private: true,
        per_connection: false,
        starter: Some(Starter(Arc::new(|_, _| panic!("the host's starter broke")))),
    }));
    let (ended, wait) = std::sync::mpsc::channel();
    assert!(quota.read_privately(move || {
        let _ = ended.send(());
    }));
    wait.recv_timeout(PATIENCE)
        .expect("the request is answered although the reading panicked");
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn a_request_that_comes_while_a_private_agent_reads_waits_for_it() {
    let state = std::env::temp_dir().join(format!("canoa-wait-test-{}", std::process::id()));
    let (release, gate) = std::sync::mpsc::channel::<()>();
    let gate = Mutex::new(gate);
    let quota = Arc::new(Quota::new(Antigravity {
        program: "x".into(),
        args: Vec::new(),
        state_dir: Some(state.clone()),
        private: true,
        per_connection: false,
        starter: Some(Starter(Arc::new(move |_, _| {
            let _ = gate.lock().expect("gate").recv();
            Err(std::io::Error::other("no agent here"))
        }))),
    }));
    let answered = Arc::new(Mutex::new(Vec::<&'static str>::new()));
    let note = |who: &'static str| {
        let answered = Arc::clone(&answered);
        move || lock(&answered).push(who)
    };
    assert!(quota.read_privately(note("first")));
    // The second comes while the first reads, and the reading is not current: it is held, not given the old report.
    std::thread::sleep(Duration::from_millis(200));
    assert!(quota.read_privately(note("second")));
    assert!(lock(&answered).is_empty());
    release.send(()).expect("released");
    let until = std::time::Instant::now() + PATIENCE;
    while lock(&answered).len() < 2 && std::time::Instant::now() < until {
        std::thread::sleep(Duration::from_millis(20));
    }
    assert_eq!(lock(&answered).len(), 2);
    let _ = std::fs::remove_dir_all(&state);
}

#[test]
fn a_failed_turn_is_in_the_report_before_its_answer_reaches_the_client() {
    let splice = splice(None);
    initialize(&splice, false, plain_answer());
    prompt(&splice, 7, "s");
    // The client asks at once, as soon as it has the answer: the send is the moment it has it.
    let asked = RefCell::new(None);
    splice
        .relay(&bytes(&failed(7, AGY)), |_| {
            *asked.borrow_mut() = Some(report_of(&splice));
            Ok(())
        })
        .expect("sent");
    let report = asked.into_inner().expect("the answer was sent");
    assert_eq!(report["buckets"][0]["id"], "inferred", "{report}");
}
