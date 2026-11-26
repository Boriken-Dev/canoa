//! What the client is told when approvers, and not its window, settled a permission request.

use serde_json::{Value, json};

use super::chain::{Outcome, Settled};

/// The method that says who settled a request and how: the app closes the card and the conversation keeps the line.
pub(crate) const APPROVED: &str = "_dev.boriken.canoa/approved";

/// The method a shared session's hub uses when one client answered, which every other client reads as the request being closed.
pub(crate) const CLOSED: &str = "_dev.boriken.coqui.acp/hub_permission_closed";

fn word(outcome: Outcome) -> &'static str {
    match outcome {
        Outcome::Allowed => "allowed",
        Outcome::Rejected => "rejected",
        Outcome::Refused => "refused",
    }
}

/// `_dev.boriken.canoa/approved` for the session `agent`'s tool call, as bytes.
pub(crate) fn approved(agent: &str, call: Option<&Value>, settled: &Settled) -> Option<Vec<u8>> {
    let call = call
        .and_then(|call| call.get("toolCallId"))
        .and_then(Value::as_str);
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": APPROVED,
        "params": {
            "sessionId": agent,
            "toolCallId": call,
            "by": settled.by,
            "outcome": word(settled.outcome),
        },
    }))
    .ok()
}

/// The notice that the request `id` is closed, with the option the agent was answered with.
pub(crate) fn closed(
    agent: &str,
    id: &Value,
    settled: &Settled,
    outcome: &Value,
) -> Option<Vec<u8>> {
    serde_json::to_vec(&json!({
        "jsonrpc": "2.0",
        "method": CLOSED,
        "params": {
            "sessionId": agent,
            "requestId": id,
            "by": settled.by,
            "outcome": outcome,
        },
    }))
    .ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn the_client_is_told_who_settled_which_call_and_how() {
        let settled = Settled {
            by: "phone".to_owned(),
            outcome: Outcome::Refused,
        };
        let call = json!({"toolCallId": "c1"});
        let bytes = approved("s1", Some(&call), &settled).expect("bytes");
        let value: Value = serde_json::from_slice(&bytes).expect("JSON");
        assert_eq!(value["method"], APPROVED);
        assert_eq!(value["params"]["toolCallId"], "c1");
        assert_eq!(value["params"]["outcome"], "refused");
        assert_eq!(value["params"]["by"], "phone");
    }
}
