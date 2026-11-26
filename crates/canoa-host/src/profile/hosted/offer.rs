//! Profile metadata keys, `initialize` selection parsers, and profile offer construction.

use alloc::borrow::ToOwned;
use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec;
use alloc::vec::Vec;
use std::ffi::OsString;

use indexmap::IndexMap;
use serde_json::{Value, json};

use super::{CLIENT_META, LOADOUT_META, LOCAL_PROFILE, OVERLAYS_META, PRESET_META, PROFILE_META};
use crate::config::HostProfile;

pub(super) const INVALID_REQUEST: i64 = -32600;
pub(super) const INVALID_PARAMS: i64 = -32602;

/// What a client asks for in `initialize`'s `_meta` when its connection starts, each as its `choose_*` function read it.
#[derive(Clone, Copy, Debug, Default)]
pub(crate) struct Choices<'a> {
    /// The overlays it starts with, which only narrow what the profile lists.
    pub only: Option<&'a [String]>,
    /// The loadout it starts with.
    pub loadout: Option<&'a str>,
    /// The preset it starts with.
    pub preset: Option<&'a str>,
    /// The kind of client it is.
    pub client: Option<&'a str>,
}

/// What an entry offers its clients: its profiles, the first by default, and the program that launches its agent.
#[derive(Debug)]
pub(crate) struct Offer {
    pub(crate) profiles: Vec<(String, HostProfile)>,
    /// Whether the profile is the default one, given because the entry names none, rather than one the entry names.
    pub(crate) by_default: bool,
    pub(crate) launcher: OsString,
    /// What goes between the launcher and a subcommand of the host's own: the forwarder is run as `launcher [before]
    /// hook-forward`.
    pub(crate) before: Vec<OsString>,
}

/// A client's first message refused: the answer to send, when it was a request, and what to log.
#[derive(Debug)]
pub(crate) struct Refusal {
    pub(crate) answer: Option<Vec<u8>>,
    pub(crate) log: &'static str,
}

pub(super) fn error(id: &Value, code: i64, message: &str) -> Vec<u8> {
    serde_json::to_vec(
        &json!({"jsonrpc": "2.0", "id": id, "error": {"code": code, "message": message}}),
    )
    .unwrap_or_default()
}

/// The profile `first` names in `initialize`'s `_meta`, by its index in `offer`; the first when it names none. A first
/// message other than `initialize`, and anything but the name of a profile the entry offers, is refused, before any
/// reference is read or any process starts.
pub(crate) fn choose(offer: &Offer, first: &[u8]) -> Result<usize, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let id = message.get("id").cloned();
    let choice = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(PROFILE_META));
    let refuse = |code: i64, message: &str, log: &'static str| Refusal {
        answer: id.as_ref().map(|id| error(id, code, message)),
        log,
    };
    if message.get("method").and_then(Value::as_str) != Some("initialize") {
        return Err(match choice {
            Some(_) => refuse(
                INVALID_PARAMS,
                "a profile is chosen in initialize",
                "a client named a profile outside initialize, which was refused",
            ),
            None => refuse(
                INVALID_REQUEST,
                "initialize comes first on an entry with a profile",
                "a client's first message was not initialize, which was refused",
            ),
        });
    }
    let Some(choice) = choice else {
        return Ok(0);
    };
    let refuse = |message: &str, log: &'static str| refuse(INVALID_PARAMS, message, log);
    let Some(name) = choice.as_str() else {
        return Err(refuse(
            "a profile is chosen by its name, among those this entry offers",
            "a client sent something other than a profile's name as its choice, which was refused",
        ));
    };
    offer
        .profiles
        .iter()
        .position(|(offered, _)| offered == name)
        .ok_or_else(|| {
            refuse(
                "this entry does not offer that profile",
                "a client asked for a profile this entry does not offer, which was refused",
            )
        })
}

