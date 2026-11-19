//! What a connection's start is settled with before anything starts: the pool it names, resolved to the account it runs on, and the
//! model it names, checked against what that account lists.

use alloc::string::String;
use alloc::vec::Vec;

use super::{ACCOUNT_META, Harness, MODEL_META};
use crate::pools::EntryPool;

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

/// The account (or pool) a client's `initialize` names: `None` when it names none, `Some(None)` for `null`.
fn named_account(first: &[u8]) -> Option<Option<String>> {
    let named = serde_json::from_slice::<serde_json::Value>(first)
        .ok()?
        .get("params")?
        .get("_meta")?
        .get(ACCOUNT_META)
        .cloned()?;
    Some(named.as_str().map(str::to_owned))
}

/// What a start was settled to.
pub(crate) struct Settled {
    /// The message to go on with: the account it names is the one the start runs on, whatever it named.
    pub(crate) first: Vec<u8>,
    /// The pool the account was taken from, when the client named a pool.
    pub(crate) pool: Option<String>,
}

/// `first` with the account it names replaced by `account`.
fn naming(first: &[u8], account: &str) -> Vec<u8> {
    let Ok(mut message) = serde_json::from_slice::<serde_json::Value>(first) else {
        return first.to_vec();
    };
    if let Some(meta) = message
        .get_mut("params")
        .and_then(|params| params.get_mut("_meta"))
        .and_then(serde_json::Value::as_object_mut)
    {
        meta.insert(ACCOUNT_META.to_owned(), serde_json::Value::from(account));
    }
    serde_json::to_vec(&message).unwrap_or_else(|_| first.to_vec())
}

impl Harness {
    /// Settles the start `first` asks for before the account is held for it: a pool it names becomes the first of the pool's
    /// accounts that can take it, and a model it names must be one that the account the start runs on lists, which the refusal
    /// names. A provider that cannot list its models before a session leaves the model to the session's own options.
    ///
    /// # Errors
    ///
    /// Why the start is refused, for the client.
    pub(crate) fn settle(&self, first: &[u8], any: bool) -> Result<Settled, String> {
        let model = named_model(first)?;
        let offer = self.accounts.as_ref();
        let pool = match named_account(first) {
            Some(Some(name)) => self.pool_named(&name, any),
            Some(None) => None,
            None => offer
                .and_then(|offer| offer.default_pool.as_deref())
                .and_then(|name| self.pool_named(name, any)),
        };
        if let Some(pool) = pool {
            let account = self.account_of(pool, model.as_deref(), any)?;
            return Ok(Settled {
                first: naming(first, &account),
                pool: Some(pool.name.clone()),
            });
        }
        let Some(model) = model else {
            return Ok(Settled {
                first: first.to_vec(),
                pool: None,
            });
        };
        let picked = self.pick(first, any)?;
        let (who, listed) = match (picked, offer.and_then(|offer| offer.config.as_ref())) {
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
            None => Ok(Settled {
                first: first.to_vec(),
                pool: None,
            }),
        }
    }

    /// The account of `pool` a start takes: the next in turn of those that are free, list `model` when the start names one, and
    /// have usage left for it.
    fn account_of(
        &self,
        pool: &EntryPool,
        model: Option<&str>,
        any: bool,
    ) -> Result<String, String> {
        let Some(config) = self
            .accounts
            .as_ref()
            .and_then(|offer| offer.config.as_deref())
        else {
            return Err(format!("this entry offers no pool {}", pool.name));
        };
        let names: Vec<String> = pool.usable(any).map(|member| member.name.clone()).collect();
        let candidates = crate::pools::gather::candidates(
            config,
            self,
            &names,
            (model, None),
            (&[], &crate::options::limits::Endpoints::default()),
            any,
        );
        let turn = crate::pools::turn::Turn {
            current: None,
            model,
            idle: None,
            lifetime: coqui_acp_api::CacheLifetime::Default,
            now: crate::usage_splice::unix_now(),
        };
        crate::pools::turn::decide(&pool.name, &crate::pools::turn::CURSOR, &candidates, &turn)
            .map(|chosen| chosen.account)
    }
}
