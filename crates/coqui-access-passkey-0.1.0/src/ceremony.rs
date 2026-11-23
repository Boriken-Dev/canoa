//! Sign-in: the challenge a consumer asks for and the signed answer it presents.

use std::net::SocketAddr;
use std::time::{Duration, Instant};

use coqui_access::guatiao::library::ProviderError;
use coqui_access::{
    Attempt, Authenticator, Challenge, ChallengeRequest, Challenger, Claims, CredentialInfo,
    Description, Lookup, Verdict, error, methods, takes,
};
use coqui_peer::{PeerId, WaitingQueue};
use data_encoding::{BASE64URL_NOPAD, HEXLOWER};
use webauthn_rs_core::error::WebauthnError;
use webauthn_rs_core::proto::RequestChallengeResponse;
use webauthn_rs_core::proto::{AuthenticationState, Credential, PublicKeyCredential};

use crate::Passkeys;
use crate::records::{Ceremony, PasskeyAccount, StoredCredential};
use crate::settings::OnRegression;

/// A passkey as [`AccountInfo::credentials`](coqui_access::AccountInfo::credentials) lists it.
/// A passkey is synced when its authenticator says it may exist on several devices.
pub(crate) fn info(stored: &StoredCredential) -> CredentialInfo {
    CredentialInfo {
        id: stored.id.clone(),
        label: stored.label.clone(),
        made_unix: Some(stored.made_unix),
        used_unix: stored.used_unix,
        synced: serde_json::from_str::<Credential>(&stored.key)
            .ok()
            .map(|credential| credential.backup_eligible),
        disabled: Some(stored.disabled),
    }
}

/// The source a challenge is counted against: the address, with a port or without.
fn source_of(peer: Option<&str>) -> Option<SocketAddr> {
    let peer = peer?;
    peer.parse::<SocketAddr>()
        .ok()
        .or_else(|| peer.parse().ok().map(|address| SocketAddr::new(address, 0)))
}

/// How long ago, as an instant the queue can compare.
fn age_of(now: Instant, now_unix: i64, created_unix: i64) -> Instant {
    let age =
        Duration::from_secs(u64::try_from(now_unix.saturating_sub(created_unix)).unwrap_or(0));
    now.checked_sub(age).unwrap_or(now)
}

/// Whether the client data says the page that asked was in a cross-origin frame. The verifier
/// refuses that at registration only; the signature covers these bytes, so what is read here is
/// what the verifier checks next.
fn crossed(response: &PublicKeyCredential) -> bool {
    serde_json::from_slice::<serde_json::Value>(&response.response.client_data_json)
        .is_ok_and(|data| data["crossOrigin"].as_bool() == Some(true))
}

/// What a sign-in presenting `credential` and a counter came to.
enum Judged {
    Signed {
        synced: bool,
        user_verified: bool,
    },
    /// The counter did not rise.
    Regressed,
    /// The account or the passkey is gone since it was read.
    Gone,
}

impl Passkeys {
    /// Whether `ceremony` is still within its time.
    fn live(&self, ceremony: &Ceremony, now: i64) -> bool {
        now.saturating_sub(ceremony.created_unix) <= i64::from(self.ceremony_seconds)
    }

    /// Whether a passkey may sign in now: a synced one is refused under `reject_synced`,
    /// including one registered before that was set.
    fn accepts(&self, credential: &Credential) -> bool {
        !(self.reject_synced && credential.backup_eligible)
    }

    /// The enabled account `name`, when it holds at least one enabled passkey.
    fn challengeable(&self, name: &str) -> Result<Vec<Credential>, ProviderError> {
        let account = self
            .accounts
            .get(name)
            .map_err(|reason| reason.reading("passkey table"))?;
        let credentials: Vec<Credential> = account
            .filter(|account| !account.disabled)
            .map(|account| {
                account
                    .credentials
                    .iter()
                    .filter(|credential| !credential.disabled)
                    .filter_map(|credential| serde_json::from_str(&credential.key).ok())
                    .filter(|credential| self.accepts(credential))
                    .collect()
            })
            .unwrap_or_default();
        if credentials.is_empty() {
            Err(error::not_found(
                "that account has no passkey to sign in with",
            ))
        } else {
            Ok(credentials)
        }
    }

