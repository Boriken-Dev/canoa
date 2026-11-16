//! An account's plan limits, read with the account's own credential and no session: the provider's account-side reader is given
//! the account's grant and answers the windows the plan has used. Claude and Codex read; Pi answers that it offers none.
//!
//! The credential leaves the host in one request: the reader's own, to the provider's own endpoint (Claude's OAuth API, the
//! ChatGPT backend or the account's `chatgpt_base_url`). A login whose access token has expired is renewed through the
//! provider's `renew`, stored as any renewal is, and read once more.

use alloc::string::{String, ToString};

use coqui_acp_api::guatiao::Status;
use coqui_acp_api::guatiao::library::ProviderError;
use coqui_acp_api::{AccountGrant, AccountRenewal, CacheLifetime, RenewRequest, UsageReport};

use crate::config::AccountConfig;
use crate::kind;

/// Where each reader sends its request in place of the provider's own address, for a test's stand-in. The credential is
/// sent there.
#[derive(Clone, Debug, Default)]
pub(crate) struct Endpoints {
    pub(crate) claude: Option<String>,
    pub(crate) codex: Option<String>,
}

/// The limits of the account `name` of `harness`, or `None` for a provider with no account-side reader (the caller then asks the
/// provider's harness).
pub(super) fn read(
    built: &kind::Harness,
    harness: &str,
    name: &str,
    account: &AccountConfig,
    endpoints: &Endpoints,
) -> Option<Result<UsageReport, String>> {
    matches!(harness, "claude" | "codex" | "pi")
        .then(|| read_account(built, harness, name, account, endpoints))
}

fn read_account(
    built: &kind::Harness,
    harness: &str,
    name: &str,
    account: &AccountConfig,
    endpoints: &Endpoints,
) -> Result<UsageReport, String> {
    let stamp = |mut report: UsageReport| {
        report.account.get_or_insert_with(|| account.id.clone());
        if report.observed_at == 0 {
            report.observed_at = crate::usage_splice::unix_now();
        }
        report
    };
    if account.kind == canoa_foundry::account::KIND {
        return Ok(stamp(UsageReport::unsupported(
            "a model that runs on this computer has no plan limits",
        )));
    }
    let grant = built
        .read_account(name)
        .map_err(|why| why.client_reason())?;
    let report = match ask(harness, &grant, endpoints) {
        Ok(report) => report,
        Err(error) if error.status == Status::GUATIAO_ERR_GONE => {
            let was = why(&error);
            renew(built, name).map_err(|failed| format!("{was}; renewing it failed: {failed}"))?;
            let grant = built
                .read_account(name)
                .map_err(|why| why.client_reason())?;
            ask(harness, &grant, endpoints).map_err(|error| why(&error))?
        }
        Err(error) => return Err(why(&error)),
    };
    Ok(stamp(report))
}

fn ask(
    harness: &str,
    grant: &AccountGrant,
    endpoints: &Endpoints,
) -> Result<UsageReport, ProviderError> {
    match harness {
        "claude" => {
            let mut options = canoa_claude::acp::account_usage::ReadOptions::new();
            if let Some(endpoint) = &endpoints.claude {
                options = options.endpoint(endpoint);
            }
            canoa_claude::acp::account_usage::read_limits(grant, &options)
        }
        "codex" => {
            let mut options = canoa_codex::acp::account_usage::ReadOptions::new();
            if let Some(endpoint) = &endpoints.codex {
                options = options.endpoint(endpoint);
            }
            canoa_codex::acp::account_usage::read_limits(grant, &options)
        }
        _ => canoa_pi::acp::account_usage::read_limits(grant),
    }
}

fn why(error: &ProviderError) -> String {
    let message = error.message();
    if message.is_empty() {
        "the provider gave no reason".to_string()
    } else {
        message.to_string()
    }
}

/// Renews the login through the provider and stores what it made. It holds the account while it does, so a connection that
/// holds it (and renews it itself) refuses this.
fn renew(built: &kind::Harness, name: &str) -> Result<(), String> {
    let held = built
        .hold_account(name, "usage", false)
        .map_err(|refusal| refusal.client_reason())?;
    let connection = built.connection(None, Some(&held), CacheLifetime::Default);
    let grant = held.grant();
    let report = connection
        .harness
        .renew(RenewRequest::new(grant.clone()), connection.spawner)
        .map_err(|error| why(&error))?;
    let Some(credentials) = report.credentials else {
        return Err(report
            .not_renewed_because
            .unwrap_or_else(|| "nothing was renewed".to_string()));
    };
    let mut renewal = AccountRenewal::new(
        grant.id.clone(),
        grant.revision,
        1,
        credentials.expose().clone(),
    );
    renewal.expires_at = report.expires_at;
    connection
        .keeper
        .renewed(renewal)
        .map_err(|error| why(&error))
}

#[cfg(test)]
mod tests;
