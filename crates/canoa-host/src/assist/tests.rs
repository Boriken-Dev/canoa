use alloc::sync::Arc;
use core::sync::atomic::Ordering;
use core::time::Duration;

use canoa::network::Network;
use canoa::sdk::{ChatRequest, EmbedRequest, RerankRequest};
use indexmap::IndexMap;

use super::testing::StandIn;
use super::{Assist, Backend, Operation};
use crate::config::AssistChoice;

const LONG: Duration = Duration::from_secs(5);

fn network() -> Arc<Network> {
    Network::current()
}

fn assist(backends: Vec<Arc<StandIn>>, choices: &[(&str, Option<&str>, &[&str])]) -> Arc<Assist> {
    let mut table = IndexMap::new();
    for (operation, choice, fallback) in choices {
        let each = AssistChoice {
            choice: choice.map(str::to_owned),
            fallback: fallback.iter().map(|id| (*id).to_owned()).collect(),
        };
        table.insert((*operation).to_owned(), each);
    }
    Assist::new(
        backends
            .into_iter()
            .map(|each| each as Arc<dyn Backend>)
            .collect(),
        table,
        Duration::from_secs(60),
    )
}

fn embed(assist: &Assist, choice: &str) -> Result<String, String> {
    assist
        .embed(
            &network(),
            choice,
            EmbedRequest::new("m", vec!["boat".to_owned()]),
            LONG,
        )
        .map(|done| done.backend)
}

#[test]
fn with_no_choice_the_first_backend_that_can_serves() {
    let chat_only = StandIn::new("chat", &[Operation::Complete]);
    let both = StandIn::new("both", &[Operation::Complete, Operation::Embed]);
    let service = assist(vec![chat_only, both], &[]);
    assert_eq!(embed(&service, "default").as_deref(), Ok("both"));
}

#[test]
fn the_persons_choice_is_asked_first_and_its_fallback_after() {
    let first = StandIn::new("first", &[Operation::Embed]);
    let second = StandIn::new("second", &[Operation::Embed]);
    let third = StandIn::new("third", &[Operation::Embed]);
    let service = assist(
        vec![Arc::clone(&first), Arc::clone(&second), third],
        &[("embed", Some("second"), &["first"])],
    );
    assert_eq!(embed(&service, "default").as_deref(), Ok("second"));
    second.failing.store(true, Ordering::SeqCst);
    assert_eq!(embed(&service, "default").as_deref(), Ok("first"));
    // `third` is not in the person's list, so it is never asked, even when both fail.
    first.failing.store(true, Ordering::SeqCst);
    let why = embed(&service, "default").expect_err("both failed");
    assert!(
        why.contains("second") && why.contains("first") && !why.contains("third"),
        "{why}"
    );
}

#[test]
fn a_named_backend_is_asked_alone_and_a_missing_or_unable_one_says_why() {
    let embedder = StandIn::new("e", &[Operation::Embed]);
    let service = assist(vec![embedder], &[]);
    assert_eq!(embed(&service, "e").as_deref(), Ok("e"));
    assert!(
        embed(&service, "nobody")
            .expect_err("missing")
            .contains("there is no backend nobody")
    );
    let why = service
        .complete(&network(), "e", ChatRequest::ask("m", "hi"), LONG)
        .expect_err("cannot");
    assert!(why.contains("e cannot complete"), "{why}");
    let none = Assist::none();
    let why = embed(&none, "default").expect_err("no backend");
    assert!(why.contains("no backend is set up to embed"), "{why}");
}

#[test]
fn a_backend_that_failed_is_left_alone_for_a_while_and_asked_again_after() {
    let broken = StandIn::new("broken", &[Operation::Embed]);
    broken.failing.store(true, Ordering::SeqCst);
    let service = Assist::new(
        vec![Arc::clone(&broken) as Arc<dyn Backend>],
        IndexMap::new(),
        Duration::from_secs(3600),
    );
    assert!(embed(&service, "default").is_err());
    assert!(
        embed(&service, "default")
            .expect_err("left alone")
            .contains("left alone")
    );
    assert_eq!(
        broken.calls.load(Ordering::SeqCst),
        1,
        "a dead backend costs one request"
    );
    let service = Assist::new(
        vec![Arc::clone(&broken) as Arc<dyn Backend>],
        IndexMap::new(),
        Duration::ZERO,
    );
    assert!(embed(&service, "default").is_err());
    assert!(embed(&service, "default").is_err());
    assert_eq!(
        broken.calls.load(Ordering::SeqCst),
        3,
        "with no wait it is asked again"
    );
}

#[test]
fn every_operation_goes_to_the_backend_and_comes_back_as_its_own_kind_of_answer() {
    let all = StandIn::new(
        "all",
        &[Operation::Complete, Operation::Embed, Operation::Rerank],
    );
    *all.says.lock().unwrap() = "[2, 1]".to_owned();
    let service = assist(vec![all], &[]);
    let said = service
        .complete(&network(), "default", ChatRequest::ask("m", "hi"), LONG)
        .expect("answered");
    assert_eq!(said.value.text, "[2, 1]");
    let ranked = service
        .rerank(
            &network(),
            "default",
            RerankRequest::new("m", "boat", vec!["bread".to_owned(), "a canoe".to_owned()]),
            LONG,
        )
        .expect("answered");
    assert_eq!(
        ranked.value.results[0].index, 1,
        "the passage of the query's topic is first"
    );
    assert_eq!(
        service.first(Operation::Embed, "default").as_deref(),
        Some("all")
    );
    assert!(service.can(Operation::Rerank, "all"));
}