    /// Takes the challenge `key` out of the table: whichever call removes it owns it, so one
    /// challenge answers one sign-in.
    fn take_ceremony(&self, key: &str) -> Result<Option<Ceremony>, ProviderError> {
        let Some(ceremony) = self
            .ceremonies
            .get(key)
            .map_err(|reason| reason.reading("passkey ceremony table"))?
        else {
            return Ok(None);
        };
        let taken = self
            .ceremonies
            .remove(key)
            .map_err(|reason| reason.reading("passkey ceremony table"))?;
        Ok(taken.then_some(ceremony))
    }

    /// The account a signed answer names: the one the challenge was for, else the one its user
    /// handle says.
    fn account_of(
        &self,
        ceremony: &Ceremony,
        response: &PublicKeyCredential,
    ) -> Result<Option<PasskeyAccount>, ProviderError> {
        let handle = response
            .get_user_unique_id()
            .map(|bytes| HEXLOWER.encode(bytes));
        let account = match &ceremony.name {
            Some(name) => self
                .accounts
                .get(name)
                .map_err(|reason| reason.reading("passkey table"))?,
            None => {
                let Some(handle) = &handle else {
                    return Ok(None);
                };
                self.accounts
                    .read(|records| records.iter().find(|held| &held.id == handle).cloned())
                    .map_err(|reason| reason.reading("passkey table"))?
            }
        };
        // A user handle the answer carries must be the account's own.
        Ok(account.filter(|account| handle.as_ref().is_none_or(|handle| *handle == account.id)))
    }

    /// Writes down that `stored` signed in, in one write under the table's lock, judging the
    /// counter against what is stored then.
    fn record_sign_in(
        &self,
        account: &PasskeyAccount,
        stored: &StoredCredential,
        result: &webauthn_rs_core::proto::AuthenticationResult,
        now: i64,
    ) -> Result<Judged, ProviderError> {
        self.accounts
            .update_one(&account.name, |current| {
                let gone = Ok((None, Judged::Gone));
                let Some(current) = current else {
                    return gone;
                };
                let mut next = current.clone();
                let Some(slot) = next
                    .credentials
                    .iter_mut()
                    .find(|slot| slot.id == stored.id && !slot.disabled)
                else {
                    return gone;
                };
                let Ok(mut credential) = serde_json::from_str::<Credential>(&slot.key) else {
                    return gone;
                };
                // Another sign-in may have written a counter since this one was read.
                if (result.counter() > 0 || credential.counter > 0)
                    && result.counter() <= credential.counter
                {
                    return Ok((self.regress(next, &stored.id), Judged::Regressed));
                }
                credential.counter = result.counter().max(credential.counter);
                credential.backup_state = result.backup_state();
                credential.backup_eligible = result.backup_eligible();
                let synced = credential.backup_eligible;
                slot.key = serde_json::to_string(&credential).map_err(|_| {
                    coqui_access_kit::SourceError::invalid("a passkey could not be written")
                })?;
                slot.used_unix = Some(now);
                Ok((
                    Some(next),
                    Judged::Signed {
                        synced,
                        user_verified: result.user_verified(),
                    },
                ))
            })
            .map_err(|_| error::internal("passkey table: the sign-in could not be written down"))
    }

    /// The account with the passkey `id` disabled, when the setting says to.
    fn regress(&self, mut account: PasskeyAccount, id: &str) -> Option<PasskeyAccount> {
        if self.counter_regression != OnRegression::Disable {
            return None;
        }
        for credential in account.credentials.iter_mut().filter(|slot| slot.id == id) {
            credential.disabled = true;
        }
        Some(account)
    }

