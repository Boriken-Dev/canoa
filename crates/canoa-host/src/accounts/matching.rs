//! Which accounts a harness can run a session on. An account's types are the standards it provides, a harness's catalog the
//! standards it accepts, and the account fits where the two meet. An account of a kind the harness's own provider makes always fits.
#![allow(
    clippy::std_instead_of_core,
    clippy::std_instead_of_alloc,
    reason = "a cache of catalogs is std's"
)]

use std::cell::{OnceCell, RefCell};
use std::collections::BTreeMap;
use std::rc::Rc;
use std::sync::Arc;

use coqui_acp_api::{AccountCatalog, AccountKindInfo};

use super::commands::provider_for;
use crate::config::{AccountConfig, Config};
use crate::kind::{Harness, Login, LoginProbe};
use crate::makers::Makers;

#[cfg(test)]
thread_local! {
    /// What a unit test makes of every bundled agent's program: found, so that no test looks at this computer's programs.
    static TEST_MISSING: core::cell::Cell<bool> = const { core::cell::Cell::new(false) };
}

/// Runs `f` with every bundled agent's program standing as missing (or found), for a unit test.
#[cfg(test)]
pub(crate) fn with_programs_missing<T>(missing: bool, f: impl FnOnce() -> T) -> T {
    let before = TEST_MISSING.with(|slot| slot.replace(missing));
    let out = f();
    TEST_MISSING.with(|slot| slot.set(before));
    out
}

/// What a provider says of its accounts: the standards it accepts, and the kinds and sign-in methods its account provider offers.
#[derive(Clone)]
struct Known {
    catalog: AccountCatalog,
    /// The kinds and sign-in methods of the harness's own account provider, with nothing of any other.
    listing: AccountCatalog,
    /// The id of the harness's account provider, when it has one.
    provider: Option<String>,
    /// What asks how the harness's own login stands, when its provider offers it as the automatic `local` account.
    probe: Option<LoginProbe>,
    /// The answer, once someone needed it: a call that only reads a catalog never asks a keyring.
    asked: Rc<OnceCell<Option<Login>>>,
}

impl Known {
    /// How the login stands, asked of the provider the first time and kept for this reading.
    fn login(&self) -> Option<Login> {
        self.asked
            .get_or_init(|| self.probe.as_ref().map(LoginProbe::ask))
            .clone()
    }
}

impl Known {
    fn of(harness: &Harness) -> Option<Self> {
        Some(Self {
            catalog: harness.catalog().cloned()?,
            listing: harness.listing().clone(),
            provider: harness.account_provider_id().map(str::to_owned),
            probe: harness.login_probe(),
            asked: Rc::default(),
        })
    }
}

