//! `looks/list`: the looks the vendor crates carry, as the token documents the app draws its families from.

use serde_json::{Value, json};

use super::REFUSED;

/// Every built-in vendor's look document, in the app's order.
const DOCUMENTS: [(&str, &str); 2] = [
    ("claude", canoa_claude::LOOK),
    ("chatgpt", canoa_codex::LOOK),
];

/// `{"looks": [document, ...]}`: each document has `id`, `label`, `vendor` and, per brightness (`light`, `dark`), the colour
/// tokens, `radius`, `controlRadius`, `railWidth` and a `chat` table. Nothing in them is secret.
pub(super) fn list() -> Result<Value, (i32, String)> {
    let mut looks = Vec::with_capacity(DOCUMENTS.len());
    for (id, text) in DOCUMENTS {
        let document: Value = serde_json::from_str(text)
            .ok()
            .filter(|document: &Value| document.get("id").and_then(Value::as_str) == Some(id))
            .ok_or_else(|| {
                (
                    REFUSED,
                    format!("the {id} look of this build is not a look document"),
                )
            })?;
        looks.push(document);
    }
    Ok(json!({ "looks": looks }))
}
