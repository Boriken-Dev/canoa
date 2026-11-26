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
    wrap_with(inner, Arc::new(crate::network::routes::account_network))
}

/// What says which network an account's calls run on: `None` for an account with no route of its own.
type Resolve = Arc<
    dyn Fn(&str) -> Result<Option<Arc<Network>>, crate::network::routes::RouteFailure>
        + Send
        + Sync,
>;

/// [`wrap`] over `resolve`, which a test makes its own.
fn wrap_with(inner: Arc<dyn AccountProvider>, resolve: Resolve) -> Arc<dyn AccountProvider> {
    Arc::new(Routed { inner, resolve })
}

struct Routed {
    inner: Arc<dyn AccountProvider>,
    resolve: Resolve,
}

/// Runs `call` as the account `id` leaves the host.
fn on_route<T>(
    resolve: &Resolve,
    id: &str,
    call: impl FnOnce() -> Result<T, ProviderError>,
) -> Result<T, ProviderError> {
    match resolve(id) {
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
        on_route(&self.resolve, &id, || self.inner.renew(request, spawner))
    }

    fn watch(
        &self,
        request: WatchRequest,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<Object<dyn AccountWatch>, ProviderError> {
        let id = request.account.id.clone();
        on_route(&self.resolve, &id, || {
            self.inner.watch(request, spawner, keeper)
        })
    }

    fn usage(
        &self,
        query: AccountUsageQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<AccountUsage, ProviderError> {
        let id = query.account.id.clone();
        on_route(&self.resolve, &id, || {
            self.inner.usage(query, spawner, keeper)
        })
    }

    fn models(
        &self,
        query: AccountModelsQuery,
        spawner: Object<dyn ProcessSpawner>,
        keeper: Object<dyn AccountKeeper>,
    ) -> Result<ModelListing, ProviderError> {
        let id = query.account.id.clone();
        on_route(&self.resolve, &id, || {
            self.inner.models(query, spawner, keeper)
        })
    }

    fn signed_in(&self, query: SignedInQuery) -> Result<SignedInCheck, ProviderError> {
        self.inner.signed_in(query)
    }
}
#[cfg(test)]
mod tests {
    use std::sync::Mutex;

    use coqui_acp_api::AccountGrant;
    use coqui_acp_api::guatiao::library::Object;
    use coqui_acp_harness::LocalSpawner;

    use super::*;
    use crate::accounts::NoAccounts;
    use crate::network::routes::{FailureKind, RouteFailure};

    /// A provider that says which network it was asked on: the address of the one `Network::current()` answered when it listed models.
    struct Asking {
        seen: Mutex<Vec<usize>>,
    }

    fn refused() -> ProviderError {
        canoa::status::unsupported("a stand-in")
    }

    impl AccountProvider for Asking {
        fn describe(&self) -> Result<AccountProviderInfo, ProviderError> {
            Err(refused())
        }
        fn offers(&self) -> Result<AccountOffer, ProviderError> {
            Err(refused())
        }
        fn provides(&self, _: AccountRef) -> Result<Vec<String>, ProviderError> {
            Err(refused())
        }
        fn login(
            &self,
            _: LoginRequest,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn LoginPrompter>,
        ) -> Result<LoginOutcome, ProviderError> {
            Err(refused())
        }
        fn renew(
            &self,
            _: RenewRequest,
            _: Object<dyn ProcessSpawner>,
        ) -> Result<RenewReport, ProviderError> {
            Err(refused())
        }
        fn watch(
            &self,
            _: WatchRequest,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<Object<dyn AccountWatch>, ProviderError> {
            Err(refused())
        }
        fn usage(
            &self,
            _: AccountUsageQuery,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<AccountUsage, ProviderError> {
            Err(refused())
        }
        fn models(
            &self,
            _: AccountModelsQuery,
            _: Object<dyn ProcessSpawner>,
            _: Object<dyn AccountKeeper>,
        ) -> Result<ModelListing, ProviderError> {
            self.seen
                .lock()
                .expect("a lock")
                .push(Arc::as_ptr(&Network::current()) as usize);
            Ok(ModelListing::unknown("a stand-in"))
        }
        fn signed_in(&self, _: SignedInQuery) -> Result<SignedInCheck, ProviderError> {
            Err(refused())
        }
    }

    fn models(
        provider: &Arc<dyn AccountProvider>,
        id: &str,
    ) -> Result<ModelListing, ProviderError> {
        provider.models(
            AccountModelsQuery::new(AccountGrant::new(id, "k")),
            LocalSpawner::new().into_object(),
            NoAccounts.into_object(),
        )
    }

    #[test]
    fn an_accounts_calls_run_on_its_route_and_another_accounts_on_the_hosts() {
        let route = Arc::new(Network::system());
        let held = Arc::clone(&route);
        let inner = Arc::new(Asking {
            seen: Mutex::new(Vec::new()),
        });
        let routed = wrap_with(
            Arc::clone(&inner) as Arc<dyn AccountProvider>,
            Arc::new(move |id| Ok((id == "with-route").then(|| Arc::clone(&held)))),
        );
        models(&routed, "with-route").expect("answers");
        models(&routed, "without").expect("answers");
        let seen = inner.seen.lock().expect("a lock").clone();
        assert_eq!(seen[0], Arc::as_ptr(&route) as usize, "the account's route");
        assert_eq!(
            seen[1],
            Arc::as_ptr(&Network::current()) as usize,
            "the host's route"
        );
        assert_ne!(seen[0], seen[1]);
    }

    #[test]
    fn a_route_that_does_not_open_refuses_the_call_with_its_words_and_runs_nothing() {
        let inner = Arc::new(Asking {
            seen: Mutex::new(Vec::new()),
        });
        let routed = wrap_with(
            Arc::clone(&inner) as Arc<dyn AccountProvider>,
            Arc::new(|_| {
                Err(RouteFailure {
                    route: "corp".to_owned(),
                    kind: FailureKind::Unreachable,
                    why: "its program did not start".to_owned(),
                })
            }),
        );
        let error = models(&routed, "any").expect_err("refused");
        assert!(
            error.message().contains("corp") && error.message().contains("did not start"),
            "{}",
            error.message()
        );
        assert!(inner.seen.lock().expect("a lock").is_empty());
    }

    #[test]
    fn within_makes_a_network_this_threads_own_and_gives_it_back() {
        let other = Arc::new(Network::system());
        let before = Network::current();
        let seen = Network::within(Arc::clone(&other), || Arc::as_ptr(&Network::current()));
        assert_eq!(seen, Arc::as_ptr(&other));
        assert!(Arc::ptr_eq(&Network::current(), &before));
        // A thread the call starts has the process's network.
        let from_thread = Network::within(Arc::clone(&other), || {
            std::thread::spawn(|| Arc::as_ptr(&Network::current()) as usize)
                .join()
                .expect("a thread")
        });
        assert_eq!(from_thread, Arc::as_ptr(&before) as usize);
    }
}
