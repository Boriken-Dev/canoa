//! Forwarded certificate parsing, matcher building, and pinned certificate enrolment for the portal.

use alloc::string::{String, ToString};
use alloc::sync::Arc;
use alloc::vec::Vec;

use coqui_access::guatiao::Value;
use coqui_access::guatiao::library::Registry;
use coqui_access::{AccountPut, Accounts as AccountKind, Directory, Lookup};
use coqui_access_kit::matcher::{MatchRule, Matcher, MatcherConfig};
use coqui_peer_tls::CertificateFields;

use super::super::settings::Settings;
use super::super::signin::{CertificateStatus, PinError, PinState, Presented, valid_user};
use super::Link;

/// The group a pinned certificate's record carries to name the user id its account had when it was pinned, so that an account
/// deleted and made again under the same name does not inherit the pin.
pub(super) const UID_MARKER: &str = "uid:";

/// The most certificates (leaf first) and bytes a forwarded header may carry.
pub(super) const MAX_CHAIN: usize = 8;
pub(super) const MAX_HEADER: usize = 16 * 1024;

/// Where enrolled certificates are kept: a certificate provider's account kind, and its directory to say whose one is.
pub(super) struct PinStore {
    pub(super) accounts: Arc<dyn AccountKind>,
    pub(super) directory: Arc<dyn Directory>,
}

/// The matcher `settings` describe: its rules, or coqui-access's defaults when it gives none.
pub(super) fn build_matcher(settings: &Settings) -> Result<Matcher, String> {
    let Some(rules) = &settings.matcher else {
        return Ok(Matcher::default());
    };
    let mut config = MatcherConfig::default();
    config.rule = rules
        .iter()
        .map(|rule| {
            let mut built = MatchRule::new(rule.by.clone(), rule.template.clone());
            if let Some(domain) = &rule.domain {
                built = built.domain(domain.clone());
            }
            if let Some(pins) = &rule.pins {
                built = built.pins(pins.clone());
            }
            built
        })
        .collect();
    Matcher::new(config).map_err(|error| format!("matcher: {error}"))
}

/// Whether a pin's groups name `uid` as the account's user id, or name none (a pin made by hand).
pub(super) fn marker_fits(groups: &[String], uid: u32) -> bool {
    groups
        .iter()
        .find_map(|group| group.strip_prefix(UID_MARKER))
        .is_none_or(|marker| marker == uid.to_string())
}

pub(super) fn pin_store(
    registry: &Registry,
    id: &str,
    config: &Value,
    link: &Link,
) -> Result<PinStore, String> {
    let built = |why: String| format!("authenticators: {id}: {why}");
    let accounts: Arc<dyn AccountKind> = Arc::new(
        registry
            .offer::<dyn AccountKind>(id)
            .ok_or_else(|| built("no provider has that id".to_owned()))?
            .map_err(|_| {
                built("it keeps no accounts, so it cannot hold enrolled certificates".to_owned())
            })?
            .instantiate(config)
            .map_err(|error| built(error.to_string()))?,
    );
    let directory = link.directory.clone().ok_or_else(|| {
        built("it has no directory, so it cannot say whose a certificate is".to_owned())
    })?;
    Ok(PinStore {
        accounts,
        directory,
    })
}

pub(super) fn leaf_fields(der: &[u8]) -> Option<CertificateFields> {
    CertificateFields::parse(der).ok()
}

/// Whether now is before the certificate's first day or after its last.
pub(super) fn outside_validity(fields: &CertificateFields) -> bool {
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .map_or(0, |since| {
            i64::try_from(since.as_secs()).unwrap_or(i64::MAX)
        });
    now < fields.not_before || now > fields.not_after
}

/// The certificates (DER, leaf first) in the one `header` of `headers`: base64, comma-separated. `None` for no header, two,
/// one too large, one that is not base64 or holds too many certificates.
pub(super) fn forwarded_chain(headers: &[(String, String)], header: &str) -> Option<Vec<Vec<u8>>> {
    let mut found = headers.iter().filter(|(name, _)| name == header);
    let (_, value) = found.next()?;
    if found.next().is_some() || value.len() > MAX_HEADER {
        return None;
    }
    let chain: Vec<_> = value
        .split(',')
        .map(|part| data_encoding::BASE64.decode(part.trim().as_bytes()).ok())
        .collect::<Option<_>>()?;
    if chain.is_empty() || chain.len() > MAX_CHAIN || chain.iter().any(Vec::is_empty) {
        return None;
    }
    Some(chain)
}