/// The loadout `first` names for the connection in `initialize`'s `_meta`; `None` when it names none. A name the profile
/// `index` of `offer` does not hold, and anything but a name, is refused with the name.
pub(crate) fn choose_loadout(
    offer: &Offer,
    index: usize,
    first: &[u8],
) -> Result<Option<String>, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let Some(chosen) = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(LOADOUT_META))
    else {
        return Ok(None);
    };
    let refuse = |text: String, log: &'static str| Refusal {
        answer: message.get("id").map(|id| error(id, INVALID_PARAMS, &text)),
        log,
    };
    let Some(name) = chosen.as_str() else {
        return Err(refuse(
            "a loadout is chosen by its name".to_owned(),
            "a client sent something other than a loadout's name as its choice, which was refused",
        ));
    };
    // An empty name is the loadout that holds nothing: no overlay, skill or server, and not `[defaults] loadout` either.
    if name.is_empty() {
        return Ok(Some(String::new()));
    }
    match offer.profiles[index].1.resolve_loadout(Some(name)) {
        Ok(_) => Ok(Some(name.to_owned())),
        Err(unknown) => Err(refuse(
            unknown.to_string(),
            "a client asked for a loadout the profile does not hold, which was refused",
        )),
    }
}

/// The preset `first` names for the connection in `initialize`'s `_meta`; `None` when it names none. A name the profile `index`
/// of `offer` does not hold, and anything but a name, is refused with the name.
pub(crate) fn choose_preset(
    offer: &Offer,
    index: usize,
    first: &[u8],
) -> Result<Option<String>, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let Some(chosen) = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(PRESET_META))
    else {
        return Ok(None);
    };
    let refuse = |text: String, log: &'static str| Refusal {
        answer: message.get("id").map(|id| error(id, INVALID_PARAMS, &text)),
        log,
    };
    let Some(name) = chosen.as_str() else {
        return Err(refuse(
            "a profile is chosen by its name".to_owned(),
            "a client sent something other than a preset's name as its choice, which was refused",
        ));
    };
    match offer.profiles[index].1.resolve_preset(name) {
        Ok(_) => Ok(Some(name.to_owned())),
        Err(unknown) => Err(refuse(
            unknown.to_string(),
            "a client asked for a preset the profile does not hold, which was refused",
        )),
    }
}

/// The kind of client `first` says it is in `initialize`'s `_meta`; `None` when it says none. Anything but text is refused.
pub(crate) fn choose_client(first: &[u8]) -> Result<Option<String>, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let Some(chosen) = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(CLIENT_META))
    else {
        return Ok(None);
    };
    match chosen.as_str() {
        Some(kind) => Ok(Some(kind.to_owned())),
        None => Err(Refusal {
            answer: message
                .get("id")
                .map(|id| error(id, INVALID_PARAMS, "a client names its kind as text")),
            log: "a client sent something other than text as its kind, which was refused",
        }),
    }
}

/// The overlays `first` chooses for the connection in `initialize`'s `_meta`; `None` when it chooses none, which is every
/// overlay the profile lists. Anything but an array of names is refused.
pub(crate) fn choose_overlays(first: &[u8]) -> Result<Option<Vec<String>>, Refusal> {
    let message: Value = serde_json::from_slice(first).unwrap_or(Value::Null);
    let Some(chosen) = message
        .get("params")
        .and_then(|params| params.get("_meta"))
        .and_then(|meta| meta.get(OVERLAYS_META))
    else {
        return Ok(None);
    };
    let names: Option<Vec<String>> = chosen.as_array().and_then(|names| {
        names
            .iter()
            .map(|name| name.as_str().map(str::to_owned))
            .collect()
    });
    names.map(Some).ok_or_else(|| Refusal {
        answer: message
            .get("id")
            .map(|id| error(id, INVALID_PARAMS, "the overlays are chosen as a list of names")),
        log: "a client sent something other than a list of overlay names as its choice, which was refused",
    })
}

/// An entry's offer, from its profile names and the configuration's `[host.profiles]`.
pub(crate) fn offer(
    names: &[String],
    profiles: &IndexMap<String, HostProfile>,
    local: Option<&HostProfile>,
    launcher: OsString,
    before: Vec<OsString>,
) -> Option<Arc<Offer>> {
    if let Some(local) = local {
        return Some(Arc::new(Offer {
            profiles: vec![(LOCAL_PROFILE.to_owned(), local.clone())],
            by_default: true,
            launcher,
            before,
        }));
    }
    if names.is_empty() {
        return None;
    }
    Some(Arc::new(Offer {
        profiles: names
            .iter()
            .filter_map(|name| Some((name.clone(), profiles.get(name)?.clone())))
            .collect(),
        by_default: false,
        launcher,
        before,
    }))
}
