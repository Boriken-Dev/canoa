//! The `mail` notifier: a notice the host tells the person about, sent by mail through one of the accounts of `[host.services.mail]` to
//! the addresses the notifier's entry lists, any addresses. It is the only thing in this library that sends: the tools an agent is
//! given save drafts and nothing more, and this provider is not reachable from them.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "the hosts of a process are kept behind a std lock"
)]

use std::sync::Arc;

use crate::mail_backend::{Draft, MailError};
use canoa::notifier::{Delivery, Notice, Notifier, NotifierError, NotifierInfo, SendRequest};
use canoa::session_service::Context;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::guatiao::{FromValue, Provider, Schema, Status, ToValue};
use guatiao_intake::Form;
use serde_json::Value;

use crate::config::{MailConfig, SERVICE};
use crate::mailboxes::Mailboxes;

/// The key under which the host tells a notifier the handle of its session services.
const SERVICES_KEY: &str = "_services";

/// What the notifier's entry holds.
#[derive(ToValue, FromValue, Schema, Form)]
#[map(crate = ::coqui_acp_api::guatiao)]
#[allow(dead_code, reason = "only its description is read")]
struct MailNotifierSettings {
    /// A name in `[host.services.mail.accounts]`.
    #[schema(title = "Mail account")]
    account: String,
    /// Where notices go, as addresses.
    #[schema(title = "Send to")]
    to: Vec<String>,
}

/// Sends notices by mail.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(Notifier, id = "mail_notifier", crate = ::coqui_acp_api::guatiao)]
pub struct MailNotifier;

/// Why `address` cannot be a recipient, or `None` when it can: one word with an `@` between two non-empty parts and nothing that
/// would end the line or add a header.
fn address_problem(address: &str) -> Option<&'static str> {
    if address
        .chars()
        .any(|c| c.is_whitespace() || c.is_control() || matches!(c, '<' | '>' | ',' | ';'))
    {
        return Some("holds a space, a control character or a separator");
    }
    match address.split_once('@') {
        Some((local, domain))
            if !local.is_empty() && !domain.is_empty() && !domain.contains('@') =>
        {
            None
        }
        _ => Some("is not a mail address"),
    }
}

/// The addresses of the settings: a list of text, or one text with the addresses divided by commas, semicolons or lines.
fn addresses_of(settings: &Value) -> Result<Vec<String>, NotifierError> {
    let listed: Vec<String> = match settings.get("to") {
        Some(Value::Array(items)) => items
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
        Some(Value::String(text)) => text.split([',', ';', '\n']).map(str::to_owned).collect(),
        _ => Vec::new(),
    };
    let addresses: Vec<String> = listed
        .into_iter()
        .map(|address| address.trim().to_owned())
        .filter(|address| !address.is_empty())
        .collect();
    if addresses.is_empty() {
        return Err(NotifierError::Bad("to names no address".to_owned()));
    }
    if let Some(bad) = addresses
        .iter()
        .find_map(|address| address_problem(address).map(|why| format!("an address in to {why}")))
    {
        return Err(NotifierError::Bad(bad));
    }
    Ok(addresses)
}

fn failure(error: ProviderError) -> NotifierError {
    if error.status == Status::GUATIAO_ERR_UNSUPPORTED {
        return NotifierError::Unavailable("this mail backend cannot send".to_owned());
    }
    match MailError::from(error) {
        MailError::Bad(why) => NotifierError::Bad(why),
        MailError::Auth(why) => NotifierError::Auth(why),
        MailError::Unreachable(why) => NotifierError::Unavailable(why),
        MailError::Backend(why) => NotifierError::Rejected(why),
    }
}

/// Sends `notice` through the account the settings name, to the addresses they list.
fn send_through(
    mailboxes: &Mailboxes,
    settings: &Value,
    notice: &Notice,
) -> Result<(), NotifierError> {
    let account = settings
        .get("account")
        .and_then(Value::as_str)
        .filter(|account| !account.is_empty())
        .ok_or_else(|| NotifierError::Bad("account names no mail account".to_owned()))?;
    let to = addresses_of(settings)?;
    let mailbox = mailboxes.backend(account).map_err(|error| match error {
        MailError::Bad(why) => NotifierError::Bad(why),
        other => failure(other.into()),
    })?;
    let subject = format!("{}: {}", notice.title, notice.body);
    mailbox
        .send_message(Draft::new_message(
            to,
            Vec::new(),
            Vec::new(),
            subject,
            format!("{}\n", notice.body),
        ))
        .map_err(failure)
}

impl Notifier for MailNotifier {
    fn describe(&self) -> Result<NotifierInfo, ProviderError> {
        let mut info = NotifierInfo::new("mail", "Mail").taking::<MailNotifierSettings>()?;
        info.help = Some(
            "Sends each notice by mail from one of your mail accounts to the addresses you list. A notice names the session and says what happened, never a command or a path; the mail passes through your mail provider."
                .to_owned(),
        );
        Ok(info)
    }

    fn send(&self, request: SendRequest) -> Result<Delivery, ProviderError> {
        let settings: Value = serde_json::from_str(&request.settings)
            .map_err(|_| NotifierError::Bad("the settings are not JSON".to_owned()))?;
        let handle = settings
            .get(SERVICES_KEY)
            .and_then(Value::as_str)
            .ok_or_else(|| NotifierError::Unavailable("no mail service is running".to_owned()))?;
        let context = Context::new(handle);
        let host = canoa::session_service::host(&context)
            .ok_or_else(|| NotifierError::Unavailable("this host is gone".to_owned()))?;
        let config = MailConfig::parse(&host.config(SERVICE).map_err(NotifierError::Bad)?)
            .map_err(NotifierError::Bad)?;
        let mailboxes =
            Mailboxes::from_config(&config, host, handle, Arc::clone(crate::service::opened()));
        send_through(&mailboxes, &settings, &request.notice)?;
        Ok(Delivery::new(request.notice.id))
    }
}

#[cfg(test)]
#[path = "notifier_tests.rs"]
mod tests;
