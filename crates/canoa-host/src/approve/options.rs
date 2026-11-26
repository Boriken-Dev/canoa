//! How a verdict becomes the option an agent offered. A hook's answer and an approver's share it: a reject takes the option that
//! refuses once, else the one that refuses always, else cancels; an allow takes only the option that allows once, and only when its
//! id is not shared with another option, never one that remembers the choice.

use serde_json::{Value, json};

/// The id of the first option of `kind`.
fn option_of(options: Option<&[Value]>, kind: &str) -> Option<Value> {
    options
        .into_iter()
        .flatten()
        .find(|option| option.get("kind").and_then(Value::as_str) == Some(kind))
        .and_then(|option| option.get("optionId").cloned())
}

/// The outcome that refuses the call.
pub(crate) fn reject(options: Option<&[Value]>) -> Value {
    match option_of(options, "reject_once").or_else(|| option_of(options, "reject_always")) {
        Some(id) => json!({"outcome": "selected", "optionId": id}),
        None => json!({"outcome": "cancelled"}),
    }
}

/// The outcome that allows the call once, or none when the agent offers no option that allows it once whose id no other option
/// shares (an id another option shares could be read as either).
pub(crate) fn allow_once(options: Option<&[Value]>) -> Option<Value> {
    let id = option_of(options, "allow_once")?;
    let single = options
        .into_iter()
        .flatten()
        .filter(|each| each.get("optionId") == Some(&id))
        .count()
        == 1;
    single.then(|| json!({"outcome": "selected", "optionId": id}))
}

/// The JSON-RPC answer to the request `id` with `outcome`.
pub(crate) fn answer(id: &Value, outcome: &Value) -> Option<Vec<u8>> {
    serde_json::to_vec(&json!({"jsonrpc": "2.0", "id": id, "result": {"outcome": outcome}})).ok()
}

/// What the agent offered, as an approver is told: the kinds and names, never the ids.
pub(crate) fn refs(options: Option<&[Value]>) -> Vec<(String, String)> {
    options
        .into_iter()
        .flatten()
        .map(|option| {
            let text = |key: &str| {
                option
                    .get(key)
                    .and_then(Value::as_str)
                    .unwrap_or_default()
                    .to_owned()
            };
            (text("kind"), text("name"))
        })
        .collect()
}

#[cfg(test)]
mod tests {
    use super::*;

    fn offered() -> Vec<Value> {
        vec![
            json!({"optionId": "yes", "name": "Allow", "kind": "allow_once"}),
            json!({"optionId": "yes-always", "name": "Always", "kind": "allow_always"}),
            json!({"optionId": "no-always", "name": "Never", "kind": "reject_always"}),
        ]
    }

    #[test]
    fn an_allow_is_the_option_that_allows_once_and_never_the_one_that_remembers() {
        let options = offered();
        assert_eq!(
            allow_once(Some(&options)),
            Some(json!({"outcome": "selected", "optionId": "yes"}))
        );
        assert_eq!(allow_once(Some(&options[1..])), None);
    }

    #[test]
    fn an_allow_whose_id_another_option_shares_is_not_taken() {
        let options = vec![
            json!({"optionId": "x", "name": "Allow", "kind": "allow_once"}),
            json!({"optionId": "x", "name": "Reject", "kind": "reject_once"}),
        ];
        assert_eq!(allow_once(Some(&options)), None);
    }

    #[test]
    fn a_reject_takes_reject_once_then_reject_always_then_cancels() {
        let mut options = offered();
        assert_eq!(reject(Some(&options))["optionId"], "no-always");
        options.push(json!({"optionId": "no", "name": "No", "kind": "reject_once"}));
        assert_eq!(reject(Some(&options))["optionId"], "no");
        assert_eq!(reject(Some(&options[..2]))["outcome"], "cancelled");
        assert_eq!(reject(None)["outcome"], "cancelled");
    }
}
