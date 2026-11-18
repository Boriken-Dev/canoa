//! Mail as a source of information, like the knowledge an overlay ships: the person's own mailbox searched, read, related to a
//! message, drafted into and organized, from any session. `[host.mail.accounts.NAME]` names each mailbox and the backend that
//! reaches it:
//!
//! - `ews`: Exchange Server's web services ([`ews`]), signed in with a user name and password the environment holds. The server
//!   is named or found by Autodiscover. Signing in with the Windows logon is one function ([`ews`]'s transport) that is not
//!   filled in yet.
//! - `outlook`: classic Outlook for Windows, through a helper that drives its object model ([`outlook`]); it covers every account
//!   Outlook holds and signs in as Outlook does.
//!
//! [`tools`] gives the agent `canoa-mail`: reads need no question, a draft is saved in Drafts and never sent, and each change to
//! messages that are already there is put to the person first.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "locks and the environment are std's"
)]

mod ews;
pub(crate) mod model;
pub(crate) mod related;
pub(crate) mod tools;
pub(crate) mod xml;

#[cfg(test)]
pub(crate) mod standin;
#[cfg(test)]
mod tests;

use core::time::Duration;
use std::sync::{Arc, Mutex, PoisonError};

use canoa::network::{Network, Secret};

use self::ews::{Auth, Ews, EwsSettings};
use self::model::{Backend, MailError};
use crate::config::{MailAccount, MailAuth, MailBackend, MailConfig};

/// A mailbox: its name, and the backend, made the first time it is asked for so that a password set after the host started, or a
/// server that is down now, is not fixed at startup.
struct Account {
    name: String,
    made: Mutex<Option<Arc<dyn Backend>>>,
    make: Box<dyn Fn() -> Result<Arc<dyn Backend>, MailError> + Send + Sync>,
    kind: &'static str,
}

impl core::fmt::Debug for Account {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Account")
            .field("name", &self.name)
            .field("kind", &self.kind)
            .finish_non_exhaustive()
    }
}

/// The mailboxes of `[host.mail]`.
#[derive(Debug, Default)]
pub(crate) struct Mailboxes {
    accounts: Vec<Account>,
}

impl Mailboxes {
    pub(crate) fn from_config(config: &MailConfig) -> Self {
        let accounts = config
            .accounts
            .iter()
            .map(|(name, account)| {
                let account = account.clone();
                let kind = match account.backend {
                    MailBackend::Ews => "ews",
                    MailBackend::Outlook => "outlook",
                };
                Account {
                    name: name.clone(),
                    made: Mutex::new(None),
                    make: Box::new(move || backend_of(&account)),
                    kind,
                }
            })
            .collect();
        Self { accounts }
    }

    /// Mailboxes that are already backends, for a test.
    #[cfg(test)]
    pub(crate) fn with(backends: Vec<(&str, &'static str, Arc<dyn Backend>)>) -> Self {
        let accounts = backends
            .into_iter()
            .map(|(name, kind, backend)| Account {
                name: name.to_owned(),
                made: Mutex::new(Some(backend)),
                make: Box::new(|| Err(MailError::Bad("made by the test".to_owned()))),
                kind,
            })
            .collect();
        Self { accounts }
    }

    pub(crate) fn is_empty(&self) -> bool {
        self.accounts.is_empty()
    }

    /// The accounts' names and backends.
    pub(crate) fn list(&self) -> Vec<(&str, &'static str)> {
        self.accounts
            .iter()
            .map(|account| (account.name.as_str(), account.kind))
            .collect()
    }

    /// The names of the accounts, in the file's order.
    pub(crate) fn names(&self) -> Vec<&str> {
        self.accounts
            .iter()
            .map(|account| account.name.as_str())
            .collect()
    }

    /// The backend of `name`.
    ///
    /// # Errors
    ///
    /// No account has that name, or its backend cannot be made (a password variable that is not set).
    pub(crate) fn backend(&self, name: &str) -> Result<Arc<dyn Backend>, MailError> {
        let account = self
            .accounts
            .iter()
            .find(|account| account.name == name)
            .ok_or_else(|| {
                MailError::Bad(format!(
                    "no mail account is named {name}: the accounts are {}",
                    self.names().join(", ")
                ))
            })?;
        let mut made = account.made.lock().unwrap_or_else(PoisonError::into_inner);
        if let Some(backend) = made.as_ref() {
            return Ok(Arc::clone(backend));
        }
        let backend = (account.make)()?;
        *made = Some(Arc::clone(&backend));
        Ok(backend)
    }

    /// The account a call means: the one it names, else the only one there is.
    ///
    /// # Errors
    ///
    /// None is named and there are several, or as [`Mailboxes::backend`].
    pub(crate) fn pick(
        &self,
        named: Option<&str>,
    ) -> Result<(String, Arc<dyn Backend>), MailError> {
        let name = match (named, self.accounts.as_slice()) {
            (Some(name), _) => name.to_owned(),
            (None, [only]) => only.name.clone(),
            (None, _) => {
                return Err(MailError::Bad(format!(
                    "name the account: {}",
                    self.names().join(", ")
                )));
            }
        };
        let backend = self.backend(&name)?;
        Ok((name, backend))
    }
}

/// A connector signed in to `standin` with Basic, for tests of the tools above it.
#[cfg(test)]
pub(crate) fn ews_for_tests(standin: &standin::Standin, password: &str) -> Ews {
    Ews::new(
        EwsSettings {
            email: Some("me@example.test".to_owned()),
            server: Some(standin.server()),
            autodiscover: None,
            auth: Auth::Basic {
                user: "EXAMPLE\\me".to_owned(),
                password: Secret::new(password),
            },
            timeout: Duration::from_secs(10),
        },
        tests::direct(),
    )
}

/// The backend an account asks for.
fn backend_of(account: &MailAccount) -> Result<Arc<dyn Backend>, MailError> {
    let timeout = Duration::from_secs(account.timeout_secs);
    match account.backend {
        MailBackend::Ews => {
            let auth = match account.auth {
                MailAuth::Windows => Auth::Windows,
                MailAuth::Basic => {
                    let variable = account.password_env.as_deref().unwrap_or_default();
                    let password = std::env::var(variable)
                        .ok()
                        .filter(|password| !password.is_empty())
                        .ok_or_else(|| {
                            MailError::Auth(format!(
                                "the variable {variable} holds no password: set it in the environment the host runs in"
                            ))
                        })?;
                    Auth::Basic {
                        user: account.user.clone().unwrap_or_default(),
                        password: Secret::new(password),
                    }
                }
            };
            Ok(Arc::new(Ews::new(
                EwsSettings {
                    email: account.email.clone(),
                    server: account.server.clone(),
                    autodiscover: account.autodiscover.clone(),
                    auth,
                    timeout,
                },
                Network::current(),
            )))
        }
        MailBackend::Outlook => Err(MailError::Bad(
            "the Outlook backend is not in this build".to_owned(),
        )),
    }
}