    /// Disables the passkey `id` of `name`, as a counter that did not rise says to.
    fn disable_credential(&self, name: &str, id: &str) -> Result<(), ProviderError> {
        self.accounts
            .update_one(name, |current| {
                Ok((current.and_then(|held| self.regress(held.clone(), id)), ()))
            })
            .map_err(|_| error::internal("passkey table: the passkey could not be disabled"))
    }
}

impl Challenger for Passkeys {
    /// The options for `navigator.credentials.get`. `name` set: that enabled account's enabled
    /// passkeys (`GUATIAO_ERR_NOT_FOUND` for an unknown or disabled account or none enabled);
    /// unset: any discoverable one. `error::busy()` when the source of `peer` already holds its
    /// share of the challenges waiting.
    fn challenge(&self, request: ChallengeRequest) -> Result<Challenge, ProviderError> {
        match request.method.as_deref() {
            None | Some(methods::PASSKEY) => {}
            Some(_) => return Err(error::unsupported("this provider challenges passkeys only")),
        }
        let credentials = match &request.name {
            Some(name) => self.challengeable(name)?,
            None => Vec::new(),
        };
        let builder = self
            .core
            .new_challenge_authenticate_builder(credentials, Some(self.user_verification.policy()))
            .map_err(|_| error::internal("the sign-in options could not be made"))?;
        let (options, state): (RequestChallengeResponse, AuthenticationState) = self
            .core
            .generate_challenge_authenticate(builder)
            .map_err(|_| error::internal("the sign-in options could not be made"))?;
        let value = serde_json::to_value(&options)
            .map_err(|_| error::internal("the sign-in options could not be written"))?;
        let key = value["publicKey"]["challenge"]
            .as_str()
            .ok_or_else(|| error::internal("the sign-in options hold no challenge"))?
            .to_owned();
        let options = value.to_string();
        let state = serde_json::to_string(&state)
            .map_err(|_| error::internal("the sign-in state could not be written"))?;
        let now = self.now();
        let ceremony = Ceremony {
            challenge: key.clone(),
            name: request.name,
            source: request.peer,
            created_unix: now,
            state,
        };
        let source = source_of(ceremony.source.as_deref());
        let id = PeerId::of(key.as_bytes());
        let mut expired = false;
        let mut admitted = false;
        // What other instances and processes wrote counts against its source and the total.
        self.ceremonies
            .update(&mut |held| {
                let (instant, unix) = (Instant::now(), now);
                let mut queue = WaitingQueue::builder()
                    .limits(self.ceremony_limits)
                    .record_interval(Duration::ZERO)
                    .build()
                    .map_err(|_| {
                        coqui_access_kit::SourceError::invalid("the ceremony bounds cannot be used")
                    })?;
                expired = false;
                let mut live = 0;
                for had in held {
                    if self.live(had, unix) {
                        live += 1;
                        queue.remember(
                            PeerId::of(had.challenge.as_bytes()),
                            source_of(had.source.as_deref()),
                            age_of(instant, unix, had.created_unix),
                        );
                    } else {
                        expired = true;
                    }
                }
                admitted = queue.admit(id, source, live, instant).record;
                Ok(admitted.then(|| ceremony.clone()))
            })
            .map_err(|reason| reason.reading("passkey ceremony table"))?;
        // In one write: a file table rewrites itself on each, so a write per expired challenge
        // would make this one wait for every challenge a flood left. Best effort: an expired
        // challenge is refused when it is presented, whether or not it is still there.
        if expired {
            let _ = self
                .ceremonies
                .remove_where(&mut |had| !self.live(had, now));
        }
        if !admitted {
            return Err(error::busy());
        }
        Ok(Challenge {
            challenge: key,
            options,
            expires_unix: now.saturating_add(i64::from(self.ceremony_seconds)),
        })
    }
}

