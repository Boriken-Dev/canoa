//! An account provider whose calls for an account leave by that account's network route.
//!
//! A provider reaches the network through [`Network::current`]; it never names a route and never sees one. The host runs each call
//! that names an account with that account's network as this thread's own ([`Network::within`]), so what the provider asks of the
//! service leaves by the route the person attached to the account, and by the host's when it has none. A route that does not open
//! refuses the call with the route's words: the account's traffic never leaves by a path the person did not name.

use alloc::sync::Arc;

use canoa::network::Network;
use coqui_acp_api::guatiao::library::{Object, ProviderError};
use coqui_acp_api::{
    AccountKeeper, AccountModelsQuery, AccountOffer, AccountProvider, AccountProviderInfo,
    AccountRef, AccountUsage, AccountUsageQuery, AccountWatch, LoginOutcome, LoginPrompter,
    LoginRequest, ModelListing, ProcessSpawner, RenewReport, RenewRequest, SignedInCheck,
    SignedInQuery, WatchRequest,
};

/// `inner`, with the calls that name an account run on that account's route.
pub(crate) fn wrap(inner: Arc<dyn AccountProvider>) -> Arc<dyn AccountProvider> {
    Arc::new(Routed { inner })
}

struct Routed {
    inner: Arc<dyn AccountProvider>,
}

/// Runs `call` as the account `id` leaves the host.
fn on_route<T>(
    id: &str,
    call: impl FnOnce() -> Result<T, ProviderError>,
) -> Result<T, ProviderError> {
    match crate::network::routes::account_network(id) {
        Ok(Some(network)) => Network::within(network, call),
        Ok(None) => call(),
        Err(failure) => Err(canoa::status::unavailable(failure.to_string())),
    }
}

impl AccountProvider for Routed {
    fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
        self.inner.describe()
    }

    fn offers(&self) -> Result<AccountOffer, ProviderError> {
        self.inner.offers()
    }

    fn provides(&self, account: AccountRef) -> Result<Vec<String>, ProviderError> {
        self.inner.provides(account)
    }

    fn login(
        &self,
        request: LoginRequest,
        spawner: Object<dyn ProcessSpawner>,
        prompter: Object<dyn LoginPrompter>,
    ) -> Result<LoginOutcome, ProviderError> {
        self.inner.login(request, spawner, prompter)
    }

    fn renew(
        &self,
        request: RenewRequest,
        spawner: Object<dyn ProcessSpawner>,
    ) -> Result<RenewReport, ProviderError> {
        let id = request.account.id.clone();
        on_route(&id, || self.inner.renew(request, spawner))
    }

    fn watch(
        &self,
        request: WatchRequest,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        let id = request.account.id.clone();
        on_route(&id, || self.inner.watch(request, spawner, keeper))
    }

    fn usage(
        &self,
        query: AccountUsageQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        let id = query.account.id.clone();
        on_route(&id, || self.inner.usage(query, spawner, keeper))
    }

    fn models(
        &self,
        query: AccountModelsQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<ModelListing, ProviderError> {
        let id = query.account.id.clone();
        on_route(&id, || self.inner.models(query, spawner, keeper))
    }

    fn signed_in(&self, query: SignedInQuery) -> Result<SignedInCheck, ProviderError> {
        self.inner.signed_in(query)
    }
}
