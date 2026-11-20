//! Account pools: a named set of accounts that provide one credential standard, which a session may run on instead of one account.
//!
//! The host names no standard and no kind. An account provider says the standards each of its kinds provides, a harness the
//! standards it accepts and the standard of its own login (the first it accepts, which the account `local` provides), and a pool
//! is the accounts that provide the standard it names. Besides the pools a person writes (`[host.pools.NAME]`), the host offers
//! `any-<standard>` for every standard at least one account provides, holding all of them, so nothing is written for it and it
//! follows the accounts as they come and go. A listing (`pools/list`, what an entry offers to choose from) shows one only while two
//! or more accounts hold it, since a pool of one account cannot rotate; a session or canoa.cfg that names it still resolves it,
//! whatever it holds.
//!
//! A pool is a set of accounts and is never an account of its own: each member keeps its own credential, usage and holder, so an
//! account that is in the pools of two standards is still one account.

use alloc::collections::BTreeSet;
use alloc::string::String;
use alloc::vec::Vec;

use coqui_acp_api::standard::is_standard;
use coqui_acp_api::{AccountKindInfo, HARNESS_LOGIN, LOCAL_ACCOUNT};

use crate::accounts::matching::{self, Catalogs};
use crate::config::{AccountConfig, Config, PoolConfig};

/// What the name of the host's own pool for a standard starts with.
pub(crate) const AUTOMATIC: &str = "any-";

/// The standard `name` is the host's own pool of, when it is one: `any-` and a standard's id.
pub(crate) fn automatic_standard(name: &str) -> Option<&str> {
    name.strip_prefix(AUTOMATIC)
        .filter(|rest| is_standard(rest))
}

/// The name of the host's own pool for `standard`.
pub(crate) fn automatic_name(standard: &str) -> String {
    format!("{AUTOMATIC}{standard}")
}

/// A pool and the accounts in it, by name, in the order it takes them.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Pool {
    pub(crate) name: String,
    pub(crate) standard: String,
    /// The host's own `any-<standard>`: in no file.
    pub(crate) automatic: bool,
    pub(crate) accounts: Vec<String>,
}

/// The standards the logins harnesses have where they run provide, one per harness that offers its login as `local`.
fn logins(catalogs: &Catalogs<'_>) -> Vec<String> {
    let mut standards = Vec::new();
    for harness in catalogs.harnesses() {
        if let Some(standard) = catalogs.login_standard(&harness)
            && !standards.contains(&standard)
        {
            standards.push(standard);
        }
    }
    standards
}

/// The kind of the account `name` and the standards it provides; the automatic `local` is of the kind every provider lists and
/// provides what the logins of the harnesses do. `None` when `name` is no account.
fn provided(config: &Config, catalogs: &Catalogs<'_>, name: &str) -> Option<(String, Vec<String>)> {
    if name == LOCAL_ACCOUNT {
        return Some((HARNESS_LOGIN.to_owned(), logins(catalogs)));
    }
    let account = config.accounts.get(name)?;
    let info = catalogs.kind(account)?;
    Some((info.id.clone(), info.provides))
}

/// Why `member` cannot be in a pool of `standard`, naming the account, its kind's standards and the pool's.
fn misfit(
    config: &Config,
    catalogs: &Catalogs<'_>,
    standard: &str,
    member: &str,
) -> Result<(), String> {
    let Some((kind, provides)) = provided(config, catalogs, member) else {
        return Err(format!(
            "{member} is not an account: [host.accounts] does not hold it and its provider lists no such kind"
        ));
    };
    if provides.iter().any(|each| each == standard) {
        return Ok(());
    }
    let list = if provides.is_empty() {
        "none".to_owned()
    } else {
        provides.join(", ")
    };
    Err(format!(
        "account {member} (kind {kind}) provides {list}, and the pool's standard is {standard}"
    ))
}

/// Checks the pool `name` of canoa.cfg: each member provides its standard.
///
/// # Errors
///
/// The member that does not, with the standards on each side.
pub(crate) fn check_one(
    config: &Config,
    catalogs: &Catalogs<'_>,
    name: &str,
    pool: &PoolConfig,
) -> Result<(), String> {
    for member in &pool.accounts {
        misfit(config, catalogs, &pool.standard, member)
            .map_err(|why| format!("pool {name}: {why}"))?;
    }
    Ok(())
}

/// Checks every pool of canoa.cfg.
///
/// # Errors
///
/// As [`check_one`], for the first pool that has a member that does not provide its standard.
#[cfg(test)]
pub(crate) fn check(config: &Config, catalogs: &Catalogs<'_>) -> Result<(), String> {
    for (name, pool) in &config.pools {
        check_one(config, catalogs, name, pool)?;
    }
    Ok(())
}

