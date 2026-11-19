//! The account makers the libraries this host carries offer: `account_provider`s that no harness owns. An agent's library offers
//! `<harness>_accounts`, which signs in to that agent's accounts; a maker is offered by a library that is no agent (a gateway, a
//! proxy, a model service) and makes accounts of kinds of its own, each saying the standards it provides. The libraries are the
//! bundled ones and those canoa.cfg's extensions and agent entries load. Nothing here names a vendor or a standard.
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

use canoa::vendor::AgentVendor;
use coqui_acp_api::guatiao::library::Registry;
use coqui_acp_api::{AccountCatalog, AccountKindInfo, AccountOffer, AccountProvider, AcpHarness};

use crate::config::Config;

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
    /// The registry the provider was made in when it is not the bundled one, which the provider points into: dropped after it.
    registry: Option<Arc<Registry>>,
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
            registry: None,
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

/// The makers `registry` offers: its account providers that are not `<agent>_accounts` of an agent it also offers (an `acp_harness`,
/// or an `agent_vendor` describing a program that has none).
pub(crate) fn makers_in(registry: &Registry) -> Vec<Maker> {
    let mut harnesses: Vec<String> = registry
        .offers::<dyn AcpHarness>()
        .map(|offer| offer.id().to_owned())
        .collect();
    harnesses.extend(registry.offers::<dyn AgentVendor>().filter_map(|offer| {
        let info = offer.default_instance()?.describe().ok()?;
        Some(info.agent)
    }));
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

/// The makers of the libraries one canoa.cfg names, found the first time they are asked for and kept for that config: the cell
/// [`Config`] carries. A copy of a config starts with none, since its entries and extensions may differ.
#[derive(Default)]
pub(crate) struct Cell(OnceLock<Arc<Makers>>);

impl Clone for Cell {
    fn clone(&self) -> Self {
        Self::default()
    }
}

impl core::fmt::Debug for Cell {
    fn fmt(&self, f: &mut core::fmt::Formatter<'_>) -> core::fmt::Result {
        f.debug_struct("Cell").finish_non_exhaustive()
    }
}

/// What the cell holds is made from the config's other fields, so it takes no part in comparing two configs.
impl PartialEq for Cell {
    fn eq(&self, _: &Self) -> bool {
        true
    }
}

impl Eq for Cell {}

/// The kind ids and sign-in method ids the makers found so far offer: what a later library cannot offer again.
#[derive(Default)]
struct Taken {
    kinds: Vec<String>,
    methods: Vec<String>,
}

impl Taken {
    fn note(&mut self, offer: &AccountOffer) {
        self.kinds
            .extend(offer.kinds.iter().map(|kind| kind.id.clone()));
        self.methods
            .extend(offer.logins.iter().map(|login| login.id.clone()));
    }

    /// `maker` of the library `library`, less any kind and sign-in method an earlier maker offers (the log says each); `None` when no
    /// kind of it is left. What it keeps is taken from later makers.
    fn claim(&mut self, mut maker: Maker, library: &str) -> Option<Maker> {
        let provider = maker.provider.as_str();
        let mut offer = maker.accounts.clone();
        offer.kinds.retain(|kind| {
            let free = !self.kinds.contains(&kind.id);
            if !free {
                tracing::warn!(
                    library,
                    provider,
                    kind = kind.id.as_str(),
                    "its account kind is not offered: an earlier library offers a kind of that id"
                );
            }
            free
        });
        if offer.kinds.is_empty() {
            return None;
        }
        let own: Vec<String> = offer.kinds.iter().map(|kind| kind.id.clone()).collect();
        offer.logins.retain(|login| {
            let ours = own.contains(&login.produces);
            let free = ours && !self.methods.contains(&login.id);
            if ours && !free {
                tracing::warn!(
                    library,
                    provider,
                    method = login.id.as_str(),
                    "its sign-in method is not offered: an earlier library offers a method of that id"
                );
            }
            free
        });
        offer.usage.retain(|id| own.contains(id));
        self.note(&offer);
        maker.accounts = offer;
        Some(maker)
    }
}

/// Every account maker in force: the bundled libraries', and those of the libraries canoa.cfg's extensions and entries load.
#[derive(Clone, Default)]
pub(crate) struct Makers {
    /// The makers of canoa.cfg's own libraries, which the bundled ones outrank.
    added: Arc<[Maker]>,
}

impl Makers {
    /// The makers `config` names: the bundled libraries' and, from every library its extensions and agent entries load (an extension
    /// verified as the Store verifies it), the account providers that no harness of that library owns. A kind or a sign-in method
    /// an earlier library already offers is not offered again, and the log says so: a library does not take another's place.
    pub(crate) fn of(config: &Config) -> Arc<Self> {
        Arc::clone(
            config
                .makers
                .0
                .get_or_init(|| Arc::new(Self::loaded(config))),
        )
    }

    fn loaded(config: &Config) -> Self {
        let mut taken = Taken::default();
        for maker in offered() {
            taken.note(&maker.accounts);
        }
        let mut added: Vec<Maker> = Vec::new();
        for (library, registry) in crate::control::providers::libraries(config) {
            let registry = Arc::new(registry);
            for maker in makers_in(&registry) {
                if let Some(mut maker) = taken.claim(maker, &library) {
                    maker.registry = Some(Arc::clone(&registry));
                    added.push(maker);
                }
            }
        }
        Self {
            added: added.into(),
        }
    }

    /// Every maker in force: in a test the stand-ins the test's thread named, the bundled ones, then those canoa.cfg's libraries add.
    fn all(&self) -> Vec<Maker> {
        #[cfg(test)]
        let mut found = stand_ins::current();
        #[cfg(not(test))]
        let mut found = Vec::new();
        found.extend(offered().iter().cloned());
        found.extend(self.added.iter().cloned());
        found
    }

    /// The maker that makes accounts of `kind`.
    pub(crate) fn for_kind(&self, kind: &str) -> Option<Maker> {
        self.all()
            .into_iter()
            .find(|maker| maker.kind(kind).is_some())
    }

    /// The id of each account provider that is a maker: no agent is theirs.
    pub(crate) fn ids(&self) -> Vec<String> {
        self.all().into_iter().map(|maker| maker.provider).collect()
    }

    /// The maker whose sign-in method is `method`.
    pub(crate) fn login_provider(&self, method: &str) -> Option<Arc<dyn AccountProvider>> {
        self.all()
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

    /// Whether the kind `kind` is made by a maker, not by a harness's own account provider.
    pub(crate) fn makes(&self, kind: &str) -> bool {
        self.for_kind(kind).is_some()
    }

    /// Adds the kinds and sign-in methods the makers offer to `listing`, the harness `harness`'s own: a kind a gateway serves joins when
    /// the harness can be given a gateway, any other when the harness accepts a standard the kind provides (any it accepts, for a kind whose accounts name their own) and, for a
    /// kind that names an address, can point it there. `catalog` is what the harness accepts; a harness that lists none takes none.
    pub(crate) fn extend(
        &self,
        listing: &mut AccountCatalog,
        harness: &str,
        catalog: Option<&AccountCatalog>,
    ) {
        for maker in self.all() {
            for kind in &maker.accounts.kinds {
                if listing.kinds.iter().any(|each| each.id == kind.id) {
                    continue;
                }
                let joins = if crate::gateways::for_kind(&kind.id).is_some() {
                    crate::vendors::gateway_route(harness).is_some()
                } else {
                    // A kind whose accounts name their own standards is open to any harness that accepts one, whichever it is. A kind
                    // that names the address of its API (a key for that address) needs a harness that can point the standard at it.
                    let own = crate::accounts::standards::property(kind).is_some();
                    let addressed = kind
                        .endpoints
                        .iter()
                        .any(|slot| slot.purpose == coqui_acp_api::MODEL_API && slot.key.is_some());
                    catalog.is_some_and(|catalog| {
                        catalog.accepts.iter().any(|accepted| {
                            (own || kind.provides.contains(&accepted.standard))
                                && (!addressed
                                    || accepted
                                        .slots
                                        .iter()
                                        .any(|slot| slot.base_url_setting.is_some()))
                        })
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
