//! An account's plan limits, read with the account's own credential and no session: the harness's account provider is given the
//! account's grant and answers the windows the plan has used. Claude and Codex read; Grok and Pi answer that they offer none.
//!
//! The credential leaves the host in one request: the account provider's own, to the provider's own endpoint (Claude's OAuth
//! API, the ChatGPT backend or the account's `chatgpt_base_url`). A login whose access token has expired is renewed through the
//! account provider's `renew`, stored as any renewal is, and read once more.

use alloc::string::{String, ToString};

use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Secret, Status};
use coqui_acp_api::{
    AccountGrant, AccountKeeper, AccountProvider, AccountRenewal, AccountUsage, AccountUsageQuery,
    CacheLifetime, ProcessSpawner, RenewRequest, SecretBytes, UsageReport,
};
use coqui_acp_harness::LocalSpawner;

use crate::accounts::NoAccounts;
use crate::config::AccountConfig;
use crate::kind;

/// Where each reader sends its request in place of the provider's own address, for a test's stand-in: the account provider's
/// `usage_endpoint`. The credential is sent there.
#[derive(Clone, Debug, Default)]
pub(crate) struct Endpoints {
    pub(crate) claude: Option<String>,
    pub(crate) codex: Option<String>,
}

impl Endpoints {
    /// The `usage_endpoint` an account provider of `harness` is configured with, when one is named.
    pub(crate) fn of(&self, harness: &str) -> Option<&str> {
        match harness {
            "claude" => self.claude.as_deref(),
            "codex" => self.codex.as_deref(),
            _ => None,
        }
    }
}

/// The limits of the account `name`, read by the account provider of its harness, or `None` for a harness with no account
/// provider (the caller then asks the provider's harness).
pub(super) fn read(
    built: &kind::Harness,
    name: &str,
    account: &AccountConfig,
) -> Option<Result<UsageReport, String>> {
    let provider = built.account_provider().ok()?;
    Some(read_account(built, provider, name, account))
}

fn read_account(
    built: &kind::Harness,
    provider: &alloc::sync::Arc<dyn AccountProvider>,
    name: &str,
    account: &AccountConfig,
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
    if account.kind == coqui_acp_api::HARNESS_LOGIN {
        return Ok(stamp(UsageReport::unsupported(
            "the login a program has where it runs is not one this host reads limits with",
        )));
    }
    let grant = built
        .read_account(name)
        .map_err(|why| why.client_reason())?;
    let report = match ask(provider, &grant) {
        Ok(report) => report,
        Err(error) if error.status == Status::GUATIAO_ERR_GONE => {
            let was = why(&error);
            renew(built, provider, name)
                .map_err(|failed| format!("{was}; renewing it failed: {failed}"))?;
            let grant = built
                .read_account(name)
                .map_err(|why| why.client_reason())?;
            ask(provider, &grant).map_err(|error| why(&error))?
        }
        Err(error) => return Err(why(&error)),
    };
    Ok(stamp(report))
}

/// What the account provider reports for `grant`: its plan's windows, or that it offers none for the kind.
fn ask(
    provider: &alloc::sync::Arc<dyn AccountProvider>,
    grant: &AccountGrant,
) -> Result<UsageReport, ProviderError> {
    let spawner = match &grant.state_dir {
        Some(dir) => LocalSpawner::new().files(dir),
        None => LocalSpawner::new(),
    };
    let usage = provider.usage(
        AccountUsageQuery::new(grant.clone()),
        spawner.into_object(),
        NoAccounts.into_object(),
    )?;
    Ok(report_of(usage))
}

fn report_of(usage: AccountUsage) -> UsageReport {
    let mut report = UsageReport::new();
    report.supported = usage.supported;
    report.unsupported_because = usage.unsupported_because;
    report.sources = usage.sources;
    report.account = usage.account;
    report.revision = usage.revision;
    report.observed_at = usage.observed_at;
    report.buckets = usage.buckets;
    report
}

fn why(error: &ProviderError) -> String {
    let message = error.message();
    if message.is_empty() {
        "the provider gave no reason".to_string()
    } else {
        message.to_string()
    }
}

/// Where a login lives in the account's directory, for the kinds whose provider renews it by looking at that directory: the
/// credential holding the file's text, and the file's name.
fn login_file(harness: &str, kind: &str) -> Option<(&'static str, &'static str)> {
    use canoa_claude::acp::accounts as claude;
    use canoa_codex::acp::accounts as codex;
    match (harness, kind) {
        ("claude", claude::LOGIN | claude::CONSOLE) => {
            Some((claude::FILE_KEY, claude::CREDENTIALS_FILE))
        }
        ("codex", codex::CHATGPT) => Some((codex::FILE_KEY, codex::AUTH_FILE)),
        _ => None,
    }
}

/// Renews the login through the account provider and stores what it made. It holds the account while it does, so a connection
/// that holds it (and watches it itself) refuses this. The account provider looks at the account's directory and never fills it
/// from the grant, so the stored login is written there first, and goes when the account is given back.
fn renew(
    built: &kind::Harness,
    provider: &alloc::sync::Arc<dyn AccountProvider>,
    name: &str,
) -> Result<(), String> {
    let held = built
        .hold_account(name, "usage", false)
        .map_err(|refusal| refusal.client_reason())?;
    let grant = held.grant();
    if let Some((key, file)) = login_file(held.owner(), &grant.kind) {
        let text = grant
            .credentials
            .expose()
            .get(key)
            .ok_or_else(|| "the account holds no login to renew".to_string())?;
        let files: Object<dyn ProcessSpawner> = held.spawner(|_| {});
        files
            .write_file(
                file,
                Secret::new(SecretBytes::new(text.as_bytes().to_vec())),
                true,
            )
            .map_err(|error| why(&error))?;
    }
    let connection = built.connection(None, Some(&held), CacheLifetime::Default);
    let report = provider
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
