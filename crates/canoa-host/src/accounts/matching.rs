//! Which accounts a harness can run a session on. An account's kind says the standards it provides, a harness's catalog the
//! standards it accepts, and the account fits where the two meet. An account of the harness's own provider always fits.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a cache of catalogs is std's"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;
use std::sync::Arc;

use coqui_acp_api::{AccountCatalog, AccountKindInfo};

use super::commands::provider_for;
use crate::config::{AccountConfig, Config};
use crate::makers::Makers;

/// What a provider says of its accounts: the standards it accepts, and the kinds and sign-in methods its account provider offers.
#[derive(Clone)]
struct Known {
    catalog: AccountCatalog,
    listing: AccountCatalog,
    /// Whether the provider offers the harness's own login as the automatic `local` account.
    local: bool,
}

/// The account catalogs of the providers canoa.cfg's accounts and entries name, each read once.
pub(crate) struct Catalogs<'a> {
    config: &'a Config,
    known: RefCell<BTreeMap<String, Option<Known>>>,
}

impl<'a> Catalogs<'a> {
    /// No catalog read yet.
    pub(crate) const fn new(config: &'a Config) -> Self {
        Self {
            config,
            known: RefCell::new(BTreeMap::new()),
        }
    }

    /// As [`Catalogs::new`], with what the provider `harness` says already in hand: the standards it accepts and the kinds
    /// its account provider offers.
    pub(crate) fn with(
        config: &'a Config,
        harness: &str,
        (catalog, listing): (&AccountCatalog, &AccountCatalog),
        local: bool,
    ) -> Self {
        let catalogs = Self::new(config);
        catalogs.known.borrow_mut().insert(
            harness.to_owned(),
            Some(Known {
                catalog: catalog.clone(),
                listing: listing.clone(),
                local,
            }),
        );
        catalogs
    }

    fn known(&self, harness: &str) -> Option<Known> {
        self.known
            .borrow_mut()
            .entry(harness.to_owned())
            .or_insert_with(|| {
                let provider = provider_for(self.config, harness).ok()?;
                Some(Known {
                    catalog: provider.catalog().cloned()?,
                    listing: provider.listing().clone(),
                    local: provider.local_account().is_some(),
                })
            })
            .clone()
    }

    /// The account makers in force for this config: the bundled libraries' and those its extensions and entries load.
    pub(crate) fn makers(&self) -> Arc<Makers> {
        Makers::of(self.config)
    }

    /// The kinds of account and ways to sign in that the provider `harness` offers; `None` when it cannot be built.
    pub(crate) fn listing(&self, harness: &str) -> Option<AccountCatalog> {
        self.known(harness).map(|known| known.listing)
    }

    /// The catalog of the provider `harness`; `None` when it cannot be built or lists none.
    pub(crate) fn of(&self, harness: &str) -> Option<AccountCatalog> {
        self.known(harness).map(|known| known.catalog)
    }

    /// The kind `account` is of, as the account provider of the provider that made it offers it, providing the standards the account
    /// provides.
    pub(crate) fn kind(&self, account: &AccountConfig) -> Option<AccountKindInfo> {
        self.known(&account.harness)
            .and_then(|known| known.listing.kind(&account.kind).cloned())
            .or_else(|| {
                // A kind a maker makes is the same whichever harness the account names, or none that can be built.
                self.makers()
                    .for_kind(&account.kind)
                    .and_then(|maker| maker.kind(&account.kind).cloned())
            })
            // The standards an account provides are its own where its kind says so.
            .map(|info| super::standards::of_account(info, account.settings.as_ref()))
    }

    /// Every harness whose accounts this host can make or run: the bundled agents' and those canoa.cfg's entries run.
    pub(crate) fn harnesses(&self) -> Vec<String> {
        let mut found: Vec<String> = crate::bundled::harness_sources()
            .map(|(_, harness)| harness.to_owned())
            .collect();
        for entry in &self.config.entries {
            if let crate::config::EntryKind::Harness { harness, .. } = &entry.kind
                && !found.contains(harness)
            {
                found.push(harness.clone());
            }
        }
        found
    }

    /// The kind of the automatic `local` account of `harness`; `None` when its provider does not offer its own login.
    pub(crate) fn local_kind(&self, harness: &str) -> Option<AccountKindInfo> {
        let known = self.known(harness)?;
        if !known.local {
            return None;
        }
        known.listing.kind(coqui_acp_api::HARNESS_LOGIN).cloned()
    }

    /// The automatic `local` account of every harness whose provider offers its own login: the harness and the kind that login is.
    pub(crate) fn locals(&self) -> Vec<(String, AccountKindInfo)> {
        self.harnesses()
            .into_iter()
            .filter_map(|harness| {
                let kind = self.local_kind(&harness)?;
                Some((harness, kind))
            })
            .collect()
    }

