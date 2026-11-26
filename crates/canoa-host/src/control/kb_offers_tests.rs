use alloc::sync::Arc;

use serde_json::json;

use super::kb_offers::{call, offers, test};
use super::{Caller, DENIED, Grant, REFUSED};
use crate::assist::testing::StandIn;
use crate::assist::{Assist, Backend, Operation};
use crate::config::Config;

fn config(text: &str) -> Config {
    Config::from_str_in(text, std::path::Path::new("")).expect("parses")
}

fn service(backends: Vec<Arc<StandIn>>, config: &Config) -> Arc<Assist> {
    Assist::new(
        backends
            .into_iter()
            .map(|each| each as Arc<dyn Backend>)
            .collect(),
        config.assist.operations.clone(),
        core::time::Duration::from_secs(60),
    )
}

#[test]
fn each_operation_lists_what_can_do_it_in_the_order_canoa_cfg_gives() {
    let config = config(
        "[assist.embed]\nuse = \"second\"\nfallback = [\"first\"]\n[assist_harness.mine]\nsource = \"Claude\"\n",
    );
    let first = StandIn::new("first", &[Operation::Embed, Operation::Complete]);
    let second = StandIn::new("second", &[Operation::Embed]);
    let assist = service(vec![first, second], &config);
    let answer = offers(&assist, &config, &json!({})).expect("offers");
    let operations = answer["operations"].as_array().expect("operations");
    let embed = operations
        .iter()
        .find(|each| each["id"] == "embed")
        .expect("embed");
    assert_eq!(embed["use"], "second");
    assert_eq!(embed["fallback"], json!(["first"]));
    assert_eq!(embed["asked"], json!(["second", "first"]));
    assert_eq!(embed["capability"], "embed");
    let ids: Vec<_> = embed["offers"]
        .as_array()
        .expect("offers")
        .iter()
        .map(|offer| offer["id"].as_str().expect("id"))
        .collect();
    assert_eq!(ids, ["first", "second"]);
    // A backend that cannot rank is not offered it, and the operation says why nothing can.
    let rerank = operations
        .iter()
        .find(|each| each["id"] == "rerank")
        .expect("rerank");
    assert_eq!(rerank["offers"], json!([]));
    assert_eq!(rerank["asked"], json!([]));
    assert!(
        rerank["problem"]
            .as_str()
            .is_some_and(|why| why.contains("no backend")),
        "{rerank}"
    );
    // What the person can make of the agents their canoa.cfg has, and the pool as it stands.
    assert_eq!(answer["pool"]["idle_seconds"], 120);
    assert_eq!(answer["pool"]["max_sessions"], 2);
    assert!(answer["harnesses"].is_array());
    // No model was listed for anyone: that is a request to a service.
    assert!(embed["offers"][0].get("models").is_none());
}

#[test]
fn the_models_of_the_offer_named_are_listed_and_split_by_what_they_report() {
    let config = config("");
    let one = StandIn::new("one", &[Operation::Embed]);
    let assist = service(vec![one], &config);
    let answer = offers(&assist, &config, &json!({"offer": "one"})).expect("offers");
    let embed = answer["operations"]
        .as_array()
        .expect("operations")
        .iter()
        .find(|each| each["id"] == "embed")
        .expect("embed");
    let models = &embed["offers"][0]["models"];
    assert_eq!(models["listed"], true);
    // The stand-in's one model reports nothing: it is apart from the models that report `embed`, not hidden.
    assert_eq!(models["offered"], json!([]));
    assert_eq!(models["unreported"][0]["id"], "stand-in");
}

#[test]
fn an_embedder_is_tried_with_a_sample_and_the_answer_says_what_came_back() {
    let one = StandIn::new("one", &[Operation::Embed]);
    let assist = service(vec![one], &config(""));
    let answer = test(
        &assist,
        &json!({"role": "embedder", "use": "one", "model": "m"}),
    )
    .expect("tried");
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["backend"], "one");
    assert_eq!(answer["vectors"], 2);
    assert_eq!(answer["length"], 4);
    let failing = StandIn::new("down", &[Operation::Embed]);
    failing
        .failing
        .store(true, core::sync::atomic::Ordering::SeqCst);
    let assist = service(vec![failing], &config(""));
    let answer = test(
        &assist,
        &json!({"role": "embedder", "use": "down", "model": "m"}),
    )
    .expect("tried");
    assert_eq!(answer["ok"], false);
    assert!(
        answer["why"]
            .as_str()
            .is_some_and(|why| why.contains("down"))
    );
}

#[test]
fn a_ranker_is_tried_and_says_whether_the_best_passage_came_first() {
    let one = StandIn::new("one", &[Operation::Rerank]);
    let assist = service(vec![one], &config(""));
    let answer = test(
        &assist,
        &json!({"role": "ranker", "use": "one", "model": "m"}),
    )
    .expect("tried");
    assert_eq!(answer["ok"], true, "{answer}");
    assert_eq!(answer["best_first"], true);
    assert_eq!(answer["native"], true);
    // A chat model that answers no number is no ranker, and the answer says so.
    let chat = StandIn::new("chat", &[Operation::Complete]);
    *chat.says.lock().expect("not poisoned") = "I cannot say".to_owned();
    let assist = service(vec![chat], &config(""));
    let answer = test(
        &assist,
        &json!({"role": "ranker", "use": "chat", "model": "m"}),
    )
    .expect("tried");
    assert_eq!(answer["ok"], false, "{answer}");
}

#[test]
fn a_test_needs_a_role_and_a_model() {
    let assist = service(vec![], &config(""));
    assert!(test(&assist, &json!({"model": "m"})).is_err());
    assert!(test(&assist, &json!({"role": "embedder"})).is_err());
    assert!(test(&assist, &json!({"role": "nothing", "model": "m"})).is_err());
}

#[test]
fn only_a_key_with_the_accounts_grant_asks_and_a_host_that_keeps_none_refuses() {
    let key = Caller::Peer {
        id: "sha256:aa".to_owned(),
        grants: Default::default(),
    };
    assert_eq!(
        call(None, None, &key, "kb/offers", &json!({}))
            .expect_err("denied")
            .0,
        DENIED
    );
    let holding = Caller::Peer {
        id: "sha256:bb".to_owned(),
        grants: [Grant::Accounts].into_iter().collect(),
    };
    assert_eq!(
        call(None, None, &holding, "kb/test", &json!({}))
            .expect_err("no accounts")
            .0,
        REFUSED
    );
}
