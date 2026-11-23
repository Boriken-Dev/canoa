//! The account provider of a LiteLLM proxy.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a deadline is std's"
)]

use std::time::{Duration, Instant};

use canoa::api_gateway::{key_form, key_of, openai_models, value};
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::guatiao::{Provider, Status};
use coqui_acp_api::{
    AccountGrant, AccountKeeper, AccountModelsQuery, AccountOffer, AccountProvider,
    AccountProviderInfo, AccountRef, AccountUsage, AccountUsageQuery, AccountWatch, Credentials,
    LoginOutcome, LoginPrompter, LoginRequest, ModelListing, ProcessSpawner, RenewReport,
    RenewRequest, WatchRequest,
};
use serde_json::{Map, Value};

use crate::account::{self, API_KEY, CLIENT_ID, KIND, REFRESH_TOKEN, SET_KEY, SIGN_IN, Settings};
use crate::{sso, usage};

fn refused(why: impl AsRef<str>) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_BAD_VALUE, why.as_ref())
}

fn unsupported(why: &str) -> ProviderError {
    ProviderError::new(Status::GUATIAO_ERR_UNSUPPORTED, why)
}

/// The proxy's address and the key of `grant`, read as an account of this kind.
fn proxy_of(grant: &AccountGrant) -> Result<(String, String), ProviderError> {
    grant.check_id()?;
    if grant.kind != KIND {
        return Err(refused("this provider keeps no account of that kind"));
    }
    let settings = guatiao_serde::adapt::from_value::<Map<String, Value>>(&grant.settings)
        .map_err(|_| refused("the account's settings are not a table"))?;
    let base = Settings::from_map(&settings)
        .map_err(|why| refused(why.to_string()))?
        .base_url;
    let key = grant
        .credentials
        .expose()
        .get(API_KEY)
        .ok_or_else(|| refused("the account holds no key"))?
        .to_owned();
    Ok((base, key))
}

/// A LiteLLM proxy as an account: a key for one address, which serves the OpenAI and Anthropic APIs.
#[derive(Default, Debug, Clone, Copy, Provider)]
#[provider(AccountProvider, id = "litellm_accounts", crate = ::coqui_acp_api::guatiao)]
pub struct LiteLlmAccounts;

impl AccountProvider for LiteLlmAccounts {
    fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
        let mut info = AccountProviderInfo::new("LiteLLM");
        info.description = "Accounts on a LiteLLM proxy: one key for the OpenAI and Anthropic APIs, made by the proxy's sign-in or by a key you were given, with the key's budget and limits.".to_owned();
        Ok(info)
    }

    fn offers(&self) -> Result<AccountOffer, ProviderError> {
        Ok(account::offer())
    }

    fn provides(&self, account: AccountRef) -> Result<Vec<String>, ProviderError> {
        account.check_id()?;
        if account.kind != KIND {
            return Err(refused("this provider keeps no account of that kind"));
        }
        // The proxy serves both APIs on the one address, whatever the account's settings say.
        Ok(account::provides())
    }

    fn login(
        &self,
        request: LoginRequest,
        spawner: Object<dyn ProcessSpawner>,
        prompter: Object<dyn LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        drop(spawner);
        if request.method != SIGN_IN && request.method != SET_KEY {
            return Err(refused("LiteLLM has no such way to sign in"));
        }
        let inputs = guatiao_serde::adapt::from_value::<Map<String, Value>>(&request.inputs)
            .map_err(|_| refused("the inputs are not a table"))?;
        let settings = Settings::from_map(&inputs).map_err(|why| refused(why.to_string()))?;
        let host = settings
            .base_url
            .split("://")
            .nth(1)
            .and_then(|rest| rest.split('/').next())
            .map(str::to_owned);
        let mut credentials = Credentials::new();
        let mut label = host.clone();
        let mut expires_at = None;
        if request.method == SET_KEY {
            let form = key_form(
                &format!(
                    "Enter the key for {}",
                    host.as_deref().unwrap_or("the proxy")
                ),
                "The virtual key the proxy gave you. It is sent to that address only.",
            );
            let answer = prompter.ask(form)?;
            credentials.insert(API_KEY, key_of(&answer)?);
        } else {
            let wait = Duration::from_millis(request.timeout_ms);
            let (tokens, client) =
                sso::sign_in(&settings.base_url, &prompter, Instant::now() + wait)?;
            credentials.insert(API_KEY, tokens.access_token);
            if let Some(refresh) = tokens.refresh_token {
                credentials.insert(REFRESH_TOKEN, refresh);
                credentials.insert(CLIENT_ID, client);
            }
            expires_at = tokens.expires_in.map(sso::expires_at);
            if tokens.user_id.is_some() {
                label = tokens.user_id;
            }
        }
        let mut outcome = LoginOutcome::new(KIND, credentials);
        outcome.settings = value(&settings.to_value());
        outcome.label = label;
        outcome.expires_at = expires_at;
        Ok(outcome)
    }

    fn renew(
        &self,
        request: RenewRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        drop(spawner);
        let (base, _) = proxy_of(&request.account)?;
        let held = request.account.credentials.expose();
        let (Some(refresh), Some(client)) = (held.get(REFRESH_TOKEN), held.get(CLIENT_ID)) else {
            return Ok(RenewReport::unchanged(
                "a key you were given has nothing to renew it with: sign in again, or set a new key",
            ));
        };
        let tokens = match sso::refresh(&base, client, refresh) {
            Ok(tokens) => tokens,
            Err(sso::RenewError::Refused(why)) => {
                return Err(ProviderError::new(
                    Status::GUATIAO_ERR_GONE,
                    &format!("{why}: sign in again"),
                ));
            }
            Err(sso::RenewError::Failed(why)) => {
                return Err(ProviderError::new(Status::GUATIAO_ERR_INTERNAL, &why));
            }
        };
        let mut credentials = Credentials::new();
        credentials.insert(API_KEY, tokens.access_token);
        // The proxy rotates the refresh token: the next one replaces the used one, and a renewal that brought none keeps it.
        credentials.insert(
            REFRESH_TOKEN,
            tokens.refresh_token.unwrap_or_else(|| refresh.to_owned()),
        );
        credentials.insert(CLIENT_ID, client);
        let mut report = RenewReport::renewed(credentials);
        report.expires_at = tokens.expires_in.map(sso::expires_at);
        Ok(report)
    }

    fn watch(
        &self,
        request: WatchRequest,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        drop((request, spawner, keeper));
        Err(unsupported(
            "a key at a proxy is in no file a program renews: it is renewed when the proxy says it expired",
        ))
    }

    fn usage(
        &self,
        query: AccountUsageQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        drop((spawner, keeper));
        let (base, key) = proxy_of(&query.account)?;
        usage::read(&base, &key, &query.account.id)
    }

    fn models(
        &self,
        query: AccountModelsQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<ModelListing, ProviderError> {
        drop((spawner, keeper));
        let (base, key) = proxy_of(&query.account)?;
        // One list serves both APIs: the proxy lists what it serves, whichever API a program asks it through.
        Ok(openai_models(&base, &key))
    }
}

coqui_acp_api::guatiao::local_providers!(
    id = "canoa-litellm",
    version = env!("CARGO_PKG_VERSION"),
    providers = [LiteLlmAccounts]
);
