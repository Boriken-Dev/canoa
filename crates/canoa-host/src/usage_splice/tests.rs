#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::cell::RefCell;
use std::sync::Arc;

use super::*;

const AGY: &str = "Agent execution terminated due to error.\nError: HTTP 429 Too Many Requests\n{ \"error\": { \"code\": 429, \"message\": \"Resource has been exhausted (e.g. check quota).\", \"status\": \"RESOURCE_EXHAUSTED\" } }";
const GATEWAY: &str =
    "Rate limit exceeded for api_key: tokens. Limit reset at: 2099-01-01T00:00:00Z";
const PROVIDER: &str =
    "litellm.RateLimitError: RateLimitError: OpenAIException - Rate limit reached for gpt";
const CAPACITY: &str = "UNAVAILABLE (code 503): No capacity available for model gemini";
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
    splice
        .read_client(&bytes(message))
        .map(|answer| serde_json::from_slice(&answer).expect("JSON"))
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

fn report_of(splice: &Splice) -> Value {
    read_client(
        splice,
        &json!({"id": 99, "method": USAGE_METHOD, "params": {}}),
    )
    .expect("the host answers")["result"]
        .clone()
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
