//! The account makers the libraries this host carries offer: `account_provider`s that no harness owns. An agent's library offers
//! `<harness>_accounts`, which signs in to that agent's accounts; a maker is offered by a library that is no agent (a gateway, a
//! proxy, a model service) and makes accounts of kinds of its own, each saying the standards it provides. Nothing here names a
//! vendor or a standard.
//!
//! The host joins a maker's kinds and sign-in methods to the listing of each harness that accepts a standard the kind provides, so an
//! account is made through any of them and run by every harness whose accepted standards meet the kind's, and it asks the maker, never
//! the harness, for the account's usage and models.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a static and a thread-local are std's"
)]

use std::sync::{Arc, OnceLock};

use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AccountCatalog, AccountKindInfo, AccountOffer, AccountProvider, AcpHarness};

/// The suffix of an account provider's id: `<harness>_accounts` serves the accounts of harness `<harness>`.
pub(crate) const ACCOUNTS_SUFFIX: &str = "_accounts";

/// One account provider that no harness owns, and what it offers.
#[derive(Clone)]
pub(crate) struct Maker {
    /// The id of the account provider.
    pub(crate) provider: String,
    /// The kinds it makes and the ways to sign in to them.
    pub(crate) accounts: AccountOffer,
    maker: Arc<dyn AccountProvider>,
}

impl Maker {
    /// A maker with the id `provider`, whose `maker` offers `accounts`.
    pub(crate) fn new(
        provider: impl Into<String>,
        accounts: AccountOffer,
        maker: Arc<dyn AccountProvider>,
    ) -> Self {
        Self {
            provider: provider.into(),
            accounts,
            maker,
        }
    }

    /// The provider, to sign in, renew and read usage and models with.
    pub(crate) fn provider(&self) -> Arc<dyn AccountProvider> {
        Arc::clone(&self.maker)
    }

    /// The kind `id`, when this maker makes it.
    pub(crate) fn kind(&self, id: &str) -> Option<&AccountKindInfo> {
        self.accounts.kind(id)
    }
}

/// The makers `registry` offers: its account providers that are not `<harness>_accounts` of an `acp_harness` it also offers.
pub(crate) fn makers_in(registry: &Registry) -> Vec<Maker> {
    let harnesses: Vec<String> = registry
        .offers::<dyn AcpHarness>()
        .map(|offer| offer.id().to_owned())
        .collect();
    registry
        .offers::<dyn AccountProvider>()
        .filter(|offer| {
            let owner = offer.id().strip_suffix(ACCOUNTS_SUFFIX);
            !owner.is_some_and(|harness| harnesses.iter().any(|each| each == harness))
        })
        .filter_map(|offer| {
            let id = offer.id().to_owned();
            let provider = offer.shared()?;
            let accounts = provider.offers().ok()?;
            Some(Maker::new(id, accounts, provider))
        })
        .collect()
}

/// The makers the bundled libraries offer, found once.
pub(crate) fn offered() -> &'static [Maker] {
    static OFFERED: OnceLock<Vec<Maker>> = OnceLock::new();
    OFFERED.get_or_init(|| {
        crate::vendors::bundled_providers()
            .map(makers_in)
            .unwrap_or_default()
    })
}

/// Every maker in force: the bundled ones, and in a test the stand-ins the test's thread named.
fn all() -> Vec<Maker> {
    #[cfg(test)]
    let mut found = stand_ins::current();
    #[cfg(not(test))]
    let mut found = Vec::new();
    found.extend(offered().iter().cloned());
    found
}

/// The maker that makes accounts of `kind`.
pub(crate) fn for_kind(kind: &str) -> Option<Maker> {
    all().into_iter().find(|maker| maker.kind(kind).is_some())
}

/// The id of each account provider that is a maker: no agent is theirs.
pub(crate) fn ids() -> Vec<String> {
    all().into_iter().map(|maker| maker.provider).collect()
}

/// The maker whose sign-in method is `method`.
pub(crate) fn login_provider(method: &str) -> Option<Arc<dyn AccountProvider>> {
    all()
        .into_iter()
        .find(|maker| {
            maker
                .accounts
                .logins
                .iter()
                .any(|login| login.id == method && maker.kind(&login.produces).is_some())
        })
        .map(|maker| maker.provider())
}

/// Adds the kinds and sign-in methods the makers offer to `listing`, the harness `harness`'s own: a kind a gateway serves joins when
/// the harness can be given a gateway, any other when the harness accepts a standard the kind provides (any it accepts, for a kind whose accounts name their own). `catalog` is what the
/// harness accepts; a harness that lists none takes none.
pub(crate) fn extend(
    listing: &mut AccountCatalog,
    harness: &str,
    catalog: Option<&AccountCatalog>,
) {
    for maker in all() {
        for kind in &maker.accounts.kinds {
            if listing.kinds.iter().any(|each| each.id == kind.id) {
                continue;
            }
            let joins = if crate::gateways::for_kind(&kind.id).is_some() {
                crate::vendors::gateway_route(harness).is_some()
            } else {
                // A kind whose accounts name their own standards is open to any harness that accepts one, whichever it is.
                let own = crate::accounts::standards::property(kind).is_some();
                catalog.is_some_and(|catalog| {
                    catalog
                        .accepts
                        .iter()
                        .any(|accepted| own || kind.provides.contains(&accepted.standard))
                })
            };
            if !joins {
                continue;
            }
            listing.kinds.push(kind.clone());
            listing.logins.extend(
                maker
                    .accounts
                    .logins
                    .iter()
                    .filter(|method| method.produces == kind.id)
                    .cloned(),
            );
        }
    }
}

/// Whether the kind `kind` is made by a maker, not by a harness's own account provider.
pub(crate) fn makes(kind: &str) -> bool {
    for_kind(kind).is_some()
}

#[cfg(test)]
mod litellm_tests;
#[cfg(test)]
mod tests;

#[cfg(test)]
pub(crate) mod stand_ins {
    //! Makers a test names for its own thread, which the host then treats as it does the bundled libraries'.
    use std::cell::RefCell;

    use super::Maker;

    std::thread_local! {
        static NAMED: RefCell<Vec<Maker>> = const { RefCell::new(Vec::new()) };
    }

    /// The makers named for this thread.
    pub(super) fn current() -> Vec<Maker> {
        NAMED.with(|named| named.borrow().clone())
    }

    /// Names `makers` for this thread until the guard drops.
    pub(crate) fn serving(makers: Vec<Maker>) -> Guard {
        NAMED.with(|named| *named.borrow_mut() = makers);
        Guard
    }

    /// Takes the named makers away when it drops.
    pub(crate) struct Guard;

    impl Drop for Guard {
        fn drop(&mut self) {
            NAMED.with(|named| named.borrow_mut().clear());
        }
    }
}
