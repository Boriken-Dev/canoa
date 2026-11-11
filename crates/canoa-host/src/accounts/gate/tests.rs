//! The one-session rule, counted from what the provider answers.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "tests name what they use from std"
)]

use std::collections::VecDeque;
use std::sync::{Mutex, PoisonError};

use super::*;

/// A channel in memory: what the client sends waits in `inbox`, what is sent to the client is kept in `sent`.
#[derive(Default)]
struct Memory {
    inbox: Mutex<VecDeque<Vec<u8>>>,
    sent: Mutex<Vec<Value>>,
}

impl Channel for Memory {
    fn send(&self, message: &[u8]) -> Result<(), Failure> {
        self.sent
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .push(serde_json::from_slice(message).expect("JSON"));
        Ok(())
    }

    fn receive(&self, _timeout: Option<Duration>) -> Result<Option<Vec<u8>>, Failure> {
        self.inbox
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .pop_front()
            .map_or(Err(Failure::Timeout), |message| Ok(Some(message)))
    }

    fn close(&self) {}
}

/// The client sends `request`; whether the gate let it through to the provider.
fn client_sends(gate: &Gated<'_, Memory>, memory: &Memory, request: &Value) -> bool {
    memory
        .inbox
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .push_back(request.to_string().into_bytes());
    gate.receive(None).is_ok()
}

fn answers(gate: &Gated<'_, Memory>, response: &Value) {
    gate.send(response.to_string().as_bytes()).expect("sent");
}

fn request(id: u64, method: &str, params: &Value) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "method": method, "params": params})
}

fn opened(id: u64, session: &str) -> Value {
    json!({"jsonrpc": "2.0", "id": id, "result": {"sessionId": session}})
}

fn refused_by_the_gate(memory: &Memory, id: u64) -> bool {
    memory
        .sent
        .lock()
        .unwrap_or_else(PoisonError::into_inner)
        .iter()
        .any(|message| message["id"] == id && message["error"]["code"] == -32602)
}

#[test]
fn a_session_the_provider_refused_to_open_does_not_count() {
    let memory = Memory::default();
    let gate = Gated::new(&memory, None, true);
    assert!(client_sends(
        &gate,
        &memory,
        &request(1, "session/new", &json!({}))
    ));
    answers(
        &gate,
        &json!({"jsonrpc": "2.0", "id": 1, "error": {"code": -32603, "message": "no"}}),
    );
    assert!(
        client_sends(&gate, &memory, &request(2, "session/new", &json!({}))),
        "a failed open was counted"
    );
}

#[test]
fn closing_a_session_the_connection_does_not_hold_frees_nothing() {
    let memory = Memory::default();
    let gate = Gated::new(&memory, None, true);
    assert!(client_sends(
        &gate,
        &memory,
        &request(1, "session/new", &json!({}))
    ));
    answers(&gate, &opened(1, "s1"));
    assert!(client_sends(
        &gate,
        &memory,
        &request(2, "session/close", &json!({"sessionId": "other"}))
    ));
    answers(&gate, &json!({"jsonrpc": "2.0", "id": 2, "result": {}}));
    assert!(!client_sends(
        &gate,
        &memory,
        &request(3, "session/new", &json!({}))
    ));
    assert!(refused_by_the_gate(&memory, 3));
}

#[test]
fn a_fork_is_a_second_session() {
    let memory = Memory::default();
    let gate = Gated::new(&memory, None, true);
    assert!(client_sends(
        &gate,
        &memory,
        &request(1, "session/new", &json!({}))
    ));
    answers(&gate, &opened(1, "s1"));
    assert!(!client_sends(
        &gate,
        &memory,
        &request(2, "session/fork", &json!({"sessionId": "s1"}))
    ));
    assert!(refused_by_the_gate(&memory, 2));
}

#[test]
fn loading_or_resuming_a_second_session_is_refused_and_closing_the_first_allows_one() {
    let memory = Memory::default();
    let gate = Gated::new(&memory, None, true);
    assert!(client_sends(
        &gate,
        &memory,
        &request(1, "session/load", &json!({"sessionId": "s1"}))
    ));
    answers(&gate, &json!({"jsonrpc": "2.0", "id": 1, "result": {}}));
    assert!(!client_sends(
        &gate,
        &memory,
        &request(2, "session/resume", &json!({"sessionId": "s2"}))
    ));
    assert!(client_sends(
        &gate,
        &memory,
        &request(3, "session/close", &json!({"sessionId": "s1"}))
    ));
    answers(&gate, &json!({"jsonrpc": "2.0", "id": 3, "result": {}}));
    assert!(client_sends(
        &gate,
        &memory,
        &request(4, "session/resume", &json!({"sessionId": "s2"}))
    ));
}

#[test]
fn a_second_open_sent_before_the_first_is_answered_is_refused() {
    let memory = Memory::default();
    let gate = Gated::new(&memory, None, true);
    assert!(client_sends(
        &gate,
        &memory,
        &request(1, "session/new", &json!({}))
    ));
    assert!(!client_sends(
        &gate,
        &memory,
        &request(2, "session/new", &json!({}))
    ));
    answers(&gate, &opened(1, "s1"));
}

#[test]
fn an_account_that_does_not_rotate_has_no_limit() {
    let memory = Memory::default();
    let gate = Gated::new(&memory, None, false);
    for id in 1..4 {
        assert!(client_sends(
            &gate,
            &memory,
            &request(id, "session/new", &json!({}))
        ));
        answers(&gate, &opened(id, &format!("s{id}")));
    }
}
