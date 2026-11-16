//! Which accounts a harness can run a session on. An account's kind says the standards it provides, a harness's catalog the
//! standards it accepts, and the account fits where the two meet. An account of the harness's own provider always fits.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a cache of catalogs is std's"
)]

use std::cell::RefCell;
use std::collections::BTreeMap;

use canoa_foundry::account::{KIND as FOUNDRY, route};
use coqui_acp_api::{AccountCatalog, AccountKindInfo};

use super::commands::provider_for;
use crate::config::{AccountConfig, Config};

/// The account catalogs of the providers canoa.cfg's accounts and entries name, each read once.
pub(crate) struct Catalogs<'a> {
    config: &'a Config,
    known: RefCell<BTreeMap<String, Option<AccountCatalog>>>,
}

impl<'a> Catalogs<'a> {
    /// No catalog read yet.
    pub(crate) const fn new(config: &'a Config) -> Self {
        Self {
            config,
            known: RefCell::new(BTreeMap::new()),
        }
    }

    /// As [`Catalogs::new`], with the catalog of `harness` already in hand.
    pub(crate) fn with(config: &'a Config, harness: &str, catalog: &AccountCatalog) -> Self {
        let catalogs = Self::new(config);
        catalogs
            .known
            .borrow_mut()
            .insert(harness.to_owned(), Some(catalog.clone()));
        catalogs
    }

    /// The catalog of the provider `harness`; `None` when it cannot be built or lists none.
    pub(crate) fn of(&self, harness: &str) -> Option<AccountCatalog> {
        self.known
            .borrow_mut()
            .entry(harness.to_owned())
            .or_insert_with(|| {
                provider_for(self.config, harness)
                    .ok()
                    .and_then(|provider| provider.catalog().cloned())
            })
            .clone()
    }

    /// The kind `account` is of, in the catalog of the provider that made it.
    pub(crate) fn kind(&self, account: &AccountConfig) -> Option<AccountKindInfo> {
        self.of(&account.harness)?.kind(&account.kind).cloned()
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

/// Why `account`, of the kind `info`, cannot run a session on `runs`, whose catalog is `catalog`; `Ok` when it can. The reason
/// names standards, never a value.
///
/// # Errors
///
/// The standards the account provides and the harness accepts, when none is both.
pub(crate) fn fits(
    catalog: &AccountCatalog,
    runs: &str,
    account: &AccountConfig,
    info: &AccountKindInfo,
) -> Result<(), String> {
    if account.harness == runs {
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
    if account.kind == FOUNDRY && route(runs).is_none() {
        return Err(format!("{runs} cannot be given a gateway"));
    }
    Ok(())
}

/// The harness a session runs on, and its catalog when the caller has it.
#[derive(Clone, Copy)]
pub(crate) struct Runs<'a> {
    pub(crate) harness: &'a str,
    pub(crate) catalog: Option<&'a AccountCatalog>,
}

impl<'a> Runs<'a> {
    /// The harness `harness`, whose catalog the caller has not read.
    #[cfg(test)]
    pub(crate) const fn plain(harness: &'a str) -> Self {
        Self {
            harness,
            catalog: None,
        }
    }

    /// Whether the harness applies an account of the kind `info` that another provider made from a private directory of the
    /// account's, as the kind of its own that the standard names does.
    pub(crate) fn directory(&self, account: &AccountConfig, info: &AccountKindInfo) -> bool {
        if account.harness == self.harness {
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
    config
        .accounts
        .iter()
        .filter(|(_, account)| {
            account.harness == harness
                || catalog.as_ref().is_some_and(|catalog| {
                    catalogs
                        .kind(account)
                        .is_some_and(|info| fits(catalog, harness, account, &info).is_ok())
                })
        })
        .map(|(name, _)| name.clone())
        .collect()
}

#[cfg(test)]
mod tests {
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
        assert!(fits(&catalog(&[]), "codex", &own, &kind(&[])).is_ok());
    }

    #[test]
    fn another_providers_account_fits_when_a_standard_it_provides_is_accepted() {
        let key = account("claude", "k");
        let info = kind(&[standard::ANTHROPIC_API]);
        let pi = catalog(&[standard::OPENAI_API, standard::ANTHROPIC_API]);
        assert!(fits(&pi, "pi", &key, &info).is_ok());
        let codex = catalog(&[standard::CODEX, standard::OPENAI_API]);
        let why = fits(&codex, "codex", &key, &info).expect_err("not accepted");
        assert_eq!(
            why,
            "it provides anthropic-api and codex accepts codex, openai-api"
        );
    }

    #[test]
    fn a_kind_that_provides_nothing_is_taken_only_by_its_own_provider() {
        let key = account("claude", "k");
        let why = fits(&catalog(&[standard::ANTHROPIC_API]), "pi", &key, &kind(&[]))
            .expect_err("provides none");
        assert!(why.starts_with("it provides none"), "{why}");
    }

    #[test]
    fn a_gateway_account_fits_only_where_a_gateway_can_be_given() {
        let gateway = account("claude", FOUNDRY);
        let info = kind(&[standard::ANTHROPIC_API, standard::OPENAI_API]);
        assert!(fits(&catalog(&[standard::OPENAI_API]), "codex", &gateway, &info).is_ok());
        let why = fits(&catalog(&[standard::OPENAI_API]), "grok", &gateway, &info)
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
        };
        assert!(runs.directory(&account("claude", "k"), &info));
        assert!(
            !runs.directory(&account("pi", "k"), &info),
            "its own kind says"
        );
        assert!(!Runs::plain("pi").directory(&account("claude", "k"), &info));
    }
}