pub(super) fn presented(headers: &[(String, String)], header: &str) -> Option<Presented> {
    let chain = forwarded_chain(headers, header)?;
    let fields = leaf_fields(chain.first()?)?;
    let expired = outside_validity(&fields);
    let map = fields.to_map();
    let first = |key: &str| map.get(key).and_then(|values| values.first()).cloned();
    Some(Presented {
        subject: first("subject").unwrap_or_default(),
        fingerprint: first("fingerprint")?,
        expired,
    })
}

impl PinStore {
    pub(super) fn status(
        &self,
        header: &str,
        user: &str,
        uid: u32,
        headers: &[(String, String)],
    ) -> CertificateStatus {
        let presented = presented(headers, header).map(|presented| {
            let state = match self.directory.lookup(Lookup {
                by: "certificate".to_owned(),
                value: presented.fingerprint.clone(),
                claims: None,
            }) {
                Ok(Some(person)) if person.name == user && marker_fits(&person.groups, uid) => {
                    PinState::Yours
                }
                // Another name's, or this name's from before the account was made again: neither signs this account in.
                Ok(Some(person)) if person.name != user => PinState::Another,
                _ => PinState::Unpinned,
            };
            (presented, state)
        });
        let (pinned, disabled) = self.accounts.accounts().map_or((false, false), |all| {
            all.iter()
                .find(|account| account.name == user)
                .map_or((false, false), |account| {
                    let disabled = account.disabled == Some(true);
                    (!disabled && marker_fits(&account.groups, uid), disabled)
                })
        });
        CertificateStatus {
            presented,
            pinned,
            disabled,
        }
    }

    pub(super) fn pin(
        &self,
        header: &str,
        user: &str,
        uid: u32,
        headers: &[(String, String)],
    ) -> Result<(), PinError> {
        let presented = presented(headers, header).ok_or(PinError::NoCertificate)?;
        if presented.expired {
            return Err(PinError::Expired);
        }
        if !valid_user(user) {
            return Err(PinError::Failed("not an account name".to_owned()));
        }
        let all = self
            .accounts
            .accounts()
            .map_err(|error| PinError::Failed(error.to_string()))?;
        // A pin an administrator disabled is theirs to lift: removing it and enrolling again would lift it from here.
        if all
            .iter()
            .any(|account| account.name == user && account.disabled == Some(true))
        {
            return Err(PinError::Disabled);
        }
        match self.directory.lookup(Lookup {
            by: "certificate".to_owned(),
            value: presented.fingerprint.clone(),
            claims: None,
        }) {
            Ok(Some(person)) if person.name != user => return Err(PinError::Taken),
            Err(error) => return Err(PinError::Failed(error.to_string())),
            _ => {}
        }
        // The provider adds a pin to an account that has one. Enrolling means this certificate is the account's only one, so the
        // record goes first: a failure after that leaves the account with no pin, never with the old one.
        if all.iter().any(|account| account.name == user) {
            self.accounts
                .remove(user.to_owned())
                .map_err(|error| PinError::Failed(error.to_string()))?;
        }
        self.accounts
            .put(AccountPut {
                name: user.to_owned(),
                secret: Some(presented.fingerprint),
                groups: Some(vec![format!("{UID_MARKER}{uid}")]),
                ..AccountPut::default()
            })
            .map(drop)
            .map_err(|error| PinError::Failed(error.to_string()))
    }

    pub(super) fn unpin(&self, user: &str) -> Result<bool, PinError> {
        let all = self
            .accounts
            .accounts()
            .map_err(|error| PinError::Failed(error.to_string()))?;
        let Some(found) = all.iter().find(|account| account.name == user) else {
            return Ok(false);
        };
        if found.disabled == Some(true) {
            return Err(PinError::Disabled);
        }
        self.accounts
            .remove(user.to_owned())
            .map(|()| true)
            .map_err(|error| PinError::Failed(error.to_string()))
    }
}