/// Who made accounts of a kind: an account provider, and the agent it belongs to when an agent's own.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Made {
    /// The id of the account provider.
    pub(crate) provider: String,
    /// The harness whose own provider it is; `None` for a maker, which no agent owns.
    pub(crate) harness: Option<String>,
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

    /// As [`Catalogs::new`], with what the provider `harness` (named `id`) says already in hand: the standards it accepts and the
    /// kinds its account provider offers.
    pub(crate) fn with(config: &'a Config, id: &str, harness: &Harness) -> Self {
        let catalogs = Self::new(config);
        catalogs
            .known
            .borrow_mut()
            .insert(id.to_owned(), Known::of(harness));
        catalogs
    }

    /// As [`Catalogs::new`], with what the provider `id` says given as it is, for a test with no provider to build.
    #[cfg(test)]
    pub(crate) fn with_known(
        config: &'a Config,
        id: &str,
        (catalog, listing): (&AccountCatalog, &AccountCatalog),
        (provider, local): (Option<&str>, bool),
    ) -> Self {
        let catalogs = Self::new(config);
        catalogs.known.borrow_mut().insert(
            id.to_owned(),
            Some(Known {
                catalog: catalog.clone(),
                listing: listing.clone(),
                provider: provider.map(str::to_owned),
                probe: None,
                asked: Rc::new(OnceCell::from(local.then_some(Login::SignedIn(None)))),
            }),
        );
        catalogs
    }

    fn known(&self, harness: &str) -> Option<Known> {
        self.known
            .borrow_mut()
            .entry(harness.to_owned())
            .or_insert_with(|| Known::of(&provider_for(self.config, harness).ok()?))
            .clone()
    }

    /// The account makers in force for this config: the bundled libraries' and those its extensions and entries load.
    pub(crate) fn makers(&self) -> Arc<Makers> {
        Makers::of(self.config)
    }

    /// The catalog of the provider `harness`; `None` when it cannot be built or lists none.
    pub(crate) fn of(&self, harness: &str) -> Option<AccountCatalog> {
        self.known(harness).map(|known| known.catalog)
    }

    /// The kind `account` is of, as the account provider that made it offers it, providing the standards the account provides. The
    /// automatic `local` account is of its harness's own provider; any other is found by its kind, which no two providers share.
    pub(crate) fn kind(&self, account: &AccountConfig) -> Option<AccountKindInfo> {
        let info = match &account.owner {
            Some(harness) => self.known(harness)?.listing.kind(&account.kind).cloned()?,
            None => self.made(&account.kind)?.1,
        };
        Some(super::standards::of_account(
            info,
            account.settings.as_ref(),
        ))
    }

    /// Every kind of the account provider that makes accounts of `kind`; none when no provider makes it.
    pub(crate) fn kinds_beside(&self, kind: &str) -> Vec<AccountKindInfo> {
        if let Some(maker) = self.makers().for_kind(kind) {
            return maker.accounts.kinds;
        }
        self.made(kind)
            .and_then(|(made, _)| self.known(&made.harness?))
            .map(|known| known.listing.kinds)
            .unwrap_or_default()
    }

    /// The account provider that makes accounts of `kind`, and the kind as it offers it: a maker's, else the one a harness's own
    /// account provider offers. `None` when no provider this host can build makes it.
    pub(crate) fn made(&self, kind: &str) -> Option<(Made, AccountKindInfo)> {
        if let Some(maker) = self.makers().for_kind(kind) {
            let info = maker.kind(kind).cloned()?;
            return Some((
                Made {
                    provider: maker.provider,
                    harness: None,
                },
                info,
            ));
        }
        if kind == coqui_acp_api::HARNESS_LOGIN {
            return None;
        }
        // The harnesses already read first, so that a caller that has its own in hand builds no other.
        let read: Vec<String> = self
            .known
            .borrow()
            .iter()
            .filter(|(_, known)| known.is_some())
            .map(|(harness, _)| harness.clone())
            .collect();
        read.into_iter()
            .chain(self.harnesses())
            .find_map(|harness| {
                let known = self.known(&harness)?;
                let info = known.listing.kind(kind)?.clone();
                Some((
                    Made {
                        provider: known.provider?,
                        harness: Some(harness),
                    },
                    info,
                ))
            })
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

    /// The harness a call that has to go through one reads `account` with: its own login's harness, the harness whose provider made
    /// its kind, or for a maker's account the first harness that can run it. The reason when there is none.
    pub(crate) fn reading_harness(&self, account: &AccountConfig) -> Result<String, String> {
        if let Some(owner) = &account.owner {
            return Ok(owner.clone());
        }
        if let Some((made, _)) = self.made(&account.kind)
            && let Some(harness) = made.harness
        {
            return Ok(harness);
        }
        let info = self.kind(account).ok_or_else(|| {
            format!(
                "no account provider this host can build makes a {} account",
                account.kind
            )
        })?;
        fitting(self, account, &info)
            .into_iter()
            .next()
            .ok_or_else(|| {
                format!(
                    "no agent this host runs takes a {} account, so it is read through none",
                    account.kind
                )
            })
    }

    /// The id of the account provider of `harness`; `None` when it has none.
    pub(crate) fn provider_id_of(&self, harness: &str) -> Option<String> {
        self.known(harness)?.provider
    }

    /// The kind of the automatic `local` account of `harness`; `None` when its provider does not offer its own login, or it says the
    /// login is not on this computer.
    pub(crate) fn local_kind(&self, harness: &str) -> Option<AccountKindInfo> {
        let known = self.known(harness)?;
        if !self.login(harness)?.listed() {
            return None;
        }
        known.listing.kind(coqui_acp_api::HARNESS_LOGIN).cloned()
    }

    /// How the login of `harness` stands, as its account provider answers; `None` when it offers none as the account `local`.
    ///
    /// A provider that could not tell whether the login is there is taken as signed out while the program itself is not on this
    /// computer: there is nothing to be signed in to, and the Store already says it is not installed.
    pub(crate) fn login(&self, harness: &str) -> Option<Login> {
        let login = self.known(harness)?.login()?;
        if matches!(login, Login::Unknown(_)) && self.program_missing(harness) {
            return Some(Login::SignedOut(
                "its program is not installed on this computer".to_owned(),
            ));
        }
        Some(login)
    }

    /// Whether `harness` is one of the bundled agents whose program this computer lacks, as the Store finds it.
    #[cfg(test)]
    fn program_missing(&self, _: &str) -> bool {
        TEST_MISSING.with(core::cell::Cell::get)
    }

    /// Whether `harness` is one of the bundled agents whose program this computer lacks, as the Store finds it.
    #[cfg(not(test))]
    fn program_missing(&self, harness: &str) -> bool {
        let Some((name, _)) = crate::bundled::harness_sources().find(|(_, id)| *id == harness)
        else {
            return false;
        };
        crate::sources::entries(self.config)
            .iter()
            .any(|source| source.entry.name == name && !source.missing.is_empty())
    }

    /// The automatic `local` account of every harness whose provider offers its own login: the harness and the kind that login is. Only a login
    /// that is there, or one the provider could not look for, is listed.
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
        if !self.login(harness)?.listed() {
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

/// Whether `runs`, whose catalog is `catalog` when the caller has it, applies `account` as an account of a standard it accepts, not
/// as one of its own kinds. A maker's accounts are always applied so; the automatic `local` account is its harness's own; any other
/// is its own when the harness's catalog lists its kind. A caller with no catalog makes no claim that the account is applied as a
/// standard.
pub(crate) fn borrowed(
    makers: &Makers,
    account: &AccountConfig,
    runs: &str,
    catalog: Option<&AccountCatalog>,
) -> bool {
    if makers.makes(&account.kind) {
        return true;
    }
    if account.kind == coqui_acp_api::HARNESS_LOGIN {
        return account.owner.as_deref() != Some(runs);
    }
    catalog.is_some_and(|catalog| catalog.kind(&account.kind).is_none())
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
    if !borrowed(makers, account, runs, Some(catalog)) {
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

/// The harnesses `account`, of the kind `info`, can run a session on, in the order the harnesses are listed.
pub(crate) fn fitting(
    catalogs: &Catalogs<'_>,
    account: &AccountConfig,
    info: &AccountKindInfo,
) -> Vec<String> {
    let makers = catalogs.makers();
    catalogs
        .harnesses()
        .into_iter()
        .filter(|harness| {
            catalogs
                .of(harness)
                .is_some_and(|catalog| fits(&makers, &catalog, harness, account, info).is_ok())
        })
        .collect()
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
            self.catalog,
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
            !borrowed(&makers, account, harness, catalog.as_ref())
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

    fn account(kind: &str) -> AccountConfig {
        AccountConfig {
            owner: None,
            kind: kind.to_owned(),
            id: "id".to_owned(),
            settings: None,
        }
    }

    /// A catalog that accepts `accepts`, and lists the kind `k` as one it applies itself when `own`.
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
        let own = account("k");
        let mut codex = catalog(&[]);
        codex.kinds.push(AccountKindInfo::new("k", "K"));
        assert!(fits(&Makers::default(), &codex, "codex", &own, &kind(&[])).is_ok());
        assert!(
            fits(
                &Makers::default(),
                &catalog(&[]),
                "claude",
                &own,
                &kind(&[])
            )
            .is_err(),
            "a harness that does not list the kind applies it as a standard"
        );
    }

    #[test]
    fn another_providers_account_fits_when_a_standard_it_provides_is_accepted() {
        let key = account("k");
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
        let key = account("k");
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
        let gateway = account(FOUNDRY);
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
        assert!(runs.directory(&account("k"), &info));
        assert!(
            !runs.directory(&account("own-key"), &info),
            "its own kind says"
        );
        assert!(!Runs::plain("pi").directory(&account("k"), &info));
    }
}
