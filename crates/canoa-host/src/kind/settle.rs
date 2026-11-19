//! What a connection's start is checked against before anything starts: the model the client named in `initialize`, against what
//! the account it will run on lists.

use alloc::string::String;
use alloc::vec::Vec;

use super::{Harness, MODEL_META};

/// The model a client's `initialize` names under [`MODEL_META`]; `None` when it names none.
///
/// # Errors
///
/// A model that is not text, or is empty.
fn named_model(first: &[u8]) -> Result<Option<String>, String> {
    let named = serde_json::from_slice::<serde_json::Value>(first)
        .ok()
        .and_then(|message| {
            message
                .get("params")?
                .get("_meta")?
                .get(MODEL_META)
                .cloned()
        });
    match named {
        None => Ok(None),
        Some(serde_json::Value::String(model)) if !model.trim().is_empty() => Ok(Some(model)),
        Some(_) => Err("the model is named by its id, as text".to_owned()),
    }
}

impl Harness {
    /// Checks the start `first` asks for before the account is held for it: a model it names must be one that the account the
    /// start runs on lists, and the refusal names both. A provider that cannot list its models before a session leaves the
    /// model to the session's own options. Returns the message to forward.
    ///
    /// # Errors
    ///
    /// Why the start is refused, for the client.
    pub(crate) fn settle(&self, first: &[u8], any: bool) -> Result<Vec<u8>, String> {
        let Some(model) = named_model(first)? else {
            return Ok(first.to_vec());
        };
        let picked = self.pick(first, any)?;
        let (who, listed) = match (
            picked,
            self.accounts
                .as_ref()
                .and_then(|offer| offer.config.as_ref()),
        ) {
            (Some((name, ..)), Some(config)) => (
                format!("account {name:?}"),
                crate::options::as_account(config, self, name, any),
            ),
            _ => (
                format!("the login {} has where it runs", self.id),
                crate::options::as_local(self),
            ),
        };
        match listed.refuses(&who, &model) {
            Some(why) => Err(why),
            None => Ok(first.to_vec()),
        }
    }
}