/// Every pool of the host: those canoa.cfg declares, in its order, then `any-<standard>` for each standard some account provides
/// (the logins of the harnesses included), each holding all the accounts that provide it, `local` first.
pub(crate) fn listing(config: &Config, catalogs: &Catalogs<'_>) -> Vec<Pool> {
    let mut pools: Vec<Pool> = config
        .pools
        .iter()
        .map(|(name, pool)| Pool {
            name: name.clone(),
            standard: pool.standard.clone(),
            automatic: false,
            accounts: pool.accounts.clone(),
        })
        .collect();
    let by_login = logins(catalogs);
    let mut standards: BTreeSet<String> = by_login.iter().cloned().collect();
    let kinds: Vec<(&String, Vec<String>)> = config
        .accounts
        .iter()
        .map(|(name, account)| {
            (
                name,
                catalogs
                    .kind(account)
                    .map(|info| info.provides)
                    .unwrap_or_default(),
            )
        })
        .collect();
    for (_, provides) in &kinds {
        standards.extend(provides.iter().cloned());
    }
    for standard in standards {
        let mut accounts = Vec::new();
        if by_login.contains(&standard) {
            accounts.push(LOCAL_ACCOUNT.to_owned());
        }
        accounts.extend(
            kinds
                .iter()
                .filter(|(_, provides)| provides.contains(&standard))
                .map(|(name, _)| (*name).clone()),
        );
        pools.push(Pool {
            name: automatic_name(&standard),
            automatic: true,
            standard,
            accounts,
        });
    }
    pools
}

/// Whether a pool is shown in a listing: the person's own always, the host's `any-<standard>` only with two or more accounts,
/// since one cannot rotate. Resolving a pool by name does not ask this.
pub(crate) const fn worth_listing(automatic: bool, accounts: usize) -> bool {
    !automatic || accounts >= 2
}

/// [`listing`] as a person is shown it: without the host's pools that hold fewer than two accounts.
pub(crate) fn listed(config: &Config, catalogs: &Catalogs<'_>) -> Vec<Pool> {
    let mut pools = listing(config, catalogs);
    pools.retain(|pool| worth_listing(pool.automatic, pool.accounts.len()));
    pools
}

/// An account a pool holds, as one entry sees it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct Member {
    pub(crate) name: String,
    /// Whether the entry offers it, by its own name or through a pool it names: a connection that holds no grant for accounts may
    /// run only on those.
    pub(crate) listed: bool,
}

/// A pool as one entry of a harness offers it.
#[derive(Clone, Debug, PartialEq, Eq)]
pub(crate) struct EntryPool {
    pub(crate) name: String,
    pub(crate) standard: String,
    pub(crate) automatic: bool,
    /// Whether the entry names it, which gives its members to every connection; any other pool the harness accepts is open to a
    /// connection that holds a grant for accounts.
    pub(crate) listed: bool,
    pub(crate) members: Vec<Member>,
}

impl EntryPool {
    /// The members a connection may run on: all of them with a grant for accounts, else those the entry offers.
    pub(crate) fn usable(&self, any: bool) -> impl Iterator<Item = &Member> {
        self.members
            .iter()
            .filter(move |member| any || member.listed || self.listed)
    }

    /// Whether a connection may choose this pool: the entry's own, or the host's `any-<standard>`, or any with a grant for accounts.
    pub(crate) fn open_to(&self, any: bool) -> bool {
        any || self.listed || self.automatic
    }
}

/// The account `member`, as a pool of `standard` on `harness` would hold it: its configuration and kind when it provides the
/// standard and can run there; the automatic `local` when the harness's own login is of that standard.
fn fitting<'a>(
    (config, catalogs): (&'a Config, &Catalogs<'_>),
    (harness, login): (&str, Option<&str>),
    standard: &str,
    member: &str,
) -> Option<Option<(&'a AccountConfig, AccountKindInfo)>> {
    if member == LOCAL_ACCOUNT {
        return (login == Some(standard)).then_some(None);
    }
    let account = config.accounts.get(member)?;
    let info = catalogs.kind(account)?;
    if !info.provides.iter().any(|each| each == standard) {
        return None;
    }
    let catalog = catalogs.of(harness)?;
    matching::fits(
        &crate::makers::Makers::of(config),
        &catalog,
        harness,
        account,
        &info,
    )
    .ok()?;
    Some(Some((account, info)))
}