impl Authenticator for Passkeys {
    /// `challenge` is what [`Challenger::challenge`] answered and `secret` the browser's signed
    /// answer (JSON text), both required, else `Unknown`; so is a method other than `passkey`.
    ///
    /// The challenge is taken out of the table first, so one answers one sign-in, and `Unknown`
    /// means it was not issued here, was used already or was swept. An expired challenge, an
    /// answer too large or unreadable, no account (the challenge's, else the user handle's), a
    /// user handle that is not that account's, a disabled account or passkey, a synced passkey
    /// under `reject_synced`, a passkey the account does not hold and anything the verifier
    /// refuses are `Invalid`. A success is
    /// written down (counter, backup flags, last use) before it is answered, and a write that
    /// fails is an error, never a success.
    ///
    /// A success is `Valid(Lookup { by: "id", .. })` with the claims `passkey.credential` (the
    /// id), `passkey.user_verified` and `passkey.synced` (`true` or `false`).
    fn authenticate(&self, attempt: Attempt) -> Result<Verdict, ProviderError> {
        match attempt.method.as_deref() {
            None | Some(methods::PASSKEY) => {}
            Some(_) => return Ok(Verdict::Unknown),
        }
        let (Some(challenge), Some(answer)) = (attempt.challenge, attempt.secret) else {
            return Ok(Verdict::Unknown);
        };
        let Some(ceremony) = self.take_ceremony(&challenge)? else {
            return Ok(Verdict::Unknown);
        };
        let now = self.now();
        if !self.live(&ceremony, now) || !self.max_response_bytes.allows(answer.len()) {
            return Ok(Verdict::Invalid);
        }
        let Ok(response) = serde_json::from_str::<PublicKeyCredential>(&answer) else {
            return Ok(Verdict::Invalid);
        };
        if crossed(&response) {
            return Ok(Verdict::Invalid);
        }
        let state: AuthenticationState = serde_json::from_str(&ceremony.state)
            .map_err(|_| error::internal("a waiting sign-in cannot be read"))?;
        let Some(account) = self.account_of(&ceremony, &response)? else {
            return Ok(Verdict::Invalid);
        };
        let raw = BASE64URL_NOPAD.encode(&response.raw_id);
        let Some(stored) = account
            .credentials
            .iter()
            .find(|credential| credential.id == raw)
            .filter(|_| !account.disabled)
            .filter(|credential| !credential.disabled)
        else {
            return Ok(Verdict::Invalid);
        };
        let credential: Credential = serde_json::from_str(&stored.key)
            .map_err(|_| error::internal("a passkey cannot be read"))?;
        if !self.accepts(&credential) {
            return Ok(Verdict::Invalid);
        }
        let result = match self
            .core
            .authenticate_single_credential(&response, &state, &credential)
        {
            Ok(result) => result,
            Err(WebauthnError::CredentialPossibleCompromise) => {
                self.disable_credential(&account.name, &stored.id)?;
                return Ok(Verdict::Invalid);
            }
            Err(_) => return Ok(Verdict::Invalid),
        };
        match self.record_sign_in(&account, stored, &result, now)? {
            Judged::Signed {
                synced,
                user_verified,
            } => {
                let mut claims = Claims::default();
                claims
                    .values
                    .insert("passkey.credential".to_owned(), vec![stored.id.clone()]);
                claims.values.insert(
                    "passkey.user_verified".to_owned(),
                    vec![user_verified.to_string()],
                );
                claims
                    .values
                    .insert("passkey.synced".to_owned(), vec![synced.to_string()]);
                Ok(Verdict::Valid(Lookup {
                    by: "id".to_owned(),
                    value: account.id,
                    claims: Some(claims),
                }))
            }
            Judged::Regressed | Judged::Gone => Ok(Verdict::Invalid),
        }
    }

    fn describe(&self) -> Result<Description, ProviderError> {
        Ok(Description {
            name: Some("Passkeys".to_owned()),
            takes: Some(takes::PASSKEY.to_owned()),
        })
    }
}