    /// The standard the login `harness` has where it runs provides, when its provider offers that login as the account `local`.
    /// A harness lists first what it was made for, but a harness also takes the APIs others do, so the login is the first standard
    /// it accepts that no other harness accepts: one only its own login makes. `None` when it has no such login.
    pub(crate) fn login_standard(&self, harness: &str) -> Option<String> {
        if !self.known(harness)?.local {
            return None;
        }
        let others: Vec<Vec<String>> = self
            .harnesses()
            .iter()
            .filter(|other| other.as_str() != harness)
            .map(|other| self.accepts(other))
            .collect();
        self.accepts(harness)
            .into_iter()
            .find(|standard| others.iter().all(|taken| !taken.contains(standard)))
    }

    /// The standards the provider `harness` accepts, most preferred first.
    pub(crate) fn accepts(&self, harness: &str) -> Vec<String> {
        self.of(harness)
            .map(|catalog| {
                catalog
                    .accepts
                    .iter()
                    .map(|each| each.standard.clone())
                    .collect()
            })
            .unwrap_or_default()
    }
}

/// Whether `runs` applies `account` as an account of a standard it accepts, not as one of its own kinds: another provider made it,
/// or a maker did, whichever harness it names.
pub(crate) fn borrowed(makers: &Makers, account: &AccountConfig, runs: &str) -> bool {
    account.harness != runs || makers.makes(&account.kind)
}

/// Why `account`, of the kind `info`, cannot run a session on `runs`, whose catalog is `catalog`; `Ok` when it can. The reason
/// names standards, never a value.
///
/// # Errors
///
/// The standards the account provides and the harness accepts, when none is both.
pub(crate) fn fits(
    makers: &Makers,
    catalog: &AccountCatalog,
    runs: &str,
    account: &AccountConfig,
    info: &AccountKindInfo,
) -> Result<(), String> {
    if !borrowed(makers, account, runs) {
        return Ok(());
    }
    let list = |ids: Vec<&str>| {
        if ids.is_empty() {
            "none".to_owned()
        } else {
            ids.join(", ")
        }
    };
    if !catalog
        .accepts
        .iter()
        .any(|accepted| info.provides.contains(&accepted.standard))
    {
        return Err(format!(
            "it provides {} and {runs} accepts {}",
            list(info.provides.iter().map(String::as_str).collect()),
            list(
                catalog
                    .accepts
                    .iter()
                    .map(|each| each.standard.as_str())
                    .collect()
            ),
        ));
    }
    if crate::gateways::for_kind(&account.kind).is_some()
        && crate::vendors::gateway_route(runs).is_none()
    {
        return Err(format!("{runs} cannot be given a gateway"));
    }
    // An account that names an address is a key for that address: a harness that takes the standard only at its vendor's own
    // would send the key there, and refuses.
    if account
        .settings
        .as_ref()
        .is_some_and(|settings| settings.as_map().contains_key(BASE_URL))
        && !moves_endpoint(catalog, &info.provides)
    {
        return Err(format!(
            "{runs} takes {} at its vendor's own address and cannot be pointed at the account's",
            list(
                catalog
                    .accepted_by(&info.provides)
                    .map(|accepted| vec![accepted.standard.as_str()])
                    .unwrap_or_default()
            )
        ));
    }
    Ok(())
}

/// The setting of an account that names the address of the API it is a key for: what a harness's slot moves to its own endpoint
/// setting.
pub(crate) const BASE_URL: &str = "base_url";

/// Whether `catalog` can point the standard it applies an account that provides `provides` as at an address of the account's: one of
/// the slots of that standard has a setting for it.
pub(crate) fn moves_endpoint(catalog: &AccountCatalog, provides: &[String]) -> bool {
    catalog.accepted_by(provides).is_ok_and(|accepted| {
        accepted
            .slots
            .iter()
            .any(|slot| slot.base_url_setting.is_some())
    })
}

/// The harness a session runs on, and its catalog when the caller has it.
#[derive(Clone, Copy)]
pub(crate) struct Runs<'a> {
    pub(crate) harness: &'a str,
    pub(crate) catalog: Option<&'a AccountCatalog>,
    /// The account makers in force; the bundled ones when the caller has none to name.
    pub(crate) makers: Option<&'a Makers>,
}

impl<'a> Runs<'a> {
    /// The harness `harness`, whose catalog the caller has not read.
    #[cfg(test)]
    pub(crate) const fn plain(harness: &'a str) -> Self {
        Self {
            harness,
            catalog: None,
            makers: None,
        }
    }

    /// Whether the harness applies `account` as an account of a standard it accepts, not as one of its own kinds.
    pub(crate) fn borrowed(&self, account: &AccountConfig) -> bool {
        borrowed(
            self.makers.unwrap_or(&Makers::default()),
            account,
            self.harness,
        )
    }

    /// The standard the harness applies `account` of the kind `info` as, when another provider made it and the harness accepts one of
    /// the standards it provides: the first, in the harness's own order of preference.
    pub(crate) fn standard(
        &self,
        account: &AccountConfig,
        info: &AccountKindInfo,
    ) -> Option<String> {
        if !self.borrowed(account) {
            return None;
        }
        self.catalog?
            .accepted_by(&info.provides)
            .ok()
            .map(|accepted| accepted.standard.clone())
    }

