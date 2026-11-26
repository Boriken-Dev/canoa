//! Where Graph's access token comes from. [`TokenSource`] is the one seam: the real one ([`Wam`]) takes the token silently from the
//! person's Windows sign-in through the Web Account Manager, by a helper process, for an app registration the account names; a
//! test supplies a fixed one. [`Tokens`] holds a token until shortly before it expires.
//!
//! The helper is Windows PowerShell with WinRT, not a native call: the host forbids `unsafe` outside one Win32 module, WinRT needs
//! the thread initialised through `unsafe` calls, and a process of its own keeps that, and a dependency the offline mirror would
//! have to carry, out of the host.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the clock and shared ownership are std's"
)]

use core::fmt;
use core::time::Duration;
use std::sync::{Arc, Mutex, PoisonError};
use std::time::Instant;

use canoa::network::Secret;
use data_encoding::BASE64URL_NOPAD;
use serde_json::{Value, json};

use crate::outlook::{PowerShell, Runner};
use canoa_mail::mail_backend::MailError;

/// The helper script: it asks WAM for a token and writes it to standard output.
pub(crate) const SCRIPT: &str = include_str!("wam.ps1");

/// The permission the app registration is asked for: read and write the person's mail, which includes saving a draft and does not
/// include sending.
pub(crate) const SCOPE: &str = "https://graph.microsoft.com/Mail.ReadWrite";

/// How long before its end a token is no longer used, so a call that starts with it does not finish without it.
const MARGIN: Duration = Duration::from_secs(300);

/// How long a token is held when it says nothing of its own end.
const UNKNOWN: Duration = Duration::from_secs(600);

/// An access token and how long it lasts, when it says. Its `Debug` shows no token: a `Secret` prints as a mask.
#[derive(Debug)]
pub(crate) struct Token {
    pub(crate) value: Secret,
    pub(crate) lifetime: Option<Duration>,
}

/// Something that gives Graph access tokens for the signed-in person.
pub(crate) trait TokenSource: Send + Sync + fmt::Debug {
    /// A token for [`SCOPE`]. It is asked again after Graph refuses one, so a source that holds tokens drops them first.
    fn token(&self) -> Result<Token, MailError>;
}

/// A source's token, held until near its end.
#[derive(Debug)]
pub(crate) struct Tokens {
    source: Arc<dyn TokenSource>,
    held: Mutex<Option<(Secret, Instant)>>,
}

impl Tokens {
    pub(crate) fn new(source: Arc<dyn TokenSource>) -> Self {
        Self {
            source,
            held: Mutex::new(None),
        }
    }

    /// The token to send: the one held while it lasts, else a new one.
    ///
    /// # Errors
    ///
    /// The source gave none.
    pub(crate) fn get(&self) -> Result<Secret, MailError> {
        let mut held = self.held.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some((value, until)) = held.as_ref()
            && Instant::now() < *until
        {
            return Ok(value.clone());
        }
        let token = self.source.token()?;
        let lifetime = token
            .lifetime
            .or_else(|| lifetime_of(token.value.expose()))
            .unwrap_or(UNKNOWN);
        let until = Instant::now() + lifetime.saturating_sub(MARGIN);
        *held = Some((token.value.clone(), until));
        Ok(token.value)
    }

    /// Drops the token held, after Graph refused it.
    pub(crate) fn forget(&self) {
        *self.held.lock().unwrap_or_else(PoisonError::into_inner) = None;
    }
}

/// How long from now a JSON Web Token lasts, by its `exp` claim; none when it is not one or has none.
pub(crate) fn lifetime_of(token: &str) -> Option<Duration> {
    let payload = token.split('.').nth(1)?;
    let bytes = BASE64URL_NOPAD
        .decode(payload.trim_end_matches('=').as_bytes())
        .ok()?;
    let claims: Value = serde_json::from_slice(&bytes).ok()?;
    let end = claims["exp"].as_u64()?;
    let now = std::time::SystemTime::now()
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_secs();
    Some(Duration::from_secs(end.saturating_sub(now)))
}

/// The Windows sign-in, through the Web Account Manager.
#[derive(Debug)]
pub(crate) struct Wam {
    runner: Arc<dyn Runner>,
    client_id: String,
    tenant: String,
    login_hint: Option<String>,
}

impl Wam {
    pub(crate) fn new(
        runner: Arc<dyn Runner>,
        client_id: &str,
        tenant: &str,
        login_hint: Option<String>,
    ) -> Self {
        Self {
            runner,
            client_id: client_id.to_owned(),
            tenant: tenant.to_owned(),
            login_hint,
        }
    }

    /// The Windows sign-in reached by the helper through the command and time an account names.
    pub(crate) fn powershell(
        command: Option<Vec<String>>,
        client_id: &str,
        tenant: &str,
        login_hint: Option<String>,
        timeout: Duration,
    ) -> Self {
        let runner = PowerShell::new(command, timeout).with_script(SCRIPT, "the Windows sign-in");
        Self::new(Arc::new(runner), client_id, tenant, login_hint)
    }
}

impl TokenSource for Wam {
    fn token(&self) -> Result<Token, MailError> {
        let answer = self
            .runner
            .run(&json!({
                "op": "token",
                "client_id": self.client_id,
                "tenant": self.tenant,
                "scope": SCOPE,
                "login_hint": self.login_hint,
            }))
            .map_err(|error| {
                MailError::Auth(format!(
                    "no token for Microsoft Graph came from the Windows sign-in for the app {}: {error}. The person must be \
                     signed in to Windows or Office with the mailbox's account, and the app registered in Entra ID as a public \
                     client with the delegated permission Mail.ReadWrite: register one, or use the `outlook` backend, which \
                     needs no registration",
                    self.client_id
                ))
            })?;
        let value = answer["token"]
            .as_str()
            .filter(|token| !token.is_empty())
            .ok_or_else(|| MailError::Auth("the Windows sign-in gave no token".to_owned()))?;
        Ok(Token {
            value: Secret::new(value),
            lifetime: answer["expires_in"].as_u64().map(Duration::from_secs),
        })
    }
}
