//! `history/account`: carrying a conversation from the account it ran on to another account of the same entry.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "threads and the clock are std's"
)]

use std::time::Duration;

use coqui_acp_api::guatiao::Status;
use coqui_acp_api::{CacheLifetime, SessionExport, SessionImport};
use serde_json::{Value, json};

use super::moves::new_uuid;
use super::{BAD_PARAMS, Control, NOT_FOUND, REFUSED, bad, text_param};
use crate::accounts::{Held, Refusal};
use crate::history;
use crate::kind;

/// The most history one account move reads, as `[host.history]` bounds a recorded one.
const MAX_BYTES: usize = 64 * 1024 * 1024;
/// How long a move waits for an account whose last connection is still ending: 50 looks, 100 ms apart.
const LOOKS: u32 = 50;

type Refused = (i32, String);

/// The account `name` of `harness`, waiting a little while the connection that ran on it is still ending.
fn held(harness: &kind::Harness, name: &str) -> Result<Held, Refused> {
    for look in 0..LOOKS {
        match harness.hold_account(name, "moving a conversation") {
            Ok(held) => return Ok(held),
            Err(Refusal::InUse(by)) if look + 1 == LOOKS => {
                return Err((
                    REFUSED,
                    format!(
                        "account {name} is held by {by}, so its conversation cannot be moved now"
                    ),
                ));
            }
            Err(Refusal::InUse(_)) => std::thread::sleep(Duration::from_millis(100)),
            Err(other) => return Err((REFUSED, other.client_reason())),
        }
    }
    Err((REFUSED, format!("account {name} could not be held")))
}

/// The whole history of `session` as `harness` exports it from account `from`'s folder, or the provider's refusal.
fn export_from(
    harness: &kind::Harness,
    from: &str,
    session: &str,
    cwd: &str,
) -> Result<String, Result<(Status, String), Refused>> {
    let held = held(harness, from).map_err(Err)?;
    let mut text = String::new();
    let mut after: Option<String> = None;
    loop {
        let connection = harness.connection(None, Some(&held), CacheLifetime::default());
        let mut ask = SessionExport::new(session, cwd).account(held.grant());
        if let Some(position) = after.take() {
            ask = ask.after(position);
        }
        let page = connection
            .harness
            .export_session(ask, connection.spawner, connection.keeper)
            .map_err(|error| Ok((error.status, error.message().to_owned())))?;
        text.push_str(&page.text);
        if text.len() > MAX_BYTES {
            return Err(Err((
                REFUSED,
                format!("the history of it is more than {MAX_BYTES} bytes"),
            )));
        }
        match page.next {
            Some(next) => after = Some(next),
            None => return Ok(text),
        }
    }
}

impl Control {
    /// `history/account` `{"entry", "session", "cwd", "from"?, "to"}`: what carries a conversation of the harness entry `entry` from
    /// the account it ran on (`from`, else the entry's first) to another account the entry offers. Accounts with no folder of their
    /// own keep conversations in one place, so nothing moves: `{"moved": false}`, and the session resumes as it is. Where an account
    /// has a folder of its own, the provider exports the conversation from the first account's folder and imports it into the
    /// second's at a new session id, which is answered (`{"moved": true, "session", "via": "provider"}`). A provider that cannot
    /// export it falls back to the history this host recorded (`"via": "record"`); one that can do neither is refused with its reason.
    pub(super) fn account_move(&self, params: &Value) -> Result<Value, Refused> {
        let moves = self
            .history
            .as_ref()
            .ok_or((REFUSED, "this host moves no sessions".to_owned()))?;
        let entry = text_param(params, "entry")?.ok_or_else(|| bad("entry is needed"))?;
        let session = text_param(params, "session")?.ok_or_else(|| bad("session is needed"))?;
        let cwd = text_param(params, "cwd")?.ok_or_else(|| bad("cwd is needed"))?;
        let to = text_param(params, "to")?.ok_or_else(|| bad("to is needed"))?;
        if entry.len() > 256 || session.len() > 256 || cwd.chars().any(char::is_control) {
            return Err(bad("entry, session or cwd is not one"));
        }
        let harness = moves
            .targets
            .get(entry)
            .ok_or((NOT_FOUND, "this host has no such source".to_owned()))?;
        let offered = harness.offered();
        let from = match text_param(params, "from")? {
            Some(name) => name,
            None => offered
                .first()
                .map(|(name, ..)| name.as_str())
                .ok_or((REFUSED, format!("{entry} offers no accounts")))?,
        };
        let info_of = |name: &str| {
            offered
                .iter()
                .find(|(offered, ..)| offered == name)
                .map(|(_, _, info)| info)
                .ok_or((
                    NOT_FOUND,
                    format!("{entry} does not offer the account {name}"),
                ))
        };
        let (from_info, to_info) = (info_of(from)?, info_of(to)?);
        if from == to {
            return Err((
                BAD_PARAMS,
                "the conversation is on that account already".to_owned(),
            ));
        }
        // Accounts with no folder of their own keep conversations where the program's own login does: one place.
        if !from_info.state_dir && !to_info.state_dir {
            return Ok(json!({"moved": false, "session": session, "cwd": cwd}));
        }
        let (text, via) = match export_from(harness, from, session, cwd) {
            Ok(text) => (text, "provider"),
            Err(Err(refused)) => return Err(refused),
            Err(Ok((status, why))) => {
                // What this host recorded of it is the standard history too.
                let recorded = (status == Status::GUATIAO_ERR_UNSUPPORTED
                    || status == Status::GUATIAO_ERR_NOT_FOUND)
                    .then(|| moves.store.load(entry, session).ok())
                    .flatten()
                    .and_then(|doc| doc.to_string().ok());
                match recorded {
                    Some(text) => (text, "record"),
                    None => {
                        return Err((
                            REFUSED,
                            format!(
                                "{entry} cannot export this conversation from account {from}, and the host has no record of it: {why}"
                            ),
                        ));
                    }
                }
            }
        };
        let doc: coqui_acp_history::Document =
            text.parse()
                .map_err(|error: coqui_acp_history::HistoryError| {
                    (
                        REFUSED,
                        format!("the exported history cannot be read: {error}"),
                    )
                })?;
        let new = new_uuid()?;
        let moved = history::reidentified(&doc, &new);
        let body = moved
            .to_string()
            .map_err(|e| (REFUSED, format!("the history cannot be written: {e}")))?;
        let account = held(harness, to)?;
        let connection = harness.connection(None, Some(&account), CacheLifetime::default());
        let report = connection
            .harness
            .import_session(
                SessionImport::new(new.clone(), cwd, body).account(account.grant()),
                connection.spawner,
                connection.keeper,
            )
            .map_err(|error| {
                (
                    REFUSED,
                    format!(
                        "{entry} cannot take this conversation into account {to}: {}",
                        error.message()
                    ),
                )
            })?;
        drop(account);
        if let Err(error) = moves.store.seed(entry, &new, &moved) {
            tracing::warn!("a moved session's history was not kept: {error}");
        }
        let losses: Vec<Value> = report
            .losses
            .iter()
            .map(|loss| json!({"kind": loss.kind, "count": loss.count, "detail": loss.detail}))
            .collect();
        Ok(json!({
            "moved": true,
            "session": new,
            "cwd": cwd,
            "via": via,
            "replayed": report.replayed(),
            "losses": losses,
        }))
    }
}