/// The members of the pool `pool` (`None` for the host's own pool of `standard`) that run on `harness`, in the pool's order.
fn runnable(
    (config, catalogs): (&Config, &Catalogs<'_>),
    harness: &str,
    standard: &str,
    pool: Option<&PoolConfig>,
) -> Vec<String> {
    let login = catalogs.login_standard(harness);
    let candidates: Vec<String> = match pool {
        Some(pool) => pool.accounts.clone(),
        None => core::iter::once(LOCAL_ACCOUNT.to_owned())
            .chain(config.accounts.keys().cloned())
            .collect(),
    };
    candidates
        .into_iter()
        .filter(|member| {
            fitting(
                (config, catalogs),
                (harness, login.as_deref()),
                standard,
                member,
            )
            .is_some()
        })
        .collect()
}

/// The standards a harness accepts, and why a pool of `standard` is no choice for it when it does not.
fn accepted(
    catalogs: &Catalogs<'_>,
    harness: &str,
    pool: &str,
    standard: &str,
) -> Result<(), String> {
    let accepts = catalogs.accepts(harness);
    if accepts.iter().any(|each| each == standard) {
        return Ok(());
    }
    Err(format!(
        "pool {pool} is of {standard}, and {harness} accepts {}",
        if accepts.is_empty() {
            "none".to_owned()
        } else {
            accepts.join(", ")
        }
    ))
}

/// What an entry that runs `harness` and names `names` (its `accounts`) offers, with pools named by name replaced by the
/// accounts they hold that can run there, in order and once each. The automatic `local` stands for the harness's own login, and is
/// left out of a pool of another standard.
///
/// # Errors
///
/// A pool the entry names whose standard the harness does not accept.
pub(crate) fn expand(
    config: &Config,
    catalogs: &Catalogs<'_>,
    harness: &str,
    names: &[String],
) -> Result<Vec<String>, String> {
    let mut out: Vec<String> = Vec::new();
    for name in names {
        let held = if let Some(pool) = config.pools.get(name) {
            check_one(config, catalogs, name, pool)?;
            accepted(catalogs, harness, name, &pool.standard)?;
            runnable((config, catalogs), harness, &pool.standard, Some(pool))
        } else if let Some(standard) = automatic_standard(name) {
            accepted(catalogs, harness, name, standard)?;
            runnable((config, catalogs), harness, standard, None)
        } else {
            vec![name.clone()]
        };
        for member in held {
            if !out.contains(&member) {
                out.push(member);
            }
        }
    }
    Ok(out)
}

/// Whether `name` names a pool, one of canoa.cfg's or the host's own.
pub(crate) fn is_pool(config: &Config, name: &str) -> bool {
    config.pools.contains_key(name) || automatic_standard(name).is_some()
}

/// The pools an entry that runs `harness` and names `names` offers: those it names, every other pool of canoa.cfg whose
/// standard the harness accepts, and `any-<standard>` for each standard it accepts that an account of the entry provides. A
/// member is `listed` when the entry offers its account; one it does not offer is left out when its credential would reach the
/// agent in its environment, as for an account the entry does not list.
///
/// # Errors
///
/// A pool the entry names whose standard the harness does not accept.
pub(crate) fn for_entry(
    config: &Config,
    catalogs: &Catalogs<'_>,
    harness: &str,
    names: &[String],
) -> Result<Vec<EntryPool>, String> {
    let accepts = catalogs.accepts(harness);
    if accepts.is_empty() {
        return Ok(Vec::new());
    }
    let login = catalogs.login_standard(harness);
    let offered: BTreeSet<String> = if names.is_empty() {
        login.iter().map(|_| LOCAL_ACCOUNT.to_owned()).collect()
    } else {
        expand(config, catalogs, harness, names)?
            .into_iter()
            .collect()
    };
    let named = |name: &str| names.iter().any(|each| each == name);
    let members_of = |standard: &str, held: Vec<String>, listed: bool| -> Vec<Member> {
        held.into_iter()
            .filter_map(|member| {
                let on_offer = listed || offered.contains(&member);
                if !on_offer
                    && let Some(Some((account, info))) = fitting(
                        (config, catalogs),
                        (harness, login.as_deref()),
                        standard,
                        &member,
                    )
                    && crate::serve::keeps_key_in_environment(
                        &crate::makers::Makers::of(config),
                        &[(member.clone(), account.clone(), info)],
                        harness,
                    )
                {
                    return None;
                }
                Some(Member {
                    name: member,
                    listed: on_offer,
                })
            })
            .collect()
    };
    let mut pools = Vec::new();
    for (name, pool) in &config.pools {
        if !accepts.contains(&pool.standard) {
            continue;
        }
        let held = runnable((config, catalogs), harness, &pool.standard, Some(pool));
        pools.push(EntryPool {
            name: name.clone(),
            standard: pool.standard.clone(),
            automatic: false,
            listed: named(name),
            members: members_of(&pool.standard, held, named(name)),
        });
    }
    for standard in &accepts {
        let name = automatic_name(standard);
        let listed = named(&name);
        let held = runnable((config, catalogs), harness, standard, None);
        let members = members_of(standard, held, listed);
        if members.is_empty() {
            continue;
        }
        pools.push(EntryPool {
            name,
            standard: standard.clone(),
            automatic: true,
            listed,
            members,
        });
    }
    Ok(pools)
}

pub(crate) mod gather;
pub(crate) mod turn;

#[cfg(test)]
mod tests;