    /// Whether the harness applies an account of the kind `info` that another provider made from a private directory of the
    /// account's, as the kind of its own that the standard names does.
    pub(crate) fn directory(&self, account: &AccountConfig, info: &AccountKindInfo) -> bool {
        if !self.borrowed(account) {
            return false;
        }
        let Some(catalog) = self.catalog else {
            return false;
        };
        catalog
            .accepts
            .iter()
            .find(|accepted| info.provides.contains(&accepted.standard))
            .is_some_and(|accepted| {
                accepted
                    .slots
                    .iter()
                    .any(|slot| catalog.kind(&slot.kind).is_some_and(|kind| kind.state_dir))
            })
    }
}

/// The names of canoa.cfg's accounts that can run a session on `harness`, in the file's order: its own provider's, and the
/// others' whose kind provides a standard it accepts.
pub(crate) fn usable(config: &Config, catalogs: &Catalogs<'_>, harness: &str) -> Vec<String> {
    let catalog = catalogs.of(harness);
    let makers = catalogs.makers();
    config
        .accounts
        .iter()
        .filter(|(_, account)| {
            !borrowed(&makers, account, harness)
                || catalog.as_ref().is_some_and(|catalog| {
                    catalogs
                        .kind(account)
                        .is_some_and(|info| fits(&makers, catalog, harness, account, &info).is_ok())
                })
        })
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
    use canoa_foundry::account::KIND as FOUNDRY;
    use coqui_acp_api::{Accepted, standard};

    use super::*;

    fn account(harness: &str, kind: &str) -> AccountConfig {
        AccountConfig {
            harness: harness.to_owned(),
            kind: kind.to_owned(),
            id: "id".to_owned(),
            settings: None,
        }
    }

    fn catalog(accepts: &[&str]) -> AccountCatalog {
        let mut catalog = AccountCatalog::new();
        catalog.accepts = accepts.iter().map(|id| Accepted::new(*id)).collect();
        catalog
    }

    fn kind(provides: &[&str]) -> AccountKindInfo {
        AccountKindInfo::new("k", "K").provides(provides.iter().copied())
    }

    #[test]
    fn an_account_of_the_harness_s_own_provider_always_fits() {
        let own = account("codex", "k");
        assert!(fits(&Makers::default(), &catalog(&[]), "codex", &own, &kind(&[])).is_ok());
    }

    #[test]
    fn another_providers_account_fits_when_a_standard_it_provides_is_accepted() {
        let key = account("claude", "k");
        let info = kind(&[standard::ANTHROPIC_API]);
        let pi = catalog(&[standard::OPENAI_API, standard::ANTHROPIC_API]);
        assert!(fits(&Makers::default(), &pi, "pi", &key, &info).is_ok());
        let codex = catalog(&[standard::CODEX, standard::OPENAI_API]);
        let why = fits(&Makers::default(), &codex, "codex", &key, &info).expect_err("not accepted");
        assert_eq!(
            why,
            "it provides anthropic-api and codex accepts codex, openai-api"
        );
    }

    #[test]
    fn a_kind_that_provides_nothing_is_taken_only_by_its_own_provider() {
        let key = account("claude", "k");
        let why = fits(
            &Makers::default(),
            &catalog(&[standard::ANTHROPIC_API]),
            "pi",
            &key,
            &kind(&[]),
        )
        .expect_err("provides none");
        assert!(why.starts_with("it provides none"), "{why}");
    }

    #[test]
    fn a_gateway_account_fits_only_where_a_gateway_can_be_given() {
        let gateway = account("claude", FOUNDRY);
        let info = kind(&[standard::ANTHROPIC_API, standard::OPENAI_API]);
        assert!(
            fits(
                &Makers::default(),
                &catalog(&[standard::OPENAI_API]),
                "codex",
                &gateway,
                &info
            )
            .is_ok()
        );
        let why = fits(
            &Makers::default(),
            &catalog(&[standard::OPENAI_API]),
            "grok",
            &gateway,
            &info,
        )
        .expect_err("no gateway route");
        assert_eq!(why, "grok cannot be given a gateway");
    }

    #[test]
    fn another_providers_key_needs_a_directory_where_the_standard_s_kind_keeps_one() {
        let mut harness = catalog(&[]);
        harness.accepts = vec![
            Accepted::new(standard::ANTHROPIC_API)
                .slot(coqui_acp_api::AcceptedSlot::new("api_key", "own-key")),
        ];
        let mut own = AccountKindInfo::new("own-key", "Key");
        own.state_dir = true;
        harness.kinds.push(own);
        let info = kind(&[standard::ANTHROPIC_API]);
        let runs = Runs {
            harness: "pi",
            catalog: Some(&harness),
            makers: None,
        };
        assert!(runs.directory(&account("claude", "k"), &info));
        assert!(
            !runs.directory(&account("pi", "k"), &info),
            "its own kind says"
        );
        assert!(!Runs::plain("pi").directory(&account("claude", "k"), &info));
    }
}
